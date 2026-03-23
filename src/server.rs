use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::routing::post;
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
}

#[cfg(feature = "oauth")]
impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            client_id: "mock-client".to_string(),
            client_secret: "mock-secret".to_string(),
        }
    }
}

pub(crate) struct AppState {
    pub(crate) fixtures: Vec<Fixture>,
    pub(crate) id_gen: IdGenerator,
    pub(crate) verbose: bool,
    /// Separate counter for x-request-id headers (doesn't interfere with response IDs).
    pub(crate) request_counter: AtomicU64,
    pub(crate) auth: Option<crate::auth::AuthState>,
}

impl AppState {
    pub(crate) fn next_request_id(&self) -> String {
        let n = self.request_counter.fetch_add(1, Ordering::Relaxed);
        format!("req-llmposter-{}", n)
    }
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

    // Auto-emit provider-specific rate limit headers on 429 responses
    if resp.status() == StatusCode::TOO_MANY_REQUESTS {
        let headers = resp.headers_mut();
        headers
            .entry("retry-after")
            .or_insert("60".parse().unwrap());

        if path.starts_with("/v1/messages") {
            // Anthropic: anthropic-ratelimit-requests-{limit,remaining,reset}
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
            // Gemini: no x-ratelimit-* headers; retry-after only
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

pub struct ServerBuilder {
    fixtures: Vec<Fixture>,
    bind_addr: String,
    verbose: bool,
    auth_enabled: bool,
    bearer_tokens: Vec<(String, Option<u64>)>,
    #[cfg(feature = "oauth")]
    oauth_config: Option<OAuthConfig>,
}

impl ServerBuilder {
    pub fn new() -> Self {
        Self {
            fixtures: Vec::new(),
            bind_addr: "127.0.0.1:0".to_string(),
            verbose: false,
            auth_enabled: false,
            bearer_tokens: Vec::new(),
            #[cfg(feature = "oauth")]
            oauth_config: None,
        }
    }

    pub fn fixture(mut self, f: Fixture) -> Self {
        self.fixtures.push(f);
        self
    }

    pub fn fixtures(mut self, fixtures: Vec<Fixture>) -> Self {
        self.fixtures.extend(fixtures);
        self
    }

    pub fn bind(mut self, addr: &str) -> Self {
        self.bind_addr = addr.to_string();
        self
    }

    pub fn verbose(mut self, v: bool) -> Self {
        self.verbose = v;
        self
    }

    pub fn with_auth(mut self, enabled: bool) -> Self {
        self.auth_enabled = enabled;
        self
    }

    pub fn with_bearer_token(mut self, token: &str) -> Self {
        self.auth_enabled = true;
        self.bearer_tokens.push((token.to_string(), None));
        self
    }

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

    pub fn load_yaml(mut self, path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let fixtures = crate::fixture::load_yaml_file(path)?;
        self.fixtures.extend(fixtures);
        Ok(self)
    }

    pub fn load_yaml_dir(
        mut self,
        dir: &std::path::Path,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let fixtures = crate::fixture::load_yaml_dir(dir)?;
        self.fixtures.extend(fixtures);
        Ok(self)
    }

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
            let oauth = oauth_mock::MockServer::builder()
                .with_client(
                    &config.client_id,
                    &config.client_secret,
                    ["https://example.com/callback"],
                    ["openid", "profile", "email"],
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
                            .unwrap_or_default(),
                    });
                }
            }
            Some(auth_state)
        } else {
            None
        };

        let state = Arc::new(AppState {
            fixtures: self.fixtures,
            id_gen: IdGenerator::new(),
            verbose: self.verbose,
            request_counter: AtomicU64::new(1),
            auth,
        });

        let app = Router::new()
            .route("/v1/chat/completions", post(crate::handler::openai::handle))
            .route("/v1/messages", post(crate::handler::anthropic::handle))
            .route("/v1/responses", post(crate::handler::responses::handle))
            .route(
                "/v1beta/models/{*path}",
                post(crate::handler::gemini::handle),
            )
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

pub struct MockServer {
    addr: std::net::SocketAddr,
    _handle: tokio::task::JoinHandle<()>,
    /// Check for post-bind server errors via `check_error()`.
    server_error: tokio::sync::Mutex<tokio::sync::oneshot::Receiver<String>>,
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
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

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
}
