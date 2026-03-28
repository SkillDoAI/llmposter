---
name: llmposter
description: Rust library for mocking LLM API servers (Anthropic and OpenAI-compatible) in tests with configurable fixtures, failure injection, and streaming.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.1"
  ecosystem: rust
  generated-by: skilldo/claude-haiku-4-5 + review:claude-haiku-4-5
---

## Imports

Add to `Cargo.toml`:

```toml
[dev-dependencies]
llmposter = "0.4.1"
tokio = { version = "1", features = ["full"] }
serde_json = "1"
reqwest = { version = "0.13", default-features = false, features = ["json"] }

# OAuth feature (optional):
# llmposter = { version = "0.4.1", features = ["oauth"] }
```

Rust imports by type:

```rust
// Core types — re-exported at crate root
use llmposter::{Fixture, Provider, ServerBuilder};

// FailureConfig and ToolCall — sub-module path used throughout this document;
// also re-exported at crate root: use llmposter::{FailureConfig, ToolCall};
use llmposter::fixture::{FailureConfig, ToolCall};

// FixtureResponse — not re-exported at crate root; must import from sub-module
use llmposter::fixture::FixtureResponse;

// OAuth (requires features = ["oauth"]) — OAuthConfig is needed only when calling with_oauth(config: OAuthConfig)
// use llmposter::OAuthConfig;
```

## Core Patterns

### Minimal text response server

```rust
use llmposter::{Fixture, ServerBuilder};

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
    // POST {base_url}/v1/messages        → Anthropic format
    // POST {base_url}/v1/chat/completions → OpenAI format
    // Server shuts down when `server` is dropped.
    let _ = base_url;
    Ok(())
}
```

Fixtures are evaluated in registration order; the first match wins. An unmatched request returns HTTP 404 with `{ "error": { "message": "No fixture matched" } }`.

### Tool-call response with provider filtering

```rust
use llmposter::{fixture::ToolCall, Fixture, Provider, ServerBuilder};
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

Tool-call IDs are globally unique across the lifetime of the server via an internal counter with no multi-turn collisions. Each tool call receives a monotonically increasing ID. This guarantee holds even across multiple test requests within the same server instance:

```rust
use llmposter::{fixture::ToolCall, Fixture, ServerBuilder};
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
            "messages": [{"role": "user", "content": "action"}],
        }))
        .send()
        .await?;
    let body1: serde_json::Value = resp1.json().await?;
    let id1 = &body1["content"][0]["id"];  // e.g. "toolu_01..."

    // Second request — tool-call ID is guaranteed different
    let resp2 = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-3-5-sonnet",
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

    // Content item without role field is treated as user turn
    let resp = client
        .post(format!("{}/v1/generateContent", base_url))
        .json(&json!({
            "contents": [
                {
                    "parts": [{"text": "hello"}]   // no role — treated as user
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

Responses API (`Provider::Responses`) is a variant supported for testing ChatGPT's backend API format. Responses with status `incomplete` emit an `incomplete_details` field containing a `reason` explaining why generation stopped:

```rust
use llmposter::{fixture::FixtureResponse, Fixture, Provider, ServerBuilder};
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
    
    // When status is 'incomplete', the response includes incomplete_details
    if body["status"].as_str() == Some("incomplete") {
        assert!(
            body["incomplete_details"]["reason"].is_string(),
            "incomplete_details.reason must be present when status is 'incomplete'"
        );
    }

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

OpenAI/Responses API endpoints use a different event format. Total streaming time ≈ `ceil(content_len / chunk_size) × latency_ms`.

### Failure injection

```rust
use llmposter::{fixture::FailureConfig, Fixture, ServerBuilder};

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
        .build()
        .await?;
    Ok(())
}
```

`latency_ms` and `corrupt_body` can be combined on the same `FailureConfig`; the delay is applied first. `with_failure` requires a response to also be set (via `respond_with_content` or `respond_with_tool_calls`). `disconnect_after_ms` closes the TCP connection mid-stream and is most useful with `with_streaming`.

### Bearer token authentication

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_bearer_auth() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .with_bearer_token("mock-test-token")       // unlimited uses
        .with_bearer_token_uses("one-shot-token", 1) // expires after 1 request
        .fixture(Fixture::new().respond_with_content("authorized"))
        .build()
        .await?;

    // Requests without Authorization: Bearer <token> receive HTTP 401.
    // Requests with an exhausted token receive HTTP 401.
    let _ = server.url();
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

**Verbose logging**: `.verbose(true)` prints request/match details to stderr. Response semantics are unchanged.

**Response headers**: Every successful LLM response (streaming and non-streaming) includes an `x-request-id` header with the value `req-llmposter-{N}` (N = monotonically increasing request counter). This header is also present on 413 body-too-large responses due to middleware layer ordering.

`with_error(429, ...)` responses inject provider-specific rate-limit headers in addition to the error body:
- OpenAI / Responses API: `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, `x-ratelimit-reset-requests`
- Anthropic: `anthropic-ratelimit-requests-limit`, `anthropic-ratelimit-requests-remaining`, `anthropic-ratelimit-requests-reset`
- Gemini: `retry-after`

**Error response bodies**: `with_error(status, message)` returns a provider-specific JSON body.

- **OpenAI / Responses API** (`/v1/chat/completions`, `/v1/responses`): `{ "error": { "type": "<string>", "code": "<string>", "param": null, "message": "<message>" } }` — `type`, `code`, and `message` are strings; `param` is always present and defaults to `null`.
- **Anthropic** (`/v1/messages`): `{ "type": "error", "error": { "type": "<string>", "message": "<message>" } }` — the outer `type` is always `"error"`; the inner `type` describes the error kind (e.g. `"api_error"`). The body is strictly typed (`deny_unknown_fields`); no extra fields are present.

**OAuth (feature-gated)**:

```rust
// Cargo.toml: llmposter = { version = "0.4.1", features = ["oauth"] }
use llmposter::ServerBuilder;

#[tokio::test]
async fn test_oauth_defaults() -> Result<(), Box<dyn std::error::Error>> {
    // Default: client_id="mock-client", client_secret="mock-secret"
    // redirect_uris=["https://example.com/callback"], scopes=["openid","profile","email"]
    let server = ServerBuilder::new()
        .with_oauth_defaults()
        .fixture(llmposter::Fixture::new().respond_with_content("ok"))
        .build()
        .await?;
    let _ = server.url();
    Ok(())
}
```

Tokens issued by the embedded OAuth server are automatically treated as valid on all LLM endpoints (`/v1/messages`, `/v1/chat/completions`). No additional `with_bearer_token()` call is required.

**Custom stop/finish reason** (struct literal for full field control):

```rust
use llmposter::{fixture::FixtureResponse, Fixture};

let fixture = Fixture {
    response: Some(FixtureResponse {
        content: Some("hit token limit".to_string()),
        tool_calls: None,
        stop_reason: Some("max_tokens".to_string()),   // Anthropic field
        finish_reason: None,                            // OpenAI field
    }),
    ..Fixture::new()
};
```

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
    // GET /code/429 returns HTTP 429 with { "code": 429, "description": "..." }
    let resp = client.get(format!("{}/code/429", server.url())).send().await?;
    assert_eq!(resp.status(), 429);

    // GET /code/500 returns HTTP 500 with { "code": 500, "description": "..." }
    let resp = client.get(format!("{}/code/500", server.url())).send().await?;
    assert_eq!(resp.status(), 500);

    // Invalid codes (0, 600, non-numeric) return HTTP 400
    let resp = client.get(format!("{}/code/999", server.url())).send().await?;
    assert_eq!(resp.status(), 400);

    Ok(())
}
```

The `/code/{N}` endpoint is useful for testing HTTP error handling without crafting full LLM response fixtures. Valid codes: 100–599. Returns 400 for invalid or out-of-range codes. **This endpoint is exempt from authentication requirements** — requests succeed even without a bearer token, making it suitable for testing unauthenticated error paths.

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

Rejected at fixture validation since v0.3.5. `build()` returns `Err` if an empty pattern is present.

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

Both Anthropic and Gemini require tool call arguments to be JSON objects.

### Wrong: `with_failure` without a response set

```rust
use llmposter::{fixture::FailureConfig, Fixture};

Fixture::new()
    .with_failure(FailureConfig {
        latency_ms: Some(200),
        ..FailureConfig::default()
    })
    // Missing: .respond_with_content(...) or .respond_with_tool_calls(...)
```

### Right: Always pair `with_failure` with a response

```rust
use llmposter::{fixture::FailureConfig, Fixture};

Fixture::new()
    .respond_with_content("delayed body")
    .with_failure(FailureConfig {
        latency_ms: Some(200),
        ..FailureConfig::default()
    })
```

### Wrong: Streaming config on non-streaming fixture

```rust
use llmposter::{fixture::FailureConfig, Fixture};

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
use llmposter::{fixture::FailureConfig, Fixture};

Fixture::new()
    .respond_with_content("will be truncated")
    .with_streaming(Some(0), Some(5))
    .with_failure(FailureConfig {
        truncate_after_frames: Some(2),
        ..FailureConfig::default()
    })
```

When `truncate_after_frames` or `disconnect_after_ms` are specified on a non-streaming response, a validation warning is logged during fixture load. The failure config has no effect on non-streaming responses — this is a silent no-op. Always call `.with_streaming()` before using streaming-related failure modes.

### Wrong: General fixture placed before specific fixture

```rust
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

### Wrong: Empty regex pattern

```yaml
# YAML fixture
match:
  user_message: { regex: "" }   # empty — rejected at validation
```

### Right: Provide a non-empty regex pattern

```yaml
match:
  user_message: { regex: "keyword" }
```

Empty regex patterns are rejected at fixture validation time, the same as empty substring patterns. `build()` returns `Err` if any fixture contains an empty regex.

### Wrong: Regex pattern with DFA exceeding 1 MB

Very complex patterns with large alternation sets can produce a compiled DFA exceeding 1 MB.

### Right: Keep regex patterns simple

The `regex` crate's DFA size is capped at 1 MB per pattern. Patterns that exceed this limit are rejected at fixture validation time to prevent out-of-memory errors. Simplify or split overly complex alternation patterns.

### Wrong: Anthropic/Gemini request with latest user turn missing text content

```rust
// Anthropic — latest user turn is whitespace-only
json!({
    "model": "claude-sonnet-4-6",
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

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)
- [Migration Guide (v0.4.0 to v0.4.1)](https://github.com/SkillDoAI/llmposter/blob/main/UPGRADE.md)
- [API Reference](https://docs.rs/llmposter/latest/llmposter/)
