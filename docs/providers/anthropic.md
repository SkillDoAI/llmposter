# Anthropic Messages API

**Endpoint:** `POST /v1/messages`

**Spec:** https://docs.anthropic.com/en/api/messages

## Response Fields

### Non-Streaming

| Field | Type | Value |
|-------|------|-------|
| `id` | string | `msg-llmposter-{N}` |
| `type` | string | `"message"` |
| `role` | string | `"assistant"` |
| `model` | string | Echoed from request |
| `content` | array | Content blocks (text or tool_use) |
| `stop_reason` | string | `"end_turn"` or `"tool_use"` |
| `stop_sequence` | null | Always null |
| `usage.input_tokens` | integer | Estimated |
| `usage.output_tokens` | integer | Estimated |
| `usage.cache_creation_input_tokens` | integer | `0` (caching not simulated) |
| `usage.cache_read_input_tokens` | integer | `0` (caching not simulated) |

### Content Blocks

**Text block:**
```json
{"type": "text", "text": "Hello!"}
```

**Tool use block:**
```json
{"type": "tool_use", "id": "toolu_llmposter_1", "name": "get_weather", "input": {"location": "SF"}}
```

Note: Anthropic sends tool `input` as a JSON **object**, not a string (unlike OpenAI).

## Streaming Event Sequence

### Text response
1. `ping` → `{"type": "ping"}`
2. `message_start` → `{"type": "message_start", "message": {"id": "...", "type": "message", "role": "assistant", "model": "...", "content": [], "stop_reason": null, "stop_sequence": null, "usage": {...}}}`
3. `content_block_start` → `{"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}`
4. `content_block_delta` (repeated) → `{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "..."}}`
5. `content_block_stop` → `{"type": "content_block_stop", "index": 0}`
6. `message_delta` → `{"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}, "usage": {"output_tokens": N}}`
7. `message_stop` → `{"type": "message_stop"}`

### Tool use response
1. `ping`
2. `message_start` (same structure as text)
3. `content_block_start` → `{"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "...", "name": "...", "input": {}}}`
4. `content_block_delta` → `{"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "..."}}`
5. `content_block_stop` → `{"type": "content_block_stop", "index": 0}`
6. `message_delta` → `{"type": "message_delta", "delta": {"stop_reason": "tool_use", "stop_sequence": null}, "usage": {"output_tokens": N}}`
7. `message_stop` → `{"type": "message_stop"}`

## Error Response Format

```json
{
  "type": "error",
  "error": {
    "type": "rate_limit_error",
    "message": "Rate limit exceeded"
  }
}
```

| Status | Error Type |
|--------|-----------|
| 400 | `invalid_request_error` |
| 401 | `authentication_error` |
| 403 | `permission_error` |
| 404 | `not_found_error` |
| 429 | `rate_limit_error` |
| 500/502/503 | `api_error` |
| 529 | `overloaded_error` |

## Configurable Fields

The `stop_reason` value can be overridden per-fixture using `stop_reason` or `finish_reason` in the fixture YAML. The defaults shown above (`"end_turn"`, `"tool_use"`) apply when no override is set.

## Known Deviations

See [Spec Deviations](../spec-deviations.md#all-providers) for documented gaps.

*Full compliance audit in progress.*
