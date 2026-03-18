# llmposter

A Rust crate + CLI for mocking LLM API endpoints. Fixture-driven, deterministic responses for testing.

Speaks 4 LLM API formats — OpenAI Chat Completions, Anthropic Messages, Gemini generateContent, and OpenAI Responses API — with SSE streaming and failure simulation.

Inspired by [llmock](https://github.com/CopilotKit/llmock). Built in Rust with zero runtime dependencies for users.

## Quick Start (Library)

Add to your `Cargo.toml`:

```toml
[dev-dependencies]
llmposter = "0.3"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
reqwest = "0.12"
serde_json = "1"
```

```rust
use llmposter::{ServerBuilder, Fixture};

#[tokio::test]
async fn test_llm_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from the mock!")
        )
        .build()
        .await
        .unwrap();

    // Point your LLM client at server.url()
    let url = format!("{}/v1/chat/completions", server.url());
    // ... make requests, get deterministic responses
    // Server shuts down when dropped
}
```

## Quick Start (CLI)

```bash
# Install via Homebrew
brew install SkillDoAI/tap/llmposter

# Or install via Cargo
cargo install llmposter

# Create fixtures
cat > fixtures.yaml << 'EOF'
fixtures:
  - match:
      user_message: "hello"
    response:
      content: "Hi from the mock!"
EOF

# Run server
llmposter --fixtures fixtures.yaml --port 8080

# Point your app at http://127.0.0.1:8080
```

## Fixture Format (YAML)

```yaml
fixtures:
  # Simple text response
  - match:
      user_message: "stock price"
    response:
      content: "AAPL is $150.42"

  # Regex match with streaming config
  - match:
      user_message:
        regex: "stock price of \\w+"
    response:
      content: "I can help with stock prices."
    streaming:
      latency: 50
      chunk_size: 20

  # Error simulation
  - match:
      model: "fail-model"
    error:
      status: 429
      message: "Rate limit exceeded"

  # Failure simulation
  - match:
      user_message: "slow"
    response:
      content: "This will be delayed"
    failure:
      latency_ms: 5000

  # Catch-all (no match criteria)
  - response:
      content: "Default response"
```

## Supported Endpoints

| Route | Provider |
|-------|----------|
| `POST /v1/chat/completions` | OpenAI |
| `POST /v1/messages` | Anthropic |
| `POST /v1/responses` | OpenAI Responses API |
| `POST /v1beta/models/{model}:generateContent` | Gemini |
| `POST /v1beta/models/{model}:streamGenerateContent` | Gemini (streaming) |

All providers support both streaming and non-streaming responses.
Gemini streaming uses JSON arrays by default, with SSE available via `?alt=sse`.

## Failure Simulation

| Fixture Field | Effect |
|---------------|--------|
| `error.status` | Return HTTP error (429, 500, 503, etc.) |
| `failure.latency_ms` | Delay response by N ms |
| `failure.corrupt_body` | Return "overloaded" plain text |
| `failure.truncate_after_frames` | Cut stream after N SSE frames |
| `failure.disconnect_after_ms` | Drop connection after N ms |

## CLI Options

```text
llmposter --fixtures <PATH>  Path to YAML file or directory
          --validate         Validate fixtures without starting
          --port <PORT>      Port (default: 2112)
          --bind <ADDR>      Bind address (default: 127.0.0.1)
          --verbose          Log requests to stderr
```

## License

AGPL-3.0
