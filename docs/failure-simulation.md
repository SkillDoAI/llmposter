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
  latency_jitter_ms: 10       # per-frame jitter in [-10ms, +10ms], clamped at 0
  duplicate_frames: true      # duplicate each frame when chaos activates
  probability: 0.3            # 30% per-request dice roll for the chaos block
  chaos_seed: 42              # override the PRNG seed (default: per-request counter)
```

### How the fields interact

- **`latency_jitter_ms`** adds a symmetric random offset in
  `[-range, +range]` to each frame's delay, then clamps the result at 0
  (no negative delays). Requires a non-zero `streaming.latency` to act
  on. Useful for catching consumers that assume uniform inter-frame
  timing.
- **`duplicate_frames`** duplicates **every** streamed frame (not
  "occasional" duplication) for requests where chaos is active.
  Use this to verify idempotent event handlers and assert that
  downstream code tolerates replayed messages. **Note:** duplication
  runs before truncation, so combining `duplicate_frames: true` with
  `truncate_after_frames: N` cuts the stream after `N` *doubled*
  frames (i.e. `N/2` source frames if `N` is even). Use
  `truncate_after_frames: 2 * N` if you want to cut after `N`
  original frames.
- **`probability`** (default `1.0`) is a per-request dice roll in
  `[0.0, 1.0]` gating the chaos block as a whole (both
  `latency_jitter_ms` and `duplicate_frames`). Classical failures
  (`latency_ms`, `corrupt_body`, `truncate_after_frames`,
  `disconnect_after_ms`) ignore `probability` and always apply when
  set.
- **`chaos_seed`** overrides the per-request seed used to roll the
  activation dice and compute jitter values. Without it, the seed is
  derived from a monotonically increasing server-internal counter, so
  successive requests in the same test produce a deterministic but
  distinct sequence of chaos outcomes. Set an explicit `chaos_seed` when
  you need two server instances (or two test runs) to produce
  *identical* chaos patterns.

### Deterministic reproducibility

When `chaos_seed` is **explicit**, the chaos plan (jitter values,
duplication decision, probability roll) is fully determined by the
seed alone — the per-request counter is ignored, so chaos is
reproducible independently of request ordering. Two servers with
the same fixture and same `chaos_seed` produce bit-identical chaos
for every matching request, in any order.

When `chaos_seed` is **unset**, the seed is derived from an internal
per-server request counter. In that mode reproducibility still
holds, but only for a fixed request order — a test that sends the
same requests in the same sequence will see the same chaos outcomes
across runs; a test that reorders requests (or mixes in extra
requests that also use chaos) will see a different sequence.

Either way, flaky streaming tests caused by chaos are impossible:
if you see a failure once you can re-run the exact same test and
reproduce it.

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
