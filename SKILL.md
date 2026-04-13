---
name: llmposter
description: Rust library for mocking LLM API servers (OpenAI, Anthropic, Gemini, and Responses API) in tests with configurable fixtures, failure injection, and streaming.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.6"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---

## Imports

Add to `Cargo.toml`:

```toml
[dev-dependencies]
llmposter = "0.4.6"
tokio = { version = "1", features = ["full"] }
serde_json = "1"
reqwest = { version = "0.13", default-features = false, features = ["json"] }

# OAuth feature (optional, on by default):
# llmposter = { version = "0.4.6", features = ["oauth"] }

# Templating feature (OFF by default, opt-in):
# llmposter = { version = "0.4.6", features = ["templating"] }

# JSONPath matching (on by default):
# llmposter = { version = "0.4.6", features = ["jsonpath"] }
```

Rust imports by type:

```rust
// Core types (re-exported at crate root)
use llmposter::{Fixture, Provider, ServerBuilder};

// Failure and tool-call types (re-exported at crate root)
use llmposter::{FailureConfig, ToolCall};

// Request capture types (re-exported at crate root)
use llmposter::{CapturedRequest, RequestOutcome};

// Additional re-exported types at crate root:
// llmposter::{MockServer, AuthState, TokenStatus, StreamingConfig, ScenarioConfig, Refusal, OAuthConfig}

// Types NOT re-exported at crate root (submodule path required):
// llmposter::fixture::{FixtureMatch, FixtureResponse, FixtureError, StringMatch, RegexMatch, F64Match, F64Range}
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
            "model": "claude-sonnet-4-6",
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

Fixtures are evaluated using a two-pass priority system. In the first pass, non-catch-all fixtures are sorted by descending priority (default 0) and the first match wins. If no non-catch-all fixture matches, a second pass considers catch-all fixtures. Within each pass, ties in priority fall back to registration order. See [Priority and catch-all matching](#priority-and-catch-all-matching) for details.

An unmatched request returns HTTP 404 with a provider-specific error body (e.g., OpenAI: `{ "error": { "message": "No fixture matched for model='...'" } }`, Anthropic: `{ "type": "error", "error": { ... } }`, Gemini: `{ "error": { "code": 404, ... } }`).

### Fixture match methods

Beyond `match_user_message`, fixtures support several additional match constraints. All match fields stack via AND — a fixture with multiple match constraints only matches requests satisfying every condition.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_match_methods() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        // Match by model name (substring match)
        .fixture(
            Fixture::new()
                .match_model("claude-sonnet")
                .respond_with_content("Sonnet response"),
        )
        // Match by system prompt content (substring match)
        .fixture(
            Fixture::new()
                .match_system_prompt("You are a helpful assistant")
                .respond_with_content("System prompt matched"),
        )
        // Match by HTTP header (substring match on value, keys are lowercased at load time)
        .fixture(
            Fixture::new()
                .match_header("x-custom-tenant", "acme-corp")
                .respond_with_content("Tenant matched"),
        )
        // Match by exact temperature value (plain f64 equality)
        .fixture(
            Fixture::new()
                .match_temperature(0.7)
                .respond_with_content("Temperature matched"),
        )
        // Match by temperature range (inclusive bounds, either bound optional)
        .fixture(
            Fixture::new()
                .match_temperature_range(Some(0.5), Some(1.0))
                .respond_with_content("Temperature in range"),
        )
        // Match by metadata key-value pair
        .fixture(
            Fixture::new()
                .match_metadata("env", "staging")
                .respond_with_content("Metadata matched"),
        )
        // Match by tool schema content (substring match on serialized tool definitions)
        .fixture(
            Fixture::new()
                .match_tool_schema("get_weather")
                .respond_with_content("Tool schema matched"),
        )
        // Match by JSONPath expression on the request body (requires 'jsonpath' feature, on by default)
        .fixture(
            Fixture::new()
                .match_body_jsonpath("$.messages[?(@.role == 'system')]")
                .respond_with_content("JSONPath matched"),
        )
        // Combine multiple match constraints (AND logic)
        .fixture(
            Fixture::new()
                .match_user_message("deploy")
                .match_model("gpt-4")
                .match_header("x-env", "production")
                .respond_with_content("All conditions met"),
        )
        .build()
        .await?;

    let _ = server.url();
    Ok(())
}
```

**Notes:**
- `match_model` and `match_user_message` use substring matching. For regex matching, use the YAML fixture format with `regex:` syntax.
- `match_header` keys are lowercased at fixture load time; post-fold duplicate keys are rejected.
- `match_temperature` uses plain f64 equality (not epsilon-based). Use `match_temperature_range` for tolerance-based matching.
- `match_body_jsonpath` requires the `jsonpath` feature (on by default). Invalid JSONPath expressions are rejected at fixture load time. If the `jsonpath` feature is disabled, the `body_jsonpath` field is still present in the struct (so serde gives a clear validation error rather than a confusing "unknown field" message), but any fixture using it will be rejected at load time.

### Tool-call response with provider filtering

```rust
use llmposter::{Fixture, Provider, ServerBuilder, ToolCall};
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

`for_provider` pins a fixture to one endpoint. An `Anthropic`-pinned fixture is invisible at `/v1/chat/completions` and vice versa. Unset fixtures match all providers. Provider variants: `Provider::Anthropic` (`/v1/messages`), `Provider::OpenAI` (`/v1/chat/completions`), `Provider::Gemini` (`/v1beta/models/{model}:generateContent`), `Provider::Responses` (`/v1/responses`).

### Tool call ID uniqueness across turns

Tool-call IDs are globally unique across the lifetime of the server via an internal counter with no multi-turn collisions. Each tool call receives a monotonically increasing ID. This guarantee holds even across multiple test requests within the same server instance, **including streaming responses** (v0.4.2+):

```rust
use llmposter::{Fixture, ServerBuilder, ToolCall};
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
            "model": "claude-sonnet-4-6",
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
            "model": "claude-sonnet-4-6",
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

### Safety refusal responses

Use `respond_with_refusal` to simulate an LLM refusing a request for safety reasons:

```rust
use llmposter::{Fixture, ServerBuilder};
use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_refusal_response() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("harmful")
                .respond_with_refusal("I cannot help with that request."),
        )
        .build()
        .await?;

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "something harmful"}],
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);

    Ok(())
}
```

**Important:** `respond_with_refusal` is mutually exclusive with `respond_with_content`, `with_error`, `with_failure`, and `with_streaming`. A refusal fixture matched against a `stream: true` request returns HTTP 400 because streaming refusal envelopes are not yet implemented. Only use refusal fixtures for non-streaming requests.

### Custom stop reason with `with_stop_reason` and `with_finish_reason`

Use `with_stop_reason` or `with_finish_reason` to override the default stop reason in responses. Both methods are functionally equivalent (aliases):

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_custom_stop_reason() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        // with_stop_reason — sets the stop_reason field
        .fixture(
            Fixture::new()
                .match_user_message("truncated")
                .respond_with_content("hit the limit")
                .with_stop_reason("max_tokens"),
        )
        // with_finish_reason — alias for with_stop_reason
        .fixture(
            Fixture::new()
                .match_user_message("partial")
                .respond_with_content("partial generation")
                .with_finish_reason("max_tokens"),
        )
        .build()
        .await?;

    let _ = server.url();
    Ok(())
}
```

Default stop reason is `end_turn` for Anthropic, `stop` for OpenAI. Tool-call responses default to `tool_use` (Anthropic) or `tool_calls` (OpenAI).

### Priority and catch-all matching

Fixture matching uses a two-pass algorithm. Use `with_priority` and `as_catch_all` to control match order beyond simple registration order:

```rust
use llmposter::{Fixture, ServerBuilder};
use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_priority_and_catch_all() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        // Low priority — matches if nothing higher does
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("low priority")
                .with_priority(-1),
        )
        // High priority — wins even though it was registered second
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("high priority")
                .with_priority(10),
        )
        // Catch-all — only matches if ALL non-catch-all fixtures fail
        .fixture(
            Fixture::new()
                .as_catch_all()
                .respond_with_content("fallback response"),
        )
        .build()
        .await?;

    let client = Client::new();
    let base_url = server.url();

    // "hello" matches the priority=10 fixture, not priority=-1
    let resp = client
        .post(format!("{}/v1/chat/completions", base_url))
        .json(&json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello world"}],
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);

    // Unmatched message hits the catch-all
    let resp = client
        .post(format!("{}/v1/chat/completions", base_url))
        .json(&json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "something else"}],
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);

    Ok(())
}
```

**Matching algorithm:**
1. **First pass:** Non-catch-all fixtures are sorted by descending priority (default 0). Higher priority wins regardless of registration order. Ties fall back to registration order.
2. **Second pass:** If no non-catch-all fixture matched, catch-all fixtures (`as_catch_all()`) are considered. Within the catch-all pass, priority and registration order still apply.

**Important:** A bare fixture with no match constraints participates in the first pass and matches everything immediately. Use `as_catch_all()` explicitly for fallback behavior — it defers to the second pass.

### Gemini-specific request format and validation

Gemini requests use a different format from Anthropic and OpenAI. When a Gemini request includes a content item without a `role` field, it is treated as a user turn. Requests must have substantive text content in the final turn:

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
        .post(format!("{}/v1beta/models/gemini-1.5-flash:generateContent", base_url))
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
        .post(format!("{}/v1beta/models/gemini-1.5-flash:generateContent", base_url))
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

    // Responses API endpoint is /v1/responses — uses "input" field, NOT "messages"
    let resp = client
        .post(format!("{}/v1/responses", base_url))
        .json(&json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "continue"}],
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
            "model": "claude-sonnet-4-6",
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

**Important:** `.with_streaming(Some(0), Some(5))` is **required** to enable SSE responses. The first argument is inter-chunk latency in milliseconds, the second is chunk size in characters. Omitting `with_streaming` leaves streaming disabled and returns JSON instead of Server-Sent Events.

Anthropic endpoint (`/v1/messages`) events:
- `message_start`, `content_block_start`, `content_block_delta`, `content_block_stop`, `message_delta`, `message_stop`

OpenAI/Responses API endpoints (`/v1/chat/completions`, `/v1/responses`) use different event formats. Total streaming time ≈ `ceil(content_len / chunk_size) × latency_ms`.

### Failure injection

```rust
use llmposter::{FailureConfig, Fixture, ServerBuilder};

#[tokio::test]
async fn test_failure_modes() -> Result<(), Box<dyn std::error::Error>> {
    // Latency before response
    let latency_fixture = Fixture::new()
        .respond_with_content("delayed")
        .with_failure(FailureConfig {
            latency_ms: Some(200),
            ..FailureConfig::default()
        });

    // Corrupt body (non-streaming: returns plain text "overloaded" with Content-Type: text/plain)
    // For streaming requests: returns a single malformed SSE frame "data: overloaded\n\n"
    // with Content-Type: text/event-stream
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
    // Injects a ConnectionReset error into the SSE stream (not a clean EOF)
    let disconnect_fixture = Fixture::new()
        .respond_with_content("This will be cut short")
        .with_streaming(Some(10), Some(5))  // latency > 0 needed for disconnect to race correctly
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

`latency_ms` and `corrupt_body` can be combined on the same `FailureConfig`; the delay is applied first. `with_failure` requires a response to also be set (via `respond_with_content` or `respond_with_tool_calls`). `disconnect_after_ms` closes the TCP connection mid-stream and is most useful with `with_streaming` — use `latency > 0` on the stream so the select! has an actual await point to interrupt on.

**Classical failure fields** (`latency_ms`, `corrupt_body`, `truncate_after_frames`, `disconnect_after_ms`) always fire when set and are NOT gated by probability.

**Chaos failure fields** (`duplicate_frames`, `latency_jitter_ms`, `probability`, `chaos_seed`) provide additional simulation capabilities:
- `duplicate_frames: true` — duplicates each SSE frame during streaming. When combined with `truncate_after_frames: N`, duplication runs first, so truncation counts doubled frames (N/2 original frames if N is even).
- `latency_jitter_ms` — adds random jitter to per-frame streaming delay; requires non-zero `streaming.latency` to act on.
- Chaos fields are seeded for reproducibility — same seed + same request order = bit-identical chaos.

**`corrupt_body` streaming behavior:** On streaming SSE requests, `corrupt_body: true` returns a single malformed SSE frame (`data: overloaded\n\n`) with `Content-Type: text/event-stream`. On non-streaming requests and Gemini JSON-array responses, it returns plain text `overloaded` with `Content-Type: text/plain`.

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
            "model": "claude-sonnet-4-6",
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
            "model": "claude-sonnet-4-6",
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
            "model": "claude-sonnet-4-6",
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
            "model": "claude-sonnet-4-6",
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

### Stateful multi-turn scenarios (v0.4.3+)

Scenarios enable multi-turn fixture matching via named state machines. A fixture can require a specific state to match and advance the state after matching — ideal for testing tool-call loops, retry sequences, and conversation branching.

```rust
use llmposter::{Fixture, ServerBuilder, ToolCall};
use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_tool_call_loop() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        // Step 1: ask about weather → tool call (initial state)
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: json!({"location": "Paris"}),
                }])
                .with_scenario("weather-flow", Some(""), Some("tool_called")),
        )
        // Step 2: after tool call → text response (requires tool_called state)
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_content("It's 22°C in Paris")
                .with_scenario("weather-flow", Some("tool_called"), Some("done")),
        )
        .build()
        .await?;

    let client = Client::new();
    let base_url = server.url();

    // First request: fixture 1 matches (state is empty), advances state to "tool_called"
    let resp = client
        .post(format!("{}/v1/chat/completions", base_url))
        .json(&json!({
            "model": "gpt-4",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "weather in Paris"}]
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    assert_eq!(server.scenario_state("weather-flow"), Some("tool_called".to_string()));

    // Second request: fixture 2 matches (state is "tool_called"), advances to "done"
    let resp = client
        .post(format!("{}/v1/chat/completions", base_url))
        .json(&json!({
            "model": "gpt-4",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "weather in Paris"}]
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    assert_eq!(server.scenario_state("weather-flow"), Some("done".to_string()));

    Ok(())
}
```

Scenario config fields:
- `name`: scenario identifier (shared across fixtures in the same flow)
- `required_state`: only match when scenario is in this state (`None` = always match, `Some("")` = match only when unset/initial)
- `set_state`: advance to this state after matching (`None` = no change)

Use `server.scenario_state(name)` to query state at any point. Use `server.reset()` to clear all scenarios and captured requests between test phases.

### Request capture and assertion API (v0.4.3+)

llmposter automatically captures every request received. Use the capture API to verify what your client actually sent — not just what it received.

```rust
use llmposter::{CapturedRequest, Fixture, RequestOutcome, ServerBuilder};
use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_client_sends_correct_model() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await?;

    let client = Client::new();
    client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await?;

    // Verify what the client sent
    let requests = server.get_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/v1/chat/completions");

    // Check the outcome — was a fixture matched?
    assert!(requests[0].was_matched());
    // Or match on the outcome enum directly (always include a catch-all arm — it's #[non_exhaustive])
    match requests[0].outcome {
        RequestOutcome::Matched => { /* fixture was selected (includes error fixtures) */ }
        RequestOutcome::NoFixtureMatch => { /* no fixture matched — 404 returned */ }
        RequestOutcome::BadRequest => { /* malformed request — 400 returned */ }
        RequestOutcome::AuthRejected => { /* auth failure — 401 returned; body is empty string */ }
        RequestOutcome::CodeEndpoint => { /* GET /code/{N} request */ }
        _ => { /* future variants */ }
    }

    let body: serde_json::Value = serde_json::from_str(&requests[0].body)?;
    assert_eq!(body["model"], "gpt-4");
    assert_eq!(body["messages"][0]["content"], "hello");

    // Or use request_count() for quick checks
    assert_eq!(server.request_count(), 1);

    Ok(())
}
```

`CapturedRequest` fields (the struct is `#[non_exhaustive]`): `method` (always "POST" for LLM endpoints), `path`, `body` (raw JSON string — **empty string for auth-rejected requests** since the body is not captured when auth fails), `outcome` (`RequestOutcome` — whether the request was matched, rejected, etc.), `matched_scenario` (scenario name if any), `timestamp`.

**Note:** `was_matched()` / `RequestOutcome::Matched` means "a fixture was selected", NOT "HTTP 200 was returned". Error fixtures returning 4xx/5xx and refusal fixtures are also considered Matched. A 429 from an error fixture is still `Matched`.

**Note:** When `outcome` is `RequestOutcome::AuthRejected`, the `body` field is an empty string — path, method, and outcome are still captured, but the request body is not. If you parse captured request bodies with `serde_json::from_str`, guard against empty strings for auth-rejected entries.

### Hot-swapping fixtures at runtime

Use `set_fixtures` to replace all fixtures on a running server without restarting:

```rust
use llmposter::{Fixture, ServerBuilder};
use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_hot_swap_fixtures() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("phase 1"))
        .build()
        .await?;

    let client = Client::new();
    let base_url = server.url();

    // Phase 1 response
    let resp = client
        .post(format!("{}/v1/chat/completions", base_url))
        .json(&json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "test"}],
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);

    // Swap fixtures — old fixtures are replaced atomically
    server.set_fixtures(vec![
        Fixture::new()
            .match_user_message("test")
            .respond_with_content("phase 2"),
    ])?;

    // Phase 2 response
    let resp = client
        .post(format!("{}/v1/chat/completions", base_url))
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

If `set_fixtures` is called with invalid fixtures, it returns an error and the previously loaded fixtures continue serving unchanged. Scenario state is preserved across fixture swaps.

### Checking for server errors

Use `check_error` to verify the server encountered no internal errors:

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_check_error() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await?;

    // ... send requests ...

    // Verify server had no internal errors
    server.check_error().await?;

    Ok(())
}
```

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

**Batch fixture loading**: Use `.fixtures(vec![...])` to add multiple fixtures at once, or `.fixture(f)` to add one at a time. Use `.fixture_count()` on the builder or running server to check how many fixtures are loaded.

**Hot-reload with file watching**: Use `.watch(true)` (requires `watch` feature, on by default) to enable automatic fixture reloading when YAML files change on disk. The server also reloads on SIGHUP signals. Invalid YAML during hot-reload is logged to stderr and the previous fixtures continue serving unchanged — partial edits never take down the live server.

```rust
use llmposter::ServerBuilder;
use std::path::Path;

#[tokio::test]
async fn test_watch_mode() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .load_yaml_dir(Path::new("tests/fixtures/"))?
        .watch(true)  // auto-reload when fixture files change
        .build()
        .await?;
    let _ = server.url();
    Ok(())
}
```

**Note on SIGHUP:** SIGHUP is process-wide. When multiple `MockServer` instances exist, each installs its own handler and all reload on every signal, each from its own source list. Programmatically-added fixtures (via `ServerBuilder::fixture()` or `set_fixtures()`) are untouched by file-based hot-reload.

**Capture capacity**: Use `.capture_capacity(max)` on `ServerBuilder` to bound the number of captured requests in memory. Useful for long-lived standalone servers. Short `#[tokio::test]` servers default to unbounded. Use `.capture_capacity(0)` to disable request capture entirely.

**Verbose logging**: `.verbose(true)` prints request/match details to stderr, including matched fixture information and request metadata. Response semantics are unchanged.

**Response headers**: Every HTTP response from llmposter includes an `x-request-id` header with the deterministic value `req-llmposter-{N}` (N = monotonically increasing request counter). This applies to all responses regardless of status code.

`with_error(429, ...)` responses inject provider-specific rate-limit headers in addition to the error body:
- OpenAI / Responses API: `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, `x-ratelimit-reset-requests`
- Anthropic: `anthropic-ratelimit-requests-limit`, `anthropic-ratelimit-requests-remaining`, `anthropic-ratelimit-requests-reset`
- Gemini: `retry-after`

**Error response bodies**: `with_error(status, message)` returns a provider-specific JSON body.

- **OpenAI / Responses API** (`/v1/chat/completions`, `/v1/responses`): `{ "error": { "type": "<string>", "code": "<string>", "param": null, "message": "<message>" } }`
- **Anthropic** (`/v1/messages`): `{ "type": "error", "error": { "type": "<string>", "message": "<message>" } }`

**Custom error response headers**: `with_error_headers(status, message, headers)` allows you to add custom headers to an error response. Status codes must be in the range 400–599:

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_error_with_custom_headers() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("error")
                .with_error_headers(429, "Rate limited", [
                    ("X-Custom-Header", "custom-value"),
                ])?,
        )
        .build()
        .await?;

    let _ = server.url();
    Ok(())
}
```

`with_error_headers` takes a status code (400–599), error message, and an iterable of key-value pairs for headers (`IntoIterator<Item = (K, V)>` where `K: AsRef<str>, V: AsRef<str>`). It returns `Result<Self, String>` to validate header construction. The method validates that header names and values are well-formed and returns a string error message if validation fails.

**OAuth (feature-gated)**:

```rust
// Cargo.toml: llmposter = { version = "0.4.6", features = ["oauth"] }
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

    let resp = client
        .post(format!("{}/v1/chat/completions", base_url))
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

Tokens issued by the embedded OAuth server are automatically treated as valid on all LLM endpoints (`/v1/messages`, `/v1/chat/completions`, `/v1beta/models/{model}:generateContent`, `/v1/responses`). No additional `with_bearer_token()` call is required. Use `server.oauth_url()` to get the OAuth server URL, `server.oauth_client_credentials().await` to get the `(client_id, client_secret)` pair, and `server.approve_device_code(user_code).await?` to approve a device authorization code.

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
    let resp = client.get(format!("{}/code/429", server.url())).send().await?;
    assert_eq!(resp.status(), 429);

    // GET /code/500 returns HTTP 500
    let resp = client.get(format!("{}/code/500", server.url())).send().await?;
    assert_eq!(resp.status(), 500);

    // Invalid codes (outside 100–599) return HTTP 400
    let resp = client.get(format!("{}/code/999", server.url())).send().await?;
    assert_eq!(resp.status(), 400);

    let resp = client.get(format!("{}/code/99", server.url())).send().await?;
    assert_eq!(resp.status(), 400);

    Ok(())
}
```

The `/code/{N}` endpoint is useful for testing HTTP error handling without crafting full LLM response fixtures. Valid codes: 100–599. Returns 400 for invalid or out-of-range codes. Special cases: 204/205/304 return empty body; 3xx responses include `Location: /` header. **This endpoint is exempt from authentication requirements** — requests succeed even without a bearer token.

**Feature flags**:

| Feature | Default | Description |
|---------|---------|-------------|
| `oauth` | on | OAuth 2.0 mock server via `oauth-mock` |
| `watch` | on | File-watching hot-reload via `notify-debouncer-mini` |
| `jsonpath` | on | RFC 9535 JSONPath matching via `match_body_jsonpath` |
| `templating` | OFF | Jinja-style response templating via `content_template` |

When building with `--no-default-features`, explicitly opt in to needed features. Using `content_template` in fixtures without the `templating` feature causes rejection at fixture load time with an error pointing at the feature flag. Similarly, using `match_body_jsonpath` (or `body_jsonpath` in YAML) without the `jsonpath` feature is rejected at fixture load time — the field is always present in the struct so serde gives a clear validation error rather than a confusing "unknown field" message. Template render errors at request time return HTTP 500 without crashing the server.

Using `match_body_jsonpath` with syntactically invalid JSONPath expressions is rejected at fixture load time during validation, not at request time. Regex patterns exceeding 1MB DFA size are also rejected at fixture validation to prevent OOM.

## Pitfalls

### Wrong: Empty substring match silently catches all requests

```rust
use llmposter::Fixture;

Fixture::new()
    .match_user_message("")   // empty string — rejected at validation
    .respond_with_content("unexpected catch-all");
```

### Right: Always provide a non-empty pattern

```rust
use llmposter::Fixture;

Fixture::new()
    .match_user_message("specific keyword")
    .respond_with_content("targeted response");
```

Rejected at fixture validation. When `.build()` is called, it internally validates all fixtures by calling `.validate()` on each. If an empty pattern is present, validation fails and `build()` returns `Err`.

### Wrong: Tool call arguments as array or scalar

```rust
use llmposter::ToolCall;

ToolCall {
    name: "search".to_string(),
    arguments: serde_json::json!(["query string"]),  // array — invalid
};
```

### Right: Tool call arguments must be a JSON object

```rust
use llmposter::ToolCall;

ToolCall {
    name: "search".to_string(),
    arguments: serde_json::json!({"query": "query string"}),  // object — valid
};
```

Both Anthropic and Gemini require tool call arguments to be JSON objects. Passing an array or scalar will cause the request to be rejected with HTTP 400.

### Wrong: `with_failure` without a response set

```rust
use llmposter::{FailureConfig, Fixture};

Fixture::new()
    .with_failure(FailureConfig {
        latency_ms: Some(200),
        ..FailureConfig::default()
    });
    // Missing: .respond_with_content(...) or .respond_with_tool_calls(...)
```

### Right: Always pair `with_failure` with a response

```rust
use llmposter::{FailureConfig, Fixture};

Fixture::new()
    .respond_with_content("delayed body")
    .with_failure(FailureConfig {
        latency_ms: Some(200),
        ..FailureConfig::default()
    });
```

### Wrong: Streaming config on non-streaming fixture

```rust
use llmposter::{FailureConfig, Fixture};

Fixture::new()
    .respond_with_content("no streaming set")
    .with_failure(FailureConfig {
        truncate_after_frames: Some(2),  // streaming config without with_streaming()
        ..FailureConfig::default()
    });
    // Missing: .with_streaming(Some(0), Some(5))
```

### Right: Pair streaming failure config with `with_streaming`

```rust
use llmposter::{FailureConfig, Fixture};

Fixture::new()
    .respond_with_content("will be truncated")
    .with_streaming(Some(0), Some(5))
    .with_failure(FailureConfig {
        truncate_after_frames: Some(2),
        ..FailureConfig::default()
    });
```

When `truncate_after_frames`, `disconnect_after_ms`, `duplicate_frames`, or `latency_jitter_ms` are specified on a non-streaming response, the configuration is silently ignored and has no effect on the response. Always call `.with_streaming()` before using streaming-related failure modes.

### Wrong: General fixture placed before specific fixture

```rust
use llmposter::{Fixture, ServerBuilder};

ServerBuilder::new()
    .fixture(Fixture::new().respond_with_content("generic"))         // matches everything
    .fixture(Fixture::new().match_user_message("error case").with_error(500, "boom"));
```

### Right: Specific patterns first, catch-all last (or use priority/catch-all)

```rust
use llmposter::{Fixture, ServerBuilder};

// Option 1: Registration order (when all fixtures use default priority)
ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("error case").with_error(500, "boom"))
    .fixture(Fixture::new().respond_with_content("generic fallback"));

// Option 2: Priority + catch-all (preferred — order-independent)
ServerBuilder::new()
    .fixture(
        Fixture::new()
            .match_user_message("error case")
            .with_error(500, "boom")
            .with_priority(10),
    )
    .fixture(
        Fixture::new()
            .as_catch_all()
            .respond_with_content("generic fallback"),
    );
```

A fixture with no match constraints and no `as_catch_all()` matches all requests in the first pass. Use `as_catch_all()` for explicit fallback behavior — it defers to the second matching pass.

### Wrong: HTTP error status code outside 400–599

```rust
use llmposter::Fixture;

Fixture::new().with_error(200, "not actually an error");  // rejected
Fixture::new().with_error(302, "redirect");               // rejected
```

### Right: Use status codes 400–599 only

```rust
use llmposter::Fixture;

Fixture::new()
    .match_user_message("rate limit")
    .with_error(429, "Rate limit exceeded");
```

Codes outside 400–599 are rejected at fixture validation.

### Wrong: Anthropic request missing `max_tokens`

```rust
// Anthropic endpoint requires max_tokens — omitting it returns HTTP 400
serde_json::json!({
    "model": "claude-sonnet-4-6",
    "messages": [{"role": "user", "content": "hello"}]
    // Missing: "max_tokens": 1024
});
```

### Right: Always include `max_tokens` for Anthropic requests

```rust
serde_json::json!({
    "model": "claude-sonnet-4-6",
    "max_tokens": 1024,   // required — must be a positive integer
    "messages": [{"role": "user", "content": "hello"}]
});
```

**Why:** The Anthropic `/v1/messages` endpoint requires `max_tokens` as a positive integer. Omitting it returns HTTP 400. OpenAI and Responses API endpoints do not have this requirement.

### Wrong: Blank or whitespace-only user message (any provider)

```rust
// ALL four providers reject blank/whitespace-only user messages with HTTP 400
// Anthropic
serde_json::json!({
    "model": "claude-sonnet-4-6",
    "max_tokens": 1024,
    "messages": [{"role": "user", "content": "   "}]  // whitespace only — rejected
});

// OpenAI
serde_json::json!({
    "model": "gpt-4",
    "messages": [{"role": "user", "content": ""}]     // empty — rejected
});
```

### Right: Ensure user message has substantive text

```rust
serde_json::json!({
    "model": "claude-sonnet-4-6",
    "max_tokens": 1024,
    "messages": [{"role": "user", "content": "actual question here"}]
});
```

**Why:** All four providers (Anthropic, OpenAI, Gemini, Responses) trim and reject blank content with HTTP 400.

### Wrong: Non-boolean stream field

```rust
// String instead of boolean — rejected with 400
serde_json::json!({
    "model": "claude-sonnet-4-6",
    "max_tokens": 1024,
    "stream": "true",   // string — rejected
    "messages": [{"role": "user", "content": "hello"}]
});
```

### Right: Use JSON boolean for stream field

```rust
serde_json::json!({
    "model": "claude-sonnet-4-6",
    "max_tokens": 1024,
    "stream": true,     // boolean — correct
    "messages": [{"role": "user", "content": "hello"}]
});
```

**Why:** v0.4.1+ rejects non-boolean stream values with HTTP 400 to catch client SDK bugs that accidentally serialize stream as a string or number.

### Wrong: Refusal fixture with streaming

```rust
use llmposter::Fixture;

Fixture::new()
    .match_user_message("harmful")
    .respond_with_refusal("I cannot help with that.");
// Even without .with_streaming(), a client sending stream: true will get HTTP 400
```

### Right: Only use refusal for non-streaming requests

```rust
use llmposter::Fixture;

// Refusal is mutually exclusive with streaming, error, failure, and content response
Fixture::new()
    .match_user_message("harmful")
    .respond_with_refusal("I cannot help with that.");
// Ensure clients send stream: false (or omit stream) when hitting refusal fixtures
```

**Why:** `respond_with_refusal` is mutually exclusive with `respond_with_content`, `with_error`, `with_failure`, and `with_streaming` at validation time. A matched refusal fixture against a `stream: true` request returns HTTP 400 because streaming refusal envelopes are not yet implemented.

### Wrong: Streaming jitter without base latency

```yaml
# YAML fixture example
streaming:
  chunk_size: 5
  # latency: 0  (or omitted)
failure:
  latency_jitter_ms: 10  # has nothing to modify without base latency
```

### Right: Pair jitter with non-zero streaming latency

```yaml
streaming:
  latency: 20
  chunk_size: 5
failure:
  latency_jitter_ms: 10
```

**Why:** `latency_jitter_ms` adds random jitter to the per-frame streaming delay but requires a non-zero `streaming.latency` to act on.

### Wrong: `duplicate_frames` with incorrect `truncate_after_frames` count

```yaml
failure:
  duplicate_frames: true
  truncate_after_frames: 5  # expecting 5 original frames
```

### Right: Account for duplication when setting truncation count

```yaml
failure:
  duplicate_frames: true
  truncate_after_frames: 10  # 10 doubled frames = 5 original frames
```

**Why:** Duplication runs before truncation. With `duplicate_frames: true`, `truncate_after_frames: N` cuts after N doubled frames (i.e. N/2 original frames if N is even).

## Migration Guide

### v0.4.5 → v0.4.6

#### Header match case-folding (behavioral)

**What changed:** Header match keys are lowercased once at fixture load time, and post-fold duplicate keys are now rejected. Previously, case-variant keys (e.g. `X-Foo` and `x-foo`) could coexist as distinct entries.

**Migration:** Ensure fixture header match keys are unique after case-folding. Remove duplicate header entries that differ only in case.

#### F64Match::Exact now uses plain f64 equality (behavioral)

**What changed:** `match_temperature` with an exact value now uses plain f64 equality instead of epsilon-based comparison. Temperature matching of `0.7` will no longer match `0.7000000001`.

**Migration:** If you relied on epsilon-tolerance matching, switch to `match_temperature_range` with explicit min/max bounds for your desired tolerance:

```rust
use llmposter::Fixture;

// Before: relied on epsilon tolerance
// Fixture::new().match_temperature(0.7)

// After: explicit range for tolerance
Fixture::new().match_temperature_range(Some(0.69), Some(0.71));
```

#### New match fields (additive — no migration needed)

Six new match fields added: `match_header`, `match_system_prompt`, `match_temperature`, `match_temperature_range`, `match_metadata`, `match_tool_schema`, `match_body_jsonpath`. All are optional and stack with existing fields via AND. Existing fixtures continue to work unchanged.

#### New priority/catch-all system (additive — no migration needed)

`with_priority(i32)` and `as_catch_all()` are new optional methods. Without them, behavior is identical to v0.4.5 (first-match-wins registration order).

### v0.4.2 → v0.4.3

#### Streaming tool-call IDs now globally unique

**What changed:** Tool-call IDs in streaming responses are now globally unique across all requests on a server, matching the behavior of non-streaming responses.

**Migration:** If your tests assert on tool-call IDs, use `starts_with` or `contains("llmposter_")` rather than exact ID comparisons — the counter value depends on prior requests in the session.

#### `disconnect_after_ms` now simulates real transport failure

**What changed:** `disconnect_after_ms` now injects a `ConnectionReset` I/O error into the SSE stream instead of closing it cleanly.

**Migration:** Tests using `disconnect_after_ms` that call `.unwrap()` on `resp.text().await` should use `.unwrap_or_default()` or a match pattern:

```rust
match resp.text().await {
    Ok(body) => { /* partial content received before disconnect */ }
    Err(_) => { /* ConnectionReset propagated to client */ }
}
```

### v0.4.1 → v0.4.2

#### 404 no-match error redacted

**What changed:** When a request matches no fixture, the 404 response body no longer includes the user prompt text. Previously, the error response echoed back the user message.

**Migration:** Tests that parse 404 response bodies to verify the prompt text must be updated. The response body is now a provider-specific error shape with a redacted message (model name only, no user input).

#### Responses API streaming now includes `incomplete_details`

**What changed:** When using the Responses API with streaming enabled, responses with status `incomplete` now include the `incomplete_details` object with a `reason` field.

**Migration:** If your tests branch on `stop_reason` in streaming Responses API responses, update them to also check `incomplete_details.reason` as needed.

### v0.3.x → v0.4.0

- MSRV bumped to 1.89 (required by oauth-mock dependency).
- Auth is off by default — existing code works without changes.
- Add `with_bearer_token()` or `with_oauth_defaults()` to enable auth.
- OAuth feature is on by default; disable with `default-features = false` for smaller binary.

### v0.1.0 → v0.2.0

- `truncate_after_chunks` renamed to `truncate_after_frames` (serde alias preserves backward compat in YAML).
- 404 responses now use provider-specific error formats — update test assertions if checking error body shape.

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)
- [CHANGELOG](https://github.com/SkillDoAI/llmposter/blob/main/CHANGELOG.md)
- [API Reference](https://docs.rs/llmposter/latest/llmposter/)
