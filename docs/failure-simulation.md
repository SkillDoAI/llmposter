# Failure Simulation

llmposter can simulate various failure modes to test your application's resilience.

## Error Responses

Return HTTP error codes with provider-specific error shapes:

```yaml
- match:
    model: "fail-model"
  error:
    status: 429
    message: "Rate limit exceeded"
```

| Status | Error Type | Error Code |
|--------|-----------|------------|
| 400 | `invalid_request_error` | `invalid_request` |
| 401 | `authentication_error` | `invalid_api_key` |
| 403 | `permission_denied_error` | `permission_denied` |
| 404 | `not_found_error` | `not_found` |
| 429 | `rate_limit_error` | `rate_limit_exceeded` |
| 500 | `server_error` | `server_error` |
| 502 | `server_error` | `bad_gateway` |
| 503 | `server_error` | `service_unavailable` |
| 529 | `server_error` | `overloaded` |

Error response format (OpenAI/Responses):
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

Anthropic and Gemini have their own error shapes — see [provider guides](providers/).

## Latency Injection

Delay the response by a fixed duration:

```yaml
failure:
  latency_ms: 5000    # 5 second delay before responding
```

## Body Corruption

Return `"overloaded"` as plain text instead of a valid JSON response:

```yaml
failure:
  corrupt_body: true
```

## Stream Truncation

Cut off a streaming response after N SSE frames:

```yaml
failure:
  truncate_after_frames: 3    # send 3 frames then stop
  # Also accepted: truncate_after_chunks (legacy alias)
```

## Connection Disconnect

Drop the connection after N milliseconds of streaming:

```yaml
failure:
  disconnect_after_ms: 500    # disconnect 500ms into the stream
```

## Combining Failures

Latency can be combined with other failure modes:

```yaml
failure:
  latency_ms: 2000            # wait 2 seconds
  truncate_after_frames: 5    # then truncate after 5 frames
```

Note: `corrupt_body` returns immediately with plain text, while streaming failures (`truncate_after_frames`, `disconnect_after_ms`) require a valid response to stream. These are not validated as mutually exclusive at load time — if combined, `corrupt_body` takes priority and the streaming failure fields are ignored.

## Notes

- `error` and `failure` are mutually exclusive — use `error` for HTTP error codes, `failure` for network/streaming simulation on valid responses.
- `failure` requires a `response` block (it needs content to stream/corrupt).
- `truncate_after_frames` and `disconnect_after_ms` only apply to streaming requests. They are silently ignored on non-streaming requests.
