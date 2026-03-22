# Getting Started

## Installation

### Homebrew (macOS / Linux)

```bash
brew install SkillDoAI/tap/llmposter
```

### Cargo

```bash
cargo install llmposter
```

### As a dev dependency (Rust library)

```toml
[dev-dependencies]
llmposter = "0.3"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
reqwest = "0.12"
serde_json = "1"
```

## Your First Fixture

Create a file called `fixtures.yaml`:

```yaml
fixtures:
  - match:
      user_message: "hello"
    response:
      content: "Hi from the mock!"
```

## Running the Server (CLI)

```bash
llmposter --fixtures fixtures.yaml --port 8080
```

Point your LLM client at `http://127.0.0.1:8080` instead of the real API. No path changes needed — llmposter uses the same routes as the real providers.

## Running In-Process (Rust Tests)

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
    // Server shuts down when dropped
}
```

## Supported Providers

| Route | Provider |
|-------|----------|
| `POST /v1/chat/completions` | OpenAI Chat Completions |
| `POST /v1/messages` | Anthropic Messages |
| `POST /v1/responses` | OpenAI Responses API |
| `POST /v1beta/models/{model}:generateContent` | Gemini |
| `POST /v1beta/models/{model}:streamGenerateContent` | Gemini (streaming) |

All providers support both streaming and non-streaming responses. Just swap the base URL — no path changes needed.

## Next Steps

- [Fixture format reference](fixtures.md) — matching rules, tool calls, provider overrides
- [Failure simulation](failure-simulation.md) — error codes, latency, disconnect
- [Provider-specific guides](providers/) — streaming events, field details, spec compliance
