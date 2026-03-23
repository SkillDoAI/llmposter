# Gemini generateContent

**Endpoint:** `POST /v1beta/models/{model}:generateContent`
**Streaming:** `POST /v1beta/models/{model}:streamGenerateContent`

**Spec:** https://ai.google.dev/api/generate-content

## Response Fields

### Non-Streaming

| Field | Type | Value |
|-------|------|-------|
| `candidates[].content.parts[].text` | string | Fixture content |
| `candidates[].content.role` | string | `"model"` |
| `candidates[].finishReason` | string | `"STOP"` |
| `usageMetadata.promptTokenCount` | integer | Estimated |
| `usageMetadata.candidatesTokenCount` | integer | Estimated |
| `usageMetadata.totalTokenCount` | integer | `prompt + candidates` |

Note: All field names use **camelCase** (not snake_case). Gemini does not emit an `id` field.

### Function Call Response

```json
{
  "candidates": [{
    "content": {
      "parts": [{
        "functionCall": {
          "name": "get_weather",
          "args": {"location": "SF"}
        }
      }],
      "role": "model"
    }
  }]
}
```

Note: Gemini sends function call `args` as a JSON **object** (not a string).

## Streaming

Gemini supports two streaming modes:

### JSON Array (default)
`POST /v1beta/models/{model}:streamGenerateContent`

Returns a JSON array of `GenerateContentResponse` objects. Each element is a complete response with partial content. Only the last element has `finishReason`.

### SSE (Server-Sent Events)
`POST /v1beta/models/{model}:streamGenerateContent?alt=sse`

Returns SSE `data:` frames, each containing a `GenerateContentResponse` object. Same structure as JSON array but delivered as an event stream.

## Error Response Format

```json
{
  "error": {
    "code": 429,
    "message": "Rate limit exceeded",
    "status": "RESOURCE_EXHAUSTED"
  }
}
```

| Status | Status Name |
|--------|-------------|
| 400 | `INVALID_ARGUMENT` |
| 401 | `UNAUTHENTICATED` |
| 403 | `PERMISSION_DENIED` |
| 404 | `NOT_FOUND` |
| 429 | `RESOURCE_EXHAUSTED` |
| 500 | `INTERNAL` |
| 503 | `UNAVAILABLE` |

## Configurable Fields

The `finishReason` value can be overridden per-fixture using `stop_reason` or `finish_reason` in the fixture YAML. The default (`"STOP"`) applies when no override is set.

## Known Deviations

See [Spec Deviations](../spec-deviations.md#all-providers) for documented gaps.

*Full compliance audit in progress.*
