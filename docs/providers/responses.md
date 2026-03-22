# OpenAI Responses API

**Endpoint:** `POST /v1/responses`

**Spec:** https://platform.openai.com/docs/api-reference/responses/object

## Response Fields

### Non-Streaming

| Field | Type | Value |
|-------|------|-------|
| `id` | string | `resp-llmposter-{N}` |
| `object` | string | `"response"` |
| `status` | string | `"completed"` |
| `model` | string | Echoed from request |
| `output` | array | Output items (messages or function calls) |
| `usage.input_tokens` | integer | Estimated |
| `usage.output_tokens` | integer | Estimated |
| `usage.total_tokens` | integer | `input + output` |

### Text Output Item

```json
{
  "type": "message",
  "id": "msg_1",
  "status": "completed",
  "role": "assistant",
  "content": [{"type": "output_text", "text": "Hello!"}]
}
```

### Function Call Output Item

```json
{
  "type": "function_call",
  "id": "fc_1",
  "call_id": "call_llmposter_1",
  "status": "completed",
  "name": "get_weather",
  "arguments": "{\"location\":\"SF\"}"
}
```

Note: Responses API sends `arguments` as a JSON **string** (like OpenAI Chat Completions).

## Streaming Event Sequence

All events include a monotonically increasing `sequence_number` starting at 0.

### Text response
1. `response.created` → `{"type": "response.created", "response": {..., "status": "in_progress"}, "sequence_number": 0}`
2. `response.in_progress` → `{"type": "response.in_progress", "response": {...}, "sequence_number": 1}`
3. `response.output_item.added` → `{"type": "...", "item": {...}, "output_index": 0, "sequence_number": 2}`
4. `response.content_part.added` → `{"type": "...", "item_id": "...", "content_index": 0, "sequence_number": 3}`
5. `response.output_text.delta` (repeated) → `{"type": "...", "item_id": "...", "delta": "...", "sequence_number": N}`
6. `response.output_text.done` → `{"type": "...", "item_id": "...", "text": "full text", "sequence_number": N}`
7. `response.content_part.done`
8. `response.output_item.done`
9. `response.completed` → `{"type": "response.completed", "response": {..., "status": "completed"}, "sequence_number": N}`

### Tool call response
Same lifecycle envelope events, with:
- `response.function_call_arguments.delta` → `{"item_id": "...", "call_id": "...", "delta": "...", "sequence_number": N}`
- `response.function_call_arguments.done` → `{"item_id": "...", "call_id": "...", "arguments": "...", "sequence_number": N}`

### Key protocol details
- `response.created` and `response.completed` wrap the response object in a nested `"response"` key
- `response.in_progress` is emitted between `created` and the first output item
- Text delta events include `item_id`, `output_index`, and `content_index` for correlation
- Function call events include `item_id`, `call_id`, and `output_index` (no `content_index`)
- `status` can be overridden to `"incomplete"` via fixture `stop_reason` for non-default stop reasons

## Error Response Format

Same as [OpenAI Chat Completions](openai.md#error-response-format).

## Known Deviations

See [Spec Deviations](../spec-deviations.md#openai-responses-api) for documented gaps.
