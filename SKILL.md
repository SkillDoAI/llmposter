---
name: llmposter
description: In-process mock HTTP server for LLM provider APIs (OpenAI, Anthropic, Gemini, Responses) that matches fixtures, returns deterministic responses, and simulates failures without hitting real endpoints.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---


## Imports

```rust
// Core types — all re-exported at crate root
use llmposter::{Fixture, MockServer, ServerBuilder};
use llmposter::{FailureConfig, OAuthConfig, ToolCall};
```

```toml
# Cargo.toml — add as dev-dependencies (llmposter is a testing library)
[dev-dependencies]
llmposter = { version = "0.4.0" }
# To enable the embedded OAuth mock server:
# llmposter = { version = "0.4.0", features = ["oauth"] }
reqwest = { version = "0.13", default-features = false, features = ["json"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
```

## Core Patterns

### Text response fixture ✅ Current

Build a server, register a fixture that matches on message content, and assert the Anthropic-shaped response.

```rust
mod basic_text_response {
    use llmposter::{Fixture, MockServer, ServerBuilder};

    #[tokio::test]
    async fn anthropic_text_response() -> Result<(), Box<dyn std::error::Error>> {
        let server: MockServer = ServerBuilder::new()
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
        // Every LLM endpoint response includes x-request-id: req-llmposter-{N}
        let x_req_id = resp.headers().get("x-request-id").unwrap().to_str().unwrap();
        assert!(x_req_id.starts_with("req-llmposter-"));
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["content"][0]["text"], "Hi from Claude mock!");
        assert_eq!(body["stop_reason"], "end_turn");
        // Response ID format: msg-llmposter-{uuid}
        assert!(body["id"].as_str().unwrap().starts_with("msg-llmposter-"));
        Ok(())
    }
}
```

Fixture matching is first-match-wins across all registered fixtures. A fixture with no match conditions set matches every request. Requests with no matching fixture return `404` with body `{"error":{"message":"No fixture matched..."}}`.

Every LLM endpoint response — including error responses and 413 payload-too-large responses — includes an `x-request-id` header with the format `req-llmposter-{N}`, where N is a monotonically increasing request counter. The `x-request-id` header is present on 413 responses because `DefaultBodyLimit` is applied as an inner layer, so the outer request-ID middleware still runs.

### SSE streaming response ✅ Current

Chain `.with_streaming(latency_ms, chunk_size)` on any response fixture to enable SSE. `latency_ms` is the per-chunk delay; `chunk_size` is the number of characters per delta frame.

```rust
mod streaming_sse {
    use llmposter::{Fixture, MockServer, ServerBuilder};

    #[tokio::test]
    async fn anthropic_streaming_response() -> Result<(), Box<dyn std::error::Error>> {
        let server: MockServer = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("hello")
                    .respond_with_content("Hello world")
                    .with_streaming(Some(0), Some(5)), // 0 ms per chunk, 5 chars per frame
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

Tool call streaming emits approximately 7 frames: `message_start`, `content_block_start`, N `input_json_delta` frames, `content_block_stop`, `message_delta`, `message_stop`.

### Tool call response ✅ Current

Return a `tool_use` stop reason with `ToolCall` structs. Arguments must be a JSON object — arrays and scalars are rejected at `build()`.

```rust
mod tool_call_response {
    use llmposter::{Fixture, MockServer, ServerBuilder, ToolCall};

    #[tokio::test]
    async fn anthropic_tool_use_response() -> Result<(), Box<dyn std::error::Error>> {
        let server: MockServer = ServerBuilder::new()
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
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["name"], "get_weather");
        // Tool arguments appear under ["input"], not ["arguments"]
        assert_eq!(body["content"][0]["input"]["location"], "London");
        // Tool call ID format: toolu_llmposter_1
        assert!(body["content"][0]["id"].as_str().unwrap().starts_with("toolu_llmposter_"));
        // IDs are globally unique via a server-wide counter — no collisions across turns in multi-turn conversations
        Ok(())
    }
}
```

### Error and failure injection ✅ Current

Simulate HTTP errors, network latency, stream truncation, and corrupt responses.

```rust
mod failure_injection {
    use llmposter::{FailureConfig, Fixture, MockServer, ServerBuilder};
    use std::time::Instant;

    #[tokio::test]
    async fn http_error_response() -> Result<(), Box<dyn std::error::Error>> {
        let server: MockServer = ServerBuilder::new()
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
                "messages": [{"role": "user", "content": "rate limit"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 429);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["error"]["message"], "Rate limit exceeded");
        // Anthropic-endpoint 429s include provider-specific rate limit headers
        assert!(resp.headers().contains_key("anthropic-ratelimit-requests-limit"));
        assert!(resp.headers().contains_key("anthropic-ratelimit-requests-remaining"));
        assert!(resp.headers().contains_key("anthropic-ratelimit-requests-reset"));
        // x-request-id is present on error responses too
        assert!(resp.headers().get("x-request-id").unwrap().to_str().unwrap().starts_with("req-llmposter-"));
        Ok(())
    }

    #[tokio::test]
    async fn latency_injection() -> Result<(), Box<dyn std::error::Error>> {
        let server: MockServer = ServerBuilder::new()
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

        let start = Instant::now();
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

        assert_eq!(resp.status(), 200);
        assert!(start.elapsed().as_millis() >= 180);
        Ok(())
    }
}
```

`with_failure` requires a response to also be set. `with_error` is mutually exclusive with both `respond_with_content`/`respond_with_tool_calls` and `with_failure`. When `corrupt_body: Some(true)`, the server returns the literal string `"overloaded"` with `Content-Type: text/plain`, status 200, ignoring the fixture response content.

Provider-specific rate limit headers are included on 429 responses: the Anthropic endpoint (`/v1/messages`) returns `anthropic-ratelimit-requests-limit`, `anthropic-ratelimit-requests-remaining`, and `anthropic-ratelimit-requests-reset`; OpenAI and Responses API endpoints return `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, and `x-ratelimit-reset-requests`; the Gemini endpoint returns only `retry-after`.

## Configuration

### Bind address

Default bind is `127.0.0.1:0` (ephemeral port). Override with `.bind("host:port")`:

```rust
mod fixed_port {
    use llmposter::{Fixture, MockServer, ServerBuilder};

    #[tokio::test]
    async fn bind_to_fixed_port() -> Result<(), Box<dyn std::error::Error>> {
        let server: MockServer = ServerBuilder::new()
            .bind("127.0.0.1:9099")
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await?;
        assert!(server.url().contains(":9099"));
        Ok(())
    }
}
```

### YAML fixture loading

Both `load_yaml` and `load_yaml_dir` are synchronous and return `Result`. Chain with `?` before the `async` `.build().await?`:

```rust
mod yaml_loading {
    use llmposter::{MockServer, ServerBuilder};
    use std::path::Path;

    #[tokio::test]
    async fn load_fixtures_from_directory() -> Result<(), Box<dyn std::error::Error>> {
        let server: MockServer = ServerBuilder::new()
            .load_yaml_dir(Path::new("tests/fixtures"))?   // sync — note: no .await
            .build()
            .await?;
        assert!(!server.url().is_empty());
        Ok(())
    }
}
```

YAML fixture files use the schema: `match.user_message`, `match.model`, `response.content`, `response.tool_calls`, `error.status`, `error.message`. Plain string values under `match` fields are substring matches; `{regex: "..."}` activates regex matching.

All fixture structs use `serde(deny_unknown_fields)`. A typo in a YAML key (e.g. `mesage` instead of `message`) causes `load_yaml` / `load_yaml_dir` to return `Err` immediately — the server will not start.

### Bearer token auth

`with_bearer_token` implicitly enables auth and registers the token. Calling `with_auth(true)` without also registering a token causes all requests to return 401.

```rust
mod bearer_auth {
    use llmposter::{Fixture, MockServer, ServerBuilder};

    #[tokio::test]
    async fn bearer_token_testing() -> Result<(), Box<dyn std::error::Error>> {
        let server: MockServer = ServerBuilder::new()
            .with_bearer_token("my-test-token")
            .with_bearer_token_uses("expiring-token", 3)   // expires after 3 uses
            .fixture(Fixture::new().respond_with_content("authenticated"))
            .build()
            .await?;

        let client = reqwest::Client::new();

        // No token → 401
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "hello"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 401);

        // Valid token → 200
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .bearer_auth("my-test-token")
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "hello"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);
        Ok(())
    }
}
```

### OAuth mock server (feature = "oauth")

```toml
[dev-dependencies]
llmposter = { version = "0.4.0", features = ["oauth"] }
```

```rust
mod oauth_mock {
    use llmposter::{Fixture, MockServer, OAuthConfig, ServerBuilder};

    #[tokio::test]
    async fn oauth_custom_config() -> Result<(), Box<dyn std::error::Error>> {
        // with_oauth_defaults() uses: client_id="mock-client", client_secret="mock-secret"
        // Use with_oauth(OAuthConfig {...}) for custom credentials
        let server: MockServer = ServerBuilder::new()
            .with_oauth(OAuthConfig {
                client_id: "test-client".to_string(),
                client_secret: "test-secret".to_string(),
                redirect_uris: vec!["https://example.com/callback".to_string()],
                scopes: vec!["openid".to_string(), "profile".to_string()],
            })
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await?;
        // OAuth-issued tokens are automatically accepted on all LLM endpoints
        assert!(!server.url().is_empty());
        Ok(())
    }
}
```

### FailureConfig field reference

`FailureConfig` implements `Default`; use struct update syntax to set only the relevant fields:

```rust
// All fields are Option<T>
use llmposter::FailureConfig;

let _failure = FailureConfig {
    latency_ms: Some(500),           // pre-response delay in ms
    corrupt_body: Some(true),        // replaces body with "overloaded" (text/plain)
    truncate_after_frames: Some(2),  // cuts SSE stream after N frames
    disconnect_after_ms: Some(0),    // closes connection after N ms
};
```

`truncate_after_chunks` is a deprecated YAML alias for `truncate_after_frames`; use `truncate_after_frames` in code.

### Provider notes

**Gemini safety-blocked candidates**: When a Gemini response involves a safety-blocked candidate, the `Content.role` field may be absent (`Option<String>` — can be `None`). Parse it as optional rather than assuming it is always present.

## Pitfalls

### Catch-all fixture placed before specific ones

```rust
// Wrong — catch-all matches everything; the specific fixture is never reached
let _server = ServerBuilder::new()
    .fixture(Fixture::new().respond_with_content("catch all"))
    .fixture(Fixture::new().match_user_message("hello").respond_with_content("specific"))
    .build()
    .await
    .unwrap();
```

```rust
// Right — specific fixtures first, broad fallback last
let _server = ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("hello").respond_with_content("specific"))
    .fixture(Fixture::new().respond_with_content("catch all"))
    .build()
    .await
    .unwrap();
```

### Enabling auth without registering a token

```rust
// Wrong — auth is enforced but no token is registered; every request returns 401
let _server = ServerBuilder::new()
    .with_auth(true)
    .fixture(Fixture::new().respond_with_content("ok"))
    .build()
    .await
    .unwrap();
```

```rust
// Right — with_bearer_token implicitly enables auth and registers the token
let _server = ServerBuilder::new()
    .with_bearer_token("my-test-token")
    .fixture(Fixture::new().respond_with_content("ok"))
    .build()
    .await
    .unwrap();
```

### Tool-call arguments as array instead of object

```rust
use llmposter::ToolCall;

// Wrong — Anthropic requires tool arguments to be a JSON object; build() returns Err
let _tc = ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!(["San Francisco"]),  // array — rejected at build time
};
```

```rust
use llmposter::ToolCall;

// Right — arguments must be a JSON object
let _tc = ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!({"location": "San Francisco"}),
};
```

### Using `with_failure` without setting a response

```rust
use llmposter::{FailureConfig, Fixture};

// Wrong — with_failure applies on top of a response; without one, build() returns Err
let _fixture = Fixture::new()
    .with_failure(FailureConfig { latency_ms: Some(100), ..FailureConfig::default() });
```

```rust
use llmposter::{FailureConfig, Fixture};

// Right — always pair with_failure with respond_with_content or respond_with_tool_calls
let _fixture = Fixture::new()
    .respond_with_content("delayed response")
    .with_failure(FailureConfig { latency_ms: Some(100), ..FailureConfig::default() });
```

### Modifying request paths when pointing tests at llmposter

```text
# Wrong — prepending a prefix breaks routing
base_url = "http://127.0.0.1:2112/mock"
POST /mock/v1/messages   ← 404; llmposter does not recognize this path
```

```text
# Right — change only the host and port; leave all paths unchanged
base_url = "http://127.0.0.1:2112"
POST /v1/messages        ← routes correctly to the Anthropic mock handler
```

### Empty match patterns and blank tool names rejected at validation

Fixture validation (called automatically by `build()` and by `load_yaml` / `load_yaml_dir`) rejects several degenerate configurations that would cause silent bugs:

- **Empty substring pattern** — an empty string for `match.user_message` or `match.model` would silently behave as a catch-all. Validation rejects it; use no `match` field at all for an explicit catch-all fixture.
- **Empty regex pattern** — an empty `{regex: ""}` is rejected to prevent accidental universal matches and to avoid DFA compilation edge cases.
- **Blank tool name** — a `ToolCall` with an empty `name` string is rejected; every tool call must have a non-blank name.

### Regex patterns with large DFA state machines

The regex engine enforces a 1 MB DFA state-machine size limit. Patterns that generate a DFA exceeding this limit are rejected at `validate()` time (and therefore at `build()` or `load_yaml` time) to prevent out-of-memory conditions from malicious or pathological input. If a valid regex is rejected, simplify the pattern or split it across multiple fixtures.

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3

### MSRV bumped to Rust 1.89

Update `rust-toolchain.toml` and CI toolchain configuration. The new minimum is required by the `oauth-mock` dependency.

### Bearer token auth (new in v0.4.0)

Existing tests that do not use auth require no changes.

```rust
// v0.3 — no auth support

// v0.4 — opt-in bearer token auth; implicitly enables enforcement
let server = ServerBuilder::new()
    .with_bearer_token("my-token")
    .fixture(Fixture::new().respond_with_content("ok"))
    .build()
    .await?;
```

### OAuth mock server (new in v0.4.0)

Add `features = ["oauth"]` to the dev-dependency declaration. Use `.with_oauth_defaults()` for standard PKCE/device-code flows or `.with_oauth(OAuthConfig { ... })` for custom client config. OAuth-issued tokens are accepted automatically on all LLM endpoints — no separate token registration required.

### Responses API streaming protocol (v0.3.3 → v0.3.4)

Remove all handling of the `response.done` SSE event — it no longer exists. Add handling for `response.in_progress`. SSE events now use nested `response` envelopes; use the `sequence_number` field for ordering instead of arrival order.

### Responses API error shape (v0.3.3 → v0.3.4)

The `code` field in Responses API errors changed from integer to `String`. A nullable `param` field was added. Update any deserialization structs that parse Responses API error responses.

## API Reference

**ServerBuilder::new()** — Creates a new builder. Default bind: `127.0.0.1:0` (ephemeral port). Re-exported at crate root.

**ServerBuilder::fixture(f: Fixture)** — Appends a single fixture. Multiple calls accumulate; first-match-wins at request time.

**ServerBuilder::with_auth(enabled: bool)** — Enables or disables bearer token enforcement on all LLM endpoints. Must be paired with `with_bearer_token` to register a valid token.

**ServerBuilder::with_bearer_token(token: &str)** — Registers a bearer token with unlimited uses. Implicitly enables auth.

**ServerBuilder::with_bearer_token_uses(token: &str, max_uses: u64)** — Registers a bearer token that expires deterministically after `max_uses` requests. Implicitly enables auth. Use to test token refresh flows.

**ServerBuilder::with_oauth_defaults()** — Feature `"oauth"` required. Starts embedded OAuth server with default credentials (`client_id: "mock-client"`, `client_secret: "mock-secret"`). Implicitly enables auth.

**ServerBuilder::build()** — `async`. Validates all fixtures, binds the server, returns a running `MockServer`. Returns `Err` if any fixture fails validation.

**MockServer::url()** — Returns the server base URL as `String` (e.g., `"http://127.0.0.1:PORT"`). Point HTTP clients at this value.

**Fixture::new()** — Creates an empty fixture with all fields `None`. Use builder methods to configure match rules and responses.

**Fixture::match_user_message(pattern: &str)** — Substring match against the last user message. All active match conditions are AND-combined.

**Fixture::match_model(pattern: &str)** — Substring match against the `model` field in the request.

**Fixture::respond_with_content(content: &str)** — Sets a plain text response. Default `stop_reason`: `"end_turn"`. Mutually exclusive with `respond_with_tool_calls`.

**Fixture::respond_with_tool_calls(tool_calls: Vec<ToolCall>)** — Sets a tool-use response. Default `stop_reason`: `"tool_use"`. `ToolCall.arguments` must be a JSON object.

**Fixture::with_error(status: u16, message: &str)** — Returns an HTTP error response with body `{"error":{"message":"..."}}`. `status` must be 400–599. Mutually exclusive with response and `with_failure`.

**Fixture::with_failure(failure: FailureConfig)** — Injects latency, body corruption, stream truncation, or disconnect on top of the fixture response. Requires a response to be set. Mutually exclusive with `with_error`.

**Fixture::with_streaming(latency: Option<u64>, chunk_size: Option<usize>)** — Enables SSE streaming. `latency` = ms delay per chunk; `chunk_size` = chars per delta frame. Compatible with both text and tool call responses.
