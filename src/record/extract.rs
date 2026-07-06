//! Per-provider response extraction: turn a real upstream JSON response
//! into a [`RecordedFixture`].
//!
//! Value-path based on purpose — llmposter's typed response structs
//! reject real-API variants (e.g. Anthropic `thinking` blocks, extra
//! Gemini usage fields), so extraction walks the raw `serde_json::Value`
//! and takes only what the fixture schema can replay.

use crate::format::Provider;

use super::{
    RecordedFixture, RecordedMatch, RecordedResponse, RecordedToolCall, RECORDED_PRIORITY,
};

/// Disambiguates OpenAI-provider endpoints that share `Provider::OpenAI`
/// but have different response shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenAiEndpoint {
    Chat,
    Completions,
    Embeddings,
}

impl OpenAiEndpoint {
    /// Derive the endpoint from the forwarded request path. Anything
    /// other than the two special OpenAI paths maps to `Chat` — non-OpenAI
    /// providers never consult the value. A new OpenAI-provider route
    /// must add an arm here or it silently falls through to the Chat
    /// extractor and never records.
    pub(crate) fn from_path(path: &str) -> Self {
        match path {
            "/v1/completions" => Self::Completions,
            "/v1/embeddings" => Self::Embeddings,
            _ => Self::Chat,
        }
    }
}

/// Dispatch to the right extractor. `endpoint` discriminates the OpenAI
/// endpoints that share `Provider::OpenAI` (chat completions, legacy
/// completions, embeddings); other providers ignore it.
pub(crate) fn extract_for(
    provider: Provider,
    endpoint: OpenAiEndpoint,
    body: &serde_json::Value,
    model: &str,
    user_message: &str,
) -> Option<RecordedFixture> {
    match provider {
        Provider::OpenAI => match endpoint {
            OpenAiEndpoint::Chat => extract_openai(body, model, user_message),
            OpenAiEndpoint::Completions => extract_completions(body, model, user_message),
            OpenAiEndpoint::Embeddings => extract_embeddings(body, model, user_message),
        },
        Provider::Anthropic => extract_anthropic(body, model, user_message),
        Provider::Gemini => extract_gemini(body, model, user_message),
        Provider::Responses => extract_responses(body, model, user_message),
    }
}

/// OpenAI Chat Completions: `choices[0].message` carries either text
/// `content` or `tool_calls[*].function.{name,arguments}` (arguments as a
/// JSON-encoded string).
pub(crate) fn extract_openai(
    body: &serde_json::Value,
    model: &str,
    user_message: &str,
) -> Option<RecordedFixture> {
    let choice = body.get("choices")?.get(0)?;
    let message = choice.get("message")?;
    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
        for call in calls {
            let function = call.get("function");
            let Some(name) = function
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
            else {
                continue;
            };
            tool_calls.push(RecordedToolCall {
                name: name.to_string(),
                arguments: string_args_or_warn(name, function.and_then(|f| f.get("arguments"))),
            });
        }
    }
    let content = message
        .get("content")
        .and_then(|c| c.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let finish_reason = choice
        .get("finish_reason")
        .and_then(|r| r.as_str())
        .map(str::to_string);
    finish(
        Provider::OpenAI,
        model,
        user_message,
        content,
        tool_calls,
        None,
        finish_reason,
    )
}

/// Anthropic Messages: `content` is a block list — `text` blocks are
/// concatenated, `tool_use` blocks become tool calls, and everything
/// else (`thinking`, `redacted_thinking`, ...) is skipped.
pub(crate) fn extract_anthropic(
    body: &serde_json::Value,
    model: &str,
    user_message: &str,
) -> Option<RecordedFixture> {
    let blocks = body.get("content")?.as_array()?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in blocks {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    text.push_str(t);
                }
            }
            Some("tool_use") => {
                let Some(name) = block.get("name").and_then(|n| n.as_str()) else {
                    continue;
                };
                tool_calls.push(RecordedToolCall {
                    name: name.to_string(),
                    arguments: block
                        .get("input")
                        .and_then(parse_args)
                        .unwrap_or_else(|| serde_json::json!({})),
                });
            }
            _ => {} // thinking / redacted_thinking / unknown — skip
        }
    }
    let stop_reason = body
        .get("stop_reason")
        .and_then(|r| r.as_str())
        .map(str::to_string);
    finish(
        Provider::Anthropic,
        model,
        user_message,
        Some(text).filter(|t| !t.is_empty()),
        tool_calls,
        stop_reason,
        None,
    )
}

/// Gemini generateContent: `candidates[0].content.parts` — `text` parts
/// are concatenated, `functionCall` parts become tool calls.
pub(crate) fn extract_gemini(
    body: &serde_json::Value,
    model: &str,
    user_message: &str,
) -> Option<RecordedFixture> {
    let parts = body
        .get("candidates")?
        .get(0)?
        .get("content")?
        .get("parts")?
        .as_array()?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for part in parts {
        if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
            text.push_str(t);
        }
        if let Some(call) = part.get("functionCall") {
            let Some(name) = call.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            tool_calls.push(RecordedToolCall {
                name: name.to_string(),
                arguments: call
                    .get("args")
                    .and_then(parse_args)
                    .unwrap_or_else(|| serde_json::json!({})),
            });
        }
    }
    finish(
        Provider::Gemini,
        model,
        user_message,
        Some(text).filter(|t| !t.is_empty()),
        tool_calls,
        None,
        None,
    )
}

/// OpenAI Responses API: `output` is an item list — `message` items
/// carry `content[*].output_text` text, `function_call` items carry
/// `{name, arguments}` (arguments as a JSON-encoded string). Other item
/// types (`reasoning`, `web_search_call`, ...) are skipped.
pub(crate) fn extract_responses(
    body: &serde_json::Value,
    model: &str,
    user_message: &str,
) -> Option<RecordedFixture> {
    let output = body.get("output")?.as_array()?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for item in output {
        match item.get("type").and_then(|t| t.as_str()) {
            Some("message") => {
                let Some(content) = item.get("content").and_then(|c| c.as_array()) else {
                    continue;
                };
                for part in content {
                    if part.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                        if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                            text.push_str(t);
                        }
                    }
                }
            }
            Some("function_call") => {
                let Some(name) = item.get("name").and_then(|n| n.as_str()) else {
                    continue;
                };
                tool_calls.push(RecordedToolCall {
                    name: name.to_string(),
                    arguments: string_args_or_warn(name, item.get("arguments")),
                });
            }
            _ => {} // reasoning / tool outputs / unknown — skip
        }
    }
    finish(
        Provider::Responses,
        model,
        user_message,
        Some(text).filter(|t| !t.is_empty()),
        tool_calls,
        None,
        None,
    )
}

/// Legacy text completions: `choices[0].text`, preserved verbatim
/// (leading whitespace is significant for completion continuations).
pub(crate) fn extract_completions(
    body: &serde_json::Value,
    model: &str,
    user_message: &str,
) -> Option<RecordedFixture> {
    let choice = body.get("choices")?.get(0)?;
    let content = choice
        .get("text")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let finish_reason = choice
        .get("finish_reason")
        .and_then(|r| r.as_str())
        .map(str::to_string);
    finish(
        Provider::OpenAI,
        model,
        user_message,
        content,
        Vec::new(),
        None,
        finish_reason,
    )
}

/// OpenAI Embeddings: `data` must hold EXACTLY one entry — the fixture
/// schema stores a single vector, so multi-input responses (one entry
/// per input) pass through unrecorded. Every vector value must be a JSON
/// number; anything else (e.g. base64-encoded floats from
/// `encoding_format: "base64"`) yields `None`.
pub(crate) fn extract_embeddings(
    body: &serde_json::Value,
    model: &str,
    user_message: &str,
) -> Option<RecordedFixture> {
    let data = body.get("data")?.as_array()?;
    if data.len() != 1 {
        return None;
    }
    let values = data[0].get("embedding")?.as_array()?;
    let embedding = values
        .iter()
        .map(serde_json::Value::as_f64)
        .collect::<Option<Vec<f64>>>()?;
    let mut rec = base(Provider::OpenAI, model, user_message);
    rec.response.embedding = Some(embedding);
    Some(rec)
}

/// Resolve OpenAI-family tool-call arguments, which arrive as a
/// JSON-encoded STRING on the wire. A string that fails to parse as a
/// JSON object records `{}` and warns on stderr — the tool-call NAME and
/// the fact only, never the argument content (it could contain secrets).
/// Absent arguments and already-an-object arguments stay silent (absent
/// args are legitimate, e.g. Gemini zero-arg function calls).
fn string_args_or_warn(name: &str, args: Option<&serde_json::Value>) -> serde_json::Value {
    let Some(v) = args else {
        return serde_json::json!({});
    };
    match parse_args(v) {
        Some(parsed) => parsed,
        None => {
            if v.is_string() {
                eprintln!(
                    "[llmposter] record mode: tool call '{}' had unparseable arguments — \
                     recorded as {{}}",
                    name
                );
            }
            serde_json::json!({})
        }
    }
}

/// Assemble the final fixture. Tool calls WIN over text when both are
/// present — the fixture schema is content XOR tool_calls, and a
/// response that called tools replays most faithfully as a tool call.
/// Returns `None` when neither is present (nothing replayable).
fn finish(
    provider: Provider,
    model: &str,
    user_message: &str,
    content: Option<String>,
    tool_calls: Vec<RecordedToolCall>,
    stop_reason: Option<String>,
    finish_reason: Option<String>,
) -> Option<RecordedFixture> {
    let mut rec = base(provider, model, user_message);
    if !tool_calls.is_empty() {
        rec.response.tool_calls = Some(tool_calls);
    } else if content.is_some() {
        rec.response.content = content;
    } else {
        return None;
    }
    rec.response.stop_reason = stop_reason;
    rec.response.finish_reason = finish_reason;
    Some(rec)
}

/// Shared fixture skeleton. Takes the [`Provider`] ENUM so a wrong
/// provider string is unrepresentable — `RecordedFixture.provider`
/// stays `&'static str` fed only from `Provider::as_str()`.
fn base(provider: Provider, model: &str, user_message: &str) -> RecordedFixture {
    RecordedFixture {
        match_rule: RecordedMatch {
            user_message: user_message.to_string(),
            model: model.to_string(),
        },
        provider: provider.as_str(),
        priority: RECORDED_PRIORITY,
        response: RecordedResponse::default(),
    }
}

/// Normalize tool-call arguments to a JSON object: OpenAI-family APIs
/// send them as a JSON-encoded STRING, Anthropic/Gemini as an object.
/// Strings that don't parse to an object (and non-string/non-object
/// values) yield `None`; callers fall back to an empty object so a
/// tool call with mangled arguments still records its name.
fn parse_args(v: &serde_json::Value) -> Option<serde_json::Value> {
    match v {
        serde_json::Value::String(s) => serde_json::from_str::<serde_json::Value>(s)
            .ok()
            .filter(|parsed| parsed.is_object()),
        serde_json::Value::Object(_) => Some(v.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn should_extract_openai_text_response() {
        let body = json!({
            "id": "chatcmpl-AIdRnXqrjJXgTom1yzM6ZUX4A9CqB",
            "object": "chat.completion",
            "created": 1728933352,
            "model": "gpt-4o-2024-08-06",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "hello!",
                    "refusal": null
                },
                "logprobs": null,
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 19,
                "completion_tokens": 10,
                "total_tokens": 29,
                "completion_tokens_details": { "reasoning_tokens": 0 }
            },
            "system_fingerprint": "fp_6b68a8204b"
        });
        let rec = extract_openai(&body, "gpt-4o", "say hello").unwrap();
        assert_eq!(rec.provider, "openai");
        assert_eq!(rec.match_rule.model, "gpt-4o");
        assert_eq!(rec.match_rule.user_message, "say hello");
        assert_eq!(rec.response.content.as_deref(), Some("hello!"));
        assert_eq!(rec.response.finish_reason.as_deref(), Some("stop"));
        assert!(rec.response.tool_calls.is_none());
    }

    #[test]
    fn should_extract_openai_tool_calls_with_string_arguments() {
        let body = json!({
            "id": "chatcmpl-abc123",
            "object": "chat.completion",
            "created": 1699896916,
            "model": "gpt-4o-2024-08-06",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc123",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"city\":\"SF\"}"
                        }
                    }]
                },
                "logprobs": null,
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 82, "completion_tokens": 17, "total_tokens": 99 }
        });
        let rec = extract_openai(&body, "gpt-4o", "weather?").unwrap();
        let calls = rec.response.tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments["city"], "SF");
        assert!(
            rec.response.content.is_none(),
            "tool calls win; content stays None"
        );
    }

    #[test]
    fn should_extract_anthropic_skipping_thinking_blocks_and_preferring_tools() {
        let body = json!({
            "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-6",
            "content": [
                {
                    "type": "thinking",
                    "thinking": "The user wants a lookup; I should call the tool.",
                    "signature": "EqQBCgIYAhIM1gbcDa9GJwZA2b3hGgxBdjrkzLoky3dl1pk"
                },
                { "type": "text", "text": "Let me check." },
                {
                    "type": "tool_use",
                    "id": "toolu_01A09q90qw90lq917835lq9",
                    "name": "lookup",
                    "input": { "q": "x" }
                }
            ],
            "stop_reason": "tool_use",
            "stop_sequence": null,
            "usage": { "input_tokens": 599, "output_tokens": 152 }
        });
        let rec = extract_anthropic(&body, "claude-sonnet-4-6", "look up x").unwrap();
        assert_eq!(rec.provider, "anthropic");
        let calls = rec.response.tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "lookup");
        assert_eq!(calls[0].arguments["q"], "x");
        assert!(rec.response.content.is_none(), "tools win over text");
        assert_eq!(rec.response.stop_reason.as_deref(), Some("tool_use"));
    }

    #[test]
    fn should_extract_anthropic_text_only() {
        let body = json!({
            "id": "msg_013Zva2CMHLNnXjNJJKqJ2EF",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-6",
            "content": [
                { "type": "text", "text": "part one " },
                { "type": "text", "text": "part two" }
            ],
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": { "input_tokens": 10, "output_tokens": 25 }
        });
        let rec = extract_anthropic(&body, "claude-sonnet-4-6", "two parts").unwrap();
        assert_eq!(rec.response.content.as_deref(), Some("part one part two"));
        assert!(rec.response.tool_calls.is_none());
        assert_eq!(rec.response.stop_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn should_extract_gemini_with_extra_real_fields() {
        let body = json!({
            "candidates": [{
                "content": {
                    "parts": [{ "text": "answer" }],
                    "role": "model"
                },
                "finishReason": "STOP",
                "avgLogprobs": -0.003405,
                "index": 0
            }],
            "usageMetadata": {
                "promptTokenCount": 4,
                "candidatesTokenCount": 5,
                "totalTokenCount": 9,
                "promptTokensDetails": [{ "modality": "TEXT", "tokenCount": 4 }]
            },
            "modelVersion": "gemini-2.5-flash",
            "responseId": "wp5rZ_KNGpyO2PgPn5uD8Ac"
        });
        let rec = extract_gemini(&body, "gemini-2.5-flash", "ask").unwrap();
        assert_eq!(rec.provider, "gemini");
        assert_eq!(rec.response.content.as_deref(), Some("answer"));
        assert!(rec.response.tool_calls.is_none());
    }

    #[test]
    fn should_extract_gemini_function_call() {
        let body = json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "get_weather",
                            "args": { "city": "SF" }
                        }
                    }],
                    "role": "model"
                },
                "finishReason": "STOP",
                "index": 0
            }],
            "usageMetadata": { "promptTokenCount": 8, "totalTokenCount": 12 }
        });
        let rec = extract_gemini(&body, "gemini-2.5-flash", "weather?").unwrap();
        let calls = rec.response.tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments["city"], "SF");
        assert!(rec.response.content.is_none());
    }

    #[test]
    fn should_extract_responses_api_text_and_function_call() {
        let text_body = json!({
            "id": "resp_67ccd2bed1ec8190b14f964abc054267",
            "object": "response",
            "created_at": 1741476542,
            "status": "completed",
            "model": "gpt-4o-2024-08-06",
            "output": [
                {
                    "type": "reasoning",
                    "id": "rs_67ccd2bf17f0819081ff3bb2cf6508e6",
                    "summary": []
                },
                {
                    "type": "message",
                    "id": "msg_67ccd2bf17f0819081ff3bb2cf6508e6",
                    "status": "completed",
                    "role": "assistant",
                    "content": [{
                        "type": "output_text",
                        "text": "resp text",
                        "annotations": []
                    }]
                }
            ]
        });
        let rec = extract_responses(&text_body, "gpt-4o", "respond").unwrap();
        assert_eq!(rec.provider, "responses");
        assert_eq!(rec.response.content.as_deref(), Some("resp text"));
        assert!(rec.response.tool_calls.is_none());

        let tool_body = json!({
            "id": "resp_67ca09c5efe0819096d0511c92b8c890",
            "object": "response",
            "created_at": 1741294021,
            "status": "completed",
            "model": "gpt-4o-2024-08-06",
            "output": [{
                "type": "function_call",
                "id": "fc_67ca09c6bedc8190a7abfec07b1a1332",
                "call_id": "call_unLAR8MvFNptuiZK6K6HCy5k",
                "name": "get_weather",
                "arguments": "{\"city\":\"SF\"}",
                "status": "completed"
            }]
        });
        let rec = extract_responses(&tool_body, "gpt-4o", "weather?").unwrap();
        let calls = rec.response.tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(
            calls[0].arguments["city"], "SF",
            "string args are PARSED to an object"
        );
        assert!(rec.response.content.is_none());
    }

    #[test]
    fn should_extract_completions_text() {
        let body = json!({
            "id": "cmpl-uqkvlQyYK7bGYrRHQ0eXlWi7",
            "object": "text_completion",
            "created": 1589478378,
            "model": "davinci-002",
            "choices": [{
                "text": " legacy completion",
                "index": 0,
                "logprobs": null,
                "finish_reason": "length"
            }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 7, "total_tokens": 12 }
        });
        let rec = extract_completions(&body, "davinci-002", "legacy prompt").unwrap();
        assert_eq!(rec.provider, "openai");
        assert_eq!(
            rec.response.content.as_deref(),
            Some(" legacy completion"),
            "content preserved verbatim, including leading whitespace"
        );
        assert_eq!(rec.response.finish_reason.as_deref(), Some("length"));
    }

    #[test]
    fn should_record_empty_args_when_tool_arguments_unparseable() {
        // OpenAI: string arguments that fail to parse as a JSON object
        // fall back to {} (with a stderr warning naming the tool call).
        let body = json!({
            "id": "chatcmpl-badargs",
            "object": "chat.completion",
            "created": 1699896916,
            "model": "gpt-4o-2024-08-06",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_badargs",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "not valid json {"
                        }
                    }]
                },
                "logprobs": null,
                "finish_reason": "tool_calls"
            }]
        });
        let rec = extract_openai(&body, "gpt-4o", "weather?").unwrap();
        let calls = rec.response.tool_calls.as_ref().unwrap();
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments, json!({}));

        // Gemini: ABSENT args are legitimate (zero-arg function call) —
        // {} is recorded silently, no warning.
        let gemini_body = json!({
            "candidates": [{
                "content": {
                    "parts": [{ "functionCall": { "name": "refresh" } }],
                    "role": "model"
                },
                "finishReason": "STOP",
                "index": 0
            }]
        });
        let rec = extract_gemini(&gemini_body, "gemini-2.5-flash", "refresh it").unwrap();
        let calls = rec.response.tool_calls.as_ref().unwrap();
        assert_eq!(calls[0].name, "refresh");
        assert_eq!(calls[0].arguments, json!({}));
    }

    #[test]
    fn should_return_none_for_unextractable_real_api_variants() {
        // OpenAI: array-of-parts content (annotated-content variant) is
        // not a plain string — nothing the fixture schema can replay.
        let openai_parts = json!({
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "chunked" }]
                },
                "finish_reason": "stop"
            }]
        });
        assert!(extract_openai(&openai_parts, "m", "u").is_none());

        // Anthropic: thinking-only content — no text, no tools.
        let thinking_only = json!({
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "thinking",
                "thinking": "hmm, tricky",
                "signature": "sig"
            }],
            "stop_reason": "end_turn"
        });
        assert!(extract_anthropic(&thinking_only, "m", "u").is_none());

        // Gemini: SAFETY-blocked candidate has NO content key at all.
        let safety_blocked = json!({
            "candidates": [{
                "finishReason": "SAFETY",
                "safetyRatings": [{
                    "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
                    "probability": "HIGH"
                }],
                "index": 0
            }]
        });
        assert!(extract_gemini(&safety_blocked, "m", "u").is_none());

        // Responses: message item with an EMPTY content array.
        let empty_message = json!({
            "object": "response",
            "output": [{
                "type": "message",
                "id": "msg_1",
                "status": "completed",
                "role": "assistant",
                "content": []
            }]
        });
        assert!(extract_responses(&empty_message, "m", "u").is_none());
    }

    #[test]
    fn should_extract_single_embedding_and_skip_multi() {
        let single = json!({
            "object": "list",
            "data": [{
                "object": "embedding",
                "index": 0,
                "embedding": [0.1, -0.2, 0.3]
            }],
            "model": "text-embedding-3-small",
            "usage": { "prompt_tokens": 2, "total_tokens": 2 }
        });
        let rec = extract_embeddings(&single, "text-embedding-3-small", "some text").unwrap();
        assert_eq!(rec.provider, "openai");
        assert_eq!(rec.match_rule.model, "text-embedding-3-small");
        assert_eq!(rec.match_rule.user_message, "some text");
        assert_eq!(
            rec.response.embedding.as_ref().unwrap(),
            &vec![0.1, -0.2, 0.3]
        );
        assert!(rec.response.content.is_none());
        assert!(rec.response.tool_calls.is_none());

        // Multi-input responses carry one data entry per input — the
        // fixture schema holds ONE vector, so these pass through unrecorded.
        let multi = json!({ "data": [{ "embedding": [0.1] }, { "embedding": [0.2] }] });
        assert!(extract_embeddings(&multi, "m", "q").is_none());
    }

    #[test]
    fn should_return_none_for_malformed_embedding_data() {
        // No data key at all.
        assert!(extract_embeddings(&json!({ "object": "list" }), "m", "q").is_none());
        // Empty data array.
        assert!(extract_embeddings(&json!({ "data": [] }), "m", "q").is_none());
        // Entry without an embedding key.
        assert!(extract_embeddings(&json!({ "data": [{ "index": 0 }] }), "m", "q").is_none());
        // Non-numeric vector values (e.g. base64-encoded floats).
        let base64 = json!({ "data": [{ "embedding": "AACAPwAAAEA=" }] });
        assert!(extract_embeddings(&base64, "m", "q").is_none());
        let mixed = json!({ "data": [{ "embedding": [0.1, "x"] }] });
        assert!(extract_embeddings(&mixed, "m", "q").is_none());
    }

    #[test]
    fn should_map_paths_to_openai_endpoints() {
        assert_eq!(
            OpenAiEndpoint::from_path("/v1/completions"),
            OpenAiEndpoint::Completions
        );
        assert_eq!(
            OpenAiEndpoint::from_path("/v1/embeddings"),
            OpenAiEndpoint::Embeddings
        );
        assert_eq!(
            OpenAiEndpoint::from_path("/v1/chat/completions"),
            OpenAiEndpoint::Chat
        );
        // Non-OpenAI paths never consult the value — Chat is a harmless default.
        assert_eq!(
            OpenAiEndpoint::from_path("/v1/messages"),
            OpenAiEndpoint::Chat
        );
    }

    #[test]
    fn should_return_none_when_nothing_extractable() {
        let empty_choices = json!({ "object": "chat.completion", "choices": [] });
        assert!(extract_openai(&empty_choices, "m", "u").is_none());
        assert!(extract_completions(&empty_choices, "m", "u").is_none());

        let empty_content = json!({ "type": "message", "content": [] });
        assert!(extract_anthropic(&empty_content, "m", "u").is_none());

        let empty_candidates = json!({ "candidates": [] });
        assert!(extract_gemini(&empty_candidates, "m", "u").is_none());

        let empty_output = json!({ "object": "response", "output": [] });
        assert!(extract_responses(&empty_output, "m", "u").is_none());
    }
}
