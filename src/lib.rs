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
//! - **Auth simulation**: bearer tokens, OAuth2 mock server (default `oauth` feature)
//! - **Record & replay (VCR)**: proxy requests to the real provider API and save
//!   responses as replayable fixtures. The default `record` feature pulls in
//!   reqwest's rustls TLS stack — disable via `default-features = false` if you
//!   don't need recording and want the smaller dependency footprint.
//! - **Deterministic**: fixed IDs, sequential counters, no randomness
//!
//! ## Modules
//!
//! - [`fixture`] — fixture types, matching, YAML loading
//! - [`server`] — [`ServerBuilder`], [`MockServer`], [`CapturedRequest`]
//! - [`auth`] — bearer token and OAuth2 middleware
//! - [`record`] — VCR record/replay: [`VcrMode`], recording proxy (`record` feature)
//! - [`cli`] — CLI binary entry point

/// Bearer token authentication and OAuth2 middleware.
pub mod auth;
pub(crate) mod chaos;
/// CLI binary entry point and argument parsing.
pub mod cli;
pub(crate) mod failure;
/// Fixture types, matching logic, and YAML loading.
pub mod fixture;
pub(crate) mod format;
pub(crate) mod handler;
#[cfg(feature = "record")]
pub mod record;
/// Server builder, mock server, and request capture.
pub mod server;
pub(crate) mod stream;
#[cfg(feature = "templating")]
pub(crate) mod templating;
#[cfg(feature = "ui")]
pub(crate) mod ui;

// Re-exports for convenience — these are the primary API surface.
pub use auth::{AuthState, TokenStatus};
pub use fixture::{FailureConfig, Fixture, Refusal, ScenarioConfig, StreamingConfig, ToolCall};
pub use format::Provider;
#[cfg(feature = "record")]
pub use record::VcrMode;
#[cfg(feature = "oauth")]
pub use server::OAuthConfig;
pub use server::{CapturedRequest, MockServer, RequestOutcome, ServerBuilder};
