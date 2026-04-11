# Fixture Format Reference

Fixtures are YAML files that define canned responses. llmposter matches incoming requests against fixtures using first-match-wins ordering.

## Basic Structure

```yaml
fixtures:
  - match:
      user_message: "hello"     # substring match (default)
    response:
      content: "Hi there!"
```

## Matching Rules

### Substring match (default)

```yaml
match:
  user_message: "stock price"   # matches any message containing "stock price"
```

### Regex match

```yaml
match:
  user_message:
    regex: "stock price of \\w+"
```

### Model match (substring)

```yaml
match:
  model: "gpt-4"               # substring match — also matches "gpt-4-turbo"
```

### Model match (regex)

```yaml
match:
  model:
    regex: "^gpt-4$"           # exact match via regex
```

### Combined match

```yaml
match:
  user_message: "hello"
  model: "claude-sonnet-4-6"       # both must match
```

### Catch-all (no match criteria)

```yaml
- response:
    content: "Default response"   # matches everything not caught above
```

## Scenarios (Multi-Turn State)

Fixtures can participate in named state machines for multi-turn matching. See [Scenarios](scenarios.md) for full documentation.

```yaml
fixtures:
  - match:
      user_message: "weather"
    scenario:
      name: "weather-flow"
      required_state: ""         # initial state only
      set_state: "tool_called"
    response:
      tool_calls:
        - name: get_weather
          arguments: { location: "Paris" }

  - match:
      user_message: "weather"
    scenario:
      name: "weather-flow"
      required_state: "tool_called"
      set_state: "done"
    response:
      content: "22°C and sunny"
```

## Response Types

### Text response

```yaml
response:
  content: "The answer is 42"
```

### Tool call response

```yaml
response:
  tool_calls:
    - name: get_weather
      arguments:
        location: "San Francisco"
        unit: "celsius"
```

Tool call arguments must be JSON objects (not scalars or arrays). This is validated at fixture load time.

### Custom stop/finish reason

```yaml
response:
  content: "Partial response"
  stop_reason: "max_tokens"      # provider-native field name
  # finish_reason: "max_tokens"  # also supported — separate field, same effect
```

Both `stop_reason` and `finish_reason` are supported as separate fixture fields. When both are set, `stop_reason` takes precedence. See [provider guides](providers/) for default values per provider.

### Templated response (v0.4.4+)

Instead of `content`, a fixture may set `content_template` — a Jinja-style
template that's rendered at response time with request-derived values.
Requires the `templating` Cargo feature (off by default).

```yaml
response:
  content_template: "You said: {{ user_message }} (model={{ model }})"
```

Template context:

| Variable | Value |
|---|---|
| `user_message` | The extracted user message (same value fixture matching sees) |
| `model` | Model name from the request body |
| `provider` | `"openai"`, `"anthropic"`, `"gemini"`, or `"responses"` |
| `request` | Full parsed request JSON (e.g. `{{ request.messages[-1].content }}`) |

Validation rules:

- `content_template` is mutually exclusive with both `content` and `tool_calls`. Setting two of them is a hard error at fixture load time.
- If the `templating` feature is **off**, any fixture with `content_template` set is rejected at load time with an error pointing at the feature flag.
- Template render errors (unknown filters, bad syntax discovered at render time, etc.) surface as HTTP 500 at request time — they do not crash the server.

Enabling the feature:

```toml
[dependencies]
llmposter = { version = "0.4", features = ["templating"] }
```

Or at CLI install time:

```bash
cargo install llmposter --features templating
```

## Streaming Configuration

```yaml
streaming:
  latency: 50        # milliseconds between SSE chunks
  chunk_size: 20     # characters per chunk
```

**`chunk_size` applies to text content streaming only.** Tool-call
streams emit the full arguments in a single frame regardless of
`chunk_size` — OpenAI, Anthropic, Gemini, and Responses API all ship
tool-call arguments as one atomic delta, so there is no meaningful
way to "chunk" them the way character content is chunked. See
[docs/spec-deviations.md](spec-deviations.md#chunk_size-does-not-apply-to-tool-call-streams)
for the full rationale.

## Safety Refusal (v0.4.5+)

Return a provider-specific safety refusal — for when you're testing a
client's refusal-handling branch without hand-rolling upstream payloads.

```yaml
match:
  user_message: "how to hack"
refusal:
  reason: "I cannot help with that request."
```

Each provider gets its native refusal shape:

| Provider          | Shape |
|-------------------|-------|
| OpenAI Chat       | `message.refusal: "<reason>"`, `content: null`, `finish_reason: "stop"` |
| Anthropic         | Text content block + `stop_reason: "refusal"` |
| Gemini            | `candidates: []` + `promptFeedback.blockReason: "SAFETY"` |
| Responses API     | Message output item with a single `type: "refusal"` content part |

`refusal:` is mutually exclusive with `response:`, `error:`, and
`failure:`. Programmatically via `Fixture::respond_with_refusal(reason)`.

**Non-streaming only in v0.4.5.** A matched `refusal:` fixture against
a request that sets `stream: true` (or Gemini's
`streamGenerateContent`) returns HTTP 400 with an explanatory error
body. Streaming refusal envelopes (which real providers do support on
the wire) are not yet implemented — use non-streaming requests or a
regular `response:` fixture if you need a streamed assertion.

## Error Simulation

```yaml
error:
  status: 429                    # HTTP status code (400-599)
  message: "Rate limit exceeded"
```

Error responses use provider-specific shapes. See [provider guides](providers/) for details.

### Custom error headers

Add per-fixture response headers to error responses using the `headers` map:

```yaml
error:
  status: 429
  message: "Rate limit exceeded"
  headers:
    retry-after: "60"
    x-ratelimit-limit-requests: "100"
    x-ratelimit-remaining-requests: "0"
    x-ratelimit-reset-requests: "60s"
```

Keys and values are strings. These headers are added to the error response. If `content-type` is not specified, `application/json` is used as the default.

## Failure Simulation

```yaml
failure:
  latency_ms: 5000              # delay before responding
  corrupt_body: true             # return "overloaded" plain text
  truncate_after_frames: 3       # cut stream after N SSE frames
  disconnect_after_ms: 500       # drop connection mid-stream
```

See [Failure Simulation](failure-simulation.md) for details.

## Provider-Specific Fixtures

By default, fixtures are provider-agnostic — the same fixture serves all endpoints. To restrict a fixture to a specific provider:

```yaml
- match:
    user_message: "specific format"
  provider: anthropic            # only serves /v1/messages
  response:
    content: "Anthropic-specific response"
    stop_reason: end_turn
```

Valid provider values: `openai`, `anthropic`, `gemini`, `responses`.

## Ordering

Fixtures are matched in order — **first match wins**. Put specific matches before catch-alls:

```yaml
fixtures:
  - match:
      user_message: "weather in NYC"
    response:
      content: "72°F and sunny"

  - match:
      user_message:
        regex: "weather in \\w+"
    response:
      content: "I can check the weather for you."

  - response:
      content: "I'm not sure what you mean."   # catch-all last
```

## Loading Fixtures

### Single file
```bash
llmposter --fixtures fixtures.yaml
```

### Directory (loads all .yaml/.yml files)
```bash
llmposter --fixtures fixtures/
```

### Validate without starting
```bash
llmposter --fixtures fixtures/ --validate
```
