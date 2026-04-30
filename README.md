# llmposter

[![Crates.io](https://img.shields.io/crates/v/llmposter)](https://crates.io/crates/llmposter)
[![CI](https://github.com/SkillDoAI/llmposter/actions/workflows/ci.yml/badge.svg)](https://github.com/SkillDoAI/llmposter/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/SkillDoAI/llmposter/branch/main/graph/badge.svg)](https://codecov.io/gh/SkillDoAI/llmposter)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue)](LICENSE)

**Test your LLM apps without burning tokens, waiting on rate limits, or chasing flaky network errors.**

llmposter is a deterministic mock server for LLM APIs. It speaks OpenAI, Anthropic, Gemini, and the OpenAI Responses API — streaming included. Point your existing client at it and get repeatable, fixture-driven responses for every test run. No API keys. No network. No flakes.

Built in Rust as a library (`cargo add llmposter --dev`) or a standalone CLI binary. Single binary, zero runtime dependencies.

### Who is this for?

- **Backend teams** testing LLM integrations in CI without API keys or network access
- **Agent developers** validating tool-call loops, retries, and multi-turn flows against deterministic fixtures
- **Platform engineers** testing error handling, rate limiting, and graceful degradation
- **Anyone** tired of flaky LLM tests, surprise token bills, or mocking HTTP by hand

## What it does

**📦 Rust library *or* standalone CLI** — Use it in-process with `cargo add llmposter --dev` for Rust tests, or run the `llmposter` CLI binary for language-agnostic testing, local development, and CI pipelines. Same engine, same fixtures, same behavior.

**🎯 Speaks 4 real LLM API formats** — OpenAI Chat Completions, Anthropic Messages, Gemini generateContent, and OpenAI Responses API. Your client code doesn't change — just swap the base URL.

**📡 Full streaming support** — SSE for OpenAI/Anthropic/Responses, JSON-array + SSE modes for Gemini. Streaming tool calls included. Per-frame latency and chunk size configurable.

**🧪 Fixture-driven** — Define request → response pairs in YAML or with a fluent builder API. Match on user message, model, provider, request headers, system prompt, temperature (exact or range), metadata, tool schema, or an RFC 9535 JSONPath expression against the full body. Priority-sorted first-match-wins with explicit catch-all fallback. Validates at load time so typos don't survive to runtime.

**🛠️ Tool calling** — Mock tool-use responses with full type fidelity. Globally unique tool-call IDs across requests. Works with multi-turn agent flows.

**💥 Failure injection** — Simulate real-world LLM pain: rate limits (429), server errors (5xx), latency, body corruption, mid-stream truncation, and genuine `ConnectionReset` transport disconnects. Test your retry logic, backoff, and error handling against realistic failure modes.

**🌀 Streaming chaos** — Seeded jitter (`latency_jitter_ms`), duplicated SSE frames, and probabilistic activation (`probability`, `chaos_seed`). Randomized *but reproducible* — same seed + same request order = bit-identical chaos, so jitter-flavored tests never go flaky.

**🔁 Stateful multi-turn scenarios** — Named state machines for tool-call loops, retry sequences, and conversation branching. A fixture can require a specific state to match and advance the state on match — ideal for agent testing.

**♻️ Hot-reload fixtures** — Edit a YAML file and the running server picks up changes automatically with `--watch`, or send `kill -HUP <pid>` like a traditional daemon. Invalid YAML leaves the previous fixtures serving — partial edits never take down the server.

**🧵 Response templating** — Render fixture responses through a Jinja-style template (`content_template`) at request time with access to `user_message`, `model`, `provider`, and the full request JSON. Behind the optional `templating` feature.

**🔎 Request capture & assertion** — Every request is captured. Call `server.get_requests()` to verify what your client actually sent. Asserts that complement your response testing.

**🔐 Authentication testing** — Bearer token auth with use-count expiration. Full OAuth 2.0 mock server (PKCE, device flow, refresh, revocation, OIDC discovery) behind a feature flag. Provider-specific 401 error shapes.

**🚦 HTTP status echo** — `GET /code/200`, `GET /code/429`, etc. Mini-httpbin built in. Test client behavior against any HTTP status without writing a fixture.

**⚡ Fast and deterministic** — Fixed IDs, sequential counters, and no hidden nondeterminism. Optional chaos is seeded and reproducible. Rust async throughout — each `ServerBuilder::build()` spawns a lightweight axum server on an OS-assigned port, so every `#[tokio::test]` gets its own isolated mock.

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
| `GET /v1/models` | Model list (auto-derived from fixtures) |
| `GET /health` | Health check (`{"status": "ok"}`) |
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
- [Authentication](docs/authentication.md) — Bearer tokens, OAuth mock server
- [Stateful Scenarios](docs/scenarios.md) — Multi-turn matching, tool-call loops, retries
- [Request Capture](docs/request-capture.md) — Assert what your client sent
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
