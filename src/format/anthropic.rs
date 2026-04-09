//! Anthropic Messages API format module.
//!
//! Spec: https://docs.anthropic.com/en/api/messages
//! Streaming: https://docs.anthropic.com/en/api/messages-streaming
//! Target: latest API version (2025)

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::format::{estimate_tokens, IdGenerator};

// ---------------------------------------------------------------------------
// Response structs
// ---------------------------------------------------------------------------

/// Full non-streaming Anthropic Messages API response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessagesResponse {
    /// Unique message identifier (e.g. `msg-llmposter-1`).
    pub id: String,
    /// Always `"message"`. Serialized as `"type"` in JSON.
    #[serde(rename = "type")]
    pub response_type: String,
    /// Always `"assistant"` for responses.
    pub role: String,
    /// Model name echoed back from the request.
    pub model: String,
    /// Ordered content blocks (text and/or tool_use).
    pub content: Vec<ContentBlock>,
    /// Why generation stopped (e.g. `"end_turn"`, `"tool_use"`).
    pub stop_reason: Option<String>,
    /// The stop sequence that triggered the stop, if any.
    pub stop_sequence: Option<String>,
    /// Token usage statistics.
    pub usage: AnthropicUsage,
}

/// A content block within an Anthropic message (text or tool_use).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ContentBlock {
    /// Plain text content block.
    #[serde(rename = "text")]
    Text {
        /// The text content.
        text: String,
        /// Optional citations (reserved for future use).
        #[serde(skip_serializing_if = "Option::is_none")]
        citations: Option<Vec<Value>>,
    },
    /// Tool use request block.
    #[serde(rename = "tool_use")]
    ToolUse {
        /// Unique tool use identifier (e.g. `toolu_llmposter_1`).
        id: String,
        /// Name of the tool to invoke.
        name: String,
        /// Tool input arguments as a JSON object.
        input: Value,
    },
}

/// Token usage statistics for an Anthropic Messages response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnthropicUsage {
    /// Tokens consumed by the input prompt.
    pub input_tokens: u64,
    /// Tokens produced in the output.
    pub output_tokens: u64,
    /// Tokens written to the prompt cache (always 0 in mocks).
    pub cache_creation_input_tokens: u64,
    /// Tokens read from the prompt cache (always 0 in mocks).
    pub cache_read_input_tokens: u64,
}

// ---------------------------------------------------------------------------
// Streaming event structs
// ---------------------------------------------------------------------------

/// SSE `message_start` event — opens the streaming message.
#[derive(Debug, Clone, Serialize)]
pub struct MessageStartEvent {
    /// Always `"message_start"`.
    #[serde(rename = "type")]
    pub event_type: String,
    /// The initial (incomplete) message envelope.
    pub message: MessagesResponse,
}

/// SSE `content_block_start` event — opens a new content block.
#[derive(Debug, Clone, Serialize)]
pub struct ContentBlockStartEvent {
    /// Always `"content_block_start"`.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Zero-based index of the content block.
    pub index: u32,
    /// The initial (empty) content block.
    pub content_block: ContentBlock,
}

/// SSE `content_block_delta` event — incremental text within a block.
#[derive(Debug, Clone, Serialize)]
pub struct ContentBlockDeltaEvent {
    /// Always `"content_block_delta"`.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Zero-based index of the content block being updated.
    pub index: u32,
    /// The delta payload (text fragment).
    pub delta: ContentBlockDelta,
}

/// Delta payload for a content block delta event.
#[derive(Debug, Clone, Serialize)]
pub struct ContentBlockDelta {
    /// Always `"text_delta"` for text content.
    #[serde(rename = "type")]
    pub delta_type: String,
    /// The text fragment for this delta.
    pub text: String,
}

/// SSE `content_block_stop` event — closes a content block.
#[derive(Debug, Clone, Serialize)]
pub struct ContentBlockStopEvent {
    /// Always `"content_block_stop"`.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Zero-based index of the content block that finished.
    pub index: u32,
}

/// SSE `message_delta` event — final message metadata (stop reason, usage).
#[derive(Debug, Clone, Serialize)]
pub struct MessageDeltaEvent {
    /// Always `"message_delta"`.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Stop reason and optional stop sequence.
    pub delta: MessageDelta,
    /// Final accumulated token usage.
    pub usage: MessageDeltaUsage,
}

/// Delta payload within a `message_delta` event.
#[derive(Debug, Clone, Serialize)]
pub struct MessageDelta {
    /// Why generation stopped (e.g. `"end_turn"`).
    pub stop_reason: String,
    /// The stop sequence that triggered the stop, if any.
    pub stop_sequence: Option<String>,
}

/// Token usage reported in the `message_delta` streaming event.
#[derive(Debug, Clone, Serialize)]
pub struct MessageDeltaUsage {
    /// Tokens consumed by the input prompt.
    pub input_tokens: u64,
    /// Tokens produced in the output.
    pub output_tokens: u64,
    /// Tokens written to the prompt cache (always 0 in mocks).
    pub cache_creation_input_tokens: u64,
    /// Tokens read from the prompt cache (always 0 in mocks).
    pub cache_read_input_tokens: u64,
}

/// SSE `message_stop` event — terminates the streaming message.
#[derive(Debug, Clone, Serialize)]
pub struct MessageStopEvent {
    /// Always `"message_stop"`.
    #[serde(rename = "type")]
    pub event_type: String,
}

// ---------------------------------------------------------------------------
// Builder functions
// ---------------------------------------------------------------------------

/// Build a complete (non-streaming) Anthropic Messages text response.
pub fn build_response(
    id_gen: &IdGenerator,
    model: &str,
    content: &str,
    prompt: &str,
    stop_reason: &str,
) -> MessagesResponse {
    let input_tokens = estimate_tokens(prompt);
    let output_tokens = estimate_tokens(content);

    MessagesResponse {
        id: id_gen.next_anthropic(),
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        model: model.to_string(),
        content: vec![ContentBlock::Text {
            text: content.to_string(),
            citations: None,
        }],
        stop_reason: Some(stop_reason.to_string()),
        stop_sequence: None,
        usage: AnthropicUsage {
            input_tokens,
            output_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        },
    }
}

/// Build an Anthropic Messages response containing tool_use content blocks.
pub fn build_tool_use_response(
    id_gen: &IdGenerator,
    model: &str,
    tool_calls: &[(&str, Value)],
    prompt: &str,
) -> MessagesResponse {
    let input_tokens = estimate_tokens(prompt);

    let mut content = Vec::new();
    let mut output_token_estimate: u64 = 0;

    for (name, input) in tool_calls.iter() {
        let tool_id = format!("toolu_llmposter_{}", id_gen.next_tool_call_counter());
        let input_str = input.to_string();
        output_token_estimate += estimate_tokens(&input_str);
        content.push(ContentBlock::ToolUse {
            id: tool_id,
            name: name.to_string(),
            input: input.clone(),
        });
    }

    MessagesResponse {
        id: id_gen.next_anthropic(),
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        model: model.to_string(),
        content,
        stop_reason: Some("tool_use".to_string()),
        stop_sequence: None,
        usage: AnthropicUsage {
            input_tokens,
            output_tokens: output_token_estimate,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        },
    }
}

/// Build the full sequence of SSE events for an Anthropic streaming response.
///
/// Returns `(event_type, data_json)` pairs: ping, message_start,
/// content_block_start, content_block_delta(s), content_block_stop,
/// message_delta, message_stop.
pub fn build_stream_events(
    id_gen: &IdGenerator,
    model: &str,
    content: &str,
    chunk_size: usize,
    prompt: &str,
    stop_reason: &str,
) -> Vec<(String, Value)> {
    let input_tokens = estimate_tokens(prompt);
    let output_tokens = estimate_tokens(content);
    let msg_id = id_gen.next_anthropic();

    let mut events: Vec<(String, Value)> = Vec::new();

    // 0. ping — Anthropic always sends this first
    events.push(("ping".to_string(), serde_json::json!({"type": "ping"})));

    // 1. message_start
    let message_start = MessageStartEvent {
        event_type: "message_start".to_string(),
        message: MessagesResponse {
            id: msg_id,
            response_type: "message".to_string(),
            role: "assistant".to_string(),
            model: model.to_string(),
            content: vec![],
            stop_reason: None,
            stop_sequence: None,
            usage: AnthropicUsage {
                input_tokens,
                output_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        },
    };
    events.push((
        "message_start".to_string(),
        serde_json::to_value(&message_start).unwrap(),
    ));

    // 2. content_block_start
    let block_start = ContentBlockStartEvent {
        event_type: "content_block_start".to_string(),
        index: 0,
        content_block: ContentBlock::Text {
            text: String::new(),
            citations: None,
        },
    };
    events.push((
        "content_block_start".to_string(),
        serde_json::to_value(&block_start).unwrap(),
    ));

    // 3. content_block_delta events (chunked)
    for chunk in crate::stream::chunk_content(content, chunk_size) {
        let delta_event = ContentBlockDeltaEvent {
            event_type: "content_block_delta".to_string(),
            index: 0,
            delta: ContentBlockDelta {
                delta_type: "text_delta".to_string(),
                text: chunk,
            },
        };
        events.push((
            "content_block_delta".to_string(),
            serde_json::to_value(&delta_event).unwrap(),
        ));
    }

    // 4. content_block_stop
    let block_stop = ContentBlockStopEvent {
        event_type: "content_block_stop".to_string(),
        index: 0,
    };
    events.push((
        "content_block_stop".to_string(),
        serde_json::to_value(&block_stop).unwrap(),
    ));

    // 5. message_delta
    let msg_delta = MessageDeltaEvent {
        event_type: "message_delta".to_string(),
        delta: MessageDelta {
            stop_reason: stop_reason.to_string(),
            stop_sequence: None,
        },
        usage: MessageDeltaUsage {
            input_tokens,
            output_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        },
    };
    events.push((
        "message_delta".to_string(),
        serde_json::to_value(&msg_delta).unwrap(),
    ));

    // 6. message_stop
    let msg_stop = MessageStopEvent {
        event_type: "message_stop".to_string(),
    };
    events.push((
        "message_stop".to_string(),
        serde_json::to_value(&msg_stop).unwrap(),
    ));

    events
}

// ---------------------------------------------------------------------------
// Request extraction
// ---------------------------------------------------------------------------

/// Extract `(model, prompt_text)` from an Anthropic Messages request body.
///
/// Handles string content, array content with text blocks, and tool_result
/// follow-up turns (falls back to the prior user message for those).
pub fn extract_request_info(body: &Value) -> Result<(String, String), String> {
    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("Missing or empty 'model' field in request")?
        .to_string();

    // Anthropic requires max_tokens as a positive integer
    match body.get("max_tokens").and_then(|v| v.as_u64()) {
        Some(v) if v > 0 => {}
        _ => return Err("missing or invalid 'max_tokens': must be a positive integer".to_string()),
    }

    let messages = body
        .get("messages")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing messages array".to_string())?;

    // Find the latest user message and extract its text.
    //
    // Fallback rule: A user message that contains ONLY tool_result blocks is a
    // valid multi-turn tool-flow turn. In that case we skip it and look at the
    // prior user message for the prompt.  Any other non-text latest user turn
    // (e.g. image-only) is an error — we must not silently match the wrong
    // fixture.
    let mut prompt: Option<String> = None;
    let mut past_first_user = false;
    for msg in messages.iter().rev() {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role != "user" {
            continue;
        }
        if let Some(content) = msg.get("content") {
            if let Some(s) = content.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    prompt = Some(trimmed.to_string());
                    break;
                }
                // Blank/whitespace string in the latest turn — fail fast, don't
                // silently match against a stale earlier turn.
                if !past_first_user {
                    return Err("Latest user message has blank text content".to_string());
                }
            } else if let Some(arr) = content.as_array() {
                // Check whether all blocks are tool_result (tool-flow follow-up).
                let all_tool_results = !arr.is_empty()
                    && arr.iter().all(|block| {
                        block.get("type").and_then(|v| v.as_str()) == Some("tool_result")
                    });

                if all_tool_results && !past_first_user {
                    // This is the tool-flow follow-up: skip and look earlier.
                    past_first_user = true;
                    continue;
                }

                let texts: Vec<&str> = arr
                    .iter()
                    .filter_map(|block| {
                        let block_type = block.get("type").and_then(|v| v.as_str())?;
                        if block_type == "text" {
                            block.get("text").and_then(|v| v.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                let joined = texts.join("\n");
                let trimmed = joined.trim().to_string();
                if !trimmed.is_empty() {
                    prompt = Some(trimmed);
                    break;
                }
                // Latest user message has array content but no text (image-only, etc.).
                // Do not fall back — return an error.
                if !past_first_user {
                    return Err(
                        "Latest user message has no text content (image-only or unsupported)"
                            .to_string(),
                    );
                }
            } else if !past_first_user {
                // content is present but neither string nor array (null, number, object, etc.)
                return Err("Latest user message has unrecognized content format".to_string());
            }
        } else if !past_first_user {
            // User message with no content field at all
            return Err("Latest user message has no content field".to_string());
        }
        past_first_user = true;
    }

    let prompt = prompt
        .ok_or_else(|| "No user message with text content found in 'messages'".to_string())?;

    Ok((model, prompt))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_id_gen() -> IdGenerator {
        IdGenerator::new()
    }

    #[test]
    fn should_return_message_type_and_assistant_role() {
        let id_gen = test_id_gen();
        let resp = build_response(&id_gen, "claude-sonnet-4-6", "Hello!", "Hi", "end_turn");

        assert_eq!(resp.response_type, "message");
        assert_eq!(resp.role, "assistant");
        assert_eq!(resp.model, "claude-sonnet-4-6");
        assert_eq!(resp.stop_reason.as_deref(), Some("end_turn"));
        assert!(resp.id.starts_with("msg-llmposter-"));
        assert_eq!(resp.content.len(), 1);
        match &resp.content[0] {
            ContentBlock::Text { text, .. } => assert_eq!(text, "Hello!"),
            _ => panic!("expected text content block"),
        }
    }

    #[test]
    fn should_serialize_text_content_block_with_type_tag() {
        let block = ContentBlock::Text {
            text: "hi".to_string(),
            citations: None,
        };
        let val = serde_json::to_value(&block).unwrap();
        assert_eq!(val["type"], "text");
        assert_eq!(val["text"], "hi");
    }

    #[test]
    fn should_serialize_tool_use_content_block_with_type_tag() {
        let block = ContentBlock::ToolUse {
            id: "toolu_123".to_string(),
            name: "get_weather".to_string(),
            input: json!({"location": "SF"}),
        };
        let val = serde_json::to_value(&block).unwrap();
        assert_eq!(val["type"], "tool_use");
        assert_eq!(val["id"], "toolu_123");
        assert_eq!(val["name"], "get_weather");
        assert_eq!(val["input"]["location"], "SF");
        // input must be an object, not a string
        assert!(val["input"].is_object());
    }

    #[test]
    fn should_build_tool_use_response_with_object_input() {
        let id_gen = test_id_gen();
        let tool_calls: Vec<(&str, Value)> = vec![
            (
                "get_weather",
                json!({"location": "NYC", "units": "fahrenheit"}),
            ),
            ("get_time", json!({"timezone": "UTC"})),
        ];
        let resp = build_tool_use_response(&id_gen, "claude-sonnet-4-6", &tool_calls, "weather?");

        assert_eq!(resp.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(resp.content.len(), 2);

        // Assert prefix format and uniqueness, not exact counter values
        // (counter is shared across all IdGenerator methods)
        match &resp.content[0] {
            ContentBlock::ToolUse { id, name, input } => {
                assert!(id.starts_with("toolu_llmposter_"));
                assert_eq!(name, "get_weather");
                assert!(input.is_object());
                assert_eq!(input["location"], "NYC");
            }
            _ => panic!("expected tool_use content block"),
        }

        let id0 = match &resp.content[0] {
            ContentBlock::ToolUse { id, .. } => id.clone(),
            _ => unreachable!(),
        };
        match &resp.content[1] {
            ContentBlock::ToolUse { id, name, input } => {
                assert!(id.starts_with("toolu_llmposter_"));
                assert_ne!(id, &id0, "tool-call IDs must be unique");
                assert_eq!(name, "get_time");
                assert!(input.is_object());
            }
            _ => panic!("expected tool_use content block"),
        }
    }

    #[test]
    fn should_produce_correct_stream_event_sequence() {
        let id_gen = test_id_gen();
        let events =
            build_stream_events(&id_gen, "claude-sonnet-4-6", "Hello!", 3, "Hi", "end_turn");

        // Expected sequence: message_start, content_block_start, deltas..., content_block_stop, message_delta, message_stop
        assert!(
            events.len() >= 8,
            "expected at least 8 events, got {}",
            events.len()
        );

        // ping is always first
        assert_eq!(events[0].0, "ping");
        assert_eq!(events[0].1["type"], "ping");

        assert_eq!(events[1].0, "message_start");
        assert_eq!(events[1].1["type"], "message_start");

        assert_eq!(events[2].0, "content_block_start");
        assert_eq!(events[2].1["type"], "content_block_start");

        // "Hello!" is 6 chars, chunk_size 3 => 2 delta events
        assert_eq!(events[3].0, "content_block_delta");
        assert_eq!(events[3].1["delta"]["text"], "Hel");
        assert_eq!(events[4].0, "content_block_delta");
        assert_eq!(events[4].1["delta"]["text"], "lo!");

        assert_eq!(events[5].0, "content_block_stop");
        assert_eq!(events[5].1["type"], "content_block_stop");

        assert_eq!(events[6].0, "message_delta");
        assert_eq!(events[6].1["delta"]["stop_reason"], "end_turn");

        assert_eq!(events[7].0, "message_stop");
        assert_eq!(events[7].1["type"], "message_stop");
    }

    #[test]
    fn should_extract_model_and_prompt_from_string_content() {
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "What is Rust?"}
            ]
        });
        let (model, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(model, "claude-sonnet-4-6");
        assert_eq!(prompt, "What is Rust?");
    }

    #[test]
    fn should_extract_prompt_from_array_content_format() {
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "Tell me about Rust."}
                    ]
                }
            ]
        });
        let (model, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(model, "claude-sonnet-4-6");
        assert_eq!(prompt, "Tell me about Rust.");
    }

    #[test]
    fn should_skip_tool_result_content_blocks_in_extraction() {
        // Real Anthropic format: tool results are user messages with tool_result content blocks
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "What is the weather?"},
                {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_1", "name": "weather", "input": {}}]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "72F sunny"}]},
                {"role": "user", "content": "Thanks, and tomorrow?"}
            ]
        });
        let (_, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(prompt, "Thanks, and tomorrow?");
    }

    #[test]
    fn should_error_when_messages_array_missing() {
        let body = json!({"model": "claude-sonnet-4-6", "max_tokens": 1024});
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "missing messages array");
    }

    #[test]
    fn should_round_trip_messages_response() {
        let id_gen = test_id_gen();
        let resp = build_response(&id_gen, "claude-sonnet-4-6", "Hello!", "Hi", "end_turn");

        let json_str = serde_json::to_string(&resp).unwrap();
        let deserialized: MessagesResponse = serde_json::from_str(&json_str).unwrap();

        assert_eq!(resp, deserialized);
    }

    #[test]
    fn should_round_trip_content_block_text() {
        let block = ContentBlock::Text {
            text: "test".to_string(),
            citations: None,
        };
        let json_str = serde_json::to_string(&block).unwrap();
        let deserialized: ContentBlock = serde_json::from_str(&json_str).unwrap();
        assert_eq!(block, deserialized);
    }

    #[test]
    fn should_round_trip_content_block_tool_use() {
        let block = ContentBlock::ToolUse {
            id: "toolu_1".to_string(),
            name: "search".to_string(),
            input: json!({"query": "rust"}),
        };
        let json_str = serde_json::to_string(&block).unwrap();
        let deserialized: ContentBlock = serde_json::from_str(&json_str).unwrap();
        assert_eq!(block, deserialized);
    }

    #[test]
    fn should_handle_tool_result_blocks_in_array_content() {
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "tool_result", "tool_use_id": "toolu_1", "content": "72F"},
                        {"type": "text", "text": "What about tomorrow?"}
                    ]
                }
            ]
        });
        let (_, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(prompt, "What about tomorrow?");
    }

    #[test]
    fn should_error_when_model_is_empty() {
        let body = json!({
            "model": "",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "hello"}
            ]
        });
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("model"));
    }

    #[test]
    fn should_error_when_latest_user_message_is_blank_string() {
        // Blank string in the latest turn must fail fast — not fall back to an earlier turn.
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": ""}
            ]
        });
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("blank"),
            "expected 'blank' in error"
        );
    }

    #[test]
    fn should_not_fall_back_to_stale_turn_when_latest_is_blank() {
        // Blank latest turn must not silently match against an earlier turn's text.
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "real prompt"},
                {"role": "assistant", "content": "response"},
                {"role": "user", "content": "   "}  // blank — must fail fast
            ]
        });
        let result = extract_request_info(&body);
        assert!(result.is_err(), "should fail on blank latest turn");
        assert!(
            result.unwrap_err().contains("blank"),
            "expected 'blank' in error"
        );
    }

    #[test]
    fn should_skip_user_message_with_only_tool_result_blocks() {
        // A user message that only has tool_result blocks (no text) should be skipped.
        // If there's no earlier user message with text, we get an error.
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "tool_result", "tool_use_id": "toolu_1", "content": "72F"}
                    ]
                }
            ]
        });
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("No user message with text content"));
    }

    #[test]
    fn should_fall_back_to_earlier_user_message_when_latest_has_only_tool_results() {
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "What is the weather?"},
                {"role": "assistant", "content": "Let me check."},
                {
                    "role": "user",
                    "content": [
                        {"type": "tool_result", "tool_use_id": "toolu_1", "content": "72F"}
                    ]
                }
            ]
        });
        let (_, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(prompt, "What is the weather?");
    }

    #[test]
    fn should_error_when_no_messages_have_user_role() {
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "assistant", "content": "Hello!"},
                {"role": "system", "content": "Be helpful."}
            ]
        });
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No user message"));
    }

    #[test]
    fn should_skip_array_content_blocks_without_type_field() {
        // Content blocks that lack a "type" field should be silently skipped
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"data": "some blob without type"},
                        {"type": "text", "text": "Real text here"}
                    ]
                }
            ]
        });
        let (_, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(prompt, "Real text here");
    }

    #[test]
    fn should_reject_user_with_content_not_string_or_array() {
        // content is an object (neither string nor array) — latest user turn, must error
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "First real message"},
                {"role": "user", "content": {"nested": "object"}}
            ]
        });
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unrecognized content format"));
    }

    #[test]
    fn should_reject_user_with_null_content() {
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": null}
            ]
        });
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unrecognized content format"));
    }

    #[test]
    fn should_reject_user_message_with_no_content_field() {
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user"}
            ]
        });
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no content field"));
    }

    #[test]
    fn should_include_usage_in_response() {
        let id_gen = test_id_gen();
        let resp = build_response(
            &id_gen,
            "claude-sonnet-4-6",
            "Hello world",
            "Hi there",
            "end_turn",
        );

        assert!(resp.usage.input_tokens > 0);
        assert!(resp.usage.output_tokens > 0);
        assert_eq!(resp.usage.cache_creation_input_tokens, 0);
        assert_eq!(resp.usage.cache_read_input_tokens, 0);
    }

    #[test]
    fn should_serialize_response_with_type_field() {
        let id_gen = test_id_gen();
        let resp = build_response(&id_gen, "claude-sonnet-4-6", "Hi", "Hello", "end_turn");
        let val = serde_json::to_value(&resp).unwrap();

        // The struct field is `response_type` but serializes as `type`
        assert_eq!(val["type"], "message");
        assert!(val.get("response_type").is_none());
    }

    #[test]
    fn should_error_when_latest_user_message_is_image_only() {
        // Image-only latest user turn must not fall back to an older message —
        // that would serve the wrong fixture.
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "Tell me about this image."},
                {
                    "role": "user",
                    "content": [
                        {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "abc"}}
                    ]
                }
            ]
        });
        let result = extract_request_info(&body);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Latest user message has no text content"));
    }

    #[test]
    fn should_fall_back_when_latest_user_message_is_pure_tool_result() {
        // A tool_result-only follow-up is a valid multi-turn tool flow:
        // fall back to the prior user message for fixture matching.
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "What is the weather?"},
                {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_1", "name": "weather", "input": {}}]},
                {
                    "role": "user",
                    "content": [
                        {"type": "tool_result", "tool_use_id": "toolu_1", "content": "72F sunny"}
                    ]
                }
            ]
        });
        let (_, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(prompt, "What is the weather?");
    }
}
