---
name: llmposter
description: Mock HTTP server for LLM provider APIs (OpenAI, Anthropic, Gemini, Responses API). Use when writing integration tests that need deterministic, controllable LLM API responses without calling real providers. Supports fixture-based request matching, SSE streaming, failure injection, auth simulation, and all four provider response formats.
license: AGPL-3.0-or-later
metadata:
  author: SkillDoAI
  version: "0.4.8"
  ecosystem: rust
  generated-by: skilldo/claude-sonnet-4-6 + review:claude-sonnet-4-6
---


# llmposter

Mock HTTP server for LLM provider APIs. Clients point their base URL at llmposter and interact using real API paths — no code changes beyond the URL swap. Fixtures define request matchers and canned responses for Anthropic (`/v1/messages`), OpenAI (`/v1/chat/completions`), Gemini (`/v1beta/models/{model}:generateContent` non-streaming, `/v1beta/models/{model}:streamGenerateContent` streaming), and Responses API (`/v1/responses`). No provider prefix in routes — clients use the same paths as real APIs.

## Imports

```rust
// Core types (re-exported at crate root)
use llmposter::{Fixture, Provider, ServerBuilder};

// Fixture sub-types (in llmposter::fixture module)
use llmposter::fixture::{FailureConfig, FixtureResponse, StreamingConfig, ToolCall};
```

```toml
[dependencies]
llmposter = "0.4.8"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.13", default-features = false, features = ["json"] }
serde_json = "1"
```

Optional feature flags:

```toml
# Default features: ["oauth", "watch", "jsonpath"] — opt-in: ui, templating
llmposter = { version = "0.4", features = ["ui", "templating"] }

# Minimal (no oauth, no watch, no jsonpath)
llmposter = { version = "0.4", default-features = false }
```

## Quick Start

```rust
use llmposter::{Fixture, ServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from Claude mock!"),
        )
        .build()
        .await?;

    // Point any LLM client's base_url at server.url()
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await?;

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await?;
    assert_eq!(body["type"], "message");
    assert_eq!(body["content"][0]["text"], "Hi from Claude mock!");
    assert_eq!(body["stop_reason"], "end_turn");
    Ok(())
}
```

## API Reference

### ServerBuilder

Builder for `MockServer`. Re-exported at crate root.

- `ServerBuilder::new() -> Self` — create an empty builder
- `.fixture(f: Fixture) -> Self` — add a single fixture (chainable)
- `.fixtures(fixtures: Vec<Fixture>) -> Self` — append a vector of fixtures
- `.load_yaml(path: &Path) -> Result<Self, Box<dyn Error>>` — load fixtures from a YAML file
- `.load_yaml_dir(dir: &Path) -> Result<Self, Box<dyn Error>>` — load all YAML files in a directory
- `.fixture_count(&self) -> usize` — number of fixtures currently loaded
- `.models(models: Vec<String>) -> Self` — set explicit model list returned by `GET /v1/models`. When unset, models are auto-derived from fixtures.
- `.bind(addr: &str) -> Self` — bind address. Default: random port on `127.0.0.1`.
- `.verbose(v: bool) -> Self` — when `true`, 404 responses include the `"No fixture matched"` diagnostic detail.
- `.diagnostics(enabled: bool) -> Self` — when `true`, 404 responses include the nearest-match fixture and per-field pass/fail breakdown.
- `.capture_capacity(max: usize) -> Self` — max captured requests in ring buffer. Library default: unbounded. `0` disables capture entirely.
- `.with_auth(enabled: bool) -> Self` — enable bearer-token auth on LLM routes
- `.with_bearer_token(token: &str) -> Self` — register a bearer token with unlimited uses
- `.with_bearer_token_uses(token: &str, max_uses: u64) -> Self` — register a bearer token capped at `max_uses` requests
- `.with_oauth(config: OAuthConfig) -> Self` — enable embedded OAuth mock with a custom config (**`oauth` feature**, on by default)
- `.with_oauth_defaults(self) -> Self` — enable embedded OAuth mock with defaults (**`oauth` feature**, on by default)
- `.watch(enabled: bool) -> Self` — enable hot-reload of fixture files (**`watch` feature**)
- `.ui(enabled: bool) -> Self` — enable debug UI at `/ui` (**`ui` feature**)
- `.build(self) -> Result<MockServer, Box<dyn Error>>` — **async**. Validates fixtures and starts the server on the configured bind (random port by default).

### MockServer

Running server handle. Re-exported at crate root.

- `.url(&self) -> String` — base URL (e.g., `http://127.0.0.1:PORT`)
- `.port(&self) -> u16` — bound port number
- `.check_error(&self) -> Result<(), String>` — **async**. Returns the latest background error (e.g., fixture reload failure) or `Ok(())` if none.
- `.get_requests(&self) -> Vec<CapturedRequest>` — all captured requests in chronological order
- `.request_count(&self) -> usize` — total captured request count
- `.fixture_count(&self) -> usize` — currently loaded fixture count
- `.explicit_models(&self) -> Option<&[String]>` — explicit model list if set via `ServerBuilder::models()`, else `None`
- `.matched_requests(&self) -> Vec<CapturedRequest>` — captured requests whose outcome is `Matched`
- `.matched_count(&self) -> usize` — number of matched requests
- `.assert_matched(&self, substring: &str)` — panics unless a matched request body contains `substring`
- `.assert_not_matched(&self, substring: &str)` — panics if any matched request body contains `substring`
- `.scenario_state(&self, name: &str) -> Option<String>` — current state for the named scenario
- `.set_fixtures(&self, fixtures: Vec<Fixture>) -> Result<(), Box<dyn Error + Send + Sync>>` — atomically swap the active fixture set
- `.reset(&self)` — clear captured requests and reset scenario state
- `.oauth_url(&self) -> Option<String>` — OAuth mock base URL when enabled (**`oauth` feature**)
- `.oauth_client_credentials(&self) -> Option<(String, String)>` — **async**. `(client_id, client_secret)` when enabled (**`oauth` feature**)
- `.approve_device_code(&self, user_code: &str) -> Result<(), Box<dyn Error>>` — **async**. Approves a pending OAuth device-code grant (**`oauth` feature**)

### Fixture

Central type for defining mock behavior. Re-exported at crate root. All builder methods return `Self` (chainable). The struct uses `#[serde(deny_unknown_fields)]`.

**Constructor & priority:**
- `Fixture::new() -> Self`
- `.with_priority(priority: i32) -> Self` — higher matches first. Default is `0`.
- `.as_catch_all(self) -> Self` — marks the fixture as a fallback; catch-all fixtures are checked only after all non-catch-all fixtures regardless of priority.

**Match methods:**
- `.match_user_message(pattern: &str)` — substring match on the last user message
- `.match_model(pattern: &str)` — substring match on the model field
- `.match_header(name: &str, value: &str)` — substring match on a request header
- `.match_system_prompt(pattern: &str)` — substring match on the system prompt
- `.match_temperature(value: f64)` — exact temperature match
- `.match_temperature_range(min: Option<f64>, max: Option<f64>)` — inclusive range match on temperature
- `.match_metadata(key: &str, value: &str)` — substring match on a metadata field
- `.match_tool_schema(pattern: &str)` — substring match against tool/function schema JSON
- `.match_body_jsonpath(path: &str)` — request body matches JSONPath expression (**`jsonpath` feature**, on by default)
- `.for_provider(provider: Provider)` — restrict to one provider endpoint

**Response methods:**
- `.respond_with_content(content: &str)` — text response
- `.respond_with_tool_calls(tool_calls: Vec<ToolCall>)` — tool-use response (mutually exclusive with text content)
- `.respond_with_embedding(embedding: Vec<f64>)` — explicit embedding vector for `/v1/embeddings`
- `.respond_with_refusal(reason: &str)` — OpenAI-style refusal
- `.with_stop_reason(reason: &str)` — override Anthropic stop reason
- `.with_finish_reason(reason: &str)` — override OpenAI finish reason
- `.with_error(status: u16, message: &str)` — HTTP error response
- `.with_error_headers<I, K, V>(status, message, headers) -> Result<Self, String>` — error response with custom headers. Rejects duplicate (case-insensitive) header names at construction time.
- `.with_streaming(latency: Option<u64>, chunk_size: Option<usize>)` — enable SSE streaming with optional inter-chunk latency (ms) and chunk size (chars). Builder parameter name is `latency`; the `StreamingConfig` struct field is also `latency`.
- `.with_failure(failure: FailureConfig)` — inject failure behaviors
- `.with_scenario(name: &str, required_state: Option<&str>, set_state: Option<&str>) -> Self` — make this fixture match only when the named scenario is in `required_state`; on match, advance to `set_state`.
- `.validate(&mut self) -> Result<(), String>` — verify field combinations; called automatically by `ServerBuilder::build()`.

**Public struct fields** (for direct construction):
- `Fixture.match_rule: Option<FixtureMatch>` — match criteria
- `Fixture.provider: Option<Provider>` — provider restriction
- `Fixture.response: Option<FixtureResponse>` — response configuration
- `Fixture.error: Option<FixtureError>` — error response
- `Fixture.refusal: Option<Refusal>` — refusal payload
- `Fixture.failure: Option<FailureConfig>` — failure injection
- `Fixture.streaming: Option<StreamingConfig>` — SSE streaming config
- `Fixture.scenario: Option<ScenarioConfig>` — stateful multi-turn scenario matching
- `Fixture.priority: Option<i32>` — when `None`, treated as `0`. Higher values match first.
- `Fixture.catch_all: bool` — when `true`, fixture is checked only after all non-catch-all fixtures.

### FixtureResponse

In `llmposter::fixture`. Derives `Default`. For custom response construction.

- `FixtureResponse.content: Option<String>` — text body. When `None`, no text content block is returned.
- `FixtureResponse.content_template: Option<String>` — minijinja template (**`templating` feature**). When `None`, `content` is used as-is.
- `FixtureResponse.tool_calls: Option<Vec<ToolCall>>` — tool-use responses. Mutually exclusive with `content`.
- `FixtureResponse.stop_reason: Option<String>` — Anthropic stop reason. When `None`, defaults to `"end_turn"` for text, `"tool_use"` for tool calls.
- `FixtureResponse.finish_reason: Option<String>` — OpenAI finish reason. When `None`, defaults to `"stop"` for text.
- `FixtureResponse.embedding: Option<Vec<f64>>` — explicit embedding vector for `/v1/embeddings`. When `None`, a deterministic FNV-1a-seeded 1536-dim L2-normalized vector is generated.

### ToolCall

In `llmposter::fixture`. Re-exported at crate root.

- `ToolCall.name: String` — tool function name (required)
- `ToolCall.arguments: serde_json::Value` — tool input as a parsed JSON Value (required). NOT a stringified JSON string. Use `serde_json::json!({...})`.

### FailureConfig

In `llmposter::fixture`. Re-exported at crate root. Derives `Default`.

- `FailureConfig.latency_ms: Option<u64>` — delay (ms) before response. When `None`, no delay.
- `FailureConfig.corrupt_body: Option<bool>` — when `Some(true)`, returns literal string `"overloaded"` as `text/plain` with HTTP 200. Configured content is ignored entirely.
- `FailureConfig.truncate_after_frames: Option<u32>` — cut SSE stream after N frames. Stream ends without `message_stop`. Ignored (with warning) on non-streaming.
- `FailureConfig.disconnect_after_ms: Option<u64>` — abort connection after N ms. Requires streaming with `latency > 0`; with `latency = 0`, frames may complete before the disconnect timer fires.
- `FailureConfig.probability: Option<f32>` — probability ([0.0, 1.0]) that failure applies. When `None`, failure always applies.
- `FailureConfig.latency_jitter_ms: Option<u64>` — random jitter added to latency. Requires `latency_ms` to be `Some`; rejected at fixture load time without it.
- `FailureConfig.duplicate_frames: Option<bool>` — when `Some(true)`, duplicate SSE frames.
- `FailureConfig.chaos_seed: Option<u64>` — seed for deterministic chaos reproduction. Set this alongside `probability`, `latency_jitter_ms`, or `duplicate_frames` for reproducible runs.

### StreamingConfig

Re-exported at crate root.

- `StreamingConfig.latency: Option<u64>` — inter-chunk delay (ms). Note: the struct field is `latency` and the builder parameter is also `latency`.
- `StreamingConfig.chunk_size: Option<usize>` — characters per chunk for text content. Ignored for tool-call streaming across all four providers.

### ScenarioConfig

Re-exported at crate root.

- `ScenarioConfig.name: String` — scenario identifier (required)
- `ScenarioConfig.required_state: Option<String>` — fixture matches only when the scenario is in this state. When `None`, matches in any state.
- `ScenarioConfig.set_state: Option<String>` — on match, advance the scenario to this state.

### Refusal

Re-exported at crate root.

- `Refusal.reason: String` — refusal text injected into the response

### Provider

Enum (exactly 4 variants). Re-exported at crate root. Serde uses `rename_all = "lowercase"`, so YAML uses `openai`, `anthropic`, `gemini`, `responses`.

- `Provider::OpenAI` — serves `POST /v1/chat/completions`
- `Provider::Anthropic` — serves `POST /v1/messages`
- `Provider::Gemini` — serves `POST /v1beta/models/{model}:generateContent` (non-streaming) and `POST /v1beta/models/{model}:streamGenerateContent` (streaming)
- `Provider::Responses` — serves `POST /v1/responses` (OpenAI Responses API)
- `.as_str(&self) -> &'static str` — lowercase string representation

The OpenAI Responses API variant is named `Provider::Responses`, not `OpenAIResponses`.

### AuthState and TokenStatus

Re-exported at crate root. Auth only protects LLM routes — `/health`, `/code/{N}`, `/v1/models`, `/v1/embeddings`, `/v1/moderations`, and `/ui` are never auth-protected.

- `AuthState::new() -> Self`
- `.add_token(&self, token: &str, max_uses: Option<u64>)` — register a bearer token. `None` means unlimited uses.
- `.check_and_use(&self, token: &str) -> TokenStatus` — atomically check and decrement use count
- `.revoke(&self, token: &str)` — move a token to the deny-list

`TokenStatus` enum (3 variants):
- `TokenStatus::Valid` — token is registered and has remaining uses
- `TokenStatus::Exhausted` — token is deny-listed (uses ran out OR `revoke()` was called)
- `TokenStatus::Unknown` — token was never registered

### OAuthConfig

**Requires `oauth` feature** (on by default). Re-exported at crate root.

- `OAuthConfig.client_id: String`
- `OAuthConfig.client_secret: String`
- `OAuthConfig.redirect_uris: Vec<String>`
- `OAuthConfig.scopes: Vec<String>`

### CapturedRequest and RequestOutcome

Both re-exported at crate root. `CapturedRequest` is `#[non_exhaustive]`.

`CapturedRequest` fields:
- `method: String`
- `path: String`
- `body: String`
- `outcome: RequestOutcome`
- `matched_scenario: Option<String>` — scenario name if matched
- `capture_id: u64` — monotonic capture index
- `status_code: u16` — HTTP status returned (pre-chaos; see Pitfalls)
- `timestamp: Instant`
- `.was_matched(&self) -> bool` — true when `outcome == Matched`

`RequestOutcome` enum (`#[non_exhaustive]`):
- `RequestOutcome::Matched` — fixture matched and returned a response
- `RequestOutcome::NoFixtureMatch` — 404 path
- `RequestOutcome::BadRequest` — 400 validation failure
- `RequestOutcome::AuthRejected` — 401 from auth middleware
- `RequestOutcome::CodeEndpoint` — `/code/{N}` request
- `RequestOutcome::ModerationEndpoint` — `/v1/moderations` request
- `.label(&self) -> &'static str` and `.default_status(&self) -> u16`

### Matching Types

In `llmposter::fixture` (NOT re-exported at crate root):

- `FixtureMatch` — match criteria struct: `user_message`, `model`, `headers: HashMap<String, StringMatch>`, `system_prompt`, `temperature: Option<F64Match>`, `metadata: HashMap<String, StringMatch>`, `tool_schema`, `body_jsonpath: Option<String>`
- `FixtureError` — `{ status: u16, message: String, headers: HashMap<String, String> }`
- `StringMatch::Substring(String)` — default variant, substring/contains matching
- `StringMatch::Regex(RegexMatch)` — regex matching; construct via `StringMatch::regex(pattern: &str)`. Use anchors `^...$` for exact matching (no `Exact` variant exists).
- `RegexMatch { regex: String }`
- `F64Match::Exact(f64)` | `F64Match::Range(F64Range)`
- `F64Range { min: Option<f64>, max: Option<f64> }` — inclusive bounds

### CLI Module

In `llmposter::cli`. Useful when embedding the binary in test harnesses.

- `Cli` — clap-derived argument struct with fields `fixtures`, `validate`, `port`, `bind`, `verbose`, `capture_capacity`, `diagnostics`, and feature-gated `watch`, `ui`.
- `cli::run(cli: &Cli) -> Result<Option<MockServer>, Box<dyn Error>>` — **async**. Runs the CLI flow, returning `None` for `--validate` and `Some(MockServer)` otherwise.
- `cli::run_with_output(cli: &Cli, out: &mut (dyn Write + Send))` — **async**. Variant that writes startup messages to a custom sink.

## Core Patterns

### Tool Use Response

```rust
mod tool_use_example {
    use llmposter::fixture::ToolCall;
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("weather")
                    .respond_with_tool_calls(vec![ToolCall {
                        name: "get_weather".to_string(),
                        arguments: serde_json::json!({
                            "location": "London",
                            "unit": "celsius"
                        }),
                    }]),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "What's the weather?"}]
            }))
            .send()
            .await?;

        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["type"], "tool_use");
        assert_eq!(body["content"][0]["name"], "get_weather");
        // Tool-call IDs are deterministic: toolu_llmposter_{N} (1-indexed
        // process-wide counter). Use starts_with — the exact N depends on
        // how many other requests the process has served.
        let tool_id = body["content"][0]["id"].as_str().unwrap();
        assert!(tool_id.starts_with("toolu_llmposter_"));
        // Anthropic uses "input" field for tool-call arguments
        assert_eq!(body["content"][0]["input"]["location"], "London");
        Ok(())
    }
}
```

### SSE Streaming

```rust
mod streaming_example {
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("hello")
                    .respond_with_content("Hello world")
                    .with_streaming(Some(0), Some(5)), // latency=0ms, chunk_size=5 chars
            )
            .build()
            .await?;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "hello"}],
                "stream": true
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        let text = resp.text().await?;
        // SSE events: message_start, content_block_start, content_block_delta,
        //             content_block_stop, message_delta, message_stop
        assert!(text.contains("event: message_start"));
        assert!(text.contains("event: content_block_delta"));
        assert!(text.contains("event: message_stop"));
        Ok(())
    }
}
```

### Error Simulation

```rust
mod error_simulation {
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_user_message("rate limit")
                    .with_error(429, "Rate limit exceeded"),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "rate limit test"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 429);
        // Error shape is {"error": {"message": "..."}} for ALL providers
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["error"]["message"], "Rate limit exceeded");
        Ok(())
    }
}
```

### Failure Injection (Latency)

```rust
mod latency_injection {
    use llmposter::fixture::FailureConfig;
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("delayed response")
                    .with_failure(FailureConfig {
                        latency_ms: Some(200),
                        ..Default::default()
                    }),
            )
            .build()
            .await?;

        let start = std::time::Instant::now();
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;

        assert_eq!(resp.status(), 200);
        assert!(start.elapsed().as_millis() >= 180);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["content"][0]["text"], "delayed response");
        Ok(())
    }
}
```

### Corrupt Body (Overloaded Simulation)

```rust
mod corrupt_body_example {
    use llmposter::fixture::FailureConfig;
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("should not appear")
                    .with_failure(FailureConfig {
                        corrupt_body: Some(true),
                        ..Default::default()
                    }),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;

        // Returns HTTP 200 with text/plain body "overloaded" — NOT JSON
        assert_eq!(resp.status(), 200);
        let text = resp.text().await?;
        assert_eq!(text, "overloaded");
        Ok(())
    }
}
```

### Provider Filtering

```rust
mod provider_filtering {
    use llmposter::{Fixture, Provider, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .for_provider(Provider::OpenAI)
                    .respond_with_content("openai only"),
            )
            .fixture(
                Fixture::new()
                    .for_provider(Provider::Anthropic)
                    .respond_with_content("anthropic only"),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();

        // OpenAI fixture matches on /v1/chat/completions
        let resp = client
            .post(format!("{}/v1/chat/completions", server.url()))
            .json(&serde_json::json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);

        // OpenAI fixture does NOT match /v1/messages — Anthropic fixture does
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["content"][0]["text"], "anthropic only");
        Ok(())
    }
}
```

### Custom Stop Reason

```rust
mod custom_stop_reason {
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("hit max tokens")
                    .with_stop_reason("max_tokens"),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;

        let body: serde_json::Value = resp.json().await?;
        assert_eq!(body["stop_reason"], "max_tokens");
        assert_eq!(body["content"][0]["text"], "hit max tokens");
        Ok(())
    }
}
```

### Stream Truncation

```rust
mod stream_truncation {
    use llmposter::fixture::FailureConfig;
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("long text that gets truncated")
                    .with_streaming(Some(0), Some(5))
                    .with_failure(FailureConfig {
                        truncate_after_frames: Some(2),
                        ..Default::default()
                    }),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}],
                "stream": true
            }))
            .send()
            .await?;

        let text = resp.text().await?;
        assert!(text.contains("event: message_start"));
        // Stream ends abruptly — no message_stop event emitted
        assert!(!text.contains("event: message_stop"));
        Ok(())
    }
}
```

### Bearer Token Authentication

```rust
mod bearer_auth {
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .with_auth(true)
            .with_bearer_token_uses("sk-test-token", 3)
            .fixture(Fixture::new().respond_with_content("authorized"))
            .build()
            .await?;

        let client = reqwest::Client::new();

        // Without bearer token: 401
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 401);

        // With token: 200
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .bearer_auth("sk-test-token")
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);

        // /health is NEVER auth-protected
        let health = client.get(format!("{}/health", server.url())).send().await?;
        assert_eq!(health.status(), 200);
        Ok(())
    }
}
```

### Stateful Scenarios (Retry Behavior)

```rust
mod scenario_retry {
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        // First call: 429. After first failure, scenario advances to "retried";
        // second call matches the success fixture. The success fixture has
        // higher priority so it wins over the (still-matching) failure fixture
        // once the scenario state advances.
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .with_scenario("retry-flow", None, Some("retried"))
                    .with_error(429, "Rate limit; please retry"),
            )
            .fixture(
                Fixture::new()
                    .with_scenario("retry-flow", Some("retried"), None)
                    .respond_with_content("success after retry")
                    .with_priority(10),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();
        let req = || {
            client
                .post(format!("{}/v1/messages", server.url()))
                .json(&serde_json::json!({
                    "model": "claude-sonnet-4-6",
                    "max_tokens": 1024,
                    "messages": [{"role": "user", "content": "ping"}]
                }))
                .send()
        };

        let first = req().await?;
        assert_eq!(first.status(), 429);

        let second = req().await?;
        assert_eq!(second.status(), 200);
        let body: serde_json::Value = second.json().await?;
        assert_eq!(body["content"][0]["text"], "success after retry");
        Ok(())
    }
}
```

### YAML Fixtures

```rust
mod yaml_fixtures {
    use llmposter::ServerBuilder;
    use std::path::Path;

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .load_yaml(Path::new("fixtures/anthropic.yaml"))?
            .build()
            .await?;

        println!("Mock server at {} with fixtures loaded", server.url());
        Ok(())
    }
}
```

YAML fixture format:

```yaml
# fixtures/anthropic.yaml
fixtures:
  - match:
      user_message: "hello"
    response:
      content: "Hi from the mock!"

  - match:
      model: "claude-sonnet"
      user_message: "weather"
    response:
      tool_calls:
        - name: get_weather
          arguments:
            location: London
            unit: celsius

  - match:
      user_message: "fail"
    error:
      status: 429
      message: "Rate limit exceeded"

  - match:
      user_message: "slow"
    response:
      content: "delayed"
    streaming:
      latency: 50
      chunk_size: 5
    failure:
      latency_ms: 500

  - priority: 10
    catch_all: true
    response:
      content: "fallback response"
```

The top-level `fixtures:` key is required — a bare list will fail to load.

### Request Capture and Assertion Helpers

```rust
mod request_capture {
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(Fixture::new().respond_with_content("captured"))
            .capture_capacity(100)
            .build()
            .await?;

        let client = reqwest::Client::new();
        client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "ping the mock"}]
            }))
            .send()
            .await?;

        assert_eq!(server.request_count(), 1);
        assert_eq!(server.matched_count(), 1);
        server.assert_matched("ping the mock");
        server.assert_not_matched("does-not-appear");

        let captured = server.get_requests();
        assert_eq!(captured[0].method, "POST");
        assert_eq!(captured[0].path, "/v1/messages");
        assert!(captured[0].was_matched());
        Ok(())
    }
}
```

### Model Matching

```rust
mod model_matching {
    use llmposter::{Fixture, ServerBuilder};

    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let server = ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .match_model("claude-sonnet")
                    .respond_with_content("sonnet response"),
            )
            .build()
            .await?;

        let client = reqwest::Client::new();

        // "claude-sonnet" is a substring of "claude-sonnet-4-6" — matches
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-sonnet-4-6",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 200);

        // "claude-sonnet" is NOT a substring of "claude-haiku-3" — 404
        let resp = client
            .post(format!("{}/v1/messages", server.url()))
            .json(&serde_json::json!({
                "model": "claude-haiku-3",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await?;
        assert_eq!(resp.status(), 404);
        Ok(())
    }
}
```

## Endpoint Reference

No provider prefix in routes — clients use real API paths, just swap the base URL.

| Provider | Endpoint | Notes |
|----------|----------|-------|
| Anthropic | `POST /v1/messages` | Requires `max_tokens` in request body |
| OpenAI | `POST /v1/chat/completions` | |
| OpenAI legacy | `POST /v1/completions` | |
| Gemini (non-streaming) | `POST /v1beta/models/{model}:generateContent` | Model name in URL path |
| Gemini (streaming) | `POST /v1beta/models/{model}:streamGenerateContent` | Distinct path; use `?alt=sse` for true SSE transport |
| Responses | `POST /v1/responses` | OpenAI Responses API; `input` field optional |
| Embeddings | `POST /v1/embeddings` | Default vector: 1536-dim, FNV-1a seeded, L2-normalized |
| Moderations | `POST /v1/moderations` | Static `flagged: false` response, never auth-protected |
| Models list | `GET /v1/models` | Auto-derived from fixtures or `ServerBuilder::models()` |
| Health | `GET /health` | Returns `{"status": "ok"}`. Never auth-protected. |
| Status echo | `GET /code/{status}` | Returns specified HTTP status (100–599). Never auth-protected. |

### Response Shapes

**Anthropic non-streaming:**
- `body.type`: `"message"`
- `body.role`: `"assistant"`
- `body.id`: `"msg-llmposter-{N}"` (hyphens, not underscores — NOT `msg_...`; `N` is a 1-indexed monotonic counter shared across providers in a process)
- `body.content[N].type`: `"text"` or `"tool_use"`
- `body.content[N].id` (tool_use): `"toolu_llmposter_{N}"` (1-indexed)
- `body.content[N].input` (tool_use): the JSON object from `ToolCall.arguments`
- `body.stop_reason`: `"end_turn"` (text) | `"tool_use"` (tool calls) | custom
- `body.usage.input_tokens` / `output_tokens`: `u64` (approximate, `bytes/4` heuristic)

**Anthropic streaming** (SSE, `Content-Type: text/event-stream`):
Events in order: `message_start`, `content_block_start`, `content_block_delta` (repeated), `content_block_stop`, `message_delta`, `message_stop`

**Error responses** (all providers): `{"error": {"message": "..."}}` — nested envelope, not a top-level `message`.

**`/code/{status}`:** Returns the specified HTTP status code. `/code/204`, `/code/304`, and `/code/205` return empty bodies.

**Corrupt body** (`failure.corrupt_body: true`): literal string `"overloaded"` with `Content-Type: text/plain` and HTTP 200.

## Behavioral Semantics

- **Matching order**: first-match-wins with priority override. Non-catch-all fixtures are sorted by descending priority (higher wins), then file/declaration order breaks ties. Catch-all fixtures (`catch_all: true`) are always checked after all non-catch-all fixtures, regardless of priority. Default priority is `0`.
- **Match fields stack conjunctively**: a fixture with both `model` and `tool_schema` requires every condition to match.
- **Substring matching**: `match_user_message` and `match_model` use substring/contains matching. `"hello"` matches `"hello world"`. No `Exact` variant exists — use `StringMatch::regex("^exact$")` for exact matching.
- **Prompt redaction in no-match errors**: no-match error responses redact prompt content to avoid leaking sensitive content in logs and error messages.
- **Response IDs**: always `msg-llmposter-{N}` (hyphens, monotonic counter) for Anthropic — not a UUID. Counter is shared with OpenAI (`chatcmpl-llmposter-{N}`) and Responses (`resp-llmposter-{N}`).
- **Tool-call IDs**: deterministic `toolu_llmposter_{N}` (1-indexed, sequential), not random UUIDs.
- **Token counts**: `bytes/4` heuristic — not a real tokenizer. Never assert exact values, only `> 0`. Token totals use `saturating_add` across all format builders (v0.4.8) — no panic on extreme counts.
- **Anthropic tool input field**: `ToolCall.arguments` (Rust) maps to `content[].input` (JSON), not `content[].arguments`.
- **Anthropic `stop_reason`**: defaults to `"end_turn"` for text, `"tool_use"` for tool calls. Not `"stop"` (that is OpenAI's `finish_reason`).
- **`max_tokens` required for Anthropic** — missing `max_tokens` returns a 400 validation error.
- **Non-boolean `stream` field rejected**: requests with `stream` set to a non-boolean (e.g., `"yes"`) return an error.
- **Auth scope**: LLM routes only. `/health`, `/code/{N}`, `/v1/models`, `/v1/embeddings`, `/v1/moderations`, and `/ui` are never auth-protected.
- **`corrupt_body`**: always returns literal `"overloaded"` as `text/plain` with HTTP 200. Configured content is ignored. On streaming requests, emits a malformed SSE frame.
- **`chunk_size` ignored for tool-call streaming** across all four providers. Only affects text-content streaming.
- **Gemini streaming endpoints**: non-streaming at `/v1beta/models/{model}:generateContent`; streaming at `/v1beta/models/{model}:streamGenerateContent`. Non-SSE streaming buffers all chunks in memory and returns a single JSON array — use `?alt=sse` for true SSE transport failure simulation.
- **Hot-reload**: via `--watch` flag (requires `watch` feature) or `kill -HUP <pid>` (SIGHUP). Fixtures are swapped atomically with priority re-sorting at load time. Invalid YAML keeps prior fixtures serving — the server is never taken down by a bad reload.
- **Load-time validation**: invalid JSONPath, duplicate response headers (case-insensitive), and `latency_jitter_ms` without `latency_ms` are all rejected when fixtures are loaded — not at request time.
- **OpenAI first streaming chunk**: omits `content` field entirely via `skip_serializing_if`, not `"content": null`. All major SDKs treat absent and null identically.
- **Captured request status under chaos**: capture runs before chaos logic. A chaos-injected 500 shows as 200 in the capture log; verify chaos failures via the HTTP response, not the capture log.
- **`truncate_after_frames` / `disconnect_after_ms` on non-streaming**: warning emitted, fields ignored.
- **`disconnect_after_ms`**: requires streaming with `latency > 0`. With `latency = 0`, frames may complete before disconnect timer fires.
- **Content templating**: uses minijinja via `content_template` field (**`templating` feature**).
- **`/v1/embeddings`**: default vector is 1536-dim, L2-normalized, FNV-1a seeded by input. Deterministic per input. Override via `respond_with_embedding(Vec<f64>)`.
- **`/v1/moderations`**: static OpenAI-compatible response with `flagged: false`. Always returned, never computed from input.
- **Tool-call argument serialization fails**: falls back to `"{}"` (v0.4.8). Previous versions used `""`.

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `oauth` | **on** | `OAuthConfig`, `with_oauth*`, `oauth_url`, `approve_device_code` |
| `watch` | **on** | Hot-reload via file watcher |
| `jsonpath` | **on** | `body_jsonpath` match field |
| `ui` | off | Debug UI at `/ui` |
| `templating` | off | minijinja `content_template` rendering |

Disable defaults: `llmposter = { version = "0.4", default-features = false, features = ["jsonpath"] }`

## CLI Usage

```bash
# Start mock server with YAML fixtures (default port: 2112)
llmposter --fixtures fixtures/

# Validate fixtures without starting server
llmposter --fixtures fixtures/ --validate

# Custom port and bind address
llmposter --fixtures fixtures/ --port 8080 --bind 0.0.0.0

# Verbose logging (404 responses include match failure detail)
llmposter --fixtures fixtures/ --verbose

# Diagnostic 404s — show nearest-match fixture with per-field pass/fail
llmposter --fixtures fixtures/ --diagnostics

# Set capture capacity (CLI default: 1000; 0 disables capture)
llmposter --fixtures fixtures/ --capture-capacity 5000

# Enable debug UI (requires ui feature)
llmposter --fixtures fixtures/ --ui

# Enable hot-reload (requires watch feature)
llmposter --fixtures fixtures/ --watch
```

## Pitfalls

- **Streaming config field name**: when constructing `StreamingConfig` directly, the field is `latency` (not `latency_ms`). The builder method `with_streaming(latency, chunk_size)` also uses `latency` as its parameter name. `StreamingConfig { latency_ms: ... }` will not compile.
- **`chunk_size` and tool calls**: `chunk_size` is silently ignored for tool-call streaming across all four providers. It only affects text-content streaming.
- **Gemini disconnect simulation**: non-SSE Gemini streaming is buffered. `disconnect_after_ms` produces a shorter 200 OK array, not a transport failure. Use `?alt=sse` for real disconnect simulation.
- **Priority vs file order**: fixtures are sorted by descending priority. A `priority: 10` fixture at the bottom of the file wins over `priority: 0` at the top. File order is the tiebreaker within the same priority level.
- **Capture log under chaos**: `CapturedRequest.status_code` shows pre-chaos value (e.g., 200 even if chaos injects 500). Verify chaos failures via the HTTP response, not the capture log.
- **OpenAI first streaming chunk**: `content` field is absent, not `null`. Assert `content.is_none()` or check for absent/null — strict JSON equality fails.
- **Token count accuracy**: `bytes/4` heuristic. Never assert exact token counts; assert `> 0` only.
- **JSONPath with `default-features = false`**: `body_jsonpath` requires the `jsonpath` feature. Re-enable explicitly if you disabled defaults.
- **Jitter without latency**: `latency_jitter_ms` requires `latency_ms` to be set. Rejected at fixture load time, not runtime.
- **Duplicate response headers**: case-insensitive duplicate detection rejects fixtures at load time. Do not set `Content-Type` in custom headers — the handler sets it automatically.
- **Streaming-only fields on non-streaming**: `truncate_after_frames` and `disconnect_after_ms` are ignored with a warning on non-streaming requests.
- **CLI vs library capture defaults**: CLI defaults to 1000 captured requests with FIFO trimming. Library default is unbounded. Set `capture_capacity(0)` to disable capture entirely (`get_requests()` returns empty).
- **Provider exclusivity at the route level**: a `for_provider(Provider::OpenAI)` fixture on a `/v1/messages` (Anthropic) request returns 404 — there is no cross-provider fallback.
- **Response exclusivity**: `FixtureResponse.content` and `FixtureResponse.tool_calls` are mutually exclusive. Setting both is invalid; split into separate fixtures.
- **Tool-call arguments shape**: `ToolCall.arguments` is `serde_json::Value`, not a stringified JSON `String`. Use `serde_json::json!({"x": 1})`, NOT `r#"{"x":1}"#.to_string()`.
- **Auth coverage**: a bearer token does NOT block `GET /health`, `GET /code/{N}`, `GET /v1/models`, `POST /v1/embeddings`, `POST /v1/moderations`, or `GET /ui`. Auth only protects LLM routes.
- **Scenario state**: a retry-success fixture without `required_state` will be shadowed by the failure fixture forever, causing an infinite 429 loop. Set `required_state: Some("retried")` (or whatever the failure fixture's `set_state` is) on the success fixture.
- **Hidden legacy helper**: `llmposter::fixture::match_fixture` is `#[doc(hidden)]` and soft-deprecated since v0.4.6 — it does not honor v0.4.6+ match fields (`priority`, `catch_all`, `headers`, `system_prompt`, `temperature`, `metadata`, `tool_schema`, `body_jsonpath`). Drive the server through `ServerBuilder` for full match semantics.

## Migration

### v0.4.7 → v0.4.8

- **Tool-call argument serialization fallback** changed from `""` to `"{}"` when arguments fail to serialize. Tests that asserted on the empty-string fallback must update to `"{}"`.
- **Token total overflow protection**: token totals now use `saturating_add` across all format builders — extreme token counts no longer panic. No API change.
- **Docs example fix** (marked `[BREAKING]` in CHANGELOG): the scenario builder example in `docs/scenarios.md` previously used `respond_with_tool_calls(vec![])` (empty vec). If your code was modeled on it, provide at least one `ToolCall` or switch to `respond_with_content(...)`.

### v0.4.6 → v0.4.7

- **CLI capture capacity default changed** from unbounded to 1000 with FIFO trimming. Pass `--capture-capacity <N>` for a higher value or `0` to disable. Library users are unaffected (still unbounded by default).
- `ServerBuilder::ui(true)` and `--ui` CLI flag added (opt-in).

### v0.4.5 → v0.4.6

- **Fixture matching changed from file-order to priority-based sorting**. Fixtures with explicit `priority` values now match before lower-priority fixtures regardless of file position. Default priority is `0` — existing fixtures without priority are unaffected relative to each other.
- New match fields (`headers`, `system_prompt`, `temperature`, `metadata`, `tool_schema`, `body_jsonpath`) added — all additive and optional.
- `jsonpath` feature is on by default. Disable with `default-features = false` if not needed.
- New `Fixture` struct fields (`priority`, `catch_all`) and `FixtureMatch` fields are settable via direct struct construction or the dedicated builder methods (`with_priority`, `as_catch_all`, `match_header`, `match_system_prompt`, `match_temperature`, `match_temperature_range`, `match_metadata`, `match_tool_schema`, `match_body_jsonpath`).

### General Upgrade

All 0.4.x releases are additive at the Rust API level (the only `[BREAKING]` 0.4.8 entry is a docs example removal). YAML fixture format is backward-compatible — old fixtures work unchanged with new versions. Pin to `"0.4"` semver range.

## References

- [Repository](https://github.com/SkillDoAI/llmposter)
- [Homepage](https://skilldoai.com)
- [Documentation](https://docs.rs/llmposter)
- [Repository docs](https://github.com/SkillDoAI/llmposter/tree/HEAD/docs)
