# Known Spec Deviations

llmposter aims for 100% API spec compliance. This page documents every known gap.

## OpenAI Chat Completions

### Role-only streaming chunk omits `content: null`

**Real API:** First streaming chunk sends `"content": null` explicitly alongside `"role": "assistant"`.

**llmposter:** Omits `content` entirely on the role-only chunk (via `skip_serializing_if`).

**Impact:** None. Every OpenAI SDK treats absent and `null` identically for `Option<String>` fields.

**Reason:** We can't selectively emit `null` on one chunk type while correctly omitting `content` on all other chunk types without a custom serializer. Zero practical benefit.

### `system_fingerprint` is static

**Real API:** Returns a fingerprint like `fp_50cad350e4` that varies by backend configuration.

**llmposter:** Always returns `fp_llmposter`.

**Impact:** None for most tests. If you need to validate fingerprint-dependent logic, use the real API.

### `logprobs` is always null

**Real API:** Returns log probability data when `logprobs: true` is set.

**llmposter:** Always returns `logprobs: null` regardless of request parameters.

### `refusal` defaults to null; refusal simulation is fixture-opt-in

**Real API:** Returns a refusal message when content is filtered.

**llmposter (v0.4.5+):** Regular `response:` fixtures emit
`refusal: null`. Fixtures with a `refusal:` block emit
`choices[0].message.refusal: "<reason>"` with `content: null` and
`finish_reason: "stop"` — see `docs/fixtures.md` for the top-level
`refusal:` block syntax. Streaming refusals return HTTP 400; a
non-streaming request against the same fixture returns the refusal
shape.

## OpenAI Responses API

### Streaming event subset

**Real API:** Supports many more streaming event types, including reasoning, code interpreter, web search, MCP, file search, image generation, and audio events.

**llmposter:** Supports the core text and function-call streaming events:
- `response.created`, `response.in_progress`, `response.completed`
- `response.output_item.added`, `response.output_item.done`
- `response.content_part.added`, `response.content_part.done`
- `response.output_text.delta`, `response.output_text.done`
- `response.function_call_arguments.delta`, `response.function_call_arguments.done`

Advanced tool events are not simulated.

## All Providers

### Token counts are estimated

**Real APIs:** Return actual tokenizer-computed token counts.

**llmposter:** Uses a `bytes / 4` heuristic. Token counts are approximately correct but not exact. Assert they are positive and that `total == prompt + completion`, not specific values.

### `chunk_size` does not apply to tool-call streams

**Real APIs:** Stream tool-call arguments as incremental deltas
(OpenAI `delta.tool_calls[].function.arguments`, Anthropic
`input_json_delta`, etc.) where a long arguments JSON may be split
across multiple delta frames.

**llmposter:** Emits the full tool-call arguments in a single frame
regardless of the fixture's `streaming.chunk_size`. Chunking JSON
arguments character-by-character would produce syntactically invalid
intermediate states that real clients don't need to handle (the
delta framing exists for latency, not correctness, and real clients
concatenate all deltas before parsing). `chunk_size` still applies
to text content streaming.

**Source:** Codex audit on PR #29; documented in v0.4.6.

### Rate limit header values are defaults

**Real APIs:** Return actual quotas and reset times.

**llmposter:** Emits sensible default values on 429 responses. OpenAI uses duration format (`1m0s`), Anthropic uses RFC 3339 timestamps. Per-fixture overrides are supported via `error.headers` in YAML or `with_error_headers()` in the builder API (v0.4.1+).

### Request fields silently ignored

llmposter accepts most request fields (`temperature`, `top_p`, `tools`, `metadata`, etc.) and silently ignores them. Only `model`, `messages`/`input`/`contents`, and `stream` are used for fixture matching.

**Exception:** Anthropic's `max_tokens` field is validated — it must be present and a positive integer, matching the real API's requirement (v0.4.2+). Requests missing `max_tokens` on `/v1/messages` receive a 400 error.

All other fields are passed through without validation — your real client code can send any parameters without modification.
