# Library API Reference

llmposter can be used as an in-process Rust library for `#[tokio::test]` integration tests. No CLI or separate process needed.

## ServerBuilder

```rust
use llmposter::{ServerBuilder, Fixture};

let server = ServerBuilder::new()
    .fixture(Fixture::new()
        .match_user_message("hello")
        .respond_with_content("Hi!"))
    .build()
    .await
    .unwrap();

// server.url() returns "http://127.0.0.1:{port}"
// Server stops when `server` is dropped.
```

### Builder Methods

| Method | Description |
|--------|-------------|
| `.fixture(Fixture)` | Add a single fixture |
| `.fixtures(Vec<Fixture>)` | Add multiple fixtures at once |
| `.fixture_count()` | Number of fixtures currently staged in the builder |
| `.load_yaml(path)` | Load fixtures from a YAML file (returns `Result`). Records the path as a hot-reload source. |
| `.load_yaml_dir(path)` | Load fixtures from a directory (returns `Result`). Records the directory as a hot-reload source. |
| `.watch(bool)` | Enable file-watching hot-reload of tracked sources. Requires the `watch` feature (on by default). See [Hot Reload](#hot-reload). |
| `.bind(addr)` | Set bind address (default: `127.0.0.1:0`) |
| `.verbose(bool)` | Enable verbose logging to stderr |
| `.capture_capacity(max)` | Upper bound on captured-request ring buffer. `0` disables storage (UI live feed stays active). Default: unbounded for library, 1000 for CLI. |
| `.ui(bool)` | Enable the embedded debug UI at `/ui`. Requires the `ui` Cargo feature. |
| `.build().await` | Start the server, returns `Result<MockServer>` |

### MockServer

| Method | Description |
|--------|-------------|
| `.url()` | Base URL (e.g., `http://127.0.0.1:54321`) |
| `.port()` | The port the server is listening on |
| `.get_requests()` | All captured requests in order (see [Request Capture](request-capture.md)) |
| `.request_count()` | Number of requests captured so far |
| `.scenario_state(name)` | Current state of a named scenario, or `None` |
| `.reset()` | Clear all captured requests and reset scenario states |
| `.set_fixtures(Vec<Fixture>)` | Atomically replace the fixture list at runtime. Validates first; invalid fixtures leave the existing list unchanged. See [Hot Reload](#hot-reload). |
| `.fixture_count()` | Number of fixtures currently active (reflects live state after any `set_fixtures` swap or hot-reload) |
| `.check_error()` | Check for post-bind server errors |

The server runs on a random port by default (port 0). Drop the `MockServer` to stop it.

## Hot Reload

llmposter supports swapping fixtures into a running server without
restarting. Three paths:

### Programmatic swap

Useful in tests that need to change fixtures between phases:

```rust
use llmposter::{Fixture, ServerBuilder};

let server = ServerBuilder::new()
    .fixture(Fixture::new().respond_with_content("phase one"))
    .build()
    .await?;

// ... exercise the phase-one response ...

server.set_fixtures(vec![
    Fixture::new().respond_with_content("phase two"),
])?;

// ... subsequent requests see the phase-two response ...
# Ok::<_, Box<dyn std::error::Error>>(())
```

Validation runs before the swap. If any fixture in the new list is invalid
(e.g. missing both `response` and `error`), `set_fixtures` returns an error
and the previously loaded fixtures continue to serve requests unchanged.

### File watcher (`.watch(true)`)

When fixtures were loaded via `load_yaml` / `load_yaml_dir`, you can enable
automatic hot-reload on file-system changes:

```rust
use std::path::Path;

let server = ServerBuilder::new()
    .load_yaml(Path::new("fixtures.yaml"))?
    .watch(true)
    .build()
    .await?;
// Edits to fixtures.yaml are picked up automatically (~250 ms debounce).
# Ok::<_, Box<dyn std::error::Error>>(())
```

Requires the `watch` feature (enabled in the default feature set). Invalid
YAML or failed fixture validation during a reload is logged and the old
fixtures keep serving.

### `SIGHUP` (Unix only, always on)

On Unix, whenever any fixture source path is tracked, llmposter installs a
`SIGHUP` handler that triggers a reload on each signal. This matches
traditional daemon conventions — you can forget `--watch` / `.watch(true)`
and still reload with:

```bash
kill -HUP $(pgrep llmposter)
```

SIGHUP is process-wide: when a test suite uses multiple `MockServer`
instances that load from files, each installs its own handler and all
reload on every signal, each from its own source list. Programmatically-
added fixtures (`.fixture()` / `.fixtures()`) are untouched because there
is no source path to re-read.

## Fixture Builder

```rust
use llmposter::{Fixture, ToolCall};

// Text response
let f = Fixture::new()
    .match_user_message("hello")
    .respond_with_content("Hi there!");

// Tool call response
let f = Fixture::new()
    .match_user_message("weather")
    .respond_with_tool_calls(vec![ToolCall {
        name: "get_weather".to_string(),
        arguments: serde_json::json!({"location": "SF"}),
    }]);

// Error fixture
let f = Fixture::new()
    .match_model("fail-model")
    .with_error(429, "Rate limit exceeded");

// Provider-specific fixture
let f = Fixture::new()
    .match_user_message("weather")
    .respond_with_content("Checking weather...")
    .for_provider(llmposter::Provider::OpenAI);

// Model match
let f = Fixture::new()
    .match_model("gpt-4")
    .respond_with_content("I'm GPT-4!");
```

### Fixture Builder Methods

| Method | Description |
|--------|-------------|
| `.match_user_message(substr)` | Match by substring in last user message |
| `.match_model(name)` | Match by model name (substring) |
| `.match_header(name, value)` | Match an HTTP header by substring |
| `.match_system_prompt(pattern)` | Match provider-specific system prompt text |
| `.match_temperature(value)` | Match exact `temperature` value |
| `.match_temperature_range(min, max)` | Match inclusive temperature range; either bound may be `None` |
| `.match_metadata(key, value)` | Match top-level request metadata scalar values |
| `.match_tool_schema(pattern)` | Match declared tool/function names |
| `.match_body_jsonpath(path)` | Match full request body with JSONPath; requires `jsonpath` feature |
| `.respond_with_content(text)` | Set text response content |
| `.respond_with_tool_calls(vec)` | Set tool call response |
| `.with_error(status, message)` | Set error response |
| `.with_error_headers(status, message, headers)` | Set error response with custom headers; returns `Result<Self, String>` |
| `.with_streaming(Some(latency), Some(chunk_size))` | Configure streaming parameters (either arg may be `None`) |
| `.with_failure(FailureConfig)` | Configure failure simulation |
| `.with_stop_reason(reason)` | Set custom stop/finish reason |
| `.with_finish_reason(reason)` | Set custom finish reason; handlers treat it like `stop_reason` unless both are set |
| `.respond_with_refusal(reason)` | Set provider-native safety refusal response |
| `.for_provider(Provider)` | Restrict fixture to a specific provider |
| `.with_scenario(name, required_state, set_state)` | Attach to a named scenario state machine (see [Scenarios](scenarios.md)) |
| `.with_priority(i32)` | Prefer this fixture over lower-priority fixtures |
| `.as_catch_all()` | Defer this fixture to the fallback pass |

Note: For regex matching, use the YAML fixture format with `regex:` syntax. The programmatic builder uses substring matching.

## YAML Loading

Load fixtures from YAML files instead of building them programmatically. The
file must be a YAML object with a top-level `fixtures:` key containing a list
of fixture definitions (see [Fixture Format Reference](fixtures.md#file-schema)
for the full schema).

```yaml
# fixtures.yaml — minimal example
fixtures:
  - match:
      user_message: "hello"
    response:
      content: "Hi from YAML!"
```

```rust
use std::path::Path;

// Load from a single file
let server = ServerBuilder::new()
    .load_yaml(Path::new("fixtures.yaml"))?
    .build()
    .await?;

// Load from a directory (all .yaml/.yml files)
let server = ServerBuilder::new()
    .load_yaml_dir(Path::new("fixtures/"))?
    .build()
    .await?;
# Ok::<_, Box<dyn std::error::Error>>(())
```

See [`examples/fixtures/`](../examples/fixtures/) for complete working
YAML files covering text responses, tool calls, streaming, errors,
failures, refusals, provider scoping, and multi-turn scenarios.

## Provider Targeting

By default, fixtures serve all provider endpoints. The server determines the response format from the route:

- `/v1/chat/completions` → OpenAI format
- `/v1/messages` → Anthropic format
- `/v1/responses` → Responses API format
- `/v1beta/models/*` → Gemini format

No configuration needed — the same fixture content is formatted for each provider automatically.

## Authentication

Bearer token enforcement on LLM endpoints — off by default.

```rust
let server = ServerBuilder::new()
    .with_bearer_token("test-token-123")          // unlimited uses
    .with_bearer_token_uses("short-lived", 1)     // expires after 1 LLM request
    .fixture(Fixture::new().respond_with_content("hello"))
    .build().await.unwrap();
```

Requests without a valid `Authorization: Bearer <token>` header get a provider-specific 401.

### OAuth 2.0 Mock Server

Enable a companion OAuth server (via `oauth-mock`) for full token lifecycle testing:

```rust
let server = ServerBuilder::new()
    .with_oauth_defaults()
    .fixture(Fixture::new().respond_with_content("hello"))
    .build().await.unwrap();

let oauth_url = server.oauth_url().unwrap();  // separate port
// Point your client's token_url at oauth_url
// Tokens issued by OAuth are automatically valid on LLM endpoints
```

The OAuth feature is enabled by default. Disable with `default-features = false` in Cargo.toml.

## Deterministic IDs

All response IDs are deterministic and sequential per server instance:

- OpenAI: `chatcmpl-llmposter-1`, `chatcmpl-llmposter-2`, ...
- Anthropic: `msg-llmposter-1`, `msg-llmposter-2`, ...
- Responses: `resp-llmposter-1`, `resp-llmposter-2`, ...

This makes snapshot testing reliable.
