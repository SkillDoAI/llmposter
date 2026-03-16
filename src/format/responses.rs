//! OpenAI Responses API format module.
//!
//! Spec: https://platform.openai.com/docs/api-reference/responses/object
//! Target: latest API version (2025)
//!
//! Builds mock responses matching the OpenAI Responses API shape,
//! including streaming SSE events.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::format::{estimate_tokens, IdGenerator};

// ---------------------------------------------------------------------------
// Response structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesApiResponse {
    pub id: String,
    pub object: String,
    pub status: String,
    pub model: String,
    pub output: Vec<Value>,
    pub usage: ResponsesUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputItem {
    pub id: String,
    #[serde(rename = "type")]
    pub output_type: String,
    pub status: String,
    pub role: String,
    pub content: Vec<OutputContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
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
        usage: ResponsesUsage {
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
        },
    }
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
        .enumerate()
        .map(|(i, (name, arguments))| {
            let args_str = arguments.to_string();
            output_tokens += estimate_tokens(&args_str);
            json!({
                "type": "function_call",
                "id": format!("fc_{}", i + 1),
                "call_id": format!("call_llmposter_{}", i + 1),
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
/// Returns `(event_type, data_json)` pairs matching the Responses API
/// streaming protocol.
pub fn build_stream_events(
    id_gen: &IdGenerator,
    model: &str,
    content: &str,
    chunk_size: usize,
    prompt: &str,
) -> Vec<(String, Value)> {
    let response = build_response(id_gen, model, content, prompt);
    let response_json = serde_json::to_value(&response).unwrap();

    let mut events: Vec<(String, Value)> = Vec::new();

    // 1. response.created — response with status "in_progress" and empty output
    let mut created_json = response_json.clone();
    created_json["type"] = json!("response.created");
    created_json["status"] = json!("in_progress");
    created_json["output"] = json!([]);
    created_json["usage"]["output_tokens"] = json!(0);
    created_json["usage"]["total_tokens"] = created_json["usage"]["input_tokens"].clone();
    events.push(("response.created".to_string(), created_json));

    let item_id = response
        .output
        .first()
        .and_then(|item| item["id"].as_str())
        .unwrap_or("msg_1")
        .to_string();

    // 2. response.output_item.added — empty content initially
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
            }
        }),
    ));

    // 3. response.content_part.added
    events.push((
        "response.content_part.added".to_string(),
        json!({
            "type": "response.content_part.added",
            "output_index": 0,
            "content_index": 0,
            "part": {
                "type": "output_text",
                "text": ""
            }
        }),
    ));

    // 4. response.output_text.delta — one per chunk
    let chunks = crate::stream::chunk_content(content, chunk_size);
    for chunk_text in &chunks {
        events.push((
            "response.output_text.delta".to_string(),
            json!({
                "type": "response.output_text.delta",
                "output_index": 0,
                "content_index": 0,
                "delta": chunk_text,
            }),
        ));
    }

    // 5. response.output_text.done
    events.push((
        "response.output_text.done".to_string(),
        json!({
            "type": "response.output_text.done",
            "output_index": 0,
            "content_index": 0,
            "text": content,
        }),
    ));

    // 5b. response.content_part.done
    events.push((
        "response.content_part.done".to_string(),
        json!({
            "type": "response.content_part.done",
            "output_index": 0,
            "content_index": 0,
            "part": {
                "type": "output_text",
                "text": content,
            }
        }),
    ));

    // 6. response.output_item.done — full item
    let output_item = response.output.first().cloned().unwrap_or(json!({}));
    events.push((
        "response.output_item.done".to_string(),
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": output_item,
        }),
    ));

    // 7. response.completed — full response object
    let mut completed_json = response_json;
    completed_json["type"] = json!("response.completed");
    events.push(("response.completed".to_string(), completed_json));

    // 8. response.done — terminal sentinel event
    events.push((
        "response.done".to_string(),
        json!({"type": "response.done"}),
    ));

    events
}

// ---------------------------------------------------------------------------
// Request extraction
// ---------------------------------------------------------------------------

/// Extract `(model, prompt_text)` from a Responses API request body.
///
/// The `input` field may be a plain string or an array of message objects
/// (`[{"role": "user", "content": "..."}]`). Returns `Err` if `input` is
/// missing or unrecognisable.
pub fn extract_request_info(body: &Value) -> Result<(String, String), String> {
    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("Missing or empty 'model' field in request")?
        .to_string();

    let input = body.get("input").ok_or("missing `input` field")?;

    let prompt = if let Some(s) = input.as_str() {
        if s.is_empty() {
            return Err("empty `input` string".to_string());
        }
        s.to_string()
    } else if let Some(arr) = input.as_array() {
        // Find last user message; content can be a string or array of content parts
        let user_msg = arr
            .iter()
            .rev()
            .find(|msg| msg.get("role").and_then(|r| r.as_str()) == Some("user"));

        let content = user_msg
            .and_then(|msg| msg.get("content"))
            .ok_or_else(|| "No user message found in 'input'".to_string())?;

        let text = if let Some(s) = content.as_str() {
            s.to_string()
        } else if let Some(parts) = content.as_array() {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            return Err("Unrecognized content format in user message".to_string());
        };

        if text.is_empty() {
            return Err("No text content in user message".to_string());
        }
        text
    } else {
        return Err("invalid `input` field: expected string or array".to_string());
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
    fn extract_request_info_missing_input_is_error() {
        let body = json!({"model": "gpt-4o"});

        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "missing `input` field");
    }

    #[test]
    fn build_stream_events_sequence() {
        let gen = id_gen();
        let events = build_stream_events(&gen, "gpt-4o", "Hello world", 5, "Hi");

        let types: Vec<&str> = events.iter().map(|(t, _)| t.as_str()).collect();

        // First three are always the same preamble.
        assert_eq!(types[0], "response.created");
        assert_eq!(types[1], "response.output_item.added");
        assert_eq!(types[2], "response.content_part.added");

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

        // Tail events
        let tail = &types[types.len() - 5..];
        assert_eq!(tail[0], "response.output_text.done");
        assert_eq!(tail[1], "response.content_part.done");
        assert_eq!(tail[2], "response.output_item.done");
        assert_eq!(tail[3], "response.completed");
        assert_eq!(tail[4], "response.done");
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

        // First tool call
        assert_eq!(resp.output[0]["type"], "function_call");
        assert_eq!(resp.output[0]["name"], "get_weather");
        assert_eq!(resp.output[0]["id"], "fc_1");
        assert_eq!(resp.output[0]["call_id"], "call_llmposter_1");
        assert_eq!(resp.output[0]["status"], "completed");
        // arguments is a JSON string
        let args: Value =
            serde_json::from_str(resp.output[0]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["location"], "NYC");

        // Second tool call
        assert_eq!(resp.output[1]["type"], "function_call");
        assert_eq!(resp.output[1]["name"], "get_time");
        assert_eq!(resp.output[1]["id"], "fc_2");
        assert_eq!(resp.output[1]["call_id"], "call_llmposter_2");

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
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "empty `input` string");
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
    fn extract_request_info_array_no_user_message_is_error() {
        let body = json!({
            "model": "gpt-4o",
            "input": [
                {"role": "system", "content": "Be helpful."}
            ]
        });
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No user message"));
    }

    #[test]
    fn extract_request_info_array_content_parts() {
        let body = json!({
            "model": "gpt-4o",
            "input": [
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "Part one"},
                        {"type": "image_url", "url": "http://example.com"},
                        {"type": "text", "text": "Part two"}
                    ]
                }
            ]
        });
        let (model, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(model, "gpt-4o");
        assert_eq!(prompt, "Part one\nPart two");
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
