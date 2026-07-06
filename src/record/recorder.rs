//! Recorder: resolved record-mode state shared by the server — upstream
//! HTTP client, cassette path, redactions, and the dedupe set.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use super::{
    append_entry, apply_redactions, ensure_cassette, fixture_from_entry, RecordedFixture, VcrMode,
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
    /// The state's recorder when it is configured with exactly `mode` —
    /// the shared gate for the handler hook sites in `handler/mod.rs`
    /// and `handler/embeddings.rs`.
    pub(crate) fn active(state: &crate::server::AppState, mode: VcrMode) -> Option<Arc<Recorder>> {
        state.recorder.as_ref().filter(|r| r.mode == mode).cloned()
    }

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

    /// Redact, validate, dedupe, append to the cassette, and splice into
    /// the live fixture set. Infallible from the caller's perspective —
    /// errors are logged to stderr, never bubbled into the client response.
    ///
    /// Order matters: the entry is serialized ONCE and round-trip
    /// validated BEFORE anything is claimed or written. An entry that
    /// would not reload (e.g. the empty prompt of a Responses API
    /// continuation request) is skipped entirely — no disk write (which
    /// would brick every later load of the cassette), no dedupe claim
    /// (a later valid variant of the same key must still record), no
    /// live splice.
    ///
    /// A failed cassette append is NOT retried within the run: the
    /// in-memory set is still updated so replay keeps working, and the
    /// entry is simply re-recorded on the next run.
    pub(crate) async fn persist(
        &self,
        mut rec: RecordedFixture,
        state: &Arc<crate::server::AppState>,
    ) {
        apply_redactions(&mut rec, &self.redactions);
        let validated = rec
            .to_yaml_entry()
            .and_then(|entry| fixture_from_entry(&entry).map(|fixture| (entry, fixture)));
        let (entry, fixture) = match validated {
            Ok(v) => v,
            Err(e) => {
                // Prompt-free context only — prompts can carry secrets.
                eprintln!(
                    "[llmposter] record: skipped unrecordable {} response \
                     (model='{}', cassette={}): {}",
                    rec.provider,
                    rec.match_rule.model,
                    self.cassette_path.display(),
                    e
                );
                return;
            }
        };
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
            if let Err(e) = append_entry(&self.cassette_path, &entry) {
                eprintln!("[llmposter] ERROR: failed to write cassette: {}", e);
            }
        }
        if let Err(e) = state.append_fixture(fixture) {
            eprintln!(
                "[llmposter] ERROR: failed to add recorded fixture to live set: {}",
                e
            );
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
/// `https://` scheme only, no embedded credentials, trailing `/`
/// trimmed. Plain HTTP to a non-loopback host gets a loud stderr
/// warning — real API keys would transit in cleartext.
///
/// No error branch ever echoes the raw URL: a rejected URL can embed
/// credentials (`ftp://user:secret@host`, unparseable userinfo forms),
/// and these messages end up in logs.
fn validate_proxy_url(flag: &str, url: Option<String>) -> Result<Option<String>, String> {
    let Some(url) = url else { return Ok(None) };
    let parsed = reqwest::Url::parse(&url).map_err(|_| format!("{}: invalid proxy URL", flag))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(format!(
            "{}: proxy URL must be http:// or https://, got scheme '{}'",
            flag,
            parsed.scheme()
        ));
    }
    // The stored base is echoed into 502 bodies and stderr logs, so a
    // credentialed URL would leak its secret to clients and logs.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(format!(
            "{}: proxy URL must not contain credentials (user:pass@...) — \
             upstream auth comes from the client's own forwarded headers",
            flag
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
/// `http://localhost:80@evil.com/` still surface the real host — defense
/// in depth only, since `validate_proxy_url` rejects credentialed URLs
/// before this runs.
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
        // The scheme branch names the offending SCHEME only — never the
        // URL, which can embed credentials.
        let err = validate_proxy_url(
            "proxy_gemini",
            Some("ftp://user:secret@example.com".to_string()),
        )
        .unwrap_err();
        assert!(err.contains("proxy_gemini"), "got: {}", err);
        assert!(err.contains("http"), "got: {}", err);
        assert!(err.contains("'ftp'"), "got: {}", err);
        assert!(
            !err.contains("secret") && !err.contains("example.com"),
            "scheme rejection must not echo the URL: {}",
            err
        );
    }

    #[test]
    fn should_reject_unparseable_proxy_url() {
        let err = validate_proxy_url("proxy_openai", Some("http://".to_string())).unwrap_err();
        assert!(err.contains("invalid proxy URL"), "got: {}", err);
        assert!(err.contains("proxy_openai"), "got: {}", err);
        // Unparseable-with-userinfo: the parse branch must not echo the
        // URL either — it can carry a credential.
        let err = validate_proxy_url("proxy_openai", Some("http://user:secret@[oops".to_string()))
            .unwrap_err();
        assert!(err.contains("invalid proxy URL"), "got: {}", err);
        assert!(
            !err.contains("secret"),
            "parse rejection must not echo the URL: {}",
            err
        );
    }

    #[test]
    fn should_reject_proxy_url_with_credentials() {
        // user:pass form — credentials must be rejected, not forwarded,
        // and the rejection must not echo the credential value.
        let err = validate_proxy_url(
            "proxy_openai",
            Some("http://user:secret@127.0.0.1:9999/".to_string()),
        )
        .unwrap_err();
        assert!(err.contains("credentials"), "got: {}", err);
        assert!(
            !err.contains("secret"),
            "rejection must not echo the credential: {}",
            err
        );
        // Username-only form is credentials too.
        let err = validate_proxy_url("proxy_gemini", Some("https://user@proxy.test/".to_string()))
            .unwrap_err();
        assert!(err.contains("credentials"), "got: {}", err);
    }

    #[test]
    fn should_flag_cleartext_host_despite_userinfo() {
        // Userinfo URLs are rejected outright by validate_proxy_url —
        // they never reach the cleartext warning.
        let err = validate_proxy_url(
            "proxy_openai",
            Some("http://localhost:80@evil.com/".to_string()),
        )
        .unwrap_err();
        assert!(err.contains("credentials"), "got: {}", err);
        // Defense in depth: cleartext_warning_host itself still surfaces
        // the REAL host of a userinfo URL, not the decoy before the '@'.
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
    async fn should_not_poison_cassette_or_claim_dedupe_on_invalid_entry() {
        // An empty user_message fails fixture validation (empty substring
        // matcher) — e.g. a /v1/responses continuation request. persist
        // must validate BEFORE touching anything: no disk write (which
        // would brick every later load of the cassette), no dedupe claim,
        // no live splice.
        let path = temp_cassette("persist_reject_invalid");
        let _ = std::fs::remove_file(&path);
        let (rec, _) = Recorder::build(RecorderConfig::default(), path.clone(), false).unwrap();
        let state = minimal_state();
        let pristine = std::fs::read_to_string(&path).unwrap();

        rec.persist(sample(""), &state).await;

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            pristine,
            "invalid entry must never reach the cassette file"
        );
        assert!(
            crate::fixture::load_yaml_file(&path).unwrap().is_empty(),
            "cassette must remain loadable and empty"
        );
        assert_eq!(live_fixture_count(&state), 0);
        assert!(
            rec.seen.lock().unwrap().is_empty(),
            "a rejected entry must not claim its dedupe key"
        );

        // Normal recording still works afterwards.
        rec.persist(sample("now valid"), &state).await;
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
    fn should_warn_but_accept_cleartext_proxy_to_non_loopback_host() {
        // Plain http:// to a non-loopback host is allowed (vLLM/Ollama on
        // a LAN box) — it only earns the loud stderr warning.
        let url = validate_proxy_url(
            "proxy_openai",
            Some("http://upstream.internal:8080/".to_string()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(url, "http://upstream.internal:8080");
    }

    #[tokio::test]
    async fn should_default_content_type_when_client_sent_none() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // Raw upstream that captures the request head and answers 200.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (head_tx, head_rx) = tokio::sync::oneshot::channel::<String>();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            // Deliberately tiny read buffer: the head spans many reads,
            // so the loop provably takes both its continue and break edges.
            let mut tmp = [0u8; 8];
            loop {
                let n = sock.read(&mut tmp).await.unwrap_or(0);
                buf.extend_from_slice(&tmp[..n]);
                // EOF or end of the request head — either way, respond.
                if n == 0 || buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = head_tx.send(String::from_utf8_lossy(&buf).to_lowercase());
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}")
                .await;
            let _ = sock.shutdown().await;
        });

        let (rec, _) = Recorder::build(
            RecorderConfig {
                proxy_openai: Some(format!("http://{}", addr)),
                ..Default::default()
            },
            temp_cassette("fwd_default_ct"),
            false,
        )
        .unwrap();
        // No headers at all — forward() must synthesize the JSON content-type.
        let resp = rec
            .forward(
                crate::format::Provider::OpenAI,
                "/v1/chat/completions",
                None,
                &std::collections::HashMap::new(),
                "{}".to_string(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let head = head_rx.await.unwrap();
        assert!(
            head.contains("content-type: application/json"),
            "default content-type applied: {}",
            head
        );
    }

    #[tokio::test]
    async fn should_log_and_skip_live_splice_of_unroundtrippable_fixture() {
        // Tool-call arguments that are not a JSON object fail fixture
        // validation on the round-trip — persist must not panic and must
        // not splice the broken fixture into the live set. (The extractors
        // never produce this shape; this is the defensive Err arm.)
        let path = temp_cassette("persist_invalid_args");
        let _ = std::fs::remove_file(&path);
        let (rec, _) = Recorder::build(RecorderConfig::default(), path.clone(), false).unwrap();
        let state = minimal_state();
        let mut bad = sample("bad args");
        bad.response.content = None;
        bad.response.tool_calls = Some(vec![super::super::RecordedToolCall {
            name: "f".to_string(),
            arguments: serde_json::json!(42),
        }]);
        rec.persist(bad, &state).await;
        assert_eq!(
            live_fixture_count(&state),
            0,
            "invalid round-trip must not reach the live set"
        );
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
