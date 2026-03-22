# llmposter — Design Spec

## Purpose

A Rust crate + CLI for mocking LLM API endpoints. Fixture-driven, deterministic responses for testing. Speaks 4 provider formats with streaming and non-streaming support, plus full failure simulation.

Primary consumer: the `skilldo` project (same repo owner), which has LLM clients for Anthropic, OpenAI, Gemini, and OpenAI Responses API. llmposter replaces external service dependencies in tests.

Inspired by [llmock](https://github.com/CopilotKit/llmock) (Node/TS), but written in Rust with its own YAML fixture format, own test suite, and different architecture.

---

## Crate Structure

Single Rust crate, dual-target:

- `src/lib.rs` — Public API: server builder, fixture types, re-exports
- `src/main.rs` — CLI binary via `clap`, thin wrapper around the library

```
src/
  lib.rs
  main.rs
  fixture.rs          # YAML parsing, match logic (first-match-wins)
  server.rs           # axum router setup, shared state
  stream.rs           # SSE chunker — data: frames with configurable timing
  failure.rs          # Error responses, truncation, disconnect, latency, corruption
  handler/
    mod.rs
    openai.rs         # POST /v1/chat/completions
    anthropic.rs      # POST /v1/messages
    gemini.rs         # POST /v1beta/models/{model}:generateContent
    responses.rs      # POST /v1/responses
  format/
    mod.rs
    openai.rs         # Chat Completions request/response structs
    anthropic.rs      # Messages API request/response structs
    gemini.rs         # generateContent request/response structs
    responses.rs      # Responses API request/response structs
```

---

## Dependencies

Minimal set, pinned versions:

- `tokio` — async runtime
- `axum` — HTTP framework (built on `hyper`, HTTP/1.1 + HTTP/2 native)
- `serde` + `serde_json` — serialization
- `serde_yaml` — fixture file parsing
- `clap` — CLI argument parsing
- `regex` — regex matching in fixtures
- `uuid` — response ID generation

Dev deps: `reqwest` (for integration tests), `tokio-test`.

---

## Routing

Real API paths, no provider prefix. Paths are already unique:

| Route | Provider |
|-------|----------|
| `POST /v1/chat/completions` | OpenAI |
| `POST /v1/messages` | Anthropic |
| `POST /v1/responses` | OpenAI Responses API |
| `POST /v1beta/models/{model}:generateContent` | Gemini |

Clients swap their base URL to `http://127.0.0.1:{port}` — no path changes needed.

**Authentication headers are ignored.** llmposter does not validate `Authorization`, `x-api-key`, `anthropic-version`, or `x-goog-api-key` headers. It's a test mock — accept everything.

---

## Fixture Format (YAML)

```yaml
fixtures:
  # Simple text response — works for any provider endpoint
  - match:
      user_message: "stock price of AAPL"    # substring match (default)
    response:
      content: "The current stock price of AAPL is $150.42"

  # Regex match with streaming config
  - match:
      user_message:
        regex: "stock price of \\w+"
      model: "claude-sonnet-4-6"
    response:
      content: "I can help with stock prices."
    streaming:
      latency: 50       # ms between SSE chunks
      chunk_size: 20     # chars per chunk (default: 20)

  # Tool call response
  - match:
      user_message: "what's the weather"
    response:
      tool_calls:
        - name: get_weather
          arguments:
            location: "San Francisco"
            unit: "celsius"

  # Error simulation
  - match:
      model: "fail-model"
    error:
      status: 429
      message: "Rate limit exceeded"

  # Failure simulation
  - match:
      user_message: "long response"
    response:
      content: "This will get cut off mid-stream..."
    failure:
      truncate_after_chunks: 3
      # disconnect_after_ms: 500
      # corrupt_body: true
      # latency_ms: 5000

  # Provider-specific override (optional)
  - match:
      user_message: "specific format"
    provider: anthropic
    response:
      content: "Provider-specific response"
      stop_reason: end_turn
```

### Match Rules

- **First-match-wins** — fixtures evaluated in order, first match is used
- **`user_message`** — string value = substring match; `{ regex: "pattern" }` for regex
- **`model`** — string value = substring match; `{ regex: "pattern" }` for regex
- **All match fields are optional** — omitted fields match anything
- **A fixture with no match fields matches everything** (catch-all, put last)
- **`provider`** — when set, fixture only matches requests hitting that provider's endpoint

### Response Rules

- **`content`** — text response, provider-agnostic. llmposter wraps it in the correct format based on which endpoint was hit.
- **`tool_calls`** — array of tool invocations. `arguments` is always a YAML map in fixtures; llmposter serializes it per provider (JSON string for OpenAI, object for Anthropic/Gemini).
- **`error`** — returns HTTP error status with provider-shaped error body. Mutually exclusive with `response`. If both present, validation error.
- **`failure`** — simulates network/streaming problems on an otherwise valid response. Requires `response` to also be present. `error` + `failure` is invalid (validation error).

### Streaming Config

Top-level `streaming` block controls SSE chunk behavior (separate from `failure`):

| Field | Default | Description |
|-------|---------|-------------|
| `streaming.latency` | `0` | Milliseconds between SSE chunks |
| `streaming.chunk_size` | `20` | Characters per SSE chunk |

These control normal streaming behavior. `failure` fields control abnormal behavior (truncation, disconnect).

---

## Response ID Generation

All provider formats include IDs in responses. llmposter generates deterministic IDs using a counter per server instance:

| Provider | Format | Example |
|----------|--------|---------|
| OpenAI | `chatcmpl-llmposter-{n}` | `chatcmpl-llmposter-1` |
| Anthropic | `msg-llmposter-{n}` | `msg-llmposter-1` |
| Gemini | (none — Gemini doesn't return IDs) | — |
| Responses | `resp-llmposter-{n}` | `resp-llmposter-1` |

Deterministic IDs enable snapshot testing. Counter resets per server instance.

---

## Usage Fields

All provider responses include `usage` objects. llmposter calculates approximate values:

- `prompt_tokens` — estimated from request body length (`content.len() / 4`)
- `completion_tokens` — estimated from response content length (`content.len() / 4`)
- `total_tokens` — sum of the above

These are rough approximations, not real tokenizer output. Sufficient for testing that usage fields exist and are non-zero.

---

## Library API

Two paths to load fixtures — programmatic and YAML. Independent of each other.

### Programmatic (for Rust tests)

```rust
use llmposter::{ServerBuilder, Fixture};

#[tokio::test]
async fn test_stock_price() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("stock price")
                .respond_with_content("AAPL is $150.42")
        )
        .build()
        .await;

    let client = MyLlmClient::new(&server.url());
    let resp = client.complete("stock price of AAPL").await.unwrap();
    assert!(resp.contains("$150"));
    // server drops when test ends
}
```

### YAML file loading

```rust
let server = ServerBuilder::new()
    .load_yaml("fixtures/stocks.yaml")?       // validates, returns error if broken
    .load_yaml_dir("fixtures/")?              // loads all .yaml/.yml files
    .fixture(Fixture::new()...)               // programmatic fixtures also work
    .build()
    .await;
```

- YAML validation errors are clear and actionable (file, line, what's wrong)
- A broken YAML file does NOT prevent programmatic fixtures from working
- Builder methods are chainable, fixtures from all sources merge in order

### Server lifecycle

- `server.url()` — returns `http://127.0.0.1:{port}` with the assigned port
- `server.port()` — returns the assigned port
- Server binds to `127.0.0.1:0` by default (OS-assigned port, IPv4) for test isolation
- `.bind("[::1]:0")` for IPv6 loopback, `.bind("[::]:0")` for dual-stack
- Server shuts down on drop (graceful via `tokio` task abort)

---

## CLI Binary

```bash
# Start server with fixtures
llmposter --fixtures ./fixtures/

# Validate fixture files without starting server
llmposter --fixtures ./fixtures/ --validate

# Specify port (default: random)
llmposter --fixtures ./fixtures/ --port 8080

# Specify bind address (IPv4)
llmposter --fixtures ./fixtures/ --bind 0.0.0.0 --port 8080

# IPv6 loopback
llmposter --fixtures ./fixtures/ --bind ::1 --port 8080

# Dual-stack (IPv4 + IPv6)
llmposter --fixtures ./fixtures/ --bind :: --port 8080

# Verbose mode — log matched/unmatched requests to stderr
llmposter --fixtures ./fixtures/ --verbose
```

---

## Request Flow

1. Request hits provider endpoint (e.g., `POST /v1/messages`)
2. Handler extracts model + last user message from provider-specific request body. **Lenient parsing** — only extract what's needed, ignore unknown fields, don't validate request schema. Return 400 only if JSON is unparseable or required extraction fields (`messages`/`contents`/`input`) are missing.
3. `fixture::match_fixture()` iterates fixtures, returns first match
4. **No match → 404** with JSON body: `{ "error": { "message": "No fixture matched: model='X', user_message='Y'", "type": "no_fixture_match" } }`. Includes the extracted model and message to aid debugging.
5. If fixture has `error` → return HTTP error status with provider-shaped error body
6. If fixture has `failure` → apply simulation:
   - `latency_ms` → sleep before responding
   - `corrupt_body` → return HTTP 200 with plain text body `"overloaded"` (no JSON, no content-type: application/json). This triggers skilldo's "overloaded" string detection in `RetryClient`.
   - `truncate_after_chunks` → send N SSE chunks then close connection without `[DONE]` marker (streaming only)
   - `disconnect_after_ms` → drop TCP connection after N ms (streaming only)
7. If request has `stream: true` → `stream::sse_write()` chunks response in provider SSE format with `Content-Type: text/event-stream`
8. Otherwise → return full JSON response in provider format with `Content-Type: application/json`

---

## Provider Response Formats

Each provider module in `format/` defines request and response structs matching the real API shape. llmposter's responses must be deserializable by skilldo's client structs.

### OpenAI Chat Completions

Non-streaming: `{ "id": "chatcmpl-llmposter-1", "object": "chat.completion", "choices": [{ "message": { "role": "assistant", "content": "..." }, "finish_reason": "stop", "index": 0 }], "model": "...", "usage": { "prompt_tokens": N, "completion_tokens": N, "total_tokens": N } }`

Streaming SSE (`Content-Type: text/event-stream`): `data: { "choices": [{ "delta": { "content": "chunk" } }] }\n\n` per chunk, then `data: [DONE]\n\n`

### Anthropic Messages

Non-streaming: `{ "id": "msg-llmposter-1", "type": "message", "role": "assistant", "content": [{ "type": "text", "text": "..." }], "stop_reason": "end_turn", "model": "...", "usage": { "input_tokens": N, "output_tokens": N } }`

Streaming SSE (`Content-Type: text/event-stream`): `event: message_start\ndata: {...}\n\n`, `event: content_block_start\ndata: {...}\n\n`, `event: content_block_delta\ndata: { "delta": { "type": "text_delta", "text": "chunk" } }\n\n`, ..., `event: message_stop\ndata: {}\n\n`

### Gemini generateContent

Non-streaming: `{ "candidates": [{ "content": { "parts": [{ "text": "..." }], "role": "model" }, "finishReason": "STOP" }], "usageMetadata": { "promptTokenCount": N, "candidatesTokenCount": N, "totalTokenCount": N } }`

Streaming: Gemini's real `streamGenerateContent` endpoint returns a chunked JSON array, not SSE. llmposter matches this: `Content-Type: application/json`, response body is `[{candidate1_chunk}, {candidate2_chunk}, ...]` where each element has the same shape as the non-streaming response. The `alt=sse` query parameter variant uses SSE format instead (`data: {json}\n\n`).

### OpenAI Responses API

Non-streaming: `{ "id": "resp-llmposter-1", "object": "response", "output": [{ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "..." }] }], "model": "...", "usage": { "input_tokens": N, "output_tokens": N, "total_tokens": N } }`

Streaming SSE (`Content-Type: text/event-stream`): `event: response.created\ndata: {...}\n\n`, `event: response.output_item.added\ndata: {...}\n\n`, `event: response.output_text.delta\ndata: { "delta": "chunk" }\n\n`, ..., `event: response.completed\ndata: {...}\n\n`

---

## Failure Simulation

| Fixture Field | Effect | HTTP Status | Applies To |
|---------------|--------|-------------|------------|
| `error.status` | Return HTTP error with provider-shaped error body | As specified (429, 500, 503, etc.) | Both |
| `error.message` | Custom error message in the error body | (with error.status) | Both |
| `failure.latency_ms` | Sleep N ms before responding | 200 (delayed) | Both |
| `failure.corrupt_body` | Return plain text `"overloaded"` instead of JSON | 200 | Non-streaming |
| `failure.truncate_after_chunks` | Send N SSE chunks then close without `[DONE]` | 200 (incomplete) | Streaming |
| `failure.disconnect_after_ms` | Drop TCP connection after N ms | (connection reset) | Streaming |

These map directly to transient errors that skilldo's `RetryClient` recognizes: HTTP 429/500/502/503/529, "overloaded" body, connection reset, broken pipe, timeout.

### Mutual Exclusivity

- `error` and `response` are mutually exclusive. A fixture has one or the other.
- `error` and `failure` are mutually exclusive. `failure` requires `response`.
- `streaming` config is ignored when `error` is present.
- Violations produce a clear validation error at load time.

---

## Request Validation

**Lenient by design.** llmposter is a test mock, not a validator. It:

- Accepts any JSON body that has the minimum fields needed for fixture matching
- Ignores unknown fields
- Does not validate auth headers (`Authorization`, `x-api-key`, `anthropic-version`, `x-goog-api-key`)
- Returns 400 only for: unparseable JSON, completely missing request body, missing required structural fields (e.g., no `messages` array in a chat completions request)

---

## Testing Strategy

- **97%+ coverage target** from day one
- **TDD workflow** — failing tests before implementation
- **Unit tests per module** — fixture matching, response serialization, SSE chunking, failure simulation
- **Integration tests** — spin up in-process server, hit endpoints with `reqwest`, validate responses parse correctly
- **Cross-validation with skilldo** — skilldo's client structs (`AnthropicResponse`, `OpenAIResponse`, `GeminiResponse`, `ResponsesResponse`) should successfully deserialize llmposter's output
- **Fixture validation tests** — good YAML parses, bad YAML gives clear errors, broken YAML doesn't prevent programmatic fixtures

---

## What We're NOT Building (for now)

- WebSocket support (Realtime API, Gemini Live) — HTTP SSE covers our needs
- Journal/request logging endpoint — `--verbose` stderr logging covers debugging for now
- Predicate-based matching (arbitrary functions in fixtures) — YAML doesn't support this; programmatic builder can via closures later
- Multiple response sequences (return different responses on successive calls to same fixture) — design the fixture match logic so this can be added later without breaking changes (e.g., `response` could accept a list)
