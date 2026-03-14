//! OpenAI Responses API format module.
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
    pub model: String,
    pub output: Vec<OutputItem>,
    pub usage: ResponsesUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputItem {
    #[serde(rename = "type")]
    pub output_type: String,
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

    ResponsesApiResponse {
        id: id_gen.next_responses(),
        object: "response".to_string(),
        model: model.to_string(),
        output: vec![OutputItem {
            output_type: "message".to_string(),
            role: "assistant".to_string(),
            content: vec![OutputContent {
                content_type: "output_text".to_string(),
                text: content.to_string(),
            }],
        }],
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

    let output: Vec<OutputItem> = tool_calls
        .iter()
        .map(|(name, arguments)| {
            let args_str = arguments.to_string();
            output_tokens += estimate_tokens(&args_str);
            OutputItem {
                output_type: "function_call".to_string(),
                role: "assistant".to_string(),
                content: vec![OutputContent {
                    content_type: "function_call".to_string(),
                    text: json!({
                        "name": name,
                        "arguments": args_str,
                    })
                    .to_string(),
                }],
            }
        })
        .collect();

    ResponsesApiResponse {
        id: id_gen.next_responses(),
        object: "response".to_string(),
        model: model.to_string(),
        output,
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

    // 1. response.created — full response object
    events.push(("response.created".to_string(), response_json.clone()));

    // 2. response.output_item.added
    let output_item = &response.output[0];
    let output_item_json = serde_json::to_value(output_item).unwrap();
    events.push((
        "response.output_item.added".to_string(),
        output_item_json.clone(),
    ));

    // 3. response.content_part.added
    let content_part = &output_item.content[0];
    let content_part_json = serde_json::to_value(content_part).unwrap();
    events.push((
        "response.content_part.added".to_string(),
        content_part_json,
    ));

    // 4. response.output_text.delta — one per chunk
    let chars: Vec<char> = content.chars().collect();
    for chunk in chars.chunks(chunk_size) {
        let chunk_text: String = chunk.iter().collect();
        events.push((
            "response.output_text.delta".to_string(),
            json!({
                "type": "response.output_text.delta",
                "delta": chunk_text,
            }),
        ));
    }

    // 5. response.output_text.done
    events.push((
        "response.output_text.done".to_string(),
        json!({
            "type": "response.output_text.done",
            "text": content,
        }),
    ));

    // 6. response.output_item.done
    events.push((
        "response.output_item.done".to_string(),
        output_item_json,
    ));

    // 7. response.completed — full response object
    events.push(("response.completed".to_string(), response_json));

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
        .unwrap_or("unknown")
        .to_string();

    let input = body.get("input").ok_or("missing `input` field")?;

    let prompt = if let Some(s) = input.as_str() {
        s.to_string()
    } else if let Some(arr) = input.as_array() {
        // Take the last user message's content.
        arr.iter()
            .rev()
            .find(|msg| msg.get("role").and_then(|r| r.as_str()) == Some("user"))
            .and_then(|msg| msg.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string()
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
        assert_eq!(item.output_type, "message");
        assert_eq!(item.role, "assistant");
        assert_eq!(item.content.len(), 1);

        let part = &item.content[0];
        assert_eq!(part.content_type, "output_text");
        assert_eq!(part.text, "world");
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
        let tail = &types[types.len() - 3..];
        assert_eq!(tail[0], "response.output_text.done");
        assert_eq!(tail[1], "response.output_item.done");
        assert_eq!(tail[2], "response.completed");
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
        assert_eq!(deserialized.output[0].content[0].text, "Round-trip test");
        assert_eq!(deserialized.usage.total_tokens, resp.usage.total_tokens);

        // Verify serde rename works: JSON must contain "type", not "output_type".
        let raw: Value = serde_json::from_str(&json_str).unwrap();
        let item = &raw["output"][0];
        assert!(item.get("type").is_some());
        assert!(item.get("output_type").is_none());
    }
}
