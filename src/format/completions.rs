//! Legacy text completions API (`/v1/completions`) format types and builders.
//!
//! This is the older OpenAI completions API that takes a `prompt` string
//! instead of a `messages` array and returns `choices[].text` instead of
//! `choices[].message`.

use serde::Serialize;

use super::{estimate_tokens, IdGenerator};

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct TextCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<TextChoice>,
    pub usage: Usage,
}

#[derive(Serialize)]
pub struct TextChoice {
    pub text: String,
    pub index: u32,
    pub finish_reason: Option<String>,
    pub logprobs: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

// ---------------------------------------------------------------------------
// Streaming chunk type
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct TextCompletionChunk {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<TextChunkChoice>,
}

#[derive(Serialize)]
pub struct TextChunkChoice {
    pub text: String,
    pub index: u32,
    pub finish_reason: Option<String>,
    pub logprobs: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn build_response(
    id_gen: &IdGenerator,
    model: &str,
    text: &str,
    prompt: &str,
    stop_reason: &str,
) -> TextCompletionResponse {
    let prompt_tokens = estimate_tokens(prompt);
    let completion_tokens = estimate_tokens(text);
    TextCompletionResponse {
        id: id_gen.next_completions(),
        object: "text_completion".to_string(),
        created: unix_timestamp(),
        model: model.to_string(),
        choices: vec![TextChoice {
            text: text.to_string(),
            index: 0,
            finish_reason: Some(stop_reason.to_string()),
            logprobs: None,
        }],
        usage: Usage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens.saturating_add(completion_tokens),
        },
    }
}

pub fn build_stream_chunks(
    id: &str,
    model: &str,
    text: &str,
    chunk_size: usize,
    stop_reason: &str,
) -> Vec<TextCompletionChunk> {
    let created = unix_timestamp();
    let chunks = crate::stream::chunk_content(text, chunk_size);
    let mut result = Vec::with_capacity(chunks.len() + 1);
    for chunk_text in &chunks {
        result.push(TextCompletionChunk {
            id: id.to_string(),
            object: "text_completion".to_string(),
            created,
            model: model.to_string(),
            choices: vec![TextChunkChoice {
                text: chunk_text.to_string(),
                index: 0,
                finish_reason: None,
                logprobs: None,
            }],
        });
    }
    // Final chunk with finish_reason
    result.push(TextCompletionChunk {
        id: id.to_string(),
        object: "text_completion".to_string(),
        created,
        model: model.to_string(),
        choices: vec![TextChunkChoice {
            text: String::new(),
            index: 0,
            finish_reason: Some(stop_reason.to_string()),
            logprobs: None,
        }],
    });
    result
}

// ---------------------------------------------------------------------------
// Request extraction
// ---------------------------------------------------------------------------

/// Extract `(model, prompt)` from a legacy completions request body.
pub fn extract_request_info(body: &serde_json::Value) -> Result<(String, String), String> {
    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or("Missing or empty 'model' field in request")?
        .to_string();

    let prompt = body
        .get("prompt")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or("Missing or empty 'prompt' field in request")?
        .to_string();

    Ok((model, prompt))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_build_response() {
        let gen = IdGenerator::new();
        let resp = build_response(&gen, "davinci", "hello world", "say hi", "stop");
        assert_eq!(resp.object, "text_completion");
        assert_eq!(resp.choices.len(), 1);
        assert_eq!(resp.choices[0].text, "hello world");
        assert_eq!(resp.choices[0].finish_reason.as_deref(), Some("stop"));
        assert!(resp.id.starts_with("cmpl-llmposter-"));
    }

    #[test]
    fn should_build_stream_chunks() {
        let chunks = build_stream_chunks("cmpl-1", "davinci", "hello", 3, "stop");
        // "hel", "lo", "" (final)
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].choices[0].text, "hel");
        assert!(chunks[0].choices[0].finish_reason.is_none());
        assert_eq!(chunks[2].choices[0].text, "");
        assert_eq!(chunks[2].choices[0].finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn should_extract_request_info() {
        let body = serde_json::json!({"model": "davinci", "prompt": "hello"});
        let (model, prompt) = extract_request_info(&body).unwrap();
        assert_eq!(model, "davinci");
        assert_eq!(prompt, "hello");
    }

    #[test]
    fn should_reject_missing_prompt() {
        let body = serde_json::json!({"model": "davinci"});
        assert!(extract_request_info(&body).is_err());
    }

    #[test]
    fn should_reject_missing_model() {
        let body = serde_json::json!({"prompt": "hello"});
        assert!(extract_request_info(&body).is_err());
    }
}
