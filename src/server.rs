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

pub(crate) struct AppState {
    pub(crate) fixtures: Vec<Fixture>,
    pub(crate) id_gen: IdGenerator,
    pub(crate) verbose: bool,
    /// Separate counter for x-request-id headers (doesn't interfere with response IDs).
    pub(crate) request_counter: AtomicU64,
}

impl AppState {
    pub(crate) fn next_request_id(&self) -> String {
        let n = self.request_counter.fetch_add(1, Ordering::Relaxed);
        format!("req-llmposter-{}", n)
    }
}

/// Middleware: adds x-request-id to every response, rate limit headers on 429.
async fn add_response_headers(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: Next,
) -> axum::response::Response {
    let mut resp = next.run(request).await;
    let request_id = state.next_request_id();
    resp.headers_mut()
        .insert("x-request-id", request_id.parse().unwrap());

    // Auto-emit rate limit headers on 429 responses
    if resp.status() == StatusCode::TOO_MANY_REQUESTS {
        let headers = resp.headers_mut();
        headers
            .entry("retry-after")
            .or_insert("60".parse().unwrap());
        headers
            .entry("x-ratelimit-limit-requests")
            .or_insert("100".parse().unwrap());
        headers
            .entry("x-ratelimit-remaining-requests")
            .or_insert("0".parse().unwrap());
        headers
            .entry("x-ratelimit-reset-requests")
            .or_insert("60".parse().unwrap());
    }

    resp
}

pub struct ServerBuilder {
    fixtures: Vec<Fixture>,
    bind_addr: String,
    verbose: bool,
}

impl ServerBuilder {
    pub fn new() -> Self {
        Self {
            fixtures: Vec::new(),
            bind_addr: "127.0.0.1:0".to_string(),
            verbose: false,
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

        let state = Arc::new(AppState {
            fixtures: self.fixtures,
            id_gen: IdGenerator::new(),
            verbose: self.verbose,
            request_counter: AtomicU64::new(1),
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
