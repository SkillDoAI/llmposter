//! Deterministic chaos primitives for streaming failure injection.
//! Seeded xorshift64 PRNG and a `ChaosPlan` that turns `FailureConfig`
//! into per-request jitter/duplicate/activation decisions. See
//! [`docs/failure-simulation.md`](../../docs/failure-simulation.md)
//! for the user-facing behavior.

/// Seeded xorshift64 PRNG. Not cryptographic; deterministic and fast.
#[derive(Debug, Clone)]
pub(crate) struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    /// Seeds the generator. A seed of zero degenerates xorshift64 to a
    /// zero-only stream, so zero is replaced with a fixed non-zero
    /// constant — callers can still safely pass `0` as a "default" seed.
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0xDEAD_BEEF_CAFE_BABE
            } else {
                seed
            },
        }
    }

    /// Advances the state and returns the next 64-bit value.
    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Returns a uniformly distributed value in `[0.0, 1.0)`.
    /// Uses the top 24 bits so the mantissa is exact.
    pub(crate) fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    /// Returns a signed jitter in the range `[-range, +range]` (inclusive).
    /// For `range == 0`, always returns `0`. Extreme `range` values are
    /// clamped to `i64::MAX` before any arithmetic to avoid debug-build
    /// overflow panics (`fixture.rs::validate` also caps the configurable
    /// upper bound so this is defense-in-depth).
    pub(crate) fn jitter_i64(&mut self, range: u64) -> i64 {
        if range == 0 {
            return 0;
        }
        let range = range.min(i64::MAX as u64);
        // 2*range+1 possible values, then shift into signed range.
        // Using saturating_mul avoids overflow for the pathological cap.
        let span = range.saturating_mul(2).saturating_add(1);
        let r = self.next_u64() % span;
        // Intermediate arithmetic in i128 so `r` values above
        // `i64::MAX` (possible when span approaches 2^63) don't
        // truncate on the final cast — the subtraction always lands
        // inside `[-range, range]`, which is a valid `i64`.
        ((r as i128) - (range as i128)) as i64
    }
}

/// Derive a chaos seed from an optional explicit override and a
/// monotonically increasing request counter. When no override is given,
/// the counter is multiplied by the 64-bit golden ratio and offset by one,
/// which scatters consecutive counters across the 64-bit space so
/// successive requests see visibly different chaos outcomes.
pub(crate) fn derive_seed(explicit: Option<u64>, counter: u64) -> u64 {
    explicit.unwrap_or_else(|| {
        // 0x9E37… = floor(2^64 / phi). Classic hash scrambler.
        counter.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1)
    })
}

/// Chaos configuration resolved for a single request.
///
/// Built in the handler after fixture matching and passed to the
/// streaming helpers. `active == false` collapses the whole struct back
/// to classical behavior — no jitter, no duplication — regardless of the
/// other fields. Callers still own the RNG and advance it as needed.
#[derive(Debug, Clone)]
pub(crate) struct ChaosPlan {
    /// Per-frame delay overrides. `None` means "no chaos is changing delays
    /// — the streaming helpers should apply the base latency uniformly."
    /// When `Some`, the vec length matches the post-duplication frame count.
    pub(crate) frame_delays_ms: Option<Vec<u64>>,
    /// Whether [`apply_frame_duplication`](Self::apply_frame_duplication)
    /// should duplicate each source frame.
    pub(crate) duplicate: bool,
    /// True when chaos was actually applied on this request. Exposed for
    /// verbose logging; callers can ignore it.
    pub(crate) active: bool,
}

impl ChaosPlan {
    /// The zero-cost passthrough plan: no chaos, no allocations, stream
    /// helpers fall back to the base latency for every frame.
    pub(crate) const PASSTHROUGH: Self = Self {
        frame_delays_ms: None,
        duplicate: false,
        active: false,
    };

    /// Build a plan for `frame_count` source frames given the failure config
    /// and a monotonically increasing chaos counter.
    ///
    /// Returns [`Self::PASSTHROUGH`] when `failure` is `None`, the failure
    /// block carries no chaos fields, or `probability` rolls below the
    /// activation threshold. Otherwise computes a per-frame delay vector
    /// (possibly jittered) and records whether frames should be duplicated.
    pub(crate) fn from_failure(
        failure: Option<&crate::fixture::FailureConfig>,
        base_latency_ms: u64,
        frame_count: usize,
        chaos_counter: u64,
    ) -> Self {
        // Short-circuit when the fixture has no chaos fields at all —
        // classical failure flags (latency, corrupt, truncate, disconnect)
        // never need the PRNG, and constructing one would waste cycles on
        // every streaming request.
        let Some(f) = failure else {
            return Self::PASSTHROUGH;
        };
        if !f.has_chaos() {
            return Self::PASSTHROUGH;
        }

        let seed = derive_seed(f.chaos_seed, chaos_counter);
        let mut rng = XorShift64::new(seed);

        // Roll dice for activation. Probability=None means "always on".
        let p = f.probability.unwrap_or(1.0);
        if rng.next_f32() >= p {
            return Self::PASSTHROUGH;
        }

        let duplicate = f.duplicate_frames.unwrap_or(false);
        let effective_count = if duplicate {
            // Saturate instead of wrapping/panicking on pathological
            // frame counts — chaos-enabled streams should degrade, not
            // panic, even under a fuzzer-shaped input.
            frame_count.saturating_mul(2)
        } else {
            frame_count
        };

        let frame_delays_ms = f.latency_jitter_ms.and_then(|range| {
            if range == 0 {
                None
            } else {
                Some(
                    (0..effective_count)
                        .map(|_| {
                            let delta = rng.jitter_i64(range);
                            (base_latency_ms as i64 + delta).max(0) as u64
                        })
                        .collect(),
                )
            }
        });

        Self {
            frame_delays_ms,
            duplicate,
            active: true,
        }
    }

    /// Apply frame duplication if the plan requires it, returning the
    /// (possibly unchanged) frame vector. Pre-sizes the output so the
    /// double-and-push path performs exactly one allocation.
    pub(crate) fn apply_frame_duplication(&self, frames: Vec<String>) -> Vec<String> {
        if !self.duplicate {
            return frames;
        }
        let mut out = Vec::with_capacity(frames.len() * 2);
        for frame in frames {
            out.push(frame.clone());
            out.push(frame);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xorshift_zero_seed_is_rewritten_to_fixed_constant() {
        let mut r1 = XorShift64::new(0);
        let mut r2 = XorShift64::new(0);
        assert_eq!(r1.next_u64(), r2.next_u64());
        assert_ne!(r1.state, 0);
    }

    #[test]
    fn xorshift_nonzero_seed_is_deterministic() {
        let mut a = XorShift64::new(42);
        let mut b = XorShift64::new(42);
        for _ in 0..10 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn next_f32_is_in_unit_interval() {
        let mut r = XorShift64::new(1);
        for _ in 0..1_000 {
            let v = r.next_f32();
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn jitter_range_zero_returns_zero() {
        let mut r = XorShift64::new(7);
        for _ in 0..20 {
            assert_eq!(r.jitter_i64(0), 0);
        }
    }

    #[test]
    fn jitter_range_is_symmetric_and_bounded() {
        let mut r = XorShift64::new(123);
        let mut saw_neg = false;
        let mut saw_pos = false;
        for _ in 0..1_000 {
            let j = r.jitter_i64(10);
            assert!((-10..=10).contains(&j));
            if j < 0 {
                saw_neg = true;
            }
            if j > 0 {
                saw_pos = true;
            }
        }
        assert!(saw_neg, "expected at least one negative jitter");
        assert!(saw_pos, "expected at least one positive jitter");
    }

    #[test]
    fn derive_seed_honors_explicit() {
        assert_eq!(derive_seed(Some(1234), 999), 1234);
    }

    #[test]
    fn derive_seed_without_override_scatters_counters() {
        let a = derive_seed(None, 0);
        let b = derive_seed(None, 1);
        let c = derive_seed(None, 2);
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
    }

    #[test]
    fn plan_with_no_failure_is_passthrough() {
        let plan = ChaosPlan::from_failure(None, 25, 4, 0);
        assert!(!plan.active);
        assert!(plan.frame_delays_ms.is_none());
    }

    #[test]
    fn plan_with_no_chaos_fields_is_passthrough() {
        let f = crate::fixture::FailureConfig {
            latency_ms: Some(1000),
            corrupt_body: Some(true),
            truncate_after_frames: Some(5),
            disconnect_after_ms: Some(200),
            ..Default::default()
        };
        let plan = ChaosPlan::from_failure(Some(&f), 25, 4, 0);
        assert!(!plan.active);
        assert!(plan.frame_delays_ms.is_none());
    }

    #[test]
    fn plan_with_probability_zero_is_passthrough() {
        let f = crate::fixture::FailureConfig {
            probability: Some(0.0),
            latency_jitter_ms: Some(10),
            duplicate_frames: Some(true),
            chaos_seed: Some(1),
            ..Default::default()
        };
        let plan = ChaosPlan::from_failure(Some(&f), 20, 3, 0);
        assert!(!plan.active);
        assert!(plan.frame_delays_ms.is_none());
    }

    #[test]
    fn plan_with_probability_one_always_activates_and_duplicates() {
        let f = crate::fixture::FailureConfig {
            probability: Some(1.0),
            duplicate_frames: Some(true),
            chaos_seed: Some(1),
            ..Default::default()
        };
        let plan = ChaosPlan::from_failure(Some(&f), 20, 3, 0);
        assert!(plan.active);
        // No jitter configured → frame_delays_ms stays None (stream helpers
        // will fall back to the base latency).
        assert!(plan.frame_delays_ms.is_none());
        let dup = plan.apply_frame_duplication(vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(dup, vec!["a", "a", "b", "b", "c", "c"]);
    }

    #[test]
    fn plan_with_jitter_adjusts_delays_deterministically() {
        let f = crate::fixture::FailureConfig {
            latency_jitter_ms: Some(5),
            chaos_seed: Some(42),
            ..Default::default()
        };
        let plan_a = ChaosPlan::from_failure(Some(&f), 20, 4, 0);
        let plan_b = ChaosPlan::from_failure(Some(&f), 20, 4, 0);
        assert_eq!(plan_a.frame_delays_ms, plan_b.frame_delays_ms);
        let delays = plan_a.frame_delays_ms.as_ref().expect("delays present");
        assert_eq!(delays.len(), 4);
        for d in delays {
            // With base 20 and jitter 5, the range is [15, 25].
            assert!((15..=25).contains(d));
        }
    }

    #[test]
    fn plan_jitter_clamps_negative_to_zero() {
        let f = crate::fixture::FailureConfig {
            latency_jitter_ms: Some(100),
            chaos_seed: Some(7),
            ..Default::default()
        };
        // Base latency 5, jitter 100 — negatives clamp to 0.
        let plan = ChaosPlan::from_failure(Some(&f), 5, 20, 0);
        let delays = plan.frame_delays_ms.as_ref().expect("delays present");
        assert!(delays.contains(&0));
        assert!(delays.iter().all(|d| *d <= 105));
    }

    #[test]
    fn apply_frame_duplication_is_noop_when_not_configured() {
        let plan = ChaosPlan::PASSTHROUGH;
        let frames = vec!["a".to_string(), "b".to_string()];
        assert_eq!(plan.apply_frame_duplication(frames.clone()), frames);
    }

    #[test]
    fn jitter_i64_caps_extreme_range_without_panicking() {
        let mut r = XorShift64::new(1);
        // Must not panic under debug overflow checks — the saturating
        // arithmetic clamps `range` to i64::MAX before any subtraction.
        let _ = r.jitter_i64(u64::MAX);
        let _ = r.jitter_i64(u64::MAX - 1);
    }

    #[test]
    fn plan_zero_jitter_is_passthrough() {
        // Some(0) jitter should leave delays_override as None (use base).
        let f = crate::fixture::FailureConfig {
            latency_jitter_ms: Some(0),
            chaos_seed: Some(1),
            ..Default::default()
        };
        let plan = ChaosPlan::from_failure(Some(&f), 20, 3, 0);
        assert!(plan.active);
        assert!(plan.frame_delays_ms.is_none());
    }
}
