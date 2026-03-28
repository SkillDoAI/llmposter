use serde::Deserialize;
use std::path::Path;

/// How to match a string field — substring (default) or regex.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum StringMatch {
    Substring(String),
    Regex(RegexMatch),
}

/// Wrapper for `{ regex: "pattern" }` syntax in YAML.
/// After validation, `compiled` holds the pre-compiled regex for efficient linear-time matching.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegexMatch {
    pub regex: String,
    #[serde(skip)]
    compiled: Option<regex::Regex>,
}

impl PartialEq for RegexMatch {
    fn eq(&self, other: &Self) -> bool {
        self.regex == other.regex
    }
}

impl RegexMatch {
    fn compile(&mut self) -> Result<(), String> {
        if self.compiled.is_some() {
            return Ok(()); // Already compiled, skip
        }
        let re = regex::RegexBuilder::new(&self.regex)
            .size_limit(1 << 20)
            .dfa_size_limit(1 << 20) // Cap both compiled NFA and per-thread DFA cache
            .build()
            .map_err(|e| format!("Invalid regex '{}': {}", self.regex, e))?;
        self.compiled = Some(re);
        Ok(())
    }

    fn is_match(&self, haystack: &str) -> bool {
        match &self.compiled {
            Some(re) => re.is_match(haystack),
            None => {
                // Fallback: compile on the fly. This path is only hit if
                // validate() was not called (programmatic fixtures added
                // without going through ServerBuilder::build).
                match regex::RegexBuilder::new(&self.regex)
                    .size_limit(1 << 20)
                    .dfa_size_limit(1 << 20)
                    .build()
                {
                    Ok(re) => re.is_match(haystack),
                    Err(e) => {
                        eprintln!("[llmposter] Warning: invalid regex '{}': {}", self.regex, e);
                        false
                    }
                }
            }
        }
    }
}

impl StringMatch {
    pub fn regex(pattern: &str) -> Self {
        StringMatch::Regex(RegexMatch {
            regex: pattern.to_string(),
            compiled: None,
        })
    }
}

/// Match criteria for a fixture.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FixtureMatch {
    pub user_message: Option<StringMatch>,
    pub model: Option<StringMatch>,
}

/// A tool call in a fixture response.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// The response to return when a fixture matches.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureResponse {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub stop_reason: Option<String>,
    pub finish_reason: Option<String>,
}

/// Error simulation — returns an HTTP error status.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureError {
    pub status: u16,
    pub message: String,
}

/// Failure simulation — network/streaming problems.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FailureConfig {
    pub latency_ms: Option<u64>,
    pub corrupt_body: Option<bool>,
    /// Truncate SSE stream after N frames (including preamble events).
    /// Alias: `truncate_after_chunks` (deprecated, use `truncate_after_frames`).
    #[serde(alias = "truncate_after_chunks")]
    pub truncate_after_frames: Option<u32>,
    pub disconnect_after_ms: Option<u64>,
}

/// Streaming behavior config.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamingConfig {
    pub latency: Option<u64>,
    pub chunk_size: Option<usize>,
}

/// A single fixture entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    #[serde(rename = "match")]
    pub match_rule: Option<FixtureMatch>,
    pub provider: Option<crate::format::Provider>,
    pub response: Option<FixtureResponse>,
    pub error: Option<FixtureError>,
    pub failure: Option<FailureConfig>,
    pub streaming: Option<StreamingConfig>,
}

/// Top-level YAML file structure (internal, used for deserialization only).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FixtureFile {
    pub fixtures: Vec<Fixture>,
}

// --- Programmatic builder API ---

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
        let r = self.response.get_or_insert(FixtureResponse {
            content: None,
            tool_calls: None,
            stop_reason: None,
            finish_reason: None,
        });
        r.content = Some(content.to_string());
        r.tool_calls = None;
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

    pub fn with_stop_reason(mut self, reason: &str) -> Self {
        self.response
            .get_or_insert(FixtureResponse {
                content: None,
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            })
            .stop_reason = Some(reason.to_string());
        self
    }

    pub fn with_finish_reason(mut self, reason: &str) -> Self {
        self.response
            .get_or_insert(FixtureResponse {
                content: None,
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            })
            .finish_reason = Some(reason.to_string());
        self
    }

    pub fn with_streaming(mut self, latency: Option<u64>, chunk_size: Option<usize>) -> Self {
        self.streaming = Some(StreamingConfig {
            latency,
            chunk_size,
        });
        self
    }

    pub fn for_provider(mut self, provider: crate::format::Provider) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn respond_with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        let r = self.response.get_or_insert(FixtureResponse {
            content: None,
            tool_calls: None,
            stop_reason: None,
            finish_reason: None,
        });
        r.tool_calls = Some(tool_calls);
        r.content = None;
        self
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

// --- Validation ---

impl Fixture {
    /// Validate fixture invariants and pre-compile regex patterns.
    pub fn validate(&mut self) -> Result<(), String> {
        if let Some(ref e) = self.error {
            if !(400..=599).contains(&e.status) {
                return Err("error.status must be an error HTTP status (400-599)".to_string());
            }
        }
        if self.response.is_some() && self.error.is_some() {
            return Err("'error' and 'response' are mutually exclusive".to_string());
        }
        if self.error.is_some() && self.failure.is_some() {
            return Err("'error' and 'failure' are mutually exclusive".to_string());
        }
        if self.failure.is_some() && self.response.is_none() {
            return Err("'failure' requires response to also be present".to_string());
        }
        if let (Some(ref f), None) = (&self.failure, &self.streaming) {
            let has_stream_failure =
                f.truncate_after_frames.is_some() || f.disconnect_after_ms.is_some();
            let has_content = self
                .response
                .as_ref()
                .map(|r| r.content.is_some())
                .unwrap_or(false);
            if has_stream_failure && has_content {
                eprintln!(
                    "[llmposter] Warning: failure.truncate_after_frames/disconnect_after_ms \
                     have no effect without streaming configured"
                );
            }
        }
        // Validate FixtureResponse mutual exclusivity
        if let Some(ref r) = self.response {
            if r.content.is_some() && r.tool_calls.is_some() {
                return Err(
                    "'content' and 'tool_calls' in response are mutually exclusive".to_string(),
                );
            }
            if r.content.is_none() && r.tool_calls.is_none() {
                return Err("response must have either 'content' or 'tool_calls'".to_string());
            }
            if let Some(ref tc) = r.tool_calls {
                if tc.is_empty() {
                    return Err("tool_calls must not be empty".to_string());
                }
                for (i, call) in tc.iter().enumerate() {
                    if call.name.trim().is_empty() {
                        return Err(format!("tool_calls[{}].name must not be empty", i));
                    }
                    if !call.arguments.is_object() {
                        return Err(format!(
                            "tool_calls[{}].arguments must be a JSON object, got {}",
                            i,
                            match &call.arguments {
                                serde_json::Value::Array(_) => "array",
                                serde_json::Value::String(_) => "string",
                                serde_json::Value::Number(_) => "number",
                                serde_json::Value::Bool(_) => "boolean",
                                serde_json::Value::Null => "null",
                                _ => "non-object",
                            }
                        ));
                    }
                }
            }
        }
        if self.response.is_none() && self.error.is_none() {
            return Err("Fixture must have either 'response' or 'error'".to_string());
        }
        if let Some(ref s) = self.streaming {
            if s.chunk_size == Some(0) {
                return Err("streaming.chunk_size must be > 0".to_string());
            }
            if self.error.is_some() {
                return Err("'streaming' config has no effect on error-only fixtures".to_string());
            }
        }
        if let Some(ref mut m) = self.match_rule {
            // Reject empty substring patterns (would match everything)
            if let Some(StringMatch::Substring(ref s)) = m.user_message {
                if s.is_empty() {
                    return Err("match.user_message must not be empty".to_string());
                }
            }
            if let Some(StringMatch::Substring(ref s)) = m.model {
                if s.is_empty() {
                    return Err("match.model must not be empty".to_string());
                }
            }
            if let Some(StringMatch::Regex(ref mut r)) = m.user_message {
                if r.regex.is_empty() {
                    return Err("match.user_message regex must not be empty".to_string());
                }
                r.compile().map_err(|e| format!("user_message {}", e))?;
            }
            if let Some(StringMatch::Regex(ref mut r)) = m.model {
                if r.regex.is_empty() {
                    return Err("match.model regex must not be empty".to_string());
                }
                r.compile().map_err(|e| format!("model {}", e))?;
            }
        }
        Ok(())
    }
}

// --- Matching ---

pub fn match_fixture<'a>(
    fixtures: &'a [Fixture],
    user_message: &str,
    model: Option<&str>,
    provider: Option<crate::format::Provider>,
) -> Option<&'a Fixture> {
    fixtures
        .iter()
        .find(|f| fixture_matches(f, user_message, model, provider))
}

fn fixture_matches(
    fixture: &Fixture,
    user_message: &str,
    model: Option<&str>,
    provider: Option<crate::format::Provider>,
) -> bool {
    if let Some(fp) = fixture.provider {
        match provider {
            Some(p) if p == fp => {}
            _ => return false,
        }
    }

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
        StringMatch::Regex(r) => r.is_match(haystack),
    }
}

// --- YAML loading ---

pub fn load_yaml_file(path: &Path) -> Result<Vec<Fixture>, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let file: FixtureFile = serde_yaml_ng::from_str(&content)
        .map_err(|e| format!("Invalid YAML in {}: {}", path.display(), e))?;

    let mut fixtures = file.fixtures;
    for (i, fixture) in fixtures.iter_mut().enumerate() {
        fixture
            .validate()
            .map_err(|e| format!("Fixture #{} in {}: {}", i + 1, path.display(), e))?;
    }

    Ok(fixtures)
}

pub fn load_yaml_dir(dir: &Path) -> Result<Vec<Fixture>, Box<dyn std::error::Error>> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read directory {}: {}", dir.display(), e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Error reading directory entry in {}: {}", dir.display(), e))?
        .into_iter()
        .filter(|e| {
            let is_file = e.file_type().map(|ft| ft.is_file()).unwrap_or(false);
            if !is_file {
                return false;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- YAML parsing tests ---

    #[test]
    fn should_parse_simple_text_fixture() {
        let yaml = r#"
fixtures:
  - match:
      user_message: "hello"
    response:
      content: "Hi there!"
"#;
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(file.fixtures.len(), 1);
        let f = &file.fixtures[0];
        assert_eq!(
            f.match_rule.as_ref().unwrap().user_message,
            Some(StringMatch::Substring("hello".to_string()))
        );
        assert_eq!(
            f.response.as_ref().unwrap().content.as_deref(),
            Some("Hi there!")
        );
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
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        match &f.match_rule.as_ref().unwrap().user_message {
            Some(StringMatch::Regex(r)) => assert_eq!(r.regex, "hello \\w+"),
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
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
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
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
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
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
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
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        let tc = &file.fixtures[0]
            .response
            .as_ref()
            .unwrap()
            .tool_calls
            .as_ref()
            .unwrap()[0];
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
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        assert_eq!(f.provider, Some(crate::format::Provider::Anthropic));
    }

    #[test]
    fn should_reject_invalid_yaml() {
        let yaml = "not: [valid: yaml: {{{";
        let result: Result<FixtureFile, _> = serde_yaml_ng::from_str(yaml);
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
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
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
        let file: FixtureFile = serde_yaml_ng::from_str(yaml).unwrap();
        let f = &file.fixtures[0];
        assert!(f.match_rule.is_none());
    }

    // --- Validation tests ---

    #[test]
    fn should_reject_fixture_with_both_error_and_response() {
        let mut f = Fixture {
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
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mutually exclusive"));
    }

    #[test]
    fn should_reject_fixture_with_failure_but_no_response() {
        let mut f = Fixture {
            failure: Some(FailureConfig {
                latency_ms: Some(1000),
                corrupt_body: None,
                truncate_after_frames: None,
                disconnect_after_ms: None,
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires response"));
    }

    #[test]
    fn should_reject_fixture_with_error_and_failure() {
        let mut f = Fixture {
            error: Some(FixtureError {
                status: 429,
                message: "rate limit".to_string(),
            }),
            failure: Some(FailureConfig {
                latency_ms: Some(1000),
                corrupt_body: None,
                truncate_after_frames: None,
                disconnect_after_ms: None,
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
    }

    #[test]
    fn should_reject_fixture_with_no_response_and_no_error() {
        let mut f = Fixture {
            match_rule: Some(FixtureMatch::default()),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must have either"));
    }

    #[test]
    fn should_accept_valid_error_fixture() {
        let mut f = Fixture::new().with_error(429, "rate limit");
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_accept_valid_response_fixture() {
        let mut f = Fixture::new().respond_with_content("hi");
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_reject_invalid_regex() {
        let mut f = Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::regex("[invalid")),
                model: None,
            }),
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("regex"));
    }

    // --- Matching tests ---

    #[test]
    fn should_match_substring_user_message() {
        let fixtures = vec![Fixture::new()
            .match_user_message("hello")
            .respond_with_content("hi")];
        let result = match_fixture(&fixtures, "say hello world", None, None);
        assert!(result.is_some());
    }

    #[test]
    fn should_not_match_wrong_substring() {
        let fixtures = vec![Fixture::new()
            .match_user_message("goodbye")
            .respond_with_content("bye")];
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
            ..Fixture::new().respond_with_content("matched")
        }];
        let result = match_fixture(&fixtures, "hello world", None, None);
        assert!(result.is_some());
    }

    #[test]
    fn should_match_model() {
        let fixtures = vec![Fixture::new()
            .match_model("gpt-4")
            .respond_with_content("gpt4 response")];
        let result = match_fixture(&fixtures, "anything", Some("gpt-4-turbo"), None);
        assert!(result.is_some());
    }

    #[test]
    fn should_match_first_fixture_wins() {
        let fixtures = vec![
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("first"),
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("second"),
        ];
        let result = match_fixture(&fixtures, "hello", None, None);
        assert_eq!(
            result
                .unwrap()
                .response
                .as_ref()
                .unwrap()
                .content
                .as_deref(),
            Some("first")
        );
    }

    #[test]
    fn should_match_catch_all() {
        let fixtures = vec![Fixture::new().respond_with_content("default")];
        let result = match_fixture(&fixtures, "anything at all", None, None);
        assert!(result.is_some());
    }

    #[test]
    fn should_filter_by_provider() {
        let fixtures = vec![Fixture {
            provider: Some(crate::format::Provider::Anthropic),
            ..Fixture::new().respond_with_content("anthropic only")
        }];
        let result = match_fixture(
            &fixtures,
            "hello",
            None,
            Some(crate::format::Provider::Anthropic),
        );
        assert!(result.is_some());
        let result = match_fixture(
            &fixtures,
            "hello",
            None,
            Some(crate::format::Provider::OpenAI),
        );
        assert!(result.is_none());
    }

    // --- Builder API tests ---

    #[test]
    fn should_build_fixture_programmatically() {
        let mut f = Fixture::new()
            .match_user_message("hello")
            .respond_with_content("Hi there!");
        assert!(f.validate().is_ok());
        assert_eq!(
            f.response.as_ref().unwrap().content.as_deref(),
            Some("Hi there!")
        );
    }

    #[test]
    fn should_build_error_fixture_programmatically() {
        let mut f = Fixture::new()
            .match_model("fail-model")
            .with_error(429, "Rate limited");
        assert!(f.validate().is_ok());
        assert_eq!(f.error.as_ref().unwrap().status, 429);
    }

    #[test]
    fn should_use_default_trait_for_fixture() {
        let f = Fixture::default();
        assert!(f.response.is_none());
        assert!(f.error.is_none());
        assert!(f.match_rule.is_none());
    }

    #[test]
    fn should_compare_regex_match_by_pattern_string() {
        let a = RegexMatch {
            regex: "hello".to_string(),
            compiled: None,
        };
        let b = RegexMatch {
            regex: "hello".to_string(),
            compiled: None,
        };
        let c = RegexMatch {
            regex: "world".to_string(),
            compiled: None,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn should_reject_response_with_both_content_and_tool_calls() {
        let mut f = Fixture {
            response: Some(FixtureResponse {
                content: Some("text".to_string()),
                tool_calls: Some(vec![ToolCall {
                    name: "func".to_string(),
                    arguments: serde_json::json!({}),
                }]),
                stop_reason: None,
                finish_reason: None,
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mutually exclusive"));
    }

    #[test]
    fn should_reject_response_with_neither_content_nor_tool_calls() {
        let mut f = Fixture {
            response: Some(FixtureResponse {
                content: None,
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must have either"));
    }

    #[test]
    fn should_reject_zero_chunk_size() {
        let mut f = Fixture {
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            streaming: Some(StreamingConfig {
                latency: None,
                chunk_size: Some(0),
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("chunk_size must be > 0"));
    }

    #[test]
    fn should_compile_model_regex_on_validate() {
        let mut f = Fixture {
            match_rule: Some(FixtureMatch {
                user_message: None,
                model: Some(StringMatch::regex("gpt-4.*")),
            }),
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            ..Fixture::new()
        };
        assert!(f.validate().is_ok());
        // After validation, the compiled regex should be used for matching
        let fixtures = vec![f];
        let result = match_fixture(&fixtures, "hello", Some("gpt-4-turbo"), None);
        assert!(result.is_some());
    }

    #[test]
    fn should_match_compiled_user_message_regex() {
        let mut f = Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::regex("he.*ld")),
                model: None,
            }),
            response: Some(FixtureResponse {
                content: Some("matched".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            ..Fixture::new()
        };
        // Validate to compile the regex
        assert!(f.validate().is_ok());
        // Now match against it -- should use the compiled (Some) path
        let fixtures = vec![f];
        let result = match_fixture(&fixtures, "hello world", None, None);
        assert!(result.is_some());
    }

    #[test]
    fn should_reject_invalid_model_regex() {
        let mut f = Fixture {
            match_rule: Some(FixtureMatch {
                user_message: None,
                model: Some(StringMatch::regex("[invalid")),
            }),
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("model"));
    }

    #[test]
    fn should_not_match_model_when_no_model_provided() {
        let fixtures = vec![Fixture::new()
            .match_model("gpt-4")
            .respond_with_content("gpt4 only")];
        // model is None: should NOT match a fixture that requires a model
        let result = match_fixture(&fixtures, "hello", None, None);
        assert!(result.is_none());
    }

    #[test]
    fn should_use_regex_fallback_for_unvalidated_fixture() {
        // Fixture with valid regex that was NOT validated (no compile() called).
        // This exercises the fallback path in RegexMatch::is_match.
        let fixtures = vec![Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::regex("hel+o")),
                model: None,
            }),
            response: Some(FixtureResponse {
                content: Some("matched".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            ..Fixture::new()
        }];
        let result = match_fixture(&fixtures, "helllo world", None, None);
        assert!(result.is_some());
    }

    // --- YAML file loading tests ---

    #[test]
    fn should_load_yaml_file() {
        let dir = std::env::temp_dir().join("llmposter_test_load");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("test.yaml");
        std::fs::write(
            &file,
            r#"
fixtures:
  - match:
      user_message: "test"
    response:
      content: "loaded from file"
"#,
        )
        .unwrap();
        let fixtures = load_yaml_file(&file).unwrap();
        assert_eq!(fixtures.len(), 1);
        assert_eq!(
            fixtures[0].response.as_ref().unwrap().content.as_deref(),
            Some("loaded from file")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn should_load_yaml_dir() {
        let dir = std::env::temp_dir().join("llmposter_test_dir");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.yaml"),
            "fixtures:\n  - match:\n      user_message: \"a\"\n    response:\n      content: \"a\"",
        )
        .unwrap();
        std::fs::write(
            dir.join("b.yml"),
            "fixtures:\n  - match:\n      user_message: \"b\"\n    response:\n      content: \"b\"",
        )
        .unwrap();
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
        let result = load_yaml_file(Path::new("/nonexistent/file.yaml"));
        assert!(result.is_err());
    }

    #[test]
    fn should_validate_fixtures_on_load() {
        let dir = std::env::temp_dir().join("llmposter_test_validate_load");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("invalid_fixture.yaml");
        std::fs::write(
            &file,
            r#"
fixtures:
  - match:
      user_message: "test"
    response:
      content: "hi"
    error:
      status: 500
      message: "also error"
"#,
        )
        .unwrap();
        let result = load_yaml_file(&file);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("mutually exclusive"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn should_reject_oversized_regex_at_validation() {
        // A regex with a huge repetition count that would blow up the DFA
        let huge_pattern = format!("a{{{}}}", 999_999);
        let mut f = Fixture {
            match_rule: Some(FixtureMatch {
                user_message: Some(StringMatch::regex(&huge_pattern)),
                model: None,
            }),
            response: Some(FixtureResponse {
                content: Some("hi".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: None,
            }),
            ..Fixture::new()
        };
        let result = f.validate();
        assert!(result.is_err(), "oversized regex should be rejected");
    }

    #[test]
    fn should_return_false_for_oversized_regex_in_fallback() {
        let huge_pattern = format!("a{{{}}}", 999_999);
        let rm = RegexMatch {
            regex: huge_pattern,
            compiled: None, // No pre-compilation — exercises fallback path
        };
        // Should return false, not panic or OOM
        assert!(!rm.is_match("aaaa"));
    }

    #[test]
    fn should_reject_scalar_tool_call_arguments() {
        let mut f = Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "test".to_string(),
            arguments: serde_json::json!("not an object"),
        }]);
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be a JSON object"));
    }

    #[test]
    fn should_reject_array_tool_call_arguments() {
        let mut f = Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "test".to_string(),
            arguments: serde_json::json!([1, 2, 3]),
        }]);
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be a JSON object"));
    }

    #[test]
    fn should_accept_object_tool_call_arguments() {
        let mut f = Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "test".to_string(),
            arguments: serde_json::json!({"key": "value"}),
        }]);
        assert!(f.validate().is_ok());
    }

    #[test]
    fn should_reject_blank_tool_call_name() {
        let mut f = Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "".to_string(),
            arguments: serde_json::json!({"key": "value"}),
        }]);
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("name must not be empty"));
    }

    #[test]
    fn should_reject_whitespace_only_tool_call_name() {
        let mut f = Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "   ".to_string(),
            arguments: serde_json::json!({"key": "value"}),
        }]);
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("name must not be empty"));
    }

    #[test]
    fn should_reject_non_error_status_codes() {
        // 200, 301, etc. should be rejected — only 400-599 for error simulation
        for status in [200, 204, 301, 302] {
            let mut f = Fixture::new().with_error(status, "test");
            let result = f.validate();
            assert!(result.is_err(), "status {} should be rejected", status);
            assert!(result.unwrap_err().contains("400-599"));
        }
    }

    #[test]
    fn should_accept_error_status_codes() {
        for status in [400, 401, 403, 404, 429, 500, 502, 503, 529] {
            let mut f = Fixture::new().with_error(status, "test");
            assert!(f.validate().is_ok(), "status {} should be accepted", status);
        }
    }

    #[test]
    fn should_reject_empty_user_message_substring() {
        let mut f = Fixture::new()
            .match_user_message("")
            .respond_with_content("ok");
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
    }

    #[test]
    fn should_reject_empty_model_substring() {
        let mut f = Fixture::new().match_model("").respond_with_content("ok");
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
    }

    #[test]
    fn should_reject_empty_user_message_regex() {
        let mut f = Fixture::new().respond_with_content("ok");
        let m = f.match_rule.get_or_insert_with(FixtureMatch::default);
        m.user_message = Some(StringMatch::regex(""));
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("regex must not be empty"));
    }

    #[test]
    fn should_reject_empty_model_regex() {
        let mut f = Fixture::new().respond_with_content("ok");
        let m = f.match_rule.get_or_insert_with(FixtureMatch::default);
        m.model = Some(StringMatch::regex(""));
        let result = f.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("regex must not be empty"));
    }

    #[test]
    fn should_reject_unknown_yaml_fields() {
        let yaml =
            "fixtures:\n  - match:\n      user_mesage: typo\n    response:\n      content: ok";
        let result: Result<FixtureFile, _> = serde_yaml_ng::from_str(yaml);
        assert!(result.is_err(), "typo field 'user_mesage' must be rejected");
    }

    #[test]
    fn should_reject_unknown_fixture_fields() {
        let yaml = "fixtures:\n  - unknown_field: true\n    response:\n      content: ok";
        let result: Result<FixtureFile, _> = serde_yaml_ng::from_str(yaml);
        assert!(result.is_err(), "unknown fixture field must be rejected");
    }

    #[test]
    fn should_set_stop_reason_via_builder() {
        let f = Fixture::new()
            .respond_with_content("test")
            .with_stop_reason("max_tokens");
        assert_eq!(
            f.response.as_ref().unwrap().stop_reason.as_deref(),
            Some("max_tokens")
        );
    }

    #[test]
    fn should_set_finish_reason_via_builder() {
        let f = Fixture::new()
            .respond_with_content("test")
            .with_finish_reason("length");
        assert_eq!(
            f.response.as_ref().unwrap().finish_reason.as_deref(),
            Some("length")
        );
    }

    #[test]
    fn should_set_stop_reason_on_empty_response() {
        let f = Fixture::new().with_stop_reason("end_turn");
        assert!(f.response.is_some());
        assert_eq!(
            f.response.as_ref().unwrap().stop_reason.as_deref(),
            Some("end_turn")
        );
    }

    #[test]
    fn should_set_finish_reason_on_empty_response() {
        let f = Fixture::new().with_finish_reason("stop");
        assert!(f.response.is_some());
        assert_eq!(
            f.response.as_ref().unwrap().finish_reason.as_deref(),
            Some("stop")
        );
    }
}
