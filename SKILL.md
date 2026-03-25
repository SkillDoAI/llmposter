---
name: llmposter
description: A local mock LLM server for integration-testing Rust code against OpenAI, Anthropic, Gemini, and Responses APIs — configure fixture responses, SSE streaming, bearer-token auth, and fault injection without live network access.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---

## Imports

```rust
use llmposter::{Fixture, Provider, ServerBuilder};
use llmposter::fixture::{FailureConfig, FixtureResponse, StreamingConfig, ToolCall};
use llmposter::auth::{AuthState, TokenStatus};
use llmposter::cli::{Cli, run_with_output};
// OAuth support (requires default `oauth` feature, MSRV = Rust 1.89):
use llmposter::OAuthConfig;
```

## Core Patterns

### Minimal fixture server

```rust
#[tokio::test]
async fn test_minimal_fixture_server() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("Hello from mock!"))
        .build()
        .await
        .unwrap();
    let base_url = server.url(); // e.g. "http://127.0.0.1:PORT"
    let _ = base_url;
}
```

### SSE streaming response

```rust
#[tokio::test]
async fn test_sse_streaming() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("Hello world!")
                // with_streaming(inter_chunk_latency_ms, chars_per_delta_frame)
                .with_streaming(Some(10), Some(5)),
        )
        .build()
        .await
        .unwrap();
    let base_url = server.url();
    // Point your LLM client's base_url here and set "stream": true in the request body.
    let _ = base_url;
}
```

### Fault injection

```rust
#[tokio::test]
async fn test_fault_injection() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .for_provider(Provider::Gemini)
                .with_error(429, "rate limited"),
        )
        .build()
        .await?;
    let base_url = server.url();
    // Your HTTP client will receive a 429 response with provider-specific rate-limit headers.
    let _ = base_url;
    Ok(())
}
```

### Bearer-token auth validation

```rust
let server = ServerBuilder::new()
    .with_auth(true)
    .with_bearer_token("test-secret-key")
    .fixture(Fixture::new().respond_with_content("authenticated"))
    .build()
    .await?;
```

### Token expiry after N uses

```rust
let server = ServerBuilder::new()
    .with_auth(true)
    .with_bearer_token_uses("one-time-token", 1)
    .fixture(Fixture::new().respond_with_content("ok"))
    .build()
    .await?;
// First request succeeds; second request returns 401 (token exhausted).
```

### Tool call fixture

```rust
let server = ServerBuilder::new()
    .fixture(
        Fixture::new()
            .for_provider(Provider::OpenAI)
            .respond_with_tool_calls(vec![ToolCall {
                name: "get_weather".into(),
                arguments: serde_json::json!({"location": "NYC"}),
            }]),
    )
    .build()
    .await?;
```

## Configuration

| Method | Parameters | Description |
|--------|------------|-------------|
| `fixture(f)` | `Fixture` | Add a single fixture |
| `fixtures(v)` | `Vec<Fixture>` | Add multiple fixtures at once |
| `bind(addr)` | `&str` | Bind address, e.g. `"127.0.0.1:0"` (`:0` for random port) |
| `verbose(v)` | `bool` | Enable request/response logging |
| `with_auth(enabled)` | `bool` | Enable bearer-token auth enforcement |
| `with_bearer_token(token)` | `&str` | Register a bearer token (unlimited uses) |
| `with_bearer_token_uses(token, n)` | `&str, u64` | Register a bearer token that expires after `n` uses; request N+1 returns 401 |
| `with_oauth(config)` *(oauth feature)* | `OAuthConfig` | Enable OAuth flow with custom credentials |
| `with_oauth_defaults()` *(oauth feature)* | — | Enable OAuth with default mock credentials (`client_id="mock-client"`, `client_secret="mock-secret"`) |
| `load_yaml(path)` | `&Path` | Load fixtures from a YAML file |
| `load_yaml_dir(dir)` | `&Path` | Load all YAML fixtures from a directory |

Provider scoping is set per-fixture via `Fixture::for_provider(provider)`.
Provider variants: `Provider::OpenAI`, `Provider::Anthropic`, `Provider::Gemini`, `Provider::Responses`.
Unmatched provider requests return 404.

**OAuth note:** `MockServer::oauth_url()` is referenced in behavioral semantics for the OAuth token flow but does not appear in the extracted API surface — it may be feature-gated and absent from extraction.

## Validation

`ServerBuilder::build()` calls `Fixture::validate()` on every fixture before starting. The build returns `Err` (server fails to start) for any of:

- `with_error` status outside 400–599
- empty regex pattern in `match_user_message` or `match_model`
- empty substring pattern in `match_user_message` or `match_model`
- blank tool name in a `ToolCall`
- non-object `arguments` in a `ToolCall` (e.g. a JSON string or array)
- DFA size > 1 MB (regex too large to compile)
- unknown YAML fields (via `#[serde(deny_unknown_fields)]`)

## Response Headers

Every LLM endpoint response includes a deterministic `x-request-id` header:

```
x-request-id: req-llmposter-{N}
```

`N` increments per request within the server instance and is assertable in snapshot tests.

When a fixture returns a 429 error, provider-specific rate-limit headers are automatically added:

| Provider | Headers |
|----------|---------|
| OpenAI / Responses | `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, `x-ratelimit-reset-requests` |
| Anthropic | `anthropic-ratelimit-requests-limit`, `anthropic-ratelimit-requests-remaining`, `anthropic-ratelimit-requests-reset` |
| Gemini | `retry-after` |

## Streaming Notes

`FailureConfig::truncate_after_frames` drops the SSE stream mid-connection after N frames (not a clean close). The YAML alias `truncate_after_chunks` is deprecated — use `truncate_after_frames` in new fixtures.

Responses API streaming uses nested `response` envelopes with `sequence_number` and correlation fields. `response.in_progress` events are sent; `response.done` is **not** sent (removed in 0.3.4).

Tool-call IDs are globally unique within a server instance via a server-wide counter — no collisions across turns or parallel requests.

## Pitfalls

### Wrong: reusing a server across tests

```rust
// Shared server state leaks between tests — fixture is consumed on first hit
static SERVER: Lazy<Server> = Lazy::new(|| ServerBuilder::new()...);
```

### Right: create a new server per test

```rust
#[tokio::test]
async fn test_openai_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .for_provider(Provider::OpenAI)
                .respond_with_content("ok"),
        )
        .build().await.unwrap();
    // test against server.url()
}
```

### Wrong: hardcoding localhost port in client config

```rust
let client = MyLlmClient::new("http://localhost:8080"); // port may be taken
```

### Right: use the dynamic base URL from the running server

```rust
let server = ServerBuilder::new()...build().await?;
let client = MyLlmClient::new(&server.url()); // port allocated at runtime
```

### Wrong: enabling auth without passing the token to the client

```rust
let server = ServerBuilder::new()
    .with_auth(true)
    .with_bearer_token("secret")
    ...build().await?;
let client = MyLlmClient::new(&server.url()); // no bearer token → 401
```

### Right: pass the matching token to the client under test

```rust
let client = MyLlmClient::new(&server.url()).bearer("secret");
```

### Wrong: binding an IPv6 address on a host without IPv6 support

Attempting `bind("[::1]:0")` when IPv6 is unavailable causes server start to fail gracefully; the test is skipped rather than panicked.

### Wrong: using a string literal for tool-call arguments

```rust
ToolCall {
    name: "get_weather".into(),
    arguments: r#"{"location":"NYC"}"#.into(), // creates Value::String, not Value::Object → rejected at fixture load
}
```

### Right: use `serde_json::json!` to produce a JSON object

```rust
ToolCall {
    name: "get_weather".into(),
    arguments: serde_json::json!({"location": "NYC"}),
}
```

## References

- [llmposter crate](https://crates.io/crates/llmposter)
- [OpenAI API reference](https://platform.openai.com/docs/api-reference)
- [Anthropic API reference](https://docs.anthropic.com/reference)
- [Gemini API reference](https://ai.google.dev/api)
