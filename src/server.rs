use std::sync::Arc;

use axum::routing::post;
use axum::Router;
use tokio::net::TcpListener;

use crate::fixture::Fixture;
use crate::format::IdGenerator;

pub(crate) struct AppState {
    pub(crate) fixtures: Vec<Fixture>,
    pub(crate) id_gen: IdGenerator,
    pub(crate) verbose: bool,
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

    pub async fn build(mut self) -> MockServer {
        // Validate all fixtures (including programmatically-added ones)
        for (i, fixture) in self.fixtures.iter_mut().enumerate() {
            fixture
                .validate()
                .unwrap_or_else(|e| panic!("Fixture #{}: {}", i + 1, e));
        }

        let state = Arc::new(AppState {
            fixtures: self.fixtures,
            id_gen: IdGenerator::new(),
            verbose: self.verbose,
        });

        let app = Router::new()
            .route("/v1/chat/completions", post(crate::handler::openai::handle))
            .route("/v1/messages", post(crate::handler::anthropic::handle))
            .route("/v1/responses", post(crate::handler::responses::handle))
            .route(
                "/v1beta/models/{*path}",
                post(crate::handler::gemini::handle),
            )
            .with_state(state);

        let listener = TcpListener::bind(&self.bind_addr)
            .await
            .expect("Failed to bind server");
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("[llmposter] server error: {}", e);
            }
        });

        MockServer {
            addr,
            _handle: handle,
        }
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
}

impl MockServer {
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
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
            .await;
        assert!(server.port() > 0);
        assert!(server.url().starts_with("http://127.0.0.1:"));
    }

    #[tokio::test]
    async fn should_return_404_for_unknown_routes() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("test"))
            .build()
            .await;
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
            .await;
        assert!(server.port() > 0);
    }

    #[tokio::test]
    async fn should_support_default_builder() {
        let builder = ServerBuilder::default();
        let server = builder
            .fixture(Fixture::new().respond_with_content("default"))
            .build()
            .await;
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
        let server = ServerBuilder::new().fixtures(fixtures).build().await;
        assert!(server.port() > 0);
    }

    #[tokio::test]
    async fn should_support_verbose_mode() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("test"))
            .verbose(true)
            .build()
            .await;
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
        let server = ServerBuilder::new().load_yaml(&file).unwrap().build().await;
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
            .await;
        assert!(server.port() > 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    #[should_panic(expected = "Fixture #1")]
    async fn should_panic_on_invalid_fixture() {
        ServerBuilder::new()
            .fixture(Fixture::new()) // no response or error
            .build()
            .await;
    }
}
