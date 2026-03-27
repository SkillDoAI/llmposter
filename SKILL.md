---
name: llmposter
description: Rust library for mocking LLM API servers (Anthropic and OpenAI-compatible) in tests with configurable fixtures, failure injection, and streaming.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---

## Imports

Add to `Cargo.toml`:

```toml
[dev-dependencies]
llmposter = "0.4.0"
tokio = { version = "1", features = ["full"] }
serde_json = "1"
reqwest = { version = "0.13", default-features = false, features = ["json"] }

# OAuth feature (optional):
# llmposter = { version = "0.4.0", features = ["oauth"] }
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

### SSE streaming response

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn test_streaming_response() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("stream this")
                .respond_with_content("Streaming content here")
                // with_streaming(latency, chunk_size)
                .with_streaming(Some(0), Some(5)),  // enabled, no delay, 5 chars per frame
        )
        .build()
        .await?;

    let base_url = server.url();
    // Point your LLM client's base_url here and set "stream": true in the request body.
    // The server returns Content-Type: text/event-stream.
    // Anthropic endpoint (/v1/messages) events:
    //   message_start, content_block_start, content_block_delta,
    //   content_block_stop, message_delta, message_stop
    // OpenAI/Responses API endpoints use a different event format.
    let _ = base_url;
    Ok(())
}
```

Omitting `with_streaming` leaves streaming disabled. `Some(0)` enables streaming with no inter-chunk delay. Total streaming time ≈ `ceil(content_len / chunk_size) × latency_ms`.

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
// Cargo.toml: llmposter = { version = "0.4.0", features = ["oauth"] }
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

---

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

---

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

---

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

---

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

---

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

---

### Wrong: Regex pattern with DFA exceeding 1 MB

Very complex patterns with large alternation sets can produce a compiled DFA exceeding 1 MB.

### Right: Keep regex patterns simple

The `regex` crate's DFA size is capped at 1 MB per pattern. Patterns that exceed this limit are rejected at fixture validation time to prevent out-of-memory errors. Simplify or split overly complex alternation patterns.

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3

**MSRV bump (v0.3.x → v0.4.0)**: Minimum supported Rust version is now **1.89**, required by the `oauth-mock` dependency. Update `rust-toolchain.toml` and CI matrix accordingly.

**Auth APIs added (v0.4.0)**: `with_bearer_token`, `with_bearer_token_uses`, `with_auth`, `with_oauth`, `with_oauth_defaults` are new in v0.4.0. No migration required for existing tests that do not use auth.

**Responses API SSE breaking change (v0.3.3 → v0.3.4)**: The streaming event structure for the OpenAI Responses API changed. Events now use a nested `response` envelope, include a `sequence_number` field, and use `response.in_progress` instead of `response.done`.

**Error response JSON (v0.3.3 → v0.3.4)**: The `code` field is now a `String` (was absent or integer). A `param` field (`null` by default) is always present.

**Fixture YAML strict validation (v0.3.4 → v0.3.5)**: All fixture structs now use `deny_unknown_fields`. Typos in YAML field names that were previously silently ignored now cause a load-time error.

**Tool-call ID format (v0.3.5 → v0.3.6)**: IDs are generated from a server-wide atomic counter. Use prefix + uniqueness checks instead of hard-coded ID values in assertions.

## API Reference

**`ServerBuilder::new()`** — Creates a new builder. Defaults to OS-assigned port on `127.0.0.1`.

**`ServerBuilder::fixture(f: Fixture)`** — Appends one fixture. First match wins.

**`ServerBuilder::fixtures(fixtures: Vec<Fixture>)`** — Appends multiple fixtures in one call.

**`ServerBuilder::bind(addr: &str)`** — Overrides the bind address, e.g. `"127.0.0.1:9090"`.

**`ServerBuilder::verbose(v: bool)`** — Enables request/match logging to stderr.

**`ServerBuilder::with_auth(enabled: bool)`** — Explicitly enables or disables bearer token enforcement.

**`ServerBuilder::with_bearer_token(token: &str)`** — Enables auth and registers a token with unlimited uses.

**`ServerBuilder::with_bearer_token_uses(token: &str, max_uses: u64)`** — Token that expires after `max_uses` requests; returns HTTP 401 once exhausted.

**`ServerBuilder::with_oauth(config: OAuthConfig)`** *(feature: oauth)* — Starts an embedded OAuth mock server.

**`ServerBuilder::with_oauth_defaults()`** *(feature: oauth)* — Shorthand with `client_id="mock-client"`, `client_secret="mock-secret"`.

**`ServerBuilder::load_yaml(path: &std::path::Path) -> Result<ServerBuilder, _>`** — Loads fixtures from a single YAML file. Returns `Err` if the file cannot be read or parsed.

**`ServerBuilder::load_yaml_dir(dir: &std::path::Path) -> Result<ServerBuilder, _>`** — Loads all `*.yaml` files from a directory. Returns `Err` on read or parse failure.

**`ServerBuilder::build()`** — *async*. Validates fixtures, binds server, returns `Result<MockServer, _>`. Server shuts down on drop.

**`MockServer::url()`** — Returns base URL, e.g. `"http://127.0.0.1:54321"`.

**`Fixture::new()`** — Constructs a fixture with all fields `None`.

**`Fixture::match_user_message(pattern: &str)`** — Substring match on last `user` message. Non-empty; empty patterns rejected at validation.

**`Fixture::match_model(pattern: &str)`** — Substring match on the requested model name.

**`Fixture::respond_with_content(content: &str)`** — Sets a plain-text assistant response. Mutually exclusive with `respond_with_tool_calls`.

**`Fixture::respond_with_tool_calls(tool_calls: Vec<ToolCall>)`** — Sets a tool-call response. Mutually exclusive with `respond_with_content`. `ToolCall::arguments` must be a JSON object.

**`Fixture::with_error(status: u16, message: &str)`** — Returns an HTTP error response with the given status code and message body. Status must be 400–599; codes outside that range are rejected at validation. Mutually exclusive with `respond_with_content`, `respond_with_tool_calls`, and `with_failure`. Response body is provider-specific: OpenAI/Responses API returns `{ "error": { "type": "...", "code": "...", "param": null, "message": "..." } }`; Anthropic returns `{ "type": "error", "error": { "type": "...", "message": "..." } }`.

**`Fixture::with_failure(failure: FailureConfig)`** — Attaches a failure configuration. Requires a response (`respond_with_content` or `respond_with_tool_calls`) to also be set.

**`Fixture::with_streaming(latency: Option<u64>, chunk_size: Option<usize>)`** — Enables SSE streaming. `latency` is per-chunk delay in milliseconds; `chunk_size` is characters per SSE frame. Both may be `None` to use defaults.

**`Fixture::for_provider(provider: Provider)`** — Restricts the fixture to one provider endpoint. An `Anthropic`-pinned fixture is invisible at `/v1/chat/completions` and vice versa. Omit to match all providers.

**`Fixture::with_stop_reason(reason: &str)`** — Overrides the `stop_reason` field in Anthropic-format responses (e.g. `"max_tokens"`).

**`Fixture::with_finish_reason(reason: &str)`** — Overrides the `finish_reason` field in OpenAI-format responses (e.g. `"length"`).

**`Fixture::validate(&mut self) -> Result<(), String>`** — Validates fixture invariants and pre-compiles regex patterns. Called automatically by `ServerBuilder::build()`; invoke directly only when constructing fixtures outside a builder.

**`Provider`** — Enum restricting a fixture to one endpoint: `Provider::OpenAI` (`/v1/chat/completions`), `Provider::Anthropic` (`/v1/messages`), `Provider::Gemini`, `Provider::Responses`. Use with `Fixture::for_provider`.

**`FailureConfig`** — Struct controlling fault injection. Fields: `latency_ms: Option<u64>` (delay before response in ms), `corrupt_body: Option<bool>` (send malformed JSON), `truncate_after_frames: Option<u32>` (stop SSE stream after N frames), `disconnect_after_ms: Option<u64>` (drop TCP connection after N ms). Implements `Default`; combine fields freely.

**`StreamingConfig`** — Struct embedded in `Fixture` when `with_streaming` is called. Fields: `latency: Option<u64>` (per-chunk delay in ms), `chunk_size: Option<usize>` (characters per SSE frame).

**`ToolCall`** — Struct for tool-call responses. Fields: `name: String`, `arguments: serde_json::Value` (must be a JSON object).

**`AuthState`** — Thread-safe bearer token store with use-counting and deny-listing. Re-exported at crate root. Constructed via `AuthState::new()`.

**`AuthState::add_token(token: &str, max_uses: Option<u64>)`** — Registers a token. `None` means unlimited uses. Clears deny-list entry if the token was previously exhausted.

**`AuthState::check_and_use(token: &str) -> TokenStatus`** — Validates and consumes one use of a token.

**`AuthState::revoke(token: &str)`** — Atomically removes a token from the allow-list and adds it to the deny-list.

**`TokenStatus`** — Enum returned by `AuthState::check_and_use`: `Valid` (accepted and use consumed), `Exhausted` (deny-listed or use count reached zero), `Unknown` (token not registered).

**`OAuthConfig`** *(feature: oauth)* — OAuth configuration struct. Fields: `client_id: String`, `client_secret: String`, `redirect_uris: Vec<String>`, `scopes: Vec<String>`. Default: `client_id="mock-client"`, `client_secret="mock-secret"`, `redirect_uris=["https://example.com/callback"]`, `scopes=["openid","profile","email"]`.
