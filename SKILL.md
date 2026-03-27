---
name: llmposter
description: In-process mock LLM server for Rust integration tests — serves OpenAI, Anthropic, and Gemini API endpoints with configurable fixtures, SSE streaming, bearer auth, and failure injection.
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---


## Imports

```rust
// Crate-root re-exports
use llmposter::{Fixture, ServerBuilder};

// Fixture submodule types
use llmposter::fixture::{FailureConfig, ToolCall};
```

```toml
# Cargo.toml — add as a dev dependency
[dev-dependencies]
llmposter = "0.4.0"
reqwest = { version = "0.13", default-features = false, features = ["json"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
```

## Core Patterns

### Text Response — Anthropic Endpoint ✅ Current

Start the mock server and assert on the Anthropic `/v1/messages` response shape. Fixtures are evaluated in registration order; the first match wins. Unmatched requests return 404.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_anthropic_text() -> Result<(), Box<dyn std::error::Error>> {
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

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["content"][0]["text"], "Hi from Claude mock!");
    assert_eq!(body["stop_reason"], "end_turn");
    assert!(body["id"].as_str().unwrap().starts_with("msg-llmposter-"));
    Ok(())
}
```

### SSE Streaming Response ✅ Current

Enable streaming via `with_streaming(latency, chunk_size)` and pass `"stream": true` in the request body. `latency` is milliseconds between SSE frames; `chunk_size` is characters per delta frame. Pass `Some(0)` for zero-latency in unit tests.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_sse_streaming() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("Hello world response")
                .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await?;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "stream test"}],
            "stream": true
        }))
        .send()
        .await?;

    assert_eq!(resp.status().as_u16(), 200);
    let body = resp.text().await?;
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: content_block_delta"));
    assert!(body.contains("event: message_stop"));
    Ok(())
}
```

### Tool Call Response ✅ Current

Return a tool-use block. `ToolCall::arguments` must be a JSON object — arrays, strings, and other types are rejected at fixture validation.

```rust
use llmposter::{Fixture, ServerBuilder};
use llmposter::fixture::ToolCall;

#[tokio::test]
async fn test_tool_call() -> Result<(), Box<dyn std::error::Error>> {
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
            "messages": [{"role": "user", "content": "what is the weather?"}]
        }))
        .send()
        .await?;

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["stop_reason"], "tool_use");
    assert_eq!(body["content"][0]["type"], "tool_use");
    assert_eq!(body["content"][0]["name"], "get_weather");
    assert_eq!(body["content"][0]["id"], "toolu_llmposter_1");
    Ok(())
}
```

### Error and Failure Injection ✅ Current

Use `with_error` for HTTP status errors. Use `with_failure` for network-level chaos (latency, body corruption, stream truncation, disconnect) on otherwise valid responses.

```rust
use llmposter::{Fixture, ServerBuilder};
use llmposter::fixture::FailureConfig;

#[tokio::test]
async fn test_http_error() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().with_error(429, "Rate limit exceeded"))
        .build()
        .await?;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await?;

    assert_eq!(resp.status().as_u16(), 429);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["error"]["message"], "Rate limit exceeded");
    Ok(())
}

#[tokio::test]
async fn test_network_latency() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("delayed response")
                .with_failure(FailureConfig {
                    latency_ms: Some(200),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await?;

    let client = reqwest::Client::new();
    let start = std::time::Instant::now();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "test"}]
        }))
        .send()
        .await?;

    assert_eq!(resp.status().as_u16(), 200);
    assert!(start.elapsed().as_millis() >= 180);
    Ok(())
}

#[tokio::test]
async fn test_stream_truncation() -> Result<(), Box<dyn std::error::Error>> {
    // truncate_after_frames: N — the SSE stream sends N delta frames then disconnects;
    // the response body is truncated mid-stream before message_stop is emitted.
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("This is a long response that will be truncated mid-stream")
                .with_streaming(Some(0), Some(5))
                .with_failure(FailureConfig {
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
            "messages": [{"role": "user", "content": "test truncation"}],
            "stream": true
        }))
        .send()
        .await?;

    assert_eq!(resp.status().as_u16(), 200);
    let body = resp.text().await?;
    // SSE stream terminates before full content is delivered
    assert!(!body.contains("event: message_stop"));
    Ok(())
}
```

### Bearer Auth ✅ Current

`with_auth(true)` enforces token validation on all LLM endpoints. Unauthenticated requests receive a provider-specific HTTP 401. Use `with_bearer_token_uses` to test token-refresh flows.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_bearer_auth() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token("test-bearer-token")
        .fixture(Fixture::new().respond_with_content("authenticated"))
        .build()
        .await?;

    let client = reqwest::Client::new();

    // Valid token — succeeds
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth("test-bearer-token")
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await?;
    assert_eq!(resp.status().as_u16(), 200);

    // Missing token — rejected
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await?;
    assert_eq!(resp.status().as_u16(), 401);
    Ok(())
}
```

### Token-Limited Auth ✅ Current

Use `with_bearer_token_uses` to verify token-refresh flows. The first `max_uses` requests succeed; the `(max_uses + 1)`th request returns HTTP 401.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_token_exhaustion() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .with_bearer_token_uses("one-time-token", 1)
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await?;

    let client = reqwest::Client::new();
    let url = format!("{}/v1/messages", server.url());
    let payload = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "hello"}]
    });

    // First request — within limit, succeeds
    let resp = client
        .post(&url)
        .bearer_auth("one-time-token")
        .json(&payload)
        .send()
        .await?;
    assert_eq!(resp.status().as_u16(), 200);

    // Second request — token exhausted → 401
    let resp = client
        .post(&url)
        .bearer_auth("one-time-token")
        .json(&payload)
        .send()
        .await?;
    assert_eq!(resp.status().as_u16(), 401);
    Ok(())
}
```

## Configuration

### Bind Address

Default: `127.0.0.1:0` (OS-assigned ephemeral port). Retrieve the actual port via `server.url()`. Override with `bind`:

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_custom_bind() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .bind("127.0.0.1:0")   // explicit random port; omit this line for identical behavior
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await?;

    println!("Server at {}", server.url()); // e.g. http://127.0.0.1:54321
    Ok(())
}
```

### YAML Fixture Files

Load fixtures from YAML files instead of constructing them in code. Unknown fields cause a parse error at load time (deny_unknown_fields). Double-escape regex backslashes in YAML.

```yaml
# tests/fixtures/responses.yaml
- match_rule:
    user_message: "hello"
  response:
    content: "Hi from YAML!"
- match_rule:
    user_message:
      regex: "stock price of \\w+"   # double-escape backslashes
  response:
    content: "Stock data unavailable."
- error:
    status: 429
    message: "Rate limit exceeded"
```

```rust
use llmposter::ServerBuilder;
use std::path::Path;

#[tokio::test]
async fn test_yaml_fixtures() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .load_yaml(Path::new("tests/fixtures/responses.yaml"))?
        .build()
        .await?;

    let _ = server.url();
    Ok(())
}
```

Use `load_yaml_dir(dir)` to load every `.yaml` file from a directory. Regex patterns are compiled at load time with a 1 MB DFA size cap; patterns that would produce a DFA larger than 1 MB are rejected with a load-time error to prevent OOM.

### Verbose Mode

When enabled, 404 responses include a diagnostic error body instead of empty response — useful during fixture authoring.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_verbose_no_match() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .verbose(true)
        .fixture(Fixture::new().match_user_message("specific").respond_with_content("ok"))
        .build()
        .await?;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "unmatched request"}]
        }))
        .send()
        .await?;

    assert_eq!(resp.status().as_u16(), 404);
    let body: serde_json::Value = resp.json().await?;
    assert!(body["error"]["message"].as_str().unwrap().contains("No fixture matched"));
    Ok(())
}
```

### oauth Feature

The `oauth` Cargo feature (on by default in v0.4.0) adds `with_oauth` and `with_oauth_defaults`. Requires Rust ≥ 1.89.

```toml
[dev-dependencies]
llmposter = { version = "0.4.0", features = ["oauth"] }
```

```rust
use llmposter::{Fixture, ServerBuilder};

#[cfg(feature = "oauth")]
#[tokio::test]
async fn test_oauth_defaults() -> Result<(), Box<dyn std::error::Error>> {
    // Default creds: client_id=mock-client, client_secret=mock-secret
    let server = ServerBuilder::new()
        .with_oauth_defaults()
        .fixture(Fixture::new().respond_with_content("oauth ok"))
        .build()
        .await?;

    let _ = server.url();
    Ok(())
}
```

Tokens issued by the embedded OAuth server are automatically accepted on all LLM endpoints — no additional call to `with_bearer_token` is required. Exchange client credentials at the OAuth server's token endpoint; the returned access token can be sent as `Authorization: Bearer <token>` on any LLM endpoint of the same server instance.

## Server Behavior

### Request ID Header

Every response includes an `x-request-id` header with a deterministic counter value:

```text
x-request-id: req-llmposter-1
x-request-id: req-llmposter-2
```

This header is present on all responses, including 404, 413, and error responses.

```rust
let resp = client.post(format!("{}/v1/messages", server.url())).json(&body).send().await?;
let req_id = resp.headers()["x-request-id"].to_str().unwrap();
assert!(req_id.starts_with("req-llmposter-"));
```

### Rate Limit Headers on 429 Responses

When a fixture returns HTTP 429 via `with_error(429, ...)`, the server includes provider-specific rate limit headers:

- **OpenAI and Responses endpoints**: `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, `x-ratelimit-reset-requests`
- **Anthropic endpoint**: `anthropic-ratelimit-requests-limit`, `anthropic-ratelimit-requests-remaining`, `anthropic-ratelimit-requests-reset`
- **Gemini endpoint**: `retry-after` only

### Request Body Size Limit (413)

Requests exceeding the body size limit return HTTP 413. The `x-request-id` header is still present on 413 responses — the size limit is enforced at an inner router layer, after the request ID is assigned.

### Gemini Safety-Blocked Content

When the Gemini provider returns a response where content is blocked by safety filters, the `role` field in the `Candidate` object may be absent. Deserialization succeeds — the field is treated as optional.

## Pitfalls

### Wrong: Calling `with_auth(false)` after `with_bearer_token` — overrides the implicit auth enable

```rust
use llmposter::{Fixture, ServerBuilder};

// with_bearer_token implicitly enables auth, but with_auth(false) overrides it —
// unauthenticated requests are not rejected
let server = ServerBuilder::new()
    .with_bearer_token("secret-token")
    .with_auth(false)   // ❌ disables the implicit auth from with_bearer_token
    .fixture(Fixture::new().respond_with_content("unexpectedly open"))
    .build()
    .await?;
// Requests without Authorization header → 200 (auth not enforced)
```

### Right: `with_bearer_token` implicitly enables auth — do not follow it with `with_auth(false)`

```rust
use llmposter::{Fixture, ServerBuilder};

let server = ServerBuilder::new()
    .with_bearer_token("secret-token")
    .fixture(Fixture::new().respond_with_content("protected"))
    .build()
    .await?;
// Requests without Authorization: Bearer secret-token → 401
```

---

### Wrong: Using `with_failure` to set an HTTP error status code

```rust
use llmposter::{Fixture, ServerBuilder};
use llmposter::fixture::FailureConfig;

// with_failure does not set HTTP status — this returns 200 with corrupted body,
// not the 429 you intended
let server = ServerBuilder::new()
    .fixture(
        Fixture::new()
            .respond_with_content("placeholder")
            .with_failure(FailureConfig {
                corrupt_body: Some(true),
                ..FailureConfig::default()
            }),
    )
    .build()
    .await?;
// resp.status() == 200, body == "overloaded"
```

### Right: Use `with_error` to return HTTP status codes

```rust
use llmposter::{Fixture, ServerBuilder};

let server = ServerBuilder::new()
    .fixture(Fixture::new().with_error(429, "Rate limit exceeded"))
    .build()
    .await?;
// resp.status() == 429
```

---

### Wrong: Tool call arguments as a non-object JSON value

```rust
use llmposter::{Fixture, ServerBuilder};
use llmposter::fixture::ToolCall;

// arguments must be a JSON object — a string is rejected at fixture validation
let fixture = Fixture::new().respond_with_tool_calls(vec![ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!("San Francisco"), // ❌ string — validation fails
}]);
// ServerBuilder::build().await will return an error
```

### Right: Tool call arguments must be a JSON object

```rust
use llmposter::{Fixture, ServerBuilder};
use llmposter::fixture::ToolCall;

let fixture = Fixture::new().respond_with_tool_calls(vec![ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!({   // ✅ JSON object
        "location": "San Francisco",
        "unit": "celsius"
    }),
}]);
```

---

### Wrong: Broad/catch-all fixture placed before specific fixtures

```rust
use llmposter::{Fixture, ServerBuilder};

// Catch-all matches first — specific fixture below it is never reached
let server = ServerBuilder::new()
    .fixture(Fixture::new().respond_with_content("catch-all"))
    .fixture(Fixture::new().match_user_message("hello").respond_with_content("specific"))
    .build()
    .await?;
// Request with "hello" → "catch-all" (wrong)
```

### Right: Place specific fixtures first, broad fallback last

```rust
use llmposter::{Fixture, ServerBuilder};

let server = ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("hello").respond_with_content("specific"))
    .fixture(Fixture::new().respond_with_content("catch-all"))
    .build()
    .await?;
// Request with "hello" → "specific" (correct)
```

---

### Wrong: Single-escaped backslash in YAML regex patterns

```yaml
# YAML consumes the backslash — regex engine receives an invalid pattern
match_rule:
  user_message:
    regex: "stock price of \w+"
```

### Right: Double-escape backslashes in YAML regex patterns

```yaml
# Double-escape so regex engine receives \w+
match_rule:
  user_message:
    regex: "stock price of \\w+"
```

---

### Wrong: Empty substring or regex pattern — rejected at fixture validation

```rust
use llmposter::{Fixture, ServerBuilder};

// Empty pattern is rejected at build time — prevents an unintentional catch-all
// that would silently match every request
let server = ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("").respond_with_content("bad"))
    .build()
    .await;
// Err(...) — empty substring match rejected by Fixture::validate
```

### Right: Provide a non-empty match pattern, or omit `match_user_message` entirely for a deliberate catch-all

```rust
use llmposter::{Fixture, ServerBuilder};

// Intentional catch-all: omit the match rule
let server = ServerBuilder::new()
    .fixture(Fixture::new().respond_with_content("catch-all"))
    .build()
    .await?;
```

---

### Wrong: Blank tool name in a `ToolCall` — rejected at fixture validation

```rust
use llmposter::{Fixture, ServerBuilder};
use llmposter::fixture::ToolCall;

// Blank name is rejected at build time
let fixture = Fixture::new().respond_with_tool_calls(vec![ToolCall {
    name: "".to_string(),   // ❌ blank name — validation fails
    arguments: serde_json::json!({"location": "London"}),
}]);
// ServerBuilder::build().await will return an error
```

### Right: Provide a non-empty tool name

```rust
use llmposter::{Fixture, ServerBuilder};
use llmposter::fixture::ToolCall;

let fixture = Fixture::new().respond_with_tool_calls(vec![ToolCall {
    name: "get_weather".to_string(),   // ✅ non-blank name
    arguments: serde_json::json!({"location": "London"}),
}]);
```

---

### Wrong: Error status outside 400–599 — rejected at fixture validation

```rust
use llmposter::{Fixture, ServerBuilder};

// Status 200 is not a valid error status — rejected at build time
let server = ServerBuilder::new()
    .fixture(Fixture::new().with_error(200, "not an error"))
    .build()
    .await;
// Err(...) — status 200 is outside the accepted 400-599 range
```

### Right: Use a client or server error status code (400–599)

```rust
use llmposter::{Fixture, ServerBuilder};

let server = ServerBuilder::new()
    .fixture(Fixture::new().with_error(503, "Service unavailable"))
    .build()
    .await?;
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3

### v0.3.3 → v0.3.4: Responses API SSE Protocol (Breaking)

The `response.done` event was removed as non-spec. The stream now uses `response.in_progress` with nested response envelopes and a `sequence_number` field.

**Before (v0.3.3):**

```text
event: response.done
data: {"type": "response.done"}
```

**After (v0.3.4+):**

```text
event: response.in_progress
data: {"type": "response.in_progress", "response": {...}, "sequence_number": 1}
```

Remove `response.done` event handlers; add `response.in_progress` handling for nested response envelopes.

### v0.3.3 → v0.3.4: Error Response Shape (Breaking)

Error bodies now match the real OpenAI shape. The `code` field is a string, `type` is the error category string, and `param` is always present (may be `null`).

**Before:**

```json
{"error": {"code": 429, "message": "Rate limited"}}
```

**After:**

```json
{"error": {"type": "rate_limit_error", "code": "rate_limit_exceeded", "message": "Rate limited", "param": null}}
```

Update error body assertions: check `code` as a string, verify `param` field exists (nullable), use `type` for error category.

### v0.3.x → v0.4.0: MSRV and New Auth APIs

Minimum Supported Rust Version raised to **1.89** (required by `oauth-mock 0.4`). Update `rust-toolchain.toml` or CI matrices pinned below 1.89.

The new auth APIs (`with_auth`, `with_bearer_token`, `with_bearer_token_uses`, `with_oauth`, `with_oauth_defaults`) are additive. Existing server configurations that do not use auth are unaffected.

YAML fixture field `truncate_after_chunks` is soft-deprecated in favour of `truncate_after_frames` (doc comment only; both continue to work).

## API Reference

**`ServerBuilder::new()`** — Create a mock server builder. Default bind is `127.0.0.1:0` (random port).

**`ServerBuilder::fixture(f: Fixture) -> ServerBuilder`** — Register one fixture. Fixtures are evaluated in registration order; first match wins.

**`ServerBuilder::fixtures(fixtures: Vec<Fixture>) -> ServerBuilder`** — Bulk-register fixtures in one call.

**`ServerBuilder::verbose(v: bool) -> ServerBuilder`** — When `true`, unmatched requests return `{"error": {"message": "No fixture matched"}}` body instead of empty 404.

**`ServerBuilder::with_auth(enabled: bool) -> ServerBuilder`** — Explicitly enable or disable bearer token enforcement on all LLM endpoints.

**`ServerBuilder::with_bearer_token(token: &str) -> ServerBuilder`** — Register a bearer token with unlimited uses. Implicitly enables auth; `with_auth(true)` is not required.

**`ServerBuilder::with_bearer_token_uses(token: &str, max_uses: u64) -> ServerBuilder`** — Register a bearer token that expires after `max_uses` requests. Implicitly enables auth; `with_auth(true)` is not required. Use to test token-refresh flows.

**`ServerBuilder::load_yaml(path: &Path) -> Result<ServerBuilder, _>`** — Load fixtures from a single YAML file; validates on load. Unknown fields cause a parse error.

**`ServerBuilder::build() -> Result<MockServer, _>`** — Async. Validates all fixtures, binds the socket, spawns the server. The server stops when the returned handle is dropped.

**`MockServer::url(&self) -> String`** — Returns the base URL of the running server, e.g. `http://127.0.0.1:PORT`.

**`Fixture::new() -> Fixture`** — Create a fixture builder with all fields set to `None`. Also implements `Default`.

**`Fixture::match_user_message(pattern: &str) -> Fixture`** — Substring match against the last user message content. Builder method (consumes self).

**`Fixture::respond_with_content(content: &str) -> Fixture`** — Set a text response; clears any previously set `tool_calls`.

**`Fixture::with_error(status: u16, message: &str) -> Fixture`** — Return a provider-specific HTTP error for status codes 400–599. Values outside this range are rejected at fixture validation time. Mutually exclusive with setting a response (`respond_with_content`, `respond_with_tool_calls`) and with `with_failure`.

**`Fixture::with_failure(failure: FailureConfig) -> Fixture`** — Inject network/streaming failures: latency (`latency_ms`), body corruption (`corrupt_body`), stream truncation (`truncate_after_frames`), or disconnect (`disconnect_after_ms`). Requires a response to also be set.
