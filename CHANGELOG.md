# Changelog

## [0.4.5] - TBD

### Added
- **`ServerBuilder::capture_capacity(max)`**: cap the captured-request log
  at `max` entries with FIFO trimming. Defaults to unbounded (pre-v0.4.5
  behavior) so short `#[tokio::test]` servers still see every entry;
  long-lived standalone servers should set a cap to bound memory. Setting
  `capture_capacity(0)` disables capture entirely.
- **Safety refusal fixtures.** New top-level `refusal:` fixture block
  produces provider-specific refusal-shape responses across all four
  supported APIs. Tests that exercise client-side refusal handling no
  longer need to hand-roll upstream payloads.
  - **OpenAI Chat Completions**: `message.refusal: "<text>"` with
    `content: null`, `finish_reason: "stop"`.
  - **Anthropic Messages**: text content block carrying the refusal
    body and `stop_reason: "refusal"`.
  - **Gemini `generateContent`**: empty `candidates` array plus
    `promptFeedback.blockReason: "SAFETY"` and a `safetyRatings` entry.
  - **OpenAI Responses API**: message output item containing a single
    `type: "refusal"` content part; top-level `status: "completed"`.

  `refusal:` is mutually exclusive with `response:`, `error:`, and
  `failure:`. `Fixture::respond_with_refusal` and
  `respond_with_refusal_category` expose the same shape from the
  programmatic builder. The new `Refusal` type is re-exported from the
  crate root.
- **`RequestOutcome` on captured requests.** `CapturedRequest` gains an
  `outcome: RequestOutcome` field and a `was_matched()` convenience
  method. Captures now cover five cases: `Matched` (a fixture was
  selected — includes `response:`, `error:`, `refusal:`, and streaming
  chaos, regardless of the final HTTP status), `NoFixtureMatch` (404),
  `BadRequest` (malformed JSON, failed request extraction, or an
  invalid `/code/{status}` path), `AuthRejected` (401 from the bearer
  middleware), and `CodeEndpoint` (valid `/code/{status}` hit). Auth
  rejections capture path/method/outcome but an empty body — the auth
  middleware does not buffer the request body to stay off the hot
  path. The Gemini capture now records the *real*
  `/v1beta/models/{model}:{action}` path instead of the router
  wildcard, and Gemini's path-parse / unknown-action 400s are captured
  too. `CapturedRequest` is marked `#[non_exhaustive]` so future fields
  can land without a semver break.

### Performance
- **Matched fixtures are now `Arc<Fixture>` internally.** `AppState.fixtures`
  stores `Vec<Arc<Fixture>>` and handlers `Arc::clone` the matched fixture
  out of the read lock instead of deep-cloning the full struct. Eliminates
  per-request heap traffic for fixtures with large tool-call `arguments`
  blobs or multi-KB `content` strings. Public API is unchanged —
  `ServerBuilder::fixture`, `ServerBuilder::fixtures`, and
  `MockServer::set_fixtures` still take `Fixture` / `Vec<Fixture>`.
- **Templated responses now cache their compiled minijinja template.** The
  first render of a `content_template` fixture compiles and caches the
  template inside the fixture's new `TemplateCache`; every subsequent
  request reuses the cached `Environment` and pays only the render cost.
  Templates with compile errors cache the error message on first attempt
  so repeated requests against a broken template return instantly. Hot
  reloading a fixture resets its cache (new `Arc<Fixture>` → fresh
  `TemplateCache`). No public API changes for users who construct
  `FixtureResponse` via `..Default::default()`; `TemplateCache` is a new
  opaque type in `llmposter::fixture`.

### Security
- **`generate-skill.yml` now pins `SkillDoAI/skilldo-action` to a commit
  SHA** instead of the mutable `@v1` tag. Matches the pinning policy
  used by the other workflows in this repo — a retag of `v1` upstream
  can no longer run with `contents: write` + `ANTHROPIC_API_KEY` in
  this repository.

### Fixed
- **Gemini JSON-array streaming no longer sleeps after the final frame.**
  `stream_json_array` now mirrors `stream_sse_frames`: the inter-frame
  delay is skipped once the last frame has been collected. Removes a
  wasted `base_latency` from every JSON-array response and closes an
  edge case where `disconnect_after_ms` could pop an already-buffered
  final frame during the post-final sleep.
- **`corrupt_body` now honors streaming mode.** For streaming SSE
  responses (OpenAI, Anthropic, Responses API, Gemini `alt=sse`), a
  fixture with `corrupt_body: true` now returns a single malformed SSE
  frame (`data: overloaded\n\n`) with `Content-Type: text/event-stream`,
  so clients testing "mid-stream garbage" observe the corruption through
  their SSE parser instead of a wrong-Content-Type text/plain body.
  Non-streaming requests and Gemini's JSON-array streaming mode continue
  to return `text/plain overloaded`.
- **Content extractors now uniformly reject blank user messages.** OpenAI,
  Gemini, and Responses API prompt extraction previously could return
  `Ok("")` for whitespace-only string content or an array whose text parts
  all trimmed to empty, which silently matched any fixture with an empty
  substring match rule. All four providers now trim and reject blank
  content, matching Anthropic's behavior since v0.4.3. Additionally, the
  Responses API array-content path now filters strictly on
  `type == "input_text"` so a stray `text` field on an `input_image` or
  other non-text part can no longer leak into the extracted prompt.
- **Hot-reload workers exit promptly on `MockServer::drop`.** Both the file
  watcher thread and SIGHUP tokio task now poll their `Weak<AppState>` every
  500ms (via `recv_timeout` and a `tokio::select!` interval tick
  respectively) and exit cleanly within ~500ms of the server being dropped,
  even if no filesystem event or signal ever arrives. Before this fix, an
  idle watcher thread or SIGHUP task could linger for the rest of the
  process lifetime, accumulating idle threads and file descriptors in
  long-running test processes that churn through many short-lived
  `MockServer` instances.

## [0.4.4] - 2026-04-10

### Added
- **Hot-reload fixtures** — three paths to swap fixtures into a running server
  without restarting:
  - `MockServer::set_fixtures(Vec<Fixture>)` validates and atomically swaps
    the live fixture list. Invalid fixtures leave the old list unchanged.
  - `ServerBuilder::watch(true)` and CLI `--watch`/`-w` enable a file
    watcher (notify-debouncer-mini, ~250ms debounce) that picks up edits to
    any YAML source loaded via `load_yaml` / `load_yaml_dir`. New `watch`
    Cargo feature (on by default) gates the dependency.
  - On Unix, `kill -HUP <pid>` always triggers a reload for file-backed
    fixtures, matching traditional daemon conventions. The CLI startup
    line prints the exact command.
  - Parse or validation failures during reload are logged and leave the
    previously loaded fixtures serving — partial edits never take down
    the live server.
- **Streaming chaos** — four new `failure:` fields for reproducible
  randomized streaming behavior:
  - `latency_jitter_ms` adds signed jitter `[-range, +range]` to each
    per-frame streaming delay (clamps to zero on the low end).
  - `duplicate_frames` emits every SSE frame twice back-to-back.
  - `probability` (default 1.0) gates chaos activation on a dice roll.
    Classical failures (`latency_ms`, `corrupt_body`, truncate, disconnect)
    are not gated by probability and always fire when set.
  - `chaos_seed` overrides the PRNG seed; without it the seed is derived
    from an internal per-server request counter so successive requests
    in the same test produce deterministic-but-distinct chaos outcomes.
  - Same `chaos_seed` + same request order = bit-identical chaos, so
    flaky tests caused by jitter are impossible.
- **Response templating** — new `content_template` fixture field (gated by
  the optional `templating` Cargo feature, off by default) renders a
  Jinja-style template at response time with access to `user_message`,
  `model`, `provider`, and the full parsed `request` JSON. Mutually
  exclusive with `content` and `tool_calls`. Render errors surface as
  HTTP 500 without taking down the server.
- `MockServer::fixture_count` / `ServerBuilder::fixture_count` for test
  and CLI code that needs to know how many fixtures were loaded.

### Changed
- `AppState.fixtures` is now `RwLock<Vec<Fixture>>` internally to support
  atomic hot-reload. Public API is unchanged — programmatic callers
  continue to use the builder and `MockServer` methods as before.
- Internal streaming helpers now take a per-frame `Vec<u64>` of delays
  instead of a single `latency: u64`. Passthrough behavior is preserved
  byte-for-byte when no chaos is configured.
- `FixtureResponse` and `FailureConfig` both derive `Default`. Existing
  struct literals across the test suite were updated to use
  `..Default::default()` for the new optional fields.

### Docs
- `docs/cli.md` gains a "Hot Reload" section covering `--watch` and SIGHUP.
- `docs/library.md` documents `MockServer::set_fixtures`, `ServerBuilder::watch`,
  and `ServerBuilder::fixture_count` with programmatic and file-watcher
  examples, plus a SIGHUP note.
- `docs/failure-simulation.md` gains a "Streaming Chaos" section with field
  reference, determinism guarantees, and two worked YAML examples.
- `docs/fixtures.md` gains a "Templated response" subsection with the
  template context table and validation rules.

## [0.4.3] - 2026-04-09

### Added
- **Stateful scenarios** — multi-turn fixture matching via named state machines.
  Fixtures can require a specific state to match and advance the state after
  matching, enabling tool-call loops, retry sequences, and conversation branching.
  YAML `scenario:` block and `Fixture::with_scenario()` builder.
- **Request capture API** — `server.get_requests()` returns all captured requests
  for test assertions. Verify what your client sent, not just what it received.
  `server.request_count()` and `server.reset()` for test lifecycle management.
- `MockServer::scenario_state(name)` to query scenario state at any point.
- Streaming `function_call_arguments.done` event now includes `name` field
  per OpenAI Responses spec.
- `/code/205` returns empty body (Reset Content is bodyless per HTTP spec).

### Fixed
- **`disconnect_after_ms` now simulates real transport failure** — injects
  `ConnectionReset` error into the SSE stream instead of clean EOF, so clients
  testing retry-on-disconnect see actual broken streams.
- **Anthropic extractor rejects non-text latest user turn** — messages with
  non-string/non-array content (null, object, number) or missing `content`
  field now return 400 instead of silently falling back to an earlier turn.

## [0.4.2] - 2026-04-07

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
