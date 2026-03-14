# Changelog

## [0.1.0] - 2026-03-14

### Added
- Initial release
- Mock server for 4 LLM API providers:
  - OpenAI Chat Completions (`/v1/chat/completions`)
  - Anthropic Messages (`/v1/messages`)
  - Gemini generateContent (`/v1beta/models/{model}:generateContent`)
  - OpenAI Responses API (`/v1/responses`)
- YAML fixture format with substring and regex matching
- First-match-wins fixture ordering
- Provider-agnostic fixtures with optional provider-specific overrides
- SSE streaming support for all providers
- Gemini streaming as JSON array (default) or SSE (`alt=sse`)
- Tool call responses for all providers
- Failure simulation:
  - HTTP error codes (429, 500, 503, etc.)
  - Response latency injection
  - Body corruption ("overloaded" text)
  - Stream truncation (cut off after N chunks)
  - Connection disconnect (drop after N ms)
- In-process Rust library with `ServerBuilder` API for `#[tokio::test]`
- Programmatic `Fixture` builder with fluent API
- CLI binary with `--fixtures`, `--validate`, `--port`, `--bind`, `--verbose` flags
- IPv4 and IPv6 support
- Deterministic response IDs for snapshot testing
- Approximate token usage estimation
