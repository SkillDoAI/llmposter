use serde::{Deserialize, Serialize};

use crate::format::estimate_tokens;

// --- Response structs ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentResponse {
    pub candidates: Vec<Candidate>,
    pub usage_metadata: UsageMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub content: Content,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Content {
    pub parts: Vec<Part>,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_call: Option<FunctionCallPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCallPart {
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetadata {
    pub prompt_token_count: u64,
    pub candidates_token_count: u64,
    pub total_token_count: u64,
}

// --- Builder functions ---

pub fn build_response(content: &str, prompt: &str) -> GenerateContentResponse {
    let prompt_tokens = estimate_tokens(prompt);
    let completion_tokens = estimate_tokens(content);

    GenerateContentResponse {
        candidates: vec![Candidate {
            content: Content {
                parts: vec![Part {
                    text: Some(content.to_string()),
                    function_call: None,
                }],
                role: "model".to_string(),
            },
            finish_reason: Some("STOP".to_string()),
        }],
        usage_metadata: UsageMetadata {
            prompt_token_count: prompt_tokens,
            candidates_token_count: completion_tokens,
            total_token_count: prompt_tokens + completion_tokens,
        },
    }
}

pub fn build_tool_call_response(
    tool_calls: &[(&str, serde_json::Value)],
    prompt: &str,
) -> GenerateContentResponse {
    let prompt_tokens = estimate_tokens(prompt);

    let parts: Vec<Part> = tool_calls
        .iter()
        .map(|(name, args)| Part {
            text: None,
            function_call: Some(FunctionCallPart {
                name: name.to_string(),
                args: args.clone(),
            }),
        })
        .collect();

    let completion_tokens = estimate_tokens(&serde_json::to_string(&parts).unwrap_or_default());

    GenerateContentResponse {
        candidates: vec![Candidate {
            content: Content {
                parts,
                role: "model".to_string(),
            },
            finish_reason: Some("STOP".to_string()),
        }],
        usage_metadata: UsageMetadata {
            prompt_token_count: prompt_tokens,
            candidates_token_count: completion_tokens,
            total_token_count: prompt_tokens + completion_tokens,
        },
    }
}

pub fn build_stream_chunks(
    content: &str,
    chunk_size: usize,
    prompt: &str,
) -> Vec<GenerateContentResponse> {
    let prompt_tokens = estimate_tokens(prompt);
    let total_completion_tokens = estimate_tokens(content);
    let chunks = crate::stream::chunk_content(content, chunk_size);

    if chunks.is_empty() {
        return vec![build_response("", prompt)];
    }

    let num_chunks = chunks.len();

    chunks
        .into_iter()
        .enumerate()
        .map(|(i, chunk_text)| {
            let is_last = i == num_chunks - 1;
            let chunk_tokens = estimate_tokens(&chunk_text);

            GenerateContentResponse {
                candidates: vec![Candidate {
                    content: Content {
                        parts: vec![Part {
                            text: Some(chunk_text),
                            function_call: None,
                        }],
                        role: "model".to_string(),
                    },
                    finish_reason: if is_last {
                        Some("STOP".to_string())
                    } else {
                        None
                    },
                }],
                usage_metadata: UsageMetadata {
                    prompt_token_count: if is_last { prompt_tokens } else { 0 },
                    candidates_token_count: if is_last {
                        total_completion_tokens
                    } else {
                        chunk_tokens
                    },
                    total_token_count: if is_last {
                        prompt_tokens + total_completion_tokens
                    } else {
                        chunk_tokens
                    },
                },
            }
        })
        .collect()
}

// --- Request extraction ---

pub fn extract_request_info(
    body: &serde_json::Value,
    model_from_url: Option<&str>,
) -> Result<(String, String), String> {
    let model = model_from_url.unwrap_or("unknown").to_string();

    let contents = body
        .get("contents")
        .and_then(|c| c.as_array())
        .ok_or_else(|| "Missing or invalid 'contents' field".to_string())?;

    // Find the last user message and join all text parts
    let prompt = contents
        .iter()
        .rev()
        .filter(|msg| msg.get("role").and_then(|r| r.as_str()) == Some("user"))
        .find_map(|msg| {
            msg.get("parts").and_then(|p| p.as_array()).map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        })
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "No user message with text content found in 'contents'".to_string())?;

    Ok((model, prompt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn should_build_response_with_camel_case_json() {
        let resp = build_response("Hello world", "Say hello");
        let json = serde_json::to_value(&resp).unwrap();

        // Verify camelCase field names
        assert!(json.get("usageMetadata").is_some());
        assert!(json.get("usage_metadata").is_none());

        let candidate = &json["candidates"][0];
        assert_eq!(candidate["finishReason"], "STOP");
        assert!(candidate.get("finish_reason").is_none());

        let usage = &json["usageMetadata"];
        assert!(usage.get("promptTokenCount").is_some());
        assert!(usage.get("candidatesTokenCount").is_some());
        assert!(usage.get("totalTokenCount").is_some());

        // Verify content
        assert_eq!(candidate["content"]["parts"][0]["text"], "Hello world");
        assert_eq!(candidate["content"]["role"], "model");
    }

    #[test]
    fn should_not_include_id_field_in_response() {
        let resp = build_response("No ID here", "prompt");
        let json = serde_json::to_value(&resp).unwrap();

        assert!(json.get("id").is_none());
    }

    #[test]
    fn should_build_tool_call_response_with_function_call_parts() {
        let tool_calls: Vec<(&str, serde_json::Value)> = vec![
            ("get_weather", json!({"location": "SF", "unit": "celsius"})),
            ("get_time", json!({"timezone": "UTC"})),
        ];

        let resp = build_tool_call_response(&tool_calls, "What's the weather?");
        let json = serde_json::to_value(&resp).unwrap();

        let parts = json["candidates"][0]["content"]["parts"]
            .as_array()
            .unwrap();
        assert_eq!(parts.len(), 2);

        // First function call
        let fc0 = &parts[0]["functionCall"];
        assert_eq!(fc0["name"], "get_weather");
        assert_eq!(fc0["args"]["location"], "SF");
        assert_eq!(fc0["args"]["unit"], "celsius");

        // Second function call
        let fc1 = &parts[1]["functionCall"];
        assert_eq!(fc1["name"], "get_time");
        assert_eq!(fc1["args"]["timezone"], "UTC");

        // text should not be present
        assert!(parts[0].get("text").is_none());
        assert!(parts[1].get("text").is_none());

        assert_eq!(json["candidates"][0]["finishReason"], "STOP");
    }

    #[test]
    fn should_extract_request_info_with_model_from_url() {
        let body = json!({
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "Tell me a joke"}]
                }
            ]
        });

        let (model, prompt) = extract_request_info(&body, Some("gemini-1.5-pro")).unwrap();
        assert_eq!(model, "gemini-1.5-pro");
        assert_eq!(prompt, "Tell me a joke");
    }

    #[test]
    fn should_extract_last_user_message() {
        let body = json!({
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "First message"}]
                },
                {
                    "role": "model",
                    "parts": [{"text": "Response"}]
                },
                {
                    "role": "user",
                    "parts": [{"text": "Second message"}]
                }
            ]
        });

        let (_, prompt) = extract_request_info(&body, Some("gemini-pro")).unwrap();
        assert_eq!(prompt, "Second message");
    }

    #[test]
    fn should_return_error_when_contents_missing() {
        let body = json!({"prompt": "no contents field"});

        let result = extract_request_info(&body, Some("gemini-pro"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Missing or invalid 'contents' field"));
    }

    #[test]
    fn should_default_model_to_unknown_when_not_in_url() {
        let body = json!({
            "contents": [
                {
                    "role": "user",
                    "parts": [{"text": "Hello"}]
                }
            ]
        });

        let (model, _) = extract_request_info(&body, None).unwrap();
        assert_eq!(model, "unknown");
    }

    #[test]
    fn should_serialize_and_deserialize_round_trip() {
        let resp = build_response("Round trip test", "prompt text");
        let json_str = serde_json::to_string(&resp).unwrap();
        let deserialized: GenerateContentResponse = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.candidates.len(), 1);
        assert_eq!(
            deserialized.candidates[0].content.parts[0].text,
            Some("Round trip test".to_string())
        );
        assert_eq!(
            deserialized.candidates[0].finish_reason.as_deref(),
            Some("STOP")
        );
        assert_eq!(deserialized.candidates[0].content.role, "model");
        assert_eq!(
            deserialized.usage_metadata.total_token_count,
            deserialized.usage_metadata.prompt_token_count
                + deserialized.usage_metadata.candidates_token_count
        );
    }

    #[test]
    fn should_build_stream_chunks_with_partial_text() {
        let content = "Hello, world!";
        let chunks = build_stream_chunks(content, 5, "Say hello");

        // "Hello" ", wor" "ld!"
        assert_eq!(chunks.len(), 3);

        // Each chunk is a full GenerateContentResponse
        assert_eq!(
            chunks[0].candidates[0].content.parts[0].text,
            Some("Hello".to_string())
        );
        assert_eq!(
            chunks[1].candidates[0].content.parts[0].text,
            Some(", wor".to_string())
        );
        assert_eq!(
            chunks[2].candidates[0].content.parts[0].text,
            Some("ld!".to_string())
        );

        // Only last chunk has STOP
        assert!(chunks[0].candidates[0].finish_reason.is_none());
        assert!(chunks[1].candidates[0].finish_reason.is_none());
        assert_eq!(
            chunks[2].candidates[0].finish_reason.as_deref(),
            Some("STOP")
        );

        // All chunks have role "model"
        for chunk in &chunks {
            assert_eq!(chunk.candidates[0].content.role, "model");
        }
    }

    #[test]
    fn should_handle_empty_content_in_stream_chunks() {
        let chunks = build_stream_chunks("", 5, "prompt");
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0].candidates[0].content.parts[0].text,
            Some("".to_string())
        );
        assert_eq!(
            chunks[0].candidates[0].finish_reason.as_deref(),
            Some("STOP")
        );
    }

    #[test]
    fn should_produce_valid_usage_metadata_tokens() {
        let resp = build_response("Test response content", "Test prompt");
        assert!(resp.usage_metadata.prompt_token_count > 0);
        assert!(resp.usage_metadata.candidates_token_count > 0);
        assert_eq!(
            resp.usage_metadata.total_token_count,
            resp.usage_metadata.prompt_token_count + resp.usage_metadata.candidates_token_count
        );
    }

    #[test]
    fn should_skip_serializing_none_fields_in_part() {
        // Text part should not have functionCall
        let resp = build_response("text only", "prompt");
        let json = serde_json::to_value(&resp).unwrap();
        let part = &json["candidates"][0]["content"]["parts"][0];
        assert!(part.get("functionCall").is_none());
        assert_eq!(part["text"], "text only");

        // Function call part should not have text
        let tool_resp = build_tool_call_response(&[("fn1", json!({}))], "prompt");
        let json = serde_json::to_value(&tool_resp).unwrap();
        let part = &json["candidates"][0]["content"]["parts"][0];
        assert!(part.get("text").is_none());
        assert!(part.get("functionCall").is_some());
    }
}
