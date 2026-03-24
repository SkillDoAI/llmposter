---
name: llmposter
description: A mock server for testing LLM API interactions with support for multiple providers, streaming, tool calls, and failure simulation.
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
use llmposter::server::MockServer;
use reqwest::Client;
use serde_json::json;
use std::path::Path;
```

```toml
[dependencies]
llmposter = "0.4.0"
tokio = { features = ["full"], version = "1" }
reqwest = { features = ["json"], version = "0.13" }
serde_json = "1"
```

## Core Patterns

### Basic Mock Server with Text Response ✅ Current

```rust
mod basic_server {
    use llmposter::{Fixture, ServerBuilder};
    use reqwest::Client;

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("hello")
                    .respond_with_content("Hi from mock server!")
            )
            .build()
            .await?;

        let url = server.url();
        
        // Make a request to verify the server works
        let client = Client::new();
        let response = client
            .post(format!("{}/v1/chat/completions", url))
            .json(&serde_json::json!({
                "model": "gpt-3.5-turbo",
                "messages": [{"role": "user", "content": "hello"}]
            }))
            .send()
            .await?;

        let text = response.text().await?;
        
        // Verify the response matches expected output
        if text.contains("Hi from mock server!") {
            println!("✓ Test passed: Basic Mock Server with Text Response ✅ Current");
        } else {
            eprintln!("Expected 'Hi from mock server!' but got: {}", text);
            std::process::exit(1);
        }
        
        Ok(())
    }
}
```

Creates a mock server that responds to messages matching "hello" with a text response. The builder pattern allows chaining multiple fixtures and configuration options before building the server.

### Streaming Response ✅ Current

```rust
mod streaming_example {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("stream")
                    .respond_with_content("Streaming response")
                    .with_streaming(Some(0_u64), Some(5_usize))
            )
            .build()
            .await?;

        println!("Streaming server at: {}", server.url());
        Ok(())
    }
}
```

Configures a fixture to stream responses using Server-Sent Events (SSE). The latency parameter controls delay between chunks in milliseconds, and chunk_size determines the size of each chunk in bytes. Both parameters are optional.

### Tool Call Response ✅ Current

```rust
mod tool_calls_example {
    use llmposter::fixture::ToolCall;
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("weather")
                    .respond_with_tool_calls(vec![
                        ToolCall {
                            name: "get_weather".to_string(),
                            arguments: serde_json::json!({"location": "London"}),
                        }
                    ])
            )
            .build()
            .await?;

        println!("Tool call server at: {}", server.url());
        Ok(())
    }
}
```

Returns tool calls instead of text content when the user message matches. The `ToolCall` struct requires a `name` (String) and `arguments` (serde_json::Value). This is useful for testing applications that integrate with function/tool calling capabilities.

### Error Simulation ✅ Current

```rust
mod error_simulation {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("rate limit")
                    .with_error(429, "Rate limit exceeded")
            )
            .build()
            .await?;

        println!("Error simulation server at: {}", server.url());
        Ok(())
    }
}
```

Simulates HTTP error responses with custom status codes and messages. The error format matches OpenAI's error response shape, making it suitable for testing error handling in LLM client applications.

### Provider Filtering ✅ Current

```rust
mod provider_filtering {
    use llmposter::{Fixture, Provider, ServerBuilder};

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("Anthropic response")
                    .for_provider(Provider::Anthropic)
            )
            .fixture(
                Fixture::new()
                    .respond_with_content("OpenAI response")
                    .for_provider(Provider::OpenAI)
            )
            .build()
            .await?;

        println!("Provider-aware server at: {}", server.url());
        Ok(())
    }
}
```

Routes requests to different fixtures based on the provider (Anthropic, OpenAI, Gemini, Responses). Requests that don't match any fixture's provider criteria return 404. This enables testing multi-provider applications with a single mock server instance.

## Configuration

### Server Configuration

- **Default bind address**: `127.0.0.1:0` (random port assigned by OS)
- **Verbose logging**: Disabled by default, enable with `.verbose(true)` for debugging fixture matching
- **Authentication**: Disabled by default; enable with `.with_auth(true)` and configure tokens via `.with_bearer_token()`

### Fixture Matching

- **User message matching**: Uses substring matching via `.match_user_message(pattern)` - partial matches succeed
- **Model matching**: Uses substring matching via `.match_model(pattern)` - useful for version-specific responses
- **Provider filtering**: Exact match via `.for_provider(provider)` - one of `Provider::OpenAI`, `Provider::Anthropic`, `Provider::Gemini`, or `Provider::Responses`

### Failure Simulation

Configure `FailureConfig` to simulate various failure modes:
- `latency_ms: Option<u64>` - Delay response by specified milliseconds
- `corrupt_body: Option<bool>` - Return corrupted response body instead of configured content
- `truncate_after_frames: Option<u32>` - Truncate streaming response after N frames (useful for testing incomplete stream handling)
- `disconnect_after_ms: Option<u64>` - Close connection abruptly after specified milliseconds

### Streaming Configuration

Configure streaming via `.with_streaming(latency, chunk_size)`:
- `latency: Option<u64>` - Delay between chunks in milliseconds
- `chunk_size: Option<usize>` - Size of each chunk in bytes

### YAML Configuration

Fixtures can be loaded from YAML files or directories:
- `.load_yaml(path: &Path)` - Loads fixtures from a single YAML file
- `.load_yaml_dir(dir: &Path)` - Loads all YAML files from a directory

## Pitfalls

### Wrong: Forgetting to await build()

```rust
mod wrong_build {
    use llmposter::{Fixture, ServerBuilder};

    fn main() -> Result<(), Box<dyn std::error::Error>> {
        // This will fail because build() returns a future that must be awaited
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("test"))
            .build()?;
        Ok(())
    }
}
```

### Right: Awaiting the build result

```rust
mod right_build {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("test"))
            .build()
            .await?;
        Ok(())
    }
}
```

### Wrong: Creating fixtures without match criteria

```rust
mod wrong_fixture {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                // No match criteria - may not match any request, returns 404
                Fixture::new()
                    .respond_with_content("unmatched response")
            )
            .build()
            .await?;
        Ok(())
    }
}
```

### Right: Adding match criteria to fixtures

```rust
mod right_fixture {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("hello")  // Match criteria required
                    .respond_with_content("matched response")
            )
            .build()
            .await?;
        Ok(())
    }
}
```

### Wrong: Using incorrect streaming parameter types

```rust
mod wrong_streaming {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("streaming")
                    // Type error: expects Option<u64>, Option<usize>
                    .with_streaming(0, 5)
            )
            .build()
            .await?;
        Ok(())
    }
}
```

### Right: Using Option types for streaming parameters

```rust
mod right_streaming {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("streaming")
                    .with_streaming(Some(0), Some(5))  // Correct: wrapped in Some()
            )
            .build()
            .await?;
        Ok(())
    }
}
```

### Wrong: Not handling build errors

```rust
mod wrong_error_handling {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::main]
    async fn main() {
        // This will panic if bind fails or ports are exhausted
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("test"))
            .build()
            .await
            .unwrap();
    }
}
```

### Right: Proper error handling with Result propagation

```rust
mod right_error_handling {
    use llmposter::{Fixture, ServerBuilder};

    #[tokio::main]
    async fn main() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("test"))
            .build()
            .await?;
        Ok(())
    }
}
```

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://github.com/SkillDoAI/llmposter)
- [Documentation](https://docs.rs/llmposter)

## Migration from 0.3

### Breaking Changes in 0.4.0

**MSRV Bump to 1.89**
The Minimum Supported Rust Version (MSRV) was bumped from 1.70 to 1.89.

**Migration**: Update your `rust-toolchain.toml` or CI configuration to use Rust 1.89 or later:

```toml
[toolchain]
channel = "1.89"
```

### Breaking Changes in 0.3.4 (from 0.3.3)

**Responses API Streaming Protocol Change**
Streaming events now use nested 'response' envelopes, include 'sequence_number' and correlation fields, added 'response.in_progress' event, and removed non-spec 'response.done'.

**Migration**: Update streaming consumers to handle nested 'response' envelopes and 'sequence_number'. Replace handling of 'response.done' with 'response.in_progress' or the final event.

**Error Response Format Change**
Error response format changed to match real OpenAI error shape. 'type' now maps to error category, 'code' is a string, and 'param' field is present as null.

**Migration**: Update error parsing logic to expect the new OpenAI-compatible shape (string 'code', 'type' mapping).

## API Reference

- **ServerBuilder::new()** - Creates a new server builder instance with default configuration.
- **ServerBuilder::fixture(f: Fixture)** - Adds a single fixture to the server; can be chained.
- **ServerBuilder::fixtures(fixtures: Vec<Fixture>)** - Adds multiple fixtures to the server in one call.
- **ServerBuilder::bind(addr: &str)** - Sets the bind address for the server (default: random port on 127.0.0.1).
- **ServerBuilder::verbose(v: bool)** - Enables verbose logging for debugging fixture matching failures.
- **ServerBuilder::with_auth(enabled: bool)** - Enables authentication for all server endpoints.
- **ServerBuilder::with_bearer_token(token: &str)** - Sets a bearer token for authentication (unlimited uses).
- **ServerBuilder::with_bearer_token_uses(token: &str, max_uses: u64)** - Sets a bearer token with maximum usage limit.
- **ServerBuilder::build()** - Builds and starts the mock server, returning `Result<MockServer, Box<dyn Error>>`.
- **MockServer::url()** - Returns the full URL where the mock server is listening.
- **Fixture::new()** - Creates a new fixture with default configuration.
- **Fixture::match_user_message(pattern: &str)** - Sets the user message pattern to match (substring matching).
- **Fixture::match_model(pattern: &str)** - Sets the model name pattern to match (substring matching).
- **Fixture::respond_with_content(content: &str)** - Sets the text content for the response.
- **Fixture::respond_with_tool_calls(tool_calls: Vec<ToolCall>)** - Sets tool calls as the response instead of text.
- **Fixture::with_error(status: u16, message: &str)** - Configures the fixture to return an HTTP error with the specified status and message.
- **Fixture::with_streaming(latency: Option<u64>, chunk_size: Option<usize>)** - Enables streaming with configurable latency and chunk size.
- **Fixture::with_failure(failure: FailureConfig)** - Configures failure simulation (latency, corruption, truncation, disconnection).
- **Fixture::for_provider(provider: Provider)** - Restricts the fixture to match only requests from the specified provider.