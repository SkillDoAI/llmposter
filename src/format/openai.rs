use serde::{Deserialize, Serialize};

use crate::format::{estimate_tokens, IdGenerator};

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
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallOutput>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCallOutput {
    pub index: u32,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallOutput>>,
}

// --- Builders ---

pub fn build_response(
    id_gen: &IdGenerator,
    model: &str,
    content: &str,
    prompt: &str,
) -> ChatCompletionResponse {
    let prompt_tokens = estimate_tokens(prompt);
    let completion_tokens = estimate_tokens(content);

    ChatCompletionResponse {
        id: id_gen.next_openai(),
        object: "chat.completion".to_string(),
        model: model.to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message {
                role: "assistant".to_string(),
                content: Some(content.to_string()),
                tool_calls: None,
            },
            finish_reason: "stop".to_string(),
        }],
        usage: Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
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
            index: i as u32,
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
                content: None,
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

/// Split content into streaming chunks.
///
/// Produces:
/// 1. First chunk: delta with role "assistant" and first content piece
/// 2. Middle chunks: delta with content only
/// 3. Final chunk: empty delta with finish_reason "stop"
pub fn build_stream_chunks(
    id: &str,
    model: &str,
    content: &str,
    chunk_size: usize,
) -> Vec<ChatCompletionChunk> {
    let mut chunks = Vec::new();
    let content_pieces = crate::stream::chunk_content(content, chunk_size);

    // Always emit an initial chunk with the assistant role
    if content_pieces.is_empty() {
        chunks.push(ChatCompletionChunk {
            id: id.to_string(),
            object: "chat.completion.chunk".to_string(),
            model: model.to_string(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: Delta {
                    role: Some("assistant".to_string()),
                    content: None,
                    tool_calls: None,
                },
                finish_reason: None,
            }],
        });
    }

    // For non-empty content, emit role-only chunk first, then content chunks
    if !content_pieces.is_empty() {
        chunks.push(ChatCompletionChunk {
            id: id.to_string(),
            object: "chat.completion.chunk".to_string(),
            model: model.to_string(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: Delta {
                    role: Some("assistant".to_string()),
                    content: None,
                    tool_calls: None,
                },
                finish_reason: None,
            }],
        });
    }

    for piece in &content_pieces {
        let delta = Delta {
            role: None,
            content: Some(piece.to_string()),
            tool_calls: None,
        };

        chunks.push(ChatCompletionChunk {
            id: id.to_string(),
            object: "chat.completion.chunk".to_string(),
            model: model.to_string(),
            choices: vec![ChunkChoice {
                index: 0,
                delta,
                finish_reason: None,
            }],
        });
    }

    // Final chunk with finish_reason "stop" and empty delta
    chunks.push(ChatCompletionChunk {
        id: id.to_string(),
        object: "chat.completion.chunk".to_string(),
        model: model.to_string(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: Delta {
                role: None,
                content: None,
                tool_calls: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
    });

    chunks
}

// --- Request extraction ---

/// Extract model and last user message from an OpenAI chat completions request body.
///
/// Message content can be a plain string or an array of content parts
/// like `[{"type": "text", "text": "..."}]`.
pub fn extract_request_info(body: &serde_json::Value) -> Result<(String, String), String> {
    let model = body["model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Missing or empty 'model' field in request")?
        .to_string();

    let messages = body["messages"]
        .as_array()
        .ok_or("Missing 'messages' array in request")?;

    let user_msg = messages
        .iter()
        .rev()
        .find(|m| m["role"].as_str() == Some("user"))
        .ok_or("No user message found in request")?;

    let content = extract_content(user_msg)?;
    Ok((model, content))
}

/// Extract text content from a message, handling both string and array formats.
fn extract_content(message: &serde_json::Value) -> Result<String, String> {
    let content = &message["content"];

    // String content: {"content": "hello"}
    if let Some(s) = content.as_str() {
        return Ok(s.to_string());
    }

    // Array content: {"content": [{"type": "text", "text": "hello"}]}
    if let Some(parts) = content.as_array() {
        let texts: Vec<&str> = parts
            .iter()
            .filter(|p| p["type"].as_str() == Some("text"))
            .filter_map(|p| p["text"].as_str())
            .collect();

        if texts.is_empty() {
            return Err("No text content found in message content array".to_string());
        }

        return Ok(texts.join("\n"));
    }

    Err("Message content is neither string nor array".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{estimate_tokens, IdGenerator};

    #[test]
    fn should_build_chat_completion_response() {
        let gen = IdGenerator::new();
        let resp = build_response(&gen, "gpt-4", "Hello!", "What is Rust?");
        assert_eq!(resp.id, "chatcmpl-llmposter-1");
        assert_eq!(resp.object, "chat.completion");
        assert_eq!(resp.model, "gpt-4");
        assert_eq!(resp.choices[0].message.content.as_deref(), Some("Hello!"));
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
    fn should_round_trip_serialization() {
        let gen = IdGenerator::new();
        let resp = build_response(&gen, "gpt-4", "round trip test", "prompt text");
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: ChatCompletionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, resp.id);
        assert_eq!(parsed.model, resp.model);
        assert_eq!(
            parsed.choices[0].message.content.as_deref(),
            Some("round trip test")
        );
        assert_eq!(parsed.usage.prompt_tokens, resp.usage.prompt_tokens);
        assert_eq!(parsed.usage.completion_tokens, resp.usage.completion_tokens);
        assert_eq!(parsed.usage.total_tokens, resp.usage.total_tokens);
    }

    #[test]
    fn should_build_tool_call_response() {
        let gen = IdGenerator::new();
        let args = serde_json::json!({"location": "SF"});
        let tool_calls = vec![("get_weather", args)];
        let resp = build_tool_call_response(&gen, "gpt-4", &tool_calls, "prompt");
        assert_eq!(
            resp.choices[0].message.tool_calls.as_ref().unwrap().len(),
            1
        );
        let tc = &resp.choices[0].message.tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc.function.name, "get_weather");
        // OpenAI sends arguments as a JSON string
        assert!(tc.function.arguments.contains("SF"));
        assert_eq!(resp.choices[0].finish_reason, "tool_calls");
    }

    #[test]
    fn should_serialize_tool_call_arguments_as_json_string() {
        let gen = IdGenerator::new();
        let args = serde_json::json!({"city": "London", "units": "celsius"});
        let tool_calls = vec![("get_weather", args)];
        let resp = build_tool_call_response(&gen, "gpt-4", &tool_calls, "prompt");
        let tc = &resp.choices[0].message.tool_calls.as_ref().unwrap()[0];
        // arguments should be a JSON string, not an object
        let parsed: serde_json::Value = serde_json::from_str(&tc.function.arguments).unwrap();
        assert_eq!(parsed["city"], "London");
        assert_eq!(parsed["units"], "celsius");
    }

    #[test]
    fn should_assign_sequential_tool_call_ids() {
        let gen = IdGenerator::new();
        let tool_calls = vec![
            ("func_a", serde_json::json!({})),
            ("func_b", serde_json::json!({"x": 1})),
        ];
        let resp = build_tool_call_response(&gen, "gpt-4", &tool_calls, "prompt");
        let tcs = resp.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tcs[0].id, "call_llmposter_1");
        assert_eq!(tcs[1].id, "call_llmposter_2");
        assert_eq!(tcs[0].call_type, "function");
        assert_eq!(tcs[1].call_type, "function");
    }

    #[test]
    fn should_build_correct_number_of_stream_chunks() {
        // "Hello, world!" is 13 chars, chunk_size 5 → 1 role + 3 content + 1 stop = 5
        let chunks = build_stream_chunks("id-1", "gpt-4", "Hello, world!", 5);
        assert_eq!(chunks.len(), 5);
    }

    #[test]
    fn should_set_role_on_first_stream_chunk_only() {
        let chunks = build_stream_chunks("id-1", "gpt-4", "Hello!", 3);
        // First chunk has role
        assert_eq!(
            chunks[0].choices[0].delta.role.as_deref(),
            Some("assistant")
        );
        // Second chunk has no role
        assert!(chunks[1].choices[0].delta.role.is_none());
    }

    #[test]
    fn should_set_finish_reason_on_last_stream_chunk_only() {
        let chunks = build_stream_chunks("id-1", "gpt-4", "Hi", 2);
        // Content chunk: no finish_reason
        assert!(chunks[0].choices[0].finish_reason.is_none());
        // Final chunk: finish_reason "stop"
        let last = chunks.last().unwrap();
        assert_eq!(last.choices[0].finish_reason.as_deref(), Some("stop"));
        // Final chunk delta should be empty
        assert!(last.choices[0].delta.role.is_none());
        assert!(last.choices[0].delta.content.is_none());
    }

    #[test]
    fn should_produce_final_chunk_for_empty_content() {
        let chunks = build_stream_chunks("id-1", "gpt-4", "", 5);
        // Role chunk + final stop chunk
        assert_eq!(chunks.len(), 2);
        // First: role chunk
        assert_eq!(
            chunks[0].choices[0].delta.role.as_deref(),
            Some("assistant")
        );
        assert!(chunks[0].choices[0].finish_reason.is_none());
        // Second: stop chunk
        assert_eq!(chunks[1].choices[0].finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn should_set_chunk_object_to_chat_completion_chunk() {
        let chunks = build_stream_chunks("id-1", "gpt-4", "test", 10);
        for chunk in &chunks {
            assert_eq!(chunk.object, "chat.completion.chunk");
        }
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
    fn should_extract_array_content_from_user_message() {
        let json = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "What is this?"},
                        {"type": "image_url", "image_url": {"url": "http://example.com/img.png"}}
                    ]
                }
            ]
        });
        let (model, user_msg) = extract_request_info(&json).unwrap();
        assert_eq!(model, "gpt-4");
        assert_eq!(user_msg, "What is this?");
    }

    #[test]
    fn should_return_error_for_missing_messages() {
        let json = serde_json::json!({"model": "gpt-4"});
        let result = extract_request_info(&json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("messages"));
    }

    #[test]
    fn should_return_error_for_no_user_message() {
        let json = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "system prompt"},
            ]
        });
        let result = extract_request_info(&json);
        assert!(result.is_err());
    }

    #[test]
    fn should_return_error_for_non_string_non_array_content() {
        let json = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "user", "content": 42}
            ]
        });
        let result = extract_request_info(&json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("neither string nor array"));
    }

    #[test]
    fn should_return_error_for_array_content_with_no_text_parts() {
        let json = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "image_url", "image_url": {"url": "http://example.com/img.png"}}
                    ]
                }
            ]
        });
        let result = extract_request_info(&json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No text content"));
    }

    #[test]
    fn should_extract_multiple_text_parts_joined_with_newline() {
        let json = serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "First part"},
                        {"type": "image_url", "image_url": {"url": "http://example.com"}},
                        {"type": "text", "text": "Second part"}
                    ]
                }
            ]
        });
        let (_, content) = extract_request_info(&json).unwrap();
        assert_eq!(content, "First part\nSecond part");
    }

    #[test]
    fn should_return_error_for_empty_model() {
        let json = serde_json::json!({
            "model": "",
            "messages": [
                {"role": "user", "content": "hello"}
            ]
        });
        let result = extract_request_info(&json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("model"));
    }

    #[test]
    fn should_reject_missing_model() {
        let json = serde_json::json!({
            "messages": [
                {"role": "user", "content": "hi"},
            ]
        });
        let result = extract_request_info(&json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("model"));
    }

    #[test]
    fn should_compute_usage_tokens() {
        let gen = IdGenerator::new();
        let resp = build_response(&gen, "gpt-4", "hello", "prompt");
        assert_eq!(resp.usage.prompt_tokens, estimate_tokens("prompt"));
        assert_eq!(resp.usage.completion_tokens, estimate_tokens("hello"));
        assert_eq!(
            resp.usage.total_tokens,
            resp.usage.prompt_tokens + resp.usage.completion_tokens
        );
    }
}
