---
name: llmposter
description: A mock LLM API server for integration testing — serves OpenAI, Anthropic, Gemini, and OpenAI Responses API endpoints from in-process fixture definitions or YAML files, with streaming, auth, and failure injection support.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---


## Imports

```rust
use llmposter::{Fixture, Provider, ServerBuilder};
use llmposter::fixture::{FailureConfig, ToolCall};
```

Add to `Cargo.toml` as a dev dependency:

```toml
[dev-dependencies]
llmposter = "0.4.0"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.13", default-features = false, features = ["json"] }
serde_json = "1"
```

Disable the `oauth` feature to avoid the MSRV 1.89 requirement from `oauth-mock`:

```toml
[dev-dependencies]
llmposter = { version = "0.4.0", default-features = false }
```

## Core Patterns

### Text Response (Anthropic endpoint) — ✅ Current

`ServerBuilder` starts a server on a random port. Drop `MockServer` to stop it.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_text_response() -> Result<(), Box<dyn std::error::Error>> {
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
    assert_eq!(body["content"][0]["text"], "Hi from mock!");
    assert_eq!(body["stop_reason"], "end_turn");
    assert!(body["id"].as_str().unwrap().starts_with("msg-llmposter-"));
    Ok(())
}
```

`match_user_message` does a case-sensitive substring match on the last user message. Unmatched requests return `404` with `{"error": {"message": "No fixture matched..."}}`. First-match-wins when multiple fixtures are registered — order from most specific to least specific.

Provider endpoints served:
- Anthropic Messages API: `POST /v1/messages`
- OpenAI Chat Completions: `POST /v1/chat/completions`
- OpenAI Responses API: `POST /v1/responses`
- Gemini: `POST /v1beta/models/{model}:generateContent`

### Streaming SSE Response — ✅ Current

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_streaming() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("stream me")
                .respond_with_content("Hello world")
                .with_streaming(Some(0), Some(5)), // 0 ms between frames, 5 chars per chunk
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
            "messages": [{"role": "user", "content": "stream me"}]
        }))
        .send()
        .await?;

    assert_eq!(resp.status().as_u16(), 200);
    assert!(resp.headers()["content-type"].to_str()?.contains("text/event-stream"));
    let body = resp.text().await?;
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: content_block_delta"));
    assert!(body.contains("event: message_stop"));
    Ok(())
}
```

`with_streaming(latency, chunk_size)`: first arg is the delay in ms between SSE frames; second is characters per delta frame. Both are `Option<_>`; pass `None` to use defaults. The client must send `"stream": true` in the request body.

### Tool-Call Response — ✅ Current

```rust
use llmposter::fixture::ToolCall;
use llmposter::{Fixture, ServerBuilder};

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
            "messages": [{"role": "user", "content": "what is the weather in London"}]
        }))
        .send()
        .await?;

    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["stop_reason"], "tool_use");
    assert_eq!(body["content"][0]["type"], "tool_use");
    assert_eq!(body["content"][0]["name"], "get_weather");
    // Check prefix, not exact value — IDs use a server-wide counter
    assert!(body["content"][0]["id"].as_str().unwrap().starts_with("toolu_llmposter_"));
    Ok(())
}
```

`ToolCall::arguments` must be a `serde_json::Value::Object`. String or scalar values are rejected at fixture validation. `respond_with_tool_calls` and `respond_with_content` are mutually exclusive.

### Error and Failure Simulation — ✅ Current

```rust
use llmposter::fixture::FailureConfig;
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_error_and_failure() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        // HTTP 429 error response
        .fixture(
            Fixture::new()
                .match_user_message("rate limit")
                .with_error(429, "Rate limit exceeded"),
        )
        // Latency: delays the full response by 200 ms
        .fixture(
            Fixture::new()
                .match_user_message("slow")
                .respond_with_content("delayed response")
                .with_failure(FailureConfig {
                    latency_ms: Some(200),
                    ..FailureConfig::default()
                }),
        )
        // Corrupt body: returns plain-text "overloaded" at HTTP 200
        .fixture(
            Fixture::new()
                .match_user_message("corrupt")
                .respond_with_content("never seen")
                .with_failure(FailureConfig {
                    corrupt_body: Some(true),
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
            "messages": [{"role": "user", "content": "trigger rate limit"}]
        }))
        .send()
        .await?;

    assert_eq!(resp.status().as_u16(), 429);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["error"]["message"], "Rate limit exceeded");
    Ok(())
}
```

`FailureConfig` fields (all `Option`, implements `Default`):
- `latency_ms: Option<u64>` — delays the response by N ms before sending anything
- `corrupt_body: Option<bool>` — returns plain-text `"overloaded"` at HTTP 200
- `truncate_after_frames: Option<u32>` — closes the SSE stream after N frames (streaming only)
- `disconnect_after_ms: Option<u64>` — drops the TCP connection after N ms (streaming only)

`with_failure` requires `respond_with_content` or `respond_with_tool_calls` to also be set. `with_error` and `with_failure` are mutually exclusive.

#### Error response shape

All error responses include an `x-request-id` response header (always present, including on 413 responses). The error body follows the real provider shape:

```json
{"error": {"type": "rate_limit_error", "code": "rate_limit_exceeded", "message": "Rate limit exceeded", "param": null}}
```

- `type` — error category string
- `code` — string (not an integer)
- `param` — always present; `null` when not applicable

#### Provider-specific rate-limit headers on HTTP 429

When `with_error(429, ...)` is used, the response includes provider-specific rate-limit headers:

| Provider | Headers |
|---|---|
| OpenAI / Responses | `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, `x-ratelimit-reset-requests` |
| Anthropic | `anthropic-ratelimit-requests-limit`, `anthropic-ratelimit-requests-remaining`, `anthropic-ratelimit-requests-reset` |
| Gemini | `retry-after` only |

```rust
let resp = client
    .post(format!("{}/v1/chat/completions", server.url()))
    // ... send rate-limit-triggering request ...
    .send()
    .await?;

assert_eq!(resp.status().as_u16(), 429);
assert!(resp.headers().contains_key("x-ratelimit-limit-requests"));
assert!(resp.headers().contains_key("x-ratelimit-remaining-requests"));
assert!(resp.headers().contains_key("x-ratelimit-reset-requests"));
```

## Configuration

### ServerBuilder options

```rust
use llmposter::ServerBuilder;

let server = ServerBuilder::new()
    .bind("127.0.0.1:8080")           // fixed address; omit for random port
    .verbose(true)                      // log matched/unmatched requests to stderr
    .with_auth(true)                    // require Bearer token; missing/invalid → 401
    .with_bearer_token("my-token")     // register unlimited-use token; enables auth
    .with_bearer_token_uses("once", 1) // token expires after 1 use
    .build()
    .await?;
```

### Loading fixtures from YAML

```rust
use llmposter::ServerBuilder;
use std::path::Path;

// Single file
let server = ServerBuilder::new()
    .load_yaml(Path::new("tests/fixtures/responses.yaml"))?
    .build()
    .await?;

// All YAML files in a directory
let server = ServerBuilder::new()
    .load_yaml_dir(Path::new("tests/fixtures/"))?
    .build()
    .await?;
```

YAML fixture format:

```yaml
# tests/fixtures/responses.yaml

# Substring match
- match:
    user_message: "hello"
  response:
    content: "Hi from mock!"

# Regex match + model filter
- match:
    model: "gpt-4"
    user_message:
      regex: '^summarize .+'
  response:
    content: "Here is your summary."

# Tool-call response
- match:
    user_message: "weather"
  response:
    tool_calls:
      - name: get_weather
        arguments:
          location: London
          unit: celsius

# HTTP error
- match:
    user_message: "overload"
  error:
    status: 503
    message: "Service unavailable"

# Streaming with truncation failure
- match:
    user_message: "truncate me"
  response:
    content: "This is a long response that will be cut off"
  streaming:
    latency: 0
    chunk_size: 5
  failure:
    truncate_after_frames: 2  # deprecated YAML alias: truncate_after_chunks
```

Key YAML rules:
- All structs use `deny_unknown_fields` — field-name typos fail at startup, not silently.
- `match:` is optional; a fixture without it matches every request for the applicable provider.
- `user_message:` accepts a plain string (substring) or `{regex: '...'}` (anchored regex).
- Empty substring and regex patterns are rejected at startup.
- Regex patterns whose compiled DFA exceeds 1 MB are rejected at startup. Avoid unbounded alternations or complex lookaheads.
- `error:` and `failure:` are mutually exclusive.

### Provider scoping

```rust
use llmposter::{Fixture, Provider, ServerBuilder};

let server = ServerBuilder::new()
    .fixture(
        Fixture::new()
            .respond_with_content("openai only")
            .for_provider(Provider::OpenAI),
    )
    .fixture(
        Fixture::new()
            .respond_with_content("anthropic only")
            .for_provider(Provider::Anthropic),
    )
    .build()
    .await?;
```

`Provider` variants: `OpenAI`, `Anthropic`, `Gemini`, `Responses`. A fixture without `for_provider` matches all providers.

### Gemini safety-blocked responses

When a Gemini fixture response is safety-blocked, the `Content.role` field may be absent (`None`) in the response object. Callers that parse Gemini responses from `POST /v1beta/models/{model}:generateContent` must treat the `role` field as optional:

```rust
// Safe — handles absent role
let role = body["candidates"][0]["content"]["role"].as_str().unwrap_or("model");

// Unsafe — panics on safety-blocked responses
let role = body["candidates"][0]["content"]["role"].as_str().unwrap();
```

### OAuth (feature `oauth`, enabled by default)

```rust
use llmposter::ServerBuilder;

// Default credentials: client_id="mock-client", client_secret="mock-secret"
let server = ServerBuilder::new()
    .with_oauth_defaults()
    .build()
    .await?;
// Tokens issued by the embedded OAuth server are accepted on LLM endpoints automatically.
```

## Pitfalls

### Empty substring match — silent catch-all

**Wrong:**
```rust
// Matches every request; shadows all later fixtures
Fixture::new()
    .match_user_message("")
    .respond_with_content("catch-all");
```

**Right:**
```rust
Fixture::new()
    .match_user_message("specific query")
    .respond_with_content("targeted response");
```

Empty patterns are rejected at startup since v0.3.5. Non-empty substrings are required.

### Blank tool name in tool-call fixture

**Wrong:**
```rust
// Rejected at fixture validation — blank tool names are not allowed
Fixture::new()
    .respond_with_tool_calls(vec![ToolCall {
        name: "".to_string(),
        arguments: serde_json::json!({"location": "London"}),
    }]);
```

**Right:**
```rust
Fixture::new()
    .respond_with_tool_calls(vec![ToolCall {
        name: "get_weather".to_string(),
        arguments: serde_json::json!({"location": "London"}),
    }]);
```

A blank `name` in any `ToolCall` is rejected at fixture validation startup, the same as empty match patterns.

### Tool-call arguments as a string

**Wrong:**
```yaml
response:
  tool_calls:
    - name: get_weather
      arguments: "location=London"   # string — rejected
```

**Right:**
```yaml
response:
  tool_calls:
    - name: get_weather
      arguments:          # YAML mapping required
        location: London
        unit: celsius
```

Anthropic and Gemini require tool-call arguments to be a JSON object. String values are rejected at fixture validation.

### `error` and `failure` in the same fixture

**Wrong:**
```rust
use llmposter::fixture::FailureConfig;
use llmposter::Fixture;

Fixture::new()
    .with_error(503, "overloaded")
    .with_failure(FailureConfig { latency_ms: Some(200), ..FailureConfig::default() });
// undefined behavior — mutually exclusive
```

**Right:**
```rust
use llmposter::fixture::FailureConfig;
use llmposter::Fixture;

// Use error for HTTP status codes
Fixture::new().with_error(503, "overloaded");

// Use failure for network-level faults at HTTP 200
Fixture::new()
    .respond_with_content("slow")
    .with_failure(FailureConfig { latency_ms: Some(200), ..FailureConfig::default() });
```

`with_error` returns an HTTP error status; `with_failure` returns HTTP 200 with a network-level fault.

### Checking tool-call IDs by exact counter value

**Wrong:**
```rust
// Counter is server-wide; value depends on prior requests in the test suite
assert_eq!(body["content"][0]["id"], "toolu_llmposter_1");
```

**Right:**
```rust
let id = body["content"][0]["id"].as_str().unwrap();
assert!(id.starts_with("toolu_llmposter_"));
```

Tool-call IDs use a server-wide atomic counter shared across all parallel requests. Exact values are not stable.

### Regex pattern exceeding DFA size limit

`match_user_message(pattern: &str)` accepts only substring patterns. Regex matching requires direct struct construction using `FixtureMatch { user_message: Some(StringMatch::regex(pattern)) }`.

**Wrong:**
```rust
use llmposter::fixture::{FixtureMatch, StringMatch};
use llmposter::Fixture;

// Complex alternation may produce a DFA exceeding 1 MB — rejected at startup
let _fixture = Fixture {
    match_rule: Some(FixtureMatch {
        user_message: Some(StringMatch::regex(r"(alpha|beta|gamma|delta|epsilon|...|omega){1,20}")),
        ..FixtureMatch::default()
    }),
    ..Fixture::new()
}
.respond_with_content("matched");
```

**Right:**
```rust
// Use a simpler pattern, or split into multiple fixtures
Fixture::new()
    .match_user_message("alpha")
    .respond_with_content("matched");
```

Regex patterns whose compiled DFA exceeds 1 MB are rejected at server startup — `build()` returns an error and the server does not start. Unbounded alternations, large repetition counts, and complex lookaheads are the most common triggers. Prefer simple substrings or narrow anchored patterns.

### Duration cast truncation

**Wrong:**
```rust
let ms = start.elapsed().as_millis() as u64; // as_millis() → u128; silently truncates
```

**Right:**
```rust
let ms = u64::try_from(start.elapsed().as_millis()).expect("duration fits u64");
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3

### v0.4.0 — MSRV bumped to 1.89

The `oauth-mock` dependency requires Rust ≥ 1.89. Update your toolchain before adding the dependency:

```text
rustup update stable
```

If your project cannot meet MSRV 1.89, opt out of the `oauth` feature:

```toml
[dev-dependencies]
llmposter = { version = "0.4.0", default-features = false }
```

### v0.3.4 — Responses API streaming protocol (breaking)

Events now use a nested `response` envelope and include `sequence_number`. The non-spec `response.done` event was removed.

```rust
// Before v0.3.4 — flat event, no sequence_number
// { "type": "response.output_item.done", ... }

// After v0.3.4 — nested envelope with sequence_number
// { "response": { "type": "response.output_item.done", ... }, "sequence_number": 3 }
// Use response.in_progress for in-flight state; response.done no longer exists
```

Update SSE parsers to unwrap the `response` envelope and read `sequence_number`. Remove any handling of `response.done`.

### v0.3.4 — Error response format (breaking)

```rust
// Before v0.3.4
let code: u64 = error["code"].as_u64().unwrap();

// After v0.3.4 — code is a string; param key always present (value may be null)
let code: &str = error["code"].as_str().unwrap();
let _param = &error["param"]; // always present; serde_json::Value::Null when not applicable
```

## API Reference

**`ServerBuilder::new()`** — Creates a builder with default settings (random port, auth disabled). Chain configuration methods before calling `build()`.

**`ServerBuilder::fixture(f: Fixture)`** — Registers one fixture. Fixtures are evaluated in registration order; first match wins.

**`ServerBuilder::build()`** — `async`. Validates all fixtures, starts the HTTP server, and returns `Result<MockServer, _>`. Regex patterns are compiled here; patterns with an invalid syntax or a compiled DFA exceeding 1 MB cause startup failure.

**`ServerBuilder::with_bearer_token(token: &str)`** — Registers an unlimited-use Bearer token and implicitly enables auth enforcement. Use `with_bearer_token_uses(token, n)` for expiring tokens.

**`ServerBuilder::load_yaml(path: &Path)`** — Loads and validates fixtures from a YAML file. Fallible (returns `Result`); not async. Use `load_yaml_dir` for a directory of files.

**`MockServer::url()`** — Returns the base URL of the running server (e.g., `http://127.0.0.1:PORT`). Drop `MockServer` to shut down the server.

**`Fixture::new()`** — Creates an empty fixture. Equivalent to `Fixture::default()`. All fields are also public for direct struct construction with `..Fixture::new()`.

**`Fixture::match_user_message(pattern: &str)`** — Case-sensitive substring match on the last user message. Non-empty required. Use `StringMatch::regex(pattern)` in struct construction for regex matching.

**`Fixture::respond_with_content(content: &str)`** — Sets a text content response. Mutually exclusive with `respond_with_tool_calls`.

**`Fixture::respond_with_tool_calls(tool_calls: Vec<ToolCall>)`** — Sets a tool-use response. `ToolCall { name: String, arguments: serde_json::Value }` — `arguments` must be a JSON object. Blank `name` is rejected at validation. Mutually exclusive with `respond_with_content`.

**`Fixture::with_error(status: u16, message: &str)`** — Returns an HTTP error response. `status` must be 400–599. Response body: `{"error": {"type": "...", "code": "...", "message": "...", "param": null}}`. The `x-request-id` header is always present. HTTP 429 responses additionally include provider-specific rate-limit headers. Mutually exclusive with `with_failure`.

**`Fixture::with_failure(failure: FailureConfig)`** — Injects a network-level fault at HTTP 200. Requires a response to also be set. Mutually exclusive with `with_error`.

**`Fixture::with_streaming(latency: Option<u64>, chunk_size: Option<usize>)`** — Enables SSE streaming. `latency` = ms between frames; `chunk_size` = characters per delta frame. Client must send `"stream": true`.

**`ToolCall`** — `struct ToolCall { name: String, arguments: serde_json::Value }`. `arguments` must be a JSON object. Blank `name` is rejected at fixture validation. Used with `respond_with_tool_calls`. Located at `llmposter::fixture::ToolCall`.

**`FailureConfig`** — `struct FailureConfig { latency_ms, corrupt_body, truncate_after_frames, disconnect_after_ms }`. All fields `Option<_>`; implements `Default`. Use `..FailureConfig::default()` for partial construction. The YAML key `truncate_after_chunks` is a deprecated alias for `truncate_after_frames`.
