---
name: llmposter
description: Embedded mock server for LLM provider APIs (OpenAI, Anthropic, Gemini, Responses) with fixture-based request matching, streaming SSE simulation, bearer/OAuth auth testing, and network failure injection.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---


## Imports

```rust
// Crate-root re-exports (use these in preference to submodule paths):
use llmposter::{Fixture, FailureConfig, MockServer, Provider, ServerBuilder, ToolCall};
use llmposter::{AuthState, OAuthConfig, TokenStatus};

// Not re-exported at crate root — access via submodule when doing direct struct construction:
use llmposter::fixture::StringMatch;

// CLI integration tests only:
use llmposter::cli::{run_with_output, Cli};
```

Add as a **dev** dependency (testing library — not a runtime dep):

```toml
[dev-dependencies]
llmposter = "0.4"
reqwest = { version = "0.13", default-features = false, features = ["json"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }

# To disable the bundled OAuth mock server (reduces binary size):
# llmposter = { version = "0.4", default-features = false }
```

## Core Patterns

### Basic server with text response ✅ Current

Build a server, register a fixture, and point your LLM client at `server.url()`. The server shuts down when the returned `MockServer` is dropped.

```rust
mod basic_response_example {
    use llmposter::{Fixture, MockServer, ServerBuilder};

    #[tokio::test]
    async fn test_anthropic_response() -> Result<(), Box<dyn std::error::Error>> {
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
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["content"][0]["text"], "Hi from Claude mock!");
        assert_eq!(body["stop_reason"], "end_turn");
        assert!(body["id"].as_str().unwrap().starts_with("msg-llmposter-"));
        Ok(())
    }
}
```

`match_user_message` does a **substring** match on the last user message. First registered fixture wins. All responses include an `x-request-id: req-llmposter-{N}` header.

---

### Error and failure simulation ✅ Current

Use `with_error` for HTTP-level errors (4xx/5xx). Use `with_failure` + `FailureConfig` for network-level failures on otherwise valid responses. The two are **mutually exclusive** on a single fixture.

```rust
mod failure_simulation_example {
    use llmposter::{Fixture, FailureConfig, ServerBuilder};
    use std::time::Duration;

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
                "messages": [{"role": "user", "content": "rate limit this"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 429);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["error"]["message"], "Rate limit exceeded");
        Ok(())
    }

    #[tokio::test]
    async fn test_latency_injection() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("slow reply")
                    .with_failure(FailureConfig {
                        latency_ms: Some(200),
                        corrupt_body: None,
                        truncate_after_frames: None,
                        disconnect_after_ms: None,
                    }),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();
        let start = std::time::Instant::now();
        let resp = client
            .post(format!("{}/v1/chat/completions", server.url()))
            .json(&serde_json::json!({
                "model": "gpt-4o",
                "messages": [{"role": "user", "content": "hello"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        assert!(start.elapsed() >= Duration::from_millis(180));
        Ok(())
    }
}
```

`FailureConfig` implements `Default` — use `..FailureConfig::default()` for struct update syntax when setting only some fields.

**Provider-specific rate-limit headers on 429**: The error body is accompanied by provider-specific headers.
- **OpenAI / Responses**: `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, `x-ratelimit-reset-requests`
- **Anthropic**: `anthropic-ratelimit-requests-limit`, `anthropic-ratelimit-requests-remaining`, `anthropic-ratelimit-requests-reset`
- **Gemini**: `retry-after` only

---

### Streaming SSE responses ✅ Current

Enable SSE streaming with `with_streaming(latency_ms, chunk_size)`. Combine with `FailureConfig` to simulate truncated or disconnected streams.

```rust
mod streaming_sse_example {
    use llmposter::{Fixture, FailureConfig, ServerBuilder};

    #[tokio::test]
    async fn test_streaming_text() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("stream this")
                    .respond_with_content("Hello streaming world")
                    .with_streaming(Some(0), Some(5)), // 0ms inter-chunk latency, 5-char chunks
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

        assert_eq!(resp.status(), 200);
        let content_type = resp.headers()["content-type"].to_str().unwrap();
        assert!(content_type.contains("text/event-stream"));
        let body = resp.text().await?;
        assert!(body.contains("event: message_start"));
        assert!(body.contains("event: content_block_delta"));
        assert!(body.contains("event: message_stop"));
        Ok(())
    }

    #[tokio::test]
    async fn test_stream_truncation() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("This long response gets cut off mid-stream")
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
                "messages": [{"role": "user", "content": "hello"}],
                "stream": true
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        let body = resp.text().await?;
        assert!(body.contains("event: message_start"));
        assert!(!body.contains("event: message_stop")); // truncated before end
        Ok(())
    }
}
```

**`corrupt_body: true`** — Replaces the response body with overloaded text instead of valid JSON or SSE. The client receives a non-parseable body. Use this to verify that your client's parse-error path does not panic or hang.

**`disconnect_after_ms: N`** — The server closes the TCP connection after N milliseconds regardless of how much content has been sent. The client receives a connection error or an incomplete stream body. Use this to test client-side reconnect logic and partial-response recovery.

---

### Tool call responses ✅ Current

Return structured tool-call payloads for Anthropic or OpenAI endpoints. `respond_with_content` and `respond_with_tool_calls` are **mutually exclusive** — setting one clears the other.

```rust
mod tool_call_example {
    use llmposter::{Fixture, ServerBuilder, ToolCall};

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
                "messages": [{"role": "user", "content": "what is the weather?"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["name"], "get_weather");
        assert_eq!(body["content"][0]["id"], "toolu_llmposter_1");
        assert_eq!(body["content"][0]["input"]["location"], "London");
        Ok(())
    }
}
```

`ToolCall::arguments` must be a JSON **object**. Arrays and scalar values are rejected at fixture load time.

**Tool-call ID uniqueness**: IDs are globally unique across all requests to the same server instance (e.g., `toolu_llmposter_1`, `toolu_llmposter_2`, ...). The counter is server-wide, so multi-turn test scenarios that send multiple requests will never produce ID collisions.

---

### Bearer token auth ✅ Current

`with_bearer_token()` registers a token and implicitly enables auth enforcement. All LLM endpoints then require `Authorization: Bearer <token>`.

```rust
mod auth_bearer_example {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_bearer_and_expiry() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .with_bearer_token_uses("expiring-token", 2) // expires after 2 uses
            .fixture(Fixture::new().respond_with_content("authenticated"))
            .build()
            .await?;

        let client = reqwest::Client::new();
        let payload = serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 512,
            "messages": [{"role": "user", "content": "hi"}]
        });

        // First two requests succeed
        for _ in 0..2 {
            let resp = client
                .post(format!("{}/v1/messages", server.url()))
                .bearer_auth("expiring-token")
                .json(&payload)
                .send()
                .await?;
            assert_eq!(resp.status(), 200);
        }

        // Third request — token exhausted → 401
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .bearer_auth("expiring-token")
            .json(&payload)
            .send()
            .await?;
        assert_eq!(resp.status(), 401);

        Ok(())
    }
}
```

### AuthState and TokenStatus

`AuthState` is the thread-safe token store that backs `with_bearer_token` and `with_bearer_token_uses`. Use it directly when you need to inspect or manipulate token lifecycle programmatically inside a test.

```rust
mod auth_state_example {
    use llmposter::{AuthState, TokenStatus};

    #[test]
    fn test_token_lifecycle() {
        let state = AuthState::new();

        // Unlimited-use token
        state.add_token("unlimited", None);
        assert_eq!(state.check_and_use("unlimited"), TokenStatus::Valid);
        assert_eq!(state.check_and_use("unlimited"), TokenStatus::Valid);

        // Use-limited token
        state.add_token("one-shot", Some(1));
        assert_eq!(state.check_and_use("one-shot"), TokenStatus::Valid);
        assert_eq!(state.check_and_use("one-shot"), TokenStatus::Exhausted);

        // Unknown token (not registered)
        assert_eq!(state.check_and_use("not-registered"), TokenStatus::Unknown);

        // Revoke a token: removes from store and adds to deny-list
        state.revoke("unlimited");
        assert_eq!(state.check_and_use("unlimited"), TokenStatus::Exhausted);
    }
}
```

### CLI integration tests

`run_with_output` runs the full CLI startup path and redirects status output to a caller-supplied writer, making it suitable for integration tests that verify server startup or YAML fixture validation without capturing stderr.

```rust
mod cli_example {
    use llmposter::cli::{run_with_output, Cli};
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_cli_validate_mode() -> Result<(), Box<dyn std::error::Error>> {
        let cli = Cli {
            fixtures: PathBuf::from("tests/fixtures/"),
            validate: true,
            port: 2112,
            bind: "127.0.0.1".to_string(),
            verbose: false,
        };
        let mut out = Vec::<u8>::new();
        let result = run_with_output(&cli, &mut out).await?;
        assert!(result.is_none()); // --validate returns Ok(None) without binding
        Ok(())
    }
}
```

## Configuration

### Default server settings

| Setting | Default | Override method |
|---------|---------|----------------|
| Bind address + port | `127.0.0.1`, OS-assigned | `ServerBuilder::bind("127.0.0.1:2112")` |
| Auth enforcement | disabled | `with_bearer_token()` or `with_auth(true)` |
| Verbose error bodies | disabled | `ServerBuilder::verbose(true)` |
| OAuth support | enabled (default feature) | `with_oauth_defaults()` or `with_oauth(OAuthConfig { .. })` |

**413 request body too large**: When a request body exceeds the default limit the server returns HTTP 413. The `x-request-id` header is still present on 413 responses — the request ID is assigned before the body-size check runs.

### Fixture matching rules

- **First-match-wins**: fixtures are evaluated in registration order — place specific matchers before catch-alls.
- **Substring match**: `match_user_message("foo")` matches any user message containing `"foo"`. For regex, use `StringMatch::regex()` and construct `FixtureMatch` directly:

  ```rust
  use llmposter::fixture::{FixtureMatch, StringMatch};

  let match_rule = FixtureMatch {
      user_message: Some(StringMatch::regex(r"^(hello|hi)\b")),
      model: None,
  };
  ```

- **Regex DFA limit**: regex patterns that compile to a DFA exceeding 1 MiB are rejected at fixture load time with a DFA size error. This prevents ReDoS-style memory exhaustion from pathological patterns. Keep patterns simple and anchored.
- **Model filter**: `match_model("claude-sonnet")` does a substring match on the `model` request field.
- **Provider filter**: omit `for_provider()` to match all endpoints. Use it only when response format must be provider-specific (e.g., Anthropic-only `stop_reason` field).
- **Unmatched requests**: return `404`. With `verbose(true)`, the error body includes `"No fixture matched"`.
- **Gemini safety-blocked responses**: On the Gemini endpoint, safety-filtered responses may omit the `Content.role` field entirely. Treat `candidates[0].content.role` as `Option<String>` in your client — this matches real Gemini API behavior where the field is absent (not `null`) on blocked responses.

### YAML fixture loading

```yaml
# fixtures/responses.yaml
fixtures:
  - match:
      user_message: "hello"
    response:
      content: "Hi from YAML fixture"
  - match:
      user_message: "rate limit"
    error:
      status: 429
      message: "Rate limit exceeded"
  - match:
      user_message: "slow"
    response:
      content: "delayed response"
    failure:
      latency_ms: 300
  - response:
      content: "catch-all response"   # no match = always fires
```

All YAML fixture structs use `deny_unknown_fields` — typos in keys are caught at load time and the server refuses to start.

```rust
mod yaml_fixture_example {
    use llmposter::ServerBuilder;
    use std::path::Path;

    #[tokio::test]
    async fn test_yaml_fixture_loading() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .load_yaml(Path::new("fixtures/responses.yaml"))? // sync, uses ?
            .build()
            .await?;

        let _ = server.url(); // server is live
        Ok(())
    }
}
```

Load an entire directory of YAML files with `load_yaml_dir`:

```rust
mod yaml_dir_example {
    use llmposter::ServerBuilder;
    use std::path::Path;

    #[tokio::test]
    async fn test_yaml_dir_loading() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .load_yaml_dir(Path::new("fixtures/"))?
            .build()
            .await?;

        let _ = server.url();
        Ok(())
    }
}
```

### YAML streaming and failure config

```yaml
fixtures:
  - response:
      content: "streamed content"
    streaming:
      latency: 50        # ms between SSE frames
      chunk_size: 5      # characters per frame
    failure:
      truncate_after_chunks: 3   # YAML alias for truncate_after_frames
      disconnect_after_ms: 1000
      corrupt_body: false
      latency_ms: 100
```

### OAuth configuration (feature = `oauth`, on by default in 0.4.0)

```rust
mod oauth_config_example {
    use llmposter::{Fixture, OAuthConfig, ServerBuilder};

    #[tokio::test]
    async fn test_oauth_defaults() -> Result<(), Box<dyn std::error::Error>> {
        // Uses client_id="mock-client", client_secret="mock-secret"
        let server = ServerBuilder::new()
            .with_oauth_defaults()
            .fixture(Fixture::new().respond_with_content("oauth ok"))
            .build()
            .await?;

        let _ = server.url();
        Ok(())
    }

    #[tokio::test]
    async fn test_oauth_custom() -> Result<(), Box<dyn std::error::Error>> {
        let config = OAuthConfig {
            client_id: "my-client".to_string(),
            client_secret: "my-secret".to_string(),
            redirect_uris: vec!["https://example.com/callback".to_string()],
            scopes: vec!["openid".to_string(), "profile".to_string()],
        };
        let server = ServerBuilder::new()
            .with_oauth(config)
            .fixture(Fixture::new().respond_with_content("custom oauth"))
            .build()
            .await?;

        let _ = server.url();
        Ok(())
    }
}
```

OAuth-issued tokens are automatically valid on all LLM endpoints — no extra wiring is required.

## Pitfalls

### Empty match pattern crashes fixture validation

**Wrong** — empty substring match is rejected at load time (would silently match everything):

```rust
// Panics/errors during ServerBuilder::build()
let bad = Fixture::new().match_user_message("");
```

**Right** — use a non-empty substring or regex:

```rust
use llmposter::{Fixture, ServerBuilder};

let server = ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("hello"))
    .build()
    .await
    .unwrap();
```

---

### Tool call arguments must be a JSON object

**Wrong** — scalar values and arrays are rejected at fixture load time:

```rust
use llmposter::ToolCall;

// Compile-time valid, runtime rejected:
let bad = ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!("San Francisco"), // string — rejected
};
```

**Right** — `arguments` must always be a JSON object:

```rust
use llmposter::ToolCall;

let good = ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!({
        "location": "San Francisco",
        "unit": "celsius"
    }),
};
```

---

### Enabling auth without registering a token

**Wrong** — `with_auth(true)` enforces auth but no token is registered, so every request returns 401:

```rust
use llmposter::ServerBuilder;

// All LLM requests return 401 — no valid token exists:
let server = ServerBuilder::new()
    .with_auth(true)
    .build()
    .await
    .unwrap();
```

**Right** — use `with_bearer_token()` (which implicitly enables auth) or `with_oauth_defaults()`:

```rust
use llmposter::{Fixture, ServerBuilder};

let server = ServerBuilder::new()
    .with_bearer_token("test-token")
    .fixture(Fixture::new().respond_with_content("ok"))
    .build()
    .await
    .unwrap();
```

---

### Error status code out of valid range

**Wrong** — status codes outside 400–599 are rejected at fixture load time:

```rust
use llmposter::Fixture;

// Rejected during ServerBuilder::build():
let bad = Fixture::new().with_error(200, "OK");
let also_bad = Fixture::new().with_error(301, "Redirect");
```

**Right** — use only 4xx or 5xx status codes:

```rust
use llmposter::Fixture;

let rate_limit = Fixture::new().with_error(429, "Rate limit exceeded");
let server_error = Fixture::new().with_error(503, "Service unavailable");
```

---

### Unnecessary provider filter breaks cross-endpoint tests

**Wrong** — pinning to one provider silently returns 404 on all other endpoints:

```rust
use llmposter::{Fixture, Provider, ServerBuilder};

// Returns 200 on /v1/chat/completions but 404 on /v1/messages:
let server = ServerBuilder::new()
    .fixture(
        Fixture::new()
            .respond_with_content("generic reply")
            .for_provider(Provider::OpenAI), // unnecessarily restrictive
    )
    .build()
    .await
    .unwrap();
```

**Right** — omit `for_provider()` unless response format must be provider-specific:

```rust
use llmposter::{Fixture, ServerBuilder};

// Matches /v1/messages, /v1/chat/completions, /v1/responses, /v1/generate:
let server = ServerBuilder::new()
    .fixture(Fixture::new().respond_with_content("generic reply"))
    .build()
    .await
    .unwrap();
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3

### MSRV bump to 1.89 (v0.3.x → v0.4.0)

Update your Rust toolchain and `Cargo.toml`:

```toml
[package]
rust-version = "1.89"
```

No API-level breaking changes between v0.3.x and v0.4.0. The `oauth` feature is now on by default — disable it for minimal builds:

```toml
[dev-dependencies]
llmposter = { version = "0.4", default-features = false }
```

### Responses API streaming protocol change (v0.3.3 → v0.3.4)

SSE events on `/v1/responses` now use nested `response` envelopes and include `sequence_number`. The `response.done` event was removed.

**Before (v0.3.3):**

```text
event: response.done
data: {"delta": {"text": "..."}}
```

**After (v0.3.4+):**

```text
event: response.in_progress
data: {"response": {"delta": {"text": "..."}}, "sequence_number": 1}
```

Update SSE assertions:

```rust
// Wrong (pre-0.3.4):
assert!(body.contains("event: response.done"));

// Right (0.3.4+):
assert!(body.contains("event: response.in_progress"));
assert!(!body.contains("event: response.done")); // removed
```

### Error response shape change (v0.3.3 → v0.3.4)

The OpenAI-format error body now matches the real API shape: `code` is a string (not integer), `param` is always present as `null`.

```rust
// Wrong assertion (pre-0.3.4):
assert_eq!(body["error"]["code"], 429); // was integer

// Right (0.3.4+):
assert_eq!(body["error"]["code"], "rate_limit_exceeded"); // now string
assert!(body["error"]["param"].is_null());                // param always present
```

## API Reference

**`ServerBuilder::new()`** — Creates a builder with default settings (random port, no auth, no verbose).

**`ServerBuilder::fixture(f: Fixture)`** — Registers a single fixture; first-match-wins evaluation order.

**`ServerBuilder::fixtures(fixtures: Vec<Fixture>)`** — Registers multiple fixtures in one call.

**`ServerBuilder::build() -> Result<MockServer, _>`** — Async. Validates all fixtures, binds the server, and returns a running `MockServer`. Consumes the builder.

**`ServerBuilder::verbose(v: bool)`** — Enables debug logging and includes `"No fixture matched"` in 404 bodies.

**`ServerBuilder::bind(addr: &str)`** — Sets the bind address; e.g., `"127.0.0.1:2112"`.

**`ServerBuilder::with_bearer_token(token: &str)`** — Registers a token with unlimited uses; implicitly enables auth.

**`ServerBuilder::with_bearer_token_uses(token: &str, max_uses: u64)`** — Token expires after exactly `max_uses` successful LLM requests.

**`ServerBuilder::with_oauth_defaults()`** — Starts embedded OAuth mock with `client_id="mock-client"`, `client_secret="mock-secret"`. Requires `oauth` feature (on by default).

**`MockServer::url() -> String`** — Returns the base URL (e.g., `"http://127.0.0.1:54321"`) to use as the LLM client's base URL.

**`Fixture::new()`** — Creates an empty fixture; no match rule means it matches any request.

**`Fixture::match_user_message(pattern: &str)`** — Substring match on the last user message content (both string and array content formats).

**`Fixture::respond_with_content(content: &str)`** — Sets a text response; clears any existing `tool_calls`.

**`Fixture::respond_with_tool_calls(tool_calls: Vec<ToolCall>)`** — Sets a tool-use response; clears any existing `content`. Mutually exclusive with `respond_with_content`.

**`Fixture::with_error(status: u16, message: &str)`** — Returns an HTTP error (status 400–599) with a provider-specific error body. Mutually exclusive with `response` and `failure`.
