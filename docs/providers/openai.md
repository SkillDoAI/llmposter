# OpenAI Chat Completions

**Endpoint:** `POST /v1/chat/completions`

**Spec:** https://platform.openai.com/docs/api-reference/chat/object

## Response Fields

### Non-Streaming

| Field | Type | Value |
|-------|------|-------|
| `id` | string | `chatcmpl-llmposter-{N}` |
| `object` | string | `"chat.completion"` |
| `created` | integer | Unix timestamp |
| `model` | string | Echoed from request |
| `system_fingerprint` | string | `"fp_llmposter"` |
| `service_tier` | string | `"default"` |
| `choices[].index` | integer | `0` |
| `choices[].message.role` | string | `"assistant"` |
| `choices[].message.content` | string\|null | Fixture content |
| `choices[].message.tool_calls` | array\|null | Fixture tool calls |
| `choices[].message.refusal` | null | Always null |
| `choices[].finish_reason` | string | `"stop"` or `"tool_calls"` |
| `choices[].logprobs` | null | Always null |
| `usage.prompt_tokens` | integer | Estimated |
| `usage.completion_tokens` | integer | Estimated |
| `usage.total_tokens` | integer | `prompt + completion` |

### Streaming Chunks

| Field | Type | Notes |
|-------|------|-------|
| `id` | string | Same across all chunks |
| `object` | string | `"chat.completion.chunk"` |
| `created` | integer | Same across all chunks |
| `model` | string | Echoed from request |
| `system_fingerprint` | string | Present on all chunks |
| `service_tier` | string\|absent | Present on first chunk only |
| `choices[].delta.role` | string\|absent | `"assistant"` on first chunk only |
| `choices[].delta.content` | string\|absent | Present on content chunks |
| `choices[].delta.tool_calls` | array\|absent | Present on tool-call chunks |
| `choices[].finish_reason` | string\|null | `null` on non-final, `"stop"` or `"tool_calls"` on final |
| `choices[].logprobs` | null | Always null |

Final frame is always `data: [DONE]`.

## Streaming Event Sequence

### Text response
1. Role chunk: `{"delta": {"role": "assistant"}, "finish_reason": null}`
2. Content chunks: `{"delta": {"content": "..."}, "finish_reason": null}`
3. Stop chunk: `{"delta": {}, "finish_reason": "stop"}`
4. `data: [DONE]`

### Tool call response
1. Role chunk: `{"delta": {"role": "assistant"}, "finish_reason": null}`
2. Tool calls chunk: `{"delta": {"tool_calls": [...]}, "finish_reason": null}`
3. Stop chunk: `{"delta": {}, "finish_reason": "tool_calls"}`
4. `data: [DONE]`

## Error Response Format

```json
{
  "error": {
    "message": "Rate limit exceeded",
    "type": "rate_limit_error",
    "param": null,
    "code": "rate_limit_exceeded"
  }
}
```

See [Failure Simulation](../failure-simulation.md) for the full error type/code mapping.

## Configurable Fields

The `finish_reason` value can be overridden per-fixture using `stop_reason` or `finish_reason` in the fixture YAML. The defaults shown above (`"stop"`, `"tool_calls"`) apply when no override is set.

## Known Deviations

See [Spec Deviations](../spec-deviations.md#openai-chat-completions) for documented gaps.
