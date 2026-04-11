use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

/// How to match a string field — substring (default) or regex.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum StringMatch {
    /// Plain substring match (case-sensitive).
    Substring(String),
    /// Regex match using `{ regex: "pattern" }` YAML syntax.
    Regex(RegexMatch),
}

/// Wrapper for `{ regex: "pattern" }` syntax in YAML.
/// After validation, `compiled` holds the pre-compiled regex for efficient linear-time matching.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegexMatch {
    /// The regex pattern string from the YAML fixture.
    pub regex: String,
    #[serde(skip)]
    compiled: Option<regex::Regex>,
}

impl PartialEq for RegexMatch {
    fn eq(&self, other: &Self) -> bool {
        self.regex == other.regex
    }
}

impl RegexMatch {
    fn compile(&mut self) -> Result<(), String> {
        if self.compiled.is_some() {
            return Ok(()); // Already compiled, skip
        }
        let re = regex::RegexBuilder::new(&self.regex)
            .size_limit(1 << 20)
            .dfa_size_limit(1 << 20) // Cap both compiled NFA and per-thread DFA cache
            .build()
            .map_err(|e| format!("Invalid regex '{}': {}", self.regex, e))?;
        self.compiled = Some(re);
        Ok(())
    }

    fn is_match(&self, haystack: &str) -> bool {
        match &self.compiled {
            Some(re) => re.is_match(haystack),
            None => {
                // Fallback: compile on the fly. This path is only hit if
                // validate() was not called (programmatic fixtures added
                // without going through ServerBuilder::build).
                match regex::RegexBuilder::new(&self.regex)
                    .size_limit(1 << 20)
                    .dfa_size_limit(1 << 20)
                    .build()
                {
                    Ok(re) => re.is_match(haystack),
                    Err(e) => {
                        eprintln!("[llmposter] Warning: invalid regex '{}': {}", self.regex, e);
                        false
                    }
                }
            }
        }
    }
}

impl StringMatch {
    /// Create a regex `StringMatch` from a pattern string.
    pub fn regex(pattern: &str) -> Self {
        StringMatch::Regex(RegexMatch {
            regex: pattern.to_string(),
            compiled: None,
        })
    }
}

/// Match criteria for a fixture.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FixtureMatch {
    /// Match by substring or regex in the last user message.
    pub user_message: Option<StringMatch>,
    /// Match by model name (substring or regex).
    pub model: Option<StringMatch>,
}

/// A tool call in a fixture response.
///
/// The `arguments` field must be a JSON object (validated at load time).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    /// Function name (e.g., `"get_weather"`).
    pub name: String,
    /// Function arguments as a JSON object.
    pub arguments: serde_json::Value,
}

/// Opaque compile cache for a fixture's minijinja template.
///
/// Populated on first render and reused for every subsequent request,
/// eliminating per-request `add_template` cost for hot templated
/// fixtures. Exposes no public methods beyond `Default`.
#[cfg(feature = "templating")]
#[derive(Default)]
pub struct TemplateCache {
    cell: std::sync::OnceLock<Result<std::sync::Arc<minijinja::Environment<'static>>, String>>,
}

#[cfg(feature = "templating")]
impl std::fmt::Debug for TemplateCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemplateCache")
            .field("initialized", &self.cell.get().is_some())
            .finish()
    }
}

#[cfg(feature = "templating")]
impl Clone for TemplateCache {
    // NOTE: a clone always returns a fresh empty cache. The contract
    // relies on the invariant that fixtures live inside `Arc<Fixture>`
    // post-build (`AppState.fixtures: Vec<Arc<Fixture>>`), so
    // `Fixture::clone()` — and therefore this impl — is never called
    // on the request hot path. If a future refactor reintroduces a
    // direct `Fixture::clone()` anywhere, the compile cache is
    // silently defeated for that path.
    fn clone(&self) -> Self {
        Self::default()
    }
}

#[cfg(feature = "templating")]
impl TemplateCache {
    /// Returns a reference to the compiled environment, building it on
    /// first call. The compile result — success or the error message — is
    /// cached so subsequent calls don't pay the compile cost again.
    pub(crate) fn get_or_compile(
        &self,
        template_source: &str,
    ) -> Result<&std::sync::Arc<minijinja::Environment<'static>>, &str> {
        let entry = self.cell.get_or_init(|| {
            let mut env = minijinja::Environment::new();
            env.add_template_owned("t", template_source.to_string())
                .map_err(|e| format!("template compile error: {}", e))?;
            Ok(std::sync::Arc::new(env))
        });
        match entry {
            Ok(env) => Ok(env),
            Err(msg) => Err(msg.as_str()),
        }
    }
}

/// The response to return when a fixture matches.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FixtureResponse {
    /// Text content to return (mutually exclusive with `tool_calls` and
    /// `content_template`).
    pub content: Option<String>,
    /// Jinja-style template rendered at response time, with access to
    /// request fields (`user_message`, `model`, `provider`, `request`).
    /// Mutually exclusive with `content` and `tool_calls`. Requires the
    /// `templating` feature — if that feature is disabled, any fixture
    /// with `content_template` set is rejected at load time with a clear
    /// error pointing at the feature flag.
    pub content_template: Option<String>,
    /// Tool calls to return (mutually exclusive with `content`).
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Anthropic-style stop reason (e.g. `"end_turn"`, `"tool_use"`).
    pub stop_reason: Option<String>,
    /// OpenAI-style finish reason (e.g. `"stop"`, `"tool_calls"`).
    pub finish_reason: Option<String>,
    /// Compile cache for `content_template`. Populated lazily on first
    /// render; see [`TemplateCache`] for details. This field MUST stay
    /// `pub` (not `pub(crate)`) because external tests construct
    /// `FixtureResponse` via `..Default::default()` and Rust's
    /// functional update syntax still requires every field to be
    /// visible from the caller's position. `TemplateCache` exposes no
    /// mutation methods, so external callers can only reset it to its
    /// default value — never populate or observe the cache contents.
    /// Hidden from rustdoc so it doesn't pollute the public surface
    /// browsable on docs.rs.
    #[cfg(feature = "templating")]
    #[serde(skip)]
    #[doc(hidden)]
    pub template_cache: TemplateCache,
}

/// Error simulation — returns an HTTP error status.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureError {
    /// HTTP status code (must be 400-599).
    pub status: u16,
    /// Error message included in the response body.
    pub message: String,
    /// Optional response headers to include (e.g. override rate limit headers on 429).
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

/// Failure simulation — network/streaming problems.
///
/// Two flavors of failure:
///
/// - **Classical** (`latency_ms`, `corrupt_body`, `truncate_after_frames`,
///   `disconnect_after_ms`): deterministic, always fire when set.
/// - **Chaos** (`latency_jitter_ms`, `duplicate_frames`, `probability`,
///   `chaos_seed`): randomized but seeded, so runs are reproducible. Chaos
///   fields are gated by `probability` — rolling above the probability on
///   a given request leaves chaos inactive for that request. Classical
///   failures ignore `probability` and always apply.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FailureConfig {
    /// Inject latency (in milliseconds) before sending the response.
    pub latency_ms: Option<u64>,
    /// If `true`, corrupt the response body (invalid JSON/SSE).
    pub corrupt_body: Option<bool>,
    /// Truncate SSE stream after N frames (including preamble events).
    /// Alias: `truncate_after_chunks` (deprecated, use `truncate_after_frames`).
    ///
    /// **Interaction with `duplicate_frames`:** this count is applied to the
    /// stream AFTER duplication. If `duplicate_frames: true` doubles the
    /// source frames, `truncate_after_frames: 2` sends the first two
    /// *doubled* entries (i.e. the first source frame emitted twice). Set
    /// `truncate_after_frames` to `2 * N` if you want to cut after `N` of
    /// the original frames.
    #[serde(alias = "truncate_after_chunks")]
    pub truncate_after_frames: Option<u32>,
    /// Abruptly close the connection after this many milliseconds.
    pub disconnect_after_ms: Option<u64>,
    // --- Streaming chaos (seeded, deterministic per request) ---
    /// Add random ±jitter (milliseconds) to the per-frame streaming latency.
    /// Requires a base `streaming.latency` to act on. Jitter is symmetric:
    /// a jitter of `10` adds a value in the range `[-10, +10]` to each frame
    /// delay. The effective delay is clamped at zero — a jittered negative
    /// value becomes an immediate frame.
    pub latency_jitter_ms: Option<u64>,
    /// If `true`, emit each streaming frame twice back-to-back. Useful for
    /// testing idempotent-consumer logic that must tolerate repeated events.
    ///
    /// **Interaction with `truncate_after_frames`:** duplication happens
    /// before truncation counting, so `duplicate_frames: true` +
    /// `truncate_after_frames: N` cuts after N *doubled* frames. See the
    /// `truncate_after_frames` doc for the full explanation.
    pub duplicate_frames: Option<bool>,
    /// Probability in `[0.0, 1.0]` that the chaos fields activate for a
    /// given request. `None` or `1.0` = always. `0.0` = never. Classical
    /// failures (latency_ms, corrupt_body, truncate, disconnect) are NOT
    /// affected by this — only the chaos fields above.
    pub probability: Option<f32>,
    /// Override the chaos PRNG seed. When unset, the seed is derived from
    /// an internal per-server request counter, so successive requests from
    /// the same test produce a deterministic but distinct sequence of chaos
    /// outcomes. Setting `chaos_seed` to a fixed value reproduces the same
    /// jitter/duplicate pattern across server instances.
    pub chaos_seed: Option<u64>,
}

impl FailureConfig {
    /// Returns true if any of the chaos-specific fields are set — i.e. this
    /// failure config requires the chaos PRNG + activation roll. Classical
    /// failure fields (latency_ms, corrupt_body, truncate_after_frames,
    /// disconnect_after_ms) are not chaos and do not trigger this.
    pub(crate) fn has_chaos(&self) -> bool {
        self.latency_jitter_ms.is_some()
            || self.duplicate_frames.is_some()
            || self.probability.is_some()
            || self.chaos_seed.is_some()
    }
}

/// Streaming behavior config.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamingConfig {
    /// Delay in milliseconds between SSE frames.
    pub latency: Option<u64>,
    /// Number of Unicode characters per streaming chunk.
    pub chunk_size: Option<usize>,
}

/// Scenario state machine config for multi-turn fixtures.
///
/// When a fixture has a `scenario` block, it participates in a named state machine.
/// The fixture only matches when the scenario's current state equals `required_state`
/// (or if `required_state` is not set). After matching, the scenario state advances
/// to `set_state`.
///
/// # YAML Example
///
/// ```yaml
/// fixtures:
///   - match:
///       user_message: "weather in Paris"
///     scenario:
///       name: "weather-flow"
///       set_state: "tool_called"
///     response:
///       tool_calls:
///         - name: get_weather
///           arguments: { location: "Paris" }
///
///   - match:
///       user_message: "tool_result"
///     scenario:
///       name: "weather-flow"
///       required_state: "tool_called"
///       set_state: "completed"
///     response:
///       content: "It's 22°C in Paris"
/// ```
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioConfig {
    /// Name of the scenario state machine.
    pub name: String,
    /// Only match this fixture when the scenario is in this state.
    /// If not set, the fixture matches regardless of current state.
    pub required_state: Option<String>,
    /// Advance the scenario to this state after the fixture matches.
    /// If not set, the scenario state is unchanged.
    pub set_state: Option<String>,
}

/// Safety refusal configuration for a fixture.
///
/// Produces a provider-appropriate refusal response:
///
/// - **OpenAI Chat Completions**: `message.refusal: "<reason>"` with
///   `content: null`; `finish_reason: "stop"`.
/// - **Anthropic**: a text content block with the refusal reason and
///   `stop_reason: "refusal"` (Anthropic's native refusal stop reason).
/// - **Gemini**: `candidates: []` plus `promptFeedback.blockReason:
///   "SAFETY"`. Mirrors the real Gemini shape when the prompt itself is
///   blocked.
/// - **OpenAI Responses API**: a message output item containing a
///   single `type: "refusal"` content part; top-level
///   `status: "completed"`.
///
/// This is a first-class fixture outcome — tests exercising client-side
/// refusal handling no longer need to hand-roll provider-specific error
/// shapes.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refusal {
    /// Human-readable refusal text returned to the client. Required.
    pub reason: String,
}

/// A single fixture entry.
///
/// Fixtures are the core building block of llmposter. Each fixture defines a
/// match rule, a response (or error/failure/refusal), and optional streaming/scenario config.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    /// Match criteria (user message, model). If absent, fixture matches all requests.
    #[serde(rename = "match")]
    pub match_rule: Option<FixtureMatch>,
    /// Restrict this fixture to a specific LLM provider endpoint.
    pub provider: Option<crate::format::Provider>,
    /// The response to return when matched.
    pub response: Option<FixtureResponse>,
    /// Error simulation (HTTP error status + message).
    pub error: Option<FixtureError>,
    /// Safety refusal — provider-specific refusal-shape response.
    /// Mutually exclusive with `response`, `error`, and `failure`.
    /// Only applies to non-streaming requests: a `refusal` fixture
    /// matched against `stream: true` returns HTTP 400 (streaming
    /// refusal envelopes are not yet implemented).
    pub refusal: Option<Refusal>,
    /// Failure simulation (latency, corruption, truncation, disconnect).
    pub failure: Option<FailureConfig>,
    /// Streaming behavior (latency between frames, chunk size).
    pub streaming: Option<StreamingConfig>,
    /// Scenario state machine — enables multi-turn fixture matching.
    pub scenario: Option<ScenarioConfig>,
}

/// Top-level YAML file structure (internal, used for deserialization only).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FixtureFile {
    /// List of fixture entries from the YAML file.
    pub fixtures: Vec<Fixture>,
}

// --- Programmatic builder API ---

impl Fixture {
    /// Create a new empty fixture. Matches all requests (catch-all) by default.
    pub fn new() -> Self {
        Self {
            match_rule: None,
            provider: None,
            response: None,
            error: None,
            refusal: None,
            failure: None,
            streaming: None,
            scenario: None,
        }
    }

    /// Configure this fixture to return a provider-specific safety refusal.
    ///
    /// Mutually exclusive with `respond_with_content`, `respond_with_tool_calls`,
    /// and `with_error`. The `reason` string is the refusal text returned to the
    /// client.
    pub fn respond_with_refusal(mut self, reason: &str) -> Self {
        self.refusal = Some(Refusal {
            reason: reason.to_string(),
        });
        self
    }

    /// Match requests where the last user message contains `pattern` (substring match).
    pub fn match_user_message(mut self, pattern: &str) -> Self {
        let m = self.match_rule.get_or_insert_with(FixtureMatch::default);
        m.user_message = Some(StringMatch::Substring(pattern.to_string()));
        self
    }

    /// Match requests where the model name contains `pattern` (substring match).
    pub fn match_model(mut self, pattern: &str) -> Self {
        let m = self.match_rule.get_or_insert_with(FixtureMatch::default);
        m.model = Some(StringMatch::Substring(pattern.to_string()));
        self
    }

    /// Set a plain-text content response for this fixture.
    pub fn respond_with_content(mut self, content: &str) -> Self {
        let r = self.response.get_or_insert(FixtureResponse::default());
        r.content = Some(content.to_string());
        r.tool_calls = None;
        self
    }

    /// Configure this fixture to return an HTTP error response.
    pub fn with_error(mut self, status: u16, message: &str) -> Self {
        self.error = Some(FixtureError {
            status,
            message: message.to_string(),
            headers: HashMap::new(),
        });
        self
    }

    /// Like `with_error` but also sets custom response headers (e.g. to override
    /// rate limit header values on a 429 fixture).
    ///
    /// Returns `Err` if any header name or value is not a valid HTTP header.
    pub fn with_error_headers<I, K, V>(
        mut self,
        status: u16,
        message: &str,
        headers: I,
    ) -> Result<Self, String>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        use axum::http::{HeaderName, HeaderValue};
        use std::str::FromStr;
        let mut map = HashMap::new();
        for (k, v) in headers {
            HeaderName::from_str(k.as_ref())
                .map_err(|e| format!("invalid header name {:?}: {e}", k.as_ref()))?;
            HeaderValue::from_str(v.as_ref())
                .map_err(|e| format!("invalid header value {:?}: {e}", v.as_ref()))?;
            let lower = k.as_ref().to_ascii_lowercase();
            if map.contains_key(&lower) {
                return Err(format!(
                    "duplicate header name (case-insensitive): {lower:?}"
                ));
            }
            map.insert(lower, v.as_ref().to_string());
        }
        self.error = Some(FixtureError {
            status,
            message: message.to_string(),
            headers: map,
        });
        Ok(self)
    }

    /// Attach a failure simulation (latency, corruption, truncation, disconnect).
    pub fn with_failure(mut self, failure: FailureConfig) -> Self {
        self.failure = Some(failure);
        self
    }

    /// Set the Anthropic-style `stop_reason` on the response.
    pub fn with_stop_reason(mut self, reason: &str) -> Self {
        self.response
            .get_or_insert(FixtureResponse::default())
            .stop_reason = Some(reason.to_string());
        self
    }

    /// Set the OpenAI-style `finish_reason` on the response.
    pub fn with_finish_reason(mut self, reason: &str) -> Self {
        self.response
            .get_or_insert(FixtureResponse::default())
            .finish_reason = Some(reason.to_string());
        self
    }

    /// Configure streaming behavior (inter-frame latency and chunk size).
    pub fn with_streaming(mut self, latency: Option<u64>, chunk_size: Option<usize>) -> Self {
        self.streaming = Some(StreamingConfig {
            latency,
            chunk_size,
        });
        self
    }

    /// Attach this fixture to a named scenario state machine.
    ///
    /// - `name`: scenario identifier (shared across fixtures in the same scenario)
    /// - `required_state`: only match when the scenario is in this state (None = always match)
    /// - `set_state`: advance the scenario to this state after matching (None = no change)
    pub fn with_scenario(
        mut self,
        name: &str,
        required_state: Option<&str>,
        set_state: Option<&str>,
    ) -> Self {
        self.scenario = Some(ScenarioConfig {
            name: name.to_string(),
            required_state: required_state.map(|s| s.to_string()),
            set_state: set_state.map(|s| s.to_string()),
        });
        self
    }

    /// Restrict this fixture to a specific LLM provider endpoint.
    pub fn for_provider(mut self, provider: crate::format::Provider) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Set the response to return tool calls instead of text content.
    pub fn respond_with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        let r = self.response.get_or_insert(FixtureResponse::default());
        r.tool_calls = Some(tool_calls);
        r.content = None;
        self
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

// --- Validation ---

impl Fixture {
    /// Validate fixture invariants and pre-compile regex patterns.
    pub fn validate(&mut self) -> Result<(), String> {
        if let Some(ref e) = self.error {
            if !(400..=599).contains(&e.status) {
                return Err("error.status must be an error HTTP status (400-599)".to_string());
            }
            use axum::http::{HeaderName, HeaderValue};
            use std::str::FromStr;
            for (name, value) in &e.headers {
                HeaderName::from_str(name)
                    .map_err(|err| format!("invalid error header name {name:?}: {err}"))?;
                HeaderValue::from_str(value)
                    .map_err(|err| format!("invalid error header value {value:?}: {err}"))?;
            }
        }
        // Normalize header keys to lowercase, rejecting case-insensitive duplicates.
        // (Ends immutable borrow before mutating.)
        if let Some(ref mut e) = self.error {
            let mut normalized: HashMap<String, String> = HashMap::new();
            for (k, v) in e.headers.drain() {
                let lower = k.to_ascii_lowercase();
                if normalized.contains_key(&lower) {
                    return Err(format!(
                        "duplicate error header name (case-insensitive): {lower:?}"
                    ));
                }
                normalized.insert(lower, v);
            }
            e.headers = normalized;
        }
        if self.response.is_some() && self.error.is_some() {
            return Err("'error' and 'response' are mutually exclusive".to_string());
        }
        if self.error.is_some() && self.failure.is_some() {
            return Err("'error' and 'failure' are mutually exclusive".to_string());
        }
        if self.refusal.is_some() && self.response.is_some() {
            return Err("'refusal' and 'response' are mutually exclusive".to_string());
        }
        if self.refusal.is_some() && self.error.is_some() {
            return Err("'refusal' and 'error' are mutually exclusive".to_string());
        }
        if self.refusal.is_some() && self.failure.is_some() {
            return Err("'refusal' and 'failure' are mutually exclusive".to_string());
        }
        if let Some(ref r) = self.refusal {
            if r.reason.trim().is_empty() {
                return Err("refusal.reason must not be blank".to_string());
            }
        }
        if self.failure.is_some() && self.response.is_none() {
            return Err("'failure' requires response to also be present".to_string());
        }
        if let (Some(ref f), None) = (&self.failure, &self.streaming) {
            let has_stream_failure =
                f.truncate_after_frames.is_some() || f.disconnect_after_ms.is_some();
            if has_stream_failure {
                eprintln!(
                    "[llmposter] Warning: failure.truncate_after_frames/disconnect_after_ms \
                     have no effect without streaming configured"
                );
            }
            // Same gap applies to `duplicate_frames`: the chaos plan is only
            // consulted inside the `is_streaming` branch of the handler, so
            // a non-streaming fixture that sets duplicate_frames silently
            // drops the flag. Warn so the misconfiguration is visible.
            if f.duplicate_frames == Some(true) {
                eprintln!(
                    "[llmposter] Warning: failure.duplicate_frames has no effect \
                     without streaming configured"
                );
            }
        }
        // Validate chaos field invariants.
        if let Some(ref f) = self.failure {
            if let Some(p) = f.probability {
                if !p.is_finite() || !(0.0..=1.0).contains(&p) {
                    return Err(format!(
                        "failure.probability must be in [0.0, 1.0], got {}",
                        p
                    ));
                }
            }
            // latency_jitter_ms: Some(0) is a documented no-op (ChaosPlan
            // collapses it to None), so it needs no base latency and no cap
            // check. Only enforce the constraints when jitter > 0.
            if let Some(jitter) = f.latency_jitter_ms {
                if jitter > 0 {
                    let base_latency = self.streaming.as_ref().and_then(|s| s.latency).unwrap_or(0);
                    if base_latency == 0 {
                        return Err(
                            "failure.latency_jitter_ms requires a non-zero streaming.latency"
                                .to_string(),
                        );
                    }
                    // Cap at 1 hour. A mock server has no legitimate reason
                    // to jitter a per-frame delay beyond this, and the upper
                    // bound keeps the chaos PRNG arithmetic inside i64.
                    const MAX_JITTER_MS: u64 = 60 * 60 * 1000;
                    if jitter > MAX_JITTER_MS {
                        return Err(format!(
                            "failure.latency_jitter_ms must be <= {} (got {})",
                            MAX_JITTER_MS, jitter
                        ));
                    }
                }
            }
            // Warn on degenerate chaos config: `chaos_seed` and `probability`
            // only take effect when paired with `latency_jitter_ms` or
            // `duplicate_frames`. A fixture setting only the gating fields
            // advances the chaos counter but produces no observable effect,
            // which is confusing to debug. This is a warning, not an error —
            // the config is technically valid.
            let has_effect_field = f.latency_jitter_ms.map(|j| j > 0).unwrap_or(false)
                || f.duplicate_frames == Some(true);
            let has_gate_field = f.chaos_seed.is_some() || f.probability.is_some();
            if has_gate_field && !has_effect_field {
                eprintln!(
                    "[llmposter] Warning: failure.chaos_seed/probability set without \
                     latency_jitter_ms or duplicate_frames — chaos fields have no \
                     observable effect"
                );
            }
        }
        // Validate FixtureResponse mutual exclusivity
        if let Some(ref r) = self.response {
            // content_template requires the `templating` feature. Reject
            // early with a clear error so users who typo the feature name
            // or disable it intentionally know exactly what's wrong.
            #[cfg(not(feature = "templating"))]
            if r.content_template.is_some() {
                return Err(
                    "'content_template' requires the 'templating' feature — rebuild with \
                     `--features templating` to enable it"
                        .to_string(),
                );
            }
            if r.content.is_some() && r.content_template.is_some() {
                return Err(
                    "'content' and 'content_template' in response are mutually exclusive"
                        .to_string(),
                );
            }
            if r.content_template.is_some() && r.tool_calls.is_some() {
                return Err(
                    "'content_template' and 'tool_calls' in response are mutually exclusive"
                        .to_string(),
                );
            }
            if r.content.is_some() && r.tool_calls.is_some() {
                return Err(
                    "'content' and 'tool_calls' in response are mutually exclusive".to_string(),
                );
            }
            if r.content.is_none() && r.tool_calls.is_none() && r.content_template.is_none() {
                return Err(
                    "response must have either 'content', 'content_template', or 'tool_calls'"
                        .to_string(),
                );
            }
            if let Some(ref tc) = r.tool_calls {
                if tc.is_empty() {
                    return Err("tool_calls must not be empty".to_string());
                }
                for (i, call) in tc.iter().enumerate() {
                    if call.name.trim().is_empty() {
                        return Err(format!("tool_calls[{}].name must not be empty", i));
                    }
                    if !call.arguments.is_object() {
                        return Err(format!(
                            "tool_calls[{}].arguments must be a JSON object, got {}",
                            i,
                            match &call.arguments {
                                serde_json::Value::Array(_) => "array",
                                serde_json::Value::String(_) => "string",
                                serde_json::Value::Number(_) => "number",
                                serde_json::Value::Bool(_) => "boolean",
                                serde_json::Value::Null => "null",
                                _ => "non-object",
                            }
                        ));
                    }
                }
            }
        }
        if self.response.is_none() && self.error.is_none() && self.refusal.is_none() {
            return Err("Fixture must have either 'response', 'error', or 'refusal'".to_string());
        }
        if let Some(ref s) = self.streaming {
            if s.chunk_size == Some(0) {
                return Err("streaming.chunk_size must be > 0".to_string());
            }
            if self.error.is_some() {
                return Err("'streaming' config has no effect on error-only fixtures".to_string());
            }
        }
        if let Some(ref mut m) = self.match_rule {
            // Reject empty substring patterns (would match everything)
            if let Some(StringMatch::Substring(ref s)) = m.user_message {
                if s.is_empty() {
                    return Err("match.user_message must not be empty".to_string());
                }
            }
            if let Some(StringMatch::Substring(ref s)) = m.model {
                if s.is_empty() {
                    return Err("match.model must not be empty".to_string());
                }
            }
            if let Some(StringMatch::Regex(ref mut r)) = m.user_message {
                if r.regex.is_empty() {
                    return Err("match.user_message regex must not be empty".to_string());
                }
                r.compile().map_err(|e| format!("user_message {}", e))?;
            }
            if let Some(StringMatch::Regex(ref mut r)) = m.model {
                if r.regex.is_empty() {
                    return Err("match.model regex must not be empty".to_string());
                }
                r.compile().map_err(|e| format!("model {}", e))?;
            }
        }
        Ok(())
    }
}

// --- Matching ---

/// Find the first fixture that matches the given request parameters and scenario state.
///
/// Fixtures are evaluated in order (first-match-wins). If a fixture has a `scenario`
/// with `required_state`, it only matches when the scenario's current state equals
/// that value. Fixtures without a scenario always participate in matching.
///
/// Production code iterates `&[Arc<Fixture>]` directly and calls
/// [`fixture_matches`] — this function survives as a `&[Fixture]` helper
/// for external callers and unit tests, but is hidden from rustdoc.
#[doc(hidden)]
pub fn match_fixture<'a>(
    fixtures: &'a [Fixture],
    user_message: &str,
    model: Option<&str>,
    provider: Option<crate::format::Provider>,
    scenario_states: Option<&std::collections::HashMap<String, String>>,
) -> Option<&'a Fixture> {
    fixtures
        .iter()
        .find(|f| fixture_matches(f, user_message, model, provider, scenario_states))
}

pub(crate) fn fixture_matches(
    fixture: &Fixture,
    user_message: &str,
    model: Option<&str>,
    provider: Option<crate::format::Provider>,
    scenario_states: Option<&std::collections::HashMap<String, String>>,
) -> bool {
    if let Some(fp) = fixture.provider {
        match provider {
            Some(p) if p == fp => {}
            _ => return false,
        }
    }

    // Check scenario required_state
    if let Some(ref scenario) = fixture.scenario {
        if let Some(ref required) = scenario.required_state {
            let current = scenario_states
                .and_then(|states| states.get(&scenario.name))
                .map(|s| s.as_str())
                .unwrap_or("");
            if current != required {
                return false;
            }
        }
    }

    if let Some(ref m) = fixture.match_rule {
        if let Some(ref um) = m.user_message {
            if !string_matches(um, user_message) {
                return false;
            }
        }
        if let Some(ref mm) = m.model {
            match model {
                Some(m) => {
                    if !string_matches(mm, m) {
                        return false;
                    }
                }
                None => return false,
            }
        }
    }

    true
}

fn string_matches(pattern: &StringMatch, haystack: &str) -> bool {
    match pattern {
        StringMatch::Substring(s) => haystack.contains(s.as_str()),
        StringMatch::Regex(r) => r.is_match(haystack),
    }
}

// --- YAML loading ---

/// Load and validate fixtures from a single YAML file.
pub fn load_yaml_file(path: &Path) -> Result<Vec<Fixture>, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let file: FixtureFile = serde_yaml_ng::from_str(&content)
        .map_err(|e| format!("Invalid YAML in {}: {}", path.display(), e))?;

    let mut fixtures = file.fixtures;
    for (i, fixture) in fixtures.iter_mut().enumerate() {
        fixture
            .validate()
            .map_err(|e| format!("Fixture #{} in {}: {}", i + 1, path.display(), e))?;
    }

    Ok(fixtures)
}

/// Re-read and concatenate fixtures from a list of source paths (files or directories).
/// Used by hot-reload to rebuild the fixture list on file change or SIGHUP.
pub(crate) fn reload_sources(sources: &[std::path::PathBuf]) -> Result<Vec<Fixture>, String> {
    let mut fixtures = Vec::new();
    for path in sources {
        let loaded = if path.is_dir() {
            load_yaml_dir(path).map_err(|e| format!("{}: {}", path.display(), e))?
        } else {
            load_yaml_file(path).map_err(|e| format!("{}: {}", path.display(), e))?
        };
        fixtures.extend(loaded);
    }
    Ok(fixtures)
}

/// Load and validate fixtures from all `.yaml`/`.yml` files in a directory (sorted by filename).
pub fn load_yaml_dir(dir: &Path) -> Result<Vec<Fixture>, Box<dyn std::error::Error>> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read directory {}: {}", dir.display(), e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Error reading directory entry in {}: {}", dir.display(), e))?
        .into_iter()
        .filter(|e| {
            let is_file = e.file_type().map(|ft| ft.is_file()).unwrap_or(false);
            if !is_file {
                return false;
            }
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".yaml") || name.ends_with(".yml")
        })
        .collect();

    entries.sort_by_key(|e| e.file_name());

    let mut all_fixtures = Vec::new();
    for entry in entries {
        let fixtures = load_yaml_file(&entry.path())?;
        all_fixtures.extend(fixtures);
    }

    Ok(all_fixtures)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- YAML parsing tests ---

    #[test]
    fn should_parse_simple_text_fixture() {
        let yaml = r#"
fixtures:
  - match:
      user_message: "hello"
    response:
      content: "Hi there!"
"#;
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(file.fixtures.len(), 1);
        let f = &file.fixtures[0];
        assert_eq!(
            f.match_rule.as_ref().unwrap().user_message,
            Some(StringMatch::Substring("hello".to_string()))
        );
        assert_eq!(
            f.response.as_ref().unwrap().content.as_deref(),
            Some("Hi there!")
        );
    }

    #[test]
    fn should_parse_regex_match() {
        let yaml = r#"
fixtures:
  - match:
      user_message:
        regex: "hello \\w+"
    response:
      content: "matched regex"
"#;
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        match &f.match_rule.as_ref().unwrap().user_message {
            Some(StringMatch::Regex(r)) => assert_eq!(r.regex, "hello \\w+"),
            other => panic!("Expected Regex, got {:?}", other),
        }
    }

    #[test]
    fn should_parse_error_fixture() {
        let yaml = r#"
fixtures:
  - match:
      model: "fail-model"
    error:
      status: 429
      message: "Rate limit exceeded"
"#;
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        assert!(f.response.is_none());
        let err = f.error.as_ref().unwrap();
        assert_eq!(err.status, 429);
        assert_eq!(err.message, "Rate limit exceeded");
    }

    #[test]
    fn should_parse_failure_config() {
        let yaml = r#"
fixtures:
  - match:
      user_message: "slow"
    response:
      content: "delayed"
    failure:
      latency_ms: 5000
"#;
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        assert_eq!(f.failure.as_ref().unwrap().latency_ms, Some(5000));
    }

    #[test]
    fn should_parse_streaming_config() {
        let yaml = r#"
fixtures:
  - match:
      user_message: "stream"
    response:
      content: "streamed"
    streaming:
      latency: 50
      chunk_size: 10
"#;
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        let s = f.streaming.as_ref().unwrap();
        assert_eq!(s.latency, Some(50));
        assert_eq!(s.chunk_size, Some(10));
    }

    #[test]
    fn should_parse_tool_call_response() {
        let yaml = r#"
fixtures:
  - match:
      user_message: "weather"
    response:
      tool_calls:
        - name: get_weather
          arguments:
            location: "San Francisco"
"#;
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        let tc = &file.fixtures[0]
            .response
            .as_ref()
            .unwrap()
            .tool_calls
            .as_ref()
            .unwrap()[0];
        assert_eq!(tc.name, "get_weather");
        assert_eq!(tc.arguments["location"], "San Francisco");
    }

    #[test]
    fn should_parse_provider_specific_fixture() {
        let yaml = r#"
fixtures:
  - match:
      user_message: "test"
    provider: anthropic
    response:
      content: "response"
      stop_reason: end_turn
"#;
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        assert_eq!(f.provider, Some(crate::format::Provider::Anthropic));
    }

    #[test]
    fn should_reject_invalid_yaml() {
        let yaml = "not: [valid: yaml: {{{";
        let result: Result<FixtureFile, _> = serde_yaml_ng::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn should_parse_model_match() {
        let yaml = r#"
fixtures:
  - match:
      model: "gpt-4"
      user_message: "hello"
    response:
      content: "hi"
"#;
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        let m = file.fixtures[0].match_rule.as_ref().unwrap();
        assert_eq!(m.model, Some(StringMatch::Substring("gpt-4".to_string())));
    }

    #[test]
    fn should_parse_catch_all_fixture() {
        let yaml = r#"
fixtures:
  - response:
      content: "default response"
"#;
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        assert!(f.match_rule.is_none());
    }

    // --- Validation tests ---

    #[test]
    fn should_reject_fixture_with_both_error_and_response() {
        let mut f = Fixture {
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                ..Default::default()
            }),
            error: Some(FixtureError {
                status: 500,
                message: "fail".to_string(),
                headers: HashMap::new(),
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mutually exclusive"));
    }

    #[test]
    fn should_reject_fixture_with_failure_but_no_response() {
        let mut f = Fixture {
            failure: Some(FailureConfig {
                latency_ms: Some(1000),
                ..Default::default()
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires response"));
    }

    #[test]
    fn should_reject_fixture_with_error_and_failure() {
        let mut f = Fixture {
            error: Some(FixtureError {
                status: 429,
                message: "rate limit".to_string(),
                headers: HashMap::new(),
            }),
            failure: Some(FailureConfig {
                latency_ms: Some(1000),
                ..Default::default()
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
    }

    #[test]
    fn should_reject_fixture_with_no_response_and_no_error() {
        let mut f = Fixture {
            match_rule: Some(FixtureMatch::default()),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must have either"));
    }

    #[test]
    fn should_reject_failure_probability_above_one() {
        let mut f = Fixture::new()
            .respond_with_content("ok")
            .with_failure(FailureConfig {
                probability: Some(2.0),
                ..Default::default()
            });
        let err = f.validate().unwrap_err();
        assert!(err.contains("probability must be in"), "got: {}", err);
    }

    #[test]
    fn should_reject_failure_probability_below_zero() {
        let mut f = Fixture::new()
            .respond_with_content("ok")
            .with_failure(FailureConfig {
                probability: Some(-0.5),
                ..Default::default()
            });
        let err = f.validate().unwrap_err();
        assert!(err.contains("probability must be in"), "got: {}", err);
    }

    #[test]
    fn should_reject_failure_probability_nan() {
        let mut f = Fixture::new()
            .respond_with_content("ok")
            .with_failure(FailureConfig {
                probability: Some(f32::NAN),
                ..Default::default()
            });
        let err = f.validate().unwrap_err();
        assert!(err.contains("probability must be in"), "got: {}", err);
    }

    #[test]
    fn should_reject_latency_jitter_without_streaming_latency() {
        // No streaming block at all.
        let mut f = Fixture::new()
            .respond_with_content("ok")
            .with_failure(FailureConfig {
                latency_jitter_ms: Some(5),
                ..Default::default()
            });
        let err = f.validate().unwrap_err();
        assert!(err.contains("latency_jitter_ms requires"), "got: {}", err);
    }

    #[test]
    fn should_reject_latency_jitter_with_zero_streaming_latency() {
        let mut f = Fixture::new()
            .respond_with_content("ok")
            .with_streaming(Some(0), Some(10))
            .with_failure(FailureConfig {
                latency_jitter_ms: Some(5),
                ..Default::default()
            });
        let err = f.validate().unwrap_err();
        assert!(err.contains("latency_jitter_ms requires"), "got: {}", err);
    }

    #[test]
    fn should_accept_zero_latency_jitter_without_streaming() {
        // `latency_jitter_ms: Some(0)` is a no-op — ChaosPlan collapses it
        // to None — so it should NOT require a base streaming.latency.
        let mut f = Fixture::new()
            .respond_with_content("ok")
            .with_failure(FailureConfig {
                latency_jitter_ms: Some(0),
                ..Default::default()
            });
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_reject_latency_jitter_above_one_hour_cap() {
        let mut f = Fixture::new()
            .respond_with_content("ok")
            .with_streaming(Some(10), Some(5))
            .with_failure(FailureConfig {
                // 1 hour = 3_600_000 ms — one more than the cap.
                latency_jitter_ms: Some(3_600_001),
                ..Default::default()
            });
        let err = f.validate().unwrap_err();
        assert!(err.contains("latency_jitter_ms must be <="), "got: {}", err);
    }

    #[test]
    fn should_accept_latency_jitter_at_one_hour_cap() {
        let mut f = Fixture::new()
            .respond_with_content("ok")
            .with_streaming(Some(10), Some(5))
            .with_failure(FailureConfig {
                latency_jitter_ms: Some(3_600_000),
                ..Default::default()
            });
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_accept_latency_jitter_with_positive_streaming_latency() {
        let mut f = Fixture::new()
            .respond_with_content("ok")
            .with_streaming(Some(10), Some(5))
            .with_failure(FailureConfig {
                latency_jitter_ms: Some(5),
                ..Default::default()
            });
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_accept_failure_probability_at_boundaries() {
        let mut f = Fixture::new()
            .respond_with_content("ok")
            .with_failure(FailureConfig {
                probability: Some(0.0),
                ..Default::default()
            });
        assert!(f.validate().is_ok());
        let mut f = Fixture::new()
            .respond_with_content("ok")
            .with_failure(FailureConfig {
                probability: Some(1.0),
                ..Default::default()
            });
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_accept_valid_error_fixture() {
        let mut f = Fixture::new().with_error(429, "rate limit");
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_accept_valid_response_fixture() {
        let mut f = Fixture::new().respond_with_content("hi");
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_reject_invalid_regex() {
        let mut f = Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::regex("[invalid")),
                model: None,
            }),
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("regex"));
    }

    // --- Matching tests ---

    #[test]
    fn should_match_substring_user_message() {
        let fixtures = vec![Fixture::new()
            .match_user_message("hello")
            .respond_with_content("hi")];
        let result = match_fixture(&fixtures, "say hello world", None, None, None);
        assert!(result.is_some());
    }

    #[test]
    fn should_not_match_wrong_substring() {
        let fixtures = vec![Fixture::new()
            .match_user_message("goodbye")
            .respond_with_content("bye")];
        let result = match_fixture(&fixtures, "say hello world", None, None, None);
        assert!(result.is_none());
    }

    #[test]
    fn should_match_regex_user_message() {
        let fixtures = vec![Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::regex("hello \\w+")),
                model: None,
            }),
            ..Fixture::new().respond_with_content("matched")
        }];
        let result = match_fixture(&fixtures, "hello world", None, None, None);
        assert!(result.is_some());
    }

    #[test]
    fn should_match_model() {
        let fixtures = vec![Fixture::new()
            .match_model("gpt-4")
            .respond_with_content("gpt4 response")];
        let result = match_fixture(&fixtures, "anything", Some("gpt-4-turbo"), None, None);
        assert!(result.is_some());
    }

    #[test]
    fn should_match_first_fixture_wins() {
        let fixtures = vec![
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("first"),
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("second"),
        ];
        let result = match_fixture(&fixtures, "hello", None, None, None);
        assert_eq!(
            result
                .unwrap()
                .response
                .as_ref()
                .unwrap()
                .content
                .as_deref(),
            Some("first")
        );
    }

    #[test]
    fn should_match_catch_all() {
        let fixtures = vec![Fixture::new().respond_with_content("default")];
        let result = match_fixture(&fixtures, "anything at all", None, None, None);
        assert!(result.is_some());
    }

    #[test]
    fn should_filter_by_provider() {
        let fixtures = vec![Fixture {
            provider: Some(crate::format::Provider::Anthropic),
            ..Fixture::new().respond_with_content("anthropic only")
        }];
        let result = match_fixture(
            &fixtures,
            "hello",
            None,
            Some(crate::format::Provider::Anthropic),
            None,
        );
        assert!(result.is_some());
        let result = match_fixture(
            &fixtures,
            "hello",
            None,
            Some(crate::format::Provider::OpenAI),
            None,
        );
        assert!(result.is_none());
    }

    // --- Builder API tests ---

    #[test]
    fn should_build_fixture_programmatically() {
        let mut f = Fixture::new()
            .match_user_message("hello")
            .respond_with_content("Hi there!");
        assert!(f.validate().is_ok());
        assert_eq!(
            f.response.as_ref().unwrap().content.as_deref(),
            Some("Hi there!")
        );
    }

    #[test]
    fn should_build_error_fixture_programmatically() {
        let mut f = Fixture::new()
            .match_model("fail-model")
            .with_error(429, "Rate limited");
        assert!(f.validate().is_ok());
        assert_eq!(f.error.as_ref().unwrap().status, 429);
    }

    #[test]
    fn should_use_default_trait_for_fixture() {
        let f = Fixture::default();
        assert!(f.response.is_none());
        assert!(f.error.is_none());
        assert!(f.match_rule.is_none());
    }

    #[test]
    fn should_compare_regex_match_by_pattern_string() {
        let a = RegexMatch {
            regex: "hello".to_string(),
            compiled: None,
        };
        let b = RegexMatch {
            regex: "hello".to_string(),
            compiled: None,
        };
        let c = RegexMatch {
            regex: "world".to_string(),
            compiled: None,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn should_reject_response_with_both_content_and_tool_calls() {
        let mut f = Fixture {
            response: Some(FixtureResponse {
                content: Some("text".to_string()),
                tool_calls: Some(vec![ToolCall {
                    name: "func".to_string(),
                    arguments: serde_json::json!({}),
                }]),
                ..Default::default()
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mutually exclusive"));
    }

    #[test]
    fn should_reject_response_with_neither_content_nor_tool_calls() {
        let mut f = Fixture {
            response: Some(FixtureResponse::default()),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must have either"));
    }

    #[test]
    fn should_reject_zero_chunk_size() {
        let mut f = Fixture {
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                ..Default::default()
            }),
            streaming: Some(StreamingConfig {
                latency: None,
                chunk_size: Some(0),
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("chunk_size must be > 0"));
    }

    #[test]
    fn should_compile_model_regex_on_validate() {
        let mut f = Fixture {
            match_rule: Some(FixtureMatch {
                user_message: None,
                model: Some(StringMatch::regex("gpt-4.*")),
            }),
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        };
        assert!(f.validate().is_ok());
        // After validation, the compiled regex should be used for matching
        let fixtures = vec![f];
        let result = match_fixture(&fixtures, "hello", Some("gpt-4-turbo"), None, None);
        assert!(result.is_some());
    }

    #[test]
    fn should_match_compiled_user_message_regex() {
        let mut f = Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::regex("he.*ld")),
                model: None,
            }),
            response: Some(FixtureResponse {
                content: Some("matched".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        };
        // Validate to compile the regex
        assert!(f.validate().is_ok());
        // Now match against it -- should use the compiled (Some) path
        let fixtures = vec![f];
        let result = match_fixture(&fixtures, "hello world", None, None, None);
        assert!(result.is_some());
    }

    #[test]
    fn should_reject_invalid_model_regex() {
        let mut f = Fixture {
            match_rule: Some(FixtureMatch {
                user_message: None,
                model: Some(StringMatch::regex("[invalid")),
            }),
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("model"));
    }

    #[test]
    fn should_not_match_model_when_no_model_provided() {
        let fixtures = vec![Fixture::new()
            .match_model("gpt-4")
            .respond_with_content("gpt4 only")];
        // model is None: should NOT match a fixture that requires a model
        let result = match_fixture(&fixtures, "hello", None, None, None);
        assert!(result.is_none());
    }

    #[test]
    fn should_use_regex_fallback_for_unvalidated_fixture() {
        // Fixture with valid regex that was NOT validated (no compile() called).
        // This exercises the fallback path in RegexMatch::is_match.
        let fixtures = vec![Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::regex("hel+o")),
                model: None,
            }),
            response: Some(FixtureResponse {
                content: Some("matched".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        }];
        let result = match_fixture(&fixtures, "helllo world", None, None, None);
        assert!(result.is_some());
    }

    // --- YAML file loading tests ---

    #[test]
    fn should_load_yaml_file() {
        let dir = std::env::temp_dir().join("llmposter_test_load");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.yaml");
        std::fs::write(
            &file,
            r#"
fixtures:
  - match:
      user_message: "test"
    response:
      content: "loaded from file"
"#,
        )
        .unwrap();
        let fixtures = load_yaml_file(&file).unwrap();
        assert_eq!(fixtures.len(), 1);
        assert_eq!(
            fixtures[0].response.as_ref().unwrap().content.as_deref(),
            Some("loaded from file")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn should_load_yaml_dir() {
        let dir = std::env::temp_dir().join("llmposter_test_dir");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.yaml"),
            "fixtures:\n  - match:\n      user_message: \"a\"\n    response:\n      content: \"a\"",
        )
        .unwrap();
        std::fs::write(
            dir.join("b.yml"),
            "fixtures:\n  - match:\n      user_message: \"b\"\n    response:\n      content: \"b\"",
        )
        .unwrap();
        std::fs::write(dir.join("not_yaml.txt"), "ignored").unwrap();
        std::fs::create_dir_all(dir.join("subdir")).unwrap();
        let fixtures = load_yaml_dir(&dir).unwrap();
        assert_eq!(fixtures.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn should_return_error_for_invalid_yaml_file() {
        let dir = std::env::temp_dir().join("llmposter_test_invalid");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bad.yaml");
        std::fs::write(&file, "not: [valid: {{{").unwrap();
        let result = load_yaml_file(&file);
        assert!(result.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn should_return_error_for_missing_file() {
        let result = load_yaml_file(Path::new("/nonexistent/file.yaml"));
        assert!(result.is_err());
    }

    #[test]
    fn should_validate_fixtures_on_load() {
        let dir = std::env::temp_dir().join("llmposter_test_validate_load");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("invalid_fixture.yaml");
        std::fs::write(
            &file,
            r#"
fixtures:
  - match:
      user_message: "test"
    response:
      content: "hi"
    error:
      status: 500
      message: "also error"
"#,
        )
        .unwrap();
        let result = load_yaml_file(&file);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("mutually exclusive"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn should_reject_oversized_regex_at_validation() {
        // A regex with a huge repetition count that would blow up the DFA
        let huge_pattern = format!("a{{{}}}", 999_999);
        let mut f = Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::regex(&huge_pattern)),
                model: None,
            }),
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err(), "oversized regex should be rejected");
    }

    #[test]
    fn should_return_false_for_oversized_regex_in_fallback() {
        let huge_pattern = format!("a{{{}}}", 999_999);
        let rm = RegexMatch {
            regex: huge_pattern,
            compiled: None, // No pre-compilation — exercises fallback path
        };
        // Should return false, not panic or OOM
        assert!(!rm.is_match("aaaa"));
    }

    #[test]
    fn should_reject_scalar_tool_call_arguments() {
        let mut f = Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "test".to_string(),
            arguments: serde_json::json!("not an object"),
        }]);
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be a JSON object"));
    }

    #[test]
    fn should_reject_array_tool_call_arguments() {
        let mut f = Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "test".to_string(),
            arguments: serde_json::json!([1, 2, 3]),
        }]);
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be a JSON object"));
    }

    #[test]
    fn should_accept_object_tool_call_arguments() {
        let mut f = Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "test".to_string(),
            arguments: serde_json::json!({"key": "value"}),
        }]);
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_reject_blank_tool_call_name() {
        let mut f = Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "".to_string(),
            arguments: serde_json::json!({"key": "value"}),
        }]);
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("name must not be empty"));
    }

    #[test]
    fn should_reject_whitespace_only_tool_call_name() {
        let mut f = Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "   ".to_string(),
            arguments: serde_json::json!({"key": "value"}),
        }]);
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("name must not be empty"));
    }

    #[test]
    fn should_reject_non_error_status_codes() {
        // 200, 301, etc. should be rejected — only 400-599 for error simulation
        for status in [200, 204, 301, 302] {
            let mut f = Fixture::new().with_error(status, "test");
            let result = f.validate();
            assert!(result.is_err(), "status {} should be rejected", status);
            assert!(result.unwrap_err().contains("400-599"));
        }
    }

    #[test]
    fn should_accept_error_status_codes() {
        for status in [400, 401, 403, 404, 429, 500, 502, 503, 529] {
            let mut f = Fixture::new().with_error(status, "test");
            assert!(f.validate().is_ok(), "status {} should be accepted", status);
        }
    }

    #[test]
    fn should_reject_empty_user_message_substring() {
        let mut f = Fixture::new()
            .match_user_message("")
            .respond_with_content("ok");
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
    }

    #[test]
    fn should_reject_empty_model_substring() {
        let mut f = Fixture::new().match_model("").respond_with_content("ok");
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
    }

    #[test]
    fn should_reject_empty_user_message_regex() {
        let mut f = Fixture::new().respond_with_content("ok");
        let m = f.match_rule.get_or_insert_with(FixtureMatch::default);
        m.user_message = Some(StringMatch::regex(""));
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("regex must not be empty"));
    }

    #[test]
    fn should_reject_empty_model_regex() {
        let mut f = Fixture::new().respond_with_content("ok");
        let m = f.match_rule.get_or_insert_with(FixtureMatch::default);
        m.model = Some(StringMatch::regex(""));
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("regex must not be empty"));
    }

    #[test]
    fn should_reject_unknown_yaml_fields() {
        let yaml =
            "fixtures:\n  - match:\n      user_mesage: typo\n    response:\n      content: ok";
        let result: Result<FixtureFile, _> = serde_yaml_ng::from_str(yaml);
        assert!(result.is_err(), "typo field 'user_mesage' must be rejected");
    }

    #[test]
    fn should_reject_unknown_fixture_fields() {
        let yaml = "fixtures:\n  - unknown_field: true\n    response:\n      content: ok";
        let result: Result<FixtureFile, _> = serde_yaml_ng::from_str(yaml);
        assert!(result.is_err(), "unknown fixture field must be rejected");
    }

    #[test]
    fn should_set_stop_reason_via_builder() {
        let f = Fixture::new()
            .respond_with_content("test")
            .with_stop_reason("max_tokens");
        assert_eq!(
            f.response.as_ref().unwrap().stop_reason.as_deref(),
            Some("max_tokens")
        );
    }

    #[test]
    fn should_set_finish_reason_via_builder() {
        let f = Fixture::new()
            .respond_with_content("test")
            .with_finish_reason("length");
        assert_eq!(
            f.response.as_ref().unwrap().finish_reason.as_deref(),
            Some("length")
        );
    }

    #[test]
    fn should_set_stop_reason_on_empty_response() {
        let f = Fixture::new().with_stop_reason("end_turn");
        assert!(f.response.is_some());
        assert_eq!(
            f.response.as_ref().unwrap().stop_reason.as_deref(),
            Some("end_turn")
        );
    }

    #[test]
    fn should_set_finish_reason_on_empty_response() {
        let f = Fixture::new().with_finish_reason("stop");
        assert!(f.response.is_some());
        assert_eq!(
            f.response.as_ref().unwrap().finish_reason.as_deref(),
            Some("stop")
        );
    }

    #[test]
    fn should_warn_but_accept_truncate_without_streaming_config() {
        // Warning is printed but validation still passes — no streaming is just a no-op.
        let mut f = Fixture {
            failure: Some(FailureConfig {
                truncate_after_frames: Some(2),
                ..Default::default()
            }),
            ..Fixture::new().respond_with_content("ok")
        };
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_warn_but_accept_disconnect_without_streaming_config() {
        let mut f = Fixture {
            failure: Some(FailureConfig {
                disconnect_after_ms: Some(100),
                ..Default::default()
            }),
            ..Fixture::new().respond_with_content("ok")
        };
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_warn_but_accept_duplicate_frames_without_streaming_config() {
        // Matches the sibling warnings above: duplicate_frames on a
        // non-streaming fixture is a no-op, validated-but-warned.
        let mut f = Fixture {
            failure: Some(FailureConfig {
                duplicate_frames: Some(true),
                ..Default::default()
            }),
            ..Fixture::new().respond_with_content("ok")
        };
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_accept_duplicate_frames_with_streaming_config() {
        // Happy path: duplicate_frames alongside streaming is fine.
        let mut f = Fixture::new()
            .respond_with_content("ok")
            .with_streaming(Some(5), Some(10))
            .with_failure(FailureConfig {
                duplicate_frames: Some(true),
                ..Default::default()
            });
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_warn_but_accept_truncate_on_tool_calls_fixture() {
        // After the fix, tool_calls fixtures also produce the warning.
        let mut f = Fixture {
            failure: Some(FailureConfig {
                truncate_after_frames: Some(2),
                ..Default::default()
            }),
            ..Fixture::new().respond_with_tool_calls(vec![ToolCall {
                name: "get_weather".to_string(),
                arguments: serde_json::json!({"location": "SF"}),
            }])
        };
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_skip_compile_when_already_compiled() {
        // Calling validate() twice must not error — compile() returns Ok early.
        let mut f = Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::Regex(RegexMatch {
                    regex: "hello \\w+".to_string(),
                    compiled: None,
                })),
                model: None,
            }),
            ..Fixture::new().respond_with_content("ok")
        };
        assert!(f.validate().is_ok());
        assert!(f.validate().is_ok()); // second call hits the early-return branch
    }

    #[test]
    fn should_reject_empty_tool_calls_vec() {
        let mut f = Fixture {
            response: Some(FixtureResponse {
                tool_calls: Some(vec![]),
                ..Default::default()
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
    }

    #[test]
    fn should_reject_number_tool_call_arguments() {
        let mut f = Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "test".to_string(),
            arguments: serde_json::json!(42),
        }]);
        let result = f.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("must be a JSON object, got number"));
    }

    #[test]
    fn should_reject_bool_tool_call_arguments() {
        let mut f = Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "test".to_string(),
            arguments: serde_json::json!(true),
        }]);
        let result = f.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("must be a JSON object, got boolean"));
    }

    #[test]
    fn should_reject_null_tool_call_arguments() {
        let mut f = Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "test".to_string(),
            arguments: serde_json::json!(null),
        }]);
        let result = f.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("must be a JSON object, got null"));
    }

    #[test]
    fn should_reject_duplicate_header_name_in_validate() {
        let mut f = Fixture {
            error: Some(FixtureError {
                status: 429,
                message: "rate limit".to_string(),
                headers: HashMap::from([
                    ("x-custom".to_string(), "a".to_string()),
                    ("X-Custom".to_string(), "b".to_string()),
                ]),
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("duplicate"));
    }

    #[test]
    fn should_reject_invalid_header_name_in_validate() {
        let mut f = Fixture {
            error: Some(FixtureError {
                status: 429,
                message: "rate limit".to_string(),
                headers: HashMap::from([("invalid name!".to_string(), "value".to_string())]),
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid error header name"));
    }

    #[test]
    fn should_reject_invalid_header_value_in_validate() {
        let mut f = Fixture {
            error: Some(FixtureError {
                status: 429,
                message: "rate limit".to_string(),
                headers: HashMap::from([("x-custom".to_string(), "\x00bad".to_string())]),
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid error header value"));
    }

    #[test]
    fn should_reject_streaming_config_on_error_fixture() {
        let mut f = Fixture {
            error: Some(FixtureError {
                status: 429,
                message: "rate limit".to_string(),
                headers: HashMap::new(),
            }),
            streaming: Some(StreamingConfig {
                latency: None,
                chunk_size: Some(10),
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no effect on error-only"));
    }
}
