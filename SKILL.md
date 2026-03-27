---
name: llmposter
description: In-process HTTP mock server for LLM APIs (OpenAI, Anthropic, Gemini, Responses) that serves fixture-driven responses, SSE streaming simulation, and bearer/OAuth authentication for integration testing.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---

## Imports

```rust
use llmposter::{FailureConfig, Fixture, MockServer, Provider, ServerBuilder, ToolCall};
```

```toml
# Cargo.toml — add to [dev-dependencies]
[dev-dependencies]
llmposter = "0.4"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.13", default-features = false, features = ["json"] }
serde_json = "1"
```

## Core Patterns

### Basic text response fixture ✅ Current

Start a mock server, match on a user message substring, assert the Anthropic-shaped response.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_basic_text_response() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from mock!"),
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
    assert_eq!(body["type"], "message");
    assert_eq!(body["content"][0]["text"], "Hi from mock!");
    assert_eq!(body["stop_reason"], "end_turn");
    assert!(body["id"].as_str().unwrap().starts_with("msg-llmposter-"));
    Ok(())
}
```

`match_user_message` does a substring match on the last user message. When no fixture matches, the server returns 404 with `{"error": {"message": "No fixture matched"}}`. The server shuts down when the `MockServer` value is dropped.

Every response includes an `x-request-id` header with the deterministic value `req-llmposter-{N}`, where `N` is a monotonically increasing counter per server session. This header is present on all responses, including 404s and 413s.

### SSE streaming response ✅ Current

Attach `with_streaming` to any fixture. `latency` sets ms delay between frames; `chunk_size` sets characters per delta frame.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_sse_streaming() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("stream")
                .respond_with_content("Hello streaming world")
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
            "messages": [{"role": "user", "content": "stream this"}],
            "stream": true
        }))
        .send()
        .await?;

    assert_eq!(resp.status().as_u16(), 200);
    let ct = resp.headers()["content-type"].to_str()?;
    assert!(ct.contains("text/event-stream"));
    let body = resp.text().await?;
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: content_block_delta"));
    assert!(body.contains("event: message_stop"));
    Ok(())
}
```

### Tool call response ✅ Current

Return a tool-use response. `ToolCall::arguments` must be a JSON object — strings and arrays are rejected at fixture validation.

```rust
use llmposter::{Fixture, ServerBuilder, ToolCall};

#[tokio::test]
async fn test_tool_call() -> Result<(), Box<dyn std::error::Error>> {
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
            "messages": [{"role": "user", "content": "what is the weather in London?"}]
        }))
        .send()
        .await?;

    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["stop_reason"], "tool_use");
    assert_eq!(body["content"][0]["type"], "tool_use");
    assert_eq!(body["content"][0]["name"], "get_weather");
    assert_eq!(body["content"][0]["input"]["location"], "London");
    Ok(())
}
```

Tool-call IDs (`id` field in each tool-use block) are globally unique across all responses in the same server session, guaranteed by a server-wide counter. Multi-turn tests that invoke the same fixture repeatedly will receive distinct IDs on every turn.

### Error and failure simulation ✅ Current

`with_error` returns an HTTP error code immediately. `with_failure(FailureConfig)` injects problems (latency, corrupt body, stream truncation) into an otherwise valid response — the two are mutually exclusive.

```rust
use llmposter::{FailureConfig, Fixture, ServerBuilder};

#[tokio::test]
async fn test_error_and_failure() -> Result<(), Box<dyn std::error::Error>> {
    // HTTP error response
    let err_server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("rate limit")
                .with_error(429, "Rate limit exceeded"),
        )
        .build()
        .await?;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", err_server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "rate limit me"}]
        }))
        .send()
        .await?;
    assert_eq!(resp.status().as_u16(), 429);

    // Stream truncated after 2 frames
    let trunc_server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("This response will be cut short")
                .with_streaming(Some(0), Some(5))
                .with_failure(FailureConfig {
                    truncate_after_frames: Some(2),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await?;

    let trunc_resp = client
        .post(format!("{}/v1/messages", trunc_server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "truncate"}],
            "stream": true
        }))
        .send()
        .await?;

    let body = trunc_resp.text().await?;
    assert!(body.contains("event: message_start"));
    assert!(!body.contains("event: message_stop"));
    Ok(())
}
```

On 429 responses, the server attaches provider-specific rate limit headers automatically. OpenAI and Responses endpoints receive `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, and `x-ratelimit-reset-requests`. Anthropic endpoints receive `anthropic-ratelimit-requests-limit`, `anthropic-ratelimit-requests-remaining`, and `anthropic-ratelimit-requests-reset`. Gemini endpoints receive `retry-after` only.

### Bearer token authentication ✅ Current

Enable auth with `with_auth(true)`. Requests without a valid `Authorization: Bearer <token>` header receive a provider-specific 401.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_bearer_auth() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token("test-token-abc")
        .fixture(Fixture::new().respond_with_content("authenticated"))
        .build()
        .await?;

    let client = reqwest::Client::new();
    let payload = serde_json::json!({
        "model": "claude-sonnet-4-6",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "hi"}]
    });

    // Valid token — 200
    let ok = client
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth("test-token-abc")
        .json(&payload)
        .send()
        .await?;
    assert_eq!(ok.status().as_u16(), 200);

    // Missing token — 401
    let denied = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&payload)
        .send()
        .await?;
    assert_eq!(denied.status().as_u16(), 401);
    Ok(())
}
```

## Configuration

### Bind address and verbose logging

```rust
use llmposter::{MockServer, ServerBuilder};

#[tokio::test]
async fn test_server_options() -> Result<(), Box<dyn std::error::Error>> {
    let server: MockServer = ServerBuilder::new()
        .bind("127.0.0.1:0")   // port 0 = random (default)
        .verbose(true)          // log fixture match decisions to stderr
        .build()
        .await?;

    println!("Listening at {}", server.url()); // e.g. "http://127.0.0.1:54321"
    Ok(())
}
```

### Loading fixtures from YAML files

```rust
use std::path::Path;
use llmposter::ServerBuilder;

#[tokio::test]
async fn test_yaml_loading() -> Result<(), Box<dyn std::error::Error>> {
    // Single file
    let server = ServerBuilder::new()
        .load_yaml(Path::new("tests/fixtures/responses.yaml"))?
        .build()
        .await?;

    // Or load every .yaml file in a directory
    let server_dir = ServerBuilder::new()
        .load_yaml_dir(Path::new("tests/fixtures/"))?
        .build()
        .await?;

    let _ = (server, server_dir);
    Ok(())
}
```

YAML fixture format — all structs use `deny_unknown_fields`, so field typos cause a load-time error:

```yaml
# First-match-wins ordering — put specific matchers before broad ones
- match:
    user_message: "stock price of AAPL"   # substring match (plain string)
    model: "claude-sonnet"                 # substring match on model field
  provider: anthropic                      # restrict to Anthropic endpoint only
  response:
    content: "AAPL is $200"

- match:
    user_message:
      regex: "^(explain|describe)"         # regex match; patterns compiling to a DFA larger than 1 MB are rejected at load time
  response:
    content: "Here is an explanation"

- error:
    status: 429
    message: "Rate limit exceeded"

- response:
    content: "Truncated stream"
  streaming:
    latency: 10          # ms between frames
    chunk_size: 5        # characters per delta frame
  failure:
    truncate_after_frames: 3
    # truncate_after_chunks is a deprecated YAML alias for the same field
```

### Provider-scoped fixtures

Restrict a fixture to a specific LLM endpoint using `for_provider`. An unmatched provider returns 404.

```rust
use llmposter::{Fixture, Provider, ServerBuilder};

#[tokio::test]
async fn test_provider_scoping() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("anthropic only")
                .for_provider(Provider::Anthropic),
        )
        .build()
        .await?;

    let client = reqwest::Client::new();
    // POST /v1/messages (Anthropic) — 200
    let ok = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await?;
    assert_eq!(ok.status().as_u16(), 200);

    // POST /v1/chat/completions (OpenAI) — 404
    let miss = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await?;
    assert_eq!(miss.status().as_u16(), 404);
    Ok(())
}
```

### OAuth mock (default-on in 0.4.0)

```rust
// Requires `oauth` feature — enabled by default in 0.4.0
use llmposter::ServerBuilder;

#[tokio::test]
async fn test_oauth_defaults() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_oauth_defaults() // client_id="mock-client", client_secret="mock-secret"
        .build()
        .await?;

    // Standard OIDC endpoints now active at server.url()
    println!("OAuth base: {}", server.url());
    Ok(())
}
```

OAuth-issued access tokens obtained via the standard OIDC `/oauth/token` endpoint are automatically accepted on all LLM endpoints (`/v1/messages`, `/v1/chat/completions`, etc.) without additional configuration. Present the token as a standard `Authorization: Bearer <token>` header — no separate `add_token` call is required.

To disable OAuth endpoints entirely:

```toml
[dev-dependencies]
llmposter = { version = "0.4", default-features = false }
```

## Pitfalls

### Broad fixture placed before specific fixture

#### Wrong

```rust
use llmposter::{Fixture, ServerBuilder};

// "stock" matches "stock price of AAPL" first — specific fixture never reached
#[tokio::test]
async fn test_wrong_ordering() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().match_user_message("stock").respond_with_content("generic"))
        .fixture(Fixture::new().match_user_message("stock price of AAPL").respond_with_content("specific"))
        .build()
        .await
        .unwrap();
    // Request "stock price of AAPL" returns "generic" — wrong
}
```

#### Right

```rust
use llmposter::{Fixture, ServerBuilder};

// Specific matcher first, broad matcher last
#[tokio::test]
async fn test_correct_ordering() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().match_user_message("stock price of AAPL").respond_with_content("specific"))
        .fixture(Fixture::new().match_user_message("stock").respond_with_content("generic"))
        .build()
        .await
        .unwrap();
}
```

---

### Using `with_error` to test stream truncation

#### Wrong

```rust
use llmposter::{Fixture, ServerBuilder};

// with_error returns an HTTP error immediately — no SSE stream is produced at all
#[tokio::test]
async fn test_wrong_stream_failure() {
    let _server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("text")
                .with_error(429, "overload"), // returns 429 with no body stream
        )
        .build()
        .await
        .unwrap();
}
```

#### Right

```rust
use llmposter::{FailureConfig, Fixture, ServerBuilder};

// with_failure injects problems into a streaming response
#[tokio::test]
async fn test_correct_stream_failure() {
    let _server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("truncated response")
                .with_streaming(Some(0), Some(5))
                .with_failure(FailureConfig {
                    truncate_after_frames: Some(2),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await
        .unwrap();
}
```

---

### Tool call arguments as a non-object type

#### Wrong

```rust
use llmposter::ToolCall;

// Arguments must be a JSON object — string is rejected at fixture validation
fn wrong_tool_call() -> ToolCall {
    ToolCall {
        name: "search".to_string(),
        arguments: serde_json::json!("London"), // string — build() returns Err
    }
}
```

#### Right

```rust
use llmposter::ToolCall;

fn correct_tool_call() -> ToolCall {
    ToolCall {
        name: "search".to_string(),
        arguments: serde_json::json!({"query": "London"}), // object — valid
    }
}
```

---

### Empty string as a match pattern

#### Wrong

```rust
use llmposter::{Fixture, ServerBuilder};

// Empty patterns act as silent catch-alls — rejected at build()
#[tokio::test]
async fn test_empty_pattern() {
    let result = ServerBuilder::new()
        .fixture(Fixture::new().match_user_message("")) // validation error
        .build()
        .await;
    assert!(result.is_err());
}
```

#### Right

```rust
use llmposter::{Fixture, ServerBuilder};

// Omit match_user_message entirely to match all requests
#[tokio::test]
async fn test_catch_all() {
    let _server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("catch all")) // matches all requests
        .build()
        .await
        .unwrap();
}
```

---

### Assuming `oauth` feature is opt-in

#### Wrong

```toml
# WRONG: assumes OAuth endpoints are inactive without explicit configuration
[dev-dependencies]
llmposter = "0.4"
# In 0.4.0, oauth is a DEFAULT feature — OIDC endpoints are active automatically
```

#### Right

```toml
# Correct: opt out explicitly when OAuth endpoints are not wanted
[dev-dependencies]
llmposter = { version = "0.4", default-features = false }
```

---

### Empty regex pattern in YAML fixture

#### Wrong

```yaml
# Empty regex is rejected at fixture validation — rejected by build() or load_yaml()
- match:
    user_message:
      regex: ""   # empty regex — fixture will not load
  response:
    content: "This fixture will never load"
```

#### Right

```yaml
# Omit the matcher entirely to match all user messages
- response:
    content: "Catch all"
```

---

### Blank tool name in ToolCall

#### Wrong

```rust
use llmposter::ToolCall;

// Blank tool names are rejected at fixture validation
fn wrong_blank_name() -> ToolCall {
    ToolCall {
        name: "".to_string(), // blank — build() returns Err
        arguments: serde_json::json!({"query": "London"}),
    }
}
```

#### Right

```rust
use llmposter::ToolCall;

fn correct_named_tool() -> ToolCall {
    ToolCall {
        name: "search".to_string(), // non-blank — valid
        arguments: serde_json::json!({"query": "London"}),
    }
}
```

---

### Assuming Gemini response content always includes a role field

#### Wrong

```rust
// Gemini safety-blocked responses may omit Content.role — unconditional unwrap will panic
let role = body["candidates"][0]["content"]["role"].as_str().unwrap();
```

#### Right

```rust
// Treat role as optional — it may be absent when a Gemini response is safety-blocked
if let Some(role) = body["candidates"][0]["content"]["role"].as_str() {
    // role is present
}
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3

### 0.3.x → 0.4.0

**MSRV bumped to 1.89.** The `oauth-mock` dependency requires Rust 1.89+. Update `rust-toolchain.toml` and CI matrices before upgrading.

**`oauth` feature is on by default.** Standard OIDC endpoints are now active in every server instance. Code that asserts non-OIDC paths are the only active paths may be surprised. Opt out with `default-features = false`.

**Auth builder additions.** New methods `with_bearer_token_uses`, `with_oauth`, and `with_oauth_defaults` allow token-expiry and OAuth flow testing without custom `AuthState` wiring.

### 0.3.3 → 0.3.4 (already released)

**Responses API streaming format changed.** Events for `/v1/responses` now use a nested `response` envelope with `sequence_number` and correlation fields. The `response.done` event was removed (non-spec); `response.in_progress` was added.

Before (0.3.3):

```text
event: response.done
data: {"id": "resp_123", ...}
```

After (0.3.4+):

```text
event: response.in_progress
data: {"response": {"id": "resp_123", "sequence_number": 1, ...}}
```

**Error shape for `/v1/responses` changed.** `code` is now a string (was integer); `param` is always present as `null` (was absent).

Before:

```json
{"error": {"code": 429, "type": "rate_limit_error"}}
```

After:

```json
{"error": {"code": "rate_limit_error", "type": "rate_limit_error", "param": null}}
```

Update any SSE parsing and error-shape assertions when moving from 0.3.3 to 0.3.4+.

## API Reference

**`ServerBuilder::new()`** — Creates a builder with no fixtures, random port on `127.0.0.1`, auth disabled, and verbose disabled.

**`ServerBuilder::fixture(f: Fixture)`** — Appends one fixture. Fixtures are evaluated in insertion order; first match wins.

**`ServerBuilder::bind(addr: &str)`** — Sets the TCP bind address, e.g. `"127.0.0.1:9000"`. Omit for a random port.

**`ServerBuilder::verbose(v: bool)`** — When `true`, emits fixture matching diagnostics to stderr.

**`ServerBuilder::with_auth(enabled: bool)`** — Enables Bearer token validation on all LLM endpoints. Requests without a valid token receive a provider-specific 401.

**`ServerBuilder::with_bearer_token(token: &str)`** — Registers an unlimited-use bearer token accepted on all LLM endpoints.

**`ServerBuilder::with_bearer_token_uses(token: &str, max_uses: u64)`** — Registers a bearer token that expires after exactly `max_uses` requests; the `(max_uses + 1)`th request returns 401.

**`ServerBuilder::build()`** — Async. Validates all fixtures (pre-compiles regexes, checks constraints), binds the listener, and returns a running `MockServer`. Returns `Err` if any fixture fails validation.

**`MockServer::url()`** — Returns the base URL of the running server as `String`, e.g. `"http://127.0.0.1:54321"`. The server shuts down when `MockServer` is dropped.

**`Fixture::new()`** — Creates an empty fixture with no match rules and no response configured.

**`Fixture::match_user_message(pattern: &str)`** — Substring match on the last user message in the request. Use `{regex: '...'}` syntax in YAML for regex. Empty patterns are rejected.

**`Fixture::respond_with_content(content: &str)`** — Configures a text response body. Mutually exclusive with `respond_with_tool_calls`.

**`Fixture::respond_with_tool_calls(tool_calls: Vec<ToolCall>)`** — Configures a tool-use response. Each `ToolCall::arguments` must be a JSON object, and `ToolCall::name` must be non-blank; both constraints are enforced at fixture validation.

**`Fixture::with_error(status: u16, message: &str)`** — Returns an HTTP error response immediately. `status` must be 400–599. Mutually exclusive with `with_failure`.

**`Fixture::with_streaming(latency: Option<u64>, chunk_size: Option<usize>)`** — Enables SSE streaming. `latency` is milliseconds between frames; `chunk_size` is characters per delta frame. Combine with `with_failure` to simulate truncation or disconnect.

**Body size limit.** Requests whose body exceeds the default size limit return 413. The `x-request-id` header is still present on the 413 response because the body-size limit layer sits inside the request-ID middleware.
