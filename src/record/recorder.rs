//! Recorder: resolved record-mode state shared by the server — upstream
//! HTTP client, cassette path, redactions, and the dedupe set.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use super::{
    append_to_cassette, apply_redactions, ensure_cassette, RecordedFixture, VcrMode,
    RECORDED_PRIORITY,
};

const OPENAI_UPSTREAM: &str = "https://api.openai.com";
const ANTHROPIC_UPSTREAM: &str = "https://api.anthropic.com";
const GEMINI_UPSTREAM: &str = "https://generativelanguage.googleapis.com";

/// Headers forwarded verbatim to the upstream. Everything else is dropped —
/// notably `host`, `content-length`, and any llmposter-local headers.
const FORWARD_HEADERS: &[&str] = &[
    "authorization",
    "x-api-key",
    "x-goog-api-key",
    "anthropic-version",
    "anthropic-beta",
    "openai-organization",
    "openai-project",
    "content-type",
];

/// Builder-side record settings, resolved into a `Recorder` in `build()`.
#[derive(Debug, Default, Clone)]
pub(crate) struct RecorderConfig {
    pub mode: VcrMode,
    pub record_file: Option<PathBuf>,
    pub proxy_openai: Option<String>,
    pub proxy_anthropic: Option<String>,
    pub proxy_gemini: Option<String>,
    pub redact_patterns: Vec<String>,
    pub allow_remote: bool,
}

/// Shared record-mode state. Built once in `ServerBuilder::build()` and
/// stored on `AppState`; handlers consult it to forward misses upstream
/// and persist the responses (Task 4).
pub(crate) struct Recorder {
    pub(crate) mode: VcrMode,
    pub(crate) cassette_path: PathBuf,
    redactions: Vec<regex::Regex>,
    proxy_openai: Option<String>,
    proxy_anthropic: Option<String>,
    proxy_gemini: Option<String>,
    client: reqwest::Client,
    /// Serializes cassette read-modify-write cycles across concurrent
    /// recordings. Held only around `append_to_cassette`.
    cassette_io: tokio::sync::Mutex<()>,
    /// (provider, model, user_message) triples already recorded — seeded
    /// from the cassette at build so repeat prompts are served across
    /// runs, not re-appended.
    seen: std::sync::Mutex<HashSet<(String, String, String)>>,
}

impl Recorder {
    /// Resolve a [`RecorderConfig`] into a ready `Recorder`: compile
    /// redactions, validate proxy URLs, load any existing cassette entries
    /// (unless a directory fixture source already scanned them in), and
    /// create the cassette file in its pristine state.
    pub(crate) fn build(
        config: RecorderConfig,
        cassette_path: PathBuf,
        cassette_preloaded_by_dir_scan: bool,
    ) -> Result<(Arc<Self>, Vec<crate::fixture::Fixture>), String> {
        let mut redactions = Vec::with_capacity(config.redact_patterns.len());
        for pattern in &config.redact_patterns {
            let re = regex::Regex::new(pattern)
                .map_err(|e| format!("invalid --redact pattern '{}': {}", pattern, e))?;
            redactions.push(re);
        }

        let proxy_openai = validate_proxy_url("proxy_openai", config.proxy_openai)?;
        let proxy_anthropic = validate_proxy_url("proxy_anthropic", config.proxy_anthropic)?;
        let proxy_gemini = validate_proxy_url("proxy_gemini", config.proxy_gemini)?;

        let existing = if cassette_preloaded_by_dir_scan {
            Vec::new()
        } else if cassette_path.exists() {
            crate::fixture::load_yaml_file(&cassette_path)
                .map_err(|e| format!("cassette {}: {}", cassette_path.display(), e))?
        } else {
            Vec::new()
        };

        ensure_cassette(&cassette_path)?;

        let client = reqwest::Client::builder()
            // Providers never redirect API POSTs; following one could re-send auth headers.
            .redirect(reqwest::redirect::Policy::none())
            // Bound connect time so a dead upstream fails fast; no overall timeout —
            // streamed recordings legitimately run long.
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| format!("failed to build record-mode HTTP client: {}", e))?;

        Ok((
            Arc::new(Self {
                mode: config.mode,
                cassette_path,
                redactions,
                proxy_openai,
                proxy_anthropic,
                proxy_gemini,
                client,
                cassette_io: tokio::sync::Mutex::new(()),
                seen: std::sync::Mutex::new(HashSet::new()),
            }),
            existing,
        ))
    }

    /// Upstream base URL for a provider — the proxy override if set, else
    /// the real provider endpoint. The Responses API shares the OpenAI base.
    pub(crate) fn upstream_base(&self, provider: crate::format::Provider) -> &str {
        use crate::format::Provider;
        match provider {
            Provider::OpenAI | Provider::Responses => {
                self.proxy_openai.as_deref().unwrap_or(OPENAI_UPSTREAM)
            }
            Provider::Anthropic => self
                .proxy_anthropic
                .as_deref()
                .unwrap_or(ANTHROPIC_UPSTREAM),
            Provider::Gemini => self.proxy_gemini.as_deref().unwrap_or(GEMINI_UPSTREAM),
        }
    }

    /// Forward a request body upstream with only the allowlisted headers
    /// ([`FORWARD_HEADERS`]) copied over. `content-type` defaults to
    /// `application/json` when the client didn't send one.
    pub(crate) async fn forward(
        &self,
        provider: crate::format::Provider,
        path: &str,
        query: Option<&str>,
        headers: &std::collections::HashMap<String, String>,
        body: String,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let base = self.upstream_base(provider);
        let url = match query {
            Some(q) => format!("{}{}?{}", base, path, q),
            None => format!("{}{}", base, path),
        };
        let mut req = self.client.post(&url);
        let mut has_content_type = false;
        for &name in FORWARD_HEADERS {
            if let Some(value) = headers.get(name) {
                if name == "content-type" {
                    has_content_type = true;
                }
                req = req.header(name, value);
            }
        }
        if !has_content_type {
            req = req.header("content-type", "application/json");
        }
        req.body(body).send().await
    }

    /// Redact, dedupe, append to the cassette, and splice into the live
    /// fixture set. Infallible from the caller's perspective — cassette or
    /// splice errors are logged to stderr, never bubbled into the client
    /// response. A failed cassette append is NOT retried within the run:
    /// the in-memory set is still updated so replay keeps working, and the
    /// entry is simply re-recorded on the next run.
    pub(crate) async fn persist(
        &self,
        mut rec: RecordedFixture,
        state: &Arc<crate::server::AppState>,
    ) {
        apply_redactions(&mut rec, &self.redactions);
        let key = (
            rec.provider.to_string(),
            rec.match_rule.model.clone(),
            rec.match_rule.user_message.clone(),
        );
        {
            let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            if !seen.insert(key) {
                return; // already recorded this (provider, model, prompt) triple
            }
        }
        {
            let _io = self.cassette_io.lock().await;
            if let Err(e) = append_to_cassette(&self.cassette_path, &rec) {
                eprintln!("[llmposter] ERROR: failed to write cassette: {}", e);
            }
        }
        // Double-failure caveat: if the cassette append above AND the
        // in-memory splice below both failed after the dedupe key was
        // claimed, this prompt would simply forward for the rest of the
        // run — theoretical only, since the splice round-trips a fixture
        // we just constructed and is structurally infallible.
        match rec.into_fixture() {
            Ok(fixture) => {
                if let Err(e) = state.append_fixture(fixture) {
                    eprintln!(
                        "[llmposter] ERROR: failed to add recorded fixture to live set: {}",
                        e
                    );
                }
            }
            Err(e) => eprintln!("[llmposter] ERROR: {}", e),
        }
    }

    /// Seed the dedupe set from already-recorded fixtures (the
    /// `priority: -1` marker) so rerunning record mode against the same
    /// cassette is append-idempotent for prompts recorded in earlier runs.
    /// Entries without a provider or with non-substring matchers (e.g.
    /// hand-edited regex entries) are skipped — they can't be keyed
    /// reliably.
    pub(crate) fn seed_dedupe(&self, fixtures: &[crate::fixture::Fixture]) {
        use crate::fixture::StringMatch;
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        for fixture in fixtures {
            if fixture.priority != Some(RECORDED_PRIORITY) {
                continue;
            }
            let (Some(provider), Some(rule)) = (fixture.provider, fixture.match_rule.as_ref())
            else {
                continue;
            };
            let (Some(StringMatch::Substring(user)), Some(StringMatch::Substring(model))) =
                (rule.user_message.as_ref(), rule.model.as_ref())
            else {
                continue;
            };
            seen.insert((provider.as_str().to_string(), model.clone(), user.clone()));
        }
    }
}

/// Validate a proxy override URL via full URL parsing: `http://` or
/// `https://` scheme only, trailing `/` trimmed. Plain HTTP to a
/// non-loopback host gets a loud stderr warning — real API keys would
/// transit in cleartext.
fn validate_proxy_url(flag: &str, url: Option<String>) -> Result<Option<String>, String> {
    let Some(url) = url else { return Ok(None) };
    let parsed = reqwest::Url::parse(&url)
        .map_err(|e| format!("{}: invalid proxy URL '{}': {}", flag, url, e))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(format!(
            "{}: proxy URL must be http:// or https://, got '{}'",
            flag, url
        ));
    }
    if let Some(host) = cleartext_warning_host(&parsed) {
        eprintln!(
            "[llmposter] WARNING: {} is plain http:// to non-loopback host '{}' — \
             real API keys will transit in CLEARTEXT to that host.",
            flag, host
        );
    }
    Ok(Some(url.trim_end_matches('/').to_string()))
}

/// Host to warn about: `Some(host)` when the URL is plain `http://` to a
/// non-loopback host. Uses the parsed URL's host, so userinfo tricks like
/// `http://localhost:80@evil.com/` still surface the real host.
fn cleartext_warning_host(url: &reqwest::Url) -> Option<&str> {
    if url.scheme() != "http" {
        return None;
    }
    url.host_str().filter(|host| !host_is_loopback(host))
}

fn host_is_loopback(host: &str) -> bool {
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// `true` when the bind address targets only the local machine.
/// Conservative: anything unparseable is treated as non-loopback, so
/// unknown hosts require the explicit `allow_remote_record` opt-in.
pub(crate) fn bind_is_loopback(addr: &str) -> bool {
    if let Ok(sock) = addr.parse::<std::net::SocketAddr>() {
        return sock.ip().is_loopback();
    }
    let host = match addr.rsplit_once(':') {
        Some((host, _port)) => host,
        None => addr,
    };
    host_is_loopback(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cassette(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("llmposter_record_tests");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{}_{}.yaml", name, std::process::id()))
    }

    fn sample(msg: &str) -> RecordedFixture {
        RecordedFixture {
            match_rule: super::super::RecordedMatch {
                user_message: msg.to_string(),
                model: "gpt-test".to_string(),
            },
            provider: "openai",
            priority: RECORDED_PRIORITY,
            response: super::super::RecordedResponse {
                content: Some("hi there".to_string()),
                ..Default::default()
            },
        }
    }

    fn minimal_state() -> Arc<crate::server::AppState> {
        Arc::new(crate::server::AppState {
            fixtures: std::sync::RwLock::new(crate::server::FixtureSet::default()),
            id_gen: crate::format::IdGenerator::new(),
            verbose: false,
            request_counter: Default::default(),
            chaos_counter: Default::default(),
            capture_counter: Default::default(),
            moderation_counter: Default::default(),
            auth: None,
            scenarios: Default::default(),
            captured_requests: Default::default(),
            capture_capacity: None,
            explicit_models: None,
            diagnostics: false,
            boot_instant: std::time::Instant::now(),
            boot_epoch_ms: 0,
            #[cfg(feature = "ui")]
            ui_tx: None,
            recorder: None,
        })
    }

    fn live_fixture_count(state: &Arc<crate::server::AppState>) -> usize {
        state.fixtures.read().unwrap().len()
    }

    #[test]
    fn should_classify_bind_addresses_as_loopback_or_not() {
        assert!(bind_is_loopback("127.0.0.1:0"));
        assert!(bind_is_loopback("[::1]:4000"));
        assert!(bind_is_loopback("localhost:2112"));
        assert!(bind_is_loopback("LOCALHOST:80"));
        assert!(bind_is_loopback("127.0.0.2:80"));
        assert!(!bind_is_loopback("0.0.0.0:0"));
        assert!(!bind_is_loopback("192.168.1.5:80"));
        assert!(!bind_is_loopback("example.com:80"));
        assert!(!bind_is_loopback("%%%"));
    }

    #[test]
    fn should_trim_trailing_slash_from_proxy_url() {
        let url = validate_proxy_url("proxy_openai", Some("https://proxy.test/".to_string()))
            .unwrap()
            .unwrap();
        assert_eq!(url, "https://proxy.test");
    }

    #[test]
    fn should_reject_non_http_proxy_scheme() {
        let err =
            validate_proxy_url("proxy_gemini", Some("ftp://example.com".to_string())).unwrap_err();
        assert!(err.contains("proxy_gemini"), "got: {}", err);
        assert!(err.contains("http"), "got: {}", err);
    }

    #[test]
    fn should_reject_unparseable_proxy_url() {
        let err = validate_proxy_url("proxy_openai", Some("http://".to_string())).unwrap_err();
        assert!(err.contains("invalid proxy URL"), "got: {}", err);
        assert!(err.contains("proxy_openai"), "got: {}", err);
    }

    #[test]
    fn should_flag_cleartext_host_despite_userinfo() {
        // Userinfo bypass: naive host parsing would see "localhost" here.
        let sneaky = reqwest::Url::parse("http://localhost:80@evil.com/").unwrap();
        assert_eq!(cleartext_warning_host(&sneaky), Some("evil.com"));
        let local = reqwest::Url::parse("http://127.0.0.1:9999/").unwrap();
        assert_eq!(cleartext_warning_host(&local), None);
        let v6_local = reqwest::Url::parse("http://[::1]:9999/").unwrap();
        assert_eq!(cleartext_warning_host(&v6_local), None);
        let https = reqwest::Url::parse("https://example.com/").unwrap();
        assert_eq!(cleartext_warning_host(&https), None);
    }

    #[test]
    fn should_pick_proxy_override_or_real_upstream_per_provider() {
        let (rec, _) = Recorder::build(
            RecorderConfig {
                proxy_openai: Some("http://127.0.0.1:9999".to_string()),
                ..Default::default()
            },
            temp_cassette("upstream_base"),
            false,
        )
        .unwrap();
        use crate::format::Provider;
        assert_eq!(rec.upstream_base(Provider::OpenAI), "http://127.0.0.1:9999");
        assert_eq!(
            rec.upstream_base(Provider::Responses),
            "http://127.0.0.1:9999"
        );
        assert_eq!(rec.upstream_base(Provider::Anthropic), ANTHROPIC_UPSTREAM);
        assert_eq!(rec.upstream_base(Provider::Gemini), GEMINI_UPSTREAM);
    }

    #[tokio::test]
    async fn should_persist_recorded_fixture_to_cassette_and_live_set() {
        let path = temp_cassette("persist_happy");
        let _ = std::fs::remove_file(&path);
        let (rec, existing) =
            Recorder::build(RecorderConfig::default(), path.clone(), false).unwrap();
        assert!(existing.is_empty());
        let state = minimal_state();
        rec.persist(sample("q one"), &state).await;
        assert_eq!(crate::fixture::load_yaml_file(&path).unwrap().len(), 1);
        assert_eq!(live_fixture_count(&state), 1);
    }

    #[tokio::test]
    async fn should_dedupe_identical_persist_calls() {
        let path = temp_cassette("persist_dedupe");
        let _ = std::fs::remove_file(&path);
        let (rec, _) = Recorder::build(RecorderConfig::default(), path.clone(), false).unwrap();
        let state = minimal_state();
        rec.persist(sample("same prompt"), &state).await;
        rec.persist(sample("same prompt"), &state).await;
        assert_eq!(crate::fixture::load_yaml_file(&path).unwrap().len(), 1);
        assert_eq!(live_fixture_count(&state), 1);
    }

    #[tokio::test]
    async fn should_survive_cassette_write_failure_and_still_update_memory() {
        // Point the cassette at a DIRECTORY: exists() satisfies
        // ensure_cassette, but append_to_cassette's read fails. persist
        // must not panic and must still splice the fixture in-memory.
        let dir = std::env::temp_dir()
            .join("llmposter_record_tests")
            .join(format!("persist_dir_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (rec, _) = Recorder::build(RecorderConfig::default(), dir, true).unwrap();
        let state = minimal_state();
        rec.persist(sample("doomed write"), &state).await;
        assert_eq!(live_fixture_count(&state), 1);
    }

    #[test]
    fn should_skip_unseedable_entries_when_seeding_dedupe() {
        let (rec, _) =
            Recorder::build(RecorderConfig::default(), temp_cassette("seed_skip"), false).unwrap();
        let yaml = r#"
- match:
    user_message: "clean"
    model: "m"
  provider: openai
  priority: -1
  response: { content: "seedable" }
- match:
    user_message: { regex: "re.*" }
    model: "m"
  provider: openai
  priority: -1
  response: { content: "regex matcher - skip" }
- match:
    user_message: "no provider"
    model: "m"
  priority: -1
  response: { content: "no provider - skip" }
- match:
    user_message: "hand-written"
    model: "m"
  provider: openai
  response: { content: "default priority - skip" }
"#;
        let fixtures: Vec<crate::fixture::Fixture> = serde_yaml_ng::from_str(yaml).unwrap();
        rec.seed_dedupe(&fixtures);
        let seen = rec.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "only the clean priority:-1 entry seeds");
        assert!(seen.contains(&("openai".to_string(), "m".to_string(), "clean".to_string())));
    }
}
