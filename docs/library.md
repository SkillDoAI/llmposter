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
| `.load_yaml(path)` | Load fixtures from a YAML file (returns `Result`) |
| `.load_yaml_dir(path)` | Load fixtures from a directory (returns `Result`) |
| `.bind(addr)` | Set bind address (default: `127.0.0.1:0`) |
| `.verbose(bool)` | Enable verbose logging to stderr |
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
| `.check_error()` | Check for post-bind server errors |

The server runs on a random port by default (port 0). Drop the `MockServer` to stop it.

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
| `.respond_with_content(text)` | Set text response content |
| `.respond_with_tool_calls(vec)` | Set tool call response |
| `.with_error(status, message)` | Set error response |
| `.with_streaming(Some(latency), Some(chunk_size))` | Configure streaming parameters (either arg may be `None`) |
| `.with_failure(FailureConfig)` | Configure failure simulation |
| `.with_stop_reason(reason)` | Set custom stop/finish reason |
| `.with_finish_reason(reason)` | Alias for `.with_stop_reason()` |
| `.for_provider(Provider)` | Restrict fixture to a specific provider |
| `.with_scenario(name, required_state, set_state)` | Attach to a named scenario state machine (see [Scenarios](scenarios.md)) |

Note: For regex matching, use the YAML fixture format with `regex:` syntax. The programmatic builder uses substring matching.

## YAML Loading

```rust
// Load from file
let server = ServerBuilder::new()
    .load_yaml(Path::new("fixtures.yaml"))?
    .build()
    .await?;

// Load from directory
let server = ServerBuilder::new()
    .load_yaml_dir(Path::new("fixtures/"))?
    .build()
    .await?;
```

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
