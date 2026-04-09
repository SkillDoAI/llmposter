//! # llmposter
//!
//! A fixture-driven mock server for LLM APIs. Speaks OpenAI Chat Completions,
//! Anthropic Messages, Gemini generateContent, and OpenAI Responses API — both
//! streaming (SSE) and non-streaming.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use llmposter::{ServerBuilder, Fixture};
//!
//! #[tokio::test]
//! async fn test_llm_client() {
//!     let server = ServerBuilder::new()
//!         .fixture(
//!             Fixture::new()
//!                 .match_user_message("hello")
//!                 .respond_with_content("Hi from the mock!"),
//!         )
//!         .build()
//!         .await
//!         .unwrap();
//!
//!     // Point your LLM client at server.url() instead of the real API
//!     let base_url = server.url();
//!     // ... your client code here ...
//! }
//! ```
//!
//! ## Features
//!
//! - **4 LLM API formats**: OpenAI, Anthropic, Gemini, OpenAI Responses
//! - **Streaming & non-streaming**: SSE and JSON-array responses
//! - **Fixture-driven**: YAML files or programmatic builder API
//! - **Failure simulation**: latency, truncation, disconnect, corruption, error codes
//! - **Stateful scenarios**: multi-turn matching via named state machines
//! - **Request capture**: verify what your client sent with `server.get_requests()`
//! - **Auth simulation**: bearer tokens, OAuth2 mock server (optional `oauth` feature)
//! - **Deterministic**: fixed IDs, sequential counters, no randomness
//!
//! ## Modules
//!
//! - [`fixture`] — fixture types, matching, YAML loading
//! - [`server`] — [`ServerBuilder`], [`MockServer`], [`CapturedRequest`]
//! - [`auth`] — bearer token and OAuth2 middleware
//! - [`cli`] — CLI binary entry point

/// Bearer token authentication and OAuth2 middleware.
pub mod auth;
/// CLI binary entry point and argument parsing.
pub mod cli;
pub(crate) mod failure;
/// Fixture types, matching logic, and YAML loading.
pub mod fixture;
pub(crate) mod format;
pub(crate) mod handler;
/// Server builder, mock server, and request capture.
pub mod server;
pub(crate) mod stream;

// Re-exports for convenience — these are the primary API surface.
pub use auth::{AuthState, TokenStatus};
pub use fixture::{FailureConfig, Fixture, ScenarioConfig, StreamingConfig, ToolCall};
pub use format::Provider;
#[cfg(feature = "oauth")]
pub use server::OAuthConfig;
pub use server::{CapturedRequest, MockServer, ServerBuilder};
