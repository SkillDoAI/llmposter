---
name: llmposter
description: Mock HTTP server for LLM provider APIs (OpenAI, Anthropic, Gemini, Responses API). Use when writing integration tests that need deterministic, controllable LLM API responses without calling real providers. Supports fixture-based request matching, SSE streaming, failure injection, auth simulation, and all four provider response formats.
license: AGPL-3.0-or-later
metadata:
  author: SkillDoAI
  version: "0.4.7"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6
---


# llmposter

Mock HTTP server for LLM provider APIs. Clients point their base URL at llmposter and interact using real API paths — no code changes beyond the URL swap. Fixtures define request matchers and canned responses for Anthropic (`/v1/messages`), OpenAI (`/v1/chat/completions`), Gemini (`/v1beta/models/{model}:generateContent`), and Responses API (`/v1/responses`). No provider prefix in routes — clients use the same paths as real APIs.

## Imports

```rust
// Core types (re-exported at crate root)
use llmposter::{Fixture, Provider, ServerBuilder};

// Fixture sub-types (in llmposter::fixture module)
use llmposter::fixture::{
    FailureConfig, FixtureResponse, ToolCall,
};
```

```toml
[dev-dependencies]
llmposter = "0.4"
tokio = { version = "1", features = ["full"] }
serde_json = "1"
reqwest = { version = "0.13", features = ["json"] }
```

Optional feature flags:

```toml
# All features
llmposter = { version = "0.4", features = ["ui", "watch", "oauth"] }

# Minimal (no auth, no jsonpath)
llmposter = { version = "0.4", default-features = false }
```

## Quick Start

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from Claude mock!"),
        )
        .build()
        .await?;

    // Point any LLM client's base_url at server.url()
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["type"], "message");
    assert_eq!(body["content"][0]["text"], "Hi from Claude mock!");
    assert_eq!(body["stop_reason"], "end_turn");
    Ok(())
}
```

## API Reference

### ServerBuilder

Builder for `MockServer`. Re-exported at crate root.

- `ServerBuilder::new() -> Self` — create an empty builder
- `.fixture(fixture: Fixture) -> Self` — add a fixture (chainable)
- `.load_yaml(path: &Path) -> Result<Self, Box<dyn std::error::Error>>` — load fixtures from a YAML file
- `.load_yaml_dir(path: &Path) -> Result<Self, Box<dyn std::error::Error>>` — load all YAML files in a directory
- `.verbose(verbose: bool) -> Self` — enable verbose logging; 404 responses include `"No fixture matched"` detail
- `.bind(addr: &str) -> Self` — bind address (default: random port on `127.0.0.1`)
- `.capture_capacity(capacity: usize) -> Self` — max captured requests in ring buffer. Library default: unbounded. `0` disables capture entirely.
- `.fixture_count(&self) -> usize` — number of fixtures currently loaded
- `.watch(watch: bool) -> Self` — enable hot-reload of fixture files (**requires `watch` feature**)
- `.ui(ui: bool) -> Self` — enable debug UI at `/ui` (**requires `ui` feature**)
- `.build(self) -> Result<MockServer, Box<dyn std::error::Error>>` — **async**. Validates fixtures and starts the server on a random port.

### MockServer

Running server handle. Re-exported at crate root.

- `.url(&self) -> String` — base URL (e.g., `http://127.0.0.1:PORT`)
- `.get_requests(&self) -> Vec<CapturedRequest>` — all captured requests for assertion/verification

### Fixture

Central type for defining mock behavior. Re-exported at crate root. All builder methods return `Self` (chainable).

**Matching methods:**
- `.match_user_message(substring: &str)` — substring match on the last user message
- `.match_model(substring: &str)` — substring match on the model field
- `.for_provider(provider: Provider)` — restrict to a specific provider endpoint

**Response methods:**
- `.respond_with_content(content: &str)` — text response
- `.respond_with_tool_calls(tool_calls: Vec<ToolCall>)` — tool use response (mutually exclusive with text content)
- `.with_error(status: u16, message: &str)` — HTTP error response
- `.with_streaming(latency_ms: Option<u64>, chunk_size: Option<usize>)` — enable SSE streaming with optional inter-chunk latency and chunk size
- `.with_failure(failure: FailureConfig)` — inject failure behaviors

**Public struct fields** (for direct construction):
- `Fixture.match_rule: Option<FixtureMatch>` — match criteria (includes `headers`, `system_prompt`, `temperature`, `metadata`, `tool_schema`, `body_jsonpath` fields — set via direct `FixtureMatch` struct construction)
- `Fixture.provider: Option<Provider>` — provider restriction
- `Fixture.response: Option<FixtureResponse>` — response configuration
- `Fixture.error: Option<FixtureError>` — error response
- `Fixture.failure: Option<FailureConfig>` — failure injection
- `Fixture.streaming: Option<StreamingConfig>` — SSE streaming config
- `Fixture.scenario: Option<ScenarioConfig>` — stateful multi-turn scenario matching
- `Fixture.refusal: Option<Refusal>` — OpenAI refusal field
- `Fixture.priority: Option<i32>` — default `0`; higher values match first
- `Fixture.catch_all: bool` — default `false`; when `true`, fixture is checked only after all non-catch-all fixtures

### FixtureResponse

In `llmposter::fixture`. Derives `Default`. For custom response construction (e.g., overriding `stop_reason`).

- `FixtureResponse.content: Option<String>` — text body. When `None`, no text content block is returned.
- `FixtureResponse.content_template: Option<String>` — minijinja template for dynamic content. When `None`, `content` is used as-is.
- `FixtureResponse.tool_calls: Option<Vec<ToolCall>>` — tool use responses. When `None`, response is text-only.
- `FixtureResponse.stop_reason: Option<String>` — Anthropic stop reason. When `None`, defaults to `"end_turn"` for text, `"tool_use"` for tool calls.
- `FixtureResponse.finish_reason: Option<String>` — OpenAI finish reason. When `None`, defaults to `"stop"` for text.

### ToolCall

In `llmposter::fixture`. Represents a tool/function call in a fixture response.

- `ToolCall.name: String` — tool function name (required)
- `ToolCall.arguments: serde_json::Value` — tool input as parsed JSON Value (required). This is `serde_json::Value`, not a stringified JSON string as in some real APIs.

### FailureConfig

In `llmposter::fixture`. Derives `Default`. Configures failure injection.

- `FailureConfig.latency_ms: Option<u64>` — delay in ms before response. When `None`, no delay.
- `FailureConfig.corrupt_body: Option<bool>` — when `Some(true)`, returns literal string `"overloaded"` as `text/plain` with HTTP 200. Configured content is ignored entirely.
- `FailureConfig.truncate_after_frames: Option<usize>` — cut SSE stream after N frames. Stream ends without `message_stop` event. Only applies to streaming requests; ignored with a warning on non-streaming.
- `FailureConfig.disconnect_after_ms: Option<u64>` — abort connection after N ms. Requires streaming with `latency > 0` for reliable triggering — with `latency=0`, frames complete before disconnect timer fires.
- `FailureConfig.probability: Option<f32>` — probability that failure applies.
- `FailureConfig.latency_jitter_ms: Option<u64>` — random jitter added to latency. Requires `latency_ms` to be set; rejected at fixture load time without it.
- `FailureConfig.duplicate_frames: Option<bool>` — duplicate SSE frames.
- `FailureConfig.chaos_seed: Option<u64>` — seed for deterministic chaos reproduction.

### StreamingConfig

Re-exported at crate root. Configures SSE streaming behavior.

- `StreamingConfig.latency: Option<u64>` — inter-chunk delay in ms. Note: the struct field name is `latency`, while the builder method `with_streaming()` uses `latency_ms` as its parameter name.
- `StreamingConfig.chunk_size: Option<usize>` — characters per chunk for text content. Ignored for tool-call streaming across all four providers.

### Provider

Re-exported at crate root. Enum with exactly 4 variants:

- `Provider::OpenAI` — serves `/v1/chat/completions`
- `Provider::Anthropic` — serves `/v1/messages`
- `Provider::Gemini` — serves `/v1beta/models/{model}:generateContent`
- `Provider::Responses` — serves `/v1/responses` (OpenAI Responses API, distinct from Chat Completions)

### AuthState and TokenStatus

Re-exported at crate root. Bearer token management (`auth` feature — on by default). Auth only protects LLM routes — `/code/{N}` and `/ui` are never auth-protected.

- `AuthState::new() -> Self` — create empty token store
- `.add_token(token: &str, max_uses: Option<u64>)` — register a bearer token. `None` means unlimited uses.
- `.check_and_use(token: &str) -> TokenStatus` — check token and atomically decrement use count
- `.revoke(token: &str)` — revoke a token (moves to deny-list)

`TokenStatus` enum (3 variants):
- `TokenStatus::Valid` — token is registered and has remaining uses
- `TokenStatus::Exhausted` — token was valid but all uses consumed
- `TokenStatus::Unknown` — token was never registered

### OAuthConfig

Re-exported at crate root. **Requires `oauth` feature** (off by default). Configures embedded OAuth mock server.

- `OAuthConfig.client_id: String` — default `"mock-client"`
- `OAuthConfig.client_secret: String` — default `"mock-secret"`
- `OAuthConfig.redirect_uris: Vec<String>` — default `["https://example.com/callback"]`
- `OAuthConfig.scopes: Vec<String>` — default `["openid", "profile", "email"]`

### Matching Types

In `llmposter::fixture`:

- `StringMatch::Substring(String)` — default variant, substring/contains matching
- `StringMatch::Regex(RegexMatch)` — regex matching; construct via `StringMatch::regex(pattern: &str)`. Use anchors `^...$` for exact matching (no `Exact` variant exists).
- `FixtureMatch` — match criteria struct with optional fields: `user_message`, `model`, `headers`, `system_prompt`, `temperature`, `metadata`, `tool_schema`, `body_jsonpath`
- `F64Match::Exact(f64)` | `F64Match::Range(F64Range)` — numeric matching for temperature
- `F64Range { min: Option<f64>, max: Option<f64> }` — inclusive min/max bounds

## Core Patterns

### Tool Use Response

```rust
mod tool_use_example {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::ToolCall;

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("weather")
                    .respond_with_tool_calls(vec![ToolCall {
                        name: "get_weather".to_string(),
                        arguments: serde_json::json!({
                            "location": "London",
                            "unit": "celsius"
                        }),
                    }]),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "What's the weather?"}]
            }))
            .send()
            .await?;

        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["name"], "get_weather");
        // Tool call IDs are deterministic: toolu_llmposter_{N} (1-indexed)
        assert_eq!(body["content"][0]["id"], "toolu_llmposter_1");
        // Anthropic uses "input" field (NOT "arguments") for tool call arguments
        assert_eq!(body["content"][0]["input"]["location"], "London");
        Ok(())
    }
}
```

### SSE Streaming

```rust
mod streaming_example {
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("hello")
                    .respond_with_content("Hello world")
                    .with_streaming(Some(0), Some(5)), // latency_ms=0, chunk_size=5 chars
            )
            .build()
            .await?;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "hello"}],
                "stream": true
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        let text = resp.text().await?;
        // SSE events: message_start, content_block_start, content_block_delta,
        //             content_block_stop, message_delta, message_stop
        assert!(text.contains("event: message_start"));
        assert!(text.contains("event: content_block_delta"));
        assert!(text.contains("event: message_stop"));
        Ok(())
    }
}
```

### Error Simulation

```rust
mod error_simulation {
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("rate limit")
                    .with_error(429, "Rate limit exceeded"),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "rate limit test"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 429);
        // Error shape is {"error": {"message": "..."}} for ALL providers
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["error"]["message"], "Rate limit exceeded");
        Ok(())
    }
}
```

### Failure Injection (Latency)

```rust
mod latency_injection {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::FailureConfig;

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("delayed response")
                    .with_failure(FailureConfig {
                        latency_ms: Some(200),
                        ..Default::default()
                    }),
            )
            .build()
            .await?;

        let start = std::time::Instant::now();
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        assert!(start.elapsed().as_millis() >= 180);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["content"][0]["text"], "delayed response");
        Ok(())
    }
}
```

### Corrupt Body (Overloaded Simulation)

```rust
mod corrupt_body_example {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::FailureConfig;

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("should not appear")
                    .with_failure(FailureConfig {
                        corrupt_body: Some(true),
                        ..Default::default()
                    }),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;

        // Returns HTTP 200 with text/plain body "overloaded" — NOT JSON
        assert_eq!(resp.status(), 200);
        let text = resp.text().await?;
        assert_eq!(text, "overloaded");
        Ok(())
    }
}
```

### Provider Filtering

```rust
mod provider_filtering {
    use llmposter::{Fixture, Provider, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .for_provider(Provider::OpenAI)
                    .respond_with_content("openai only"),
            )
            .fixture(
                Fixture::new()
                    .for_provider(Provider::Anthropic)
                    .respond_with_content("anthropic only"),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();

        // OpenAI fixture matches on /v1/chat/completions
        let resp = client
            .post(format!("{}/v1/chat/completions", server.url()))
            .json(&serde_json::json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);

        // OpenAI fixture does NOT match /v1/messages — Anthropic fixture does
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["content"][0]["text"], "anthropic only");
        Ok(())
    }
}
```

### Custom Stop Reason

```rust
mod custom_stop_reason {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::FixtureResponse;

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        // No builder method for stop_reason — requires direct struct construction
        let server = ServerBuilder::new()
            .fixture(Fixture {
                response: Some(FixtureResponse {
                    content: Some("hit max tokens".to_string()),
                    stop_reason: Some("max_tokens".to_string()),
                    ..Default::default()
                }),
                ..Fixture::new()
            })
            .build()
            .await?;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;

        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["stop_reason"], "max_tokens");
        assert_eq!(body["content"][0]["text"], "hit max tokens");
        Ok(())
    }
}
```

### Stream Truncation

```rust
mod stream_truncation {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::FailureConfig;

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("long text that gets truncated")
                    .with_streaming(Some(0), Some(5))
                    .with_failure(FailureConfig {
                        truncate_after_frames: Some(2),
                        ..Default::default()
                    }),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}],
                "stream": true
            }))
            .send()
            .await?;

        let text = resp.text().await?;
        assert!(text.contains("event: message_start"));
        // Stream ends abruptly — no message_stop event emitted
        assert!(!text.contains("event: message_stop"));
        Ok(())
    }
}
```

### YAML Fixtures

```rust
mod yaml_fixtures {
    use llmposter::ServerBuilder;
    use std::path::Path;

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .load_yaml(Path::new("fixtures/anthropic.yaml"))?
            .build()
            .await?;

        println!("Mock server at {} with fixtures loaded", server.url());
        Ok(())
    }
}
```

YAML fixture format:

```yaml
# fixtures/anthropic.yaml
- match:
    user_message: "hello"
  response:
    content: "Hi from the mock!"

- match:
    model: "claude-sonnet"
    user_message: "weather"
  response:
    tool_calls:
      - name: get_weather
        arguments:
          location: London
          unit: celsius

- match:
    user_message: "fail"
  error:
    status: 429
    message: "Rate limit exceeded"

- match:
    user_message: "slow"
  response:
    content: "delayed"
  streaming:
    latency: 50
    chunk_size: 5
  failure:
    latency_ms: 500

- priority: 10
  catch_all: true
  response:
    content: "fallback response"
```

### Request Capture

```rust
mod request_capture {
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("captured"))
            .capture_capacity(100)
            .build()
            .await?;

        let client = reqwest::Client::new();
        client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;

        let requests = server.get_requests();
        assert_eq!(requests.len(), 1);
        Ok(())
    }
}
```

### Model Matching

```rust
mod model_matching {
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_model("claude-sonnet")
                    .respond_with_content("sonnet response"),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();

        // "claude-sonnet" is a substring of "claude-sonnet-4-6" — matches
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);

        // "claude-sonnet" is NOT a substring of "claude-haiku-3" — 404
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-haiku-3",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 404);
        Ok(())
    }
}
```

## Endpoint Reference

No provider prefix in routes — clients use real API paths, just swap the base URL.

| Provider | Endpoint | Notes |
|----------|----------|-------|
| Anthropic | `POST /v1/messages` | Requires `max_tokens` in request body |
| OpenAI | `POST /v1/chat/completions` | |
| Gemini | `POST /v1beta/models/{model}:generateContent` | Model name in URL path |
| Responses | `POST /v1/responses` | `input` field is optional (for continuation requests) |
| Utility | `GET /code/{status}` | Returns specified HTTP status code; not auth-protected |

### Response Shapes

**Anthropic non-streaming:**
- `body.type`: `"message"`
- `body.role`: `"assistant"`
- `body.id`: `"msg-llmposter-{uuid}"` (hyphens, not underscores)
- `body.content[N].type`: `"text"` or `"tool_use"`
- `body.stop_reason`: `"end_turn"` (text) | `"tool_use"` (tool calls) | custom
- `body.usage.input_tokens`: `u64` (approximate, `bytes/4` heuristic)
- `body.usage.output_tokens`: `u64` (approximate, `bytes/4` heuristic)

**Anthropic streaming** (SSE, `Content-Type: text/event-stream`):
Events in order: `message_start`, `content_block_start`, `content_block_delta` (repeated), `content_block_stop`, `message_delta`, `message_stop`

**Error responses** (all providers): `{"error": {"message": "..."}}`

**`/code/{status}`:** Returns the specified HTTP status code. `/code/204`, `/code/304`, and `/code/205` return empty bodies.

## Behavioral Semantics

- **Matching order**: first-match-wins with priority override. Non-catch-all fixtures are sorted by descending priority (higher wins), then file order breaks ties. Catch-all fixtures (`catch_all: true`) are always checked after all non-catch-all fixtures, regardless of priority. Default priority is `0`.
- **Match fields stack conjunctively**: a fixture with both `model` and `tool_schema` requires every condition to match.
- **Substring matching**: `match_user_message` and `match_model` use substring/contains matching. `"hello"` matches `"hello world"`. No exact match variant exists — use `StringMatch::regex("^exact$")` for exact matching.
- **Prompt redaction in no-match errors**: no-match error responses redact prompt content (since v0.4.2) to avoid leaking sensitive content in logs and error messages.
- **Response IDs**: always `msg-llmposter-{uuid}` format.
- **Tool call IDs**: deterministic `toolu_llmposter_{N}` (1-indexed, sequential). Not random UUIDs.
- **Token counts**: `bytes/4` heuristic — not a real tokenizer. Never assert exact values, only `> 0`.
- **Anthropic tool input field**: `ToolCall.arguments` in Rust maps to `content[].input` in the Anthropic JSON response — not `content[].arguments`.
- **Anthropic `stop_reason`**: defaults to `"end_turn"` for text, `"tool_use"` for tool calls. Not `"stop"` (that is OpenAI's `finish_reason`).
- **`max_tokens` required for Anthropic** (since v0.4.2). Missing `max_tokens` returns a validation error.
- **Non-boolean `stream` field rejected**: requests with `stream` set to a non-boolean type (e.g., `"yes"` string) return an error (since v0.4.1).
- **Auth scope**: LLM routes only (since v0.4.2). `/code/{N}` and `/ui` are never auth-protected.
- **`corrupt_body`**: always returns literal `"overloaded"` as `text/plain` with HTTP 200. Configured content is ignored. On streaming requests, emits a malformed SSE frame (since v0.4.5).
- **`chunk_size` ignored for tool-call streaming** across all four providers. Only affects text content streaming.
- **Gemini non-SSE streaming is buffered**: collects all chunks in memory, returns single response. `disconnect_after_ms` produces a shorter 200 OK array, not a transport failure. Use `?alt=sse` for true SSE transport failure simulation.
- **Hot-reload**: via `--watch` flag or SIGHUP signal. Fixtures are swapped atomically with priority re-sorting at load time (since v0.4.7).
- **Load-time validation**: invalid JSONPath, duplicate headers (case-insensitive), and jitter without latency are all rejected when fixtures are loaded — not at request time.
- **OpenAI first streaming chunk**: omits `content` field entirely via `skip_serializing_if`, not `"content": null`. All major SDKs treat absent and null identically.
- **Captured request status under chaos**: `push_captured` runs before chaos logic. A chaos-injected 500 shows as 200 in the capture log.
- **`truncate_after_frames` / `disconnect_after_ms` on non-streaming**: warning emitted, fields ignored.
- **`disconnect_after_ms`**: requires streaming with `latency > 0`. With `latency=0`, frames complete before disconnect timer fires.
- **Content templating**: uses minijinja via `content_template` field in `FixtureResponse`.
- **Blank message rejection**: OpenAI/Gemini/Responses API content extractors reject blank messages (since v0.4.5). Anthropic requires text in the latest user turn (since v0.4.3).
- **Responses API streaming tool calls**: `function_call_arguments.done` event includes the tool `name` field (since v0.4.3). Tool call IDs are globally unique (since v0.4.2).

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `auth` | **on** | Bearer token middleware |
| `jsonpath` | **on** (since v0.4.6) | `body_jsonpath` match field in fixtures |
| `ui` | off | Debug UI at `/ui` |
| `watch` | off | Hot-reload via file watcher |
| `oauth` | off | `OAuthConfig` for embedded OAuth mock server |

Disable defaults: `llmposter = { version = "0.4", default-features = false, features = ["jsonpath"] }`

## CLI Usage

```bash
# Start mock server with YAML fixtures (default port: 2112)
llmposter fixtures/

# Validate fixtures without starting server
llmposter fixtures/ --validate

# Custom port and bind address
llmposter fixtures/ --port 8080 --bind 0.0.0.0

# Verbose logging (404 responses include match failure detail)
llmposter fixtures/ --verbose

# Set capture capacity (default: 1000; 0 disables capture)
llmposter fixtures/ --capture-capacity 5000

# Enable debug UI (requires ui feature)
llmposter fixtures/ --ui

# Enable hot-reload (requires watch feature)
llmposter fixtures/ --watch
```

## Pitfalls

- **`chunk_size` and tool calls**: `chunk_size` is silently ignored for tool-call streaming across all four providers. It only affects text content streaming.
- **Gemini disconnect simulation**: non-SSE Gemini streaming is buffered. `disconnect_after_ms` produces a shorter 200 OK array, not a transport failure. Use `?alt=sse` for real disconnect simulation.
- **Priority vs file order**: since v0.4.6, fixtures are sorted by descending priority. A `priority: 10` fixture at the bottom of the file wins over `priority: 0` at the top. File order is the tiebreaker within the same priority level.
- **Capture log under chaos**: captured request status shows pre-chaos value (e.g., 200 even if chaos injects 500). Verify chaos failures via the HTTP response, not the capture log.
- **OpenAI first streaming chunk**: `content` field is absent, not `null`. Assert `content.is_none()` or check for absent/null — strict JSON equality fails.
- **Token count accuracy**: `bytes/4` heuristic. Never assert exact token counts; assert `> 0` only.
- **JSONPath with `default-features = false`**: `body_jsonpath` requires the `jsonpath` feature. Re-enable explicitly if you disabled defaults.
- **Jitter without latency**: `latency_jitter_ms` requires `latency_ms` to be set. Rejected at fixture load time, not runtime.
- **Duplicate response headers**: case-insensitive duplicate detection rejects fixtures at load time. Do not set `Content-Type` in custom headers — the handler sets it automatically.
- **Streaming-only fields on non-streaming**: `truncate_after_frames` and `disconnect_after_ms` are ignored with a warning on non-streaming requests.
- **CLI vs library capture defaults**: CLI defaults to 1000 captured requests (since v0.4.7) with FIFO trimming. Library default is unbounded. Set `--capture-capacity 0` to disable capture.
- **`capture_capacity(0)` disables capture**: `get_requests()` returns empty. This is "disabled", not "unlimited".

## Migration

### v0.4.6 → v0.4.7

- **CLI capture capacity default changed**: from unbounded to 1000 with FIFO trimming. Pass `--capture-capacity <N>` for a higher value. Library users are unaffected (still unbounded by default).
- `FixtureSet` is now the internal fixture storage type (was inline `Vec` + sort). No public Rust API change for library users.
- `ServerBuilder::ui(true)` and `--ui` CLI flag are new opt-in features.

### v0.4.5 → v0.4.6

- **Fixture matching changed from file-order to priority-based sorting**. Fixtures with explicit `priority` values now match before lower-priority fixtures regardless of file position. Default priority is 0 — existing fixtures without priority are unaffected relative to each other.
- New match fields (`headers`, `system_prompt`, `temperature`, `metadata`, `tool_schema`, `body_jsonpath`) are all additive and optional.
- `jsonpath` feature is on by default. Disable with `default-features = false` if not needed.
- New `Fixture` struct fields (`priority`, `catch_all`) and `FixtureMatch` fields are set via direct struct construction.

### v0.4.4 → v0.4.5

- Content extractors for OpenAI/Gemini/Responses API reject blank messages (matching real API behavior). Tests that previously sent blank messages will now get errors.
- Internal: fixture storage changed from `Vec<Fixture>` to `Vec<Arc<Fixture>>`. Standard builder/YAML usage is unaffected.

### v0.4.1 → v0.4.2

- Auth scope narrowed to LLM routes only. `/code/{N}` and utility routes no longer require auth tokens.
- SSE responses no longer include `Connection: keep-alive` header (invalid for HTTP/2).
- `max_tokens` validation added for Anthropic requests.

### General Upgrade

All 0.4.x releases are additive with no Rust API breakage. YAML fixture format is backward-compatible — old fixtures work unchanged with new versions. Pin to `"0.4"` semver range.

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://skilldoai.com)
- [Documentation](https://docs.rs/llmposter)
- [Repository docs](https://github.com/SkillDoAI/llmposter/tree/HEAD/docs)
