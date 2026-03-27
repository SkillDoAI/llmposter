---
name: llmposter
description: A Rust library for running mock LLM API servers (OpenAI, Anthropic, Gemini, Responses) in integration tests, with fixture matching, streaming simulation, failure injection, bearer token auth, and OAuth support.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---

## Imports

```rust
use llmposter::{Fixture, FailureConfig, ServerBuilder, ToolCall};
```

```toml
[dev-dependencies]
llmposter = "0.4.0"
# oauth feature is enabled by default — disable if not needed:
# llmposter = { version = "0.4.0", default-features = false }
reqwest = { version = "0.13", default-features = false, features = ["json"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
```

**MSRV: Rust 1.89+** (required since v0.4.0 by the `oauth-mock` dependency).

## Core Patterns

### Basic Text Response ✅ Current

Spin up a mock Anthropic endpoint. The server binds on a random port and shuts down when `MockServer` is dropped.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_basic_anthropic_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from the mock!"),
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
            "messages": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["content"][0]["text"], "Hi from the mock!");
    assert_eq!(body["stop_reason"], "end_turn");
    assert!(body["id"].as_str().unwrap().starts_with("msg-llmposter-"));
    // Unmatched requests → 404 {"error": {"message": "No fixture matched"}}
}
```

`match_user_message` performs a **substring** match on the last user message. The same fixture works on `/v1/chat/completions` (OpenAI) and `/v1beta/models/{model}:generateContent` (Gemini) without modification, unless `.for_provider()` is set.

### SSE Streaming Response ✅ Current

Enable SSE streaming with `.with_streaming(latency_ms, chunk_size)`. Include `"stream": true` in the request body.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_streaming_text() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("stream")
                .respond_with_content("Hello streaming world")
                .with_streaming(Some(0), Some(5)), // 0ms latency, 5 chars per chunk
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "stream": true,
            "messages": [{"role": "user", "content": "stream this"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: content_block_delta"));
    assert!(body.contains("event: message_stop"));
}
```

Total simulated latency ≈ `ceil(content_len / chunk_size) × latency_ms`. Both `latency_ms` and `chunk_size` accept `None` for defaults.

### Failure Injection ✅ Current

Use `with_failure(FailureConfig)` for network-level faults on otherwise-valid responses. Use `with_error(status, message)` for HTTP error codes. The two are **mutually exclusive** per fixture.

```rust
use llmposter::{Fixture, FailureConfig, ServerBuilder};

#[tokio::test]
async fn test_latency_injection() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("slow response")
                .with_failure(FailureConfig {
                    latency_ms: Some(200),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let start = std::time::Instant::now();
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "ping"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let elapsed = u64::try_from(start.elapsed().as_millis()).expect("duration fits u64");
    assert!(elapsed >= 180, "expected ≥180ms, got {elapsed}ms");
}

#[tokio::test]
async fn test_http_error_code() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("rate limit")
                .with_error(429, "Rate limit exceeded"),
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "rate limit me"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 429);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["message"], "Rate limit exceeded");
}
```

### Tool-Use Response ✅ Current

```rust
use llmposter::{Fixture, ServerBuilder, ToolCall};

#[tokio::test]
async fn test_tool_call_response() {
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
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "what's the weather in London?"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["stop_reason"], "tool_use");
    assert_eq!(body["content"][0]["type"], "tool_use");
    assert_eq!(body["content"][0]["name"], "get_weather");
    // Assert by prefix only — IDs use a server-wide counter for global uniqueness
    assert!(body["content"][0]["id"].as_str().unwrap().starts_with("toolu_llmposter_"));
}
```

`ToolCall.arguments` must be a JSON **object**. Non-object arguments (strings, arrays) are rejected at fixture load time. `respond_with_content` and `respond_with_tool_calls` are mutually exclusive.

### Bearer Token Authentication ✅ Current

`with_auth(true)` must be called explicitly — registering a token alone does not enforce authentication.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_bearer_auth_enforcement() {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token("test-secret-token")
        .fixture(Fixture::new().respond_with_content("authenticated"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "hello"}]
    });

    // Without token → 401 (provider-specific error shape per endpoint)
    let unauth = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401);

    // With valid Bearer token → 200
    let auth = client
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth("test-secret-token")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(auth.status(), 200);
}
```

## Configuration

### YAML Fixture Files

Load fixtures from YAML files for CI reproducibility. All YAML structs use `#[serde(deny_unknown_fields)]` — typos in field names cause a **load-time error**, not silent ignoring.

```yaml
# fixtures/responses.yaml — order from most specific to most general
- match:
    user_message: "weather in London"
  response:
    content: "It is 15°C in London."
- match:
    user_message: "weather"
  response:
    content: "I can look up weather for any city."
- response:
    content: "Default fallback."
  failure:
    latency_ms: 25
```

```rust
use std::path::Path;
use llmposter::ServerBuilder;

#[tokio::test]
async fn test_yaml_fixtures() {
    let server = ServerBuilder::new()
        .load_yaml(Path::new("fixtures/responses.yaml"))
        .unwrap() // fixture validation error
        .build()
        .await
        .unwrap();
    assert!(server.url().starts_with("http://"));
}
```

Use `load_yaml_dir(&Path)` to load all `.yaml` files from a directory.

### `FailureConfig` Fields

| Field | Type | Effect |
|---|---|---|
| `latency_ms` | `Option<u64>` | Delay before sending the response |
| `corrupt_body` | `Option<bool>` | Replace body with literal `"overloaded"` (status 200, `text/plain`) |
| `truncate_after_frames` | `Option<u32>` | Cut SSE stream after N frames; prevents `event: message_stop` |
| `disconnect_after_ms` | `Option<u64>` | Drop connection after N ms; prevents `event: message_stop` |

Always use `..FailureConfig::default()` to fill unset fields. `failure` and `error` are mutually exclusive.

### OAuth Feature Flag

The `oauth` feature is enabled by default. Disable with:

```toml
[dev-dependencies]
llmposter = { version = "0.4.0", default-features = false }
```

When `oauth` is enabled, `with_oauth_defaults()` provides pre-configured OAuth endpoints with `client_id="mock-client"` and `client_secret="mock-secret"`. Tokens issued by the OAuth mock are automatically valid on LLM endpoints.

### Server Response Headers

Every response includes `x-request-id: req-llmposter-{N}`. HTTP 429 responses include provider-specific rate-limit headers: `x-ratelimit-*` for OpenAI/Responses, `anthropic-ratelimit-*` for Anthropic, `retry-after` for Gemini.

## Pitfalls

### Auth Not Enforced Without `with_auth(true)`

#### Wrong
```rust
// Token is registered but NOT enforced — every request succeeds
ServerBuilder::new()
    .with_bearer_token("secret")
    .fixture(Fixture::new().respond_with_content("hello"))
    .build()
    .await
    .unwrap();
```

#### Right
```rust
// Auth enforcement requires the explicit opt-in
ServerBuilder::new()
    .with_auth(true)
    .with_bearer_token("secret")
    .fixture(Fixture::new().respond_with_content("hello"))
    .build()
    .await
    .unwrap();
```

### Broad Fixtures Placed Before Specific Ones

#### Wrong
```rust
// "weather" matches "weather in London" first — specific fixture is unreachable
ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("weather").respond_with_content("generic"))
    .fixture(Fixture::new().match_user_message("weather in London").respond_with_content("London"))
    .build()
    .await
    .unwrap();
```

#### Right
```rust
// Most specific match registered first
ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("weather in London").respond_with_content("London"))
    .fixture(Fixture::new().match_user_message("weather").respond_with_content("generic"))
    .build()
    .await
    .unwrap();
```

### Fragile Tool-Call ID Assertions

#### Wrong
```rust
// Counter-based assertion breaks under concurrent or multi-turn tests
assert_eq!(body["content"][0]["id"], "toolu_llmposter_1");
```

#### Right
```rust
// Assert by prefix and uniqueness — IDs use a server-wide counter
let id = body["content"][0]["id"].as_str().unwrap();
assert!(id.starts_with("toolu_llmposter_"));
```

### Using Deprecated `truncate_after_chunks` in Rust Code

#### Wrong
```rust
// truncate_after_chunks is a YAML serde alias only — not a valid Rust field name
let failure = FailureConfig {
    truncate_after_chunks: Some(2), // compile error: no field named `truncate_after_chunks`
    ..FailureConfig::default()
};
```

#### Right
```rust
use llmposter::FailureConfig;

let failure = FailureConfig {
    truncate_after_frames: Some(2), // correct Rust field name
    ..FailureConfig::default()
};
```

### Silent Duration Truncation with `as u64`

#### Wrong
```rust
// as_millis() returns u128; silently truncates if value > u64::MAX
let elapsed_ms = start.elapsed().as_millis() as u64;
```

#### Right
```rust
let elapsed_ms = u64::try_from(start.elapsed().as_millis())
    .expect("duration fits u64");
```

### Empty or Invalid Match Patterns Rejected at Validation

Empty substring patterns, empty regex patterns, and regex patterns whose compiled DFA state machine exceeds 1 MB are all rejected at fixture validation time — `ServerBuilder::build()` returns an error and the server does not start. Overly complex regex patterns are rejected to prevent OOM from malicious or accidentally unbounded patterns.

#### Wrong
```yaml
# empty substring pattern — load-time validation error
- match:
    user_message: ""
  response:
    content: "catch-all"
```

```yaml
# empty regex pattern — load-time validation error
- match:
    user_message:
      regex: ""
  response:
    content: "catch-all"
```

#### Right
```yaml
# omit match entirely for a true catch-all fixture
- response:
    content: "catch-all fallback"
```

### Blank Tool Name Rejected at Validation

A `ToolCall` with a blank `name` field is rejected at fixture validation time and prevents server start.

#### Wrong
```rust
ToolCall {
    name: "".to_string(), // blank name — load-time validation error
    arguments: serde_json::json!({}),
}
```

#### Right
```rust
ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!({"location": "London"}),
}
```

### Gemini Safety-Blocked Responses May Omit `role`

When Gemini returns a safety-blocked candidate, the `Content.role` field may be absent. Deserializing a Gemini response with `role: String` will fail on safety-blocked content.

#### Wrong
```rust
#[derive(serde::Deserialize)]
struct Content {
    role: String,  // fails when role is absent — e.g. safety-blocked candidates
    parts: Vec<Part>,
}
```

#### Right
```rust
#[derive(serde::Deserialize)]
struct Content {
    role: Option<String>,  // role is absent on safety-blocked responses
    parts: Vec<Part>,
}
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3

### MSRV Bump (v0.3.x → v0.4.0)

MSRV raised to **Rust 1.89** for `oauth-mock`. Update toolchain:

```text
rustup update stable
```

Update `rust-toolchain.toml` or CI to require `channel = "1.89"` or later.

### Responses API Streaming Protocol (v0.3.3 → v0.3.4)

`response.done` was removed as non-spec. Streams now emit `response.in_progress` with nested `response` envelopes and `sequence_number` fields.

**Before:**
```rust
// response.done is no longer emitted — this branch never fires
if event.event == "response.done" {
    break;
}
```

**After:**
```rust
if event.event == "response.in_progress" {
    // process event data; use sequence_number for ordering
}
```

### Error Response Shape (v0.3.3 → v0.3.4)

`code` changed from integer to `String`; `param: Option<String>` and `type: String` fields added to match real OpenAI error shape.

**Before:**
```rust
#[derive(serde::Deserialize)]
struct ApiError {
    code: u32,
    message: String,
}
```

**After:**
```rust
#[derive(serde::Deserialize)]
struct ApiError {
    r#type: String,
    code: String,
    message: String,
    param: Option<String>,
}
```

## API Reference

**`ServerBuilder::new()`** — Creates a server builder with no fixtures. Chain configuration methods and call `.build().await` to start.

**`ServerBuilder::fixture(Fixture)`** — Registers one fixture. Fixtures are evaluated in registration order; first match wins.

**`ServerBuilder::build()`** — Async. Validates all fixtures and binds the server port. Returns `Result<MockServer, Box<dyn std::error::Error>>`.

**`ServerBuilder::with_auth(bool)`** — Enables Bearer token enforcement on all LLM endpoints. Must be `true` for token checks to run.

**`ServerBuilder::with_bearer_token(&str)`** — Registers a Bearer token with unlimited uses. Does not enforce auth on its own; call `with_auth(true)` to activate enforcement.

**`ServerBuilder::with_bearer_token_uses(&str, u64)`** — Registers a token that expires after `max_uses` requests; subsequent calls return 401.

**`ServerBuilder::load_yaml(&Path)`** — Loads and validates fixtures from a YAML file. Returns `Result<ServerBuilder, Box<dyn std::error::Error>>`.

**`MockServer::url()`** — Returns the base URL (e.g. `http://127.0.0.1:PORT`). Server shuts down when `MockServer` is dropped.

**`Fixture::new()`** — Creates a fixture with no constraints. Matches any provider and any request content.

**`Fixture::match_user_message(&str)`** — Adds a substring match constraint on the last user message.

**`Fixture::respond_with_content(&str)`** — Sets a text response body. Mutually exclusive with `respond_with_tool_calls`.

**`Fixture::respond_with_tool_calls(Vec<ToolCall>)`** — Sets a tool-call response. `ToolCall { name: String, arguments: serde_json::Value }` — `arguments` must be a JSON object.

**`Fixture::with_streaming(Option<u64>, Option<usize>)`** — Enables SSE streaming. Parameters: per-chunk latency in ms, chunk size in chars.

**`Fixture::with_failure(FailureConfig)`** — Injects network-level faults (latency, corrupt body, stream truncation, disconnect). Mutually exclusive with `with_error`.

**`Fixture::with_error(u16, &str)`** — Returns an HTTP error with `{"error": {"message": "..."}}`. Status must be 400–599.
