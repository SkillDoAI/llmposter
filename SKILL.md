---
name: llmposter
description: Mock server for LLM APIs (OpenAI, Anthropic, Gemini, Responses) with fixture-based request matching, streaming SSE, failure injection, and optional OAuth for deterministic integration testing.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-opus-4-6
---


## Imports

```rust
use llmposter::{Fixture, FailureConfig, Provider, ServerBuilder, ToolCall};
```

```toml
[dev-dependencies]
llmposter = "0.4"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
reqwest = { version = "0.13", features = ["json"] }
serde_json = "1"
```

## Core Patterns

### Basic Text Response Mock — ✅ Current

Start a mock server with a fixture that matches by user message substring and returns text content. The server binds to a random port; use `server.url()` for the base URL.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn mock_anthropic_text() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from the mock!"),
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
    assert_eq!(body["content"][0]["text"], "Hi from the mock!");
    assert_eq!(body["stop_reason"], "end_turn");
    Ok(())
}
```

`match_user_message` uses substring matching — `"hello"` matches any message containing `"hello"`. When no fixture matches, the server returns 404.

### Streaming SSE Response — ✅ Current

Enable Server-Sent Events streaming with `with_streaming(latency, chunk_size)`. Content is split into `content_block_delta` events.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn mock_streaming_sse() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hello world")
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
            "stream": true,
            "messages": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
    let body = resp.text().await?;
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: content_block_delta"));
    assert!(body.contains("event: message_stop"));
    Ok(())
}
```

`with_streaming(Some(50), Some(5))` sends 5-character chunks with 50ms between each SSE frame.

### Tool Call Response — ✅ Current

Return `tool_use` content blocks with auto-generated tool IDs (`toolu_llmposter_1`, `toolu_llmposter_2`, ...).

```rust
use llmposter::{Fixture, ServerBuilder, ToolCall};

#[tokio::test]
async fn mock_tool_use() -> Result<(), Box<dyn std::error::Error>> {
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
            "messages": [{"role": "user", "content": "What is the weather?"}]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["stop_reason"], "tool_use");
    assert_eq!(body["content"][0]["type"], "tool_use");
    assert_eq!(body["content"][0]["name"], "get_weather");
    Ok(())
}
```

Tool-call arguments must be JSON objects. Arrays and primitives are rejected at fixture validation time.

### Error and Failure Injection — ✅ Current

`with_error` returns a clean HTTP error response. `with_failure` simulates network-level problems (latency, body corruption, stream truncation, connection drops).

```rust
use llmposter::{FailureConfig, Fixture, ServerBuilder};
use std::time::Instant;

#[tokio::test]
async fn mock_error_and_failure() -> Result<(), Box<dyn std::error::Error>> {
    // HTTP error response (clean 429)
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("rate limit")
                .with_error(429, "Rate limit exceeded"),
        )
        .fixture(
            Fixture::new()
                .match_user_message("slow")
                .respond_with_content("delayed response")
                .with_failure(FailureConfig {
                    latency_ms: Some(200),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await?;

    let client = reqwest::Client::new();

    // Error fixture returns 429 with error JSON
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

    // Failure fixture delays 200ms then returns normal response
    let start = Instant::now();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "slow request"}]
        }))
        .send()
        .await?;
    assert!(start.elapsed().as_millis() >= 180);
    assert_eq!(resp.status(), 200);
    Ok(())
}
```

`FailureConfig` fields compose — `latency_ms` + `corrupt_body` both apply. `corrupt_body: Some(true)` returns the literal string `"overloaded"` as `text/plain`.

`FailureConfig` fields:

| Field | Type | Effect |
|-------|------|--------|
| `latency_ms` | `Option<u64>` | Delay before sending response |
| `corrupt_body` | `Option<bool>` | Replace body with `"overloaded"` as `text/plain` |
| `truncate_after_frames` | `Option<u32>` | Send N SSE chunks then abruptly stop (streaming only). YAML alias: `truncate_after_chunks` |
| `disconnect_after_ms` | `Option<u64>` | Drop connection after N milliseconds |

Stream truncation example:

```rust
use llmposter::{FailureConfig, Fixture, ServerBuilder};

#[tokio::test]
async fn mock_truncated_stream() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("truncate")
                .respond_with_content("This is a long response that will be cut short")
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
            "stream": true,
            "messages": [{"role": "user", "content": "truncate this"}]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 200);
    let body = resp.text().await?;
    // Stream stops after 2 chunks — no message_stop event
    assert!(body.contains("content_block_delta"));
    assert!(!body.contains("message_stop"));
    Ok(())
}
```

### Provider-Scoped Fixtures with Custom Stop Reason — ✅ Current

Use `for_provider` to restrict a fixture to a specific API format. Use `with_stop_reason` to override the default stop reason.

```rust
use llmposter::{Fixture, Provider, ServerBuilder};

#[tokio::test]
async fn mock_provider_scoped() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("generate")
                .for_provider(Provider::Anthropic)
                .respond_with_content("hit max tokens")
                .with_stop_reason("max_tokens"),
        )
        .build()
        .await?;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "generate a long essay"}]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["stop_reason"], "max_tokens");
    Ok(())
}
```

An `OpenAI`-scoped fixture returns 404 on `/v1/messages` (Anthropic endpoint) and vice versa. Fixtures without `for_provider` match all endpoints.

## Configuration

### YAML Fixture Files

Fixtures can be defined in YAML and loaded at build time or from the CLI. All structs use `deny_unknown_fields` — field typos cause load-time errors.

```yaml
# fixtures/anthropic.yaml
- match:
    user_message: "hello"
  provider: anthropic
  response:
    content: "Hi from the mock!"

- match:
    user_message: "weather"
  response:
    tool_calls:
      - name: get_weather
        arguments:
          location: "London"
          unit: "celsius"

- match:
    model: "claude-sonnet"
  response:
    content: "Matched by model name"

- error:
    status: 429
    message: "Rate limit exceeded"
```

Load YAML fixtures in Rust:

```rust
use llmposter::ServerBuilder;
use std::path::Path;

#[tokio::test]
async fn load_yaml_fixtures() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .load_yaml(Path::new("fixtures/anthropic.yaml"))?
        .build()
        .await?;
    assert!(!server.url().is_empty());
    Ok(())
}
```

Use `load_yaml_dir` to load all YAML files from a directory.

### CLI Usage

```text
llmposter fixtures/ --port 8080 --bind 0.0.0.0 --verbose
```

| Flag | Default | Description |
|------|---------|-------------|
| `fixtures` (positional) | required | Path to YAML file or directory |
| `--port` | `2112` | Port to bind |
| `--bind` | `127.0.0.1` | Bind address |
| `--verbose` | `false` | Include match diagnostics in 404 responses |
| `--validate` | `false` | Validate fixtures and exit (no server) |

### Authentication

Auth is opt-in. Enable it with `with_auth(true)`, then register accepted tokens:

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn mock_with_auth() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token("test-token-123")
        .fixture(Fixture::new().respond_with_content("authenticated"))
        .build()
        .await?;

    let client = reqwest::Client::new();

    // Without token: 401
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "test"}]
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 401);

    // With token: 200
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .header("authorization", "Bearer test-token-123")
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "test"}]
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    Ok(())
}
```

Use `with_bearer_token_uses("token", max_uses)` to create tokens that expire after N uses. The first N requests succeed; request N+1 returns 401:

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn mock_token_expiry() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token_uses("limited-token", 2)
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await?;

    let client = reqwest::Client::new();
    let req = || {
        client
            .post(format!("{}/v1/messages", server.url()))
            .header("authorization", "Bearer limited-token")
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
    };

    // First 2 uses succeed
    assert_eq!(req().send().await?.status(), 200);
    assert_eq!(req().send().await?.status(), 200);

    // Third use: token exhausted → 401
    assert_eq!(req().send().await?.status(), 401);
    Ok(())
}
```

### OAuth Authentication

OAuth support requires the `oauth` feature (enabled by default). Use `with_oauth_defaults()` for a pre-configured OAuth flow or `with_oauth(config)` for custom settings.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn mock_with_oauth() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_oauth_defaults()
        .fixture(Fixture::new().respond_with_content("oauth authenticated"))
        .build()
        .await?;

    let client = reqwest::Client::new();

    // Step 1: Request an OAuth token
    let token_resp = client
        .post(format!("{}/oauth/token", server.url()))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", "mock-client"),
            ("client_secret", "mock-secret"),
        ])
        .send()
        .await?;
    assert_eq!(token_resp.status(), 200);
    let token_body: serde_json::Value = token_resp.json().await?;
    let access_token = token_body["access_token"].as_str().unwrap();

    // Step 2: Use the OAuth token on LLM endpoints
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .header("authorization", format!("Bearer {}", access_token))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "test"}]
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    Ok(())
}
```

OAuth-issued tokens are automatically valid for LLM endpoints — no separate `with_bearer_token` call needed.

### Regex Matching in YAML

```yaml
- match:
    user_message:
      regex: "price of [A-Z]{1,5}"
  response:
    content: "Stock price: $150.00"
```

Regex DFA size is capped at 1MB. Keep patterns simple; prefer substring matching when possible.

### Response Headers

Every response includes an `x-request-id` header with the format `req-llmposter-{N}` where N is a monotonic counter. Rate-limit error responses (429) include provider-specific rate-limit headers:

| Provider | Headers | Value Format |
|----------|---------|--------------|
| OpenAI / Responses | `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, `x-ratelimit-reset-requests` | Integer counts; reset is a duration string (e.g. `"1s"`) |
| Anthropic | `anthropic-ratelimit-requests-limit`, `anthropic-ratelimit-requests-remaining`, `anthropic-ratelimit-requests-reset` | Integer counts; reset is an RFC 3339 datetime in the future (e.g. `"2026-03-26T12:01:00Z"`) |
| Gemini | `retry-after` | Integer seconds (e.g. `"60"`) |

## Pitfalls

### Fixture Ordering — First Match Wins

#### Wrong

```rust
use llmposter::{Fixture, ServerBuilder};

// Broad pattern first — "stock" matches everything, specific pattern never reached
let _server = ServerBuilder::new()
    .fixture(
        Fixture::new()
            .match_user_message("stock")
            .respond_with_content("generic stock info"),
    )
    .fixture(
        Fixture::new()
            .match_user_message("stock price of AAPL")
            .respond_with_content("AAPL: $150"),
    );
// Request "stock price of AAPL" always returns "generic stock info"
```

#### Right

```rust
use llmposter::{Fixture, ServerBuilder};

// Specific pattern first — like firewall rules, most-specific wins
let _server = ServerBuilder::new()
    .fixture(
        Fixture::new()
            .match_user_message("stock price of AAPL")
            .respond_with_content("AAPL: $150"),
    )
    .fixture(
        Fixture::new()
            .match_user_message("stock")
            .respond_with_content("generic stock info"),
    );
```

### Error vs Failure Confusion

#### Wrong

```rust
use llmposter::Fixture;

// with_error returns a clean HTTP 500 — not a network-level failure
let _fixture = Fixture::new()
    .with_error(500, "Connection reset");
// Client receives a well-formed HTTP response with status 500
```

#### Right

```rust
use llmposter::{FailureConfig, Fixture};

// Use with_failure for network-level problems (disconnect, truncation, corruption)
let _fixture = Fixture::new()
    .respond_with_content("partial response")
    .with_streaming(Some(0), Some(5))
    .with_failure(FailureConfig {
        disconnect_after_ms: Some(50),
        ..FailureConfig::default()
    });
// Connection actually drops mid-stream
```

### Auth Not Enabled

#### Wrong

```rust
use llmposter::{Fixture, ServerBuilder};

// Token registered but auth not enabled — all requests accepted without checking
let _builder = ServerBuilder::new()
    .with_bearer_token("test-token-123")
    .fixture(Fixture::new().respond_with_content("response"));
```

#### Right

```rust
use llmposter::{Fixture, ServerBuilder};

// Must explicitly enable auth before registering tokens
let _builder = ServerBuilder::new()
    .with_auth(true)
    .with_bearer_token("test-token-123")
    .fixture(Fixture::new().respond_with_content("response"));
```

### Non-Object Tool-Call Arguments

#### Wrong

```rust
use llmposter::{Fixture, ToolCall};

// Array arguments — rejected at fixture validation time
let _fixture = Fixture::new()
    .respond_with_tool_calls(vec![ToolCall {
        name: "search".to_string(),
        arguments: serde_json::json!(["query", "limit"]),
    }]);
```

#### Right

```rust
use llmposter::{Fixture, ToolCall};

// Arguments must be JSON objects
let _fixture = Fixture::new()
    .respond_with_tool_calls(vec![ToolCall {
        name: "search".to_string(),
        arguments: serde_json::json!({"query": "rust", "limit": 10}),
    }]);
```

### Empty Match Patterns

#### Wrong

```rust
use llmposter::Fixture;

// Empty substring — rejected at validation since v0.3.5
let _fixture = Fixture::new()
    .match_user_message("")
    .respond_with_content("catch-all");
```

#### Right

```rust
use llmposter::Fixture;

// Omit match_user_message entirely for a catch-all fixture
let _fixture = Fixture::new()
    .respond_with_content("catch-all");
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3.x

### MSRV Bump (v0.3.x → v0.4.0)

Minimum supported Rust version is now **1.89** (was pre-1.89). Update your toolchain before upgrading.

### OAuth Feature (v0.4.0)

OAuth is enabled by default but purely opt-in — existing code without `with_auth(true)` is unaffected. Call `with_oauth_defaults()` or `with_oauth(config)` to enable OAuth endpoints.

### Responses API Streaming (v0.3.3 → v0.3.4)

SSE events for `/v1/responses` were reworked to match the real API spec.

Before:

```text
event: response.done
data: {"type":"response.done",...}
```

After:

```text
event: response.in_progress
data: {"type":"response.in_progress","response":{...},"sequence_number":3}

event: response.completed
data: {"type":"response.completed","response":{...},"sequence_number":5}
```

Events now use nested `response` envelopes with `sequence_number`. The stream emits `response.in_progress` during content delivery, then `response.completed` as the terminal event. The `response.done` event was removed. Handle `response.completed` for stream termination and `response.in_progress` for intermediate state updates.

### Stricter Fixture Validation (v0.3.4+)

These were previously accepted silently and now fail at load time:

| Condition | Version Rejected |
|-----------|-----------------|
| Non-object tool-call arguments | v0.3.4 |
| Blank tool names | v0.3.4 |
| Error status outside 400–599 | v0.3.4 |
| Empty substring match patterns | v0.3.5 |
| Empty regex match patterns | v0.3.6 |
| Unknown YAML fields (typos) | v0.3.5 (`deny_unknown_fields`) |

### Error Response Format (v0.3.3 → v0.3.4)

OpenAI-format error bodies now match the real API shape:

```json
{
  "error": {
    "type": "invalid_request_error",
    "code": "rate_limit_exceeded",
    "param": null,
    "message": "Rate limit exceeded"
  }
}
```

Update assertions to expect `type` (string), `code` (string), and `param` (null).

## API Reference

**ServerBuilder::new()** — Create a new mock server builder. All configuration is chained before calling `build()`.

**ServerBuilder::fixture(f: Fixture)** — Add a single fixture. Call multiple times for multiple fixtures; first-match-wins ordering.

**ServerBuilder::fixtures(fixtures: Vec\<Fixture\>)** — Add multiple fixtures at once. Equivalent to calling `fixture()` for each.

**ServerBuilder::bind(addr: &str)** — Set the bind address. Rust API equivalent of `--bind`.

**ServerBuilder::build()** — `async` — Start the mock server on a random port. Returns `Result<MockServer, Box<dyn std::error::Error>>`.

**ServerBuilder::verbose(v: bool)** — Enable match diagnostics. When `true`, 404 responses include which fixtures were checked and why they didn't match.

**ServerBuilder::with_auth(enabled: bool)** — Enable bearer token authentication. Must be called before `with_bearer_token`.

**ServerBuilder::with_bearer_token(token: &str)** — Register a bearer token accepted by the server. Requires `with_auth(true)`.

**ServerBuilder::with_bearer_token_uses(token: &str, max_uses: u64)** — Register a bearer token that expires after `max_uses` successful authentications.

**ServerBuilder::with_oauth_defaults()** — Enable OAuth endpoints with default configuration (client_id: `"mock-client"`, client_secret: `"mock-secret"`). Requires `oauth` feature.

**ServerBuilder::with_oauth(config: OAuthConfig)** — Enable OAuth endpoints with custom configuration. Requires `oauth` feature.

**ServerBuilder::load_yaml(path: &Path)** — Load fixtures from a single YAML file. Returns `Result` with validation errors.

**ServerBuilder::load_yaml_dir(dir: &Path)** — Load fixtures from all YAML files in a directory. Returns `Result` with validation errors.

**MockServer::url()** — Returns the base URL of the running server (e.g., `http://127.0.0.1:54321`).

**Fixture::new()** — Create an empty fixture. Without match rules, matches all requests.

**Fixture::match_user_message(pattern: &str)** — Match when the last user message contains `pattern` as a substring.

**Fixture::match_model(pattern: &str)** — Match when the request model name contains `pattern` as a substring.

**Fixture::respond_with_content(content: &str)** — Set the text content for the response body.

**Fixture::respond_with_tool_calls(tool_calls: Vec\<ToolCall\>)** — Set tool-call content blocks for the response. Arguments must be JSON objects.

**Fixture::with_error(status: u16, message: &str)** — Return an HTTP error response. Status must be 400–599.

**Fixture::with_failure(failure: FailureConfig)** — Inject network-level failures: latency, body corruption, stream truncation, or connection drops.

**Fixture::with_stop_reason(reason: &str)** — Override the stop reason in the response (Anthropic format: `stop_reason`).

**Fixture::with_finish_reason(reason: &str)** — Override the finish reason in the response (OpenAI format: `finish_reason`).

**Fixture::with_streaming(latency: Option\<u64\>, chunk_size: Option\<usize\>)** — Enable SSE streaming. `latency` is milliseconds between frames; `chunk_size` is characters per delta event.

**Fixture::for_provider(provider: Provider)** — Restrict this fixture to a specific API format. Unscoped fixtures match all endpoints.

**Fixture::validate()** — Validate fixture configuration. Returns `Result<(), String>`. Called automatically during `build()` and YAML loading.
