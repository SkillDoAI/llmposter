//! Gemini generateContent API format module.
//!
//! Spec: https://ai.google.dev/api/generate-content
//! Target: v1beta (latest, 2025)

use serde::{Deserialize, Serialize};

use crate::format::estimate_tokens;

/// Full Gemini generateContent response (streaming or non-streaming).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentResponse {
    /// List of candidate responses (always one for mock responses).
    pub candidates: Vec<Candidate>,
    /// Prompt safety feedback, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_feedback: Option<serde_json::Value>,
    /// Token usage statistics.
    pub usage_metadata: UsageMetadata,
    /// Model version string, if provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_version: Option<String>,
}

/// A single candidate within a Gemini response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// The generated content (parts + role).
    pub content: Content,
    /// Zero-based candidate index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u64>,
    /// Why generation stopped (e.g. `"STOP"`). `None` on non-final stream chunks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    /// Content safety ratings (not populated in mocks).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_ratings: Option<Vec<serde_json::Value>>,
}

/// Content container holding parts and an optional role.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Content {
    /// Ordered list of content parts (text and/or function calls).
    pub parts: Vec<Part>,
    /// Role of the content producer (e.g. `"model"`, `"user"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// A single part within Gemini content (text or function call).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    /// Text content, if this is a text part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Function call, if this is a tool invocation part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_call: Option<FunctionCallPart>,
}

/// See also: `tests/spec/types/gemini.rs::SpecFunctionCall` (golden struct with
/// additional forward-compat fields like `id` for Gemini 2.x).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCallPart {
    /// Name of the function to call.
    pub name: String,
    /// Arguments as a JSON object.
    pub args: serde_json::Value,
}

/// Token usage metadata for a Gemini response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetadata {
    /// Estimated tokens in the input prompt.
    pub prompt_token_count: u64,
    /// Estimated tokens in the generated candidates.
    pub candidates_token_count: u64,
    /// Sum of prompt and candidate tokens.
    pub total_token_count: u64,
}

// --- Builder functions ---

/// Build a complete (non-streaming) Gemini text response.
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
                role: Some("model".to_string()),
            },
            index: Some(0),
            finish_reason: Some("STOP".to_string()),
            safety_ratings: None,
        }],
        prompt_feedback: None,
        usage_metadata: UsageMetadata {
            prompt_token_count: prompt_tokens,
            candidates_token_count: completion_tokens,
            total_token_count: prompt_tokens + completion_tokens,
        },
        model_version: None,
    }
}

/// Build a Gemini response containing function call parts.
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
                role: Some("model".to_string()),
            },
            index: Some(0),
            finish_reason: Some("STOP".to_string()),
            safety_ratings: None,
        }],
        prompt_feedback: None,
        usage_metadata: UsageMetadata {
            prompt_token_count: prompt_tokens,
            candidates_token_count: completion_tokens,
            total_token_count: prompt_tokens + completion_tokens,
        },
        model_version: None,
    }
}

/// Split content into streaming chunks, each a complete `GenerateContentResponse`.
///
/// Only the last chunk carries `finish_reason` and full usage metadata.
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
                        role: Some("model".to_string()),
                    },
                    index: Some(0),
                    finish_reason: if is_last {
                        Some("STOP".to_string())
                    } else {
                        None
                    },
                    safety_ratings: None,
                }],
                prompt_feedback: None,
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
                model_version: None,
            }
        })
        .collect()
}

// --- Request extraction ---

/// Extract `(model, prompt_text)` from a Gemini generateContent request body.
///
/// The model comes from the URL path, not the body. Falls back to `"unknown"`
/// when `model_from_url` is `None`.
pub fn extract_request_info(
    body: &serde_json::Value,
    model_from_url: Option<&str>,
) -> Result<(String, String), String> {
    let model = model_from_url.unwrap_or("unknown").to_string();

    let contents = body
        .get("contents")
        .and_then(|c| c.as_array())
        .ok_or_else(|| "Missing or invalid 'contents' field".to_string())?;

    // Find the latest user turn (role == "user" or no role).
    // If the latest user turn has no text parts (e.g. image-only), return an
    // error rather than falling back to an older message — that would serve the
    // wrong fixture.
    let latest_user_turn = contents
        .iter()
        .rev()
        .find(|msg| is_user_turn(msg))
        .ok_or_else(|| "No user message with text content found in 'contents'".to_string())?;

    let parts = latest_user_turn
        .get("parts")
        .and_then(|p| p.as_array())
        .ok_or_else(|| "No user message with text content found in 'contents'".to_string())?;

    let text: String = parts
        .iter()
        .filter_map(|part| {
            if let Some(t) = part.get("type").and_then(|t| t.as_str()) {
                if t != "text" {
                    return None;
                }
            }
            part.get("text").and_then(|t| t.as_str())
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = if text.is_empty() {
        return Err(
            "Latest user message has no text content (image-only or unsupported)".to_string(),
        );
    } else {
        text
    };

    Ok((model, prompt))
}

/// Check if a Gemini content entry is a user turn (role "user" or absent).
fn is_user_turn(message: &serde_json::Value) -> bool {
    match message.get("role") {
        None => true,
        Some(role) => role.as_str() == Some("user"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn should_build_response_with_camel_case_json() {
        let resp = build_response("Hello world", "Say hello");
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json.get("usageMetadata").is_some());
        let candidate = &json["candidates"][0];
        assert_eq!(candidate["finishReason"], "STOP");
        assert_eq!(candidate["content"]["role"], "model");
        assert_eq!(candidate["content"]["parts"][0]["text"], "Hello world");
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
            ("get_weather", json!({"location": "SF"})),
            ("get_time", json!({"timezone": "UTC"})),
        ];
        let resp = build_tool_call_response(&tool_calls, "weather");
        let json = serde_json::to_value(&resp).unwrap();
        let parts = json["candidates"][0]["content"]["parts"]
            .as_array()
            .unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["functionCall"]["name"], "get_weather");
        assert_eq!(parts[1]["functionCall"]["name"], "get_time");
    }

    #[test]
    fn should_extract_request_info_with_model_from_url() {
        let body = json!({
            "contents": [{"role": "user", "parts": [{"text": "Hello"}]}]
        });
        let (model, prompt) = extract_request_info(&body, Some("gemini-pro")).unwrap();
        assert_eq!(model, "gemini-pro");
        assert_eq!(prompt, "Hello");
    }

    #[test]
    fn should_treat_roleless_content_as_user_message() {
        let body = json!({
            "contents": [{"parts": [{"text": "Hello"}]}]
        });
        let (model, prompt) = extract_request_info(&body, Some("gemini-pro")).unwrap();
        assert_eq!(model, "gemini-pro");
        assert_eq!(prompt, "Hello");
    }

    #[test]
    fn should_return_error_when_contents_missing() {
        let body = json!({"prompt": "no contents"});
        let result = extract_request_info(&body, Some("gemini-pro"));
        assert!(result.is_err());
    }

    #[test]
    fn should_build_stream_chunks_with_partial_text() {
        let chunks = build_stream_chunks("Hello, world!", 5, "Say hello");
        assert_eq!(chunks.len(), 3);
        // Verify actual text content of each chunk
        assert_eq!(
            chunks[0].candidates[0].content.parts[0].text,
            Some("Hello".to_string())
        );
        assert!(chunks[0].candidates[0].finish_reason.is_none());
        assert!(chunks[1].candidates[0].finish_reason.is_none());
        assert_eq!(
            chunks[2].candidates[0].content.parts[0].text,
            Some("ld!".to_string())
        );
        assert_eq!(
            chunks[2].candidates[0].finish_reason.as_deref(),
            Some("STOP")
        );
    }

    #[test]
    fn should_produce_valid_usage_metadata_tokens() {
        let resp = build_response("Test", "prompt");
        assert!(resp.usage_metadata.prompt_token_count > 0);
        assert!(resp.usage_metadata.candidates_token_count > 0);
        assert_eq!(
            resp.usage_metadata.total_token_count,
            resp.usage_metadata.prompt_token_count + resp.usage_metadata.candidates_token_count
        );
    }

    #[test]
    fn should_serialize_and_deserialize_round_trip() {
        let resp = build_response("Round trip", "prompt");
        let json_str = serde_json::to_string(&resp).unwrap();
        let deserialized: GenerateContentResponse = serde_json::from_str(&json_str).unwrap();
        assert_eq!(
            deserialized.candidates[0].content.role,
            Some("model".to_string())
        );
    }

    #[test]
    fn should_omit_role_when_none() {
        let content = Content {
            parts: vec![Part {
                text: Some("hi".to_string()),
                function_call: None,
            }],
            role: None,
        };

        let json = serde_json::to_value(&content).unwrap();
        assert!(json.get("role").is_none());

        let round_trip: Content = serde_json::from_value(json).unwrap();
        assert!(round_trip.role.is_none());
    }

    #[test]
    fn should_extract_last_user_message() {
        let body = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "First"}]},
                {"role": "model", "parts": [{"text": "Response"}]},
                {"role": "user", "parts": [{"text": "Second"}]}
            ]
        });
        let (_, prompt) = extract_request_info(&body, Some("gemini-pro")).unwrap();
        assert_eq!(prompt, "Second");
    }

    #[test]
    fn should_default_model_to_unknown_when_not_in_url() {
        let body = json!({"contents": [{"role": "user", "parts": [{"text": "Hi"}]}]});
        let (model, _) = extract_request_info(&body, None).unwrap();
        assert_eq!(model, "unknown");
    }

    #[test]
    fn should_error_when_latest_user_turn_has_no_parts() {
        // A user turn with no parts field at all is not a text message.
        // Must not fall back to an older message — return an error.
        let body = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "First"}]},
                {"role": "user"}
            ]
        });
        let result = extract_request_info(&body, Some("gemini-pro"));
        assert!(result.is_err());
    }

    #[test]
    fn should_error_when_latest_user_turn_has_only_image_parts() {
        // Image-only latest user turn must not fall back to an older message —
        // that would serve the wrong fixture.
        let body = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "First"}]},
                {"role": "user", "parts": [{"inlineData": {"mimeType": "image/png", "data": "..."}}]}
            ]
        });
        let result = extract_request_info(&body, Some("gemini-pro"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Latest user message has no text content"));
    }

    #[test]
    fn should_return_error_when_no_user_text_found() {
        let body = json!({"contents": [{"role": "model", "parts": [{"text": "I am model"}]}]});
        let result = extract_request_info(&body, Some("gemini-pro"));
        assert!(result.is_err());
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
    fn should_skip_serializing_none_fields_in_part() {
        let resp = build_response("text only", "prompt");
        let json_val = serde_json::to_value(&resp).unwrap();
        let part = &json_val["candidates"][0]["content"]["parts"][0];
        assert!(part.get("functionCall").is_none());
        assert_eq!(part["text"], "text only");

        let tool_resp = build_tool_call_response(&[("fn1", json!({}))], "prompt");
        let json_val = serde_json::to_value(&tool_resp).unwrap();
        let part = &json_val["candidates"][0]["content"]["parts"][0];
        assert!(part.get("text").is_none());
        assert!(part.get("functionCall").is_some());
    }
}
