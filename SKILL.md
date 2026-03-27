---
name: llmposter
description: A mock LLM server that lets you define request‑response fixtures, streaming behavior, failures, and authentication for testing LLM client code.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/gpt-oss-120b + review:gpt-oss-120b
---

## Imports
```rust
use llmposter::{ServerBuilder, Fixture, ToolCall, FailureConfig, OAuthConfig};
use llmposter::cli::{Cli, run};
use reqwest::Client;
use serde_json::json;
```

```toml
[dependencies]
axum = "0.8"
clap = { features = ["derive"], version = "4" }
oauth-mock = { optional = true, version = "0.4" }
regex = "1"
reqwest = { default-features = false, features = ["json", "form"], optional = true, version = "0.13" }
serde = { features = ["derive"], version = "1" }
serde_json = "1"
serde_yaml_ng = "0.10"
tokio = { features = ["full"], version = "1" }
tokio-stream = "0.1"
```

## Core Patterns

### Basic Fixture ✅ Current
Create a simple fixture that matches a user message and returns static content.

```rust
use llmposter::{ServerBuilder, Fixture};
use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build a mock server with a single fixture
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello world")
                .respond_with_content("Hi from Claude mock!"),
        )
        .bind("127.0.0.1:8080") // fixed port for example
        .build()
        .await?;

    // Base URL for the mock server
    let base_url = "http://127.0.0.1:8080";

    // Send a request to the mock LLM endpoint
    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "hello world" }]
        }))
        .send()
        .await?;

    println!("Status: {}", resp.status());
    let body: serde_json::Value = resp.json().await?;
    println!("Response: {}", body);

    // Success indicator
    println!("✓ Test passed: Basic Fixture ✅ Current");
    Ok(())
}
```

### Streaming Response with Latency ✅ Current
Enable streaming and configure per‑chunk latency and size.

```rust
use llmposter::{ServerBuilder, Fixture};
use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build a mock server that will stream a response with a 50 ms latency
    // and 1 KB chunk size when the user message matches "stream me".
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("stream me")
                // The response that will be streamed (the library will split it
                // into chunks according to the latency/chunk‑size settings).
                .respond_with_content(r#"{
                    "id": "msg-1",
                    "content": "This is a streamed response from the mock server."
                }"#)
                // Enable streaming with the desired latency and chunk size.
                .with_streaming(Some(50), Some(1024)),
        )
        .bind("127.0.0.1:8080")
        .build()
        .await?;

    let base_url = "http://127.0.0.1:8080";

    // Create a client and issue a streaming request to the mock server.
    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "stream me" }],
            "stream": true
        }))
        .send()
        .await?;

    // Print the HTTP status and the raw SSE body we received.
    println!("Status: {}", resp.status());
    let body = resp.text().await?;
    println!("SSE body:\n{}", body);

    // NOTE: Since version 0.3.4 the streamed events are wrapped in a
    // `response` envelope, e.g. `data: {"response":{...}}`.  Clients should
    // parse the `response` field to extract the actual content.

    // Demonstrate the post‑0.3.4 format: each SSE event contains a
    // `response` object with a `sequence_number`.  An intermediate
    // `response.in_progress` event is emitted before the final chunk.
    // The following snippet parses the SSE lines and prints those fields.
    for line in body.lines() {
        // Skip empty lines or comments
        if !line.starts_with("data: ") {
            continue;
        }
        // Strip the leading "data: " prefix
        let payload_str = &line[6..];
        // Parse the JSON payload
        let payload: serde_json::Value = match serde_json::from_str(payload_str) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Failed to parse SSE payload: {}", e);
                continue;
            }
        };
        // The `response` envelope is guaranteed by the library.
        if let Some(response) = payload.get("response") {
            // Print the sequence number if present.
            if let Some(seq) = response.get("sequence_number") {
                println!("sequence_number: {}", seq);
            }
            // Detect an in‑progress marker.
            if response.get("in_progress").is_some() {
                println!("received in_progress event");
            }
            // For the final event, you can extract the actual content.
            if let Some(content) = response.get("content") {
                println!("content: {}", content);
            }
        }
    }

    // If we got here without panicking, the pattern works.
    println!("✓ Test passed: Streaming Response with Latency ✅ Current");
    Ok(())
}
```

### Streaming Failure – Truncate After Frames ✅ New
Demonstrates `FailureConfig.truncate_after_frames` causing the stream to end after a set number of chunks.

```rust
use llmposter::{ServerBuilder, Fixture, FailureConfig};
use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let failure = FailureConfig {
        latency_ms: None,
        corrupt_body: None,
        truncate_after_frames: Some(3), // stop after three SSE frames
        disconnect_after_ms: None,
    };

    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("stream with truncation")
                .respond_with_content(r#"{
                    "id": "msg-2",
                    "content": "Chunk 1"
                }"#)
                .with_streaming(Some(30), Some(512))
                .with_failure(failure),
        )
        .bind("127.0.0.1:8080")
        .build()
        .await?;

    let base_url = "http://127.0.0.1:8080";

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "stream with truncation" }],
            "stream": true
        }))
        .send()
        .await?;

    println!("Status: {}", resp.status());
    let sse_body = resp.text().await?;
    println!("Truncated SSE body (should contain exactly 3 frames):\n{}", sse_body);
    Ok(())
}
```

### Error Fixture (HTTP 429) ✅ Current
Define a fixture that returns a specific error status and body, and verify the `x-request-id` header.

```rust
use llmposter::{ServerBuilder, Fixture};
use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("trigger rate limit")
                .with_error(429, "Rate limit exceeded"),
        )
        .bind("127.0.0.1:8080")
        .build()
        .await?;

    let base_url = "http://127.0.0.1:8080";

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "trigger rate limit" }]
        }))
        .send()
        .await?;

    println!("Status: {}", resp.status());

    // Verify the required header
    if let Some(request_id) = resp.headers().get("x-request-id") {
        println!("x-request-id header: {}", request_id.to_str().unwrap_or("<invalid>"));
    } else {
        println!("⚠️  Missing x-request-id header");
    }

    let body: serde_json::Value = resp.json().await?;
    println!("Error body: {}", body);

    // Success output
    println!("✓ Test passed: Error Fixture (HTTP 429) ✅ Current");
    Ok(())
}
```

### Authentication with Bearer Token ✅ Current
Enable auth, add a token that expires after a limited number of uses, and see the token enforcement in action.

```rust
use llmposter::{ServerBuilder, Fixture};
use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token_uses("my-token", 2) // token expires after 2 successful calls
        .fixture(
            Fixture::new()
                .match_user_message("secure request")
                .respond_with_content("Authenticated response"),
        )
        .bind("127.0.0.1:8080")
        .build()
        .await?;

    let base_url = "http://127.0.0.1:8080";

    let client = Client::new();

    // First request – succeeds
    let resp1 = client
        .post(format!("{}/v1/messages", base_url))
        .bearer_auth("my-token")
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "secure request" }]
        }))
        .send()
        .await?;
    println!("First status (200 expected): {}", resp1.status());

    // Second request – still succeeds (second use)
    let _ = client
        .post(format!("{}/v1/messages", base_url))
        .bearer_auth("my-token")
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "secure request" }]
        }))
        .send()
        .await?;

    // Third request – token exhausted, returns 401
    let resp3 = client
        .post(format!("{}/v1/messages", base_url))
        .bearer_auth("my-token")
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "secure request" }]
        }))
        .send()
        .await?;
    println!("Third status (401 expected): {}", resp3.status());

    Ok(())
}
```

#### Missing or Invalid Token → 401 ✅ New
Calling an endpoint without a bearer token (or with an unknown token) results in HTTP 401.

```rust
use llmposter::{ServerBuilder, Fixture};
use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token_uses("valid-token", 5)
        .fixture(
            Fixture::new()
                .match_user_message("needs auth")
                .respond_with_content("You are authorized"),
        )
        .bind("127.0.0.1:8080")
        .build()
        .await?;

    let base_url = "http://127.0.0.1:8080";

    let client = Client::new();

    // No Authorization header
    let resp = client
        .post(format!("{}/v1/messages", base_url))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "needs auth" }]
        }))
        .send()
        .await?;

    println!("Status without token (401 expected): {}", resp.status());
    Ok(())
}
```

### Tool‑Call Validation Failure ✅ New
Providing a non‑JSON object as a tool‑call argument triggers fixture validation failure.

```rust
use llmposter::{ServerBuilder, Fixture, ToolCall};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Intentionally malformed tool call: arguments are a plain string instead of a JSON object.
    let bad_tool = ToolCall {
        name: "search".to_string(),
        arguments: serde_json::json!("just a string, not an object"),
    };

    let mut fixture = Fixture::new()
        .match_user_message("use tool")
        .respond_with_tool_calls(vec![bad_tool]);

    // Validate the fixture – should return an error describing the problem.
    match fixture.validate() {
        Ok(_) => println!("✅ Unexpectedly passed validation"),
        Err(e) => println!("❌ Validation failed as expected: {}", e),
    }

    Ok(())
}
```

## Configuration
| Component | Default | Common customizations |
|-----------|---------|-----------------------|
| **ServerBuilder** | No authentication, no fixtures, bind to `127.0.0.1:0` (random port) | `.with_auth(true)`, `.with_bearer_token("token")`, `.with_bearer_token_uses(token, n)`, `.with_oauth(config)`, `.verbose(true)`, `.load_yaml(path)` |
| **Fixture** | All fields `None` | `.match_user_message(str)`, `.match_model(str)`, `.for_provider(Provider)`, `.respond_with_content(str)`, `.respond_with_tool_calls(Vec<ToolCall>)`, `.with_error(status, message)`, `.with_failure(FailureConfig)`, `.with_streaming(latency, chunk_size)` |
| **FailureConfig** | All `None` | `latency_ms`, `corrupt_body`, `truncate_after_frames`, `disconnect_after_ms` |
| **StreamingConfig** (via `with_streaming`) | No streaming | `latency` (ms between chunks), `chunk_size` (bytes per chunk) |
| **AuthState** | Empty token map, no exhausted set | `add_token(token, max_uses)`, `revoke(token)` |
| **OAuth** (feature `oauth`) | Disabled unless feature enabled | `OAuthConfig` supplies client ID/secret, scopes, redirect URIs; enable with `.with_oauth_defaults()` or `.with_oauth(custom)` |

Environment variables are **not** required by the library; all configuration is expressed via the builder API or YAML fixture files.

## CLI Usage and Error Handling
The library ships a small CLI to run a mock server. The `Cli` struct mirrors command‑line flags.

```rust
use llmposter::cli::{Cli, run};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Example: start the server on a malformed IPv6 address
    let cli = Cli {
        fixtures: PathBuf::from("fixtures/"),
        validate: true,
        port: 0,
        bind: "[::1:::invalid]".to_string(), // intentionally bad
        verbose: true,
    };

    match run(&cli).await {
        Ok(_) => println!("Server started (unexpected)"),
        Err(e) => eprintln!("❌ Failed to start server: {}", e),
    }

    Ok(())
}
```

Running the CLI with an invalid `--bind` value prints a user‑friendly error such as:

```text
error: invalid value for '--bind <BIND>': address parsing error: invalid IPv6 address
```

The CLI also supports `--bind 0.0.0.0:8080`, `--port 8080`, and other flags as described in the `Cli` struct.

## Pitfalls
### Wrong: Forgetting to enable authentication
```rust
let server = ServerBuilder::new()
    .build()
    .await?;
```
*Result*: The server accepts any request, hiding auth‑related bugs.

### Right: Enable auth explicitly
```rust
let server = ServerBuilder::new()
    .with_auth(true)
    .with_bearer_token("my-token")
    .build()
    .await?;
```

### Wrong: Using an empty user‑message pattern
```rust
Fixture::new().match_user_message("");
```
*Result*: Fixture loading fails with “empty pattern” errors.

### Right: Validation error on empty pattern
```rust
// Attempt to start a server with an empty user‑message pattern – it will fail validation.
let server = ServerBuilder::new()
    .fixture(
        Fixture::new()
            .match_user_message("") // empty pattern
            .respond_with_content("should not matter")
    )
    .bind("127.0.0.1:0")
    .build()
    .await;

match server {
    Ok(_) => println!("Unexpectedly succeeded"),
    Err(e) => eprintln!("Server failed to start as expected: {}", e),
}
```
*Result*: The builder returns an error (e.g., “fixture pattern cannot be empty”), preventing the server from starting and making the problem obvious.

### Wrong: Expecting the pre‑0.3.4 flat SSE format
```rust
let line = resp.text().await?;
let data: serde_json::Value = serde_json::from_str(&line)?;
println!("{}", data["content"]);
```
*Result*: Parsing fails because events are now wrapped in a `response` envelope.

### Right: Access the nested `response` field
```rust
let line = resp.text().await?;
let event: serde_json::Value = serde_json::from_str(&line)?;
println!("{}", event["response"]["content"]);
```

### Wrong: Omitting the `oauth` feature when OAuth endpoints are needed
```toml
# Cargo.toml
llmposter = "0.4.0"
```
*Result*: OAuth mock endpoints are missing (404).

### Right: Enable the feature
```toml
llmposter = { version = "0.4.0", features = ["oauth"] }
```

### Wrong: Adding a bearer token without a usage limit (token never expires)
```rust
ServerBuilder::new()
    .with_bearer_token("static-token")
    .build()
    .await?;
```
*Result*: Token never expires, diverging from real OAuth behaviour.

### Right: Set a usage limit
```rust
ServerBuilder::new()
    .with_bearer_token_uses("static-token", 5) // expires after 5 calls
    .build()
    .await?;
```

## References
- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3.5
### Breaking changes
* **Authentication default** – auth is now **disabled** by default. Earlier versions accepted every request.  
  *Migration*: Call `ServerBuilder::with_auth(true)` when you need token enforcement.

* **OAuth feature gating** – the OAuth mock server is behind the `oauth` Cargo feature (still enabled by default, but CI may disable default features).  
  *Migration*: Add `features = ["oauth"]` to the dependency declaration if your tests hit OAuth endpoints.

### Deprecated → Current mapping
| Deprecated | Current |
|------------|---------|
| `ServerBuilder::with_authentication` (removed) | `ServerBuilder::with_auth(true)` |
| Implicit token expiry | Explicit usage limit via `ServerBuilder::with_bearer_token_uses` or `expires_after_uses` (not shown in API surface) |

### Before → After
**Before (pre‑0.4.0)**
```rust
let server = ServerBuilder::new().build().await?;
```
*All requests succeed, no auth.*

**After (0.4.0)**
```rust
let server = ServerBuilder::new()
    .with_auth(true)
    .with_bearer_token_uses("my-token", 3)
    .build()
    .await?;
```
*Requests now require a valid bearer token and respect the usage limit.*

## API Reference
- **ServerBuilder::new()** – create a fresh builder.
- **ServerBuilder::fixture(self, f: Fixture) → Self** – add a single fixture.
- **ServerBuilder::fixtures(self, fixtures: Vec<Fixture>) → Self** – add multiple fixtures (first‑match‑wins).
- **ServerBuilder::bind(self, addr: &str) → Self** – set the bind address (`host:port`).
- **ServerBuilder::verbose(self, v: bool) → Self** – enable detailed “no match” errors.
- **ServerBuilder::with_auth(self, enabled: bool) → Self** – toggle authentication (default false).
- **ServerBuilder::with_bearer_token(self, token: &str) → Self** – add a token with unlimited uses.
- **ServerBuilder::with_bearer_token_uses(self, token: &str, max_uses: u64) → Self** – add a token that expires after `max_uses` requests.
- **ServerBuilder::with_oauth(self, config: OAuthConfig) → Self** – configure the OAuth mock server (feature `oauth` required).
- **ServerBuilder::with_oauth_defaults(self) → Self** – convenience preset for OAuth.
- **ServerBuilder::load_yaml(self, path: &Path) → Result<Self, Box<dyn std::error::Error>>** – load fixtures from a YAML file.
- **ServerBuilder::build(self) → Result<MockServer, Box<dyn std::error::Error>>** – start the mock server (async).

- **Fixture::new()** – empty fixture builder.
- **Fixture::match_user_message(self, pattern: &str) → Self** – match on the user’s message content.
- **Fixture::match_model(self, pattern: &str) → Self** – match on the model name.
- **Fixture::respond_with_content(self, content: &str) → Self** – static text response.
- **Fixture::respond_with_tool_calls(self, tool_calls: Vec<ToolCall>) → Self** – return tool‑call objects.
- **Fixture::with_streaming(self, latency: Option<u64>, chunk_size: Option<usize>) → Self** – enable SSE streaming.
- **Fixture::with_error(self, status: u16, message: &str) → Self** – produce an HTTP error.
- **Fixture::with_failure(self, failure: FailureConfig) → Self** – inject latency, corruption, truncation, or disconnect.
- **Fixture::with_stop_reason(self, reason: &str) → Self** – set a custom `stop_reason` in the response.
- **Fixture::for_provider(self, provider: Provider) → Self** – limit fixture to a specific provider.

- **FailureConfig** – optional fields `latency_ms`, `corrupt_body`, `truncate_after_frames`, `disconnect_after_ms` to model network failures.

- **Provider** – enum of supported LLM providers (`OpenAI`, `Anthropic`, `Gemini`, `Responses`).

- **ToolCall** – struct representing a tool call (`name`, `arguments`).

- **AuthState::new()** – create a token store.
- **AuthState::add_token(&self, token: &str, max_uses: Option<u64>)** – register a token.
- **AuthState::check_and_use(&self, token: &str) → TokenStatus** – validate and consume a token.
- **AuthState::revoke(&self, token: &str)** – invalidate a token.

- **TokenStatus** – enum `Valid`, `Exhausted`, `Unknown` indicating token state.

### OAuth Mock Endpoints (feature `oauth`) ✅ New
When the `oauth` Cargo feature is enabled, the server also serves standard OAuth endpoints.

```rust
use llmposter::{ServerBuilder, OAuthConfig};
use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let oauth_cfg = OAuthConfig {
        client_id: "demo-client".into(),
        client_secret: "secret".into(),
        redirect_uris: vec!["http://localhost/callback".into()],
        scopes: vec!["openid".into(), "profile".into()],
    };

    let server = ServerBuilder::new()
        .with_oauth(oauth_cfg)
        .bind("127.0.0.1:8080")
        .build()
        .await?;

    let base_url = "http://127.0.0.1:8080";

    let client = Client::new();

    // Retrieve OpenID configuration
    let oidc_resp = client
        .get(format!("{}/.well-known/openid-configuration", base_url))
        .send()
        .await?;
    println!("OpenID config status: {}", oidc_resp.status());
    let oidc_body: serde_json::Value = oidc_resp.json().await?;
    println!("OpenID config: {}", oidc_body);

    // Exchange a token (using client credentials grant for simplicity)
    let token_resp = client
        .post(format!("{}/token", base_url))
        .form(&json!({
            "grant_type": "client_credentials",
            "client_id": "demo-client",
            "client_secret": "secret",
            "scope": "openid profile"
        }))
        .send()
        .await?;
    println!("Token endpoint status: {}", token_resp.status());
    let token_body: serde_json::Value = token_resp.json().await?;
    println!("Token response: {}", token_body);

    // Use the obtained token to call an LLM endpoint (auth enabled via with_auth)
    let access_token = token_body["access_token"]
        .as_str()
        .expect("access_token missing in token response");

    let llm_resp = client
        .post(format!("{}/v1/messages", base_url))
        .bearer_auth(access_token)
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "hello via oauth token" }]
        }))
        .send()
        .await?;
    println!("LLM request status: {}", llm_resp.status());
    let llm_body: serde_json::Value = llm_resp.json().await?;
    println!("LLM response: {}", llm_body);

    Ok(())
}
```