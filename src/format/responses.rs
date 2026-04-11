//! OpenAI Responses API format module.
//!
//! Spec: https://platform.openai.com/docs/api-reference/responses/object
//! Target: latest API version (2025)
//!
//! Builds mock responses matching the OpenAI Responses API shape,
//! including streaming SSE events.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Monotonically incrementing sequence counter for streaming events.
pub(crate) fn next_seq(counter: &mut u64) -> u64 {
    let s = *counter;
    *counter += 1;
    s
}

use crate::format::{estimate_tokens, IdGenerator};

// ---------------------------------------------------------------------------
// Response structs
// ---------------------------------------------------------------------------

/// Full non-streaming OpenAI Responses API response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesApiResponse {
    /// Unique response identifier (e.g. `resp-llmposter-1`).
    pub id: String,
    /// Always `"response"`.
    pub object: String,
    /// Response status (e.g. `"completed"`).
    pub status: String,
    /// Model name echoed back from the request.
    pub model: String,
    /// Output items (messages and/or function calls) as JSON values.
    pub output: Vec<Value>,
    /// Details when the response was incomplete, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete_details: Option<Value>,
    /// Token usage statistics.
    pub usage: ResponsesUsage,
}

/// A message output item within a Responses API response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputItem {
    /// Unique item identifier (e.g. `msg_1`).
    pub id: String,
    /// Always `"message"`. Serialized as `"type"` in JSON.
    #[serde(rename = "type")]
    pub output_type: String,
    /// Item status (e.g. `"completed"`).
    pub status: String,
    /// Always `"assistant"` for output items.
    pub role: String,
    /// Content parts within this output item.
    pub content: Vec<OutputContent>,
}

/// A content part within an output item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputContent {
    /// Content type (e.g. `"output_text"`). Serialized as `"type"` in JSON.
    #[serde(rename = "type")]
    pub content_type: String,
    /// The text content.
    pub text: String,
}

/// Token usage statistics for a Responses API response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesUsage {
    /// Estimated tokens in the input prompt.
    pub input_tokens: u64,
    /// Estimated tokens in the generated output.
    pub output_tokens: u64,
    /// Sum of input and output tokens.
    pub total_tokens: u64,
}

// ---------------------------------------------------------------------------
// Builder functions
// ---------------------------------------------------------------------------

/// Build a complete (non-streaming) Responses API response.
pub fn build_response(
    id_gen: &IdGenerator,
    model: &str,
    content: &str,
    prompt: &str,
) -> ResponsesApiResponse {
    let input_tokens = estimate_tokens(prompt);
    let output_tokens = estimate_tokens(content);

    let (resp_id, counter) = id_gen.next_responses_with_counter();
    let item_id = format!("msg_{}", counter);

    ResponsesApiResponse {
        id: resp_id,
        object: "response".to_string(),
        status: "completed".to_string(),
        model: model.to_string(),
        output: vec![serde_json::to_value(OutputItem {
            id: item_id,
            output_type: "message".to_string(),
            status: "completed".to_string(),
            role: "assistant".to_string(),
            content: vec![OutputContent {
                content_type: "output_text".to_string(),
                text: content.to_string(),
            }],
        })
        .unwrap()],
        incomplete_details: None,
        usage: ResponsesUsage {
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
        },
    }
}

/// Build a Responses API refusal response.
///
/// Delegates to [`build_response`] for the envelope (id, status, usage
/// bookkeeping) and replaces the single output item's content part with
/// `{"type": "refusal", "refusal": "<reason>"}`. Top-level `status`
/// stays `"completed"`, matching how real OpenAI closes a refused
/// response.
pub fn build_refusal_response(
    id_gen: &IdGenerator,
    model: &str,
    reason: &str,
    prompt: &str,
) -> ResponsesApiResponse {
    let mut resp = build_response(id_gen, model, reason, prompt);
    // `build_response` always emits exactly one message output item.
    // Assert the invariant rather than an `if let` that would silently
    // no-op on a shape change — a future refactor that adds multiple
    // outputs should fail loudly here, not leak the text response.
    assert_eq!(
        resp.output.len(),
        1,
        "expected exactly one output in resp.output from build_response"
    );
    resp.output[0]["content"] = json!([{ "type": "refusal", "refusal": reason }]);
    resp
}

/// Build a Responses API response containing function-call output items.
pub fn build_tool_call_response(
    id_gen: &IdGenerator,
    model: &str,
    tool_calls: &[(&str, Value)],
    prompt: &str,
) -> ResponsesApiResponse {
    let input_tokens = estimate_tokens(prompt);
    let mut output_tokens: u64 = 0;

    // Responses API function_call output items are flat objects, not wrapped in message style.
    // We use serde_json::Value to emit the correct shape.
    let output_values: Vec<Value> = tool_calls
        .iter()
        .map(|(name, arguments)| {
            let tc_id = id_gen.next_tool_call_counter();
            let args_str = arguments.to_string();
            output_tokens += estimate_tokens(&args_str);
            json!({
                "type": "function_call",
                "id": format!("fc_{}", tc_id),
                "call_id": format!("call_llmposter_{}", tc_id),
                "status": "completed",
                "name": name,
                "arguments": args_str,
            })
        })
        .collect();

    let resp_id = id_gen.next_responses();

    ResponsesApiResponse {
        id: resp_id,
        object: "response".to_string(),
        status: "completed".to_string(),
        model: model.to_string(),
        output: output_values,
        incomplete_details: None,
        usage: ResponsesUsage {
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
        },
    }
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

/// Build the sequence of SSE events for a streaming Responses API call.
///
/// Returns `(event_type, data_json)` pairs matching the real OpenAI Responses
/// streaming protocol per https://platform.openai.com/docs/api-reference/responses-streaming
///
/// Event sequence for text:
///   response.created → response.in_progress → response.output_item.added →
///   response.content_part.added → response.output_text.delta(s) →
///   response.output_text.done → response.content_part.done →
///   response.output_item.done → response.completed
pub fn build_stream_events(
    id_gen: &IdGenerator,
    model: &str,
    content: &str,
    chunk_size: usize,
    prompt: &str,
) -> Vec<(String, Value)> {
    let response = build_response(id_gen, model, content, prompt);
    let response_json = serde_json::to_value(&response).unwrap();
    let mut seq_counter: u64 = 0;
    let mut events: Vec<(String, Value)> = Vec::new();

    let item_id = response
        .output
        .first()
        .and_then(|item| item["id"].as_str())
        .unwrap_or("msg_1")
        .to_string();

    // Build in_progress response envelope (status: in_progress, empty output)
    let mut in_progress_resp = response_json.clone();
    in_progress_resp["status"] = json!("in_progress");
    in_progress_resp["output"] = json!([]);
    in_progress_resp["usage"]["output_tokens"] = json!(0);
    in_progress_resp["usage"]["total_tokens"] = in_progress_resp["usage"]["input_tokens"].clone();

    // 1. response.created — wraps response in a "response" envelope
    events.push((
        "response.created".to_string(),
        json!({
            "type": "response.created",
            "response": in_progress_resp.clone(),
            "sequence_number": next_seq(&mut seq_counter),
        }),
    ));

    // 2. response.in_progress
    events.push((
        "response.in_progress".to_string(),
        json!({
            "type": "response.in_progress",
            "response": in_progress_resp,
            "sequence_number": next_seq(&mut seq_counter),
        }),
    ));

    // 3. response.output_item.added
    events.push((
        "response.output_item.added".to_string(),
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "type": "message",
                "id": item_id,
                "status": "in_progress",
                "role": "assistant",
                "content": []
            },
            "sequence_number": next_seq(&mut seq_counter),
        }),
    ));

    // 4. response.content_part.added
    events.push((
        "response.content_part.added".to_string(),
        json!({
            "type": "response.content_part.added",
            "item_id": item_id,
            "output_index": 0,
            "content_index": 0,
            "part": { "type": "output_text", "text": "" },
            "sequence_number": next_seq(&mut seq_counter),
        }),
    ));

    // 5. response.output_text.delta — one per chunk
    let chunks = crate::stream::chunk_content(content, chunk_size);
    for chunk_text in &chunks {
        events.push((
            "response.output_text.delta".to_string(),
            json!({
                "type": "response.output_text.delta",
                "item_id": item_id,
                "output_index": 0,
                "content_index": 0,
                "delta": chunk_text,
                "sequence_number": next_seq(&mut seq_counter),
            }),
        ));
    }

    // 6. response.output_text.done
    events.push((
        "response.output_text.done".to_string(),
        json!({
            "type": "response.output_text.done",
            "item_id": item_id,
            "output_index": 0,
            "content_index": 0,
            "text": content,
            "sequence_number": next_seq(&mut seq_counter),
        }),
    ));

    // 7. response.content_part.done
    events.push((
        "response.content_part.done".to_string(),
        json!({
            "type": "response.content_part.done",
            "item_id": item_id,
            "output_index": 0,
            "content_index": 0,
            "part": { "type": "output_text", "text": content },
            "sequence_number": next_seq(&mut seq_counter),
        }),
    ));

    // 8. response.output_item.done — full completed item
    let output_item = response.output.first().cloned().unwrap_or(json!({}));
    events.push((
        "response.output_item.done".to_string(),
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": output_item,
            "sequence_number": next_seq(&mut seq_counter),
        }),
    ));

    // 9. response.completed — wraps final response in envelope
    events.push((
        "response.completed".to_string(),
        json!({
            "type": "response.completed",
            "response": response_json,
            "sequence_number": next_seq(&mut seq_counter),
        }),
    ));

    events
}

// ---------------------------------------------------------------------------
// Request extraction
// ---------------------------------------------------------------------------

/// Extract `(model, prompt_text)` from a Responses API request body.
///
/// The `input` field may be a plain string or an array of message objects
/// (`[{"role": "user", "content": "..."}]`). A missing `input` is valid for
/// continuation requests (e.g. `function_call_output`) and yields an empty
/// prompt string. Returns `Err` if `input` is present but unrecognisable,
/// or if the model field is missing/empty.
pub fn extract_request_info(body: &Value) -> Result<(String, String), String> {
    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("Missing or empty 'model' field in request")?
        .to_string();

    // `input` is optional for continuation requests (function_call_output, etc.)
    let prompt = match body.get("input") {
        None => String::new(),
        Some(input) => {
            if let Some(s) = input.as_str() {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    return Err("blank `input` string".to_string());
                }
                trimmed.to_string()
            } else if let Some(arr) = input.as_array() {
                // Find last user message; content can be a string or array of content parts.
                // Continuation items (function_call_output, etc.) may not have a user role.
                let user_msg = arr
                    .iter()
                    .rev()
                    .find(|msg| msg.get("role").and_then(|r| r.as_str()) == Some("user"));

                if let Some(user_msg) = user_msg {
                    let content = user_msg
                        .get("content")
                        .ok_or_else(|| "User message missing 'content'".to_string())?;

                    let text = if let Some(s) = content.as_str() {
                        s.trim().to_string()
                    } else if let Some(parts) = content.as_array() {
                        // Only `input_text` parts carry prompt text — stray
                        // `text` fields on other part types (input_image,
                        // input_file, etc.) must not leak through.
                        parts
                            .iter()
                            .filter(|p| {
                                p.get("type").and_then(|t| t.as_str()) == Some("input_text")
                            })
                            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                            .collect::<Vec<_>>()
                            .join("\n")
                            .trim()
                            .to_string()
                    } else {
                        return Err("Unrecognized content format in user message".to_string());
                    };

                    if text.is_empty() {
                        return Err("No text content in user message".to_string());
                    }
                    text
                } else {
                    // No user message — continuation request (function_call_output, etc.)
                    String::new()
                }
            } else {
                return Err("invalid `input` field: expected string or array".to_string());
            }
        }
    };

    Ok((model, prompt))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn id_gen() -> IdGenerator {
        IdGenerator::new()
    }

    #[test]
    fn build_response_shape_has_object_response() {
        let gen = id_gen();
        let resp = build_response(&gen, "gpt-4o", "Hello!", "Hi");

        assert_eq!(resp.object, "response");
        assert!(resp.id.starts_with("resp-llmposter-"));
        assert_eq!(resp.model, "gpt-4o");
        assert_eq!(resp.output.len(), 1);
    }

    #[test]
    fn output_text_content_type() {
        let gen = id_gen();
        let resp = build_response(&gen, "gpt-4o", "world", "hello");

        let item = &resp.output[0];
        assert_eq!(item["type"], "message");
        assert_eq!(item["role"], "assistant");
        assert_eq!(item["content"].as_array().unwrap().len(), 1);

        let part = &item["content"][0];
        assert_eq!(part["type"], "output_text");
        assert_eq!(part["text"], "world");
    }

    #[test]
    fn extract_request_info_string_input() {
        let body = json!({
            "model": "gpt-4o",
            "input": "What is Rust?"
        });

        let (model, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(model, "gpt-4o");
        assert_eq!(prompt, "What is Rust?");
    }

    #[test]
    fn extract_request_info_array_input() {
        let body = json!({
            "model": "gpt-4o",
            "input": [
                {"role": "system", "content": "Be concise."},
                {"role": "user", "content": "Explain borrowing."}
            ]
        });

        let (model, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(model, "gpt-4o");
        assert_eq!(prompt, "Explain borrowing.");
    }

    #[test]
    fn extract_request_info_missing_input_returns_empty_prompt() {
        let body = json!({"model": "gpt-4o"});
        let (model, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(model, "gpt-4o");
        assert!(prompt.is_empty(), "missing input should yield empty prompt");
    }

    #[test]
    fn build_stream_events_sequence() {
        let gen = id_gen();
        let events = build_stream_events(&gen, "gpt-4o", "Hello world", 5, "Hi");

        let types: Vec<&str> = events.iter().map(|(t, _)| t.as_str()).collect();

        // First four are the preamble (with response.in_progress added per real spec).
        assert_eq!(types[0], "response.created");
        assert_eq!(types[1], "response.in_progress");
        assert_eq!(types[2], "response.output_item.added");
        assert_eq!(types[3], "response.content_part.added");

        // "Hello world" is 11 chars, chunk_size 5 => 3 delta events
        // ("Hello", " worl", "d")
        let delta_count = types
            .iter()
            .filter(|&&t| t == "response.output_text.delta")
            .count();
        assert_eq!(delta_count, 3);

        // Verify delta content
        let deltas: Vec<&str> = events
            .iter()
            .filter(|(t, _)| t == "response.output_text.delta")
            .map(|(_, v)| v["delta"].as_str().unwrap())
            .collect();
        assert_eq!(deltas.join(""), "Hello world");

        // Tail events (no more response.done — terminal is response.completed)
        let tail = &types[types.len() - 4..];
        assert_eq!(tail[0], "response.output_text.done");
        assert_eq!(tail[1], "response.content_part.done");
        assert_eq!(tail[2], "response.output_item.done");
        assert_eq!(tail[3], "response.completed");

        // Verify envelope structure on response.created
        let created = &events[0].1;
        assert!(
            created.get("response").is_some(),
            "response.created must have nested response"
        );
        assert!(
            created.get("sequence_number").is_some(),
            "must have sequence_number"
        );
    }

    #[test]
    fn build_tool_call_response_shape() {
        let gen = id_gen();
        let tool_calls: Vec<(&str, Value)> = vec![
            ("get_weather", json!({"location": "NYC"})),
            ("get_time", json!({"tz": "UTC"})),
        ];
        let resp = build_tool_call_response(&gen, "gpt-4o", &tool_calls, "prompt");

        assert_eq!(resp.object, "response");
        assert_eq!(resp.status, "completed");
        assert_eq!(resp.model, "gpt-4o");
        assert_eq!(resp.output.len(), 2);

        // First tool call — assert prefix format, not exact counter values
        // (counter is shared with other ID methods on IdGenerator)
        assert_eq!(resp.output[0]["type"], "function_call");
        assert_eq!(resp.output[0]["name"], "get_weather");
        assert!(resp.output[0]["id"].as_str().unwrap().starts_with("fc_"));
        assert!(resp.output[0]["call_id"]
            .as_str()
            .unwrap()
            .starts_with("call_llmposter_"));
        assert_eq!(resp.output[0]["status"], "completed");
        // arguments is a JSON string
        let args: Value =
            serde_json::from_str(resp.output[0]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["location"], "NYC");

        // Second tool call — IDs must be different from first
        assert_eq!(resp.output[1]["type"], "function_call");
        assert_eq!(resp.output[1]["name"], "get_time");
        assert!(resp.output[1]["id"].as_str().unwrap().starts_with("fc_"));
        assert!(resp.output[1]["call_id"]
            .as_str()
            .unwrap()
            .starts_with("call_llmposter_"));
        assert_ne!(resp.output[0]["id"], resp.output[1]["id"]);
        assert_ne!(resp.output[0]["call_id"], resp.output[1]["call_id"]);

        // Usage
        assert!(resp.usage.input_tokens > 0);
        assert!(resp.usage.output_tokens > 0);
        assert_eq!(
            resp.usage.total_tokens,
            resp.usage.input_tokens + resp.usage.output_tokens
        );
    }

    #[test]
    fn build_tool_call_response_single() {
        let gen = id_gen();
        let tool_calls: Vec<(&str, Value)> = vec![("search", json!({"q": "rust"}))];
        let resp = build_tool_call_response(&gen, "gpt-4o", &tool_calls, "find info");

        assert_eq!(resp.output.len(), 1);
        assert!(resp.id.starts_with("resp-llmposter-"));
    }

    #[test]
    fn extract_request_info_empty_model_is_error() {
        let body = json!({
            "model": "",
            "input": "hello"
        });
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("model"));
    }

    #[test]
    fn extract_request_info_empty_string_input_is_error() {
        let body = json!({
            "model": "gpt-4o",
            "input": ""
        });
        let err = extract_request_info(&body).unwrap_err();
        assert!(err.contains("blank `input`"), "unexpected: {}", err);
    }

    #[test]
    fn extract_request_info_whitespace_string_input_is_error() {
        let body = json!({
            "model": "gpt-4o",
            "input": "   \n"
        });
        let err = extract_request_info(&body).unwrap_err();
        assert!(err.contains("blank `input`"), "unexpected: {}", err);
    }

    #[test]
    fn extract_request_info_invalid_input_type_is_error() {
        let body = json!({
            "model": "gpt-4o",
            "input": 42
        });
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected string or array"));
    }

    #[test]
    fn extract_request_info_array_no_user_message_returns_empty_prompt() {
        let body = json!({
            "model": "gpt-4o",
            "input": [
                {"role": "system", "content": "Be helpful."}
            ]
        });
        let (model, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(model, "gpt-4o");
        assert!(
            prompt.is_empty(),
            "no user message should yield empty prompt for continuation"
        );
    }

    #[test]
    fn extract_request_info_array_content_parts() {
        // Responses API content parts use `type: "input_text"` (not `text`).
        // Non-text parts must not leak their `text` field through.
        let body = json!({
            "model": "gpt-4o",
            "input": [
                {
                    "role": "user",
                    "content": [
                        {"type": "input_text", "text": "Part one"},
                        {"type": "input_image", "image_url": "http://example.com"},
                        {"type": "input_text", "text": "Part two"}
                    ]
                }
            ]
        });
        let (model, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(model, "gpt-4o");
        assert_eq!(prompt, "Part one\nPart two");
    }

    #[test]
    fn extract_request_info_ignores_stray_text_field_on_non_input_text_parts() {
        // Regression: a non-`input_text` part that happens to have a `text`
        // key (e.g. a hand-written test fixture using the wrong type tag)
        // must not leak its value into the extracted prompt. Only
        // `input_text` parts are honored.
        let body = json!({
            "model": "gpt-4o",
            "input": [
                {
                    "role": "user",
                    "content": [
                        {"type": "input_text", "text": "real prompt"},
                        {"type": "image_url", "text": "LEAKED should be ignored"}
                    ]
                }
            ]
        });
        let (_model, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(prompt, "real prompt");
    }

    #[test]
    fn extract_request_info_rejects_blank_text_in_array_content() {
        let body = json!({
            "model": "gpt-4o",
            "input": [
                {
                    "role": "user",
                    "content": [
                        {"type": "input_text", "text": "   "}
                    ]
                }
            ]
        });
        let err = extract_request_info(&body).unwrap_err();
        assert!(err.contains("No text content"), "unexpected: {}", err);
    }

    #[test]
    fn extract_request_info_unrecognized_content_format_is_error() {
        let body = json!({
            "model": "gpt-4o",
            "input": [
                {
                    "role": "user",
                    "content": 42
                }
            ]
        });
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unrecognized content format"));
    }

    #[test]
    fn extract_request_info_empty_text_in_array_content_is_error() {
        let body = json!({
            "model": "gpt-4o",
            "input": [
                {
                    "role": "user",
                    "content": [
                        {"type": "image_url", "url": "http://example.com"}
                    ]
                }
            ]
        });
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No text content"));
    }

    #[test]
    fn serialization_round_trip() {
        let gen = id_gen();
        let resp = build_response(&gen, "gpt-4o", "Round-trip test", "prompt");

        let json_str = serde_json::to_string(&resp).unwrap();
        let deserialized: ResponsesApiResponse = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.id, resp.id);
        assert_eq!(deserialized.object, "response");
        assert_eq!(deserialized.model, "gpt-4o");
        assert_eq!(
            deserialized.output[0]["content"][0]["text"],
            "Round-trip test"
        );
        assert_eq!(deserialized.usage.total_tokens, resp.usage.total_tokens);

        // Verify serde rename works: JSON must contain "type", not "output_type".
        let raw: Value = serde_json::from_str(&json_str).unwrap();
        let item = &raw["output"][0];
        assert!(item.get("type").is_some());
        assert!(item.get("output_type").is_none());
    }
}
