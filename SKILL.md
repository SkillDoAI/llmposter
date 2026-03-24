---
name: llmposter
description: Rust crate that runs a local mock LLM server replaying canned fixtures for Anthropic, OpenAI, Gemini, and OpenAI Responses API endpoints in integration tests.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---

## Imports

```rust
use llmposter::{Fixture, MockServer, Provider, ServerBuilder};
use llmposter::fixture::{FailureConfig, FixtureResponse, StreamingConfig, ToolCall};
use llmposter::auth::{AuthState, TokenStatus};
```

```toml
[dependencies]
llmposter = "0.4.0"
serde_json = "1"

[dev-dependencies]
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.13", default-features = false, features = ["json"] }
```

To opt out of the `oauth` Cargo feature and reduce binary size:

```toml
[dependencies]
llmposter = { version = "0.4.0", default-features = false }
```

## Core Patterns

### Minimal mock server with wildcard fixture ✅ Current

A `Fixture` with no match rule responds to every incoming request. The server binds to an ephemeral port by default. Drop `MockServer` to shut it down.

```rust
mod basic_server {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_any_request() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("Hello from mock"))
            .build()
            .await?;

        let base = server.url(); // "http://127.0.0.1:<PORT>"
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", base))
            .header("content-type", "application/json")
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        Ok(())
    }
}
```

### Matching on user message content ✅ Current

`match_user_message` does substring matching against the last user message. Fixtures are evaluated in insertion order — first match wins. Unmatched requests return 404 with `{"error": {"message": "No fixture matched"}}`.

```rust
mod message_matching {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_matched_and_fallback() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("stock price")
                    .respond_with_content("AAPL is $200"),
            )
            .fixture(Fixture::new().respond_with_content("fallback"))
            .build()
            .await?;

        let base = server.url(); // returns String — use &base where &str is required
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", &base))
            .header("content-type", "application/json")
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 256,
                "messages": [{"role": "user", "content": "What is the stock price of AAPL?"}]
            }))
            .send()
            .await?;

        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["content"][0]["text"], "AAPL is $200");
        assert_eq!(body["stop_reason"], "end_turn");

        let resp2 = client
            .post(format!("{}/v1/messages", &base))
            .header("content-type", "application/json")
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 256,
                "messages": [{"role": "user", "content": "What is the weather today?"}]
            }))
            .send()
            .await?;

        let body2: serde_json::Value = resp2.json().await?;
        assert_eq!(body2["content"][0]["text"], "fallback");
        Ok(())
    }
}
```

### SSE streaming response ✅ Current

`with_streaming(latency_ms, chunk_size_chars)` enables SSE. The client must include `"stream": true` in the request body. Response `Content-Type` is `text/event-stream`.

```rust
mod streaming_text {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_streaming_response() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("count")
                    .respond_with_content("one two three four five")
                    .with_streaming(Some(0), Some(4)), // (latency_ms, chunk_size_chars)
            )
            .build()
            .await?;

        let base = server.url(); // returns String — use &base where &str is required
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", &base))
            .header("content-type", "application/json")
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 256,
                "stream": true,
                "messages": [{"role": "user", "content": "count to five"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        let ct = resp.headers()["content-type"].to_str()?;
        assert!(ct.contains("text/event-stream"));
        let body = resp.text().await?;
        assert!(body.contains("event: message_start"));
        assert!(body.contains("event: message_stop"));
        Ok(())
    }
}
```

### Tool call response ✅ Current

`respond_with_tool_calls` returns an Anthropic-shaped tool-use response. `stop_reason` defaults to `"tool_use"` (Anthropic) or `"tool_calls"` (OpenAI). `arguments` must be a JSON object.

```rust
mod tool_calls {
    use llmposter::{Fixture, ServerBuilder};
    use llmposter::fixture::ToolCall;

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
            .header("content-type", "application/json")
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 256,
                "messages": [{"role": "user", "content": "What is the weather in London?"}]
            }))
            .send()
            .await?;

        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["name"], "get_weather");
        assert_eq!(body["content"][0]["input"]["location"], "London");
        Ok(())
    }
}
```

### Bearer token authentication ✅ Current

`with_bearer_token` adds an unlimited-use token and implicitly enables auth. Requests without a valid `Authorization: Bearer <token>` header receive 401.

```rust
mod bearer_auth {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_auth_enforcement() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .with_bearer_token("test-token-xyz")
            .fixture(Fixture::new().respond_with_content("authorized"))
            .build()
            .await?;

        let client = reqwest::Client::new();
        let payload = serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 256,
            "messages": [{"role": "user", "content": "hi"}]
        });

        // No token → 401
        let unauth = client
            .post(format!("{}/v1/messages", server.url()))
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await?;
        assert_eq!(unauth.status(), 401);

        // Valid token → 200
        let authed = client
            .post(format!("{}/v1/messages", server.url()))
            .header("content-type", "application/json")
            .header("authorization", "Bearer test-token-xyz")
            .json(&payload)
            .send()
            .await?;
        assert_eq!(authed.status(), 200);
        Ok(())
    }
}
```

## Configuration

| Setting | Default | Builder method |
|---|---|---|
| Bind address | `127.0.0.1:0` (ephemeral) | `ServerBuilder::bind("0.0.0.0:8080")` |
| Verbose logging | `false` | `ServerBuilder::verbose(true)` |
| Bearer auth | disabled | `ServerBuilder::with_bearer_token("…")` |
| Limited-use token | — | `ServerBuilder::with_bearer_token_uses("…", 5)` |
| OAuth mock | disabled | `ServerBuilder::with_oauth_defaults()` |

**Provider routing** (base URL is swapped; paths are unchanged):

| Path | Provider format |
|---|---|
| `/v1/messages` | Anthropic |
| `/v1/chat/completions` | OpenAI |
| `/v1/responses` | OpenAI Responses API |
| `/v1beta/models/{model}:generateContent` | Gemini |

**YAML fixture files** must have a top-level `fixtures:` key. All fixture structs use `#[serde(deny_unknown_fields)]` — field name typos are caught at load time, not silently ignored. Use `load_yaml(path)` for a single file, `load_yaml_dir(dir)` for a directory.

**Error status codes** passed to `with_error` must be in range 400–599.

**OAuth defaults** (`with_oauth_defaults`): `client_id = "mock-client"`, `client_secret = "mock-secret"`, redirect URIs `["https://example.com/callback"]`, scopes `["openid","profile","email"]`. Requires the `oauth` Cargo feature (on by default).

**MSRV:** 1.89 as of v0.4.0, required by the `oauth-mock` dependency.

## Pitfalls

### Wrong — Integer truncation on duration conversion

```rust
// WRONG: as_millis() returns u128; `as u64` silently truncates on large values
// and is flagged by clippy
let elapsed_ms = start.elapsed().as_millis() as u64;
```

### Right — Use try_from for safe u128 → u64 conversion

```rust
// RIGHT: panics loudly if the value overflows u64 (unreachable in practice)
let elapsed_ms = u64::try_from(start.elapsed().as_millis())
    .expect("elapsed fits u64");
```

---

### Wrong — Empty substring match silently catches every request

```yaml
# WRONG: empty string matches all input, masking every fixture below it.
# Rejected at fixture load time in v0.3.6+.
match:
  user_message: ""
```

### Right — Provide a non-empty, specific pattern

```yaml
# RIGHT
match:
  user_message: "get weather"
```

---

### Wrong — Tool-call arguments as a non-object value

```yaml
# WRONG: arguments must be a JSON object. A string, array, or number is rejected
# at fixture load time.
tool_calls:
  - name: get_weather
    arguments: "London"
```

### Right — Arguments as a JSON object

```yaml
# RIGHT
tool_calls:
  - name: get_weather
    arguments:
      location: "London"
      unit: "celsius"
```

---

### Wrong — Wildcard fixture placed before specific fixtures

```rust
// WRONG: the wildcard catches everything; the specific fixture below never fires
ServerBuilder::new()
    .fixture(Fixture::new().respond_with_content("fallback"))
    .fixture(
        Fixture::new()
            .match_user_message("hello")
            .respond_with_content("hi"),
    )
```

### Right — Specific fixtures before the wildcard catch-all

```rust
// RIGHT: specific first, wildcard last
ServerBuilder::new()
    .fixture(
        Fixture::new()
            .match_user_message("hello")
            .respond_with_content("hi"),
    )
    .fixture(Fixture::new().respond_with_content("fallback"))
```

---

### Wrong — Enabling auth with no tokens registered

```rust
// WRONG: with_auth(true) alone causes every request to be rejected with 401
// because no tokens are in the store
let server = ServerBuilder::new()
    .with_auth(true)
    .fixture(Fixture::new().respond_with_content("unreachable"))
    .build()
    .await?;
```

### Right — Add at least one token when enabling auth

```rust
// RIGHT: with_bearer_token implicitly enables auth AND registers a token
let server = ServerBuilder::new()
    .with_bearer_token("my-test-token")
    .fixture(Fixture::new().respond_with_content("authorized"))
    .build()
    .await?;
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3

### MSRV bumped to 1.89 (v0.3.x → v0.4.0)

The `oauth-mock` dependency requires Rust 1.89. Update your toolchain:

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.89"
```

### Responses API streaming protocol changed (v0.3.3 → v0.3.4)

Events now carry a nested `response` envelope and a `sequence_number` field. The non-spec `response.done` event was removed. Consumers waiting for `response.done` will hang.

**Before:** listened for `response.done` as the terminal event.  
**After:** handle `response.in_progress`; unwrap the nested `response` object on each event; use the correct OpenAI Responses API terminal event.

### Error response shape changed (v0.3.3 → v0.3.4)

The `code` field changed from integer to `Option<String>`. A nullable `param` field was added.

**Before:**
```json
{"error": {"code": 429, "message": "Rate limit exceeded"}}
```

**After:**
```json
{"error": {"type": "rate_limit_error", "code": "rate_limit_exceeded", "message": "Rate limit exceeded", "param": null}}
```

Update any structs or assertions that deserialize or pattern-match on `error.code`. Use `SpecErrorResponse` for deserialization.

### Empty pattern and blank tool-name validation (v0.3.4 → v0.3.6)

Empty substring patterns, empty regex patterns, and blank tool names are now rejected at fixture load time. Fix any YAML fixtures that relied on these values — they were silently accepted before but never matched as intended.

## API Reference

**`ServerBuilder::new()`** — Create a builder. Default bind: `127.0.0.1:0` (ephemeral port). All fields private; configure via builder methods.

**`ServerBuilder::fixture(f: Fixture)`** — Add one fixture. First matching fixture wins at request time.

**`ServerBuilder::with_bearer_token(token: &str)`** — Add an unlimited-use bearer token; implicitly enables auth enforcement.

**`ServerBuilder::with_bearer_token_uses(token: &str, max_uses: u64)`** — Add a bearer token that expires after `max_uses` requests; further uses return 401.

**`ServerBuilder::with_oauth_defaults()`** — Start an embedded OAuth mock with `client_id = "mock-client"`, `client_secret = "mock-secret"`. Requires `oauth` feature (default-on).

**`ServerBuilder::load_yaml(path: &Path)`** — Load and validate fixtures from a single YAML file. Returns `Result<ServerBuilder, …>`.

**`ServerBuilder::build()`** — `async`. Validate all fixtures, bind the HTTP server. Returns `Result<MockServer, …>`.

**`MockServer::url()`** — Returns the running server base URL as a `String`, e.g. `"http://127.0.0.1:PORT"`. Use `&server.url()` where `&str` is required. Server shuts down when `MockServer` is dropped.

**`Fixture::new()`** — All-`None` fixture; matches any request when no match rule is set.

**`Fixture::match_user_message(pattern: &str)`** — Substring match against the last user message. Pattern must be non-empty.

**`Fixture::respond_with_content(content: &str)`** — Plain-text response. Mutually exclusive with `respond_with_tool_calls`.

**`Fixture::respond_with_tool_calls(tool_calls: Vec<ToolCall>)`** — Tool-use response. `stop_reason` defaults to `"tool_use"` (Anthropic) / `"tool_calls"` (OpenAI). Arguments must be JSON objects.

**`Fixture::with_error(status: u16, message: &str)`** — Return HTTP error (status 400–599) instead of a response body. Mutually exclusive with `with_failure`.

**`Fixture::with_failure(failure: FailureConfig)`** — Simulate network/streaming failures: `latency_ms`, `corrupt_body`, `truncate_after_frames`, `disconnect_after_ms`. Requires `response` to also be set.

**`Fixture::for_provider(provider: Provider)`** — Pin fixture to one endpoint family: `Provider::OpenAI`, `Provider::Anthropic`, `Provider::Gemini`, or `Provider::Responses`. Without this, the fixture matches any provider.
