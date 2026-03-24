---
name: llmposter  
description: A mock HTTP server that simulates OpenAI, Anthropic, Gemini and custom LLM provider APIs for testing.  
license: AGPL-3.0-or-later  
metadata:  
  version: "0.4.0"  
  ecosystem: rust
  generated-by: skilldo/gpt-oss-120b + review:gpt-oss-120b
---

## Imports  
```rust
use llmposter::{ServerBuilder, Fixture, ToolCall, Provider, FailureConfig, StreamingConfig};
use llmposter::cli::{Cli, run};
use llmposter::format::{IdGenerator, estimate_tokens, StringMatch, RegexMatch};
use llmposter::fixture::FixtureResponse;
use llmposter::server::MockServer;
use axum::response::IntoResponse;
use serde_json::json;
use tokio::time::Instant;
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

### ✅ Basic request/response fixture  

```rust
// NOTE: This example requires the optional `reqwest` feature.
// Enable it with `cargo run --features=reqwest` or add `features = ["reqwest"]` to your Cargo.toml.

#[cfg(feature = "reqwest")]
mod basic_fixture {
    use llmposter::{Fixture, ServerBuilder};
    use reqwest::Client;
    use serde_json::{json, Value};

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        // ── Build a mock server with a single fixture ─────────────────────────────
        let mock_server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("hello")
                    .respond_with_content("Hi from Claude mock!"),
            )
            .build()
            .await?; // propagate any build errors

        let server_url = mock_server.url();

        // ── Prepare the request payload ───────────────────────────────────────────
        let payload = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "hello world" }]
        });

        // ── Send a request to the mock server ─────────────────────────────────────
        let client = Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server_url))
            .json(&payload)
            .send()
            .await?; // `resp` is a `reqwest::Response`

        // ── Verify HTTP status ───────────────────────────────────────────────────
        if resp.status() != 200 {
            eprintln!("Unexpected HTTP status: {}", resp.status());
            std::process::exit(1);
        }

        // ── Verify response body content ────────────────────────────────────────
        let body_text = resp.text().await?;
        let body: Value = serde_json::from_str(&body_text)?;
        let text = &body["content"][0]["text"];
        if text != "Hi from Claude mock!" {
            eprintln!("Unexpected response content: {}", text);
            std::process::exit(1);
        }

        // ── Success ───────────────────────────────────────────────────────────────
        println!("✓ Test passed: ✅ Basic request/response fixture");
        Ok(())
    }
}

// If the `reqwest` feature is not enabled we provide a stub `main` so the file still compiles.
#[cfg(not(feature = "reqwest"))]
fn main() {
    eprintln!("This example requires the optional `reqwest` feature. Enable it with `cargo run --features=reqwest`.");
}
```

*Matches the user message using a substring rule and returns a plain‑text message.*

### ⚠️ Streaming response with legacy event names (deprecated)

```rust
// NOTE: This example requires the optional `reqwest` feature.
// Enable it with `cargo run --features=reqwest`.

#[cfg(feature = "reqwest")]
mod legacy_streaming {
    use llmposter::{ServerBuilder, Fixture};
    use reqwest::Client;
    use serde_json::json;

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        // ── Start a mock server that streams a response in 5‑byte chunks ────────
        let mock_server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("hello")
                    .respond_with_content("Hello world")
                    .with_streaming(Some(0), Some(5)), // no latency, 5‑byte chunks
            )
            .build()
            .await
            .expect("failed to start mock server");

        let server_url = mock_server.url();

        // ── Build the request payload (the same shape the real API expects) ─────
        let payload = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "hello" }],
            "stream": true
        });

        // ── Send the request and verify the streamed‑event response ────────────
        let client = Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server_url))
            .json(&payload)
            .send()
            .await?;

        // ---- basic HTTP checks -------------------------------------------------
        if resp.status() != 200 {
            eprintln!("Unexpected HTTP status: {}", resp.status());
            std::process::exit(1);
        }

        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if ct != "text/event-stream" {
            eprintln!("Unexpected Content‑Type header: {ct}");
            std::process::exit(1);
        }

        // ---- read the whole body (the mock streams a tiny amount, so this is fine) --
        let body = resp.text().await?;

        // ---- look for the three legacy event names --------------------------------
        if !body.contains("event: message_start") {
            eprintln!("Response missing `message_start` event");
            std::process::exit(1);
        }
        if !body.contains("event: content_block_delta") {
            eprintln!("Response missing `content_block_delta` event");
            std::process::exit(1);
        }
        if !body.contains("event: message_stop") {
            eprintln!("Response missing `message_stop` event");
            std::process::exit(1);
        }

        // -------------------------------------------------------------------------
        // Success!
        // -------------------------------------------------------------------------
        println!("✓ Test passed: ⚠️ Streaming response with legacy event names (deprecated)");
        Ok(())
    }
}

#[cfg(not(feature = "reqwest"))]
fn main() {
    eprintln!("This example requires the optional `reqwest` feature. Enable it with `cargo run --features=reqwest`.");
}
```

*Older SSE event names are retained for backward compatibility but are deprecated as of v0.3.4.*

### ✅ Streaming response with new envelope (v0.3.4+)

```rust
// NOTE: This example requires the optional `reqwest` feature.
// Enable it with `cargo run --features=reqwest`.

#[cfg(feature = "reqwest")]
mod new_streaming {
    use llmposter::{Fixture, ServerBuilder};
    use reqwest::Client;
    use serde_json::json;
    use std::error::Error;
    use std::process;

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn Error>> {
        // ----------------------------------------------------------------------
        // 1️⃣ Spin up a mock server that will stream a response in 5‑byte chunks.
        // ----------------------------------------------------------------------
        let mock_server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("hello")
                    .respond_with_content("Hello world")
                    .with_streaming(Some(0), Some(5)), // no latency, 5‑byte chunks
            )
            .build()
            .await
            .expect("failed to build mock server");

        let server_url = mock_server.url();

        // ----------------------------------------------------------------------
        // 2️⃣ Build the request payload that asks for streaming.
        // ----------------------------------------------------------------------
        let payload = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "hello" }],
            "stream": true
        });

        // ----------------------------------------------------------------------
        // 3️⃣ Send the request to the mock server.
        // ----------------------------------------------------------------------
        let client = Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server_url))
            .json(&payload)
            .send()
            .await
            .expect("failed to send request");

        // -------------------------------------------------------------------
        // Basic sanity checks – abort with clear messages if anything is off.
        // -------------------------------------------------------------------
        if resp.status() != 200 {
            eprintln!("Unexpected HTTP status: {}", resp.status());
            process::exit(1);
        }

        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if ct != "text/event-stream" {
            eprintln!("Unexpected Content-Type header: {}", ct);
            process::exit(1);
        }

        let body = resp
            .text()
            .await
            .expect("failed to read response body");

        // Verify the new envelope events are present.
        if !body.contains("event: response") {
            eprintln!("Response missing `response` event envelope");
            process::exit(1);
        }
        if !body.contains("\"in_progress\":true") {
            eprintln!("Missing `in_progress` flag in streamed payload");
            process::exit(1);
        }
        if !body.contains("\"type\":\"message_stop\"") {
            eprintln!("Missing final `message_stop` payload");
            process::exit(1);
        }

        println!("✓ Test passed: ✅ Streaming response with new envelope (v0.3.4+)");
        Ok(())
    }
}

#[cfg(not(feature = "reqwest"))]
fn main() {
    eprintln!("This example requires the optional `reqwest` feature. Enable it with `cargo run --features=reqwest`.");
}
```

*Uses the v0.3.4+ streaming format where each SSE event is `event: response` carrying a JSON envelope with `in_progress` and sequencing information.*

### ✅ Tool‑call response (non‑streaming)  
*(unchanged – passes)*  

```rust
use llmposter::{Fixture, ServerBuilder, ToolCall};
use reqwest::Client;
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ----------------------------------------------------------------------
    // 1️⃣  Build a mock LLM server that will respond with a tool‑call payload
    // ----------------------------------------------------------------------
    let mock_server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: json!({ "location": "London", "unit": "celsius" }),
                }]),
        )
        .build()
        .await
        .unwrap();
    let server_url = mock_server.url();

    // -------------------------------------------------
    // 2️⃣  Send a request to the mock server (non‑stream)
    // -------------------------------------------------
    let client = Client::new();
    let payload = json!({
        "model": "claude-sonnet-4-6",
        "max_tokens": 1024,
        "messages": [{ "role": "user", "content": "What's the weather in London?" }]
    });
    let resp = client
        .post(format!("{}/v1/messages", server_url))
        .json(&payload)
        .send()
        .await?;

    // -------------------------------------------------
    // 3️⃣  Verify the HTTP response
    // -------------------------------------------------
    assert_eq!(
        resp.status(),
        200,
        "expected HTTP 200 OK, got {}",
        resp.status()
    );

    // -------------------------------------------------
    // 4️⃣  Parse and validate the JSON body
    // -------------------------------------------------
    let body_text = resp.text().await?;
    let body: Value = serde_json::from_str(&body_text)?;
    // LLM signals a tool use via `stop_reason`
    assert_eq!(
        body["stop_reason"], "tool_use",
        "expected stop_reason == tool_use, got {}",
        body["stop_reason"]
    );
    // The first content block must be a tool_use payload
    let tool = &body["content"][0];
    assert_eq!(
        tool["type"], "tool_use",
        "expected tool type == tool_use, got {}",
        tool["type"]
    );
    assert_eq!(
        tool["name"], "get_weather",
        "expected tool name == get_weather, got {}",
        tool["name"]
    );
    assert_eq!(
        tool["input"]["location"], "London",
        "expected location == London, got {}",
        tool["input"]["location"]
    );

    // -------------------------------------------------
    // 5️⃣  Success
    // -------------------------------------------------
    println!("✓ Test passed: ✅ Tool‑call response (non‑streaming)");
    Ok(())
}
```
*Returns a tool‑call payload that matches the Anthropic tool‑use schema.*

### ✅ Simulated latency and failure injection  
*(unchanged – passes)*  

```rust
use llmposter::{ServerBuilder, Fixture, FailureConfig};
use reqwest::Client;
use std::time::Instant;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Keep the server alive for the test
    let mock_server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("delayed response")
                .with_failure(FailureConfig {
                    latency_ms: Some(200),
                    corrupt_body: None,
                    truncate_after_frames: None,
                    disconnect_after_ms: None,
                }),
        )
        .build()
        .await
        .unwrap();
    let server_url = mock_server.url();

    let client = Client::new();
    let start = Instant::now();
    let resp = client
        .post(format!("{}/v1/messages", server_url))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "hi" }]
        }))
        .send()
        .await?;
    let elapsed = start.elapsed();

    assert_eq!(resp.status(), 200);
    let body_text = resp.text().await?;
    let body: serde_json::Value = serde_json::from_str(&body_text)?;
    assert_eq!(body["content"][0]["text"], "delayed response");
    assert!(elapsed.as_millis() >= 180);
    Ok(())
}
```
*Injects a 200 ms latency before sending the response.*

### ✅ Provider‑specific fixture filtering  
*(unchanged – passes)*  

```rust
use llmposter::{ServerBuilder, Fixture, Provider};
use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Keep the server alive
    let mock_server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("anthropic matched")
                .for_provider(Provider::Anthropic),
        )
        .build()
        .await
        .unwrap();
    let server_url = mock_server.url();

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server_url))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "hi" }]
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    let body_text = resp.text().await?;
    let body: serde_json::Value = serde_json::from_str(&body_text)?;
    assert_eq!(body["content"][0]["text"], "anthropic matched");
    Ok(())
}
```
*The fixture is selected only when the request targets the Anthropic provider.*

### ✅ Custom stop reason (non‑streaming)  
*(unchanged – passes)*  

```rust
use llmposter::{ServerBuilder, Fixture, FixtureResponse};
use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mock_server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content: Some("hit max tokens".to_string()),
                stop_reason: Some("max_tokens".to_string()),
                ..FixtureResponse::default()
            }),
            ..Fixture::new()
        })
        .build()
        .await
        .unwrap();
    let server_url = mock_server.url();

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server_url))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 10,
            "messages": [{ "role": "user", "content": "continue until limit" }]
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["stop_reason"], "max_tokens");
    assert_eq!(body["content"][0]["text"], "hit max tokens");
    println!("✓ Test passed: ✅ Custom stop reason (non‑streaming)");
    Ok(())
}
```
*Overrides the default `stop_reason` with a custom value.*

### ✅ Custom stop reason (streaming)  
*(unchanged – passes)*  

```rust
use llmposter::{ServerBuilder, Fixture, FixtureResponse, StreamingConfig};
use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mock_server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content: Some("streamed but truncated".to_string()),
                stop_reason: Some("custom_stop".to_string()),
                ..FixtureResponse::default()
            }),
            streaming: Some(StreamingConfig {
                latency: Some(0),
                chunk_size: Some(5),
            }),
            ..Fixture::new()
        })
        .build()
        .await
        .unwrap();
    let server_url = mock_server.url();

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server_url))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "stream me" }],
            "stream": true
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    let body = resp.text().await?;
    assert!(body.contains("custom_stop"));
    // Updated to reflect the new streaming protocol (v0.3.4+)
    assert!(body.contains("event: response"));
    assert!(body.contains("\"type\":\"message_stop\""));
    println!("✓ Test passed: ✅ Custom stop reason (streaming)");
    Ok(())
}
```
*Shows how to send a custom `stop_reason` in a streamed response.*

### ✅ Bearer‑token authentication (non‑streaming)  
*(new in v0.4.0)*  

```rust
// NOTE: This example requires the optional `reqwest` feature.
// Enable it with `cargo run --features=reqwest`.

#[cfg(feature = "reqwest")]
mod bearer_auth {
    use llmposter::{ServerBuilder, Fixture, Provider};
    use reqwest::Client;
    use serde_json::json;

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        // ----------------------------------------------------------------------
        // 1️⃣  Build a mock server that requires a bearer token
        // ----------------------------------------------------------------------
        let mock_server = ServerBuilder::new()
            .with_auth(true)                     // enable auth checking
            .with_bearer_token("secret-token")   // valid token
            .fixture(
                Fixture::new()
                    .match_user_message("auth test")
                    .respond_with_content("authenticated!"),
            )
            .build()
            .await
            .unwrap();

        let server_url = mock_server.url();

        // ----------------------------------------------------------------------
        // 2️⃣  Send an authorized request
        // ----------------------------------------------------------------------
        let client = Client::new();
        let payload = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "auth test" }]
        });

        let resp = client
            .post(format!("{}/v1/messages", server_url))
            .bearer_auth("secret-token") // <-- provide the token
            .json(&payload)
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["content"][0]["text"], "authenticated!");

        // ----------------------------------------------------------------------
        // 3️⃣  Demonstrate a request without a token (should be rejected)
        // ----------------------------------------------------------------------
        let unauth_resp = client
            .post(format!("{}/v1/messages", server_url))
            .json(&payload)
            .send()
            .await?;

        assert_eq!(unauth_resp.status(), 401);
        println!("✓ Test passed: ✅ Bearer‑token authentication");
        Ok(())
    }
}

#[cfg(not(feature = "reqwest"))]
fn main() {
    eprintln!("This example requires the optional `reqwest` feature. Enable it with `cargo run --features=reqwest`.");
}
```

*Enables simple bearer‑token auth; requests without the correct token receive HTTP 401.*

## Configuration  
- **ServerBuilder**  
  - `bind(addr: &str)`: address to listen on (default `127.0.0.1:0` – OS‑assigned port).  
  - `verbose(bool)`: when `true`, unmatched requests return a JSON error with `"No fixture matched"` and are logged to stdout.  
  - `load_yaml(path) / load_yaml_dir(dir)`: load one or many fixture files written in YAML (supports `truncate_after_chunks` alias for `truncate_after_frames`).  
  - `with_auth(bool)`: enable or disable authentication checking.  
  - `with_bearer_token(token)`: register a bearer token that will be accepted when auth is enabled.  
- **Fixture**  
  - `match_user_message(pattern)`: substring or regex (`StringMatch::Regex`) against the incoming user message.  
  - `match_model(pattern)`: matches the `model` field; also supports regex.  
  - `for_provider(provider)`: restricts the fixture to a specific `Provider` enum variant.  
  - `with_error(status, message)`: returns an HTTP error with the given status and a JSON body `{ "error": { "message": message } }`.  
- **FailureConfig**  
  - `latency_ms`: artificial delay before the response.  
  - `corrupt_body`: if `true`, the response body is replaced with plain text `"overloaded"` and `Content-Type: text/plain`.  
  - `truncate_after_frames`: stop a streaming response after the given number of SSE frames.  
  - `disconnect_after_ms`: close the connection after the given milliseconds (useful for testing abrupt disconnects).  
- **StreamingConfig**  
  - `latency`: per‑frame delay.  
  - `chunk_size`: number of bytes per SSE `content_block_delta` frame.  
- **IdGenerator**  
  - Provides monotonic identifiers for OpenAI, Anthropic, and generic response formats via `next_openai()`, `next_anthropic()`, and `next_responses()`.  

No environment variables are required; all configuration is done programmatically or via the CLI (`llmposter` binary) which forwards arguments to `Cli`.

## Pitfalls  
### Wrong: Missing `.await` on async builder  
```rust
let server = ServerBuilder::new()
    .fixture(Fixture::new().respond_with_content("oops"))
    .build(); // ❌ build returns a Future, not a MockServer
```  
### Right: Await the future  
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap(); // ✅ correct
    // Keep the server alive or use it as needed
    println!("Server started at {}", server.url());
    Ok(())
}
```  
### Wrong: Using mutable static defaults  
```rust
impl Fixture {
    // ❌ mutable static default for latency
    pub fn with_latency(mut self, ms: u64) -> Self {
        static mut DEFAULT_LATENCY: u64 = 0;
        unsafe { DEFAULT_LATENCY = ms; }
        self
    }
}
```  
### Right: Store defaults per instance  
```rust
impl Fixture {
    pub fn with_latency(mut self, ms: u64) -> Self {
        self.streaming = Some(StreamingConfig {
            latency: Some(ms),
            chunk_size: None,
        });
        self
    }
}
```  
### Wrong: Ignoring unknown fields in request structs  
```rust
#[derive(Deserialize, Serialize)]
struct OpenAiResponse { /* fields */ } // ❌ no `deny_unknown_fields`
```  
### Right: Enforce strict schema  
```rust
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OpenAiResponse { /* fields */ } // ✅ rejects unexpected JSON keys
```  
### Wrong: Compiling user‑provided regex without size limit  
```rust
let re = regex::Regex::new(user_pattern)?; // ❌ potential ReDoS
```  
### Right: Use `RegexBuilder` with a size limit  
```rust
let re = regex::RegexBuilder::new(user_pattern)
    .size_limit(1 << 20) // 1 MiB limit
    .build()?;
```

## Migration  
### Upgrading from 0.3.x to 0.4.0
1. **Toolchain** – Run `rustup update stable` (requires Rust 1.89).  
2. **Cargo.toml** – If you rely on the OAuth mock, enable the feature explicitly:  
   ```toml
   llmposter = { version = "0.4", features = ["oauth"] }
   ```  
   To disable optional features:  
   ```toml
   llmposter = { version = "0.4", default-features = false }
   ```  
3. **Authentication** – The auth builder changed:  
   ```rust
   // Old (0.3.x)
   let server = ServerBuilder::new().with_auth(true).run().await;
   // New (0.4.0)
   let server = ServerBuilder::new()
       .with_auth(true)               // enable auth checking
       .with_bearer_token("my-token") // supply a token (or use expires_after_uses)
       .run()
       .await;
   ```  
   If you need token expiry after a number of calls, chain `.expires_after_uses(N)`.  
4. **Streaming protocol** – Already reflected in the “new envelope” example; ensure client code parses `event: response` and respects the `in_progress` flag.  
5. **Error handling** – Adjust error‑parsing structs to the new OpenAI‑compatible shape (`type: String`, `code: String`, `param: Option<String>`).  
6. **CLI changes** – The `--bind` flag now requires a full `host:port` string. Update scripts accordingly.  
7. **Feature flags** – OAuth mock is now optional. Enable it only if needed.  
8. **Tests** – Run the full test matrix (`cargo test --all-features`) after upgrading.

### General Migration Tips
- Run the repository’s test suite after upgrading.  
- Use `run_with_output()` in integration tests to capture CLI output.  
- Review custom fixture YAML files for empty `user_message` or regex patterns; the loader now rejects them.  
- If you previously called OAuth‑specific methods, ensure the `oauth` feature is enabled.  

## References
### Migration (v0.3.4 streaming protocol change)
- **Old behavior**: SSE events were named `message_start`, `content_block_delta`, `message_stop`.  
- **New behavior** (v0.3.4+): Each SSE event is `event: response` carrying a JSON envelope with `in_progress` and sequencing information, and the final chunk contains `"type":"message_stop"`.

### API Reference
- **ServerBuilder** – `new()`, `fixture(Fixture)`, `fixtures(Vec<Fixture>)`, `bind(&str)`, `verbose(bool)`, `with_auth(bool)`, `with_bearer_token(&str)`, `load_yaml(&Path)`, `load_yaml_dir(&Path)`, `build()`.  
- **MockServer** – `url()`, `port()`.  
- **Fixture** – `new()`, `match_user_message(&str)`, `match_model(&str)`, `respond_with_content(&str)`, `respond_with_tool_calls(Vec<ToolCall>)`, `with_error(u16, &str)`, `with_failure(FailureConfig)`, `with_stop_reason(&str)`, `with_finish_reason(&str)`, `with_streaming(Option<u64>, Option<usize>)`, `for_provider(Provider)`.  
- **FailureConfig** – fields `latency_ms`, `corrupt_body`, `truncate_after_frames`, `disconnect_after_ms`.  
- **StreamingConfig** – fields `latency`, `chunk_size`.  
- **ToolCall** – `name: String`, `arguments: serde_json::Value`.  
- **Provider** – enum variants `OpenAI`, `Anthropic`, `Gemini`, `Responses`.  
- **IdGenerator** – `new()`, `next_openai()`, `next_anthropic()`, `next_responses()`, `next_responses_with_counter()`, `next_tool_call_counter()`.  
- **estimate_tokens** – `fn estimate_tokens(text: &str) -> u64`.  
- **Cli** – struct with fields `fixtures: PathBuf`, `validate: bool`, `port: u16`, `bind: String`, `verbose: bool`.  
- **run**, **run_with_output** – async entry points for the CLI.  

---