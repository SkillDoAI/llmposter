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
description: In-process mock LLM server for testing OpenAI, Anthropic, Gemini, and Responses API clients without live network calls.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
---

## Imports

```rust
use llmposter::{Fixture, Provider, ServerBuilder};
use llmposter::fixture::{FailureConfig, FixtureMatch, FixtureResponse, StreamingConfig, StringMatch, ToolCall};
use llmposter::{AuthState, TokenStatus};
```

```toml
[dependencies]
llmposter = "0.4"
reqwest = { default-features = false, features = ["json"], version = "0.13" }
serde_json = "1"
tokio = { features = ["full"], version = "1" }
```

## Core Patterns

### Basic text response ✅ Current

Spins up a mock Anthropic-compatible server and asserts a response shape. The server binds to a random available port and shuts down when dropped.

```rust
mod basic_text_response {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn should_return_text_content() -> Result<(), Box<dyn std::error::Error>> {
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

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["content"][0]["text"], "Hi from mock!");
        assert_eq!(body["stop_reason"], "end_turn");
        Ok(())
    }
}
```

### Streaming text response ✅ Current

`with_streaming(latency, chunk_size)` enables SSE streaming. Setting `latency` to `Some(N)` injects N milliseconds between chunks; `chunk_size` controls characters per SSE frame.

```rust
mod streaming_text {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn should_stream_sse_events() -> Result<(), Box<dyn std::error::Error>> {
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
                "messages": [{"role": "user", "content": "hello"}],
                "stream": true
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        assert!(resp.headers()["content-type"].to_str()?.contains("text/event-stream"));
        let body = resp.text().await?;
        assert!(body.contains("event: message_start"));
        assert!(body.contains("event: content_block_delta"));
        assert!(body.contains("event: message_stop"));
        Ok(())
    }
}
```

### Tool call response ✅ Current

`respond_with_tool_calls` returns a `tool_use` stop reason. Tool-call IDs are auto-generated as `toolu_llmposter_N` using a server-wide counter.

```rust
mod tool_call_response {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::ToolCall;

    #[tokio::test]
    async fn should_return_tool_use_stop_reason() -> Result<(), Box<dyn std::error::Error>> {
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
                "messages": [{"role": "user", "content": "what's the weather in London?"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["name"], "get_weather");
        assert!(body["content"][0]["id"].as_str().unwrap().starts_with("toolu_llmposter_"));
        Ok(())
    }
}
```

### Error and failure simulation ✅ Current

Use `with_error` for HTTP error codes (4xx/5xx). Use `with_failure` for network-level simulation (truncation, disconnect, latency, body corruption) on otherwise valid responses. The two are mutually exclusive per fixture.

On 429 responses, the server adds provider-shaped rate-limit headers automatically:
- **OpenAI / Responses API**: `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, `x-ratelimit-reset-requests`
- **Anthropic**: `anthropic-ratelimit-requests-limit`, `anthropic-ratelimit-requests-remaining`, `anthropic-ratelimit-requests-reset`
- **Gemini**: `retry-after`

```rust
mod failure_simulation {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::FailureConfig;

    #[tokio::test]
    async fn should_return_rate_limit_error() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("overloaded")
                    .with_error(429, "Rate limit exceeded"),
            )
            .build()
            .await?;

        let resp = reqwest::Client::new()
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "overloaded request"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 429);
        // Anthropic endpoint: provider-shaped rate-limit headers are present
        assert!(resp.headers().contains_key("anthropic-ratelimit-requests-limit"));
        assert!(resp.headers().contains_key("anthropic-ratelimit-requests-remaining"));
        assert!(resp.headers().contains_key("anthropic-ratelimit-requests-reset"));
        Ok(())
    }

    #[tokio::test]
    async fn should_truncate_stream_after_n_frames() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("This is a very long response that gets cut off")
                    .with_streaming(Some(0), Some(5))
                    .with_failure(FailureConfig {
                        truncate_after_frames: Some(2),
                        ..FailureConfig::default()
                    }),
            )
            .build()
            .await?;

        let resp = reqwest::Client::new()
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}],
                "stream": true
            }))
            .send()
            .await?;

        let body = resp.text().await?;
        assert!(body.contains("event: message_start"));
        assert!(!body.contains("event: message_stop"));
        Ok(())
    }
}
```

### Bearer token authentication ✅ Current

Authentication is opt-in. Call `with_auth(true)` to enforce token checking, then register tokens with `with_bearer_token` or `with_bearer_token_uses`. Requests without a valid token receive a provider-shaped 401 response.

```rust
mod bearer_auth {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn should_reject_missing_token() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .with_auth(true)
            .with_bearer_token("test-token-abc")
            .fixture(
                Fixture::new().respond_with_content("authenticated"),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();

        let authed = client
            .post(format!("{}/v1/messages", server.url()))
            .header("Authorization", "Bearer test-token-abc")
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await?;
        assert_eq!(authed.status(), 200);

        let unauthed = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await?;
        assert_eq!(unauthed.status(), 401);
        Ok(())
    }

    #[tokio::test]
    async fn should_reject_after_uses_exhausted() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .with_auth(true)
            .with_bearer_token_uses("limited-token", 2)
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await?;

        let client = reqwest::Client::new();
        let post = |s: &'_ _| {
            client
                .post(format!("{}/v1/messages", s))
                .header("Authorization", "Bearer limited-token")
                .json(&serde_json::json!({
                    "model": "claude-sonnet-4-6",
                    "max_tokens": 1024,
                    "messages": [{"role": "user", "content": "hi"}]
                }))
        };

        // First two requests succeed
        assert_eq!(post(&server.url()).send().await?.status(), 200);
        assert_eq!(post(&server.url()).send().await?.status(), 200);
        // Third request: token is exhausted, returns 401
        assert_eq!(post(&server.url()).send().await?.status(), 401);
        Ok(())
    }
}
```

## Configuration

**ServerBuilder options:**

| Method | Default | Description |
|---|---|---|
| `.bind("host:port")` | random port on 127.0.0.1 | Override bind address |
| `.verbose(true)` | `false` | Diagnostic logging; no-match returns `{"error":{"message":"No fixture matched"}}` with 404 |
| `.with_auth(true)` | `false` | Enforce bearer token on all LLM endpoints |
| `.with_bearer_token("token")` | — | Register unlimited-use token |
| `.with_bearer_token_uses("token", N)` | — | Register token valid for exactly N requests |
| `.load_yaml(path)` | — | Load fixtures from a YAML file |
| `.load_yaml_dir(dir)` | — | Load all YAML files from a directory |

**Response headers:** Every response (success or error) includes an `x-request-id: req-llmposter-{N}` header, where `N` is a deterministic per-server counter starting at 1. This header is useful for correlating log output with specific requests in verbose mode.

```rust
let resp = client.post(format!("{}/v1/messages", server.url()))
    /* ... */
    .send().await?;
assert!(resp.headers()["x-request-id"].to_str()?.starts_with("req-llmposter-"));
```

**YAML fixture format:**

```yaml
- match:
    user_message: "hello"        # substring match
    # user_message: { regex: "^(hi|hello)" }  # regex match
  response:
    content: "Hi from YAML fixture!"

- match:
    model: "claude-sonnet"
  response:
    tool_calls:
      - name: get_weather
        arguments:
          location: "London"
          unit: "celsius"

- error:
    status: 429
    message: "Rate limit exceeded"
```

**Provider routing** — paths are provider-unique; swap only the base URL in clients:

| Provider | Path |
|---|---|
| Anthropic | `/v1/messages` |
| OpenAI Chat | `/v1/chat/completions` |
| OpenAI Responses | `/v1/responses` |
| Gemini | `/v1beta/models/{model}:generateContent` |

**Fixture ordering:** first registered fixture wins. Place specific matches before catch-alls.

**`run_with_output` for CLI testing:** use `llmposter::cli::run_with_output(&cli, &mut buf)` when capturing CLI status output in tests — all status messages go through the provided writer, not stderr.

## Pitfalls

### `failure:` vs `error:` field confusion

**Wrong** — using `with_failure` to simulate an HTTP error code:

```rust
// This does NOT return a 429. It injects a network-level fault on a 200 response.
Fixture::new()
    .with_failure(FailureConfig {
        latency_ms: Some(0),
        ..FailureConfig::default()
    })
```

**Right** — use `with_error` for HTTP error status codes:

```rust
use llmposter::{Fixture, ServerBuilder};
use llmposter::fixture::FailureConfig;

// HTTP error response
Fixture::new().with_error(429, "Rate limit exceeded");

// Network-level fault on an otherwise valid 200 response
Fixture::new()
    .respond_with_content("response")
    .with_failure(FailureConfig {
        corrupt_body: Some(true),
        ..FailureConfig::default()
    });
```

### Empty substring catch-all is rejected at startup

**Wrong** — an empty `match_user_message` silently matched everything before v0.3.5 and is now a startup error:

```rust
// Server fails to start: empty substring patterns are rejected at fixture validation.
Fixture::new()
    .match_user_message("")
    .respond_with_content("fallback")
```

**Right** — omit the match rule entirely for a true catch-all, or use a specific pattern:

```rust
use llmposter::{Fixture, ServerBuilder};

// Catch-all: no match rule means match any request
Fixture::new().respond_with_content("fallback");

// Wildcard via regex
use llmposter::fixture::{Fixture as F, FixtureMatch, StringMatch};
// Use struct literal with StringMatch::regex for regex matching
```

### Empty regex pattern is rejected at startup

**Wrong** — an empty regex string is a distinct validation failure from an empty substring:

```rust
use llmposter::fixture::{FixtureMatch, StringMatch};

// Server fails to start: empty regex patterns are rejected at fixture validation.
FixtureMatch {
    user_message: Some(StringMatch::Regex("".to_string())),
    ..FixtureMatch::default()
}
```

**Right** — use a specific regex pattern or omit the match rule for a catch-all:

```rust
// Specific regex match
FixtureMatch {
    user_message: Some(StringMatch::regex("^(hi|hello)")),
    ..FixtureMatch::default()
}
```

### Regex DFA size limit causes load-time failure

Regex patterns are compiled with a 1 MB DFA size cap. Pathologically large or deeply nested patterns hit this limit at fixture load time (both `.build()` and `.load_yaml()`), not at match time.

**Wrong** — overly complex alternation or repetition that causes DFA explosion:

```rust
use llmposter::fixture::{FixtureMatch, StringMatch};

// May fail at ServerBuilder::build() with a DFA size error
FixtureMatch {
    user_message: Some(StringMatch::regex("(a+)+b|(a+)+c|(a+)+d")), // exponential DFA blowup
    ..FixtureMatch::default()
}
```

**Right** — keep regex patterns simple; rewrite exponential constructs as multiple fixtures or literal matches:

```rust
// Use multiple specific fixtures instead of one complex regex
ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("ab").respond_with_content("matched a"))
    .fixture(Fixture::new().match_user_message("ac").respond_with_content("matched b"))
```

### Tool-call `arguments` must be a JSON object

**Wrong** — passing a string or array as tool arguments:

```rust
use llmposter::fixture::ToolCall;

// Fails fixture validation: arguments must be a serde_json::Value::Object
ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!("San Francisco"),  // string — rejected
}
```

**Right** — always use a JSON object:

```rust
use llmposter::fixture::ToolCall;

ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!({"location": "San Francisco", "unit": "celsius"}),
}
```

### Blank tool name is rejected at startup

**Wrong** — a tool call with an empty name fails fixture validation:

```rust
use llmposter::fixture::ToolCall;

// Server fails to start: blank tool names are rejected at fixture validation.
ToolCall {
    name: "".to_string(),
    arguments: serde_json::json!({"location": "London"}),
}
```

**Right** — always provide a non-empty tool name:

```rust
use llmposter::fixture::ToolCall;

ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!({"location": "London"}),
}
```

### Error status codes outside 400–599 are rejected at startup

**Wrong** — passing a status code below 400 or above 599 fails fixture validation:

```rust
// Server fails to start: status 99 and status 600 are outside the valid 400–599 range.
Fixture::new().with_error(99, "not a real error");
Fixture::new().with_error(600, "also invalid");
```

**Right** — use a standard HTTP error status code in the 4xx–5xx range:

```rust
Fixture::new().with_error(400, "Bad request");
Fixture::new().with_error(500, "Internal server error");
```

### Asserting exact tool-call IDs is fragile

**Wrong** — tool-call IDs use a server-wide counter shared across all tests:

```rust
// Fragile: counter value depends on test execution order
assert_eq!(body["content"][0]["id"], "toolu_llmposter_1");
```

**Right** — assert on prefix and uniqueness:

```rust
let id = body["content"][0]["id"].as_str().unwrap();
assert!(id.starts_with("toolu_llmposter_"));
```

### Assuming `Gemini Content.role` is always present

**Wrong** — treating `role` as a non-optional string on Gemini responses:

```rust
// Panics on safety-blocked responses where role is absent
let role: &str = body["candidates"][0]["content"]["role"].as_str().unwrap();
```

**Right** — handle `role` as optional with a default:

```rust
let role = body["candidates"][0]["content"]["role"]
    .as_str()
    .unwrap_or("model");
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3

### v0.3.x → v0.4.0

**MSRV bump to Rust 1.89** (required by `oauth-mock` dependency). Update `rust-toolchain.toml` and CI matrix:

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.89"
```

**`oauth` feature enabled by default.** To opt out:

```toml
# Cargo.toml
[dev-dependencies]
llmposter = { version = "0.4", default-features = false }
```

**Authentication is now opt-in.** Servers without `with_auth(true)` behave as before. OAuth-issued tokens are automatically valid on LLM endpoints — no separate `with_bearer_token` registration needed for OAuth flows.

### v0.3.3 → v0.3.4 (breaking)

**Responses API streaming protocol changed.** Events now use nested `response` envelopes with `sequence_number`. `response.in_progress` was added; `response.done` was removed.

Before (v0.3.3):

```rust
// Flat event stream, looked for "response.done"
if line.contains("response.done") { break; }
```

After (v0.3.4+):

```rust
// Nested envelope — handle "response.in_progress", no "response.done"
if line.contains("response.completed") { break; }
```

**OpenAI error shape changed.** Update deserialization structs:

```rust
// Before (v0.3.3)
struct ApiError { code: Option<u32>, message: String }

// After (v0.3.4+)
#[derive(serde::Deserialize)]
struct ApiError {
    #[serde(rename = "type")]
    error_type: String,
    code: Option<String>,       // now String, not u32
    param: Option<serde_json::Value>,  // new field, present as null
    message: String,
}
```

## API Reference

**`ServerBuilder::new()`** — Creates a builder. No fixtures registered; binds to a random available port by default.

**`ServerBuilder::fixture(f: Fixture)`** — Registers a single fixture. Fixtures match in registration order; first match wins.

**`ServerBuilder::with_auth(enabled: bool)`** — Enables bearer token enforcement on all LLM endpoints. Unauthenticated requests receive a provider-shaped 401 response.

**`ServerBuilder::with_bearer_token(token: &str)`** — Registers an unlimited-use bearer token. Requires `with_auth(true)`.

**`ServerBuilder::with_bearer_token_uses(token: &str, max_uses: u64)`** — Registers a token valid for exactly `max_uses` requests; subsequent requests receive 401.

**`ServerBuilder::load_yaml(path: &Path)`** — Loads fixtures from a YAML file. Returns `Err` if the file is missing or any fixture fails validation.

**`ServerBuilder::load_yaml_dir(dir: &Path)`** — Loads all YAML files from a directory. Files are processed in filesystem order.

**`ServerBuilder::build()`** — Async. Validates all fixtures, binds the server, and returns `Result<MockServer, _>`. Fixture validation errors (empty patterns, invalid status codes, non-object tool arguments, blank tool names) surface here.

**`MockServer::url()`** — Returns the base URL as `String` (e.g., `http://127.0.0.1:PORT`). Use to construct endpoint paths: `format!("{}/v1/messages", server.url())`.

**`Fixture::new()`** — Creates an empty fixture that matches any request for any provider.

**`Fixture::match_user_message(pattern: &str)`** — Substring match against the last user message. For regex, use struct literal with `FixtureMatch { user_message: Some(StringMatch::regex("pattern")), .. }`.

**`Fixture::match_model(pattern: &str)`** — Substring match against the `model` field. `"claude-sonnet"` matches `"claude-sonnet-4-6"` but not `"claude-haiku-3"`.

**`Fixture::respond_with_content(content: &str)`** — Sets a text response. Default `stop_reason` is `"end_turn"`. Override via struct literal with `FixtureResponse { stop_reason: Some("max_tokens".to_string()), .. }`.

**`Fixture::respond_with_tool_calls(tool_calls: Vec<ToolCall>)`** — Sets a tool-use response. Default `stop_reason` is `"tool_use"`. `ToolCall.arguments` must be a JSON object. `ToolCall.name` must be non-empty.

**`Fixture::with_error(status: u16, message: &str)`** — Returns an HTTP error response (status 400–599). Mutually exclusive with `response` and `failure` on the same fixture.
```

---

**Summary of the three targeted fixes:**

1. **Empty regex Right example** (`StringMatch::Regex("^(hi|hello)".to_string())` → `StringMatch::regex("^(hi|hello)")`) — uses the documented static constructor to match the actual `Regex(RegexMatch)` variant shape.

2. **DFA Wrong example** (replaced `Fixture::new().match_user_message("(a+)+b|...")` with a `FixtureMatch { user_message: Some(StringMatch::regex("(a+)+b|...")), .. }` struct literal) — substring matching never compiles a DFA; the pitfall only applies to regex fixtures.

3. **`should_reject_after_uses_exhausted`** (all three `post(server.url())` calls → `post(&server.url())`) — `server.url()` returns an owned `String`; the closure parameter `s: &'_ _` requires a reference, which Rust does not auto-provide in function-call argument position.
```
