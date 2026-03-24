---
name: llmposter
description: A mock LLM server for testing — emulates Anthropic, OpenAI, Gemini, and Responses API endpoints with fixture-based responses, streaming simulation, failure injection, and optional OAuth.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---

## Imports

```rust
use llmposter::{Fixture, MockServer, Provider, ServerBuilder};
use llmposter::fixture::{FailureConfig, FixtureResponse, StreamingConfig, ToolCall};
use llmposter::auth::{AuthState, TokenStatus};
```

```toml
[dependencies]
llmposter = "0.4.0"
tokio = { version = "1", features = ["full"] }
serde_json = "1"
reqwest = { version = "0.13", default-features = false, features = ["json"] }

# Optional: enable OAuth mock support (requires Rust >= 1.89)
# llmposter = { version = "0.4.0", features = ["oauth"] }
```

## Core Patterns

### Basic text fixture — Anthropic endpoint ✅ Current

```rust
mod basic_text {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_anthropic_text_response() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("hello")
                    .respond_with_content("Hi from Claude mock!"),
            )
            .build()
            .await?;

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
        assert_eq!(body["content"][0]["text"], "Hi from Claude mock!");
        assert_eq!(body["stop_reason"], "end_turn");
        Ok(())
    }
}
```

Fixtures are matched in registration order; the first matching fixture wins. `match_user_message` performs substring matching against the last user message. If no fixture matches, the server returns 404 with `{ error: { message: "No fixture matched" } }`.

### Tool-call response ✅ Current

```rust
mod tool_call {
    use llmposter::fixture::ToolCall;
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_tool_call_response() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("weather")
                    .respond_with_tool_calls(vec![ToolCall {
                        name: "get_weather".to_string(),
                        arguments: serde_json::json!({"location": "London", "unit": "celsius"}),
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
                "messages": [{"role": "user", "content": "What's the weather in London?"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["name"], "get_weather");
        Ok(())
    }
}
```

`respond_with_content` and `respond_with_tool_calls` are mutually exclusive — calling one clears the other. `ToolCall::arguments` must be a JSON object, not a string or array.

### Streaming (SSE) response ✅ Current

```rust
mod streaming {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_streaming_response() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("hello")
                    .respond_with_content("Hello world")
                    .with_streaming(Some(0), Some(5)), // (latency_ms, chunk_size_chars)
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
        let body = resp.text().await?;
        assert!(body.contains("event: message_start"));
        assert!(body.contains("event: content_block_delta"));
        assert!(body.contains("event: message_stop"));
        Ok(())
    }
}
```

`with_streaming(latency_ms, chunk_size)`: first argument is milliseconds between SSE chunks, second is characters per chunk. Pass `Some(0)` for no inter-chunk delay.

### Failure injection ✅ Current

```rust
mod failure_inject {
    use llmposter::fixture::FailureConfig;
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_latency_and_truncation() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("This is a long response that gets cut off")
                    .with_streaming(Some(0), Some(5))
                    .with_failure(FailureConfig {
                        latency_ms: Some(50),
                        truncate_after_frames: Some(2),
                        ..FailureConfig::default()
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
                "messages": [{"role": "user", "content": "hi"}],
                "stream": true
            }))
            .send()
            .await?;

        let body = resp.text().await?;
        assert!(body.contains("event: message_start"));
        assert!(!body.contains("event: message_stop")); // truncated before stop
        Ok(())
    }
}
```

`with_failure` requires a response to also be set. `corrupt_body: Some(true)` ignores the fixture content and returns plain-text `"overloaded"`. `truncate_after_frames` cuts the SSE stream before `message_stop` is emitted.

### Bearer auth enforcement ✅ Current

```rust
mod bearer_auth {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_bearer_auth() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .with_bearer_token("test-token-abc")
            .fixture(Fixture::new().respond_with_content("authorized"))
            .build()
            .await?;

        let client = reqwest::Client::new();

        // Unauthorized request
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 401);

        // Authorized request
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .bearer_auth("test-token-abc")
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);
        Ok(())
    }
}
```

`with_bearer_token` implicitly enables auth. Use `with_bearer_token_uses(token, max_uses)` for tokens that expire after N requests.

## Configuration

| Method | Default | Description |
|---|---|---|
| `ServerBuilder::new()` | bind `127.0.0.1:0` | Random port; use `server.url()` to get the actual address |
| `.bind(addr)` | `"127.0.0.1:0"` | Override bind address; supports `host:port`, `[::1]:port` |
| `.verbose(true)` | `false` | Log matched/unmatched requests to stderr |
| `.with_bearer_token(token)` | auth disabled | Add unlimited-use token; implicitly enables auth |
| `.with_bearer_token_uses(token, n)` | — | Token expires after `n` uses |
| `.with_auth(enabled)` | `false` | Explicit auth toggle; requires tokens added separately |
| `.fixture(f)` | — | Append one fixture; first-match-wins at request time |
| `.fixtures(vec)` | — | Batch-append fixtures (use after `load_yaml` / `load_yaml_dir`) |
| `.load_yaml(path)` | — | Parse fixtures from a single YAML file; returns `Result` |
| `.load_yaml_dir(dir)` | — | Parse all YAML files in directory; returns `Result` |

**OAuth** (requires `features = ["oauth"]`, Rust >= 1.89):

```rust
// Defaults: client_id="mock-client", client_secret="mock-secret"
let server = ServerBuilder::new()
    .with_oauth_defaults()
    .fixture(Fixture::new().respond_with_content("ok"))
    .build()
    .await?;
```

**Provider routing** — endpoints served per provider:
- `Provider::Anthropic` → `POST /v1/messages`
- `Provider::OpenAI` → `POST /v1/chat/completions`
- `Provider::Gemini` → Gemini endpoint
- `Provider::Responses` → OpenAI Responses API (`POST /v1/responses`)

Fixtures without `.for_provider(...)` match any provider endpoint.

## Pitfalls

### Empty match pattern is a silent catch-all

**Wrong:**
```rust
// Empty string matches every request — shadows all subsequent fixtures
Fixture::new().match_user_message("").respond_with_content("catch all")
```

**Right:**
```rust
// Always provide a non-empty pattern; as of 0.3.5, empty patterns are rejected at build time
Fixture::new().match_user_message("specific keyword").respond_with_content("matched")
```

### Tool-call arguments must be a JSON object

**Wrong:**
```rust
// String arguments are rejected at fixture load time (since 0.3.4)
ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!("San Francisco"), // string, not object
}
```

**Right:**
```rust
ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!({"location": "San Francisco", "unit": "celsius"}),
}
```

### `with_failure` without a response set

**Wrong:**
```rust
// with_failure alone has no effect — no response is defined to apply failure to
Fixture::new()
    .with_failure(FailureConfig { latency_ms: Some(200), ..FailureConfig::default() })
```

**Right:**
```rust
// Always pair with_failure with respond_with_content or respond_with_tool_calls
Fixture::new()
    .respond_with_content("delayed response")
    .with_failure(FailureConfig { latency_ms: Some(200), ..FailureConfig::default() })
```

### `respond_with_content` and `respond_with_tool_calls` are mutually exclusive

**Wrong:**
```rust
// respond_with_tool_calls clears any content set before it
Fixture::new()
    .respond_with_content("text")
    .respond_with_tool_calls(vec![ToolCall {
        name: "fn".to_string(),
        arguments: serde_json::json!({}),
    }])
// "text" is silently dropped
```

**Right:**
```rust
// Choose one response type per fixture; use separate fixtures for different response shapes
Fixture::new()
    .match_user_message("tool request")
    .respond_with_tool_calls(vec![ToolCall {
        name: "fn".to_string(),
        arguments: serde_json::json!({"param": "value"}),
    }])
```

### Provider filter mismatch produces 404

**Wrong:**
```rust
// OpenAI fixture is invisible to the Anthropic /v1/messages handler
ServerBuilder::new()
    .fixture(
        Fixture::new()
            .respond_with_content("hello")
            .for_provider(Provider::OpenAI),
    )
    // Requests to /v1/messages return 404 — no Anthropic fixture defined
```

**Right:**
```rust
// Use for_provider only when you need to restrict to a specific endpoint
// Omitting it matches all providers
ServerBuilder::new()
    .fixture(
        Fixture::new()
            .respond_with_content("hello")
            .for_provider(Provider::Anthropic), // or omit for_provider entirely
    )
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3

**v0.3.3 → v0.3.4: Responses API SSE envelope changed**

Consumers of the Responses API streaming endpoint must update their event parsers. The old non-spec `response.done` event is removed; replace with `response.in_progress`. Events now include a `sequence_number` field and a nested `response` envelope.

Before:
```json
{"event": "response.done", "data": {...}}
```

After:
```json
{"event": "response.in_progress", "sequence_number": 1, "response": {...}}
```

**v0.3.3 → v0.3.4: Error response `code` field is now a string**

Error responses now follow the real OpenAI shape: `code` is a `String`, and `param` is always present (as `null` when not applicable). Update any code that treats `code` as an integer.

**v0.3.6 → v0.4.0: MSRV bumped to Rust 1.89 when using the `oauth` feature**

The `oauth` feature (opt-in; add `features = ["oauth"]` to enable) depends on `oauth-mock`, which requires Rust 1.89. If you use this feature, update `rust-toolchain.toml` or CI matrix to `>= 1.89` before upgrading.

## API Reference

**`ServerBuilder::new()`** — Create a builder; default bind is `127.0.0.1:0` (random port).

**`ServerBuilder::fixture(f: Fixture) -> Self`** — Append one fixture. First-match-wins at request time.

**`ServerBuilder::fixtures(fixtures: Vec<Fixture>) -> Self`** — Batch-append fixtures; use after `load_yaml` / `load_yaml_dir`.

**`ServerBuilder::bind(addr: &str) -> Self`** — Override bind address; supports IPv4 (`host:port`), IPv6 (`[::1]:port`), and hostnames.

**`ServerBuilder::with_bearer_token(token: &str) -> Self`** — Add an unlimited-use bearer token; implicitly enables auth enforcement.

**`ServerBuilder::with_bearer_token_uses(token: &str, max_uses: u64) -> Self`** — Add a token that expires after `max_uses` requests.

**`ServerBuilder::build() -> Result<MockServer, _>`** — Async. Validates fixtures, binds TCP, starts server. Returns the `MockServer` handle.

**`MockServer::url() -> String`** — Returns the base URL, e.g. `http://127.0.0.1:PORT`. Server stops when the handle is dropped.

**`Fixture::new() -> Fixture`** — Create an empty fixture with no match rules and no response.

**`Fixture::match_user_message(pattern: &str) -> Self`** — Substring match on the last user message content.

**`Fixture::match_model(pattern: &str) -> Self`** — Substring match on the request `model` field.

**`Fixture::respond_with_content(content: &str) -> Self`** — Set a plain text response; clears any tool calls.

**`Fixture::respond_with_tool_calls(tool_calls: Vec<ToolCall>) -> Self`** — Set a tool-use response; clears any content.

**`Fixture::with_error(status: u16, message: &str) -> Self`** — Return an HTTP error (400–599) with `{ error: { message } }` body.

**`Fixture::with_streaming(latency: Option<u64>, chunk_size: Option<usize>) -> Self`** — Enable SSE mode; `latency` is ms between chunks, `chunk_size` is characters per chunk.