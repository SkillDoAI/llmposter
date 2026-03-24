---
name: llmposter
description: rust library
license: AGPL-3.0-or-later
metadata:
  version: "0.4.0"
  ecosystem: rust
  generated-by: skilldo/zai-glm-4.7 + review:zai-glm-4.7
---

## Imports

```rust
use llmposter::{Fixture, Provider, ServerBuilder};
use llmposter::fixture::{FailureConfig, ToolCall};
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

### Basic Message Mocking ✅ Current

Create a mock server that matches user messages and returns predefined responses.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from Claude mock!"),
        )
        .build()?;

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
    Ok(())
}
```

### Tool Use Mocking ✅ Current

Configure fixtures to return tool calls instead of text content.

```rust
use llmposter::fixture::ToolCall;
use llmposter::{Fixture, ServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "London", "unit": "celsius"}),
                }]),
        )
        .build()?;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "What's the weather in London?"}]
        }))
        .send()
        .await?;

    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["stop_reason"], "tool_use");
    Ok(())
}
```

### Streaming Response Simulation ✅ Current

Mock streaming responses with configurable latency and chunk sizes.

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hello world")
                .with_streaming(Some(0), Some(5)), // latency 0ms, chunk_size 5
        )
        .build()?;

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
    assert_eq!(
        resp.headers().get("content-type").unwrap().to_str().unwrap(),
        "text/event-stream"
    );
    Ok(())
}
```

### Provider and Model Filtering ✅ Current

Route requests based on provider and model patterns.

```rust
use llmposter::{Fixture, Provider, ServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_model("claude-sonnet")
                .respond_with_content("sonnet response")
                .for_provider(Provider::Anthropic),
        )
        .build()?;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 200);
    Ok(())
}
```

### Error and Failure Simulation ✅ Current

Simulate HTTP errors, latency, and connection failures.

```rust
use llmposter::fixture::FailureConfig;
use llmposter::{Fixture, ServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // HTTP error simulation
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("rate limit")
                .with_error(429, "Rate limit exceeded"),
        )
        .build()?;

    // Latency simulation
    let server_delayed = ServerBuilder::new()
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
        .build()?;

    Ok(())
}
```

## Configuration

### Server Defaults
- **Bind address**: `127.0.0.1:0` (random port assigned by OS)
- **Verbose logging**: `false`
- **Authentication**: `false` by default

### Fixture Defaults
- **Match patterns**: No filters (matches all requests if not specified)
- **Response**: Empty content unless configured
- **Streaming**: Disabled (`latency: None`, `chunk_size: None`)
- **Failure**: None (`FailureConfig::default()`)

### Common Customizations

**Enable verbose logging**:
```rust
ServerBuilder::new().verbose(true)
```

**Set specific bind address**:
```rust
ServerBuilder::new().bind("127.0.0.1:8080")
```

**Enable bearer token authentication**:
```rust
ServerBuilder::new()
    .with_auth(true)
    .with_bearer_token("test-token")
```

**Load fixtures from YAML**:
```rust
ServerBuilder::new().load_yaml(Path::new("fixtures.yaml"))?
```

## Pitfalls

### Empty Regex Patterns

**### Wrong**
```rust
Fixture::new()
    .match_user_message("")  // Empty regex - validation error
```

**### Right**
```rust
Fixture::new()
    .match_user_message(".+")  // Valid pattern
```

### Blank Tool Names

**### Wrong**
```rust
ToolCall {
    name: "".to_string(),  // Blank name rejected
    arguments: serde_json::json!({}),
}
```

**### Right**
```rust
ToolCall {
    name: "get_weather".to_string(),
    arguments: serde_json::json!({"location": "London"}),
}
```

### Missing Error Handling

**### Wrong**
```rust
let server = ServerBuilder::new()
    .fixture(Fixture::new())
    .build();  // Returns Result<MockServer>, error not handled
```

**### Right**
```rust
let server = ServerBuilder::new()
    .fixture(Fixture::new())
    .build()?;  // Properly handles the Result
```

### Fixture Validation

**### Wrong**
```rust
let fixture = Fixture::new()
    .with_error(429, "");  // Empty error message may fail validation
```

**### Right**
```rust
let mut fixture = Fixture::new()
    .with_error(429, "Rate limit exceeded");
fixture.validate()?;  // Explicit validation
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from 0.3.x

**Minimum Rust Version Update**

The Minimum Supported Rust Version (MSRV) has been bumped from 1.70 to 1.89.

**Before (0.3.x)**:
```toml
[package]
rust-version = "1.70"
```

**After (0.4.0)**:
```toml
[package]
rust-version = "1.89"
```

Update your Rust toolchain:
```bash
rustup update stable
rustup default stable
```

**Responses API Streaming Protocol (0.3.3 → 0.3.4)**

If upgrading from 0.3.3 or earlier and using the Responses API streaming endpoint, update your parsing logic:

- Events now use nested `response` envelopes
- Added `sequence_number` and correlation fields
- New event: `response.in_progress`
- Removed: `response.done`

## API Reference

**ServerBuilder::new()** - Creates a new server builder with default settings

**ServerBuilder::fixture()** - Adds a single fixture to the server configuration

**ServerBuilder::fixtures()** - Adds multiple fixtures at once

**ServerBuilder::bind()** - Sets the bind address for the server (default: random port)

**ServerBuilder::verbose()** - Enables verbose logging for request matching

**ServerBuilder::with_auth()** - Enables or disables authentication

**ServerBuilder::with_bearer_token()** - Sets a bearer token for authentication

**ServerBuilder::with_bearer_token_uses()** - Sets a bearer token with maximum usage limit

**ServerBuilder::load_yaml()** - Loads fixtures from a YAML file

**ServerBuilder::load_yaml_dir()** - Loads fixtures from all YAML files in a directory

**ServerBuilder::build()** - Builds and starts the mock server

**MockServer::url()** - Returns the URL where the server is listening

**Fixture::new()** - Creates a new empty fixture

**Fixture::match_user_message()** - Sets a pattern to match against user messages

**Fixture::match_model()** - Sets a pattern to match against the model field

**Fixture::respond_with_content()** - Sets the text content for the response

**Fixture::respond_with_tool_calls()** - Sets tool calls as the response

**Fixture::with_error()** - Configures an HTTP error response with status code and message

**Fixture::with_streaming()** - Enables streaming with optional latency and chunk size

**Fixture::with_failure()** - Configures failure injection (latency, corruption, truncation)

**Fixture::for_provider()** - Restricts the fixture to a specific provider (OpenAI, Anthropic, Gemini, Responses)

**ToolCall { name, arguments }** - Represents a tool call with name and JSON arguments

**FailureConfig** - Configuration for failure injection (latency_ms, corrupt_body, truncate_after_frames, disconnect_after_ms)

**Provider** - Enum representing LLM providers (OpenAI, Anthropic, Gemini, Responses)
