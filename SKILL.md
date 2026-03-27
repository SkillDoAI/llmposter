---
name: llmposter
description: Mock LLM API server for testing clients against OpenAI, Anthropic, Gemini, and Responses API endpoints with configurable fixture-based responses, SSE streaming simulation, auth enforcement, and network failure injection.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---


## Imports

```rust
use llmposter::{Fixture, ServerBuilder};
use llmposter::{FailureConfig, ToolCall};
```

```toml
[dev-dependencies]
llmposter = "0.4"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.13", default-features = false, features = ["json"] }
serde_json = "1"
```

The `oauth` feature is enabled by default. To disable: `llmposter = { version = "0.4", default-features = false }`.

## Core Patterns

### Basic Text Fixture — Anthropic Endpoint ✅ Current

Register a fixture that matches a user message substring and returns a text response. The server binds to a random port on `127.0.0.1` by default; use `server.url()` to retrieve the base URL.

```rust
mod anthropic_text {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_anthropic_text_response() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("hello")
                    .respond_with_content("Hi from mock Claude!"),
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
        assert_eq!(body["content"][0]["text"], "Hi from mock Claude!");
        assert_eq!(body["stop_reason"], "end_turn");
        Ok(())
    }
}
```

Fixtures are evaluated in registration order; the first match wins. `match_user_message` performs substring/contains matching on the last user message. A fixture with no `match_rule` matches every request.

### Tool Call Response ✅ Current

Return a tool-use response by constructing `ToolCall` structs. `arguments` must be a JSON object — strings or arrays are rejected at fixture validation.

```rust
mod tool_call_response {
    use llmposter::{Fixture, ServerBuilder, ToolCall};

    #[tokio::test]
    async fn test_tool_call_fixture() -> Result<(), Box<dyn std::error::Error>> {
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
                "messages": [{"role": "user", "content": "what's the weather?"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["name"], "get_weather");
        // Tool-call IDs use a server-wide counter — assert on prefix, not exact value
        assert!(body["content"][0]["id"]
            .as_str()
            .unwrap()
            .starts_with("toolu_llmposter_"));
        Ok(())
    }
}
```

### SSE Streaming ✅ Current

Configure streaming with `with_streaming(latency_ms, chunk_size)`. Both parameters are `Option`; `None` uses defaults. Set `latency_ms = Some(0)` to disable inter-chunk delay in tests.

```rust
mod sse_streaming {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_anthropic_streaming() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("stream")
                    .respond_with_content("Hello streaming world")
                    .with_streaming(Some(0), Some(5)), // 0ms latency, 5-char chunks
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

### Network Failure Simulation ✅ Current

Use `FailureConfig` on a fixture that also has a response set. `FailureConfig::default()` fills all fields with `None`; use struct update syntax to set only what you need.

```rust
mod failure_simulation {
    use llmposter::{FailureConfig, Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_latency_then_truncated_stream() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("This is a long response that will be cut short")
                    .with_streaming(Some(0), Some(5))
                    .with_failure(FailureConfig {
                        latency_ms: Some(100),
                        truncate_after_frames: Some(2),
                        ..FailureConfig::default()
                    }),
            )
            .build()
            .await?;

        let start = std::time::Instant::now();
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "long response"}],
                "stream": true
            }))
            .send()
            .await?;

        assert!(start.elapsed().as_millis() >= 80);
        assert_eq!(resp.status(), 200);
        let body = resp.text().await?;
        assert!(body.contains("event: message_start"));
        assert!(!body.contains("event: message_stop")); // stream was truncated
        Ok(())
    }
}
```

`error` and `failure` are mutually exclusive on a single fixture. Use `with_error(status, message)` for HTTP error codes; use `with_failure(FailureConfig)` only when the fixture also has a valid response set.

### Bearer Token Auth ✅ Current

`with_auth(true)` alone is insufficient — it enables enforcement but registers no valid token, so every request would be rejected. `with_bearer_token` alone is sufficient; it implicitly enables auth.

```rust
mod bearer_token_auth {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_bearer_auth() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .with_auth(true)
            .with_bearer_token("test-token")
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
        let authed = client
            .post(format!("{base}/v1/messages"))
            .bearer_auth("test-token")
            .json(&payload)
            .send()
            .await?;
        assert_eq!(authed.status(), 200);

        // Missing token → 401
        let unauthed = client
            .post(format!("{base}/v1/messages"))
            .json(&payload)
            .send()
            .await?;
        assert_eq!(unauthed.status(), 401);

        Ok(())
    }
}
```

**401 error bodies are provider-specific.** The response body on an unauthenticated request matches the provider's native error shape:

- **Anthropic** (`/v1/messages`): `{"type":"error","error":{"type":"authentication_error","message":"Invalid API key"}}`
- **OpenAI** (`/v1/chat/completions`): `{"error":{"message":"Invalid API key","type":"authentication_error","code":"invalid_api_key"}}`
- **Responses** (`/v1/responses`): same shape as OpenAI
- **Gemini** (`/v1beta/models/...`): `{"error":{"code":401,"message":"API key not valid","status":"UNAUTHENTICATED"}}`

## Configuration

### ServerBuilder Options

| Method | Default | Description |
|--------|---------|-------------|
| `bind(addr)` | `"127.0.0.1:0"` | Bind address; port `0` = OS-assigned random port |
| `verbose(bool)` | `false` | Enriches 404 "no fixture matched" body with a diagnostic message |
| `with_auth(bool)` | disabled | Enables auth enforcement; must be paired with `with_bearer_token` |
| `with_bearer_token(token)` | none | Registers an unlimited-use bearer token; implicitly enables auth |
| `with_bearer_token_uses(token, n)` | none | Token expires after `n` LLM requests; returns 401 on exhaustion |
| `with_oauth_defaults()` | n/a | Mounts OAuth endpoints (`client_id="mock-client"`, `client_secret="mock-secret"`) |
| `fixture(f)` / `fixtures(vec)` | none | Register one or multiple fixtures |
| `load_yaml(path)` | n/a | Load fixtures from a YAML file |
| `load_yaml_dir(dir)` | n/a | Load all `*.yaml`/`*.yml` fixtures from a directory |

### YAML Fixture Format

```yaml
fixtures:
  - match:
      user_message: "hello"               # substring match
      model: "claude-sonnet"              # optional model substring match
    provider: anthropic                   # openai | anthropic | gemini | responses
    response:
      content: "Hi there"
      stop_reason: "end_turn"             # Anthropic-specific override
    streaming:
      latency: 0                          # ms between SSE frames
      chunk_size: 5                       # chars per delta frame

  - match:
      user_message: { regex: "^weather.*" }   # regex match
    response:
      tool_calls:
        - name: get_weather
          arguments:
            location: "London"            # must be a YAML mapping (JSON object), not a string

  - error:
      status: 429
      message: "Rate limit exceeded"      # status must be 400–599

  - response:
      content: "degraded"
    failure:
      latency_ms: 200
      corrupt_body: true                  # returns HTTP 200 with body "overloaded"
      truncate_after_frames: 3            # cut SSE stream after N frames
      disconnect_after_ms: 500            # close connection after N ms
```

`deny_unknown_fields` is enforced on all fixture structs — unknown YAML keys cause a startup error.

### OAuth (feature `oauth`, enabled by default)

```rust
use llmposter::{OAuthConfig, ServerBuilder};

// Quick setup — client_id="mock-client", client_secret="mock-secret"
let server = ServerBuilder::new()
    .with_oauth_defaults()
    .build()
    .await?;

// Custom config
let server = ServerBuilder::new()
    .with_oauth(OAuthConfig {
        client_id: "my-client".to_string(),
        client_secret: "my-secret".to_string(),
        redirect_uris: vec!["https://example.com/callback".to_string()],
        scopes: vec!["openid".to_string(), "profile".to_string()],
    })
    .build()
    .await?;
```

OAuth-issued tokens are automatically accepted on LLM endpoints — no extra plumbing needed.

### Provider Endpoints

All four provider paths are unique and served simultaneously:

```text
POST /v1/messages             → Anthropic
POST /v1/chat/completions     → OpenAI
POST /v1beta/models/*/...     → Gemini
POST /v1/responses            → Responses API
```

No provider prefix is needed in the base URL — point the client at `http://127.0.0.1:{port}`.

### Response Headers

Every response from every endpoint includes an `x-request-id` header with a deterministic value of the form `req-llmposter-{N}`, where `N` is a server-wide incrementing counter. This is useful for correlating requests in test logs.

```rust
let resp = client.post(format!("{}/v1/messages", server.url())).json(&payload).send().await?;
let req_id = resp.headers()["x-request-id"].to_str()?;
assert!(req_id.starts_with("req-llmposter-"));
```

**429 error fixtures emit provider-specific rate limit headers:**

| Provider | Headers emitted |
|----------|----------------|
| OpenAI / Responses | `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, `x-ratelimit-reset-requests` |
| Anthropic | `anthropic-ratelimit-requests-limit`, `anthropic-ratelimit-requests-remaining`, `anthropic-ratelimit-requests-reset` |
| Gemini | `retry-after` |

## Pitfalls

### `with_auth(true)` alone does not enforce tokens

```rust
// WRONG — no token registered, auth behavior undefined
let server = ServerBuilder::new()
    .with_auth(true)
    .fixture(Fixture::new().respond_with_content("x"))
    .build()
    .await?;
```

```rust
// RIGHT — with_bearer_token implicitly enables auth; with_auth(true) alone registers no token
use llmposter::{Fixture, ServerBuilder};

let server = ServerBuilder::new()
    .with_auth(true)
    .with_bearer_token("test-token")
    .fixture(Fixture::new().respond_with_content("x"))
    .build()
    .await?;
```

### `corrupt_body: true` returns text `"overloaded"`, not an empty body

```rust
// WRONG — corrupt_body does not produce an empty body or a connection drop
let body = resp.bytes().await?;
assert!(body.is_empty());
```

```rust
// RIGHT — HTTP 200 with literal text "overloaded"
assert_eq!(resp.status(), 200);
assert_eq!(resp.text().await?, "overloaded");
```

### Tool-call IDs use a server-wide counter — never assert exact values

```rust
// WRONG — counter is shared across all tests and sessions; exact value is unstable
assert_eq!(body["content"][0]["id"], "toolu_llmposter_1");
```

```rust
// RIGHT — assert on prefix; for multi-call responses, assert uniqueness
let id = body["content"][0]["id"].as_str().unwrap();
assert!(id.starts_with("toolu_llmposter_"));
```

### Empty match patterns are rejected at fixture load time

```rust
// WRONG — empty substring pattern fails validation; server will not start
use llmposter::{Fixture, ServerBuilder};

let server = ServerBuilder::new()
    .fixture(
        Fixture::new()
            .match_user_message("")  // rejected
            .respond_with_content("fallback"),
    )
    .build()
    .await; // returns Err
```

```rust
// RIGHT — omit match_rule entirely for a match-all fixture
use llmposter::{Fixture, ServerBuilder};

let server = ServerBuilder::new()
    .fixture(Fixture::new().respond_with_content("fallback")) // no match_rule = match-all
    .build()
    .await?;
```

### Responses API: `response.done` was removed in v0.3.4

```rust
// WRONG — response.done no longer exists; waiting for it will hang
assert!(frames.iter().any(|f| f.event == "response.done"));
```

```rust
// RIGHT — use response.completed and handle nested response envelopes with sequence_number
assert!(frames.iter().any(|f| f.event == "response.completed"));
```

### Gemini safety-blocked responses omit `Content.role`

When a Gemini request is safety-blocked, the `Content.role` field is absent from the response — serialized as `null` or omitted entirely. Deserializing into a struct with `role: String` will fail. Use `role: Option<String>` for Gemini response structs:

```rust
// WRONG — Content.role is absent on safety blocks; deserialization panics
#[derive(serde::Deserialize)]
struct Content { role: String, parts: Vec<serde_json::Value> }
```

```rust
// RIGHT — role is optional; handle None for safety-blocked responses
#[derive(serde::Deserialize)]
struct Content { role: Option<String>, parts: Vec<serde_json::Value> }
```

### Regex patterns whose compiled DFA exceeds 1MB are rejected at fixture load time

The fixture validator compiles each regex and rejects any pattern whose compiled DFA exceeds 1MB. This check runs during `ServerBuilder::build()` — the server will not start and `build()` returns `Err`.

```rust
// WRONG — overly complex alternation may produce a DFA that exceeds the size limit
Fixture::new().match_user_message(/* would be passed as StringMatch::Regex */ "((a+|b+|c+|d+){1,50}){5}")
```

```rust
// RIGHT — simplify patterns that trigger the DFA size limit; use substring match where possible
Fixture::new().match_user_message("the specific phrase you actually need to match")
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3

### v0.3.x → v0.4.0

**MSRV raised to 1.89** (required by `oauth-mock`):

```text
rustup update stable
# Update rust-toolchain.toml channel to >= 1.89
```

**`oauth` feature is now enabled by default.** To opt out:

```toml
[dev-dependencies]
llmposter = { version = "0.4", default-features = false }
```

**OAuth routes require explicit opt-in** — the feature alone does not mount them:

```rust
// Before: no OAuth endpoints
ServerBuilder::new().build().await?

// After: OAuth endpoints mounted
ServerBuilder::new().with_oauth_defaults().build().await?
```

**Bearer auth now requires both methods together:**

```rust
// Before: with_auth(true) was sometimes sufficient
ServerBuilder::new().with_auth(true).build().await?

// After: token must also be registered
ServerBuilder::new()
    .with_auth(true)
    .with_bearer_token("test-token")
    .build()
    .await?
```

### v0.3.3 → v0.3.4 (Responses API streaming)

`response.done` SSE event removed; `response.in_progress` added; events now use nested `response` envelopes with `sequence_number`:

```rust
// Before (v0.3.3)
assert!(frames.iter().any(|f| f.event == "response.done"));

// After (v0.3.4+)
assert!(frames.iter().any(|f| f.event == "response.completed"));
// Also: unwrap nested `response` envelope and read `sequence_number` for ordering
```

## API Reference

**`ServerBuilder::new()`** — Creates a builder with default bind `127.0.0.1:0`, auth disabled, no fixtures registered.

**`ServerBuilder::fixture(f: Fixture)`** — Registers a single fixture. Evaluated in registration order; first match wins.

**`ServerBuilder::fixtures(fixtures: Vec<Fixture>)`** — Registers multiple fixtures at once.

**`ServerBuilder::bind(addr: &str)`** — Overrides bind address/port. `"127.0.0.1:0"` assigns a random port.

**`ServerBuilder::verbose(v: bool)`** — When `true`, 404 "no fixture matched" responses include a diagnostic message body.

**`ServerBuilder::with_auth(enabled: bool)`** — Enables auth enforcement; must be paired with `with_bearer_token`.

**`ServerBuilder::with_bearer_token(token: &str)`** — Registers an unlimited-use bearer token; implicitly enables auth.

**`ServerBuilder::with_bearer_token_uses(token: &str, max_uses: u64)`** — Token expires after `max_uses` LLM endpoint requests; returns 401 thereafter.

**`ServerBuilder::with_oauth_defaults()`** — Mounts OAuth endpoints with `client_id="mock-client"`, `client_secret="mock-secret"`. Requires feature `oauth` (default).

**`ServerBuilder::build() -> Result<MockServer, _>`** — Async. Validates all fixtures and starts the server. Server shuts down when `MockServer` is dropped.

**`MockServer::url() -> String`** — Returns the server base URL, e.g. `"http://127.0.0.1:PORT"`.

**`Fixture::new()`** — Creates a match-all fixture with no response set. Configure via builder methods or struct literal.

**`Fixture::match_user_message(pattern: &str)`** — Substring match on the last user message content.

**`Fixture::respond_with_content(content: &str)`** — Sets a text response. Mutually exclusive with `respond_with_tool_calls`.

**`Fixture::respond_with_tool_calls(tool_calls: Vec<ToolCall>)`** — Sets a tool-use response. `ToolCall.arguments` must be a JSON object.
