---
name: llmposter
description: Fixture-driven deterministic mock server for OpenAI, Anthropic, Gemini, and Responses APIs, enabling hermetic in-process LLM integration tests without real network calls.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---

## Imports

```rust
use llmposter::{Fixture, FailureConfig, Provider, ServerBuilder, ToolCall};
use llmposter::fixture::{FixtureResponse, StreamingConfig};
```

```toml
[dev-dependencies]
llmposter = { version = "0.4.0" }
# OAuth mock endpoints are on by default; disable to reduce compile time:
# llmposter = { version = "0.4.0", default-features = false }
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.13", default-features = false, features = ["json"] }
serde_json = "1"
```

## Core Patterns

### Basic Mock Server Build ✅ Current

Start a server with a single fixture. The server binds to a random available port and stops when dropped.

```rust
mod basic_usage {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn should_return_fixture_content() {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("Hello from mock!"))
            .build()
            .await
            .unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["type"], "message");
        assert_eq!(body["content"][0]["text"], "Hello from mock!");
        assert_eq!(body["stop_reason"], "end_turn");
        assert!(body["id"].as_str().unwrap().starts_with("msg-llmposter-"));
    }
}
```

### Fixture Matching and Provider Filtering ✅ Current

Match on user message (substring), model name (substring), or restrict to a specific provider endpoint. First registered match wins; unmatched requests return 404.

```rust
mod fixture_matching {
    use llmposter::{Fixture, Provider, ServerBuilder};

    #[tokio::test]
    async fn should_route_by_message_and_provider() {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("weather")
                    .for_provider(Provider::Anthropic)
                    .respond_with_content("It's sunny today."),
            )
            .fixture(
                Fixture::new()
                    .match_user_message("weather")
                    .for_provider(Provider::OpenAI)
                    .respond_with_content("OpenAI: clear skies."),
            )
            .build()
            .await
            .unwrap();

        let client = reqwest::Client::new();

        // Anthropic endpoint — first fixture matches
        let hit = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "what's the weather?"}]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(hit.status(), 200);

        // No match — returns 404 with structured error
        let miss = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "unrelated question"}]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(miss.status(), 404);
        let err: serde_json::Value = miss.json().await.unwrap();
        assert!(err["error"]["message"].as_str().unwrap().contains("No fixture matched"));
    }
}
```

### Streaming Response ✅ Current

Enable SSE streaming with `.with_streaming(latency_ms, chunk_size)`. The inbound request must include `"stream": true`.

```rust
mod streaming_usage {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn should_emit_sse_events() {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("Hello, streaming world!")
                    .with_streaming(Some(0), Some(5)), // 0 ms latency, 5-char chunks
            )
            .build()
            .await
            .unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "stream": true,
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers()["content-type"].to_str().unwrap(),
            "text/event-stream"
        );
        let body = resp.text().await.unwrap();
        assert!(body.contains("event: message_start"));
        assert!(body.contains("event: content_block_delta"));
        assert!(body.contains("event: message_stop"));
    }
}
```

### Failure Simulation ✅ Current

`FailureConfig` injects network-level faults: added latency, corrupt body, stream truncation, or mid-stream disconnect. Always pair with a response.

```rust
mod failure_simulation {
    use llmposter::fixture::FailureConfig;
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn should_truncate_stream_mid_flight() {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("This long response will be cut short mid-stream.")
                    .with_streaming(Some(0), Some(5))
                    .with_failure(FailureConfig {
                        truncate_after_frames: Some(2),
                        ..FailureConfig::default()
                    }),
            )
            .build()
            .await
            .unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "stream": true,
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await
            .unwrap();

        let body = resp.text().await.unwrap();
        assert!(body.contains("event: message_start"));
        assert!(!body.contains("event: message_stop")); // truncated before completion
    }
}
```

### Tool Call Response ✅ Current

Return a tool-use response. `ToolCall::arguments` must be a JSON object. Assert on ID prefix, not exact counter value.

```rust
mod tool_call_usage {
    use llmposter::{Fixture, ServerBuilder, ToolCall};

    #[tokio::test]
    async fn should_return_tool_use_response() {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("weather in London")
                    .respond_with_tool_calls(vec![ToolCall {
                        name: "get_weather".to_string(),
                        arguments: serde_json::json!({"location": "London", "unit": "celsius"}),
                    }]),
            )
            .build()
            .await
            .unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "weather in London"}]
            }))
            .send()
            .await
            .unwrap();

        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["name"], "get_weather");
        let id = body["content"][0]["id"].as_str().unwrap();
        assert!(id.starts_with("toolu_llmposter_")); // assert prefix, not exact value
        assert_eq!(body["content"][0]["input"]["location"], "London");
    }
}
```

## Configuration

**Bind address:** defaults to `127.0.0.1` with a random port. Override with `.bind("127.0.0.1:8080")` or `"[::1]:0"` for IPv6.

**Verbose mode:** `.verbose(true)` enables server-side logging; does not change HTTP behavior.

**Loading fixtures from YAML:**

```rust
mod yaml_loading {
    use llmposter::ServerBuilder;
    use std::path::Path;

    #[tokio::test]
    async fn should_load_fixtures_from_directory() -> Result<(), Box<dyn std::error::Error>> {
        let _server = ServerBuilder::new()
            .load_yaml_dir(Path::new("tests/fixtures/"))?
            .build()
            .await?;
        Ok(())
    }
}
```

YAML fixture format:

```yaml
- match:
    user_message: "hello"              # substring match
    # user_message: { regex: "hel+" } # regex match
  response:
    content: "Hi there!"
- match:
    user_message: "rate limit"
  error:
    status: 429
    message: "Rate limit exceeded"
- response:
    content: "default response"        # no match rule = catch-all (register last)
```

**Bearer token auth:**

```rust
mod auth_usage {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn should_reject_unauthenticated_requests() {
        let server = ServerBuilder::new()
            .with_bearer_token("<test-token>") // implicitly enables auth
            .fixture(Fixture::new().respond_with_content("authenticated"))
            .build()
            .await
            .unwrap();

        let client = reqwest::Client::new();
        let unauth = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(unauth.status(), 401);
    }
}
```

**OAuth endpoints** (on by default via `oauth` feature; disable with `default-features = false`). Use `with_oauth_defaults()` for standard mock credentials (`client_id=mock-client`, `client_secret=mock-secret`).

**Fixture ordering:** first match wins — register specific fixtures before broad catch-all fixtures.

## Pitfalls

### Wrong: Empty match pattern silently catches all requests

```rust
// Rejected at ServerBuilder::build() in v0.3.5+, but silently matches everything on older versions
Fixture::new()
    .match_user_message("") // empty substring always matches — overrides all later fixtures
    .respond_with_content("catch-all")
```

### Right: Omit the match entirely for an intentional catch-all; use non-empty patterns otherwise

```rust
use llmposter::Fixture;

// intentional catch-all — no match rule
let catch_all = Fixture::new().respond_with_content("default response");

// specific match
let specific = Fixture::new()
    .match_user_message("hello")
    .respond_with_content("Hi!");
```

---

### Wrong: Confusing `with_error` and `with_failure`

```rust
use llmposter::fixture::FailureConfig;
use llmposter::Fixture;

// WRONG — with_error only accepts HTTP 400-599 status codes; it returns an error body,
// not a valid response with injected latency
let _ = Fixture::new()
    .match_user_message("slow")
    .with_error(200, "delayed"); // status 200 is outside 400-599 — rejected at validation
```

### Right: Use `with_error` for HTTP error codes; `with_failure` for network fault injection on a valid response

```rust
use llmposter::fixture::FailureConfig;
use llmposter::Fixture;

// HTTP 429 error body
let rate_limit = Fixture::new()
    .match_user_message("rate limit")
    .with_error(429, "Rate limit exceeded");

// 200ms latency on a valid response
let slow = Fixture::new()
    .match_user_message("slow")
    .respond_with_content("delayed response")
    .with_failure(FailureConfig {
        latency_ms: Some(200),
        ..FailureConfig::default()
    });
```

---

### Wrong: Tool-call arguments as a string or array

```rust
use llmposter::ToolCall;

// WRONG — rejected at fixture validation; arguments must be a JSON object
let bad = ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!("San Francisco"), // string, not object
};
```

### Right: Tool-call arguments must be a JSON object

```rust
use llmposter::ToolCall;

let good = ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!({"location": "San Francisco", "unit": "celsius"}),
};
```

---

### Wrong: Asserting exact tool-call ID counter values

```rust
// WRONG — counter changes whenever other tool calls are added or test order shifts
let id = body["content"][0]["id"].as_str().unwrap();
assert_eq!(id, "toolu_llmposter_1");
```

### Right: Assert on prefix and uniqueness only

```rust
let id1 = body1["content"][0]["id"].as_str().unwrap();
let id2 = body2["content"][0]["id"].as_str().unwrap();
assert!(id1.starts_with("toolu_llmposter_"));
assert_ne!(id1, id2);
```

---

### Wrong: Registering a broad catch-all fixture before specific ones

```rust
use llmposter::{Fixture, ServerBuilder};

// WRONG — the first (catch-all) fixture fires for every request; the specific one never matches
let _ = ServerBuilder::new()
    .fixture(Fixture::new().respond_with_content("default"))       // catch-all first
    .fixture(Fixture::new().match_user_message("hello").respond_with_content("Hi!")); // unreachable
```

### Right: Register specific fixtures first; catch-all last

```rust
use llmposter::{Fixture, ServerBuilder};

let _ = ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("hello").respond_with_content("Hi!"))
    .fixture(Fixture::new().respond_with_content("default")); // catch-all last
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3

### v0.3.3 → v0.3.4 (breaking)

**Responses API streaming protocol changed.** Events now use nested `response` envelopes with `sequence_number` and correlation fields. The non-spec `response.done` event was removed; `response.in_progress` was added.

Before (v0.3.3):
```
event: response.done
data: { ... }
```

After (v0.3.4+):
```
event: response.in_progress
data: { "response": { ... }, "sequence_number": 1 }
```

Update SSE frame assertions: check for `response.in_progress` instead of `response.done`; unwrap the nested `response` envelope when reading streamed event data.

**Error response format changed.** `code` is now a string (not an integer); `param` is always present (as `null` if unused).

Before:
```json
{ "error": { "message": "Rate limit", "type": "rate_limit_error" } }
```

After:
```json
{ "error": { "message": "Rate limit", "type": "requests", "code": "rate_limit_exceeded", "param": null } }
```

Update any test code that parses error response bodies against the old shape.

### v0.3.x → v0.4.0

**MSRV bumped to 1.89** (required by `oauth-mock` dependency). Update `rust-toolchain.toml` or CI matrix accordingly.

**`oauth` feature is on by default.** Disable it with `default-features = false` to reduce binary size and compile time when OAuth mock endpoints are not needed:

```toml
[dev-dependencies]
llmposter = { version = "0.4.0", default-features = false }
```

New builder methods (`with_auth`, `with_bearer_token`, `with_bearer_token_uses`, `with_oauth`, `with_oauth_defaults`) are additive — existing code without auth configuration continues to work with no authentication enforced.

## API Reference

**`ServerBuilder::new()`** — Creates a new server builder. Re-exported at crate root.

**`ServerBuilder::fixture(f: Fixture)`** — Registers a single fixture. Fixtures are evaluated in registration order; first match wins.

**`ServerBuilder::fixtures(v: Vec<Fixture>)`** — Registers multiple fixtures in a single call.

**`ServerBuilder::bind(addr: &str)`** — Sets the bind address, e.g., `"127.0.0.1:8080"` or `"[::1]:0"`. Defaults to `127.0.0.1` with a random port.

**`ServerBuilder::verbose(v: bool)`** — Enables server-side request/response logging. Does not affect HTTP behavior.

**`ServerBuilder::with_bearer_token(token: &str)`** — Adds a bearer token with unlimited uses and implicitly enables authentication.

**`ServerBuilder::build()`** *(async)* — Validates all fixtures, binds the port, starts the server, and returns `Result<MockServer, _>`. Consumes the builder.

**`MockServer::url()`** — Returns the server base URL as `"http://127.0.0.1:PORT"`.

**`Fixture::new()`** — Creates an empty fixture with all fields set to `None`.

**`Fixture::match_user_message(pattern: &str)`** — Restricts the fixture to requests whose last user message *contains* `pattern` (substring match). Rejected if `pattern` is empty.

**`Fixture::match_model(pattern: &str)`** — Restricts the fixture to requests whose `model` field *contains* `pattern`.

**`Fixture::respond_with_content(content: &str)`** — Sets a text response body. Mutually exclusive with `respond_with_tool_calls`.

**`Fixture::with_error(status: u16, message: &str)`** — Returns an HTTP error response. `status` must be 400–599. Mutually exclusive with response and failure config.

**`Fixture::with_failure(failure: FailureConfig)`** — Injects a network fault (latency, corrupt body, stream truncation, or disconnect). Requires a response to also be set. Mutually exclusive with `with_error`.

**`Fixture::for_provider(provider: Provider)`** — Restricts the fixture to a specific provider endpoint (`Provider::OpenAI`, `Provider::Anthropic`, `Provider::Gemini`, `Provider::Responses`). Without this call, the fixture matches all provider endpoints.
