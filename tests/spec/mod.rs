//! Shared test helpers for spec compliance tests.

mod anthropic;
mod gemini;
mod openai;
mod responses;
pub mod types;

use llmposter::{Fixture, MockServer, ServerBuilder, ToolCall};
use reqwest::Client;

/// Build a mock server with a simple text response fixture.
pub async fn server_with_text(user_message: &str, content: &str) -> (MockServer, Client) {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message(user_message)
                .respond_with_content(content),
        )
        .build()
        .await
        .unwrap();
    (server, Client::new())
}

/// Build a mock server with a tool call fixture.
pub async fn server_with_tool_call(
    user_message: &str,
    name: &str,
    args: serde_json::Value,
) -> (MockServer, Client) {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message(user_message)
                .respond_with_tool_calls(vec![ToolCall {
                    name: name.to_string(),
                    arguments: args,
                }]),
        )
        .build()
        .await
        .unwrap();
    (server, Client::new())
}

/// Parse an SSE response body into a list of data payloads.
/// Returns each `data: <payload>` line's payload, excluding `[DONE]`.
/// Note: assumes one `data:` line per event (our mock server always emits
/// single-line JSON). For multi-line SSE data, use `parse_anthropic_sse`.
pub fn parse_sse_data(body: &str) -> Vec<String> {
    body.lines()
        .filter(|line| line.starts_with("data: "))
        .map(|line| line.trim_start_matches("data: ").to_string())
        .filter(|data| data != "[DONE]")
        .collect()
}

/// Check if the SSE body ends with a `data: [DONE]` frame.
pub fn has_done_sentinel(body: &str) -> bool {
    body.lines()
        .rfind(|line| line.starts_with("data: "))
        .map(|line| line.trim_start_matches("data: ") == "[DONE]")
        .unwrap_or(false)
}
