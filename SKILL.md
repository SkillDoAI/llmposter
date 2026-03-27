---
name: llmposter
description: A mock LLM API server that returns deterministic responses from YAML fixtures, supporting streaming, failure injection, and optional authentication.
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/gpt-oss-120b + review:gpt-oss-120b
---

## Imports
```rust
use llmposter::{
    ServerBuilder,
    Fixture,
    ToolCall,
    FailureConfig,
    StreamingConfig,
    OAuthConfig,
    Provider,
};
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

### Basic fixture response ✅ Current
Return a static message when the user's prompt contains a given substring.
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build a mock server with a single fixture.
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from Claude mock!"),
        )
        .build()
        .await?; // `build` is async

    // Issue a request to the mock endpoint.
    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "hello world" }]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["type"], "message");
    assert_eq!(body["role"], "assistant");
    assert_eq!(body["content"][0]["type"], "text");
    assert_eq!(body["content"][0]["text"], "Hi from Claude mock!");
    Ok(())
}
```

### Streaming response with artificial latency ✅ Current
Configure a fixture to stream the response in chunks with a per‑chunk delay.
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hello world")
                .with_streaming(Some(0), Some(5)), // latency 0 ms, 5‑byte chunks
        )
        .build()
        .await?;

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "hello" }],
            "stream": true
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/event-stream");
    let body = resp.text().await?;
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: content_block_delta"));
    assert!(body.contains("event: message_stop"));
    Ok(())
}
```

### Bearer‑token authentication ✅ Current
Enable authentication and supply a static bearer token that the server will accept.
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .with_auth(true)                // require a token
        .with_bearer_token("test-token") // token that will be accepted
        .fixture(
            Fixture::new()
                .match_user_message("secure")
                .respond_with_content("Authenticated response"),
        )
        .build()
        .await?;

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth("test-token") // send the token
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "secure request" }]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["content"][0]["text"], "Authenticated response");
    Ok(())
}
```

### Bearer‑token authentication failure (401) ✅ New
Calling an LLM endpoint without a valid bearer token when authentication is enabled returns a 401 error.
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Server requires authentication but we do NOT send a token.
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token("valid-token")
        .fixture(
            Fixture::new()
                .match_user_message("secure")
                .respond_with_content("Should not be reachable"),
        )
        .build()
        .await?;

    let client = Client::new();
    // Omit the bearer token deliberately.
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "messages": [{ "role": "user", "content": "secure" }]
        }))
        .send()
        .await?;

    // The server rejects the request with 401 Unauthorized.
    assert_eq!(resp.status(), 401);
    Ok(())
}
```

### Bearer‑token usage limits ✅ New
Limit a token to a fixed number of uses. After the limit is reached the server returns an *exhausted* status.
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Token "limited-token" may be used only 2 times.
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token_uses("limited-token", 2)
        .fixture(
            Fixture::new()
                .match_user_message("count")
                .respond_with_content("First use"),
        )
        .build()
        .await?;

    let client = Client::new();

    // First request – succeeds.
    let resp1 = client
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth("limited-token")
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "messages": [{ "role": "user", "content": "count" }]
        }))
        .send()
        .await?;
    assert_eq!(resp1.status(), 200);

    // Second request – also succeeds (second allowed use).
    let resp2 = client
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth("limited-token")
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "messages": [{ "role": "user", "content": "count" }]
        }))
        .send()
        .await?;
    assert_eq!(resp2.status(), 200);

    // Third request – token exhausted, server returns 401.
    let resp3 = client
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth("limited-token")
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "messages": [{ "role": "user", "content": "count" }]
        }))
        .send()
        .await?;
    assert_eq!(resp3.status(), 401);
    Ok(())
}
```

### Tool‑call response (non‑streaming) ✅ Current
Return a tool‑call payload instead of plain text.
```rust
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

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["type"], "message");
    assert_eq!(body["stop_reason"], "tool_use");
    let tool = &body["content"][0];
    assert_eq!(tool["type"], "tool_use");
    assert_eq!(tool["name"], "get_weather");
    assert_eq!(tool["input"]["location"], "London");
    Ok(())
}
```

### Fixture error response ✅ New
Return a custom HTTP error status and JSON body when a fixture matches.
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("bad request")
                .with_error(400, "Invalid input provided"),
        )
        .build()
        .await?;

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "messages": [{ "role": "user", "content": "bad request" }]
        }))
        .send()
        .await?;

    // The server returns the configured status and error payload.
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["error"], "Invalid input provided");
    Ok(())
}
```

### Failure injection (truncation) ✅ New
Inject a failure that truncates the response after a few frames, simulating a dropped connection.
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let failure = FailureConfig {
        latency_ms: None,
        corrupt_body: None,
        truncate_after_frames: Some(2), // close after 2 frames
        disconnect_after_ms: None,
    };

    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("truncate")
                .respond_with_content("This response will be cut off")
                .with_failure(failure),
        )
        .build()
        .await?;

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "messages": [{ "role": "user", "content": "truncate" }],
            "stream": true
        }))
        .send()
        .await?;

    // The connection is closed early; we can verify that only part of the stream arrived.
    let body = resp.text().await?;
    assert!(body.contains("event: message_start"));
    // Because of truncation we expect fewer `content_block_delta` events.
    assert!(body.matches("event: content_block_delta").count() <= 2);
    Ok(())
}
```

### OAuth flow (discovery & token) ✅ New
Enable OAuth discovery endpoints and token validation. The example shows configuring the server with an `OAuthConfig`.
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let oauth_cfg = OAuthConfig {
        client_id: "my-client".to_string(),
        client_secret: "secret".to_string(),
        redirect_uris: vec!["http://localhost/callback".to_string()],
        scopes: vec!["read".to_string(), "write".to_string()],
    };

    let server = ServerBuilder::new()
        .with_oauth(oauth_cfg)          // enable OAuth endpoints
        .fixture(
            Fixture::new()
                .match_user_message("secure")
                .respond_with_content("OAuth protected response"),
        )
        .build()
        .await?;

    let client = Client::new();

    // 1️⃣ Discover the token endpoint.
    let discovery = client
        .get(format!("{}/.well-known/openid-configuration", server.url()))
        .send()
        .await?;
    assert_eq!(discovery.status(), 200);
    let discovery_json: serde_json::Value = discovery.json().await?;
    let token_endpoint = discovery_json["token_endpoint"]
        .as_str()
        .expect("token_endpoint missing");

    // 2️⃣ Exchange client credentials for an access token.
    let token_resp = client
        .post(token_endpoint)
        .form(&json!({
            "grant_type": "client_credentials",
            "client_id": "my-client",
            "client_secret": "secret",
            "scope": "read write"
        }))
        .send()
        .await?;
    assert_eq!(token_resp.status(), 200);
    let token_json: serde_json::Value = token_resp.json().await?;
    let access_token = token_json["access_token"]
        .as_str()
        .expect("access_token missing");

    // 3️⃣ Use the token to call the mock LLM endpoint.
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth(access_token)
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "messages": [{ "role": "user", "content": "secure" }]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["content"][0]["text"], "OAuth protected response");
    Ok(())
}
```

### OAuth flow with defaults (JWKS, device‑code, PKCE) ✅ New
Demonstrates the convenience of `with_oauth_defaults()` which automatically provides JWKS, device‑code, and PKCE endpoints.
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Enable OAuth with the built‑in default configuration.
    let server = ServerBuilder::new()
        .with_oauth_defaults()
        .fixture(
            Fixture::new()
                .match_user_message("secure")
                .respond_with_content("OAuth defaults response"),
        )
        .build()
        .await?;

    let client = Client::new();

    // 1️⃣ JWKS endpoint – contains the server's public keys.
    let jwks = client
        .get(format!("{}/.well-known/jwks.json", server.url()))
        .send()
        .await?;
    assert_eq!(jwks.status(), 200);
    let jwks_json: serde_json::Value = jwks.json().await?;
    assert!(jwks_json["keys"].as_array().unwrap().len() > 0);

    // 2️⃣ Device‑code flow – obtain a device code.
    let device_resp = client
        .post(format!("{}/device/code", server.url()))
        .form(&json!({
            "client_id": "default-client",
            "scope": "read write"
        }))
        .send()
        .await?;
    assert_eq!(device_resp.status(), 200);
    let device_json: serde_json::Value = device_resp.json().await?;
    let device_code = device_json["device_code"]
        .as_str()
        .expect("device_code missing");

    // 3️⃣ Poll the token endpoint using the device code.
    let token_resp = client
        .post(format!("{}/token", server.url()))
        .form(&json!({
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            "device_code": device_code,
            "client_id": "default-client"
        }))
        .send()
        .await?;
    assert_eq!(token_resp.status(), 200);
    let token_json: serde_json::Value = token_resp.json().await?;
    let access_token = token_json["access_token"]
        .as_str()
        .expect("access_token missing");

    // 4️⃣ PKCE flow – exchange an authorization code.
    // (In a real flow the user would be redirected; here we simulate it.)
    let pkce_resp = client
        .post(format!("{}/token", server.url()))
        .form(&json!({
            "grant_type": "authorization_code",
            "code": "dummy_code",
            "code_verifier": "dummy_verifier",
            "client_id": "default-client",
            "redirect_uri": "http://localhost/callback"
        }))
        .send()
        .await?;
    assert_eq!(pkce_resp.status(), 200);
    let pkce_json: serde_json::Value = pkce_resp.json().await?;
    assert!(pkce_json["access_token"].is_string());

    // 5️⃣ Use the obtained token to call the mock LLM endpoint.
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .bearer_auth(access_token)
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "messages": [{ "role": "user", "content": "secure" }]
        }))
        .send()
        .await?;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["content"][0]["text"], "OAuth defaults response");
    Ok(())
}
```

### Rate‑limit error response ✅ New
Generate a 429 response with provider‑specific rate‑limit headers and verify them.
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // OpenAI example
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("rate limit")
                .with_error(429, "Too many requests")
                .for_provider(Provider::OpenAI), // provider‑specific handling
        )
        .build()
        .await?;

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "gpt-4",
            "messages": [{ "role": "user", "content": "rate limit" }]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 429);
    // OpenAI rate‑limit headers
    assert!(resp.headers().contains_key("x-ratelimit-limit-requests"));
    assert!(resp.headers().contains_key("x-ratelimit-remaining-requests"));
    assert!(resp.headers().contains_key("x-ratelimit-reset-requests"));
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["error"], "Too many requests");

    // Anthropic example
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("rate limit")
                .with_error(429, "Too many requests")
                .for_provider(Provider::Anthropic),
        )
        .build()
        .await?;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-2.1",
            "messages": [{ "role": "user", "content": "rate limit" }]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 429);
    // Anthropic rate‑limit headers
    assert!(resp.headers().contains_key("anthropic-ratelimit-requests-limit"));
    assert!(resp.headers().contains_key("anthropic-ratelimit-requests-remaining"));
    assert!(resp.headers().contains_key("anthropic-ratelimit-requests-reset"));
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["error"], "Too many requests");

    // Gemini example
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("rate limit")
                .with_error(429, "Too many requests")
                .for_provider(Provider::Gemini),
        )
        .build()
        .await?;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "gemini-1.0-pro",
            "messages": [{ "role": "user", "content": "rate limit" }]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 429);
    // Gemini rate‑limit header
    assert!(resp.headers().contains_key("retry-after"));
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["error"], "Too many requests");
    Ok(())
}
```

### Deterministic `x-request-id` header ✅ New
Every successful response includes a deterministic `x-request-id` header. The example checks its presence and format.
```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("header")
                .respond_with_content("Header test"),
        )
        .build()
        .await?;

    let client = Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&json!({
            "model": "claude-sonnet-4-6",
            "messages": [{ "role": "user", "content": "header" }]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 200);
    // The response must contain a deterministic request identifier.
    let request_id = resp
        .headers()
        .get("x-request-id")
        .expect("missing x-request-id")
        .to_str()
        .expect("invalid header value");
    // The ID follows the pattern "req-llmposter-{N}".
    assert!(request_id.starts_with("req-llmposter-"));
    // Optional: ensure the suffix parses as a number.
    let suffix = &request_id["req-llmposter-".len()..];
    assert!(suffix.parse::<u64>().is_ok());
    Ok(())
}
```

## Configuration
- **ServerBuilder**  
  - `bind` = `"127.0.0.1"`  
  - `port` = `2112`  
  - `verbose` = `false`  
  - No fixtures by default  
  - Authentication disabled by default  

- **FailureConfig**  
  - All fields `None` – no latency, no corruption, no truncation, no disconnect.  

- **StreamingConfig**  
  - `latency = None`, `chunk_size = None` – streaming disabled unless explicitly enabled.  

- **AuthState** (internal)  
  - Empty token map, no exhausted tokens.  

- **Environment variables**  
  - No required env vars; all configuration is done via the builder or YAML fixtures.  

- **Fixture file format**  
  - YAML, first‑match‑wins. Fields: `match`, `provider`, `response`, `error`, `failure`, `streaming`.  
  - Empty substring patterns are rejected at load time.  

## Pitfalls
### Wrong: Missing `.await` on an async builder
```rust
let server = ServerBuilder::new()
    .fixture(Fixture::new().respond_with_content("oops"))
    .build(); // ❌ forgot .await
```
### Right: Properly await the async `build` call
```rust
let server = ServerBuilder::new()
    .fixture(Fixture::new().respond_with_content("ok"))
    .build()
    .await?;
```

### Wrong: Enabling authentication but not providing a token
```rust
let server = ServerBuilder::new()
    .with_auth(true) // authentication required
    .fixture(Fixture::new().respond_with_content("secret"))
    .build()
    .await?;
```
### Right: Supply a bearer token that matches the server configuration
```rust
let server = ServerBuilder::new()
    .with_auth(true)
    .with_bearer_token("my-token")
    .fixture(Fixture::new().respond_with_content("secret"))
    .build()
    .await?;
```

### Wrong: Using an empty user‑message pattern (matches everything)
```rust
let fixture = Fixture::new()
    .match_user_message("") // ❌ empty substring – catches all requests
    .respond_with_content("fallback");
```
### Right: Provide a non‑empty, specific substring
```rust
let fixture = Fixture::new()
    .match_user_message("stock price")
    .respond_with_content("The price is $123");
```

### Wrong: Server starts with an empty pattern (runtime validation error)
```rust
let server = ServerBuilder::new()
    .fixture(
        Fixture::new()
            .match_user_message("") // empty pattern – should cause error
            .respond_with_content("should not start")
    )
    .build()
    .await; // ❌ this returns an Err
```
### Right: Server fails to start with a clear error
```rust
let result = ServerBuilder::new()
    .fixture(
        Fixture::new()
            .match_user_message("") // empty pattern – invalid
            .respond_with_content("invalid")
    )
    .build()
    .await;

assert!(result.is_err());
let err = result.unwrap_err();
assert!(err.to_string().contains("empty user‑message pattern"));
```

### Wrong: Using an excessively large regex pattern (exceeds 1 MiB DFA limit)
```rust
let huge_pattern = "a".repeat(2_000_000); // far exceeds the limit
let server = ServerBuilder::new()
    .fixture(
        Fixture::new()
            .match_user_message(&huge_pattern)
            .respond_with_content("won't start")
    )
    .build()
    .await; // ❌ should error
```
### Right: Server reports a regex‑size‑limit error
```rust
let huge_pattern = "a".repeat(2_000_000);
let result = ServerBuilder::new()
    .fixture(
        Fixture::new()
            .match_user_message(&huge_pattern)
            .respond_with_content("invalid")
    )
    .build()
    .await;

assert!(result.is_err());
let err = result.unwrap_err();
assert!(err.to_string().contains("regex size limit exceeded"));
```

## References
- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)