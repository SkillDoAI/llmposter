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

**OpenAI / Responses API error types:**

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

Anthropic and Gemini use different error shapes — see their [provider guides](providers/).

**OpenAI / Responses API error format:**
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

## Streaming Chaos (v0.4.4+)

The chaos fields randomize streaming behavior for resilience testing —
jittered per-frame delays, duplicated frames, and probabilistic
activation. All chaos is **seeded**, so runs are reproducible even though
they appear random.

```yaml
failure:
  latency_jitter_ms: 10       # ±10ms jitter on each streaming.latency delay
  duplicate_frames: true      # emit each SSE frame twice back-to-back
  probability: 0.3            # only fire the chaos fields 30% of the time
  chaos_seed: 42              # override the PRNG seed (default: per-request counter)
```

### How the fields interact

- **`latency_jitter_ms`** adds a signed jitter in `[-range, +range]` to each
  frame's delay. Requires a non-zero `streaming.latency` to act on —
  a negative jitter clamps to zero, so there's no "negative delay". Useful
  for catching consumers that assume uniform inter-frame timing.
- **`duplicate_frames`** emits every streamed frame twice. Use this to
  verify idempotent event handlers and assert that downstream code
  tolerates replayed messages.
- **`probability`** (default `1.0`) gates whether the *chaos* fields fire
  on a given request. Classical failures (`latency_ms`, `corrupt_body`,
  `truncate_after_frames`, `disconnect_after_ms`) ignore `probability`
  and always apply when set.
- **`chaos_seed`** overrides the per-request seed used to roll the
  activation dice and compute jitter values. Without it, the seed is
  derived from a monotonically increasing server-internal counter, so
  successive requests in the same test produce a deterministic but
  distinct sequence of chaos outcomes. Set an explicit `chaos_seed` when
  you need two server instances (or two test runs) to produce
  *identical* chaos patterns.

### Deterministic reproducibility

Two servers built from the same fixture will produce the same jitter and
the same duplication decisions as long as `chaos_seed` is explicit and
requests arrive in the same order. This means flaky streaming tests
caused by chaos are impossible: if you see a failure once, you can
re-run the exact same test and reproduce it.

### Example: jittered streaming

```yaml
fixtures:
  - match:
      user_message: "hello"
    response:
      content: "A somewhat longer response that spans several chunks."
    streaming:
      latency: 20           # base 20ms between chunks
      chunk_size: 8
    failure:
      latency_jitter_ms: 10 # ±10ms, so real delays land in [10, 30]
      chaos_seed: 1
```

### Example: mid-stream duplication at 50% probability

```yaml
fixtures:
  - match:
      user_message: "duplicated"
    response:
      content: "This might be duplicated."
    streaming:
      latency: 10
      chunk_size: 6
    failure:
      duplicate_frames: true
      probability: 0.5
      chaos_seed: 7
```

## Notes

- `error` and `failure` are mutually exclusive — use `error` for HTTP error codes, `failure` for network/streaming simulation on valid responses.
- `failure` requires a `response` block (it needs content to stream/corrupt).
- `truncate_after_frames`, `disconnect_after_ms`, `latency_jitter_ms`, and `duplicate_frames` only apply to streaming requests. They are silently ignored on non-streaming requests.
- Chaos fields (`latency_jitter_ms`, `duplicate_frames`, `probability`, `chaos_seed`) ride on top of existing streaming/classical failure fields. They never introduce non-determinism: same seed + same request order = same outcome.
