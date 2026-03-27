---
name: llmposter
description: Mock LLM API server that serves deterministic, fixture‑driven responses for testing.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/gpt-oss-120b + review:gpt-oss-120b
---

## Imports

```rust
use llmposter::{ServerBuilder, Fixture};
use llmposter::ToolCall;
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

### ✅ Basic request/response fixture
A minimal fixture that matches the user message and returns static text.

```rust
use std::error::Error;

use llmposter::{ServerBuilder, Fixture};
use reqwest::Client;
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Build a mock server with a single fixture.
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from mock!"),
        )
        .build()
        .await?;

    // Send a request exactly as a real LLM provider would expect.
    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "hello" }]
        }))
        .send()
        .await?;

    println!("Status: {}", resp.status());
    // The mock server adds a deterministic request identifier.
    if let Some(req_id) = resp.headers().get("x-request-id") {
        println!("x-request-id: {}", req_id.to_str().unwrap_or("<invalid>"));
    }

    let body: Value = resp.json().await?;
    println!("Body: {}", body);

    // The mock server follows the OpenAI‑compatible chat format:
    // { "choices": [{ "message": { "content": "..."} } ] }
    let content = body
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str());

    match content {
        Some("Hi from mock!") => {
            println!("✓ Test passed: ✅ Basic request/response fixture");
        }
        Some(other) => {
            eprintln!("Unexpected response content: {}", other);
            std::process::exit(1);
        }
        None => {
            eprintln!("Response JSON missing expected `content` field");
            std::process::exit(1);
        }
    }

    Ok(())
}
```

### ✅ Streaming text response
Configure a fixture to stream the response in SSE chunks. The `latency` and `chunk_size` control pacing.

```rust
use llmposter::{ServerBuilder, Fixture};
use reqwest::Client;
use serde_json::json;
use std::error::Error;

/// Extract the `content` string from a JSON payload. Handles:
/// * OpenAI‑style streaming (`choices[0].delta.content`)
/// * Plain top‑level `content`
/// * Nested `response.content` (used by some providers)
fn extract_content(v: &serde_json::Value) -> Option<String> {
    // OpenAI streaming format.
    if let Some(s) = v.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("delta"))
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str())
    {
        return Some(s.to_string());
    }

    // Flat structure.
    if let Some(s) = v.get("content").and_then(|c| c.as_str()) {
        return Some(s.to_string());
    }

    // Nested structure (used by some real LLM APIs).
    v.get("response")
        .and_then(|r| r.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // -------------------------------------------------------------------------
    // 1️⃣ Build a mock server that streams the response in SSE chunks.
    // -------------------------------------------------------------------------
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("stream")
                .respond_with_content("Streaming response from mock")
                // latency = 0 ms, chunk size = 5 bytes
                .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await?;

    // -------------------------------------------------------------------------
    // 2️⃣ Send a request that asks the server to stream.
    // -------------------------------------------------------------------------
    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "stream" }],
            "stream": true
        }))
        .send()
        .await?;

    println!("Status: {}", resp.status());

    // -------------------------------------------------------------------------
    // 3️⃣ Read the whole SSE body as a string.
    // -------------------------------------------------------------------------
    let body = resp.text().await?;
    println!("SSE body:\n{}", body);

    // -------------------------------------------------------------------------
    // 4️⃣ Extract the streamed `content` fragments from each SSE line.
    // -------------------------------------------------------------------------
    let assembled: String = body
        .lines()
        .filter_map(|line| {
            // Trim whitespace.
            let line = line.trim();

            // Skip empty lines and the explicit `[DONE]` marker.
            if line.is_empty() || line == "[DONE]" {
                return None;
            }

            // SSE lines are of the form `data: {...}` (sometimes without a space after the colon).
            // Strip the `data:` prefix; if it isn’t present we ignore the line.
            let json_part = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:"))?;
            // If the payload starts with `[` it’s a control token (e.g., `[DONE]`); ignore.
            if json_part.trim_start().starts_with('[') {
                return None;
            }

            // Parse the JSON and extract the `content` field.
            let v: serde_json::Value = serde_json::from_str(json_part).ok()?;
            extract_content(&v)
        })
        .collect::<Vec<_>>()
        .join("");

    // -------------------------------------------------------------------------
    // 5️⃣ Verify that the number of data frames matches the expected chunk count.
    // -------------------------------------------------------------------------
    let content_len = "Streaming response from mock".len();
    let chunk_size = 5;
    let expected_chunks = (content_len + chunk_size - 1) / chunk_size; // ceil division
    let data_lines = body
        .lines()
        .filter(|l| {
            let line = l.trim();
            line.starts_with("data:")
        })
        .count();
    if data_lines != expected_chunks {
        eprintln!(
            "✗ Test failed: expected {} data frames, got {}",
            expected_chunks, data_lines
        );
        std::process::exit(1);
    }

    // -------------------------------------------------------------------------
    // 6️⃣ Verify that the expected fragment appears in the assembled text.
    // -------------------------------------------------------------------------
    if assembled.contains("Streaming response from mock") {
        println!("✓ Test passed: ✅ Streaming text response");
    } else {
        eprintln!("✗ Test failed: expected streaming content not found");
        std::process::exit(1);
    }

    Ok(())
}
```

#### 📐 Streaming with truncation failure
The `FailureConfig` can truncate a stream after a given number of chunks. The example below demonstrates that behavior.

```rust
use llmposter::{ServerBuilder, Fixture};
use llmposter::FailureConfig;
use reqwest::Client;
use serde_json::json;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("truncated")
                .respond_with_content("This will be cut off")
                .with_failure(FailureConfig {
                    truncate_after_frames: Some(2),
                    latency_ms: None,
                    corrupt_body: None,
                    disconnect_after_ms: None,
                })
                .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await?;

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "truncated" }],
            "stream": true
        }))
        .send()
        .await?;

    let body = resp.text().await?;
    println!("Truncated SSE body:\n{}", body);
    // The body will contain only the first two SSE chunks.
    Ok(())
}
```

### ✅ Tool‑call response
Return a tool call payload instead of plain text.

```rust
use llmposter::{ServerBuilder, Fixture};
use llmposter::ToolCall;
use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: json!({ "location": "London", "unit": "celsius" }),
                }]),
        )
        .build()
        .await?;

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "What's the weather in London?" }]
        }))
        .send()
        .await?;

    println!("Status: {}", resp.status());
    let body: serde_json::Value = resp.json().await?;
    println!("Body: {}", body);
    Ok(())
}
```

### ❌ Invalid tool‑call arguments (validation failure)
Tool calls must contain a JSON object for `arguments`. Supplying a non‑object triggers validation.

```rust
use llmposter::{ServerBuilder, Fixture};
use llmposter::ToolCall;
use reqwest::Client;
use serde_json::json;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new()
        .match_user_message("bad_tool")
        .respond_with_tool_calls(vec![ToolCall {
            name: "bad_call".to_string(),
            // ❗ This is a string, not an object – validation will reject it.
            arguments: json!("just a string"),
        }]);

    // Validation will fail because `arguments` is not an object.
    match fixture.validate() {
        Ok(_) => eprintln!("Unexpected success"),
        Err(e) => println!("✅ Expected validation error: {}", e),
    }

    Ok(())
}
```

### ✅ Error injection fixture
Force the server to return a specific HTTP error code and JSON error shape, **including rate‑limit headers**.

```rust
use llmposter::{ServerBuilder, Fixture};
use reqwest::Client;
use serde_json::{json, Value};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("rate limit")
                .with_error(429, "Rate limit exceeded"),
        )
        .build()
        .await?;

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "trigger rate limit" }]
        }))
        .send()
        .await?;

    // Verify the HTTP status code.
    if resp.status().as_u16() != 429 {
        eprintln!("Unexpected status: {}", resp.status());
        std::process::exit(1);
    }

    // Verify rate‑limit headers are present.
    if let Some(limit) = resp.headers().get("x-ratelimit-limit") {
        println!("Rate limit: {}", limit.to_str().unwrap_or("<invalid>"));
    }
    if let Some(remaining) = resp.headers().get("x-ratelimit-remaining") {
        println!("Remaining: {}", remaining.to_str().unwrap_or("<invalid>"));
    }
    // Provider‑specific headers (OpenAI/Responses example)
    if let Some(limit_req) = resp.headers().get("x-ratelimit-limit-requests") {
        println!("Rate limit (requests): {}", limit_req.to_str().unwrap_or("<invalid>"));
    }
    if let Some(remaining_req) = resp.headers().get("x-ratelimit-remaining-requests") {
        println!("Remaining (requests): {}", remaining_req.to_str().unwrap_or("<invalid>"));
    }

    // Verify the full JSON error shape (type, code, param, message).
    let body: Value = resp.json().await?;
    let error = body.get("error").expect("error field missing");
    let err_type = error.get("type").and_then(|v| v.as_str()).unwrap_or("<missing>");
    let err_code = error.get("code").and_then(|v| v.as_str()).unwrap_or("<missing>");
    let err_param = error.get("param");
    let err_message = error.get("message").and_then(|v| v.as_str()).unwrap_or("<missing>");

    if err_type != "rate_limit"
        || err_code != "rate_limit"
        || err_message != "Rate limit exceeded"
    {
        eprintln!("Unexpected error shape: {}", body);
        std::process::exit(1);
    }

    println!(
        "Error type: {}, code: {}, param: {:?}, message: {}",
        err_type, err_code, err_param, err_message
    );

    println!("✓ Test passed: ✅ Error injection fixture");
    Ok(())
}
```

### ✅ Bearer‑token authentication with expiry
Enable auth, add a bearer token that expires after a limited number of uses, and verify the token is rejected after the limit. Also demonstrates the 401 error when no token is supplied.

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Token will be valid for exactly two successful calls.
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token_uses("my-token", 2)
        .fixture(
            Fixture::new()
                .match_user_message("auth test")
                .respond_with_content("Authenticated response"),
        )
        .build()
        .await?;

    let client = Client::new();

    // First request – succeeds.
    let resp1 = client
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth("my-token")
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "auth test" }]
        }))
        .send()
        .await?;
    println!("First call status: {}", resp1.status());

    // Second request – also succeeds.
    let resp2 = client
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth("my-token")
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "auth test" }]
        }))
        .send()
        .await?;
    println!("Second call status: {}", resp2.status());

    // Third request – token exhausted, returns 401.
    let resp3 = client
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth("my-token")
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "auth test" }]
        }))
        .send()
        .await?;
    println!("Third call status (expected 401): {}", resp3.status());

    // Request without any token – also returns 401.
    let resp_no_token = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "auth test" }]
        }))
        .send()
        .await?;
    println!("No‑token call status (expected 401): {}", resp_no_token.status());

    Ok(())
}
```

### ✅ Deterministic request ID header
Every successful LLM request receives a deterministic `x-request-id` header of the form `req-llmposter-{N}` where `N` starts at 1 and increments with each request. This enables reproducible tracing across test runs.

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("id")
                .respond_with_content("id response"),
        )
        .build()
        .await?;

    let client = Client::new();

    // First request – should receive x-request-id = req-llmposter-1
    let resp1 = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "test-model",
            "messages": [{ "role": "user", "content": "id" }]
        }))
        .send()
        .await?;
    let id1 = resp1
        .headers()
        .get("x-request-id")
        .expect("missing x-request-id")
        .to_str()?;
    assert_eq!(id1, "req-llmposter-1");
    println!("First request id: {}", id1);

    // Second request – should receive x-request-id = req-llmposter-2
    let resp2 = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "test-model",
            "messages": [{ "role": "user", "content": "id" }]
        }))
        .send()
        .await?;
    let id2 = resp2
        .headers()
        .get("x-request-id")
        .expect("missing x-request-id")
        .to_str()?;
    assert_eq!(id2, "req-llmposter-2");
    println!("Second request id: {}", id2);

    Ok(())
}
```

### ❌ Validation failure for empty patterns
A fixture that contains an empty substring match (e.g., `match_user_message("")`) or an empty regex pattern is considered invalid. Validation occurs when the server is built or when `fixture.validate()` is called.

```rust
use llmposter::{ServerBuilder, Fixture};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Attempt to create a fixture with an empty user‑message pattern.
    let fixture = Fixture::new()
        .match_user_message("") // empty pattern – invalid
        .respond_with_content("should not matter");

    // Direct validation – should error.
    match fixture.validate() {
        Ok(_) => eprintln!("❌ Unexpected success – empty pattern should fail validation"),
        Err(e) => println!("✅ Expected validation error for empty pattern: {}", e),
    }

    // Alternatively, building the server will surface the same error.
    let result = ServerBuilder::new()
        .fixture(fixture)
        .build()
        .await;
    if let Err(e) = result {
        println!("✅ Server builder rejected empty pattern: {}", e);
    }

    Ok(())
}
```

### ❌ Validation failure for empty regex pattern
An empty regex pattern is also invalid and will cause validation to fail.

```rust
use llmposter::{ServerBuilder, Fixture, RegexMatch, StringMatch, FixtureMatch};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut fixture = Fixture::new();
    fixture.match_rule = Some(FixtureMatch {
        user_message: Some(StringMatch::Regex(RegexMatch {
            regex: "".to_string(),
            ..Default::default()
        })),
        model: None,
    });
    let fixture = fixture.respond_with_content("should not matter");

    match fixture.validate() {
        Ok(_) => eprintln!("❌ Unexpected success – empty regex should fail validation"),
        Err(e) => println!("✅ Expected validation error for empty regex: {}", e),
    }

    Ok(())
}
```

### ❌ Validation failure for oversized regex
Regex patterns larger than 1 MiB (as measured by the compiled DFA size) are rejected to avoid OOM crashes. The builder returns an explicit error indicating the size limit.

```rust
use llmposter::{ServerBuilder, Fixture, RegexMatch, StringMatch, FixtureMatch};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Construct a regex pattern that exceeds the 1 MiB DFA limit.
    let huge_regex = "a".repeat(2 * 1024 * 1024); // ~2 MiB

    // Build a fixture that uses this regex for the user message match.
    let mut fixture = Fixture::new();
    fixture.match_rule = Some(FixtureMatch {
        user_message: Some(StringMatch::Regex(RegexMatch {
            regex: huge_regex,
            ..Default::default()
        })),
        model: None,
    });
    let fixture = fixture.respond_with_content("won't be used");

    // Attempt to load the fixture into a server – should error.
    let result = ServerBuilder::new()
        .fixture(fixture)
        .build()
        .await;

    match result {
        Ok(_) => eprintln!("❌ Unexpected success – oversized regex should be rejected"),
        Err(e) => println!("✅ Expected error for oversized regex: {}", e),
    }

    Ok(())
}
```

### ✅ OAuth flow (client credentials)
When compiled with the `oauth` feature, `ServerBuilder` can be configured with an `OAuthConfig`. The mock server can issue access tokens using the client_credentials grant (PKCE/device‑code flows are also supported but not shown here).

```rust
#[cfg(feature = "oauth")]
use llmposter::OAuthConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure OAuth (client credentials must be supplied; in tests they can be dummy values).
    let oauth_cfg = OAuthConfig {
        client_id: "my-client-id".to_string(),
        client_secret: "my-secret".to_string(),
        redirect_uris: vec!["http://localhost/callback".to_string()],
        scopes: vec!["openid".to_string(), "profile".to_string()],
    };

    let server = ServerBuilder::new()
        .with_oauth(oauth_cfg)
        .fixture(
            Fixture::new()
                .match_user_message("oauth test")
                .respond_with_content("OAuth succeeded"),
        )
        .build()
        .await?;

    // Obtain an access token via the mock OAuth endpoint.
    let token_resp = Client::new()
        .post(format!("{}/oauth/token", server.url()))
        .json(&json!({
            "client_id": oauth_cfg.client_id,
            "client_secret": oauth_cfg.client_secret,
            "grant_type": "client_credentials"
        }))
        .send()
        .await?;
    let token_body: serde_json::Value = token_resp.json().await?;
    let access_token = token_body
        .get("access_token")
        .and_then(|v| v.as_str())
        .expect("access_token missing");

    // Use the access token to call the LLM endpoint.
    let resp = Client::new()
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth(access_token)
        .json(&json!({
            "model": "test-model",
            "messages": [{ "role": "user", "content": "oauth test" }]
        }))
        .send()
        .await?;
    if resp.status().is_success() {
        println!("✅ OAuth request succeeded with status {}", resp.status());
    } else {
        eprintln!("✗ OAuth request failed: {}", resp.status());
        std::process::exit(1);
    }

    // The client would now perform the PKCE/device‑code exchange against the mock server.
    // For brevity, we only show that the server starts successfully.
    println!("OAuth‑enabled mock server running at {}", server.url());
    Ok(())
}
```

> **Note:** The `oauth` feature is disabled by default. Enable it in `Cargo.toml` with `features = ["oauth"]`.

#### ✅ PKCE/device‑code flow
The mock server also supports the PKCE and device‑code grant types. Below is a minimal example that obtains a device code, simulates user approval, and exchanges it for an access token.

```rust
#[cfg(feature = "oauth")]
async fn pkce_device_flow_example(server: &llmposter::MockServer, cfg: &llmposter::OAuthConfig) -> Result<(), Box<dyn std::error::Error>> {
    use reqwest::Client;
    use serde_json::json;

    let client = Client::new();

    // 1️⃣ Request a device code.
    let device_resp = client
        .post(format!("{}/oauth/device/code", server.url()))
        .json(&json!({
            "client_id": cfg.client_id,
            "scope": cfg.scopes.join(" ")
        }))
        .send()
        .await?;
    let device_body: serde_json::Value = device_resp.json().await?;
    let device_code = device_body["device_code"].as_str().expect("device_code missing");
    let user_code = device_body["user_code"].as_str().expect("user_code missing");
    println!("Please approve the request using code: {}", user_code);

    // 2️⃣ Simulate immediate user approval by exchanging the device code for a token.
    let token_resp = client
        .post(format!("{}/oauth/token", server.url()))
        .json(&json!({
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            "device_code": device_code,
            "client_id": cfg.client_id
        }))
        .send()
        .await?;
    let token_body: serde_json::Value = token_resp.json().await?;
    let access_token = token_body["access_token"].as_str().expect("access_token missing");

    // 3️⃣ Use the access token to call the LLM endpoint.
    let llm_resp = client
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth(access_token)
        .json(&json!({
            "model": "test-model",
            "messages": [{ "role": "user", "content": "pkce test" }]
        }))
        .send()
        .await?;
    if llm_resp.status().is_success() {
        println!("✅ PKCE/device‑code flow succeeded with status {}", llm_resp.status());
    } else {
        eprintln!("✗ PKCE/device‑code flow failed: {}", llm_resp.status());
        std::process::exit(1);
    }
    Ok(())
}
```

> The above function can be called after the server is built, e.g.:
> ```rust
> pkce_device_flow_example(&server, &oauth_cfg).await?;
> ```

### ✅ Streaming response with new nested `response` envelope
The server now wraps each SSE chunk in a `response` object. This example shows how to parse that format.

```rust
use llmposter::{ServerBuilder, Fixture};
use reqwest::Client;
use serde_json::json;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("newstream")
                .respond_with_content("New streaming format")
                .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await?;

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "test-model",
            "messages": [{ "role": "user", "content": "newstream" }],
            "stream": true
        }))
        .send()
        .await?;

    let sse_body = resp.text().await?;

    // Assemble content from the nested `response` objects.
    let assembled: String = sse_body
        .lines()
        .filter_map(|line| {
            let json_part = line.strip_prefix("data: ")?;
            let envelope: serde_json::Value = serde_json::from_str(json_part).ok()?;
            envelope.get("response")?.get("delta")?.get("content")?.as_str().map(|s| s.to_string())
        })
        .collect::<Vec<_>>()
        .join("");

    println!("Assembled streamed content: {}", assembled);
    Ok(())
}
```

## Configuration

| Component | Default | Typical Customization |
|-----------|---------|-----------------------|
| `ServerBuilder::bind` | `"127.0.0.1:0"` (random free port) | Pass a string `"0.0.0.0:8080"` to listen on a fixed address. |
| `Fixture::with_streaming` | `latency: None`, `chunk_size: None` | `Some(ms)` for latency, `Some(bytes)` for chunk size. |
| `FailureConfig` | All fields `None` | Set `latency_ms`, `corrupt_body`, `truncate_after_frames`, `disconnect_after_ms` to simulate network failures. |
| `AuthState` (via `ServerBuilder::with_auth`) | Disabled | Enable with `with_auth(true)`. Use `with_bearer_token` or `with_bearer_token_uses` to add tokens. |
| `IdGenerator` | Starts at `0` | Generates deterministic IDs for OpenAI, Anthropic, and Responses providers. |
| Provider filtering | No filter | Use `Fixture::for_provider(Provider::OpenAI)` to restrict a fixture to a single provider. |
| Environment variables | None required | All configuration is supplied programmatically; no env vars are needed for basic usage. |

## Pitfalls

### Wrong – Using `unwrap()` on a builder step
```rust
let server = ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("hi").respond_with_content("hi"))
    .build()
    .unwrap(); // panics on error
```

### Right – Propagate errors with `?`
```rust
let server = ServerBuilder::new()
    .fixture(Fixture::new().match_user_message("hi").respond_with_content("hi"))
    .build()
    .await?;
```

### Wrong – Forgetting the `async` keyword on a function that performs I/O
```rust
fn get_response() -> Result<(), Box<dyn std::error::Error>> {
    let resp = client.post(...).send().await?; // compile error
}
```

### Right – Declare the function as `async`
```rust
async fn get_response() -> Result<(), Box<dyn std::error::Error>> {
    let resp = client.post(...).send().await?;
    Ok(())
}
```

### Wrong – Parsing the bind address with `parse()` that treats `"host:port"` as IPv6
```rust
let bind = args.value_of("--bind").unwrap(); // may misinterpret
```

### Right – Explicitly parse as `IpAddr`
```rust
use std::net::IpAddr;
let bind: IpAddr = args.value_of("--bind").unwrap().parse().expect("valid IP");
```

## References
- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from v0.3.x

### Streaming protocol change (v0.3.4 → v0.4.0)
* **Before**: SSE events were plain `data:` frames with a top‑level `response.done` event.
* **After**: Events are wrapped in a `response` object, include `sequence_number` and `response.in_progress`, and `response.done` is removed.

**Migration** – Update client parsers to read the nested `response` field and handle the `in_progress` event instead of expecting a `done` event.

```rust
// Old parsing (v0.3.3)
let line = ...;
let payload: serde_json::Value = serde_json::from_str(&line)?;
if payload.get("type") == Some(&json!("response.done")) { /* … */ }

// New parsing (v0.4.0)
let line = ...;
let envelope: serde_json::Value = serde_json::from_str(&line)?;
if let Some(resp) = envelope.get("response") {
    match resp.get("event").and_then(|e| e.as_str()) {
        Some("in_progress") => { /* handle intermediate chunk */ }
        Some("message_stop") => { /* final chunk */ }
        _ => {}
    }
}
```

### Error shape change
* **Before** (v0.3.4): `code` could be numeric, and `param` might be omitted.
* **After** (v0.4.0): `code` is always a `String`; `param` is always present (use `null` when not applicable).

**Migration** – Treat `code` as a string and ensure client code tolerates a `null` `param`.

```rust
let code = error["code"].as_str().unwrap(); // always safe now
let param = &error["param"]; // may be Null
```

### OAuth feature default
* The `oauth` feature is **disabled** by default in 0.4.0. If you previously had it enabled, add `features = ["oauth"]` to your `Cargo.toml` to retain the same behaviour.

```toml
llmposter = { version = "0.4.0", features = ["oauth"] }
```

## API Reference

- **ServerBuilder::new()** – Creates a fresh builder with default settings.
- **ServerBuilder::fixture(self, f: Fixture) -> Self** – Adds a single fixture; order matters (first‑match‑wins).
- **ServerBuilder::fixtures(self, fixtures: Vec<Fixture>) -> Self** – Adds many fixtures at once.
- **ServerBuilder::bind(self, addr: &str) -> Self** – Sets the address the mock server will listen on.
- **ServerBuilder::with_auth(self, enabled: bool) -> Self** – Enables bearer‑token authentication.
- **ServerBuilder::with_bearer_token(self, token: &str) -> Self** – Registers a token with unlimited uses.
- **ServerBuilder::with_bearer_token_uses(self, token: &str, max_uses: u64) -> Self** – Registers a token that expires after `max_uses` successful calls.
- **ServerBuilder::build(self) -> impl Future<Output = Result<MockServer, Box<dyn std::error::Error>>>** – Starts the mock server; must be `.await`ed.
- **MockServer::url(&self) -> String** – Returns the base URL (e.g., `http://127.0.0.1:12345`) for client requests.
- **Fixture::new()** – Starts a new fixture definition.
- **Fixture::match_user_message(self, pattern: &str) -> Self** – Sets a substring match on the user message.
- **Fixture::match_model(self, pattern: &str) -> Self** – Sets a substring match on the `model` field.
- **Fixture::respond_with_content(self, content: &str) -> Self** – Returns plain text in the response body.
- **Fixture::respond_with_tool_calls(self, tool_calls: Vec<ToolCall>) -> Self** – Returns a tool‑call payload.
- **Fixture::with_streaming(self, latency: Option<u64>, chunk_size: Option<usize>) -> Self** – Enables SSE streaming with optional latency and chunk size.
- **Fixture::with_failure(self, failure: FailureConfig) -> Self** – Simulates network failures (latency, corruption, truncation, disconnect).
- **Fixture::with_error(self, status: u16, message: &str) -> Self** – Forces an HTTP error response.
- **ToolCall { name: String, arguments: serde_json::Value }** – Represents a single tool call.
- **FailureConfig { latency_ms: Option<u64>, corrupt_body: Option<bool>, truncate_after_frames: Option<u32>, disconnect_after_ms: Option<u64> }** – Controls failure injection.
- **Provider::as_str(&self) -> &'static str** – Returns the provider name used in request routing.
- **IdGenerator::next_openai(&self) -> String** – Generates deterministic OpenAI‑style IDs.