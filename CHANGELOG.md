# Changelog

## [Unreleased] — v0.4.2

### Fixed
- **Streaming Responses API** now includes `incomplete_details.reason` when
  `status` is `"incomplete"` — previously only non-streaming paths set it,
  breaking clients that branch on stop reason.
- **Streaming tool-call IDs** are now globally unique across requests —
  OpenAI and Anthropic streaming previously restarted at `_1` per request,
  causing ID collisions in multi-turn tool flows.
- **404 no-match error** no longer echoes user prompt text in the response
  body — prevents leaking secrets or PII in CI logs.
- `build_tool_call_stream_frames` trait now accepts `chunk_size` parameter
  (plumbing for future incremental tool-argument streaming).
- Stale `sdk_fixtures` output reference removed from CI changes job.

### Added
- **Windows CI** — `cargo test` on `windows-latest` runner. Pure Rust, no
  platform-specific code changes needed.
- CI: `/tmp` PID file path replaced with `${{ runner.temp }}` for cross-platform
  compatibility.
- Tests: streaming tool-call ID uniqueness (OpenAI + Anthropic), streaming
  `incomplete_details` (Responses API text + tool-call), 404 prompt redaction,
  Gemini 403/404/503/unknown error shapes, Anthropic 418 error type,
  exhausted token 401, non-bearer auth rejection, blank/image-only user
  message edge cases, CLI empty-dir warning, non-IP bind address, subdir
  skip in `load_yaml_dir`.

## [0.4.1] - 2026-03-28

### Added
- **`GET /code/:status`** — static HTTP status echo route (mini-httpbin built-in).
  Returns `{"code": N, "description": "..."}` with the requested status (100–599).
  3xx responses include `Location: /`. Invalid codes return 400. Auth-exempt.
- Gemini role-less REST payloads accepted: `contents[*]` items without `role` field
  are now treated as user turns (matches official single-turn Gemini examples).
- **SDK round-trip CI** — parallel matrix job (OpenAI + Anthropic + Gemini); validates that
  the real Python SDKs parse non-streaming, streaming, and tool call/use responses.
  Gemini SDK (`google-genai`) covers non-streaming and streaming text.
- Fixture validation warning when `truncate_after_frames` / `disconnect_after_ms`
  set on a non-streaming fixture (silent no-op would be confusing); fires for both
  `content` and `tool_calls` fixtures.
- SDK round-trip CI now also triggers on changes to `tests/fixtures/sdk-roundtrip*.py`
  and `tests/fixtures/sdk-roundtrip.yaml` (not just Rust source changes).
- `llmposter` startup in CI uses a retry loop instead of a fixed sleep for reliability.

### Fixed
- Gemini extractor no longer falls back to a stale prior user turn when the latest
  user turn has no text content — returns an error instead.
- Anthropic extractor: same stale-turn fix; tool_result follow-up still works.
- Anthropic: blank/whitespace latest user turn now fails fast instead of silently
  falling back and serving a stale fixture (was a P1 silent wrong-response bug).
- Responses API: `incomplete_details.reason` is now emitted when `status` is
  `"incomplete"` — clients can now assert why generation stopped.
- `GET /code/:status` is auth-exempt; bearer auth now only enforces on LLM endpoints.
- Non-boolean `stream` field (e.g. `"stream": "true"`) now returns 400 instead of
  silently treating as non-streaming — masks client serialization bugs (P2 Codex).

### Security
- `@claude` GitHub workflow trigger restricted to OWNER/MEMBER/COLLABORATOR
  (previously any commenter with repo access could invoke Claude with `id-token:write`).
- Homebrew tap push no longer embeds PAT in the `git clone` URL; token is set via
  `git remote set-url` after an anonymous clone to avoid credential exposure in logs.

## [0.4.0] - 2026-03-23

### Added
- **Bearer token authentication** on LLM endpoints — configurable via `with_auth(true)` + `with_bearer_token()`
- `expires_after_uses(N)` — deterministic token expiry after N LLM requests
- Provider-specific 401 responses (OpenAI, Anthropic, Gemini format)
- **OAuth 2.0 mock server** via `oauth-mock` integration (optional `oauth` feature, on by default)
  - Full PKCE + device code flow support
  - Token refresh, revocation, introspection
  - OIDC discovery + JWKS endpoints
  - OAuth-issued tokens automatically valid on LLM endpoints
- `with_oauth_defaults()` / `with_oauth(OAuthConfig)` on `ServerBuilder`
- `oauth_url()`, `oauth_client_credentials()`, `approve_device_code()` on `MockServer`
- **Published to crates.io** — `cargo install llmposter` or `cargo add llmposter --dev`
- `cargo-binstall` support — prebuilt binaries from GitHub Releases

### Changed
- MSRV bumped to 1.89 (required by oauth-mock)

## [0.3.6] - 2026-03-22

### Added
- Responses API full compliance: error shapes (429/500/400/401), request-id header, rate limit headers
- Globally unique tool-call IDs via server-wide counter (fixes multi-turn collisions)
- Auth error (401) spec tests for all 4 providers
- Empty regex pattern rejection in fixture validation
- IPv6 bind address test with graceful skip on unsupported hosts
- SSE disconnect test with latency for streaming truncation coverage
- Shared `parse_typed_sse` helper and `elapsed_ms` helper (DRY refactoring)
- `codecov.yml` excludes `src/main.rs`, targets aligned to 98%

### Fixed
- CLI `--bind` parsing uses `IpAddr::parse` (was misidentifying `host:port` as IPv6)
- `as_millis() as u64` replaced with safe `u64::try_from` (prevents silent truncation)
- CI coverage threshold aligned to 98% (was 96%, drifted from CLAUDE.md policy)
- Disconnect assertion tightened to validate actual truncation (was trivially true)
- Tool-call ID assertions use prefix + uniqueness checks (not fragile counter values)

## [0.3.5] - 2026-03-22

### Added
- `x-request-id` header on every response (deterministic `req-llmposter-{N}`)
- Provider-specific rate limit headers on 429 errors:
  - OpenAI/Responses: `x-ratelimit-{limit,remaining,reset}-requests`
  - Anthropic: `anthropic-ratelimit-requests-{limit,remaining,reset}`
  - Gemini: `retry-after` only (matches Google API behavior)
- `MockServer::check_error()` for surfacing post-bind server errors
- Anthropic `MessageDeltaUsage` now includes `input_tokens`, `cache_creation/read_input_tokens`
- Anthropic `ContentBlock::Text` gains `citations` field per spec
- Gemini `Content.role` now `Option<String>` (may be absent on safety-blocked responses)
- Gemini `Candidate` gains `index` and `safety_ratings` optional fields
- Gemini `GenerateContentResponse` gains `prompt_feedback`, `model_version` optional fields
- Error golden structs for Anthropic and Gemini with `deny_unknown_fields`
- 20 new spec tests: error shapes (3 Anthropic, 2 Gemini), request-id headers (3),
  rate limit headers (5 with value assertions), message_delta usage (2 text+tool-call),
  candidate index (1), validation (4), Gemini role:None round-trip (1)

### Fixed
- `#[serde(deny_unknown_fields)]` on all fixture YAML structs — typos are now caught at load time
- Empty substring match patterns rejected at validation (prevents silent catch-all)
- Anthropic rate limit reset is a future RFC 3339 timestamp (was hardcoded past date)
- Blank tool names rejected at fixture validation
- Error status restricted to 400-599 (was 100-599)
- Layer ordering: `DefaultBodyLimit` is now inner so 413s get `x-request-id`
- Fragile Value lookups in Responses handler replaced with explicit `expect()`

## [0.3.4] - 2026-03-21

### Fixed
- **RegexBuilder size limit** — DFA size capped at 1MB to prevent OOM from malicious fixture patterns
- **Tool-call argument validation** — rejects non-object arguments at fixture load time (Anthropic/Gemini require objects)
- **Responses API streaming protocol** — events now use nested `response` envelopes, include `sequence_number` and correlation fields, added `response.in_progress` event, removed non-spec `response.done`
- **Error response format** — matches real OpenAI error shape: `type` maps to error category, `code` is a string, `param` field present as null

### Added
- CLI output testability via `run_with_output()` — all status messages use `writeln!(writer)` instead of `eprintln!`
- Error response golden struct (`SpecErrorResponse`) for spec compliance testing
- 13 new tests: regex size limit (2), tool-call validation (3), CLI output capture (3), error shape (3), multiple tool calls (1), stop chunk empty delta (1)

## [0.3.3] - 2026-03-16

### Added
- `input_tokens_details` and `output_tokens_details` forward-compat stubs on `SpecResponsesUsage`
- `refusal` forward-compat stub on `SpecOutputMessage`
- `index` forward-compat stub on `SpecFunctionCallItem`

## [0.3.2] - 2026-03-16

### Added
- `#[serde(deny_unknown_fields)]` on all Gemini golden structs for strict compliance

### Changed
- `FunctionCallPart` cross-reference doc comment linking to golden struct

## [0.3.1] - 2026-03-16

### Added
- `cache_creation_input_tokens` and `cache_read_input_tokens` on Anthropic usage (emit as 0)
- `#[serde(deny_unknown_fields)]` on Anthropic golden structs for strict compliance
- Cache token assertion in Anthropic spec compliance tests

## [0.3.0] - 2026-03-16

### Added
- **OpenAI Chat Completions spec compliance test suite** — 18 tests validating every response field against the real API spec
- `system_fingerprint` field on `ChatCompletionResponse` and `ChatCompletionChunk` (`"fp_llmposter"`)
- `service_tier` field on `ChatCompletionResponse` and `ChatCompletionChunk` (`"default"`)
- `logprobs` field on `Choice` and `ChunkChoice` (always `null`)
- `refusal` field on `Message` and `Delta` (always `null`)
- `created` timestamp on streaming `ChatCompletionChunk` (was missing)
- Golden spec-faithful structs in `tests/spec/types/openai.rs` for compliance testing
- Spec URLs in doc comments on `src/format/openai.rs`

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
