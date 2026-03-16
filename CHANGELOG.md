# Changelog

## [0.2.0] - 2026-03-16

### Changed
- **BREAKING:** Handler DRY refactor — `ProviderHandler` trait replaces 4 duplicated handler implementations
- **BREAKING:** 404 no-match responses now use provider-specific error formats (was OpenAI-style for all)
- `truncate_after_chunks` renamed to `truncate_after_frames` (serde alias preserves backward compat)

### Added
- Provider-specific error bodies: Anthropic `{"type":"error","error":{...}}`, Gemini `{"error":{"code":...,"status":...}}`
- `has_explicit_reason` parameter for precise finish_reason/stop_reason control on streaming and non-streaming paths
- `created` timestamp field on OpenAI `ChatCompletionResponse`
- `tokio::select!` disconnect enforcement in both SSE and JSON-array streaming paths
- CLI testability: `cli::run()` extracted to public module with `Result<Option<MockServer>>` return
- 6 CLI integration tests (validate, error paths, server startup)
- CI/CD pipeline: auto-tag on version bump, cross-platform release builds, Homebrew tap, git-cliff release notes
- Pre-commit hooks: conventional commits, cargo fmt/clippy/check/test

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
