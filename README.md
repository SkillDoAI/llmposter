# llmposter

A Rust crate + CLI for mocking LLM API endpoints. Fixture-driven, deterministic responses for testing.

Speaks 4 LLM API formats — OpenAI Chat Completions, Anthropic Messages, Gemini generateContent, and OpenAI Responses API — with SSE streaming and failure simulation.

Inspired by [llmock](https://github.com/CopilotKit/llmock). Built in Rust with zero runtime dependencies for users.

## Quick Start (Library)

```toml
[dev-dependencies]
llmposter = "0.4"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
reqwest = "0.13"
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

## Supported Providers

| Route | Provider |
|-------|----------|
| `POST /v1/chat/completions` | OpenAI Chat Completions |
| `POST /v1/messages` | Anthropic Messages |
| `POST /v1/responses` | OpenAI Responses API |
| `POST /v1beta/models/{model}:generateContent` | Gemini |
| `POST /v1beta/models/{model}:streamGenerateContent` | Gemini (streaming) |
| `GET /code/200` (any 100–599) | HTTP status echo (mini-httpbin) |

All providers support streaming and non-streaming. For OpenAI, Anthropic, and Responses API, just swap the base URL — the paths are identical to the real APIs. Gemini uses separate endpoints for streaming (`streamGenerateContent`) and non-streaming (`generateContent`).

## Authentication

Bearer token enforcement on LLM endpoints — off by default, fully backward compatible.

```rust
let server = ServerBuilder::new()
    .with_bearer_token("test-token-123")          // valid forever
    .with_bearer_token_uses("short-lived", 1)     // expires after 1 use
    .fixture(Fixture::new().respond_with_content("hello"))
    .build().await.unwrap();

// Requests must include: Authorization: Bearer test-token-123
```

### OAuth 2.0 Mock Server

Full OAuth server via `oauth-mock` integration — PKCE, device code, token refresh, revocation.

```rust
let server = ServerBuilder::new()
    .with_oauth_defaults()  // spawns OAuth server on separate port
    .fixture(Fixture::new().respond_with_content("hello"))
    .build().await.unwrap();

let oauth_url = server.oauth_url().unwrap();  // e.g. http://127.0.0.1:12345
// Point your client's token_url at oauth_url
// Tokens issued by the OAuth server are automatically valid on LLM endpoints
```

## Documentation

- [Getting Started](docs/getting-started.md) — Installation, first fixture, first test
- [Fixtures](docs/fixtures.md) — YAML format, matching rules, tool calls
- [Failure Simulation](docs/failure-simulation.md) — Error codes, latency, truncation, disconnect
- [CLI Reference](docs/cli.md) — Flags, validate mode, verbose logging
- [Library API](docs/library.md) — Rust `ServerBuilder`, programmatic fixtures
- [Spec Deviations](docs/spec-deviations.md) — Known gaps from real APIs

### Provider Guides

- [OpenAI Chat Completions](docs/providers/openai.md) — Fields, streaming, error shapes
- [Anthropic Messages](docs/providers/anthropic.md) — Fields, streaming, error shapes
- [Gemini generateContent](docs/providers/gemini.md) — Fields, streaming, camelCase
- [OpenAI Responses API](docs/providers/responses.md) — Fields, streaming events, envelopes

## License

AGPL-3.0
