---
name: llmposter
description: Mock HTTP server for LLM provider APIs (OpenAI, Anthropic, Gemini, Responses API). Use when writing integration tests that need deterministic, controllable LLM API responses without calling real providers. Supports fixture-based request matching, SSE streaming, failure injection, auth simulation, VCR record/replay modes, and all four provider response formats.
license: AGPL-3.0-or-later
metadata:
  author: SkillDoAI
  version: "0.5.0"
  ecosystem: rust
  generated-by: skilldo/claude-haiku-4-5-20251001 + review:claude-haiku-4-5-20251001
---

# llmposter

Mock HTTP server for LLM provider APIs. Clients point their base URL at llmposter and interact using real API paths — no code changes beyond the URL swap. Fixtures define request matchers and canned responses for Anthropic (`POST /v1/messages`), OpenAI (`POST /v1/chat/completions`, `POST /v1/completions`), Gemini (non-streaming `POST /v1beta/models/{model}:generateContent`, streaming `POST /v1beta/models/{model}:streamGenerateContent`), and Responses API (`POST /v1/responses`). Response format automatically matches provider (no prefixing). New in 0.5.0: VCR record/replay modes for recording live upstream responses and deterministic replay; new endpoints `GET /health`, `GET /v1/models`, `POST /v1/embeddings`, `POST /v1/moderations`.

## Imports

```rust
// Core types (re-exported at crate root)
use llmposter::{Fixture, Provider, ServerBuilder, MockServer, CapturedRequest};

// Fixture sub-types
use llmposter::fixture::FailureConfig;

// VCR types (0.5.0+, gated by "record" feature)
use llmposter::record::VcrMode;
```

```toml
[dependencies]
llmposter = "0.5.0"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.13", default-features = false, features = ["json"] }
serde_json = "1"

# Optional: Disable default features to minimize dependencies
# llmposter = { version = "0.5.0", default-features = false, features = ["watch"] }

# Optional: UI debug interface
# llmposter = { version = "0.5.0", features = ["ui"] }
```

**Default features:** `oauth`, `watch`, `jsonpath`, `record`  
**Optional features:** `ui`, `templating`

## Core Patterns

### Quick Start

```rust
mod quick_start {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_basic_mock_server() -> Result<(), Box<dyn std::error::Error>> {
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

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["type"], "message");
        assert_eq!(body["content"][0]["text"], "Hi from Claude mock!");
        assert_eq!(body["stop_reason"], "end_turn");
        assert!(body["id"].as_str().unwrap().starts_with("msg-llmposter-"));
        assert!(body["usage"]["input_tokens"].as_u64().unwrap_or(0) > 0);
        Ok(())
    }
}
```

### Tool Use Response

```rust
mod tool_use_example {
    use llmposter::fixture::ToolCall;
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_tool_use_response() -> Result<(), Box<dyn std::error::Error>> {
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
                "messages": [{"role": "user", "content": "What's the weather?"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["name"], "get_weather");
        let tool_id = body["content"][0]["id"].as_str().unwrap();
        assert!(tool_id.starts_with("toolu_llmposter_"));
        assert_eq!(body["content"][0]["input"]["location"], "London");
        Ok(())
    }
}
```

### SSE Streaming

```rust
mod streaming_example {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_streaming_response() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("hello")
                    .respond_with_content("Hello world")
                    .with_streaming(Some(0), Some(5)), // latency=0ms, chunk_size=5 chars
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
        let text = resp.text().await?;
        assert!(text.contains("event: message_start"));
        assert!(text.contains("event: content_block_delta"));
        assert!(text.contains("event: message_stop"));
        Ok(())
    }
}
```

### Error Simulation

```rust
mod error_simulation {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_rate_limit_error() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
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
                "messages": [{"role": "user", "content": "rate limit test"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 429);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["error"]["message"], "Rate limit exceeded");
        Ok(())
    }
}
```

### Failure Injection (Latency)

```rust
mod latency_injection {
    use llmposter::fixture::FailureConfig;
    use llmposter::{Fixture, ServerBuilder};
    use std::time::Instant;

    #[tokio::test]
    async fn test_latency_injection() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("delayed response")
                    .with_failure(FailureConfig {
                        latency_ms: Some(200),
                        ..Default::default()
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
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        assert!(start.elapsed().as_millis() >= 180);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["content"][0]["text"], "delayed response");
        Ok(())
    }
}
```

### Corrupt Body (Overloaded Simulation)

```rust
mod corrupt_body_example {
    use llmposter::fixture::FailureConfig;
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_corrupt_body() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("should not appear")
                    .with_failure(FailureConfig {
                        corrupt_body: Some(true),
                        ..Default::default()
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
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        let text = resp.text().await?;
        assert_eq!(text, "overloaded");
        Ok(())
    }
}
```

### Provider Filtering

```rust
mod provider_filtering {
    use llmposter::{Fixture, Provider, ServerBuilder};

    #[tokio::test]
    async fn test_provider_filters() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .for_provider(Provider::OpenAI)
                    .respond_with_content("openai only"),
            )
            .fixture(
                Fixture::new()
                    .for_provider(Provider::Anthropic)
                    .respond_with_content("anthropic only"),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();

        // OpenAI route should match OpenAI fixture
        let resp = client
            .post(format!("{}/v1/chat/completions", server.url()))
            .json(&serde_json::json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);

        // Anthropic route should match Anthropic fixture
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["content"][0]["text"], "anthropic only");
        Ok(())
    }
}
```

### Model Matching (Substring)

```rust
mod model_matching {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_model_substring_matching() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_model("claude-sonnet")
                    .respond_with_content("sonnet response"),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();

        // Matches: "claude-sonnet" is substring of "claude-sonnet-4-6"
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);

        // Does not match: "claude-haiku" does not contain "claude-sonnet"
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-haiku-3",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 404);
        Ok(())
    }
}
```

### Stream Truncation

```rust
mod stream_truncation {
    use llmposter::fixture::FailureConfig;
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_stream_truncation() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("long text that gets truncated")
                    .with_streaming(Some(0), Some(5))
                    .with_failure(FailureConfig {
                        truncate_after_frames: Some(2),
                        ..Default::default()
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
                "messages": [{"role": "user", "content": "test"}],
                "stream": true
            }))
            .send()
            .await?;

        let text = resp.text().await?;
        assert!(text.contains("event: message_start"));
        assert!(!text.contains("event: message_stop"));
        Ok(())
    }
}
```

### Bearer Token Authentication

```rust
mod bearer_auth {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_bearer_token_auth() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .with_bearer_token_uses("sk-test-token", 3)
            .fixture(Fixture::new().respond_with_content("authorized"))
            .build()
            .await?;

        let client = reqwest::Client::new();

        // Request without token: 401
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

        // Request with token: 200
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .bearer_auth("sk-test-token")
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);

        // Health endpoint requires no auth
        let health = client.get(format!("{}/health", server.url())).send().await?;
        assert_eq!(health.status(), 200);
        Ok(())
    }
}
```

### YAML Fixtures

```rust
mod yaml_fixtures {
    use llmposter::ServerBuilder;
    use std::path::Path;

    #[tokio::test]
    async fn test_yaml_fixtures() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .load_yaml(Path::new("fixtures/anthropic.yaml"))?
            .build()
            .await?;

        println!("Mock server at {} with fixtures loaded", server.url());
        Ok(())
    }
}
```

**YAML fixture format example:**

```yaml
fixtures:
  - match:
      user_message: "hello"
    response:
      content: "Hi from the mock!"

  - match:
      model: "claude-sonnet"
      user_message: "weather"
    response:
      tool_calls:
        - name: get_weather
          arguments:
            location: London
            unit: celsius

  - match:
      user_message: "fail"
    error:
      status: 429
      message: "Rate limit exceeded"

  - match:
      user_message: "slow"
    response:
      content: "delayed"
    streaming:
      latency: 50
      chunk_size: 5
    failure:
      latency_ms: 500
```

The top-level `fixtures:` key is required — a bare list will fail to load.

### Request Capture and Assertion

```rust
mod request_capture {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_request_capture() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("captured"))
            .capture_capacity(100)
            .build()
            .await?;

        let client = reqwest::Client::new();
        client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "ping the mock"}]
            }))
            .send()
            .await?;

        let captured = server.get_requests();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].method, "POST");
        assert_eq!(captured[0].path, "/v1/messages");
        assert!(captured[0].was_matched());
        Ok(())
    }
}
```

### VCR Record Mode (0.5.0+)

```rust
mod vcr_record_example {
    use llmposter::record::VcrMode;
    use llmposter::{Fixture, ServerBuilder};
    use std::path::Path;

    #[tokio::test]
    async fn test_vcr_record_mode() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .vcr_mode(VcrMode::Record)
            .proxy_anthropic("https://api.anthropic.com")
            .record_file(Path::new("recorded.yaml"))
            .redact("sk-ant-.*")
            .fixture(Fixture::new().respond_with_content("fallback"))
            .build()
            .await?;

        println!("Mock server at {}", server.url());
        println!("Recording to: {:?}", server.recorded_cassette_path());
        Ok(())
    }
}
```

### VCR Record-On-Miss Mode (0.5.0+)

```rust
mod vcr_record_on_miss_example {
    use llmposter::record::VcrMode;
    use llmposter::{Fixture, ServerBuilder};
    use std::path::Path;

    #[tokio::test]
    async fn test_vcr_record_on_miss() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .vcr_mode(VcrMode::RecordOnMiss)
            .proxy_openai("https://api.openai.com")
            .record_file(Path::new("recorded.yaml"))
            .fixture(
                Fixture::new()
                    .match_user_message("predefined")
                    .respond_with_content("from local fixture"),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();

        // Matches local fixture — served locally
        let resp = client
            .post(format!("{}/v1/chat/completions", server.url()))
            .json(&serde_json::json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": "predefined"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);

        // Does not match — forwarded upstream and recorded
        let resp = client
            .post(format!("{}/v1/chat/completions", server.url()))
            .json(&serde_json::json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": "new prompt"}]
            }))
            .send()
            .await?;
        println!("Response status: {}", resp.status());
        Ok(())
    }
}
```

## Pitfalls

### Wrong: Streaming config field name in direct construction

When constructing `StreamingConfig` directly, the field is named `latency`, not `latency_ms`:

```rust
// ❌ Will not compile
let config = StreamingConfig {
    latency_ms: Some(100),
    chunk_size: Some(5),
};
```

### Right: Use the correct struct field name

```rust
// ✅ Correct direct construction
let config = StreamingConfig {
    latency: Some(100),
    chunk_size: Some(5),
};

// ✅ Or use the builder method (which accepts latency_ms parameter)
Fixture::new().with_streaming(Some(100), Some(5))
```

### Wrong: Assuming chunk_size applies to tool-call streaming

`chunk_size` is silently ignored for tool-call streaming across all providers:

```rust
// ❌ chunk_size will be ignored for tool calls
Fixture::new()
    .respond_with_tool_calls(vec![ToolCall { 
        name: "get_weather".to_string(),
        arguments: serde_json::json!({})
    }])
    .with_streaming(Some(0), Some(5))  // chunk_size=5 is ignored
```

### Right: chunk_size only affects text-content streaming

```rust
// ✅ Text content streaming respects chunk_size
Fixture::new()
    .respond_with_content("hello world")
    .with_streaming(Some(0), Some(5))  // chunk_size applies

// ✅ Tool calls stream without chunking regardless of chunk_size
Fixture::new()
    .respond_with_tool_calls(vec![ToolCall {
        name: "get_weather".to_string(),
        arguments: serde_json::json!({})
    }])
    .with_streaming(Some(50), Some(5))  // chunk_size ignored for tools
```

### Wrong: Capture log status under chaos injection

`CapturedRequest.status_code` shows the pre-chaos value, not the chaos-injected status:

```rust
// ❌ Capture shows 200 even if chaos injects 500
let capture = server.get_requests()[0];
assert_eq!(capture.status_code, 200);  // Pre-chaos value!
```

### Right: Verify chaos failures via the HTTP response

```rust
// ✅ Check the actual HTTP response for chaos-injected status
let resp = client.post(format!("{}/v1/messages", server.url())).send().await?;
assert_eq!(resp.status(), 500);  // Chaos-injected value
```

### Wrong: Bearer tokens protect all endpoints

Bearer tokens only protect LLM routes:

```rust
// ❌ These endpoints are NEVER auth-protected
// GET /health (no auth)
// GET /code/{N} (no auth)
// GET /v1/models (no auth)
// POST /v1/embeddings (no auth)
// POST /v1/moderations (no auth)
// GET /ui (no auth)
```

### Right: Auth only protects LLM routes

```rust
// ✅ These routes ARE auth-protected
// POST /v1/messages (requires bearer token if configured)
// POST /v1/chat/completions (requires bearer token if configured)
// POST /v1beta/models/{model}:generateContent (requires bearer token if configured)
// POST /v1/responses (requires bearer token if configured)

// Health/utility endpoints are always open
```

### Wrong: VCR record mode with embedded credentials

Proxy URLs with embedded credentials are rejected at build time:

```rust
// ❌ Rejected — credentials should not be in URL
ServerBuilder::new()
    .vcr_mode(VcrMode::Record)
    .proxy_anthropic("https://user:password@api.anthropic.com")
    .build()
    .await  // ERROR: credentials in proxy URL rejected
```

### Right: Use environment variables or separate auth

```rust
// ✅ Use environment variables or pass auth separately
std::env::set_var("ANTHROPIC_API_KEY", "sk-...");
ServerBuilder::new()
    .vcr_mode(VcrMode::Record)
    .proxy_anthropic("https://api.anthropic.com")
    .build()
    .await?
```

### Wrong: VCR record mode from non-loopback address

Record mode enforces loopback-only bind by default to prevent key leakage:

```rust
// ❌ Rejected in record mode with non-loopback bind
ServerBuilder::new()
    .vcr_mode(VcrMode::Record)
    .proxy_anthropic("https://api.anthropic.com")
    .bind("0.0.0.0:8080")  // ERROR: not loopback
    .build()
    .await
```

### Right: Use allow_remote_record only in trusted environments

```rust
// ✅ Loopback bind (default, safe)
ServerBuilder::new()
    .vcr_mode(VcrMode::Record)
    .proxy_anthropic("https://api.anthropic.com")
    .bind("127.0.0.1:8080")
    .build()
    .await?

// ✅ Non-loopback only with explicit opt-in (trusted env only)
ServerBuilder::new()
    .vcr_mode(VcrMode::Record)
    .proxy_anthropic("https://api.anthropic.com")
    .bind("0.0.0.0:8080")
    .allow_remote_record(true)  // Security: only in trusted environments
    .build()
    .await?
```

### Wrong: Redacting match keys breaks replay

Redacting patterns that appear in match criteria breaks replay:

```yaml
# ❌ Redacting in match key breaks lookup
fixtures:
  - match:
      user_message: "sk-ant-secret-key"  # Matched on this
    response:
      content: "response"

# If redact pattern matches this, cassette records "[REDACTED]" and replay fails
```

### Right: Redact only sensitive response content

```rust
// ✅ Redact response content, not match criteria
ServerBuilder::new()
    .vcr_mode(VcrMode::Record)
    .proxy_anthropic("https://api.anthropic.com")
    .redact("sk-ant-.*")  // Masks API keys in responses only
    .build()
    .await?
```

### Wrong: Tool arguments as stringified JSON

`ToolCall.arguments` is `serde_json::Value`, not a stringified JSON string:

```rust
// ❌ Wrong: passing a string
ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::Value::String(r#"{"location": "London"}"#.to_string()),
}
```

### Right: Use json! macro for parsed values

```rust
// ✅ Correct: parsed JSON value
ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!({
        "location": "London",
        "unit": "celsius"
    }),
}
```

### Wrong: Assuming moderation endpoint processes input

Moderation endpoint returns static response regardless of input:

```rust
// ❌ Response is always static
let resp = client.post(format!("{}/v1/moderations", server.url()))
    .json(&serde_json::json!({
        "model": "text-moderation-latest",
        "input": "offensive content"
    }))
    .send()
    .await?;

let body: serde_json::Value = resp.json().await?;
assert_eq!(body["results"][0]["flagged"], false);  // Always false
```

### Right: Moderation returns static values

```rust
// ✅ Moderation is static mock with flagged: false
let resp = client.post(format!("{}/v1/moderations", server.url()))
    .json(&serde_json::json!({
        "model": "text-moderation-latest",
        "input": "any content"
    }))
    .send()
    .await?;

let body: serde_json::Value = resp.json().await?;
// Always returns { "flagged": false, "categories": {...with zeros...} }
```

## References

### ServerBuilder API

- `ServerBuilder::new() -> Self` — create an empty builder
- `.fixture(f: Fixture) -> Self` — add a single fixture (chainable)
- `.fixtures(fixtures: Vec<Fixture>) -> Self` — append a vector of fixtures
- `.load_yaml(path: &Path) -> Result<Self, Box<dyn Error>>` — load fixtures from a YAML file
- `.load_yaml_dir(dir: &Path) -> Result<Self, Box<dyn Error>>` — load all YAML files in a directory
- `.fixture_count(&self) -> usize` — number of fixtures currently loaded (immutable borrow)
- `.bind(addr: &str) -> Self` — bind address as single string (e.g., `"127.0.0.1:8080"`). Default: random port on `127.0.0.1`.
- `.verbose(v: bool) -> Self` — when `true`, 404 responses include `"No fixture matched"` diagnostic detail
- `.diagnostics(enabled: bool) -> Self` — when `true`, 404 responses include nearest-match fixture and per-field pass/fail breakdown
- `.capture_capacity(max: usize) -> Self` — max captured requests in ring buffer. `0` disables capture entirely.
- `.with_bearer_token_uses(token: &str, max_uses: u64) -> Self` — registers a bearer token capped at `max_uses` requests AND enables auth enforcement
- `.watch(enabled: bool) -> Self` — enable hot-reload of fixture files (**`watch` feature**, gated by `#[cfg(feature = "watch")]`)
- `.ui(enabled: bool) -> Self` — enable debug UI at `/ui` (**`ui` feature**, gated by `#[cfg(feature = "ui")]`)
- `.vcr_mode(mode: VcrMode) -> Self` — set VCR mode: `Replay`, `Record`, or `RecordOnMiss` (**`record` feature**, gated by `#[cfg(feature = "record")]`)
- `.record_file(path: &Path) -> Self` — cassette file path for recording (default: `recorded.yaml`) (**`record` feature**, gated by `#[cfg(feature = "record")]`)
- `.allow_remote_record(allowed: bool) -> Self` — when `true`, allow non-loopback bind addresses in record mode (**`record` feature**, gated by `#[cfg(feature = "record")]`)
- `.proxy_openai(url: &str) -> Self` — upstream URL for OpenAI routes (**`record` feature**, gated by `#[cfg(feature = "record")]`)
- `.proxy_anthropic(url: &str) -> Self` — upstream URL for Anthropic route (**`record` feature**, gated by `#[cfg(feature = "record")]`)
- `.proxy_gemini(url: &str) -> Self` — upstream URL for Gemini routes (**`record` feature**, gated by `#[cfg(feature = "record")]`)
- `.redact(pattern: &str) -> Self` — regex pattern to mask as `[REDACTED]` in recorded responses (**`record` feature**, gated by `#[cfg(feature = "record")]`)
- `.build(self) -> Result<MockServer, Box<dyn Error>>` — **async**. Validates fixtures and starts the server

### MockServer API

- `.url(&self) -> String` — base URL (e.g., `http://127.0.0.1:PORT`)
- `.get_requests(&self) -> Vec<CapturedRequest>` — all captured requests in chronological order (owned vector)
- `.recorded_cassette_path(&self) -> Option<&Path>` — path to the recorded cassette file if VCR record mode is active (**`record` feature**, gated by `#[cfg(feature = "record")]`, new in 0.5.0)

### Fixture API

All builder methods are chainable and return `Self`.

**Constructor:**
- `Fixture::new() -> Self`

**Match methods (all use substring matching):**
- `.match_user_message(pattern: &str)` — substring match on the last user message
- `.match_model(pattern: &str)` — substring match on the model field

**Response methods:**
- `.respond_with_content(content: &str)` — text response
- `.respond_with_tool_calls(tool_calls: Vec<ToolCall>)` — tool-use response

**Failure & streaming:**
- `.with_streaming(latency_ms: Option<u64>, chunk_size: Option<usize>) -> Self` — enable SSE streaming. Both parameters optional (`None` disables that aspect).
- `.with_failure(failure: FailureConfig) -> Self` — inject failure behaviors

**Error response:**
- `.with_error(status: u16, message: &str)` — HTTP error response with provider-specific error envelope

**Provider filtering:**
- `.for_provider(provider: Provider)` — restrict to one provider endpoint (exact provider match, not substring)

### Endpoint Reference

| Provider | Endpoint | Notes |
|----------|----------|-------|
| Anthropic | `POST /v1/messages` | Requires `max_tokens` in request body |
| OpenAI | `POST /v1/chat/completions` | Standard chat completion |
| OpenAI legacy | `POST /v1/completions` | Text completion (new in 0.5.0) |
| Gemini (non-streaming) | `POST /v1beta/models/{model}:generateContent` | Model name in URL path |
| Gemini (streaming) | `POST /v1beta/models/{model}:streamGenerateContent` | Use `?alt=sse` for true SSE |
| Responses | `POST /v1/responses` | OpenAI Responses API |
| Embeddings | `POST /v1/embeddings` | New in 0.5.0. Default: 1536-dim |
| Moderations | `POST /v1/moderations` | New in 0.5.0. Static `flagged: false` |
| Models list | `GET /v1/models` | New in 0.5.0. OpenAI-compatible |
| Health | `GET /health` | New in 0.5.0. Returns `{"status": "ok"}` |
| Status echo | `GET /code/{status}` | New in 0.5.0. Returns specified HTTP status (100–599) |

### Behavioral Semantics

- **Matching order**: First-match-wins. Fixtures checked in priority order (highest first), with catch-all fixtures checked last.
- **Match fields stack conjunctively**: All conditions must match for a fixture to serve.
- **Substring matching**: `match_user_message("hello")` matches request message containing "hello" exactly (case-sensitive).
- **Provider matching is exact**: `for_provider(Provider::Anthropic)` matches only requests to Anthropic endpoints. Mismatched provider → 404.
- **Bearer token validation is case-sensitive and exact**: `Bearer sk-123` matches only token `sk-123`; comparison is character-for-character.
- **Response IDs deterministic**: `msg-llmposter-{N}` (Anthropic), `chatcmpl-llmposter-{N}` (OpenAI).
- **Tool-call IDs deterministic**: Format `toolu_llmposter_{N}` (1-indexed, sequential within run). Same fixture invoked twice produces identical IDs.
- **Token counts heuristic**: `bytes / 4` calculation. Never assert exact values, only `> 0`.
- **Anthropic `stop_reason` defaults**: `"end_turn"` for text, `"tool_use"` for tool calls.
- **`max_tokens` required for Anthropic**: Missing returns HTTP 400 validation error.
- **Auth scope**: LLM routes only (Anthropic, OpenAI, Gemini, Responses). `/health`, `/code/{N}`, `/v1/models`, `/v1/embeddings`, `/v1/moderations`, `/ui` never protected.
- **`corrupt_body` value**: Always returns exact string `"overloaded"` as `text/plain` with HTTP 200, regardless of configured response.
- **`chunk_size` ignored for tool-call streaming**: Only applies to text-content streaming.
- **Latency timing**: Applied between chunks in streaming (each chunk delayed by `latency_ms`). In non-streaming, applied before response.
- **VCR replay mode** (default): Only serves fixtures, never contacts upstream.
- **VCR record mode**: All requests forwarded upstream; 2xx responses recorded with `priority: -1` so hand-written fixtures always match first.
- **VCR record-on-miss mode**: Local fixtures served if matched; unmatched requests forwarded upstream and recorded.
- **Cassette deduplication**: Re-recording same prompt is idempotent (no duplicate entries).
- **Streaming response recording**: Complete streams recorded; truncated/errored streams not recorded.
- **Unmatched request**: Returns HTTP 404 with error body (not 400 or silent 200).
- **Invalid JSON body**: Returns HTTP 400.
- **Missing required field**: Returns HTTP 400.

## Current Library State (v0.5.0)

**Stability:** Production-ready. New VCR modes and endpoints in 0.5.0 are fully tested and stabilized.

**Breaking changes from 0.4.x:** None to existing API surface. Additions only:
- `ServerBuilder::vcr_mode()`, `proxy_openai()`, etc. (new methods)
- `VcrMode` enum and VCR-related types (new, feature-gated)
- `GET /v1/models`, `GET /health`, `POST /v1/embeddings`, `POST /v1/moderations` (new endpoints)
- `MockServer::recorded_cassette_path()` (new method, feature-gated)

**Feature flags:**

| Feature | Default | Description |
|---------|---------|-------------|
| `oauth` | **on** | OAuth mock support |
| `watch` | **on** | Hot-reload via file watcher |
| `jsonpath` | **on** | `body_jsonpath` match field |
| `record` | **on** | VCR record/replay modes |
| `ui` | off | Debug UI at `/ui` |
| `templating` | off | minijinja response template rendering |

To minimize dependencies, use:
```toml
llmposter = { version = "0.5.0", default-features = false, features = ["watch"] }
```

## Migration from 0.4.x

No code changes required for existing tests. New features in 0.5.0 are opt-in:

1. **VCR record/replay** — Call `ServerBuilder::vcr_mode(VcrMode::Record)` and configure proxy URL.
2. **New endpoints** — `/health`, `/v1/models`, `/v1/embeddings`, `/v1/moderations` available automatically.
3. **Request capture** — Use `server.get_requests()` to access captured requests and assertions.
4. **Cassette files** — Recorded fixtures stored in YAML with `priority: -1` for deterministic replay.

Existing fixture-only tests compile and run unchanged.