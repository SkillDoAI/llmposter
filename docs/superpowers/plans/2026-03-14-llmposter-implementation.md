# llmposter Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust crate + CLI that mocks 4 LLM API endpoints (OpenAI, Anthropic, Gemini, Responses API) with YAML fixture-driven responses, SSE streaming, and failure simulation.

**Architecture:** Single crate, dual-target (lib.rs + main.rs). axum HTTP server with per-provider handlers. Fixtures loaded from YAML or built programmatically. Response formatters generate provider-specific JSON. SSE streamer chunks responses with configurable timing. Failure module simulates errors, corruption, truncation, and disconnects.

**Tech Stack:** Rust, tokio, axum, serde, serde_json, serde_yaml, clap, regex, uuid. Dev: reqwest.

**Spec:** `docs/superpowers/specs/2026-03-14-llmposter-design.md`

---

## File Structure

```
Cargo.toml
src/
  lib.rs              # Public API re-exports: ServerBuilder, Fixture, MockServer
  main.rs             # CLI binary — clap args → ServerBuilder → run
  fixture.rs          # Fixture, FixtureMatch, FixtureResponse, YAML deser, match_fixture()
  server.rs           # ServerBuilder, MockServer, axum router, shared AppState
  stream.rs           # write_sse_stream(), chunk_content()
  failure.rs          # FailureConfig, apply_failure(), error response builders
  handler/
    mod.rs            # extract_request_info() trait/helper, re-exports
    openai.rs         # handle_chat_completions()
    anthropic.rs      # handle_messages()
    gemini.rs         # handle_generate_content()
    responses.rs      # handle_responses()
  format/
    mod.rs            # Provider enum, shared types, re-exports
    openai.rs         # OpenAI Chat Completions request/response structs
    anthropic.rs      # Anthropic Messages request/response structs
    gemini.rs         # Gemini generateContent request/response structs
    responses.rs      # OpenAI Responses API request/response structs
tests/
  integration.rs          # Root test harness: mod declarations for submodules
  integration/
    openai_test.rs
    anthropic_test.rs
    gemini_test.rs
    responses_test.rs
    failure_test.rs
    fixture_yaml_test.rs
  fixtures/
    basic.yaml            # Sample fixtures for integration tests
```

**Note:** `tests/integration.rs` is required by Rust's test harness as the root file. It contains:
```rust
mod integration;
// where integration/ is the directory with submodules
```
Alternatively, each test file can be standalone in `tests/` — use whichever structure the executor prefers.

---

## Chunk 1: Project Scaffold + Fixture Types

### Task 1: Project Scaffold

**Files:**
- Create: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/main.rs`

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "llmposter"
version = "0.1.0"
edition = "2021"
description = "Mock LLM API server — fixture-driven, deterministic responses for testing"
license = "AGPL-3.0"

[[bin]]
name = "llmposter"
path = "src/main.rs"

[lib]
name = "llmposter"
path = "src/lib.rs"

[dependencies]
axum = "0.8"
clap = { version = "4", features = ["derive"] }
regex = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
tokio = { version = "1", features = ["full"] }
uuid = { version = "1", features = ["v4"] }

[dev-dependencies]
reqwest = { version = "0.12", features = ["json", "stream"] }
tokio-test = "0.4"
```

- [ ] **Step 2: Create minimal lib.rs**

```rust
pub mod fixture;

// Modules added as we build them
```

- [ ] **Step 3: Create minimal main.rs**

```rust
fn main() {
    println!("llmposter - mock LLM server");
}
```

- [ ] **Step 4: Verify project compiles**

Run: `cargo build`
Expected: Compiles with no errors.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/lib.rs src/main.rs
git commit -m "$(cat <<'EOF'
feat: scaffold project with Cargo.toml, lib.rs, main.rs
EOF
)"
```

---

### Task 2: Fixture Types + YAML Deserialization

**Files:**
- Create: `src/fixture.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write failing tests for fixture YAML parsing**

In `src/fixture.rs`, add a `#[cfg(test)] mod tests` block:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_parse_simple_text_fixture() {
        let yaml = r#"
fixtures:
  - match:
      user_message: "hello"
    response:
      content: "Hi there!"
"#;
        let file: FixtureFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(file.fixtures.len(), 1);
        let f = &file.fixtures[0];
        assert_eq!(
            f.match_rule.as_ref().unwrap().user_message,
            Some(StringMatch::Substring("hello".to_string()))
        );
        assert_eq!(f.response.as_ref().unwrap().content.as_deref(), Some("Hi there!"));
    }

    #[test]
    fn should_parse_regex_match() {
        let yaml = r#"
fixtures:
  - match:
      user_message:
        regex: "hello \\w+"
    response:
      content: "matched regex"
"#;
        let file: FixtureFile = serde_yaml::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        match &f.match_rule.as_ref().unwrap().user_message {
            Some(StringMatch::Regex(r)) => assert_eq!(r, "hello \\w+"),
            other => panic!("Expected Regex, got {:?}", other),
        }
    }

    #[test]
    fn should_parse_error_fixture() {
        let yaml = r#"
fixtures:
  - match:
      model: "fail-model"
    error:
      status: 429
      message: "Rate limit exceeded"
"#;
        let file: FixtureFile = serde_yaml::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        assert!(f.response.is_none());
        let err = f.error.as_ref().unwrap();
        assert_eq!(err.status, 429);
        assert_eq!(err.message, "Rate limit exceeded");
    }

    #[test]
    fn should_parse_failure_config() {
        let yaml = r#"
fixtures:
  - match:
      user_message: "slow"
    response:
      content: "delayed"
    failure:
      latency_ms: 5000
"#;
        let file: FixtureFile = serde_yaml::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        assert_eq!(f.failure.as_ref().unwrap().latency_ms, Some(5000));
    }

    #[test]
    fn should_parse_streaming_config() {
        let yaml = r#"
fixtures:
  - match:
      user_message: "stream"
    response:
      content: "streamed"
    streaming:
      latency: 50
      chunk_size: 10
"#;
        let file: FixtureFile = serde_yaml::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        let s = f.streaming.as_ref().unwrap();
        assert_eq!(s.latency, Some(50));
        assert_eq!(s.chunk_size, Some(10));
    }

    #[test]
    fn should_parse_tool_call_response() {
        let yaml = r#"
fixtures:
  - match:
      user_message: "weather"
    response:
      tool_calls:
        - name: get_weather
          arguments:
            location: "San Francisco"
"#;
        let file: FixtureFile = serde_yaml::from_str(yaml).unwrap();
        let tc = &file.fixtures[0].response.as_ref().unwrap().tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc.name, "get_weather");
        assert_eq!(tc.arguments["location"], "San Francisco");
    }

    #[test]
    fn should_parse_provider_specific_fixture() {
        let yaml = r#"
fixtures:
  - match:
      user_message: "test"
    provider: anthropic
    response:
      content: "response"
      stop_reason: end_turn
"#;
        let file: FixtureFile = serde_yaml::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        assert_eq!(f.provider.as_deref(), Some("anthropic"));
    }

    #[test]
    fn should_reject_invalid_yaml() {
        let yaml = "not: [valid: yaml: {{{";
        let result: Result<FixtureFile, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn should_parse_model_match() {
        let yaml = r#"
fixtures:
  - match:
      model: "gpt-4"
      user_message: "hello"
    response:
      content: "hi"
"#;
        let file: FixtureFile = serde_yaml::from_str(yaml).unwrap();
        let m = file.fixtures[0].match_rule.as_ref().unwrap();
        assert_eq!(m.model, Some(StringMatch::Substring("gpt-4".to_string())));
    }

    #[test]
    fn should_parse_catch_all_fixture() {
        let yaml = r#"
fixtures:
  - response:
      content: "default response"
"#;
        let file: FixtureFile = serde_yaml::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        assert!(f.match_rule.is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib fixture`
Expected: Compilation errors — types don't exist yet.

- [ ] **Step 3: Implement fixture types**

In `src/fixture.rs`:

```rust
use serde::Deserialize;
use std::collections::HashMap;

/// How to match a string field — substring (default) or regex.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum StringMatch {
    Substring(String),
    Regex(RegexMatch),
}

/// Wrapper for `{ regex: "pattern" }` syntax in YAML.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RegexMatch {
    pub regex: String,
}

impl StringMatch {
    /// Shorthand for creating a regex match (used in tests and programmatic builder).
    pub fn regex(pattern: &str) -> Self {
        StringMatch::Regex(RegexMatch { regex: pattern.to_string() })
    }
}

/// Match criteria for a fixture.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FixtureMatch {
    pub user_message: Option<StringMatch>,
    pub model: Option<StringMatch>,
}

/// A tool call in a fixture response.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// The response to return when a fixture matches.
#[derive(Debug, Clone, Deserialize)]
pub struct FixtureResponse {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub stop_reason: Option<String>,
    pub finish_reason: Option<String>,
}

/// Error simulation — returns an HTTP error status.
#[derive(Debug, Clone, Deserialize)]
pub struct FixtureError {
    pub status: u16,
    pub message: String,
}

/// Failure simulation — network/streaming problems.
#[derive(Debug, Clone, Deserialize)]
pub struct FailureConfig {
    pub latency_ms: Option<u64>,
    pub corrupt_body: Option<bool>,
    pub truncate_after_chunks: Option<u32>,
    pub disconnect_after_ms: Option<u64>,
}

/// Streaming behavior config.
#[derive(Debug, Clone, Deserialize)]
pub struct StreamingConfig {
    pub latency: Option<u64>,
    pub chunk_size: Option<usize>,
}

/// A single fixture entry.
#[derive(Debug, Clone, Deserialize)]
pub struct Fixture {
    #[serde(rename = "match")]
    pub match_rule: Option<FixtureMatch>,
    pub provider: Option<String>,
    pub response: Option<FixtureResponse>,
    pub error: Option<FixtureError>,
    pub failure: Option<FailureConfig>,
    pub streaming: Option<StreamingConfig>,
}

/// Top-level YAML file structure.
#[derive(Debug, Clone, Deserialize)]
pub struct FixtureFile {
    pub fixtures: Vec<Fixture>,
}

/// Programmatic builder for Fixture (ergonomic API for Rust tests).
impl Fixture {
    pub fn new() -> Self {
        Self {
            match_rule: None,
            provider: None,
            response: None,
            error: None,
            failure: None,
            streaming: None,
        }
    }

    pub fn match_user_message(mut self, pattern: &str) -> Self {
        let m = self.match_rule.get_or_insert_with(FixtureMatch::default);
        m.user_message = Some(StringMatch::Substring(pattern.to_string()));
        self
    }

    pub fn match_model(mut self, pattern: &str) -> Self {
        let m = self.match_rule.get_or_insert_with(FixtureMatch::default);
        m.model = Some(StringMatch::Substring(pattern.to_string()));
        self
    }

    pub fn respond_with_content(mut self, content: &str) -> Self {
        self.response = Some(FixtureResponse {
            content: Some(content.to_string()),
            tool_calls: None,
            stop_reason: None,
            finish_reason: None,
        });
        self
    }

    pub fn with_error(mut self, status: u16, message: &str) -> Self {
        self.error = Some(FixtureError {
            status,
            message: message.to_string(),
        });
        self
    }

    pub fn with_failure(mut self, failure: FailureConfig) -> Self {
        self.failure = Some(failure);
        self
    }

    pub fn with_streaming(mut self, latency: Option<u64>, chunk_size: Option<usize>) -> Self {
        self.streaming = Some(StreamingConfig { latency, chunk_size });
        self
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}
```

Add builder API tests:

```rust
    #[test]
    fn should_build_fixture_programmatically() {
        let f = Fixture::new()
            .match_user_message("hello")
            .respond_with_content("Hi there!");
        assert!(f.validate().is_ok());
        assert_eq!(f.response.as_ref().unwrap().content.as_deref(), Some("Hi there!"));
    }

    #[test]
    fn should_build_error_fixture_programmatically() {
        let f = Fixture::new()
            .match_model("fail-model")
            .with_error(429, "Rate limited");
        assert!(f.validate().is_ok());
        assert_eq!(f.error.as_ref().unwrap().status, 429);
    }
```

- [ ] **Step 4: Update lib.rs to include fixture module**

```rust
pub mod fixture;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib fixture`
Expected: All 10 tests pass.

- [ ] **Step 6: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings.

- [ ] **Step 7: Commit**

```bash
git add src/fixture.rs src/lib.rs
git commit -m "$(cat <<'EOF'
feat: add fixture types with YAML deserialization and tests
EOF
)"
```

---

### Task 3: Fixture Validation

**Files:**
- Modify: `src/fixture.rs`

- [ ] **Step 1: Write failing tests for fixture validation**

Add to the test module in `src/fixture.rs`:

```rust
    #[test]
    fn should_reject_fixture_with_both_error_and_response() {
        let f = Fixture {
            match_rule: None,
            provider: None,
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            error: Some(FixtureError {
                status: 500,
                message: "fail".to_string(),
            }),
            failure: None,
            streaming: None,
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mutually exclusive"));
    }

    #[test]
    fn should_reject_fixture_with_failure_but_no_response() {
        let f = Fixture {
            match_rule: None,
            provider: None,
            response: None,
            error: None,
            failure: Some(FailureConfig {
                latency_ms: Some(1000),
                corrupt_body: None,
                truncate_after_chunks: None,
                disconnect_after_ms: None,
            }),
            streaming: None,
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires response"));
    }

    #[test]
    fn should_reject_fixture_with_error_and_failure() {
        let f = Fixture {
            match_rule: None,
            provider: None,
            response: None,
            error: Some(FixtureError { status: 429, message: "rate limit".to_string() }),
            failure: Some(FailureConfig {
                latency_ms: Some(1000),
                corrupt_body: None,
                truncate_after_chunks: None,
                disconnect_after_ms: None,
            }),
            streaming: None,
        };
        let result = f.validate();
        assert!(result.is_err());
    }

    #[test]
    fn should_reject_fixture_with_no_response_and_no_error() {
        let f = Fixture {
            match_rule: Some(FixtureMatch::default()),
            provider: None,
            response: None,
            error: None,
            failure: None,
            streaming: None,
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must have either"));
    }

    #[test]
    fn should_accept_valid_error_fixture() {
        let f = Fixture {
            match_rule: None,
            provider: None,
            response: None,
            error: Some(FixtureError { status: 429, message: "rate limit".to_string() }),
            failure: None,
            streaming: None,
        };
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_accept_valid_response_fixture() {
        let f = Fixture {
            match_rule: None,
            provider: None,
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            error: None,
            failure: None,
            streaming: None,
        };
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_reject_invalid_regex() {
        let f = Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::regex("[invalid")),
                model: None,
            }),
            provider: None,
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            error: None,
            failure: None,
            streaming: None,
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("regex"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib fixture::tests::should_reject`
Expected: Compilation error — `validate()` doesn't exist.

- [ ] **Step 3: Implement validate()**

Add to `Fixture` impl in `src/fixture.rs`:

```rust
impl Fixture {
    /// Validates fixture invariants. Returns Err with a descriptive message on failure.
    pub fn validate(&self) -> Result<(), String> {
        // Must have response or error
        if self.response.is_none() && self.error.is_none() {
            return Err("Fixture must have either 'response' or 'error'".to_string());
        }

        // error and response are mutually exclusive
        if self.response.is_some() && self.error.is_some() {
            return Err("'error' and 'response' are mutually exclusive".to_string());
        }

        // error and failure are mutually exclusive
        if self.error.is_some() && self.failure.is_some() {
            return Err("'error' and 'failure' are mutually exclusive".to_string());
        }

        // failure requires response
        if self.failure.is_some() && self.response.is_none() {
            return Err("'failure' requires response to also be present".to_string());
        }

        // Validate regex patterns compile
        if let Some(ref m) = self.match_rule {
            if let Some(StringMatch::Regex(ref r)) = m.user_message {
                regex::Regex::new(&r.regex)
                    .map_err(|e| format!("Invalid user_message regex '{}': {}", r.regex, e))?;
            }
            if let Some(StringMatch::Regex(ref r)) = m.model {
                regex::Regex::new(&r.regex)
                    .map_err(|e| format!("Invalid model regex '{}': {}", r.regex, e))?;
            }
        }

        Ok(())
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib fixture`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/fixture.rs
git commit -m "$(cat <<'EOF'
feat: add fixture validation with mutual exclusivity checks
EOF
)"
```

---

### Task 4: Fixture Matching Logic

**Files:**
- Modify: `src/fixture.rs`

- [ ] **Step 1: Write failing tests for fixture matching**

Add to test module in `src/fixture.rs`:

```rust
    #[test]
    fn should_match_substring_user_message() {
        let fixtures = vec![Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::Substring("hello".to_string())),
                model: None,
            }),
            provider: None,
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            error: None,
            failure: None,
            streaming: None,
        }];
        let result = match_fixture(&fixtures, "say hello world", None, None);
        assert!(result.is_some());
    }

    #[test]
    fn should_not_match_wrong_substring() {
        let fixtures = vec![Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::Substring("goodbye".to_string())),
                model: None,
            }),
            provider: None,
            response: Some(FixtureResponse {
                content: Some("bye".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            error: None,
            failure: None,
            streaming: None,
        }];
        let result = match_fixture(&fixtures, "say hello world", None, None);
        assert!(result.is_none());
    }

    #[test]
    fn should_match_regex_user_message() {
        let fixtures = vec![Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::regex("hello \\w+")),
                model: None,
            }),
            provider: None,
            response: Some(FixtureResponse {
                content: Some("matched".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            error: None,
            failure: None,
            streaming: None,
        }];
        let result = match_fixture(&fixtures, "hello world", None, None);
        assert!(result.is_some());
    }

    #[test]
    fn should_match_model() {
        let fixtures = vec![Fixture {
            match_rule: Some(FixtureMatch {
                user_message: None,
                model: Some(StringMatch::Substring("gpt-4".to_string())),
            }),
            provider: None,
            response: Some(FixtureResponse {
                content: Some("gpt4 response".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            error: None,
            failure: None,
            streaming: None,
        }];
        let result = match_fixture(&fixtures, "anything", Some("gpt-4-turbo"), None);
        assert!(result.is_some());
    }

    #[test]
    fn should_match_first_fixture_wins() {
        let fixtures = vec![
            Fixture {
                match_rule: Some(FixtureMatch {
                    user_message: Some(StringMatch::Substring("hello".to_string())),
                    model: None,
                }),
                provider: None,
                response: Some(FixtureResponse {
                    content: Some("first".to_string()),
                    tool_calls: None,
                    stop_reason: None,
                    finish_reason: None,
                }),
                error: None,
                failure: None,
                streaming: None,
            },
            Fixture {
                match_rule: Some(FixtureMatch {
                    user_message: Some(StringMatch::Substring("hello".to_string())),
                    model: None,
                }),
                provider: None,
                response: Some(FixtureResponse {
                    content: Some("second".to_string()),
                    tool_calls: None,
                    stop_reason: None,
                    finish_reason: None,
                }),
                error: None,
                failure: None,
                streaming: None,
            },
        ];
        let result = match_fixture(&fixtures, "hello", None, None);
        assert_eq!(result.unwrap().response.as_ref().unwrap().content.as_deref(), Some("first"));
    }

    #[test]
    fn should_match_catch_all() {
        let fixtures = vec![Fixture {
            match_rule: None,
            provider: None,
            response: Some(FixtureResponse {
                content: Some("default".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            error: None,
            failure: None,
            streaming: None,
        }];
        let result = match_fixture(&fixtures, "anything at all", None, None);
        assert!(result.is_some());
    }

    #[test]
    fn should_filter_by_provider() {
        let fixtures = vec![Fixture {
            match_rule: None,
            provider: Some("anthropic".to_string()),
            response: Some(FixtureResponse {
                content: Some("anthropic only".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            error: None,
            failure: None,
            streaming: None,
        }];
        // Should match when provider matches
        let result = match_fixture(&fixtures, "hello", None, Some("anthropic"));
        assert!(result.is_some());
        // Should not match when provider differs
        let result = match_fixture(&fixtures, "hello", None, Some("openai"));
        assert!(result.is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib fixture::tests::should_match`
Expected: Compilation error — `match_fixture()` doesn't exist.

- [ ] **Step 3: Implement match_fixture()**

Add to `src/fixture.rs`:

```rust
/// Matches a request against fixtures. Returns the first matching fixture.
///
/// - `user_message`: the last user message content from the request
/// - `model`: the model name from the request (if present)
/// - `provider`: which provider endpoint was hit (e.g., "openai", "anthropic")
pub fn match_fixture<'a>(
    fixtures: &'a [Fixture],
    user_message: &str,
    model: Option<&str>,
    provider: Option<&str>,
) -> Option<&'a Fixture> {
    fixtures.iter().find(|f| fixture_matches(f, user_message, model, provider))
}

fn fixture_matches(
    fixture: &Fixture,
    user_message: &str,
    model: Option<&str>,
    provider: Option<&str>,
) -> bool {
    // Check provider filter first
    if let Some(ref fp) = fixture.provider {
        match provider {
            Some(p) if p == fp => {}
            _ => return false,
        }
    }

    // Check match rules
    if let Some(ref m) = fixture.match_rule {
        if let Some(ref um) = m.user_message {
            if !string_matches(um, user_message) {
                return false;
            }
        }
        if let Some(ref mm) = m.model {
            match model {
                Some(m) => {
                    if !string_matches(mm, m) {
                        return false;
                    }
                }
                None => return false,
            }
        }
    }

    true
}

fn string_matches(pattern: &StringMatch, haystack: &str) -> bool {
    match pattern {
        StringMatch::Substring(s) => haystack.contains(s.as_str()),
        StringMatch::Regex(r) => {
            regex::Regex::new(&r.regex)
                .map(|re| re.is_match(haystack))
                .unwrap_or(false)
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib fixture`
Expected: All tests pass.

- [ ] **Step 5: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings.

- [ ] **Step 6: Commit**

```bash
git add src/fixture.rs
git commit -m "$(cat <<'EOF'
feat: add fixture matching logic with substring, regex, and provider filtering
EOF
)"
```

---

### Task 5: YAML File Loading

**Files:**
- Modify: `src/fixture.rs`

- [ ] **Step 1: Write failing tests for file loading**

Add to test module in `src/fixture.rs`:

```rust
    #[test]
    fn should_load_yaml_file() {
        let dir = std::env::temp_dir().join("llmposter_test_load");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.yaml");
        std::fs::write(&file, r#"
fixtures:
  - match:
      user_message: "test"
    response:
      content: "loaded from file"
"#).unwrap();
        let fixtures = load_yaml_file(&file).unwrap();
        assert_eq!(fixtures.len(), 1);
        assert_eq!(fixtures[0].response.as_ref().unwrap().content.as_deref(), Some("loaded from file"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn should_load_yaml_dir() {
        let dir = std::env::temp_dir().join("llmposter_test_dir");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.yaml"), r#"
fixtures:
  - match:
      user_message: "a"
    response:
      content: "response a"
"#).unwrap();
        std::fs::write(dir.join("b.yml"), r#"
fixtures:
  - match:
      user_message: "b"
    response:
      content: "response b"
"#).unwrap();
        std::fs::write(dir.join("not_yaml.txt"), "ignored").unwrap();
        let fixtures = load_yaml_dir(&dir).unwrap();
        assert_eq!(fixtures.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn should_return_error_for_invalid_yaml_file() {
        let dir = std::env::temp_dir().join("llmposter_test_invalid");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bad.yaml");
        std::fs::write(&file, "not: [valid: {{{").unwrap();
        let result = load_yaml_file(&file);
        assert!(result.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn should_return_error_for_missing_file() {
        let result = load_yaml_file(std::path::Path::new("/nonexistent/file.yaml"));
        assert!(result.is_err());
    }

    #[test]
    fn should_validate_fixtures_on_load() {
        let dir = std::env::temp_dir().join("llmposter_test_validate");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("invalid_fixture.yaml");
        std::fs::write(&file, r#"
fixtures:
  - match:
      user_message: "test"
    response:
      content: "hi"
    error:
      status: 500
      message: "also error"
"#).unwrap();
        let result = load_yaml_file(&file);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mutually exclusive"));
        std::fs::remove_dir_all(&dir).ok();
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib fixture::tests::should_load`
Expected: Compilation error — functions don't exist.

- [ ] **Step 3: Implement load_yaml_file() and load_yaml_dir()**

Add to `src/fixture.rs`:

```rust
use std::path::Path;

/// Load and validate fixtures from a single YAML file.
pub fn load_yaml_file(path: &Path) -> Result<Vec<Fixture>, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let file: FixtureFile = serde_yaml::from_str(&content)
        .map_err(|e| format!("Invalid YAML in {}: {}", path.display(), e))?;

    for (i, fixture) in file.fixtures.iter().enumerate() {
        fixture.validate().map_err(|e| {
            format!("Fixture #{} in {}: {}", i + 1, path.display(), e)
        })?;
    }

    Ok(file.fixtures)
}

/// Load and validate fixtures from all .yaml/.yml files in a directory.
/// Files are loaded in alphabetical order.
pub fn load_yaml_dir(dir: &Path) -> Result<Vec<Fixture>, Box<dyn std::error::Error>> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read directory {}: {}", dir.display(), e))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".yaml") || name.ends_with(".yml")
        })
        .collect();

    entries.sort_by_key(|e| e.file_name());

    let mut all_fixtures = Vec::new();
    for entry in entries {
        let fixtures = load_yaml_file(&entry.path())?;
        all_fixtures.extend(fixtures);
    }

    Ok(all_fixtures)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib fixture`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/fixture.rs
git commit -m "$(cat <<'EOF'
feat: add YAML file and directory loading with validation
EOF
)"
```

---

## Chunk 2: Provider Response Formats

### Task 6: Format Module + Shared Types

**Files:**
- Create: `src/format/mod.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create format module with shared types**

```rust
pub mod openai;
pub mod anthropic;
pub mod gemini;
pub mod responses;

use std::sync::atomic::{AtomicU64, Ordering};

/// Global ID counter for deterministic response IDs.
pub struct IdGenerator {
    counter: AtomicU64,
}

impl IdGenerator {
    pub fn new() -> Self {
        Self { counter: AtomicU64::new(1) }
    }

    pub fn next_openai(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("chatcmpl-llmposter-{}", n)
    }

    pub fn next_anthropic(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("msg-llmposter-{}", n)
    }

    pub fn next_responses(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("resp-llmposter-{}", n)
    }
}

impl Default for IdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Estimate token count from text (rough: chars / 4).
pub fn estimate_tokens(text: &str) -> u64 {
    (text.len() as u64 + 3) / 4 // round up
}

/// Provider identifier — which endpoint was hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAI,
    Anthropic,
    Gemini,
    Responses,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::OpenAI => "openai",
            Provider::Anthropic => "anthropic",
            Provider::Gemini => "gemini",
            Provider::Responses => "responses",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_generate_sequential_openai_ids() {
        let gen = IdGenerator::new();
        assert_eq!(gen.next_openai(), "chatcmpl-llmposter-1");
        assert_eq!(gen.next_openai(), "chatcmpl-llmposter-2");
    }

    #[test]
    fn should_generate_sequential_anthropic_ids() {
        let gen = IdGenerator::new();
        assert_eq!(gen.next_anthropic(), "msg-llmposter-1");
        assert_eq!(gen.next_anthropic(), "msg-llmposter-2");
    }

    #[test]
    fn should_generate_sequential_responses_ids() {
        let gen = IdGenerator::new();
        assert_eq!(gen.next_responses(), "resp-llmposter-1");
        assert_eq!(gen.next_responses(), "resp-llmposter-2");
    }

    #[test]
    fn should_share_counter_across_providers() {
        let gen = IdGenerator::new();
        assert_eq!(gen.next_openai(), "chatcmpl-llmposter-1");
        assert_eq!(gen.next_anthropic(), "msg-llmposter-2");
        assert_eq!(gen.next_responses(), "resp-llmposter-3");
    }

    #[test]
    fn should_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hi"), 1);
        assert_eq!(estimate_tokens("hello world"), 3); // 11 chars / 4 = 2.75 → 3
    }
}
```

- [ ] **Step 2: Update lib.rs**

```rust
pub mod fixture;
pub mod format;
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib format`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/format/mod.rs src/lib.rs
git commit -m "$(cat <<'EOF'
feat: add format module with ID generator, token estimation, and Provider enum
EOF
)"
```

---

### Task 7: OpenAI Chat Completions Format

**Files:**
- Create: `src/format/openai.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{IdGenerator, estimate_tokens};

    #[test]
    fn should_build_chat_completion_response() {
        let gen = IdGenerator::new();
        let resp = build_response(&gen, "gpt-4", "Hello!", "What is Rust?");
        assert_eq!(resp.id, "chatcmpl-llmposter-1");
        assert_eq!(resp.object, "chat.completion");
        assert_eq!(resp.model, "gpt-4");
        assert_eq!(resp.choices[0].message.content, "Hello!");
        assert_eq!(resp.choices[0].message.role, "assistant");
        assert_eq!(resp.choices[0].finish_reason, "stop");
        assert_eq!(resp.choices[0].index, 0);
        assert!(resp.usage.prompt_tokens > 0);
        assert!(resp.usage.completion_tokens > 0);
    }

    #[test]
    fn should_serialize_to_valid_json() {
        let gen = IdGenerator::new();
        let resp = build_response(&gen, "gpt-4", "test", "prompt");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("chat.completion"));
        assert!(json.contains("chatcmpl-llmposter-1"));
        // Should be deserializable back
        let _: ChatCompletionResponse = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn should_build_tool_call_response() {
        let gen = IdGenerator::new();
        let args = serde_json::json!({"location": "SF"});
        let tool_calls = vec![("get_weather", args)];
        let resp = build_tool_call_response(&gen, "gpt-4", &tool_calls, "prompt");
        assert_eq!(resp.choices[0].message.tool_calls.as_ref().unwrap().len(), 1);
        let tc = &resp.choices[0].message.tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc.function.name, "get_weather");
        // OpenAI sends arguments as a JSON string
        assert!(tc.function.arguments.contains("SF"));
        assert_eq!(resp.choices[0].finish_reason, "tool_calls");
    }

    #[test]
    fn should_extract_user_message_from_request() {
        let json = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "You are helpful"},
                {"role": "user", "content": "Hello!"},
            ]
        });
        let (model, user_msg) = extract_request_info(&json).unwrap();
        assert_eq!(model, "gpt-4");
        assert_eq!(user_msg, "Hello!");
    }

    #[test]
    fn should_extract_last_user_message() {
        let json = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "user", "content": "first"},
                {"role": "assistant", "content": "response"},
                {"role": "user", "content": "second"},
            ]
        });
        let (_, user_msg) = extract_request_info(&json).unwrap();
        assert_eq!(user_msg, "second");
    }

    #[test]
    fn should_return_error_for_missing_messages() {
        let json = serde_json::json!({"model": "gpt-4"});
        let result = extract_request_info(&json);
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib format::openai`
Expected: Compilation errors.

- [ ] **Step 3: Implement OpenAI format structs and builders**

In `src/format/openai.rs`:

```rust
use serde::{Deserialize, Serialize};
use crate::format::{IdGenerator, estimate_tokens};

// --- Response types ---

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: Message,
    pub finish_reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallOutput>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCallOutput {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String, // JSON string, not object
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

// --- Streaming types ---

#[derive(Debug, Serialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub model: String,
    pub choices: Vec<ChunkChoice>,
}

#[derive(Debug, Serialize)]
pub struct ChunkChoice {
    pub index: u32,
    pub delta: Delta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Delta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

// --- Builders ---

pub fn build_response(id_gen: &IdGenerator, model: &str, content: &str, prompt: &str) -> ChatCompletionResponse {
    ChatCompletionResponse {
        id: id_gen.next_openai(),
        object: "chat.completion".to_string(),
        model: model.to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: "assistant".to_string(),
                content: content.to_string(),
                tool_calls: None,
            },
            finish_reason: "stop".to_string(),
        }],
        usage: Usage {
            prompt_tokens: estimate_tokens(prompt),
            completion_tokens: estimate_tokens(content),
            total_tokens: estimate_tokens(prompt) + estimate_tokens(content),
        },
    }
}

pub fn build_tool_call_response(
    id_gen: &IdGenerator,
    model: &str,
    tool_calls: &[(&str, serde_json::Value)],
    prompt: &str,
) -> ChatCompletionResponse {
    let tc_outputs: Vec<ToolCallOutput> = tool_calls
        .iter()
        .enumerate()
        .map(|(i, (name, args))| ToolCallOutput {
            id: format!("call_llmposter_{}", i + 1),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: name.to_string(),
                arguments: serde_json::to_string(args).unwrap_or_default(),
            },
        })
        .collect();

    let args_str = tool_calls
        .iter()
        .map(|(_, a)| serde_json::to_string(a).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("");

    ChatCompletionResponse {
        id: id_gen.next_openai(),
        object: "chat.completion".to_string(),
        model: model.to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: "assistant".to_string(),
                content: String::new(),
                tool_calls: Some(tc_outputs),
            },
            finish_reason: "tool_calls".to_string(),
        }],
        usage: Usage {
            prompt_tokens: estimate_tokens(prompt),
            completion_tokens: estimate_tokens(&args_str),
            total_tokens: estimate_tokens(prompt) + estimate_tokens(&args_str),
        },
    }
}

// --- Request extraction ---

/// Extract model and last user message from an OpenAI chat completions request body.
pub fn extract_request_info(body: &serde_json::Value) -> Result<(String, String), String> {
    let model = body["model"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let messages = body["messages"]
        .as_array()
        .ok_or("Missing 'messages' array in request")?;

    let user_msg = messages
        .iter()
        .rev()
        .find(|m| m["role"].as_str() == Some("user"))
        .and_then(|m| m["content"].as_str())
        .ok_or("No user message found in request")?
        .to_string();

    Ok((model, user_msg))
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib format::openai`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/format/openai.rs
git commit -m "$(cat <<'EOF'
feat: add OpenAI Chat Completions format with request/response types
EOF
)"
```

---

### Task 8: Anthropic Messages Format

**Files:**
- Create: `src/format/anthropic.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::IdGenerator;

    #[test]
    fn should_build_messages_response() {
        let gen = IdGenerator::new();
        let resp = build_response(&gen, "claude-sonnet-4-6", "Hello!", "prompt");
        assert_eq!(resp.id, "msg-llmposter-1");
        assert_eq!(resp.response_type, "message");
        assert_eq!(resp.role, "assistant");
        assert_eq!(resp.model, "claude-sonnet-4-6");
        assert_eq!(resp.content[0].text, "Hello!");
        assert_eq!(resp.content[0].content_type, "text");
        assert_eq!(resp.stop_reason, "end_turn");
        assert!(resp.usage.input_tokens > 0);
    }

    #[test]
    fn should_serialize_to_valid_json() {
        let gen = IdGenerator::new();
        let resp = build_response(&gen, "claude-sonnet-4-6", "test", "prompt");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"type\":\"message\""));
        assert!(json.contains("msg-llmposter-1"));
        let _: MessagesResponse = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn should_build_tool_use_response() {
        let gen = IdGenerator::new();
        let args = serde_json::json!({"location": "SF"});
        let tool_calls = vec![("get_weather", args)];
        let resp = build_tool_use_response(&gen, "claude-sonnet-4-6", &tool_calls, "prompt");
        assert_eq!(resp.stop_reason, "tool_use");
        assert_eq!(resp.content.len(), 1);
        // Should have tool_use content block
        let json = serde_json::to_string(&resp.content[0]).unwrap();
        assert!(json.contains("tool_use"));
    }

    #[test]
    fn should_extract_request_info() {
        let json = serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "Hello Claude!"}
            ]
        });
        let (model, msg) = extract_request_info(&json).unwrap();
        assert_eq!(model, "claude-sonnet-4-6");
        assert_eq!(msg, "Hello Claude!");
    }

    #[test]
    fn should_extract_content_from_array_format() {
        let json = serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "array content"}]}
            ]
        });
        let (_, msg) = extract_request_info(&json).unwrap();
        assert_eq!(msg, "array content");
    }
}
```

- [ ] **Step 2: Implement Anthropic format**

In `src/format/anthropic.rs`, implement `MessagesResponse`, `ContentBlock` (with `text` and `tool_use` variants), `AnthropicUsage`, `build_response()`, `build_tool_use_response()`, and `extract_request_info()`. Follow the same pattern as OpenAI but with Anthropic's JSON shape:
- `content` is an array of `ContentBlock` objects
- `stop_reason` instead of `finish_reason`
- `input_tokens`/`output_tokens` instead of `prompt_tokens`/`completion_tokens`
- Tool calls use `tool_use` content blocks with `input` (object, not string) instead of `function.arguments`

- [ ] **Step 3: Run tests**

Run: `cargo test --lib format::anthropic`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/format/anthropic.rs
git commit -m "$(cat <<'EOF'
feat: add Anthropic Messages API format with request/response types
EOF
)"
```

---

### Task 9: Gemini generateContent Format

**Files:**
- Create: `src/format/gemini.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_build_generate_content_response() {
        let resp = build_response("Hello!", "prompt");
        assert_eq!(resp.candidates[0].content.parts[0].text, "Hello!");
        assert_eq!(resp.candidates[0].content.role, "model");
        assert_eq!(resp.candidates[0].finish_reason, "STOP");
        assert!(resp.usage_metadata.prompt_token_count > 0);
    }

    #[test]
    fn should_serialize_to_valid_json() {
        let resp = build_response("test", "prompt");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"finishReason\":\"STOP\""));
        assert!(json.contains("\"role\":\"model\""));
        let _: GenerateContentResponse = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn should_extract_request_info() {
        let json = serde_json::json!({
            "contents": [
                {"role": "user", "parts": [{"text": "Hello Gemini!"}]}
            ]
        });
        let (model, msg) = extract_request_info(&json, Some("gemini-pro")).unwrap();
        assert_eq!(model, "gemini-pro");
        assert_eq!(msg, "Hello Gemini!");
    }
}
```

- [ ] **Step 2: Implement Gemini format**

In `src/format/gemini.rs`, implement `GenerateContentResponse`, `Candidate`, `Content`, `Part`, `UsageMetadata`, `build_response()`, and `extract_request_info()`. Note:
- Gemini doesn't return response IDs
- Model name comes from URL path, not request body — `extract_request_info` takes model as a parameter
- Field names are camelCase: `finishReason`, `usageMetadata`, `promptTokenCount`, `candidatesTokenCount`
- Use `#[serde(rename = "camelCase")]` or `#[serde(rename_all = "camelCase")]`

- [ ] **Step 3: Run tests**

Run: `cargo test --lib format::gemini`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/format/gemini.rs
git commit -m "$(cat <<'EOF'
feat: add Gemini generateContent format with request/response types
EOF
)"
```

---

### Task 10: OpenAI Responses API Format

**Files:**
- Create: `src/format/responses.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::IdGenerator;

    #[test]
    fn should_build_responses_api_response() {
        let gen = IdGenerator::new();
        let resp = build_response(&gen, "gpt-4", "Hello!", "prompt");
        assert_eq!(resp.id, "resp-llmposter-1");
        assert_eq!(resp.object, "response");
        assert_eq!(resp.output[0].content[0].text, "Hello!");
        assert_eq!(resp.output[0].output_type, "message");
    }

    #[test]
    fn should_serialize_to_valid_json() {
        let gen = IdGenerator::new();
        let resp = build_response(&gen, "gpt-4", "test", "prompt");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"object\":\"response\""));
        assert!(json.contains("output_text"));
        let _: ResponsesApiResponse = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn should_extract_request_info() {
        let json = serde_json::json!({
            "model": "gpt-4",
            "input": [
                {"role": "user", "content": "Hello!"}
            ]
        });
        let (model, msg) = extract_request_info(&json).unwrap();
        assert_eq!(model, "gpt-4");
        assert_eq!(msg, "Hello!");
    }

    #[test]
    fn should_handle_string_input() {
        let json = serde_json::json!({
            "model": "gpt-4",
            "input": "Just a string prompt"
        });
        let (_, msg) = extract_request_info(&json).unwrap();
        assert_eq!(msg, "Just a string prompt");
    }
}
```

- [ ] **Step 2: Implement Responses API format**

In `src/format/responses.rs`, implement `ResponsesApiResponse`, `OutputItem`, `OutputContent`, `ResponsesUsage`, `build_response()`, and `extract_request_info()`. Note:
- `output` contains `OutputItem` objects with `type: "message"`
- `content` array has objects with `type: "output_text"` and `text` field
- `input` can be a string or an array of messages
- Usage has `input_tokens`, `output_tokens`, `total_tokens`

- [ ] **Step 3: Run tests**

Run: `cargo test --lib format::responses`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/format/responses.rs
git commit -m "$(cat <<'EOF'
feat: add OpenAI Responses API format with request/response types
EOF
)"
```

---

## Chunk 3: Server + Non-Streaming Handlers

### Task 11: Server Builder + MockServer

**Files:**
- Create: `src/server.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Fixture, FixtureMatch, FixtureResponse, StringMatch};

    #[tokio::test]
    async fn should_build_and_start_server() {
        let server = ServerBuilder::new()
            .fixture(Fixture {
                match_rule: None,
                provider: None,
                response: Some(FixtureResponse {
                    content: Some("test".to_string()),
                    tool_calls: None,
                    stop_reason: None,
                    finish_reason: None,
                }),
                error: None,
                failure: None,
                streaming: None,
            })
            .build()
            .await;
        assert!(server.port() > 0);
        assert!(server.url().starts_with("http://127.0.0.1:"));
    }

    #[tokio::test]
    async fn should_return_404_for_unknown_routes() {
        let server = ServerBuilder::new()
            .fixture(Fixture {
                match_rule: None,
                provider: None,
                response: Some(FixtureResponse {
                    content: Some("test".to_string()),
                    tool_calls: None,
                    stop_reason: None,
                    finish_reason: None,
                }),
                error: None,
                failure: None,
                streaming: None,
            })
            .build()
            .await;
        let resp = reqwest::get(format!("{}/unknown", server.url())).await.unwrap();
        assert_eq!(resp.status(), 404);
    }
}
```

- [ ] **Step 2: Implement ServerBuilder and MockServer**

In `src/server.rs`:

```rust
use std::sync::Arc;
use axum::{Router, routing::post};
use tokio::net::TcpListener;
use crate::fixture::Fixture;
use crate::format::IdGenerator;

pub struct AppState {
    pub fixtures: Vec<Fixture>,
    pub id_gen: IdGenerator,
}

pub struct ServerBuilder {
    fixtures: Vec<Fixture>,
    bind_addr: String,
}

impl ServerBuilder {
    pub fn new() -> Self {
        Self {
            fixtures: Vec::new(),
            bind_addr: "127.0.0.1:0".to_string(),
        }
    }

    pub fn fixture(mut self, f: Fixture) -> Self {
        self.fixtures.push(f);
        self
    }

    pub fn fixtures(mut self, fixtures: Vec<Fixture>) -> Self {
        self.fixtures.extend(fixtures);
        self
    }

    pub fn bind(mut self, addr: &str) -> Self {
        self.bind_addr = addr.to_string();
        self
    }

    pub fn load_yaml(mut self, path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let fixtures = crate::fixture::load_yaml_file(path)?;
        self.fixtures.extend(fixtures);
        Ok(self)
    }

    pub fn load_yaml_dir(mut self, dir: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let fixtures = crate::fixture::load_yaml_dir(dir)?;
        self.fixtures.extend(fixtures);
        Ok(self)
    }

    pub async fn build(self) -> MockServer {
        let state = Arc::new(AppState {
            fixtures: self.fixtures,
            id_gen: IdGenerator::new(),
        });

        let app = Router::new()
            .route("/v1/chat/completions", post(crate::handler::openai::handle))
            .route("/v1/messages", post(crate::handler::anthropic::handle))
            .route("/v1/responses", post(crate::handler::responses::handle))
            .route("/v1beta/models/{model}:generateContent",
                post(crate::handler::gemini::handle))
            .with_state(state.clone());

        let listener = TcpListener::bind(&self.bind_addr).await
            .expect("Failed to bind server");
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        MockServer { addr, _handle: handle }
    }
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MockServer {
    addr: std::net::SocketAddr,
    _handle: tokio::task::JoinHandle<()>,
}

impl MockServer {
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self._handle.abort();
    }
}
```

- [ ] **Step 3: Update lib.rs**

```rust
pub mod fixture;
pub mod format;
pub mod server;
pub mod handler;

pub use fixture::Fixture;
pub use server::{ServerBuilder, MockServer};
```

- [ ] **Step 4: Create handler module stubs** (needed for compilation)

Create `src/handler/mod.rs`:
```rust
pub mod openai;
pub mod anthropic;
pub mod gemini;
pub mod responses;
```

Create stub handlers (`src/handler/openai.rs`, etc.) with placeholder `handle` functions that return 501:

```rust
use std::sync::Arc;
use axum::extract::State;
use axum::http::StatusCode;
use crate::server::AppState;

pub async fn handle(
    State(_state): State<Arc<AppState>>,
    body: String,
) -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --lib server`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/server.rs src/handler/mod.rs src/handler/openai.rs src/handler/anthropic.rs src/handler/gemini.rs src/handler/responses.rs src/lib.rs
git commit -m "$(cat <<'EOF'
feat: add ServerBuilder, MockServer, and handler stubs
EOF
)"
```

---

### Task 12: OpenAI Handler (Non-Streaming)

**Files:**
- Modify: `src/handler/openai.rs`

- [ ] **Step 1: Write integration test**

Create `tests/integration/mod.rs` and `tests/integration/openai_test.rs`:

```rust
// tests/integration/openai_test.rs
use llmposter::{ServerBuilder, Fixture};
use llmposter::fixture::{FixtureMatch, FixtureResponse, FixtureError, StringMatch};

#[tokio::test]
async fn should_return_openai_chat_completion() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::Substring("hello".to_string())),
                model: None,
            }),
            provider: None,
            response: Some(FixtureResponse {
                content: Some("Hi from mock!".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            error: None,
            failure: None,
            streaming: None,
        })
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "Hi from mock!");
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
    assert_eq!(body["object"], "chat.completion");
    assert!(body["id"].as_str().unwrap().starts_with("chatcmpl-llmposter-"));
}

#[tokio::test]
async fn should_return_404_when_no_fixture_matches() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::Substring("specific".to_string())),
                model: None,
            }),
            provider: None,
            response: Some(FixtureResponse {
                content: Some("specific response".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            error: None,
            failure: None,
            streaming: None,
        })
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "unmatched"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("No fixture matched"));
}

#[tokio::test]
async fn should_return_error_fixture() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            match_rule: None,
            provider: None,
            response: None,
            error: Some(FixtureError {
                status: 429,
                message: "Rate limit exceeded".to_string(),
            }),
            failure: None,
            streaming: None,
        })
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "anything"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 429);
}
```

- [ ] **Step 2: Implement the OpenAI handler**

Update `src/handler/openai.rs` with full non-streaming handling:
- Parse JSON body
- Call `format::openai::extract_request_info()`
- Call `fixture::match_fixture()` with provider = "openai"
- If no match → 404 with JSON error
- If error fixture → return error status
- If response fixture → call `format::openai::build_response()` and return JSON
- Handle `stream: true` flag (placeholder for now — return non-streaming)

- [ ] **Step 3: Run integration tests**

Run: `cargo test --test integration`
Expected: All OpenAI integration tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/handler/openai.rs tests/
git commit -m "$(cat <<'EOF'
feat: implement OpenAI chat completions handler (non-streaming)
EOF
)"
```

---

### Task 13: Anthropic Handler (Non-Streaming)

**Files:**
- Modify: `src/handler/anthropic.rs`
- Create: `tests/integration/anthropic_test.rs`

- [ ] **Step 1: Write integration tests**

```rust
#[tokio::test]
async fn should_return_anthropic_messages_response() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .match_user_message("hello")
            .respond_with_content("Hi from Claude mock!"))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["type"], "message");
    assert_eq!(body["role"], "assistant");
    assert_eq!(body["content"][0]["type"], "text");
    assert_eq!(body["content"][0]["text"], "Hi from Claude mock!");
    assert_eq!(body["stop_reason"], "end_turn");
    assert!(body["id"].as_str().unwrap().starts_with("msg-llmposter-"));
    assert!(body["usage"]["input_tokens"].as_u64().unwrap() > 0);
    assert!(body["usage"]["output_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn should_return_400_for_unparseable_anthropic_request() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .header("content-type", "application/json")
        .body("not json")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_extract_content_from_array_format() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .match_user_message("array content")
            .respond_with_content("got it"))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": [{"type": "text", "text": "array content"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}
```

- [ ] **Step 2: Implement Anthropic handler**

In `src/handler/anthropic.rs`:
- Parse JSON body, return 400 if unparseable
- Call `format::anthropic::extract_request_info()` — handle both string and array content formats
- Call `fixture::match_fixture()` with provider = "anthropic"
- If no match → 404 with JSON error
- If error fixture → return error status
- If response fixture → call `format::anthropic::build_response()`, return JSON
- If fixture has `stop_reason` override, use it instead of default "end_turn"

- [ ] **Step 3: Run tests**

Run: `cargo test --test integration anthropic`
Expected: All Anthropic integration tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/handler/anthropic.rs tests/integration/anthropic_test.rs
git commit -m "$(cat <<'EOF'
feat: implement Anthropic messages handler (non-streaming)
EOF
)"
```

---

### Task 14: Gemini Handler (Non-Streaming)

**Files:**
- Modify: `src/handler/gemini.rs`
- Create: `tests/integration/gemini_test.rs`

- [ ] **Step 1: Write integration tests**

```rust
#[tokio::test]
async fn should_return_gemini_generate_content_response() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .match_user_message("hello")
            .respond_with_content("Hi from Gemini mock!"))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1beta/models/gemini-pro:generateContent", server.url()))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello world"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["candidates"][0]["content"]["parts"][0]["text"], "Hi from Gemini mock!");
    assert_eq!(body["candidates"][0]["content"]["role"], "model");
    assert_eq!(body["candidates"][0]["finishReason"], "STOP");
    assert!(body["usageMetadata"]["promptTokenCount"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn should_extract_model_from_url_path() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .match_model("gemini-pro")
            .respond_with_content("matched model"))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1beta/models/gemini-pro:generateContent", server.url()))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["candidates"][0]["content"]["parts"][0]["text"], "matched model");
}

#[tokio::test]
async fn should_return_400_for_missing_contents() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1beta/models/gemini-pro:generateContent", server.url()))
        .json(&serde_json::json!({"not_contents": "bad"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}
```

- [ ] **Step 2: Implement Gemini handler**

In `src/handler/gemini.rs`:
- Extract model from URL path parameter (`{model}` segment, strip `:generateContent` suffix)
- Parse JSON body, extract user message from `contents[].parts[].text`
- Return 400 if `contents` missing
- Match fixtures with provider = "gemini"
- Return Gemini-shaped response (camelCase fields)

- [ ] **Step 3: Run tests**

Run: `cargo test --test integration gemini`
Expected: All Gemini integration tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/handler/gemini.rs tests/integration/gemini_test.rs
git commit -m "$(cat <<'EOF'
feat: implement Gemini generateContent handler (non-streaming)
EOF
)"
```

---

### Task 15: Responses API Handler (Non-Streaming)

**Files:**
- Modify: `src/handler/responses.rs`
- Create: `tests/integration/responses_test.rs`

- [ ] **Step 1: Write integration tests**

```rust
#[tokio::test]
async fn should_return_responses_api_response() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .match_user_message("hello")
            .respond_with_content("Hi from Responses mock!"))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "response");
    assert_eq!(body["output"][0]["type"], "message");
    assert_eq!(body["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(body["output"][0]["content"][0]["text"], "Hi from Responses mock!");
    assert!(body["id"].as_str().unwrap().starts_with("resp-llmposter-"));
}

#[tokio::test]
async fn should_handle_string_input() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .match_user_message("string prompt")
            .respond_with_content("got string"))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": "string prompt"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_return_400_for_missing_input() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({"model": "gpt-4"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}
```

- [ ] **Step 2: Implement Responses handler**

In `src/handler/responses.rs`:
- Parse JSON body, extract model and user message
- `input` can be a string (use directly) or array of messages (find last user message)
- Return 400 if `input` missing
- Match fixtures with provider = "responses"
- Return Responses API shape

- [ ] **Step 3: Run tests**

Run: `cargo test --test integration responses`
Expected: All Responses API integration tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/handler/responses.rs tests/integration/responses_test.rs
git commit -m "$(cat <<'EOF'
feat: implement Responses API handler (non-streaming)
EOF
)"
```

---

## Chunk 4: SSE Streaming

### Task 16: SSE Stream Writer

**Files:**
- Create: `src/stream.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_chunk_content() {
        let chunks = chunk_content("Hello, world!", 5);
        assert_eq!(chunks, vec!["Hello", ", wor", "ld!"]);
    }

    #[test]
    fn should_chunk_content_exact_size() {
        let chunks = chunk_content("abcdef", 3);
        assert_eq!(chunks, vec!["abc", "def"]);
    }

    #[test]
    fn should_chunk_content_default_size() {
        let chunks = chunk_content("short", 20);
        assert_eq!(chunks, vec!["short"]);
    }

    #[test]
    fn should_handle_empty_content() {
        let chunks = chunk_content("", 20);
        assert!(chunks.is_empty());
    }
}
```

- [ ] **Step 2: Implement stream utilities**

In `src/stream.rs`:

```rust
/// Split content into chunks of at most `chunk_size` characters.
pub fn chunk_content(content: &str, chunk_size: usize) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }
    content
        .chars()
        .collect::<Vec<_>>()
        .chunks(chunk_size)
        .map(|c| c.iter().collect::<String>())
        .collect()
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test --lib stream`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/stream.rs src/lib.rs
git commit -m "$(cat <<'EOF'
feat: add SSE stream utilities with content chunking
EOF
)"
```

---

### Task 17: OpenAI Streaming

**Files:**
- Modify: `src/handler/openai.rs`
- Modify: `src/format/openai.rs`

- [ ] **Step 1: Write streaming integration test**

In `tests/integration/openai_test.rs`:

```rust
#[tokio::test]
async fn should_stream_openai_response() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            match_rule: None,
            provider: None,
            response: Some(FixtureResponse {
                content: Some("Hello world".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            error: None,
            failure: None,
            streaming: Some(StreamingConfig { latency: Some(0), chunk_size: Some(5) }),
        })
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap().to_str().unwrap(),
        "text/event-stream"
    );

    let body = resp.text().await.unwrap();
    // Should contain SSE data lines
    assert!(body.contains("data: "));
    // Should end with [DONE]
    assert!(body.contains("data: [DONE]"));
    // Should contain content chunks
    assert!(body.contains("Hello"));
}
```

- [ ] **Step 2: Add streaming chunk builders to format/openai.rs**

Add `build_stream_chunks()` that returns `Vec<ChatCompletionChunk>` — one per content chunk, plus a final chunk with `finish_reason: "stop"`.

- [ ] **Step 3: Update handler to check `stream: true` and return SSE**

When `stream: true` in the request body:
- Set response headers: `Content-Type: text/event-stream`, `Cache-Control: no-cache`, `Connection: keep-alive`
- Use axum's `Body::from_stream()` or `Sse` extractor
- For each chunk: write `data: {json}\n\n`
- End with `data: [DONE]\n\n`
- Apply `streaming.latency` delay between chunks

- [ ] **Step 4: Run tests**

Run: `cargo test --test integration openai`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/handler/openai.rs src/format/openai.rs tests/
git commit -m "$(cat <<'EOF'
feat: add SSE streaming for OpenAI chat completions
EOF
)"
```

---

### Task 18: Anthropic Streaming

**Files:**
- Modify: `src/handler/anthropic.rs`
- Modify: `src/format/anthropic.rs`

- [ ] **Step 1: Write streaming integration test**

```rust
#[tokio::test]
async fn should_stream_anthropic_response() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .match_user_message("hello")
            .respond_with_content("Hello world")
            .with_streaming(Some(0), Some(5)))
        .build()
        .await;

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
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers().get("content-type").unwrap().to_str().unwrap(), "text/event-stream");

    let body = resp.text().await.unwrap();
    // Must contain Anthropic's SSE event types
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: content_block_start"));
    assert!(body.contains("event: content_block_delta"));
    assert!(body.contains("event: message_stop"));
    // Deltas should contain text chunks
    assert!(body.contains("text_delta"));
}
```

- [ ] **Step 2: Add streaming event builders to `format/anthropic.rs`**

Build SSE event sequence for Anthropic streaming:
1. `event: message_start\ndata: {"type":"message_start","message":{"id":"...","type":"message","role":"assistant","model":"...","usage":{"input_tokens":N}}}\n\n`
2. `event: content_block_start\ndata: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}\n\n`
3. For each chunk: `event: content_block_delta\ndata: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"CHUNK"}}\n\n`
4. `event: content_block_stop\ndata: {"type":"content_block_stop","index":0}\n\n`
5. `event: message_delta\ndata: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":N}}\n\n`
6. `event: message_stop\ndata: {"type":"message_stop"}\n\n`

Return a `Vec<(String, serde_json::Value)>` of (event_type, data) pairs.

- [ ] **Step 3: Update handler to check `stream: true`**

When `stream: true` in request body:
- Set `Content-Type: text/event-stream`
- Write each SSE event with optional latency between content_block_delta events
- Apply streaming config (latency, chunk_size)

- [ ] **Step 4: Run tests**

Run: `cargo test --test integration anthropic`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/handler/anthropic.rs src/format/anthropic.rs tests/integration/anthropic_test.rs
git commit -m "$(cat <<'EOF'
feat: add SSE streaming for Anthropic messages
EOF
)"
```

---

### Task 19: Gemini Streaming

**Files:**
- Modify: `src/handler/gemini.rs`
- Modify: `src/format/gemini.rs`
- Modify: `src/server.rs` — **add `streamGenerateContent` route**

- [ ] **Step 1: Add streaming route to server.rs**

In `ServerBuilder::build()`, add alongside the existing Gemini route:
```rust
.route("/v1beta/models/{model}:streamGenerateContent",
    post(crate::handler::gemini::handle_stream))
```

- [ ] **Step 2: Write streaming integration tests**

```rust
#[tokio::test]
async fn should_stream_gemini_response_as_json_array() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .match_user_message("hello")
            .respond_with_content("Hello world")
            .with_streaming(Some(0), Some(5)))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1beta/models/gemini-pro:streamGenerateContent", server.url()))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert!(resp.headers().get("content-type").unwrap().to_str().unwrap().contains("application/json"));

    let body: serde_json::Value = resp.json().await.unwrap();
    // Gemini streaming returns a JSON array
    assert!(body.is_array());
    let arr = body.as_array().unwrap();
    assert!(!arr.is_empty());
    // Each element has same shape as non-streaming
    assert!(arr[0]["candidates"][0]["content"]["parts"][0]["text"].is_string());
}

#[tokio::test]
async fn should_stream_gemini_response_as_sse_with_alt_param() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .match_user_message("hello")
            .respond_with_content("Hello world")
            .with_streaming(Some(0), Some(5)))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1beta/models/gemini-pro:streamGenerateContent?alt=sse", server.url()))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers().get("content-type").unwrap().to_str().unwrap(), "text/event-stream");

    let body = resp.text().await.unwrap();
    assert!(body.contains("data: "));
}
```

- [ ] **Step 3: Implement `handle_stream` function**

Two modes based on `alt=sse` query param:
- Default: collect all chunks into a JSON array, return as `application/json`
- `alt=sse`: return SSE format with `data: {json}\n\n` per chunk

- [ ] **Step 4: Run tests**

Run: `cargo test --test integration gemini`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/handler/gemini.rs src/format/gemini.rs src/server.rs tests/integration/gemini_test.rs
git commit -m "$(cat <<'EOF'
feat: add streaming for Gemini (JSON array + alt=sse)
EOF
)"
```

---

### Task 20: Responses API Streaming

**Files:**
- Modify: `src/handler/responses.rs`
- Modify: `src/format/responses.rs`

- [ ] **Step 1: Write streaming integration test**

```rust
#[tokio::test]
async fn should_stream_responses_api_response() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .match_user_message("hello")
            .respond_with_content("Hello world")
            .with_streaming(Some(0), Some(5)))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers().get("content-type").unwrap().to_str().unwrap(), "text/event-stream");

    let body = resp.text().await.unwrap();
    // Must contain Responses API event types
    assert!(body.contains("event: response.created"));
    assert!(body.contains("event: response.output_text.delta"));
    assert!(body.contains("event: response.completed"));
}
```

- [ ] **Step 2: Add streaming event builders to `format/responses.rs`**

Build SSE event sequence:
1. `event: response.created\ndata: {"type":"response.created","response":{...}}\n\n`
2. `event: response.output_item.added\ndata: {"type":"response.output_item.added",...}\n\n`
3. `event: response.content_part.added\ndata: {"type":"response.content_part.added",...}\n\n`
4. For each chunk: `event: response.output_text.delta\ndata: {"type":"response.output_text.delta","delta":"CHUNK"}\n\n`
5. `event: response.output_text.done\ndata: {...}\n\n`
6. `event: response.output_item.done\ndata: {...}\n\n`
7. `event: response.completed\ndata: {"type":"response.completed","response":{...}}\n\n`

- [ ] **Step 3: Update handler to check `stream: true`**
- [ ] **Step 4: Run tests**

Run: `cargo test --test integration responses`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/handler/responses.rs src/format/responses.rs tests/integration/responses_test.rs
git commit -m "$(cat <<'EOF'
feat: add SSE streaming for Responses API
EOF
)"
```

---

## Chunk 5: Failure Simulation

### Task 21: Failure Module

**Files:**
- Create: `src/failure.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write tests for failure module**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_build_error_response_body() {
        let body = build_error_body(429, "Rate limit exceeded");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["error"]["message"], "Rate limit exceeded");
    }

    #[test]
    fn should_build_no_match_response() {
        let body = build_no_match_body("gpt-4", "hello world");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(json["error"]["message"].as_str().unwrap().contains("gpt-4"));
        assert!(json["error"]["message"].as_str().unwrap().contains("hello world"));
        assert_eq!(json["error"]["type"], "no_fixture_match");
    }
}
```

- [ ] **Step 2: Implement failure module**

```rust
pub fn build_error_body(status: u16, message: &str) -> String {
    serde_json::json!({
        "error": {
            "message": message,
            "type": "error",
            "code": status
        }
    }).to_string()
}

pub fn build_no_match_body(model: &str, user_message: &str) -> String {
    serde_json::json!({
        "error": {
            "message": format!("No fixture matched: model='{}', user_message='{}'", model, user_message),
            "type": "no_fixture_match"
        }
    }).to_string()
}
```

- [ ] **Step 3: Run tests and commit**

```bash
git commit -m "feat: add failure module with error response builders"
```

---

### Task 22: Failure Simulation Integration

**Files:**
- Create: `tests/integration/failure_test.rs`
- Modify: handler files as needed

- [ ] **Step 1: Write integration tests for each failure type**

```rust
use llmposter::{ServerBuilder, Fixture};
use llmposter::fixture::{FailureConfig, StreamingConfig};
use std::time::Instant;

#[tokio::test]
async fn should_simulate_latency() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .respond_with_content("delayed")
            .with_failure(FailureConfig {
                latency_ms: Some(500),
                corrupt_body: None,
                truncate_after_chunks: None,
                disconnect_after_ms: None,
            }))
        .build()
        .await;

    let client = reqwest::Client::new();
    let start = Instant::now();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    let elapsed = start.elapsed();
    assert_eq!(resp.status(), 200);
    assert!(elapsed.as_millis() >= 450, "Expected >= 450ms delay, got {}ms", elapsed.as_millis());
}

#[tokio::test]
async fn should_simulate_corrupt_body() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .respond_with_content("should not appear")
            .with_failure(FailureConfig {
                latency_ms: None,
                corrupt_body: Some(true),
                truncate_after_chunks: None,
                disconnect_after_ms: None,
            }))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let content_type = resp.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(content_type.contains("text/plain"));
    let body = resp.text().await.unwrap();
    assert_eq!(body, "overloaded");
}

#[tokio::test]
async fn should_simulate_truncated_stream() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .respond_with_content("This is a long response that should be truncated after 2 chunks")
            .with_streaming(Some(0), Some(5))
            .with_failure(FailureConfig {
                latency_ms: None,
                corrupt_body: None,
                truncate_after_chunks: Some(2),
                disconnect_after_ms: None,
            }))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Should have content but NO [DONE] marker
    assert!(body.contains("data: "));
    assert!(!body.contains("[DONE]"));
    // Count data lines (excluding empty lines)
    let data_lines: Vec<&str> = body.lines().filter(|l| l.starts_with("data: {")).collect();
    // Should have at most 2 content chunks (plus possibly a role chunk)
    assert!(data_lines.len() <= 3, "Expected <= 3 data lines, got {}", data_lines.len());
}

#[tokio::test]
async fn should_simulate_disconnect() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new()
            .respond_with_content("This response will be cut off by a disconnect")
            .with_streaming(Some(50), Some(5))  // 50ms between chunks to ensure disconnect hits
            .with_failure(FailureConfig {
                latency_ms: None,
                corrupt_body: None,
                truncate_after_chunks: None,
                disconnect_after_ms: Some(100),
            }))
        .build()
        .await;

    let client = reqwest::Client::new();
    let result = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await;

    // The request may succeed initially but reading the body should fail,
    // or the connection may be reset entirely
    match result {
        Ok(resp) => {
            let body_result = resp.text().await;
            // Body should either be incomplete or fail to read
            match body_result {
                Ok(body) => assert!(!body.contains("[DONE]"), "Stream should not complete"),
                Err(_) => {} // Connection reset — expected
            }
        }
        Err(_) => {} // Connection refused/reset — expected
    }
}
```

- [ ] **Step 2: Implement failure handling in handlers**

Add failure processing to each handler:
- Check for `failure.latency_ms` → `tokio::time::sleep`
- Check for `failure.corrupt_body` → return `"overloaded"` with `Content-Type: text/plain`
- For streaming: check `failure.truncate_after_chunks` → stop after N chunks
- For streaming: check `failure.disconnect_after_ms` → abort stream after delay

- [ ] **Step 3: Run all tests**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat: implement failure simulation (latency, corruption, truncation, disconnect)"
```

---

## Chunk 6: CLI Binary

### Task 23: CLI with clap

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Implement CLI**

```rust
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "llmposter", about = "Mock LLM API server")]
struct Cli {
    /// Path to fixtures directory or file
    #[arg(short, long)]
    fixtures: PathBuf,

    /// Validate fixtures without starting server
    #[arg(long)]
    validate: bool,

    /// Port to listen on (default: random)
    #[arg(short, long, default_value = "0")]
    port: u16,

    /// Bind address
    #[arg(short, long, default_value = "127.0.0.1")]
    bind: String,

    /// Verbose logging to stderr
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let fixtures = if cli.fixtures.is_dir() {
        llmposter::fixture::load_yaml_dir(&cli.fixtures)
    } else {
        llmposter::fixture::load_yaml_file(&cli.fixtures)
    };

    let fixtures = match fixtures {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error loading fixtures: {}", e);
            std::process::exit(1);
        }
    };

    if cli.validate {
        eprintln!("Validated {} fixtures successfully", fixtures.len());
        return;
    }

    let bind_addr = format!("{}:{}", cli.bind, cli.port);
    let server = llmposter::ServerBuilder::new()
        .fixtures(fixtures)
        .bind(&bind_addr)
        .build()
        .await;

    eprintln!("llmposter listening on {}", server.url());
    eprintln!("Press Ctrl+C to stop");

    tokio::signal::ctrl_c().await.ok();
}
```

- [ ] **Step 2: Test CLI compiles and runs**

Run: `cargo build`
Run: `./target/debug/llmposter --help`
Expected: Shows help text with all flags.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "$(cat <<'EOF'
feat: add CLI binary with clap (fixtures, validate, port, bind, verbose)
EOF
)"
```

---

## Chunk 7: Integration Tests + Polish

### Task 24: YAML Fixture Integration Tests

**Files:**
- Create: `tests/integration/fixture_yaml_test.rs`
- Create: `tests/fixtures/` directory with sample YAML files

- [ ] **Step 1: Create sample fixture files**

Create `tests/fixtures/basic.yaml` with fixtures covering all features.

- [ ] **Step 2: Write tests that load YAML and hit endpoints**

```rust
#[tokio::test]
async fn should_serve_responses_from_yaml_fixtures() {
    let server = ServerBuilder::new()
        .load_yaml_dir(Path::new("tests/fixtures/")).unwrap()
        .build()
        .await;
    // Hit each endpoint and verify responses match fixture content
}
```

- [ ] **Step 3: Run all tests**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git commit -m "test: add YAML fixture integration tests"
```

---

### Task 25: Gap Coverage — Tool Calls, 400s, IDs, Verbose, Stop Reason

**Goal:** Address remaining spec requirements not fully covered by earlier tasks.

- [ ] **Step 1: Add tool call builders for Gemini and Responses API**

In `src/format/gemini.rs`, add `build_tool_call_response()`:
- Gemini tool calls use `functionCall` content parts with `name` and `args` (object, not string)
- Test: round-trip serialize a tool call response and verify shape

In `src/format/responses.rs`, add `build_tool_call_response()`:
- Responses API uses `function_call` output items
- Test: same pattern

- [ ] **Step 2: Add 400 error handling test for OpenAI**

In `tests/integration/openai_test.rs`:
```rust
#[tokio::test]
async fn should_return_400_for_unparseable_json() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("content-type", "application/json")
        .body("not json at all")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_return_400_for_missing_messages() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({"model": "gpt-4"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}
```

- [ ] **Step 3: Add ID counter isolation test**

```rust
#[tokio::test]
async fn should_have_independent_id_counters_per_server() {
    let server1 = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("one"))
        .build()
        .await;
    let server2 = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("two"))
        .build()
        .await;

    let client = reqwest::Client::new();

    let resp1: serde_json::Value = client
        .post(format!("{}/v1/chat/completions", server1.url()))
        .json(&serde_json::json!({"model": "x", "messages": [{"role": "user", "content": "hi"}]}))
        .send().await.unwrap().json().await.unwrap();

    let resp2: serde_json::Value = client
        .post(format!("{}/v1/chat/completions", server2.url()))
        .json(&serde_json::json!({"model": "x", "messages": [{"role": "user", "content": "hi"}]}))
        .send().await.unwrap().json().await.unwrap();

    // Both should start at 1 — independent counters
    assert_eq!(resp1["id"], "chatcmpl-llmposter-1");
    assert_eq!(resp2["id"], "chatcmpl-llmposter-1");
}
```

- [ ] **Step 4: Wire `--verbose` flag through ServerBuilder to handlers**

Add `verbose: bool` to `AppState`. When true, handlers log to stderr:
- On match: `eprintln!("[llmposter] {} {} → fixture #{}", method, path, index)`
- On no match: `eprintln!("[llmposter] {} {} → no match (model='{}', msg='{}')", ...)`

Add `verbose(bool)` method to `ServerBuilder`.

- [ ] **Step 5: Add stop_reason/finish_reason fixture override support**

In handlers, when building responses:
- If fixture has `stop_reason` set, use it instead of default ("end_turn" for Anthropic)
- If fixture has `finish_reason` set, use it instead of default ("stop" for OpenAI)
- Test: fixture with `stop_reason: "max_tokens"` → response has that value

- [ ] **Step 6: Run all tests**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
git commit -m "$(cat <<'EOF'
feat: add tool calls for Gemini/Responses, 400 handling, ID isolation, verbose, stop_reason override
EOF
)"
```

---

### Task 26: Coverage Check + Clippy Clean

- [ ] **Step 1: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings.

- [ ] **Step 2: Run full test suite**

Run: `cargo test`
Expected: All tests pass.

- [ ] **Step 3: Check coverage (if cargo-tarpaulin installed)**

Run: `cargo tarpaulin --out Stdout`
Expected: >= 97% coverage.

- [ ] **Step 4: Fix any coverage gaps**

Add tests for uncovered branches.

- [ ] **Step 5: Commit**

```bash
git commit -m "test: achieve 97%+ coverage target"
```

---

### Task 26: CHANGELOG + README

**Files:**
- Create: `CHANGELOG.md`
- Modify: `README.md`

- [ ] **Step 1: Write CHANGELOG.md**

```markdown
# Changelog

## [0.1.0] - 2026-03-14

### Added
- Initial release
- Mock server for 4 LLM API providers: OpenAI Chat Completions, Anthropic Messages, Gemini generateContent, OpenAI Responses API
- YAML fixture format with substring and regex matching
- SSE streaming support for all providers
- Failure simulation: HTTP errors, latency, body corruption, stream truncation, connection disconnect
- In-process Rust library with `ServerBuilder` API
- CLI binary with `--fixtures`, `--validate`, `--port`, `--bind`, `--verbose` flags
- IPv4 and IPv6 support
```

- [ ] **Step 2: Update README.md**

Update with project description, quick start examples (library + CLI), fixture format reference, and credit to llmock for inspiration.

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md README.md
git commit -m "docs: add CHANGELOG and update README for v0.1.0"
```

---

### Task 27: Final Audit

- [ ] **Step 1: Run the 4 horsemen**

1. `/simplify` — code reuse, quality, efficiency
2. `dev/scripts/run-codex-audit.sh` — architecture, security
3. `dev/scripts/run-gemini-audit.sh` — output quality
4. `dev/scripts/run-coderabbit.sh` — nits, style
5. `dev/scripts/run-claude-audit.sh` — Rust deep dive, security

- [ ] **Step 2: Fix P1/P2 findings**
- [ ] **Step 3: Final commit and push**

```bash
git push -u origin feat/initial-implementation
```

- [ ] **Step 4: Open draft PR**

```bash
gh pr create --draft --title "feat: llmposter v0.1.0 — mock LLM server" --body "..."
```
