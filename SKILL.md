---
name: llmposter
description: An in-process mock LLM server for Rust integration tests — serves OpenAI, Anthropic, Gemini, and Responses API endpoints with configurable fixtures, bearer-token auth, SSE streaming, and network failure injection.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---

## Imports

Prefer crate-root re-exports for all common types:

```rust
use llmposter::{
    FailureConfig, Fixture, Provider,
    ServerBuilder, ToolCall,
};
```

Add to `Cargo.toml`:

```toml
[dev-dependencies]
llmposter = "0.4.0"
# OAuth support:
# llmposter = { version = "0.4.0", features = ["oauth"] }
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.13", features = ["json"], default-features = false }
serde_json = "1"
```

## Core Patterns

### Start a mock server with a simple fixture ✅ Current

```rust
mod basic_server {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_basic_response() {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("hello")
                    .respond_with_content("Hi from Claude mock!"),
            )
            .build()
            .await
            .unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "hello world"}]
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status().as_u16(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["content"][0]["text"], "Hi from Claude mock!");
        assert_eq!(body["stop_reason"], "end_turn");
        assert!(body["id"].as_str().unwrap().starts_with("msg-llmposter-"));
    }
}
```

`ServerBuilder` binds to `127.0.0.1:0` (random port) by default. `MockServer` is RAII — the server stops when dropped. Unmatched requests return HTTP 404 with `{"error": {"message": "No fixture matched ..."}}`.

### Fixture matching and tool-call responses ✅ Current

```rust
mod tool_call_server {
    use llmposter::{Fixture, Provider, ServerBuilder, ToolCall};

    #[tokio::test]
    async fn test_tool_use_and_provider_filter() {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("weather")
                    .match_model("claude-sonnet")
                    .for_provider(Provider::Anthropic)
                    .respond_with_tool_calls(vec![ToolCall {
                        name: "get_weather".to_string(),
                        arguments: serde_json::json!({"location": "London", "unit": "celsius"}),
                    }]),
            )
            .build()
            .await
            .unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "what is the weather in London?"}]
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status().as_u16(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["name"], "get_weather");
        assert_eq!(body["content"][0]["input"]["location"], "London");
        assert_eq!(body["content"][0]["id"], "toolu_llmposter_1");
    }
}
```

`for_provider` scopes a fixture to one provider endpoint. Omit it to match all providers. First-match-wins: place specific fixtures before broad ones.

When a fixture responds with multiple tool calls, each call receives a globally unique ID drawn from a server-wide counter — e.g. `"toolu_llmposter_1"`, `"toolu_llmposter_2"`, `"toolu_llmposter_3"`. The counter increments across all requests and all fixtures, so IDs never collide even in multi-turn conversations. Do not hard-code an expected ID like `"toolu_llmposter_1"` if the test server has already served earlier requests; use a prefix assertion instead:

```rust
// Multiple tool calls in one response — IDs are distinct and globally ordered
let body: serde_json::Value = resp.json().await.unwrap();
let ids: Vec<&str> = body["content"]
    .as_array()
    .unwrap()
    .iter()
    .map(|c| c["id"].as_str().unwrap())
    .collect();
// All IDs share the "toolu_llmposter_" prefix and are unique
let unique: std::collections::HashSet<_> = ids.iter().collect();
assert_eq!(unique.len(), ids.len(), "tool call IDs must be distinct");
for id in &ids {
    assert!(id.starts_with("toolu_llmposter_"));
}
```

### SSE streaming responses ✅ Current

```rust
mod streaming_server {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_streaming_response() {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("Hello world from streaming mock")
                    // latency per chunk (ms), chunk_size in chars
                    .with_streaming(Some(0), Some(5)),
            )
            .build()
            .await
            .unwrap();

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
            .await
            .unwrap();

        assert_eq!(resp.status().as_u16(), 200);
        let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
        assert!(ct.contains("text/event-stream"));
        let body = resp.text().await.unwrap();
        assert!(body.contains("event: message_start"));
        assert!(body.contains("event: content_block_delta"));
        assert!(body.contains("event: message_stop"));
    }
}
```

`with_streaming(latency, chunk_size)` — first arg is per-chunk delay in milliseconds, second is characters per SSE frame. Both accept `None` for defaults. Applies to both text and tool-call streams.

### Network failure injection ✅ Current

```rust
mod failure_injection {
    use llmposter::{FailureConfig, Fixture, ServerBuilder};
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn test_latency_and_http_error() {
        let server = ServerBuilder::new()
            // HTTP error — use `with_error`, not `with_failure`
            .fixture(
                Fixture::new()
                    .match_user_message("rate limit")
                    .with_error(429, "Rate limit exceeded"),
            )
            // Simulated latency on otherwise-valid response
            .fixture(
                Fixture::new()
                    .respond_with_content("delayed response")
                    .with_failure(FailureConfig {
                        latency_ms: Some(200),
                        ..FailureConfig::default()
                    }),
            )
            .build()
            .await
            .unwrap();

        let client = reqwest::Client::new();

        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "rate limit test"}]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 429);

        let start = Instant::now();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "hello"}]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        assert!(start.elapsed() >= Duration::from_millis(180));
    }
}
```

`FailureConfig` simulates network problems (latency, corrupt body, stream truncation, abrupt disconnect). `with_error` sets an HTTP error status code. The two are mutually exclusive per fixture.

`FailureConfig` fields and their observable effects:

| Field | Type | Effect |
|-------|------|--------|
| `latency_ms` | `Option<u64>` | Delays the response by this many milliseconds |
| `corrupt_body` | `Option<bool>` | Replaces the response body with overloaded text instead of the normal content |
| `truncate_after_frames` | `Option<u32>` | Streams N SSE frames then drops the connection; the client observes a truncated stream, not an HTTP error. YAML alias: `truncate_after_chunks` (deprecated) |
| `disconnect_after_ms` | `Option<u64>` | Drops the TCP connection after the given delay in milliseconds |

### Bearer token authentication ✅ Current

```rust
mod auth_server {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::test]
    async fn test_bearer_auth_and_expiry() {
        let server = ServerBuilder::new()
            .with_bearer_token("unlimited-token")         // unlimited uses; implicitly enables auth
            .with_bearer_token_uses("expiring-token", 2)  // expires after exactly 2 requests
            .fixture(Fixture::new().respond_with_content("ok"))
            .build()
            .await
            .unwrap();

        let client = reqwest::Client::new();
        let payload = serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}]
        });

        // No token → 401
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&payload)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 401);

        // Valid unlimited token → 200
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .bearer_auth("unlimited-token")
            .json(&payload)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);

        // Expiring token: first two uses succeed, third → 401
        for _ in 0..2 {
            let r = client
                .post(format!("{}/v1/messages", server.url()))
                .bearer_auth("expiring-token")
                .json(&payload)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status().as_u16(), 200);
        }
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .bearer_auth("expiring-token")
            .json(&payload)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 401);
    }
}
```

`with_bearer_token_uses` expires a token after exactly `max_uses` requests — useful for testing token refresh flows without real-time clocks. OAuth-issued tokens are automatically accepted by the LLM endpoint auth layer.

## Configuration

### ServerBuilder defaults

| Option | Default | Override |
|--------|---------|----------|
| Bind address | `127.0.0.1:0` (random port) | `.bind("0.0.0.0:8080")` |
| Auth enforcement | disabled | `.with_auth(true)` |
| Verbose logging | disabled | `.verbose(true)` |
| Fixtures | none | `.fixture(f)` / `.fixtures(vec)` |

### YAML fixture format

```yaml
# fixtures/mock.yaml
fixtures:
  - match:
      user_message: "stock price of AAPL"   # specific — must come first
    response:
      content: "AAPL is $150.42"
  - match:
      user_message: "stock"                  # broad catch — comes after specific
    response:
      content: "Generic stock response"
  - match:
      model: "bad-model"
    error:
      status: 429
      message: "Rate limit exceeded"
```

```rust
mod yaml_fixtures {
    use llmposter::ServerBuilder;

    #[tokio::test]
    async fn test_yaml_fixtures() {
        let server = ServerBuilder::new()
            .load_yaml(std::path::Path::new("fixtures/mock.yaml"))
            .unwrap()
            .build()
            .await
            .unwrap();

        let _ = server.url(); // "http://127.0.0.1:{port}"
    }
}
```

`load_yaml` and `load_yaml_dir` return `Result<ServerBuilder, _>` — call `?` or `.unwrap()` before chaining `.build()`. Use `load_yaml_dir` to load all `.yaml` files from a directory.

### OAuth feature flag

```toml
[dev-dependencies]
llmposter = { version = "0.4.0", features = ["oauth"] }
```

```rust
mod oauth_server {
    #[cfg(feature = "oauth")]
    use llmposter::{Fixture, ServerBuilder};

    #[cfg(feature = "oauth")]
    #[tokio::test]
    async fn test_oauth_defaults() {
        // client_id = "mock-client", client_secret = "mock-secret"
        let server = ServerBuilder::new()
            .with_oauth_defaults()
            .fixture(Fixture::new().respond_with_content("oauth ok"))
            .build()
            .await
            .unwrap();

        let _ = server.url();
    }
}
```

OAuth-issued tokens are accepted by the LLM endpoint auth layer without separate configuration. MSRV is **1.89** when using the `oauth` feature (required by `oauth-mock`).

### Response headers

Every response includes `x-request-id: req-llmposter-{N}` (server-wide counter). Rate limit headers are provider-specific:

```text
# OpenAI / Responses API (429 only):
x-ratelimit-limit-requests: 60
x-ratelimit-remaining-requests: 0
x-ratelimit-reset-requests: 1s

# Anthropic (429 only):
anthropic-ratelimit-requests-limit: 60
anthropic-ratelimit-requests-remaining: 0
anthropic-ratelimit-requests-reset: <RFC 3339 timestamp>

# Gemini (429 only):
retry-after: 1
```

Requests exceeding the server body size limit return HTTP 413. The `x-request-id` header is present on 413 responses. No fixture matching is attempted — the rejection occurs before routing.

## Pitfalls

### Using `failure` to return an HTTP error status

**Wrong** — `FailureConfig` has no `status` field; this YAML is rejected at load time:
```yaml
- match:
    user_message: "overloaded"
  failure:
    status: 503
    message: "Service overloaded"
```

**Right** — use `error` for HTTP error status codes:
```yaml
- match:
    user_message: "overloaded"
  error:
    status: 503
    message: "Service overloaded"
```

`failure` simulates network-layer problems (latency, corrupt body, truncated streams, disconnect) on an otherwise-valid 200 response. `error` sets an HTTP error status. They are mutually exclusive and serve different testing purposes.

### Catch-all fixture placed before specific fixture

**Wrong** — `"stock"` matches `"stock price of AAPL"` first; the specific fixture is never reached:
```rust
ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("stock").respond_with_content("generic"))
    .fixture(Fixture::new().match_user_message("stock price of AAPL").respond_with_content("$150.42"))
```

**Right** — specific patterns must appear before broad ones:
```rust
ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("stock price of AAPL").respond_with_content("$150.42"))
    .fixture(Fixture::new().match_user_message("stock").respond_with_content("generic"))
```

First-match-wins: the server iterates fixtures in registration order and returns on the first match.

### Tool-call arguments as a non-object type

**Wrong** — a string or array is rejected by `Fixture::validate()` at server startup:
```rust
ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!("San Francisco"),  // string — invalid
}
```

**Right** — arguments must always be a JSON object:
```rust
ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!({"location": "San Francisco", "unit": "celsius"}),
}
```

Validation is called automatically inside `ServerBuilder::build()`, so the error surfaces at test startup, not at request time.

### Adding a provider path prefix to the base URL

**Wrong** — llmposter serves real provider paths directly; adding a prefix breaks all requests:
```text
base_url = "http://127.0.0.1:8080/anthropic"
```

**Right** — point clients at the bare base URL, then append the real API path:
```text
base_url = "http://127.0.0.1:8080"
// POST http://127.0.0.1:8080/v1/messages        (Anthropic)
// POST http://127.0.0.1:8080/v1/chat/completions (OpenAI)
```

Provider routes are globally unique and served with no prefix.

### Empty match patterns

**Wrong** — empty substring matches every request and are rejected at load time:
```yaml
- match:
    user_message: ""
  response:
    content: "default"
```

**Right** — omit `user_message` entirely to create a catch-all fixture:
```yaml
- response:
    content: "default"
```

The server rejects empty substrings and empty regex patterns (`regex: ""`) during `Fixture::validate()` to prevent silent catch-all behavior.

### Blank tool name

**Wrong** — a blank tool name is rejected by `Fixture::validate()` at server startup:
```rust
ToolCall {
    name: "".to_string(),
    arguments: serde_json::json!({"location": "London"}),
}
```

**Right** — tool names must be non-empty strings:
```rust
ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!({"location": "London"}),
}
```

### HTTP error status outside 400–599

**Wrong** — status codes outside the 400–599 range are rejected by `Fixture::validate()` at server startup:
```yaml
- match:
    user_message: "redirect"
  error:
    status: 301
    message: "Moved Permanently"
```

**Right** — `error.status` must be 400–599:
```yaml
- match:
    user_message: "rate limit"
  error:
    status: 429
    message: "Rate limit exceeded"
```

### Unrecognized YAML keys rejected at load time

All fixture structs (`Fixture`, `FailureConfig`, `StreamingConfig`, `ToolCall`, `FixtureMatch`, `FixtureError`, `FixtureResponse`) are annotated `#[serde(deny_unknown_fields)]`. Any YAML key that does not correspond to a known field causes the entire file to be rejected at load time — no partial loading.

**Wrong** — `retries` is not a field on `FailureConfig`; the file is rejected:
```yaml
- match:
    user_message: "flaky"
  failure:
    latency_ms: 100
    retries: 3        # unknown field — rejected
```

**Right** — use only documented fields:
```yaml
- match:
    user_message: "flaky"
  failure:
    latency_ms: 100
```

This catches YAML typos (e.g. `mesage` instead of `message`, `content_type` instead of `content`) immediately at server startup rather than silently producing unexpected behavior at request time.

### Regex pattern exceeding DFA size limit

Regex patterns whose compiled DFA exceeds 1 MB are rejected at load time by `Fixture::validate()`. This prevents OOM from pathological patterns.

**Wrong** — combinatorial nested repetition produces a DFA that exceeds the limit:
```yaml
- match:
    user_message:
      regex: "(a{1,100}){1,100}"
  response:
    content: "ok"
```

**Right** — use straightforward patterns; avoid unbounded nested repetition:
```yaml
- match:
    user_message:
      regex: "a{1,100}"
  response:
    content: "ok"
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3.x

### v0.3.3 → v0.3.4 — Responses API breaking changes

**SSE stream shape changed.** The `response.done` event was removed (non-spec). Streams now emit `response.in_progress` events. The data object has a top-level `sequence_number` field alongside a nested `response` envelope with correlation fields.

Before:
```text
event: response.done
data: {"type": "response.done", ...}
```

After:
```text
event: response.in_progress
data: {"type": "response.in_progress", "response": {...}, "sequence_number": 1}
```

Update stream assertions: remove `response.done` checks, add `response.in_progress` checks.

**Error body shape changed** to match the real OpenAI format. `code` is now `Option<String>` (was an integer), `param` is always present as `null`, and `type` is an error-category string. Update deserialization structs and assertions accordingly.

### v0.3.x → v0.4.0 — MSRV bump

MSRV raised to **1.89**, required by the `oauth-mock` dependency. Update your toolchain before upgrading:

```bash
rustup update stable
```

No existing public API was removed. `with_auth`, `with_bearer_token`, `with_bearer_token_uses`, `with_oauth`, and `with_oauth_defaults` are purely additive.

## API Reference

**`ServerBuilder::new()`** — Creates a builder with defaults: `127.0.0.1:0` (random port), auth disabled, verbose disabled.

**`ServerBuilder::fixture(f: Fixture)`** — Appends one fixture. First-match-wins ordering — most specific fixtures must appear first.

**`ServerBuilder::fixtures(fixtures: Vec<Fixture>)`** — Appends multiple fixtures in one call.

**`ServerBuilder::bind(addr: &str)`** — Sets bind address; accepts `"host:port"` or `"[ipv6]:port"`.

**`ServerBuilder::verbose(v: bool)`** — Enables server-side debug logging; does not change response shapes.

**`ServerBuilder::with_auth(enabled: bool)`** — Enables auth enforcement. Without registered tokens, all requests return 401.

**`ServerBuilder::with_bearer_token(token: &str)`** — Registers an unlimited-use bearer token; implicitly enables auth.

**`ServerBuilder::with_bearer_token_uses(token: &str, max_uses: u64)`** — Registers a token that expires after exactly `max_uses` requests; enables auth.

**`ServerBuilder::load_yaml(path: &Path) -> Result<ServerBuilder, _>`** — Loads fixtures from a YAML file. Returns `Result` — `?` or `.unwrap()` required before `.build()`.

**`ServerBuilder::build() -> Result<MockServer, _>`** *(async)* — Validates all fixtures, binds the TCP listener, and starts the server. Fails if any fixture is invalid.

**`MockServer::url() -> String`** — Returns the server base URL, e.g. `"http://127.0.0.1:52341"`.

**`Fixture::new()`** — Creates an empty fixture matching all requests. Chain builder methods to constrain it.

**`Fixture::match_user_message(pattern: &str)`** — Substring match on user message content.

**`Fixture::respond_with_content(content: &str)`** — Sets a plain-text response; clears any prior `tool_calls` on the response.

**`Fixture::with_streaming(latency: Option<u64>, chunk_size: Option<usize>)`** — Enables SSE streaming; `latency` is per-chunk delay in ms, `chunk_size` is characters per SSE frame.
