# Known Spec Deviations

llmposter aims for 100% API spec compliance, but some deviations are intentional. This page documents every known gap so you can account for them in your tests.

## OpenAI Chat Completions

### Role-only streaming chunk omits `content: null`

**Real API:** First streaming chunk sends `"content": null` explicitly alongside `"role": "assistant"`.

**llmposter:** Omits `content` entirely on the role-only chunk (via `skip_serializing_if`).

**Impact:** None. Every OpenAI SDK (Python, Node, Go) treats absent and `null` identically for `Option<String>` fields. No client breaks on this.

**Reason:** We can't selectively emit `null` on one chunk type while correctly omitting `content` on all other chunk types (tool-call deltas, stop chunks) without a custom serializer. The added complexity has zero practical benefit.

### Token counts are estimated

**Real API:** Returns actual tokenizer-computed token counts.

**llmposter:** Uses a `bytes / 4` heuristic for token estimation.

**Impact:** Token counts are approximately correct but not exact. Don't assert on specific token values — assert they are positive and that `total == prompt + completion`.

### `system_fingerprint` is static

**Real API:** Returns a fingerprint like `fp_50cad350e4` that varies by backend configuration.

**llmposter:** Always returns `fp_llmposter`.

**Impact:** None for testing. If you need to test fingerprint-dependent logic, you'll need the real API.

### `logprobs` is always null

**Real API:** Returns log probability data when `logprobs: true` is set in the request.

**llmposter:** Always returns `logprobs: null` regardless of request parameters.

### `refusal` is always null

**Real API:** Returns a refusal message when content is filtered.

**llmposter:** Always returns `refusal: null`. Refusal simulation is not supported.

## Anthropic Messages

*Full compliance audit in progress — deviations will be documented as found.*

## Gemini generateContent

*Full compliance audit in progress — deviations will be documented as found.*

## OpenAI Responses API

### Streaming event subset

**Real API:** Supports 30+ streaming event types including reasoning, code interpreter, web search, MCP, file search, image generation, and audio events.

**llmposter:** Supports the core event types for text and function-call streaming:
- `response.created`, `response.in_progress`, `response.completed`
- `response.output_item.added`, `response.output_item.done`
- `response.content_part.added`, `response.content_part.done`
- `response.output_text.delta`, `response.output_text.done`
- `response.function_call_arguments.delta`, `response.function_call_arguments.done`

Advanced tool events (reasoning, code execution, web search, MCP) are not simulated.

## Anthropic Messages

*Full compliance achieved in v0.3.5. No known deviations.*

## Gemini generateContent

*Full compliance achieved in v0.3.5. No known deviations.*

## All Providers

### ~~No request-id headers~~ (Fixed in v0.3.5)

Every response now includes `x-request-id: req-llmposter-{N}`.

### ~~No rate limit headers~~ (Fixed in v0.3.5)

429 responses now automatically include `retry-after`, `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, and `x-ratelimit-reset-requests`.

### Rate limit header values are defaults

**Real APIs:** Return actual rate limit quotas and reset times.

**llmposter:** Emits sensible defaults on 429 responses (`retry-after: 60`, `x-ratelimit-remaining-requests: 0`, etc.). Custom values per-fixture are planned for a future release.

### Request fields silently ignored

llmposter accepts all valid request fields (`temperature`, `top_p`, `max_tokens`, `tools`, `metadata`, etc.) and silently ignores them. Only `model`, `messages`/`input`/`contents`, and `stream` are used for fixture matching and response generation.

This is intentional — your real client code can send any parameters without modification.
