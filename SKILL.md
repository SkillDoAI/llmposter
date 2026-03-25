---
name: llmposter
description: rust library
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---

---
name: llmposter
description: A mock LLM API server for integration testing that simulates OpenAI, Anthropic, and Gemini endpoints with configurable response fixtures, SSE streaming, failure injection, and bearer token or OAuth authentication.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
---

## Imports

```rust
use llmposter::{Fixture, MockServer, Provider, ServerBuilder};
use llmposter::fixture::{FailureConfig, FixtureResponse, StreamingConfig, ToolCall};
// Feature-gated — requires `oauth` Cargo feature (enabled by default):
// use llmposter::OAuthConfig;
```

Add to `Cargo.toml`:

```toml
[dependencies]
llmposter = { version = "0.4.0" }
# With OAuth support (enabled by default; set default-features = false to opt out):
# llmposter = { version = "0.4.0", features = ["oauth"] }
reqwest = { default-features = false, features = ["json"], version = "0.13" }
serde_json = "1"
tokio = { features = ["full"], version = "1" }
```

## Core Patterns

### Basic Content Response ✅ Current

Start a mock server with a single fixture that matches on user message content and returns a static response. The server runs until the `MockServer` value is dropped. `build()` validates all fixtures before binding — invalid fixtures return `Err` before any port is opened.

```rust
mod basic_content_response {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_anthropic_content_response() -> Result<(), Box<dyn std::error::Error>> {
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
                "messages": [{"role": "user", "content": "say hello to me"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status().as_u16(), 200);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["type"], "message");
        assert_eq!(body["content"][0]["text"], "Hi from mock!");
        assert_eq!(body["stop_reason"], "end_turn");
        // Body 'id' field uses msg-llmposter-{N} prefix — assert prefix, not exact value
        assert!(body["id"].as_str().unwrap().starts_with("msg-llmposter-"));
        Ok(())
    }
}
```

### SSE Streaming Response ✅ Current

`with_streaming(latency, chunk_size)` switches the response to SSE. `latency` is the delay between frames in milliseconds; `chunk_size` is characters per delta event.

```rust
mod streaming_text_response {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_streaming_content() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("stream this")
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
                "messages": [{"role": "user", "content": "stream this please"}],
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
}
```

### Tool Call Response ✅ Current

`respond_with_tool_calls` returns a `tool_use` block. Arguments must be a JSON object — not an array or scalar. `build()` rejects non-object arguments at load time. Tool call names must be non-empty — `build()` rejects blank names at fixture validation time.

```rust
mod tool_call_response {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::ToolCall;

    #[tokio::test]
    async fn test_tool_call_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("weather in London")
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
                "messages": [{"role": "user", "content": "what's the weather in London?"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status().as_u16(), 200);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["name"], "get_weather");
        // Tool-call IDs use a server-wide counter — assert prefix, not exact value
        assert!(body["content"][0]["id"].as_str().unwrap().starts_with("toolu_llmposter_"));
        Ok(())
    }
}
```

### Failure Injection ✅ Current

Use `with_error` for HTTP error responses (status code must be in the 400–599 range — values outside this range are rejected at fixture validation time). Use `with_failure` for network-level failures on otherwise valid responses. These are mutually exclusive. `with_failure` requires a response to also be set.

```rust
mod failure_injection_patterns {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::FailureConfig;

    #[tokio::test]
    async fn test_http_error_response() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("rate limit me")
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
                "messages": [{"role": "user", "content": "rate limit me"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status().as_u16(), 429);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["error"]["message"], "Rate limit exceeded");
        Ok(())
    }

    #[tokio::test]
    async fn test_streaming_truncation() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("A very long response that will be cut short")
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
                "messages": [{"role": "user", "content": "give me a long answer"}],
                "stream": true
            }))
            .send()
            .await?;

        assert_eq!(resp.status().as_u16(), 200);
        let body = resp.text().await?;
        assert!(body.contains("event: message_start"));
        assert!(!body.contains("event: message_stop")); // stream was truncated
        Ok(())
    }
}
```

### Bearer Token Authentication ✅ Current

`with_bearer_token` enables auth; all LLM requests must include `Authorization: Bearer <token>`. `with_bearer_token_uses` creates a token that expires after N uses; the (N+1)th request with that token returns 401 with a provider-specific error body.

```rust
mod bearer_token_auth {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_auth_enforcement() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .with_bearer_token("test-token-abc123")
            .fixture(Fixture::new().respond_with_content("authenticated"))
            .build()
            .await?;

        let client = reqwest::Client::new();
        let base = server.url();
        let payload = serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}]
        });

        // Valid token → 200
        let resp = client
            .post(format!("{}/v1/messages", base))
            .header("Authorization", "Bearer test-token-abc123")
            .json(&payload)
            .send()
            .await?;
        assert_eq!(resp.status().as_u16(), 200);

        // Missing token → 401
        let resp = client
            .post(format!("{}/v1/messages", base))
            .json(&payload)
            .send()
            .await?;
        assert_eq!(resp.status().as_u16(), 401);
        Ok(())
    }
}
```

## Configuration

**Bind address**: Default is `127.0.0.1` with an OS-assigned port. Override with `.bind("0.0.0.0")`. The `--bind` CLI flag takes an IP address only — not `host:port`. Port is separate (`--port`, default `2112`).

**Port**: OS-assigned in `ServerBuilder`; retrieve the actual address via `server.url()`. The CLI binary defaults to port `2112`.

**Fixture ordering**: First-match-wins. Register specific patterns before broad ones.

**YAML fixtures**: Load with `ServerBuilder::load_yaml(path)` or `ServerBuilder::load_yaml_dir(dir)`. All structs use `#[serde(deny_unknown_fields)]` — a typo causes a startup error rather than silent misconfiguration.

```yaml
# fixtures/responses.yaml
- match:
    user_message: "stock price of AAPL"  # specific pattern first
  response:
    content: "$150.42"

- match:
    user_message: "stock"              # broad pattern after specific
  response:
    content: "I can look up stock prices."

- match:
    model: "claude-sonnet"             # substring match on model name
    user_message:
      regex: "^summarize"              # regex match (use { regex: "..." } syntax)
  response:
    content: "Here is a summary."
    stop_reason: "end_turn"            # override for Anthropic/Gemini
```

**Regex DFA size cap**: Patterns that produce a DFA exceeding 1 MB are rejected at load time to prevent OOM. Avoid catastrophically backtracking or unbounded alternation patterns; `load_yaml()` returns `Err` for oversized DFA patterns.

**FailureConfig fields** (all optional, default `None`):

| Field | Type | Effect |
|---|---|---|
| `latency_ms` | `Option<u64>` | Delay before sending the response |
| `corrupt_body` | `Option<bool>` | Replaces body with plain-text `"overloaded"` (status still 200) |
| `truncate_after_frames` | `Option<u32>` | Cuts SSE stream after N frames (YAML alias `truncate_after_chunks` is deprecated) |
| `disconnect_after_ms` | `Option<u64>` | Drops the connection after N milliseconds |

**Custom stop/finish reason** via struct literal:

```rust
mod custom_stop_reason_example {
    use llmposter::Fixture;
    use llmposter::fixture::FixtureResponse;

    fn fixture_with_max_tokens() -> Fixture {
        Fixture {
            response: Some(FixtureResponse {
                content: Some("hit max tokens".to_string()),
                tool_calls: None,
                stop_reason: Some("max_tokens".to_string()),   // Anthropic/Gemini
                finish_reason: Some("length".to_string()),     // OpenAI
            }),
            ..Fixture::new()
        }
    }
}
```

**OAuth feature**: Enabled by default. Adds `/oauth/authorize`, `/oauth/token`, and `/oauth/device/code` endpoints. Configure with `with_oauth_defaults()` (uses `client_id="mock-client"`, `client_secret="mock-secret"`) or `with_oauth(OAuthConfig { ... })`. OAuth-issued tokens are automatically accepted on LLM endpoints.

**MSRV**: 1.89 (required by `oauth-mock` dependency).

**x-request-id**: Present on every response as `req-llmposter-{N}`. Assert on the prefix, not the exact counter value.

**429 rate-limit headers**: Provider-specific — Anthropic responses include `anthropic-ratelimit-requests-{limit,remaining,reset}`; OpenAI includes `x-ratelimit-*` headers; Gemini includes only `retry-after`.

## Pitfalls

### Wrong: Broad fixture registered before specific one

First-match-wins means a broad substring swallows requests before a more specific pattern is reached.

```rust
mod wrong_fixture_order {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn registers_broad_before_specific() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            // BAD — "stock" matches "stock price of AAPL" first
            .fixture(Fixture::new().match_user_message("stock").respond_with_content("generic"))
            .fixture(Fixture::new().match_user_message("stock price of AAPL").respond_with_content("$150.42"))
            .build()
            .await?;
        // A request containing "stock price of AAPL" returns "generic", not "$150.42"
        drop(server);
        Ok(())
    }
}
```

### Right: Specific fixture registered before broad one

```rust
mod right_fixture_order {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn registers_specific_before_broad() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            // GOOD — specific pattern first
            .fixture(Fixture::new().match_user_message("stock price of AAPL").respond_with_content("$150.42"))
            .fixture(Fixture::new().match_user_message("stock").respond_with_content("generic"))
            .build()
            .await?;
        drop(server);
        Ok(())
    }
}
```

### Wrong: Tool-call arguments as array or scalar

Array and scalar arguments are rejected by `build()` at fixture validation time.

```rust
mod wrong_tool_call_args {
    use llmposter::fixture::ToolCall;

    fn make_tool_call() -> ToolCall {
        ToolCall {
            name: "get_items".to_string(),
            // BAD — array; rejected at fixture load time
            arguments: serde_json::json!(["item1", "item2"]),
        }
    }
}
```

### Right: Tool-call arguments as JSON object

```rust
mod right_tool_call_args {
    use llmposter::fixture::ToolCall;

    fn make_tool_call() -> ToolCall {
        ToolCall {
            name: "get_items".to_string(),
            // GOOD — JSON object (key-value map)
            arguments: serde_json::json!({"query": "items", "limit": 10}),
        }
    }
}
```

### Wrong: Tool-call with blank (empty) name

A `ToolCall` with an empty `name` string is rejected by `build()` at fixture validation time.

```rust
mod wrong_blank_tool_call_name {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::ToolCall;

    #[tokio::test]
    async fn blank_name_rejected() {
        // BAD — empty name; build() returns Err
        let result = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_tool_calls(vec![ToolCall {
                        name: "".to_string(),
                        arguments: serde_json::json!({"key": "value"}),
                    }]),
            )
            .build()
            .await;
        assert!(result.is_err());
    }
}
```

### Right: Tool-call name must be non-empty

```rust
mod right_tool_call_name {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::ToolCall;

    #[tokio::test]
    async fn non_empty_name_accepted() -> Result<(), Box<dyn std::error::Error>> {
        // GOOD — non-empty name passes validation
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_tool_calls(vec![ToolCall {
                        name: "my_tool".to_string(),
                        arguments: serde_json::json!({"key": "value"}),
                    }]),
            )
            .build()
            .await?;
        drop(server);
        Ok(())
    }
}
```

### Wrong: Asserting exact tool-call IDs

Tool-call IDs use a server-wide counter. The exact suffix is non-deterministic across test runs if other requests precede it.

```rust
mod wrong_tool_id_assert {
    fn check_id(body: &serde_json::Value) {
        // BAD — counter value depends on request ordering across all tests
        assert_eq!(body["content"][0]["id"], "toolu_llmposter_1");
    }
}
```

### Right: Assert tool-call ID prefix only

```rust
mod right_tool_id_assert {
    fn check_id(body: &serde_json::Value) {
        // GOOD — assert prefix; verify uniqueness across multiple tool calls
        let id = body["content"][0]["id"].as_str().unwrap();
        assert!(id.starts_with("toolu_llmposter_"));
    }
}
```

### Wrong: Empty string match pattern used as catch-all

An empty pattern is disallowed at validation. Omit the match key entirely for a fixture that matches all requests.

```rust
mod wrong_empty_match_pattern {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn empty_pattern_panics() {
        // BAD — build() returns Err; empty substring patterns are not allowed
        let result = ServerBuilder::new()
            .fixture(Fixture::new().match_user_message("").respond_with_content("default"))
            .build()
            .await;
        assert!(result.is_err());
    }
}
```

### Right: Omit match key for catch-all fixture

```rust
mod right_catch_all_fixture {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn omit_match_for_catch_all() -> Result<(), Box<dyn std::error::Error>> {
        // GOOD — no match_user_message call; fixture matches any request
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("default response"))
            .build()
            .await?;
        drop(server);
        Ok(())
    }
}
```

### Wrong: `with_failure` without a paired response

`with_failure` describes network-level fault behavior on an otherwise valid response. Without `respond_with_content` or `respond_with_tool_calls`, the fixture is invalid and `build()` returns `Err`.

```rust
mod wrong_failure_no_response {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::FailureConfig;

    #[tokio::test]
    async fn failure_without_response_fails() {
        // BAD — with_failure requires a response; build() returns Err
        let result = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .with_failure(FailureConfig {
                        latency_ms: Some(200),
                        ..FailureConfig::default()
                    }),
            )
            .build()
            .await;
        assert!(result.is_err());
    }
}
```

### Right: `with_failure` always paired with a response

```rust
mod right_failure_with_response {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::FailureConfig;

    #[tokio::test]
    async fn failure_paired_with_response() -> Result<(), Box<dyn std::error::Error>> {
        // GOOD — failure always paired with a response
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
            .await?;
        drop(server);
        Ok(())
    }
}
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)
```

---

Summary of all 6 fixes made:

1. **Issue 1** — Comment before `body["id"]` assertion changed from `// x-request-id is always present; assert prefix, not exact value` to `// Body 'id' field uses msg-llmposter-{N} prefix — assert prefix, not exact value`

2. **Issue 2** — Added `### Wrong: Tool-call with blank (empty) name` / `### Right: Tool-call name must be non-empty` pitfall pair (plus a one-sentence note in the Tool Call Response section prose)

3. **Issue 3** — `with_error` prose updated: `"Use \`with_error\` for HTTP error responses"` → `"Use \`with_error\` for HTTP error responses (status code must be in the 400–599 range — values outside this range are rejected at fixture validation time)"`

4. **Issue 4** — Added **Regex DFA size cap** paragraph after the YAML fixtures block

5. **Issue 5** — `with_streaming(latency_ms, chunk_size)` → `with_streaming(latency, chunk_size)` and `latency_ms is the delay` → `latency is the delay` in SSE Streaming prose

6. **Issue 6** — `with_bearer_token_uses` description updated from `"creates a token that expires after N requests"` to `"creates a token that expires after N uses; the (N+1)th request with that token returns 401 with a provider-specific error body"`
```
