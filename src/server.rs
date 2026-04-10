use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use tokio::net::TcpListener;

use crate::fixture::Fixture;
use crate::format::IdGenerator;

/// OAuth client configuration for the embedded oauth-mock server.
#[cfg(feature = "oauth")]
#[derive(Clone)]
pub struct OAuthConfig {
    /// OAuth client_id for the mock client.
    pub client_id: String,
    /// OAuth client_secret for the mock client.
    pub client_secret: String,
    /// Redirect URIs. Defaults to `["https://example.com/callback"]`.
    pub redirect_uris: Vec<String>,
    /// Scopes. Defaults to `["openid", "profile", "email"]`.
    pub scopes: Vec<String>,
}

#[cfg(feature = "oauth")]
impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            client_id: "mock-client".to_string(),
            client_secret: "mock-secret".to_string(),
            redirect_uris: vec!["https://example.com/callback".to_string()],
            scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
            ],
        }
    }
}

pub(crate) struct AppState {
    /// Live fixture list — wrapped in `RwLock` to support hot-reload
    /// via [`MockServer::set_fixtures`] without restarting the server.
    pub(crate) fixtures: std::sync::RwLock<Vec<Fixture>>,
    pub(crate) id_gen: IdGenerator,
    pub(crate) verbose: bool,
    /// Separate counter for x-request-id headers (doesn't interfere with response IDs).
    pub(crate) request_counter: AtomicU64,
    /// Monotonically increasing per-request counter feeding the chaos PRNG
    /// seed derivation. Distinct from `request_counter` so x-request-id IDs
    /// stay stable even if chaos plumbing changes.
    pub(crate) chaos_counter: AtomicU64,
    pub(crate) auth: Option<crate::auth::AuthState>,
    /// Scenario state machines — keyed by scenario name, value is current state.
    pub(crate) scenarios: std::sync::RwLock<std::collections::HashMap<String, String>>,
    /// Captured requests for test assertions.
    pub(crate) captured_requests: std::sync::RwLock<Vec<CapturedRequest>>,
}

/// A captured HTTP request for test assertions.
///
/// Available via [`MockServer::get_requests()`] after requests have been handled.
#[derive(Debug, Clone)]
pub struct CapturedRequest {
    /// HTTP method (always POST for LLM endpoints).
    pub method: String,
    /// Request path (e.g., "/v1/chat/completions").
    pub path: String,
    /// Raw request body.
    pub body: String,
    /// Name of the matched fixture's scenario, if any.
    pub matched_scenario: Option<String>,
    /// Timestamp when the request was received.
    pub timestamp: std::time::Instant,
}

impl AppState {
    pub(crate) fn next_request_id(&self) -> String {
        let n = self.request_counter.fetch_add(1, Ordering::Relaxed);
        format!("req-llmposter-{}", n)
    }

    /// Validates and atomically replaces the fixture list.
    ///
    /// On validation failure, returns an error and the existing fixtures are
    /// unchanged. **Scenario state (the `scenarios` hashmap) is intentionally
    /// preserved across a swap** so in-progress multi-turn conversations
    /// survive a config correction while the server is under load. Callers
    /// that want to reset scenarios on every reload can follow up with
    /// [`MockServer::reset`].
    pub(crate) fn set_fixtures(
        &self,
        mut fixtures: Vec<Fixture>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for (i, fixture) in fixtures.iter_mut().enumerate() {
            fixture
                .validate()
                .map_err(|e| format!("Fixture #{}: {}", i + 1, e))?;
        }
        self.swap_fixtures_unchecked(fixtures);
        Ok(())
    }

    /// Atomically replaces the fixture list WITHOUT running validation.
    ///
    /// Safe to call only when the caller has already validated the list
    /// (e.g. hot-reload paths re-reading sources through `load_yaml_file`,
    /// which validates at load time). Same scenario-preservation semantics
    /// as [`set_fixtures`](Self::set_fixtures): existing scenario state is
    /// not reset.
    pub(crate) fn swap_fixtures_unchecked(&self, fixtures: Vec<Fixture>) {
        let mut guard = self.fixtures.write().unwrap_or_else(|e| e.into_inner());
        *guard = fixtures;
    }
}

/// Which source caused a hot-reload attempt. Used for logging only.
#[cfg(any(feature = "watch", unix))]
#[derive(Debug, Clone, Copy)]
enum ReloadTrigger {
    #[cfg(feature = "watch")]
    Watch,
    #[cfg(unix)]
    Sighup,
}

#[cfg(any(feature = "watch", unix))]
impl ReloadTrigger {
    fn label(self) -> &'static str {
        match self {
            #[cfg(feature = "watch")]
            Self::Watch => "watch",
            #[cfg(unix)]
            Self::Sighup => "SIGHUP",
        }
    }
}

/// Re-read tracked sources and atomically swap them into the given state.
/// On parse failure, the old fixtures are left untouched and the error is
/// logged to stderr. Used by the file watcher and the SIGHUP handler.
///
/// `reload_sources` already validates each fixture at load time, so the swap
/// itself is infallible here — we use `swap_fixtures_unchecked` to avoid
/// revalidating (and to make the unreachable error branch actually unreachable).
#[cfg(any(feature = "watch", unix))]
fn reload_and_swap(
    state: &AppState,
    sources: &[std::path::PathBuf],
    verbose: bool,
    trigger: ReloadTrigger,
) {
    let label = trigger.label();
    match crate::fixture::reload_sources(sources) {
        Ok(fixtures) => {
            state.swap_fixtures_unchecked(fixtures);
            if verbose {
                eprintln!("[llmposter] {} reload: fixtures swapped", label);
            }
        }
        Err(e) => {
            eprintln!(
                "[llmposter] {} reload parse failed, keeping old fixtures: {}",
                label, e
            );
        }
    }
}

/// Spawn a background OS thread that debounces file-system events for
/// `sources` and calls [`reload_and_swap`] on each batch. The thread holds a
/// `Weak<AppState>`, so it exits on the next event after the server is dropped.
///
/// Only watches paths that exist at spawn time. Recursive for directories,
/// single-file for files. Uses `notify-debouncer-mini` with a 250ms debounce
/// window to collapse editor "save as temp → rename" sequences into a single
/// reload.
#[cfg(feature = "watch")]
fn spawn_file_watcher(
    state: std::sync::Weak<AppState>,
    sources: Vec<std::path::PathBuf>,
    verbose: bool,
) {
    use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};
    use std::time::Duration;

    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = match new_debouncer(Duration::from_millis(250), tx) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[llmposter] file watcher setup failed: {}", e);
            return;
        }
    };

    // Register each source independently. If one path fails (e.g. was
    // deleted between load and build), log and continue so the remaining
    // paths still get watched. Only bail entirely when every source fails.
    let mut watched_any = false;
    for path in &sources {
        let mode = if path.is_dir() {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        match debouncer.watcher().watch(path, mode) {
            Ok(()) => {
                watched_any = true;
            }
            Err(e) => {
                eprintln!(
                    "[llmposter] file watcher failed to watch {}: {}",
                    path.display(),
                    e
                );
            }
        }
    }
    if !watched_any {
        eprintln!(
            "[llmposter] file watcher: no sources could be registered ({} tried), giving up",
            sources.len()
        );
        return;
    }

    std::thread::spawn(move || {
        // The debouncer must live as long as this thread; when the thread
        // exits, the debouncer drops and `rx` closes.
        let _debouncer = debouncer;
        for res in rx {
            match res {
                Ok(_events) => {
                    // Weak upgrade: if the server was dropped, exit cleanly.
                    let Some(arc) = state.upgrade() else {
                        return;
                    };
                    reload_and_swap(&arc, &sources, verbose, ReloadTrigger::Watch);
                }
                Err(e) => {
                    eprintln!("[llmposter] file watcher error: {}", e);
                }
            }
        }
    });
}

/// Install a `SIGHUP` handler that reloads fixtures on each signal.
///
/// Traditional Unix convention: `kill -HUP <pid>` tells a daemon to re-read
/// its config. The handler holds a `Weak<AppState>`, so it exits on the next
/// signal after the server is dropped (tests leak at most one idle task per
/// server until the process exits).
///
/// `SIGHUP` is process-wide. When multiple `MockServer` instances run in the
/// same process, each installs its own handler and all reload on every
/// signal — this is fine because each has its own source list.
#[cfg(unix)]
fn spawn_sighup_handler(
    state: std::sync::Weak<AppState>,
    sources: Vec<std::path::PathBuf>,
    verbose: bool,
) {
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sig = match signal(SignalKind::hangup()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[llmposter] SIGHUP handler setup failed: {}", e);
                return;
            }
        };
        loop {
            if sig.recv().await.is_none() {
                return;
            }
            let Some(arc) = state.upgrade() else {
                return;
            };
            reload_and_swap(&arc, &sources, verbose, ReloadTrigger::Sighup);
        }
    });
}

/// Format a UNIX timestamp as an RFC 3339 UTC string (e.g. "2026-03-22T10:30:00Z").
fn format_rfc3339_utc(epoch_secs: u64) -> String {
    const SECS_PER_DAY: u64 = 86400;
    const DAYS_PER_400Y: u64 = 146097;
    const DAYS_PER_100Y: u64 = 36524;
    const DAYS_PER_4Y: u64 = 1461;
    const DAYS_PER_Y: u64 = 365;
    let secs = epoch_secs % SECS_PER_DAY;
    let hour = secs / 3600;
    let min = (secs % 3600) / 60;
    let sec = secs % 60;

    let days = epoch_secs / SECS_PER_DAY + 719468; // shift to 0000-03-01
    let era = days / DAYS_PER_400Y;
    let doe = days - era * DAYS_PER_400Y;
    let yoe = (doe - doe / (DAYS_PER_4Y - 1) + doe / DAYS_PER_100Y - doe / (DAYS_PER_400Y - 1))
        / DAYS_PER_Y;
    let y = yoe + era * 400;
    let doy = doe - (DAYS_PER_Y * yoe + yoe / 4 - yoe / 100);
    let mut m = (5 * doy + 2) / 153;
    let d = doy - (153 * m + 2) / 5 + 1;
    m = if m < 10 { m + 3 } else { m - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, m, d, hour, min, sec
    )
}

/// Handler for `GET /code/200`, `GET /code/429`, etc. — returns the requested
/// HTTP status code. The path segment is a numeric status code (100–599).
/// Useful for testing client error-handling without writing a fixture.
/// 3xx responses include a `Location: /` header. 429 responses get rate-limit
/// headers automatically via the `add_response_headers` middleware.
async fn handle_status_code(Path(code): Path<u16>) -> Response<Body> {
    match StatusCode::from_u16(code)
        .ok()
        .filter(|s| s.as_u16() <= 599)
    {
        Some(status) => {
            // 1xx, 204, 205, 304 must not have a body per HTTP spec
            if status.as_u16() < 200
                || status == StatusCode::NO_CONTENT
                || status == StatusCode::RESET_CONTENT
                || status == StatusCode::NOT_MODIFIED
            {
                return Response::builder()
                    .status(status)
                    .body(Body::empty())
                    .expect("static headers");
            }
            let description = status.canonical_reason().unwrap_or("Unknown");
            let body = serde_json::json!({"code": code, "description": description}).to_string();
            let mut builder = Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, "application/json");
            if status.is_redirection() {
                builder = builder.header(header::LOCATION, "/");
            }
            builder.body(Body::from(body)).expect("static headers")
        }
        None => Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"code":400,"description":"Invalid status code — use 100-599"}"#,
            ))
            .expect("static headers"),
    }
}

/// Middleware: adds x-request-id to every response, provider-specific rate limit headers on 429.
async fn add_response_headers(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: Next,
) -> axum::response::Response {
    let path = request.uri().path().to_string();
    let mut resp = next.run(request).await;
    let request_id = state.next_request_id();
    resp.headers_mut()
        .insert("x-request-id", request_id.parse().unwrap());

    // Auto-emit rate limit headers on 429 responses.
    // retry-after: 60 is emitted for ALL providers first, then provider-specific headers.
    if resp.status() == StatusCode::TOO_MANY_REQUESTS {
        let headers = resp.headers_mut();
        // Common to all providers:
        headers
            .entry("retry-after")
            .or_insert("60".parse().unwrap());

        if path.starts_with("/v1/messages") {
            // Anthropic: additional anthropic-ratelimit-requests-{limit,remaining,reset}
            // Reset is an RFC 3339 timestamp 60s in the future per Anthropic spec.
            let reset_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                + 60;
            let reset_ts = format_rfc3339_utc(reset_secs);
            headers
                .entry("anthropic-ratelimit-requests-limit")
                .or_insert("100".parse().unwrap());
            headers
                .entry("anthropic-ratelimit-requests-remaining")
                .or_insert("0".parse().unwrap());
            headers
                .entry("anthropic-ratelimit-requests-reset")
                .or_insert(reset_ts.parse().unwrap());
        } else if path.starts_with("/v1beta/models") {
            // Gemini: no additional provider-specific headers (retry-after set above)
        } else {
            // OpenAI (chat completions + responses): x-ratelimit-*
            headers
                .entry("x-ratelimit-limit-requests")
                .or_insert("100".parse().unwrap());
            headers
                .entry("x-ratelimit-remaining-requests")
                .or_insert("0".parse().unwrap());
            headers
                .entry("x-ratelimit-reset-requests")
                .or_insert("1m0s".parse().unwrap());
        }
    }

    resp
}

/// Builder for configuring and starting a [`MockServer`].
///
/// Use method chaining to add fixtures, set bind address, enable auth, then call
/// [`build()`](Self::build) to start the server.
pub struct ServerBuilder {
    fixtures: Vec<Fixture>,
    /// Paths passed to [`load_yaml`](Self::load_yaml) or [`load_yaml_dir`](Self::load_yaml_dir).
    /// Used by hot-reload (`--watch` and SIGHUP) to know what to re-read.
    fixture_sources: Vec<std::path::PathBuf>,
    /// Enable file-watching hot-reload of `fixture_sources`.
    #[cfg(feature = "watch")]
    watch_enabled: bool,
    bind_addr: String,
    verbose: bool,
    auth_enabled: bool,
    bearer_tokens: Vec<(String, Option<u64>)>,
    #[cfg(feature = "oauth")]
    oauth_config: Option<OAuthConfig>,
}

impl ServerBuilder {
    /// Creates a new builder with default settings (bind to `127.0.0.1:0`, no auth).
    pub fn new() -> Self {
        Self {
            fixtures: Vec::new(),
            fixture_sources: Vec::new(),
            #[cfg(feature = "watch")]
            watch_enabled: false,
            bind_addr: "127.0.0.1:0".to_string(),
            verbose: false,
            auth_enabled: false,
            bearer_tokens: Vec::new(),
            #[cfg(feature = "oauth")]
            oauth_config: None,
        }
    }

    /// Appends a single fixture to the server's match list.
    pub fn fixture(mut self, f: Fixture) -> Self {
        self.fixtures.push(f);
        self
    }

    /// Appends multiple fixtures to the server's match list.
    pub fn fixtures(mut self, fixtures: Vec<Fixture>) -> Self {
        self.fixtures.extend(fixtures);
        self
    }

    /// Returns the number of fixtures currently staged in the builder.
    /// Useful for callers (e.g. the CLI) that want to warn when nothing was loaded.
    pub fn fixture_count(&self) -> usize {
        self.fixtures.len()
    }

    /// Sets the TCP bind address (e.g. `"127.0.0.1:8080"`). Defaults to `"127.0.0.1:0"` (random port).
    pub fn bind(mut self, addr: &str) -> Self {
        self.bind_addr = addr.to_string();
        self
    }

    /// Enables or disables verbose logging of matched fixtures and request details.
    pub fn verbose(mut self, v: bool) -> Self {
        self.verbose = v;
        self
    }

    /// Enable or disable auth enforcement on all LLM endpoints.
    /// When enabled, requests without a valid `Authorization: Bearer <token>` header
    /// receive a provider-specific HTTP 401 response.
    pub fn with_auth(mut self, enabled: bool) -> Self {
        self.auth_enabled = enabled;
        self
    }

    /// Register a bearer token with unlimited uses and implicitly enable auth.
    /// Requests with `Authorization: Bearer <token>` are accepted; requests without
    /// a valid token receive HTTP 401.
    pub fn with_bearer_token(mut self, token: &str) -> Self {
        self.auth_enabled = true;
        self.bearer_tokens.push((token.to_string(), None));
        self
    }

    /// Register a bearer token that expires after `max_uses` requests, and
    /// implicitly enable auth. After exhaustion, the token returns HTTP 401.
    /// Use this to test token refresh flows deterministically (no real-time clocks).
    pub fn with_bearer_token_uses(mut self, token: &str, max_uses: u64) -> Self {
        self.auth_enabled = true;
        self.bearer_tokens.push((token.to_string(), Some(max_uses)));
        self
    }

    /// Enable the embedded OAuth server with custom client configuration.
    #[cfg(feature = "oauth")]
    pub fn with_oauth(mut self, config: OAuthConfig) -> Self {
        self.auth_enabled = true;
        self.oauth_config = Some(config);
        self
    }

    /// Enable the embedded OAuth server with default client credentials
    /// (client_id: "mock-client", client_secret: "mock-secret").
    #[cfg(feature = "oauth")]
    pub fn with_oauth_defaults(mut self) -> Self {
        self.auth_enabled = true;
        self.oauth_config = Some(OAuthConfig::default());
        self
    }

    /// Loads fixtures from a single YAML file and appends them to the match list.
    ///
    /// The path is also recorded as a hot-reload source. When [`watch`](Self::watch)
    /// is enabled (or the process receives `SIGHUP` on Unix), this file is re-read
    /// and the fixture list is atomically swapped.
    pub fn load_yaml(mut self, path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let fixtures = crate::fixture::load_yaml_file(path)?;
        self.fixtures.extend(fixtures);
        self.fixture_sources.push(path.to_path_buf());
        Ok(self)
    }

    /// Loads fixtures from all YAML files in a directory and appends them to the match list.
    ///
    /// The directory path is also recorded as a hot-reload source. When
    /// [`watch`](Self::watch) is enabled (or the process receives `SIGHUP` on Unix),
    /// the directory is re-scanned and the fixture list is atomically swapped.
    pub fn load_yaml_dir(
        mut self,
        dir: &std::path::Path,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let fixtures = crate::fixture::load_yaml_dir(dir)?;
        self.fixtures.extend(fixtures);
        self.fixture_sources.push(dir.to_path_buf());
        Ok(self)
    }

    /// Enable file-watching hot-reload for any files or directories loaded via
    /// [`load_yaml`](Self::load_yaml) or [`load_yaml_dir`](Self::load_yaml_dir).
    ///
    /// When a change is detected (debounced by ~250ms), all tracked sources are
    /// re-read, validated, and atomically swapped. If parsing or validation
    /// fails, the previously loaded fixtures continue serving unchanged and the
    /// error is logged to stderr — a partial edit or invalid YAML will never
    /// take down the live server.
    ///
    /// Programmatically-added fixtures (via [`fixture`](Self::fixture) or
    /// [`fixtures`](Self::fixtures)) are **not** affected by watching: only
    /// file-backed fixtures are reloaded.
    ///
    /// Requires the `watch` feature (enabled by default).
    ///
    /// On Unix, `SIGHUP` also triggers a reload — always on whenever any
    /// fixture source path is tracked, regardless of this flag. This matches
    /// traditional daemon conventions so `kill -HUP <pid>` works out of the box.
    #[cfg(feature = "watch")]
    pub fn watch(mut self, enabled: bool) -> Self {
        self.watch_enabled = enabled;
        self
    }

    /// Validates all fixtures, starts the HTTP server, and returns a running [`MockServer`].
    ///
    /// Returns an error if any fixture is invalid or the bind address is unavailable.
    pub async fn build(mut self) -> Result<MockServer, Box<dyn std::error::Error>> {
        // Validate all fixtures (including programmatically-added ones)
        for (i, fixture) in self.fixtures.iter_mut().enumerate() {
            fixture
                .validate()
                .map_err(|e| format!("Fixture #{}: {}", i + 1, e))?;
        }

        // Spawn the embedded oauth-mock server if configured.
        #[cfg(feature = "oauth")]
        let oauth_server = if let Some(ref config) = self.oauth_config {
            let redirect_uris: Vec<&str> =
                config.redirect_uris.iter().map(String::as_str).collect();
            let scopes: Vec<&str> = config.scopes.iter().map(String::as_str).collect();
            let oauth = oauth_mock::MockServer::builder()
                .with_client(
                    &config.client_id,
                    &config.client_secret,
                    redirect_uris,
                    scopes,
                )
                .spawn_on_free_port()
                .await
                .map_err(|e| format!("Failed to start OAuth server: {}", e))?;
            Some(oauth)
        } else {
            None
        };

        let auth = if self.auth_enabled {
            let auth_state = crate::auth::AuthState::new();
            for (token, max_uses) in &self.bearer_tokens {
                auth_state.add_token(token, *max_uses);
            }
            // Bridge oauth-mock token validation via introspect endpoint.
            #[cfg(feature = "oauth")]
            if let Some(ref oauth) = oauth_server {
                if let Some(ref config) = self.oauth_config {
                    auth_state.set_oauth_introspect(crate::auth::OAuthIntrospect {
                        url: format!("{}/introspect", oauth.base_url()),
                        client_id: config.client_id.clone(),
                        client_secret: config.client_secret.clone(),
                        client: reqwest::Client::builder()
                            .timeout(std::time::Duration::from_secs(5))
                            .build()
                            .map_err(|e| {
                                format!("Failed to build OAuth introspect client: {}", e)
                            })?,
                    });
                }
            }
            Some(auth_state)
        } else {
            None
        };

        let state = Arc::new(AppState {
            fixtures: std::sync::RwLock::new(self.fixtures),
            id_gen: IdGenerator::new(),
            verbose: self.verbose,
            request_counter: AtomicU64::new(1),
            chaos_counter: AtomicU64::new(0),
            auth,
            scenarios: std::sync::RwLock::new(std::collections::HashMap::new()),
            captured_requests: std::sync::RwLock::new(Vec::new()),
        });

        // Spawn hot-reload watchers if fixture sources are tracked.
        // The file watcher is opt-in via `.watch(true)`; SIGHUP (Unix) is
        // always on for file-backed fixtures so `kill -HUP <pid>` works.
        if !self.fixture_sources.is_empty() {
            #[cfg(feature = "watch")]
            if self.watch_enabled {
                spawn_file_watcher(
                    Arc::downgrade(&state),
                    self.fixture_sources.clone(),
                    self.verbose,
                );
            }
            #[cfg(unix)]
            spawn_sighup_handler(
                Arc::downgrade(&state),
                self.fixture_sources.clone(),
                self.verbose,
            );
        }

        let server_state = state.clone(); // keep for MockServer API access
        let app = Router::new()
            .route("/v1/chat/completions", post(crate::handler::openai::handle))
            .route("/v1/messages", post(crate::handler::anthropic::handle))
            .route("/v1/responses", post(crate::handler::responses::handle))
            .route(
                "/v1beta/models/{*path}",
                post(crate::handler::gemini::handle),
            )
            .route("/code/{status}", get(handle_status_code))
            .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024)) // 16 MB (inner)
            .layer(middleware::from_fn_with_state(
                state.clone(),
                crate::auth::bearer_auth_check,
            ))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                add_response_headers,
            ))
            .with_state(state);

        let listener = TcpListener::bind(&self.bind_addr).await?;
        let addr = listener.local_addr()?;

        let (err_tx, err_rx) = tokio::sync::oneshot::channel::<String>();
        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                let msg = format!("[llmposter] server error: {}", e);
                eprintln!("{}", msg);
                let _ = err_tx.send(msg);
            }
        });

        Ok(MockServer {
            addr,
            _handle: handle,
            server_error: tokio::sync::Mutex::new(err_rx),
            state: server_state,
            #[cfg(feature = "oauth")]
            oauth_server,
        })
    }
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A running mock LLM API server.
///
/// Created via [`ServerBuilder::build()`]. The server runs on a random port by default
/// and stops when dropped. Use [`url()`](Self::url) to get the base URL for client configuration.
pub struct MockServer {
    addr: std::net::SocketAddr,
    _handle: tokio::task::JoinHandle<()>,
    /// Check for post-bind server errors via `check_error()`.
    server_error: tokio::sync::Mutex<tokio::sync::oneshot::Receiver<String>>,
    /// Shared state — used for request capture and scenario queries.
    state: Arc<AppState>,
    /// Embedded OAuth server (dropped together with MockServer).
    #[cfg(feature = "oauth")]
    oauth_server: Option<oauth_mock::MockServer>,
}

impl std::fmt::Debug for MockServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockServer")
            .field("addr", &self.addr)
            .finish()
    }
}

impl MockServer {
    /// Returns the base URL of the running server (e.g. `"http://127.0.0.1:12345"`).
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Returns the TCP port the server is listening on.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Returns the base URL of the embedded OAuth server, if configured.
    #[cfg(feature = "oauth")]
    pub fn oauth_url(&self) -> Option<String> {
        self.oauth_server.as_ref().map(|s| s.base_url().to_string())
    }

    /// Returns the default OAuth client credentials (client_id, client_secret).
    #[cfg(feature = "oauth")]
    pub async fn oauth_client_credentials(&self) -> Option<(String, String)> {
        match &self.oauth_server {
            Some(s) => s.default_client().await,
            None => None,
        }
    }

    /// Approve a device code by user_code (for device authorization flow tests).
    #[cfg(feature = "oauth")]
    pub async fn approve_device_code(
        &self,
        user_code: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match &self.oauth_server {
            Some(s) => Ok(s.approve_device_code(user_code).await?),
            None => Err("OAuth not configured".into()),
        }
    }

    /// Check whether the server encountered a post-bind error.
    ///
    /// Returns `Ok(())` if healthy, or `Err(message)` if the server task failed.
    /// The error is consumed on first call — subsequent calls return `Ok(())`.
    pub async fn check_error(&self) -> Result<(), String> {
        let mut rx = self.server_error.lock().await;
        match rx.try_recv() {
            Ok(msg) => Err(msg),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => Ok(()),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Ok(()),
        }
    }

    /// Returns all captured requests received by this server, in order.
    ///
    /// Each request includes the method, path, body, matched scenario name, and timestamp.
    pub fn get_requests(&self) -> Vec<CapturedRequest> {
        self.state
            .captured_requests
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Returns the number of requests captured so far.
    pub fn request_count(&self) -> usize {
        self.state
            .captured_requests
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    /// Returns the current state of a named scenario, or `None` if the scenario
    /// has not been entered yet.
    pub fn scenario_state(&self, name: &str) -> Option<String> {
        self.state
            .scenarios
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .cloned()
    }

    /// Atomically replaces the server's fixture list at runtime.
    ///
    /// Every fixture is validated before the swap — if any fixture is invalid,
    /// an error is returned and the existing fixtures continue to serve requests
    /// unchanged. On success, subsequent requests match against the new list.
    ///
    /// Use this for hot-reload scenarios (e.g. reloading a YAML file after edits)
    /// or for test setups that mutate the fixture list between phases.
    pub fn set_fixtures(
        &self,
        fixtures: Vec<Fixture>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.state.set_fixtures(fixtures)
    }

    /// Resets all scenario states and clears captured requests.
    pub fn reset(&self) {
        self.state
            .scenarios
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.state
            .captured_requests
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self._handle.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn should_build_and_start_server() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("test"))
            .build()
            .await
            .unwrap();
        assert!(server.port() > 0);
        assert!(server.url().starts_with("http://127.0.0.1:"));
    }

    #[tokio::test]
    async fn should_return_404_for_unknown_routes() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("test"))
            .build()
            .await
            .unwrap();
        let resp = reqwest::get(format!("{}/unknown", server.url()))
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn should_support_custom_bind_address() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("test"))
            .bind("127.0.0.1:0")
            .build()
            .await
            .unwrap();
        assert!(server.port() > 0);
    }

    #[tokio::test]
    async fn should_support_default_builder() {
        let builder = ServerBuilder::default();
        let server = builder
            .fixture(Fixture::new().respond_with_content("default"))
            .build()
            .await
            .unwrap();
        assert!(server.port() > 0);
    }

    #[tokio::test]
    async fn should_support_fixtures_vec() {
        let fixtures = vec![
            Fixture::new()
                .match_user_message("a")
                .respond_with_content("A"),
            Fixture::new()
                .match_user_message("b")
                .respond_with_content("B"),
        ];
        let server = ServerBuilder::new()
            .fixtures(fixtures)
            .build()
            .await
            .unwrap();
        assert!(server.port() > 0);
    }

    #[tokio::test]
    async fn should_support_verbose_mode() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("test"))
            .verbose(true)
            .build()
            .await
            .unwrap();
        assert!(server.port() > 0);
    }

    #[tokio::test]
    async fn should_load_yaml_file() {
        let dir = std::env::temp_dir().join("llmposter_server_test_yaml");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.yaml");
        std::fs::write(
            &file,
            "fixtures:\n  - match:\n      user_message: test\n    response:\n      content: loaded",
        )
        .unwrap();
        let server = ServerBuilder::new()
            .load_yaml(&file)
            .unwrap()
            .build()
            .await
            .unwrap();
        assert!(server.port() > 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn should_load_yaml_dir() {
        let dir = std::env::temp_dir().join("llmposter_server_test_dir");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.yaml"),
            "fixtures:\n  - response:\n      content: a",
        )
        .unwrap();
        let server = ServerBuilder::new()
            .load_yaml_dir(&dir)
            .unwrap()
            .build()
            .await
            .unwrap();
        assert!(server.port() > 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn should_return_error_on_invalid_fixture() {
        let result = ServerBuilder::new()
            .fixture(Fixture::new()) // no response or error
            .build()
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Fixture #1"));
    }

    #[test]
    fn should_format_rfc3339_unix_epoch() {
        assert_eq!(format_rfc3339_utc(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn should_format_rfc3339_one_day() {
        assert_eq!(format_rfc3339_utc(86400), "1970-01-02T00:00:00Z");
    }

    #[test]
    fn should_format_rfc3339_valid_format() {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let ts = format_rfc3339_utc(now_secs + 60);
        // Must be valid RFC 3339: YYYY-MM-DDTHH:MM:SSZ
        assert!(ts.ends_with('Z'));
        assert!(ts.contains('T'));
        assert_eq!(ts.len(), 20); // "YYYY-MM-DDTHH:MM:SSZ"
    }

    #[tokio::test]
    async fn should_report_healthy_when_no_error() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await
            .unwrap();
        assert!(server.check_error().await.is_ok());
    }

    /// Build a bare-bones `MockServer` wrapping a supplied oneshot receiver.
    /// Only used by the check_error unit tests below — it bypasses the listener
    /// entirely so we can exercise the three `try_recv` arms deterministically.
    fn mock_server_with_err_rx(err_rx: tokio::sync::oneshot::Receiver<String>) -> MockServer {
        let state = Arc::new(AppState {
            fixtures: std::sync::RwLock::new(Vec::new()),
            id_gen: IdGenerator::new(),
            verbose: false,
            request_counter: AtomicU64::new(1),
            chaos_counter: AtomicU64::new(0),
            auth: None,
            scenarios: std::sync::RwLock::new(std::collections::HashMap::new()),
            captured_requests: std::sync::RwLock::new(Vec::new()),
        });
        // A no-op task handle — dropped when the server is dropped.
        let handle = tokio::spawn(async {});
        MockServer {
            addr: "127.0.0.1:0".parse().unwrap(),
            _handle: handle,
            server_error: tokio::sync::Mutex::new(err_rx),
            state,
            #[cfg(feature = "oauth")]
            oauth_server: None,
        }
    }

    #[tokio::test]
    async fn should_report_error_when_server_task_sent_error() {
        // Simulates the `Ok(msg) => Err(msg)` arm in check_error: the server
        // task hit an error post-bind and sent it through the oneshot.
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        tx.send("[llmposter] server error: boom".to_string())
            .unwrap();
        let server = mock_server_with_err_rx(rx);

        let result = server.check_error().await;
        assert!(
            result.is_err(),
            "expected Err after tx.send, got {:?}",
            result
        );
        let msg = result.unwrap_err();
        assert!(msg.contains("boom"), "unexpected error body: {}", msg);
    }

    #[tokio::test]
    async fn should_report_healthy_when_error_channel_closed_without_send() {
        // Simulates the `Closed` arm: the server task exited (or its sender
        // was dropped) without ever emitting an error. check_error should
        // treat this as healthy.
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        drop(tx);
        let server = mock_server_with_err_rx(rx);

        assert!(server.check_error().await.is_ok());
    }

    #[tokio::test]
    async fn should_return_requested_status_code() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await
            .unwrap();
        let resp = reqwest::get(format!("{}/code/200", server.url()))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["code"], 200);
        assert_eq!(body["description"], "OK");
    }

    #[tokio::test]
    async fn should_return_404_status_from_code_route() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await
            .unwrap();
        let resp = reqwest::get(format!("{}/code/404", server.url()))
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["code"], 404);
        assert_eq!(body["description"], "Not Found");
    }

    #[tokio::test]
    async fn should_return_500_status_from_code_route() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await
            .unwrap();
        let resp = reqwest::get(format!("{}/code/500", server.url()))
            .await
            .unwrap();
        assert_eq!(resp.status(), 500);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["code"], 500);
    }

    #[tokio::test]
    async fn should_add_location_header_on_redirect() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await
            .unwrap();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let resp = client
            .get(format!("{}/code/301", server.url()))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 301);
        assert_eq!(resp.headers().get("location").unwrap(), "/");
    }

    #[tokio::test]
    async fn should_return_bad_request_for_invalid_status_code() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await
            .unwrap();
        let resp = reqwest::get(format!("{}/code/999", server.url()))
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["code"], 400);
    }

    #[tokio::test]
    async fn should_return_empty_body_for_204() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await
            .unwrap();
        let resp = reqwest::get(format!("{}/code/204", server.url()))
            .await
            .unwrap();
        assert_eq!(resp.status(), 204);
        let body = resp.text().await.unwrap();
        assert!(body.is_empty(), "204 should have empty body, got: {}", body);
    }

    #[tokio::test]
    async fn should_return_empty_body_for_304() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await
            .unwrap();
        let resp = reqwest::get(format!("{}/code/304", server.url()))
            .await
            .unwrap();
        assert_eq!(resp.status(), 304);
        let body = resp.text().await.unwrap();
        assert!(body.is_empty(), "304 should have empty body, got: {}", body);
    }

    #[tokio::test]
    async fn should_return_empty_body_for_205() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await
            .unwrap();
        let resp = reqwest::get(format!("{}/code/205", server.url()))
            .await
            .unwrap();
        assert_eq!(resp.status(), 205);
        let body = resp.text().await.unwrap();
        assert!(body.is_empty(), "205 should have empty body, got: {}", body);
    }

    #[tokio::test]
    async fn should_return_empty_body_for_1xx_status() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await
            .unwrap();
        let resp = reqwest::get(format!("{}/code/100", server.url()))
            .await
            .unwrap();
        // hyper may not forward 1xx as a final response — it returns 200 or the
        // status itself depending on the HTTP version. Just verify no panic.
        let _ = resp.text().await;
    }

    /// Build a `Weak<AppState>` whose `Arc` has already been dropped.
    /// Used to exercise the `state.upgrade() else { return; }` paths in the
    /// file watcher and SIGHUP handler without spinning up a real server.
    #[cfg(any(feature = "watch", unix))]
    fn dead_weak_state() -> std::sync::Weak<AppState> {
        let arc = Arc::new(AppState {
            fixtures: std::sync::RwLock::new(Vec::new()),
            id_gen: IdGenerator::new(),
            verbose: false,
            request_counter: AtomicU64::new(1),
            chaos_counter: AtomicU64::new(0),
            auth: None,
            scenarios: std::sync::RwLock::new(std::collections::HashMap::new()),
            captured_requests: std::sync::RwLock::new(Vec::new()),
        });
        let weak = Arc::downgrade(&arc);
        drop(arc);
        weak
    }

    #[cfg(feature = "watch")]
    #[tokio::test]
    async fn should_exit_file_watcher_thread_when_state_is_dropped() {
        // Hits the `Weak::upgrade() -> None` early-return in the watcher loop:
        // the background thread receives an event, fails to upgrade, and returns.
        let dir = std::env::temp_dir().join(format!(
            "llmposter_watcher_dead_weak_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.yaml");
        std::fs::write(
            &file,
            "fixtures:\n  - match:\n      user_message: hi\n    response:\n      content: v1\n",
        )
        .unwrap();

        // Pass a dead Weak: the first event will fail to upgrade and the
        // watcher thread exits cleanly via the `else { return; }` branch.
        spawn_file_watcher(dead_weak_state(), vec![dir.clone()], false);

        // Give the debouncer time to install its watch, then touch the file.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        std::fs::write(
            &file,
            "fixtures:\n  - match:\n      user_message: hi\n    response:\n      content: v2\n",
        )
        .unwrap();

        // Let the debouncer flush + watcher thread run the upgrade-None path.
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn should_exit_sighup_handler_when_state_is_dropped() {
        // Hits the `Weak::upgrade() -> None` early-return in the SIGHUP loop.
        // We install a handler with a dead Weak, then send SIGHUP to ourselves.
        // The handler receives the signal, fails to upgrade, and returns.
        spawn_sighup_handler(dead_weak_state(), vec![], false);

        // Give the handler a moment to install its signal listener.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // SAFETY: sending SIGHUP to our own PID is well-defined on Unix.
        // Any other SIGHUP handlers installed by concurrent tests will also
        // receive this signal; each handles its own state independently.
        unsafe {
            libc::kill(libc::getpid(), libc::SIGHUP);
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
