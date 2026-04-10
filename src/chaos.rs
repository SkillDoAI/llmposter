//! Deterministic chaos primitives for streaming failure injection.
//!
//! llmposter's core promise is determinism: same fixtures + same requests
//! produce the same responses every run. Streaming chaos (jitter,
//! duplicate frames, probabilistic activation) is randomized — but the
//! randomness is seeded, so each chaos decision is reproducible.
//!
//! We deliberately avoid a `rand` dependency: chaos needs exactly one
//! small PRNG and one seed-derivation helper, and both fit in ~30 lines.
//! A tiny `xorshift64` is statistically adequate for jitter/dice rolls and
//! has zero dep footprint.

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
    /// For `range == 0`, always returns `0`.
    pub(crate) fn jitter_i64(&mut self, range: u64) -> i64 {
        if range == 0 {
            return 0;
        }
        // 2*range+1 possible values, then shift into signed range.
        let r = self.next_u64() % (2 * range + 1);
        (r as i64) - (range as i64)
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
    /// Per-frame delay overrides (length == frame count after duplication).
    /// Empty vec means "use the base latency for every frame".
    pub(crate) frame_delays_ms: Vec<u64>,
    /// If true, each source frame has already been duplicated in the
    /// caller's frame list (so the stream helper does not duplicate again).
    pub(crate) frames_duplicated: bool,
}

impl ChaosPlan {
    /// Build a plan for `frame_count` source frames given the failure config
    /// and a monotonically increasing chaos counter.
    ///
    /// - If `failure.probability` rolls above the threshold, chaos is
    ///   inactive and this returns `(ChaosPlan::passthrough(...), false)`.
    /// - Otherwise `latency_jitter_ms` and `duplicate_frames` take effect,
    ///   and the returned plan reflects the duplicated-frame length.
    ///
    /// Returns the plan plus a `bool` indicating whether chaos was active
    /// for this request (useful for tests and verbose logging).
    pub(crate) fn from_failure(
        failure: Option<&crate::fixture::FailureConfig>,
        base_latency_ms: u64,
        frame_count: usize,
        chaos_counter: u64,
    ) -> (Self, bool) {
        let Some(f) = failure else {
            return (Self::passthrough(base_latency_ms, frame_count), false);
        };

        let seed = derive_seed(f.chaos_seed, chaos_counter);
        let mut rng = XorShift64::new(seed);

        // Roll dice for activation. Probability=None means "always on".
        let active = {
            let p = f.probability.unwrap_or(1.0);
            rng.next_f32() < p
        };

        if !active {
            return (Self::passthrough(base_latency_ms, frame_count), false);
        }

        let duplicate = f.duplicate_frames.unwrap_or(false);
        let effective_count = if duplicate {
            frame_count * 2
        } else {
            frame_count
        };

        let frame_delays_ms = match f.latency_jitter_ms {
            Some(range) if range > 0 => (0..effective_count)
                .map(|_| {
                    let delta = rng.jitter_i64(range);
                    (base_latency_ms as i64 + delta).max(0) as u64
                })
                .collect(),
            _ => vec![base_latency_ms; effective_count],
        };

        (
            ChaosPlan {
                frame_delays_ms,
                frames_duplicated: duplicate,
            },
            true,
        )
    }

    /// A passthrough plan: every frame uses the base latency unchanged.
    fn passthrough(base_latency_ms: u64, frame_count: usize) -> Self {
        ChaosPlan {
            frame_delays_ms: vec![base_latency_ms; frame_count],
            frames_duplicated: false,
        }
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
        let (plan, active) = ChaosPlan::from_failure(None, 25, 4, 0);
        assert!(!active);
        assert!(!plan.frames_duplicated);
        assert_eq!(plan.frame_delays_ms, vec![25, 25, 25, 25]);
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
        let (plan, active) = ChaosPlan::from_failure(Some(&f), 20, 3, 0);
        assert!(!active);
        assert!(!plan.frames_duplicated);
        assert_eq!(plan.frame_delays_ms, vec![20, 20, 20]);
    }

    #[test]
    fn plan_with_probability_one_always_activates() {
        let f = crate::fixture::FailureConfig {
            probability: Some(1.0),
            duplicate_frames: Some(true),
            chaos_seed: Some(1),
            ..Default::default()
        };
        let (plan, active) = ChaosPlan::from_failure(Some(&f), 20, 3, 0);
        assert!(active);
        assert!(plan.frames_duplicated);
        // 3 frames × 2 (duplicate) = 6 delays, all equal to 20 (no jitter).
        assert_eq!(plan.frame_delays_ms, vec![20; 6]);
    }

    #[test]
    fn plan_with_jitter_adjusts_delays_deterministically() {
        let f = crate::fixture::FailureConfig {
            latency_jitter_ms: Some(5),
            chaos_seed: Some(42),
            ..Default::default()
        };
        let (plan_a, _) = ChaosPlan::from_failure(Some(&f), 20, 4, 0);
        let (plan_b, _) = ChaosPlan::from_failure(Some(&f), 20, 4, 0);
        assert_eq!(plan_a.frame_delays_ms, plan_b.frame_delays_ms);
        for d in &plan_a.frame_delays_ms {
            assert!(*d <= 25);
            // With base 20 and jitter 5, the minimum is 15 (never clamps to 0).
            assert!(*d >= 15);
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
        let (plan, _) = ChaosPlan::from_failure(Some(&f), 5, 20, 0);
        assert!(plan.frame_delays_ms.contains(&0));
        assert!(plan.frame_delays_ms.iter().all(|d| *d <= 105));
    }
}
