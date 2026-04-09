---
name: llmposter
description: Rust library for mocking LLM API servers (Anthropic and OpenAI-compatible) in tests with configurable fixtures, failure injection, and streaming.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.3"
  ecosystem: rust
  generated-by: skilldo/claude-haiku-4-5 + review:claude-haiku-4-5
---

## Imports

Add to `Cargo.toml`:

```toml
[dev-dependencies]
llmposter = "0.4.3"
tokio = { version = "1", features = ["full"] }
serde_json = "1"
reqwest = { version = "0.13", default-features = false, features = ["json"] }

# OAuth feature (optional, on by default):
# llmposter = { version = "0.4.3", features = ["oauth"] }
```

Rust imports by type:

```rust
// Core types
use llmposter::{Fixture, Provider, ServerBuilder};

// FailureConfig and ToolCall
use llmposter::fixture::{FailureConfig, ToolCall};
```

## Core Patterns

### Minimal text response server

```rust
use llmposter::{Fixture, ServerBuilder};
use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_basic_text_response() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")      // substring match on last user message
                .respond_with_content("Hi from the mock!"),
        )
        .build()   // async — must .await
        .await?;

    let base_url = server.url(); // e.g. "http://127.0.0.1:PORT"
    
    // Verify the mock server responds to requests
    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-3-5-sonnet",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}],
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    
    // Server shuts down when `server` is dropped.
    Ok(())
}
```

Fixtures are evaluated in registration order; the first match wins. An unmatched request returns HTTP 404 with `{ "error": { "message": "No fixture matched" } }`.

### Tool-call response with provider filtering

```rust
use llmposter::fixture::ToolCall;
use llmposter::{Fixture, Provider, ServerBuilder};
use serde_json::json;

#[tokio::test]
async fn test_tool_call_response() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .for_provider(Provider::Anthropic)        // only matches /v1/messages
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: json!({                    // MUST be a JSON object
                        "location": "London",
                        "unit": "celsius"
                    }),
                }]),
        )
        .build()
        .await?;

    let _ = server.url();
    Ok(())
}
```

`for_provider` pins a fixture to one endpoint. An `Anthropic`-pinned fixture is invisible at `/v1/chat/completions` and vice versa. Unset fixtures match all providers.

### Tool call ID uniqueness across turns

Tool-call IDs are globally unique across the lifetime of the server via an internal counter with no multi-turn collisions. Each tool call receives a monotonically increasing ID. This guarantee holds even across multiple test requests within the same server instance, **including streaming responses** (v0.4.2+):

```rust
use llmposter::fixture::ToolCall;
use llmposter::{Fixture, ServerBuilder};
use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_tool_call_id_uniqueness() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("action")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "act".to_string(),
                    arguments: json!({}),
                }]),
        )
        .build()
        .await?;

    let client = Client::new();
    let base_url = server.url();

    // First request
    let resp1 = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-3-5-sonnet",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "action"}],
        }))
        .send()
        .await?;
    let body1: serde_json::Value = resp1.json().await?;
    let id1 = &body1["content"][0]["id"];

    // Second request — tool-call ID is guaranteed different
    let resp2 = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-3-5-sonnet",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "action"}],
        }))
        .send()
        .await?;
    let body2: serde_json::Value = resp2.json().await?;
    let id2 = &body2["content"][0]["id"];

    assert_ne!(id1, id2, "Tool call IDs must be globally unique across turns");
    Ok(())
}
```

### Gemini-specific request format and validation

Gemini requests use a different format from Anthropic and OpenAI. When a Gemini request includes a content item without a `role` field, it is treated as a user turn. Requests must have a substantive text content in the final turn:

```rust
use llmposter::{Fixture, Provider, ServerBuilder};
use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_gemini_request_format() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .for_provider(Provider::Gemini)
                .match_user_message("hello")
                .respond_with_content("Gemini response"),
        )
        .build()
        .await?;

    let client = Client::new();
    let base_url = server.url();

    // Correct Gemini format with explicit role
    let resp = client
        .post(format!("{}/v1/generateContent", base_url))
        .json(&json!({
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "hello"}]
                }
            ]
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);

    // Final turn must have substantive text — empty parts rejected
    let resp = client
        .post(format!("{}/v1/generateContent", base_url))
        .json(&json!({
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "hello"}]
                },
                {
                    "role": "user",
                    "parts": []   // empty — will be rejected
                }
            ]
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 400, "Empty final user turn must be rejected");

    Ok(())
}
```

### Responses API with incomplete_details

Responses API (`Provider::Responses`) is a variant supported for testing ChatGPT's backend API format. Responses with status `incomplete` emit an `incomplete_details` field containing a `reason` explaining why generation stopped. **v0.4.2+: this field is now present in both streaming and non-streaming responses**:

```rust
use llmposter::{Fixture, Provider, ServerBuilder};
use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_responses_api_incomplete() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .for_provider(Provider::Responses)
                .respond_with_content("partial generation")
                .with_finish_reason("max_tokens"),
        )
        .build()
        .await?;

    let client = Client::new();
    let base_url = server.url();

    // Responses API endpoint is /v1/responses
    let resp = client
        .post(format!("{}/v1/responses", base_url))
        .json(&json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "continue"}],
        }))
        .send()
        .await?;

    let body: serde_json::Value = resp.json().await?;
    
    // When finish_reason is set, the response status is incomplete and includes incomplete_details
    assert_eq!(body["status"].as_str(), Some("incomplete"));
    assert_eq!(
        body["incomplete_details"]["reason"].as_str(),
        Some("max_tokens")
    );

    Ok(())
}
```

### SSE streaming response

```rust
use llmposter::{Fixture, ServerBuilder};
use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_streaming_response() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("stream this")
                .respond_with_content("Streaming content here")
                .with_streaming(Some(0), Some(5)),  // REQUIRED: enables SSE; latency=0ms, 5 chars per frame
        )
        .build()
        .await?;

    let base_url = server.url();

    // Make a streaming request to verify the server returns Server-Sent Events
    let client = Client::new();
    let response = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-3-5-sonnet",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "stream this"}],
            "stream": true
        }))
        .send()
        .await?;

    assert_eq!(response.status(), 200);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(content_type.contains("text/event-stream"), "Expected text/event-stream, got: {}", content_type);

    Ok(())
}
```

**Important:** `.with_streaming(Some(0), Some(5))` is **required** to enable SSE responses. Omitting it leaves streaming disabled and returns JSON instead of Server-Sent Events.

Anthropic endpoint (`/v1/messages`) events:
- `message_start`, `content_block_start`, `content_block_delta`, `content_block_stop`, `message_delta`, `message_stop`

OpenAI/Responses API endpoints (`/v1/chat/completions`, `/v1/responses`) use different event formats. For OpenAI, streaming uses `chunk` events with content deltas; for Responses API, events follow the same delta structure. Total streaming time ≈ `ceil(content_len / chunk_size) × latency_ms`.

### Failure injection

```rust
use llmposter::fixture::FailureConfig;
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_failure_modes() -> Result<(), Box<dyn std::error::Error>> {
    // Latency before response
    let latency_fixture = Fixture::new()
        .respond_with_content("delayed")
        .with_failure(FailureConfig {
            latency_ms: Some(200),
            ..FailureConfig::default()
        });

    // Corrupt body
    let corrupt_fixture = Fixture::new()
        .respond_with_content("ignored")
        .with_failure(FailureConfig {
            corrupt_body: Some(true),
            ..FailureConfig::default()
        });

    // Truncate SSE stream after 2 frames (requires with_streaming)
    let truncate_fixture = Fixture::new()
        .respond_with_content("This is a very long response to truncate")
        .with_streaming(Some(0), Some(5))
        .with_failure(FailureConfig {
            truncate_after_frames: Some(2),
            ..FailureConfig::default()
        });

    // Drop the TCP connection mid-stream after 50 ms (requires with_streaming)
    let disconnect_fixture = Fixture::new()
        .respond_with_content("This will be cut short")
        .with_streaming(Some(0), Some(5))
        .with_failure(FailureConfig {
            disconnect_after_ms: Some(50),
            ..FailureConfig::default()
        });

    let _ = ServerBuilder::new()
        .fixture(latency_fixture)
        .fixture(corrupt_fixture)
        .fixture(truncate_fixture)
        .fixture(disconnect_fixture)
        .build()
        .await?;
    Ok(())
}
```

`latency_ms` and `corrupt_body` can be combined on the same `FailureConfig`; the delay is applied first. `with_failure` requires a response to also be set (via `respond_with_content` or `respond_with_tool_calls`). `disconnect_after_ms` closes the TCP connection mid-stream and is most useful with `with_streaming`.

### Bearer token authentication

```rust
use llmposter::{Fixture, ServerBuilder};
use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_bearer_auth() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .with_bearer_token("mock-test-token")       // unlimited uses
        .with_bearer_token_uses("one-shot-token", 1) // expires after 1 request
        .fixture(Fixture::new().respond_with_content("authorized"))
        .build()
        .await?;

    let client = Client::new();
    let base_url = server.url();

    // Request with valid token succeeds
    let resp = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-3-5-sonnet",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "test"}],
        }))
        .header("Authorization", "Bearer mock-test-token")
        .send()
        .await?;
    assert_eq!(resp.status(), 200);

    // Request without Authorization header receives HTTP 401
    let resp = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-3-5-sonnet",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "test"}],
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 401);

    // First use of one-shot token succeeds
    let resp = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-3-5-sonnet",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "test"}],
        }))
        .header("Authorization", "Bearer one-shot-token")
        .send()
        .await?;
    assert_eq!(resp.status(), 200);

    // Second use of exhausted token receives HTTP 401
    let resp = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-3-5-sonnet",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "test"}],
        }))
        .header("Authorization", "Bearer one-shot-token")
        .send()
        .await?;
    assert_eq!(resp.status(), 401);

    Ok(())
}
```

`with_bearer_token` and `with_bearer_token_uses` both implicitly enable auth (no separate `with_auth(true)` call required). Use `with_auth(false)` to explicitly disable auth on a builder that has tokens registered.

## Configuration

**Bind address**: The server binds to `127.0.0.1` on an OS-assigned port by default. Override with `.bind("127.0.0.1:8080")`.

**Fixture loading from YAML files**:

```rust
use llmposter::ServerBuilder;
use std::path::Path;

#[tokio::test]
async fn test_yaml_fixtures() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .load_yaml(Path::new("tests/fixtures/my_fixture.yaml"))?  // single file
        .load_yaml_dir(Path::new("tests/fixtures/"))?              // all *.yaml in dir
        .build()
        .await?;
    let _ = server.url();
    Ok(())
}
```

**Verbose logging**: `.verbose(true)` prints request/match details to stderr, including matched fixture information and request metadata. Response semantics are unchanged.

**Response headers**: Every successful LLM response (streaming and non-streaming) includes an `x-request-id` header with the value `req-llmposter-{N}` (N = monotonically increasing request counter). This header is also present on 413 body-too-large responses due to middleware layer ordering.

`with_error(429, ...)` responses inject provider-specific rate-limit headers in addition to the error body:
- OpenAI / Responses API: `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, `x-ratelimit-reset-requests`
- Anthropic: `anthropic-ratelimit-requests-limit`, `anthropic-ratelimit-requests-remaining`, `anthropic-ratelimit-requests-reset`
- Gemini: `retry-after`

**Error response bodies**: `with_error(status, message)` returns a provider-specific JSON body.

- **OpenAI / Responses API** (`/v1/chat/completions`, `/v1/responses`): `{ "error": { "type": "<string>", "code": "<string>", "param": null, "message": "<message>" } }` — `type`, `code`, and `message` are strings; `param` is always present and defaults to `null`.
- **Anthropic** (`/v1/messages`): `{ "type": "error", "error": { "type": "<string>", "message": "<message>" } }` — the outer `type` is always `"error"`; the inner `type` describes the error kind (e.g. `"api_error"`).

**Custom error response headers**: `with_error_headers(status, message, headers)` allows you to add custom headers to an error response. Status codes must be in the range 400–599:

```rust
use llmposter::{Fixture, ServerBuilder};
use std::collections::HashMap;

#[tokio::test]
async fn test_error_with_custom_headers() -> Result<(), Box<dyn std::error::Error>> {
    let mut custom_headers = HashMap::new();
    custom_headers.insert("X-Custom-Header".to_string(), "custom-value".to_string());
    
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("error")
                .with_error_headers(429, "Rate limited", custom_headers)?,
        )
        .build()
        .await?;

    let _ = server.url();
    Ok(())
}
```

`with_error_headers` takes a status code (400–599), error message, and an iterable of key-value pairs for headers. It returns `Result<Self, String>` to validate header construction. The method validates that header names and values are well-formed and returns a string error message if validation fails.

**OAuth (feature-gated)**:

```rust
// Cargo.toml: llmposter = { version = "0.4.3", features = ["oauth"] }
use llmposter::ServerBuilder;
use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_oauth_defaults() -> Result<(), Box<dyn std::error::Error>> {
    // Default: client_id="mock-client", client_secret="mock-secret"
    // redirect_uris=["https://example.com/callback"], scopes=["openid","profile","email"]
    let server = ServerBuilder::new()
        .with_oauth_defaults()
        .fixture(llmposter::Fixture::new().respond_with_content("ok"))
        .build()
        .await?;
    
    // Tokens issued by the embedded OAuth server are automatically treated as valid
    // on all LLM endpoints. No additional bearer token configuration is required.
    let base_url = server.url();
    let client = Client::new();
    
    // Make a request to verify the server is running
    let resp = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "test"}],
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    
    Ok(())
}
```

Tokens issued by the embedded OAuth server are automatically treated as valid on all LLM endpoints (`/v1/messages`, `/v1/chat/completions`, `/v1/generateContent`, `/v1/responses`). No additional `with_bearer_token()` call is required.

**GET /code/{N} utility endpoint (v0.4.1+)** — **Auth-exempt**:

```rust
use llmposter::ServerBuilder;
use reqwest::Client;

#[tokio::test]
async fn test_code_endpoint() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .with_bearer_token("required-for-lm-endpoints")
        .fixture(llmposter::Fixture::new().respond_with_content("ok"))
        .build()
        .await?;

    let client = Client::new();
    
    // GET /code/{N} does NOT require bearer token — auth is exempted
    // Request succeeds without Authorization header even though server requires auth for LM endpoints
    let resp = client.get(format!("{}/code/429", server.url())).send().await?;
    assert_eq!(resp.status(), 429);

    // GET /code/500 returns HTTP 500
    let resp = client.get(format!("{}/code/500", server.url())).send().await?;
    assert_eq!(resp.status(), 500);

    // Invalid codes (outside 200–599) return HTTP 400
    let resp = client.get(format!("{}/code/999", server.url())).send().await?;
    assert_eq!(resp.status(), 400);

    Ok(())
}
```

The `/code/{N}` endpoint is useful for testing HTTP error handling without crafting full LLM response fixtures. Valid codes: 200–599. Returns 400 for invalid or out-of-range codes. **This endpoint is exempt from authentication requirements** — requests succeed even without a bearer token, making it suitable for testing unauthenticated error paths.

## Pitfalls

### Wrong: Empty substring match silently catches all requests

```rust
Fixture::new()
    .match_user_message("")   // empty string matches every request
    .respond_with_content("unexpected catch-all")
```

### Right: Always provide a non-empty pattern

```rust
Fixture::new()
    .match_user_message("specific keyword")
    .respond_with_content("targeted response")
```

Rejected at fixture validation. When `.build()` is called, it internally validates all fixtures by calling `.validate()` on each. If an empty pattern is present, validation fails and `build()` returns `Err`.

### Wrong: Tool call arguments as array or scalar

```rust
use llmposter::fixture::ToolCall;

ToolCall {
    name: "search".to_string(),
    arguments: serde_json::json!(["query string"]),  // array — invalid
}
```

### Right: Tool call arguments must be a JSON object

```rust
use llmposter::fixture::ToolCall;

ToolCall {
    name: "search".to_string(),
    arguments: serde_json::json!({"query": "query string"}),  // object — valid
}
```

Both Anthropic and Gemini require tool call arguments to be JSON objects. Passing an array or scalar will cause the request to be rejected with HTTP 400.

### Wrong: `with_failure` without a response set

```rust
use llmposter::fixture::FailureConfig;
use llmposter::Fixture;

Fixture::new()
    .with_failure(FailureConfig {
        latency_ms: Some(200),
        ..FailureConfig::default()
    })
    // Missing: .respond_with_content(...) or .respond_with_tool_calls(...)
```

### Right: Always pair `with_failure` with a response

```rust
use llmposter::fixture::FailureConfig;
use llmposter::Fixture;

Fixture::new()
    .respond_with_content("delayed body")
    .with_failure(FailureConfig {
        latency_ms: Some(200),
        ..FailureConfig::default()
    })
```

### Wrong: Streaming config on non-streaming fixture

```rust
use llmposter::fixture::FailureConfig;
use llmposter::Fixture;

Fixture::new()
    .respond_with_content("no streaming set")
    .with_failure(FailureConfig {
        truncate_after_frames: Some(2),  // streaming config without with_streaming()
        ..FailureConfig::default()
    })
    // Missing: .with_streaming(Some(0), Some(5))
```

### Right: Pair streaming failure config with `with_streaming`

```rust
use llmposter::fixture::FailureConfig;
use llmposter::Fixture;

Fixture::new()
    .respond_with_content("will be truncated")
    .with_streaming(Some(0), Some(5))
    .with_failure(FailureConfig {
        truncate_after_frames: Some(2),
        ..FailureConfig::default()
    })
```

When `truncate_after_frames` or `disconnect_after_ms` are specified on a non-streaming response, the configuration is silently ignored and has no effect on the response. Always call `.with_streaming()` before using streaming-related failure modes to ensure your configuration is applied.

### Wrong: General fixture placed before specific fixture

```rust
use llmposter::{Fixture, ServerBuilder};

ServerBuilder::new()
    .fixture(Fixture::new().respond_with_content("generic"))         // matches everything
    .fixture(Fixture::new().match_user_message("error case").with_error(500, "boom"))
```

### Right: Specific patterns first, catch-all last

```rust
use llmposter::{Fixture, ServerBuilder};

ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("error case").with_error(500, "boom"))
    .fixture(Fixture::new().respond_with_content("generic fallback"))
```

A fixture with no match constraints (no `.match_user_message()` call) matches all requests and should be placed last to serve as a fallback.

### Wrong: HTTP error status code outside 400–599

```rust
Fixture::new().with_error(200, "not actually an error")  // rejected
Fixture::new().with_error(302, "redirect")               // rejected
```

### Right: Use status codes 400–599 only

```rust
Fixture::new()
    .match_user_message("rate limit")
    .with_error(429, "Rate limit exceeded")
```

Codes outside 400–599 are rejected at fixture validation.

### Wrong: Anthropic/Gemini request with latest user turn missing text content

```rust
// Anthropic — latest user turn is whitespace-only
json!({
    "model": "claude-sonnet-4-6",
    "max_tokens": 1024,
    "messages": [
        {"role": "user", "content": "hello"},
        {"role": "user", "content": "   "}   // whitespace only — now rejected
    ]
})

// Gemini — latest user turn missing text
json!({
    "contents": [
        {"role": "user", "parts": [{"text": "hello"}]},
        {"role": "user", "parts": []}   // empty parts — now rejected
    ]
})
```

### Right: Ensure latest user turn has substantive text

```rust
// Anthropic
json!({
    "model": "claude-sonnet-4-6",
    "max_tokens": 1024,
    "messages": [
        {"role": "user", "content": "hello"},
        {"role": "user", "content": "continue or ask follow-up"}   // text required
    ]
})

// Gemini
json!({
    "contents": [
        {"role": "user", "parts": [{"text": "hello"}]},
        {"role": "user", "parts": [{"text": "follow-up"}]}   // text required
    ]
})
```

**Why:** v0.4.1 removes silent fallback to stale prior user turns. This prevents bugs where client code sends incomplete requests but llmposter masks the error by using outdated context. Now returns HTTP 400 instead, making the bug obvious.

### Wrong: Non-boolean stream field

```rust
// String instead of boolean
json!({
    "model": "claude-sonnet-4-6",
    "stream": "true",   // string — now rejected with 400
    "messages": [{"role": "user", "content": "hello"}]
})

// Number instead of boolean
json!({
    "model": "gpt-4",
    "stream": 1,        // number — now rejected with 400
    "messages": [{"role": "user", "content": "hello"}]
})
```

### Right: Use JSON boolean for stream field

```rust
json!({
    "model": "claude-sonnet-4-6",
    "stream": true,     // boolean — correct
    "messages": [{"role": "user", "content": "hello"}]
})

json!({
    "model": "gpt-4",
    "stream": false,    // boolean — correct
    "messages": [{"role": "user", "content": "hello"}]
})
```

**Why:** v0.4.1 rejects non-boolean stream values with HTTP 400 to catch client SDK bugs that accidentally serialize stream as a string or number. This prevents silent wrong-behavior (request treated as non-streaming when client intended streaming).

## Migration Guide (v0.4.1 → v0.4.2 → v0.4.3)

### Streaming tool-call IDs now globally unique

**What changed:** Tool-call IDs in streaming responses are now globally unique across all requests on a server, matching the behavior of non-streaming responses. Previously, streaming tool-call IDs restarted at `toolu_llmposter_1` for each new request, causing collisions in multi-turn flows.

**Example:**
```rust
// v0.4.1: Two streaming requests for tool calls → both return id = "toolu_llmposter_1" (collision!)
// v0.4.2: Two streaming requests for tool calls → IDs are unique and increase over time
// (e.g. "toolu_llmposter_*" for Anthropic, "call_llmposter_*" for OpenAI)
```

**Migration:** If your tests assert on tool-call IDs, use `starts_with` or `contains("llmposter_")` rather than exact ID comparisons — the counter value depends on prior requests in the session.

### 404 no-match error redacted

**What changed:** When a request matches no fixture, the 404 response body no longer includes the user prompt text. Previously, the error response echoed back the user message, which could leak secrets or PII in CI logs.

**Example (v0.4.1):**
```json
{
  "error": {
    "message": "No fixture matched for user message: 'my-secret-api-key'"
  }
}
```

**Example (v0.4.2+):**
```json
{
  "error": {
    "message": "No fixture matched"
  }
}
```

**Migration:** Tests that parse 404 response bodies to verify the prompt text must be removed or updated. The response body is now identical for all unmatched requests and does not include the user input.

### Responses API streaming now includes incomplete_details

**What changed:** When using the Responses API (`Provider::Responses`) with streaming enabled, responses with status `incomplete` now include the `incomplete_details` object with a `reason` field. Previously, this field was only present in non-streaming responses.

**Example (v0.4.1):**
```rust
// Streaming Responses API response with status: incomplete
// incomplete_details field was missing
```

**Example (v0.4.2+):**
```rust
// Streaming Responses API response with status: incomplete
// incomplete_details.reason is now present
{
  "status": "incomplete",
  "incomplete_details": {
    "reason": "max_tokens"
  }
}
```

**Migration:** If your tests branch on `stop_reason` in streaming Responses API responses, update them to also check `incomplete_details.reason` as needed.

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)
- [CHANGELOG](https://github.com/SkillDoAI/llmposter/blob/main/CHANGELOG.md)
- [API Reference](https://docs.rs/llmposter/latest/llmposter/)