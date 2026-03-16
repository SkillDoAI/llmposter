//! Gemini generateContent API spec compliance tests.
//!
//! Spec: https://ai.google.dev/api/generate-content
//!
//! Golden structs in `types::gemini` are the source of truth.

use super::*;
use types::gemini::*;

// ===========================================================================
// Shape compliance — non-streaming
// ===========================================================================

#[tokio::test]
async fn spec_gemini_non_streaming_text_response_shape() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: SpecGenerateContentResponse = resp.json().await.unwrap();

    assert!(!body.candidates.is_empty());
    let candidate = &body.candidates[0];
    assert_eq!(candidate.content.role.as_deref(), Some("model"));
    assert!(!candidate.content.parts.is_empty());
    assert_eq!(candidate.content.parts[0].text.as_deref(), Some("world"));

    // Usage metadata should be present
    assert!(body.usage_metadata.is_some());
    let usage = body.usage_metadata.as_ref().unwrap();
    assert!(usage.prompt_token_count.unwrap_or(0) > 0);
    assert!(usage.total_token_count.unwrap_or(0) > 0);
}

#[tokio::test]
async fn spec_gemini_non_streaming_tool_call_response_shape() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "weather"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: SpecGenerateContentResponse = resp.json().await.unwrap();

    let candidate = &body.candidates[0];
    let fc_part = candidate
        .content
        .parts
        .iter()
        .find(|p| p.function_call.is_some())
        .expect("must have function_call part");

    let fc = fc_part.function_call.as_ref().unwrap();
    assert_eq!(fc.name, "get_weather");
    // Gemini sends args as JSON object (not string)
    assert!(fc.args.is_object());
    assert_eq!(fc.args["location"], "SF");
}

// ===========================================================================
// Shape compliance — streaming
// ===========================================================================

#[tokio::test]
async fn spec_gemini_streaming_text_response_shape() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent?alt=sse",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let chunks: Vec<SpecGenerateContentResponse> = parse_sse_data(&body)
        .iter()
        .map(|data| serde_json::from_str(data).unwrap())
        .collect();

    assert!(!chunks.is_empty());
    for chunk in &chunks {
        assert!(!chunk.candidates.is_empty());
        assert_eq!(chunk.candidates[0].content.role.as_deref(), Some("model"));
    }
}

// ===========================================================================
// Semantic compliance
// ===========================================================================

#[tokio::test]
async fn spec_gemini_finish_reason_stop() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecGenerateContentResponse = resp.json().await.unwrap();
    // Gemini uses "STOP" (uppercase) for normal completion
    assert_eq!(body.candidates[0].finish_reason.as_deref(), Some("STOP"));
}

#[tokio::test]
async fn spec_gemini_uses_camel_case() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
        }))
        .send()
        .await
        .unwrap();

    // Check raw JSON for camelCase
    let raw: serde_json::Value = resp.json().await.unwrap();
    assert!(
        raw.get("usageMetadata").is_some(),
        "should use camelCase usageMetadata"
    );
    let usage = &raw["usageMetadata"];
    assert!(
        usage.get("promptTokenCount").is_some(),
        "should use camelCase promptTokenCount"
    );
    assert!(
        usage.get("totalTokenCount").is_some(),
        "should use camelCase totalTokenCount"
    );
}

#[tokio::test]
async fn spec_gemini_no_id_field() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
        }))
        .send()
        .await
        .unwrap();

    // Gemini responses don't have an 'id' field at the top level
    let raw: serde_json::Value = resp.json().await.unwrap();
    assert!(
        raw.get("id").is_none(),
        "Gemini response should not have 'id' field"
    );
}

#[tokio::test]
async fn spec_gemini_function_call_args_are_object() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "weather"}]}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecGenerateContentResponse = resp.json().await.unwrap();
    let fc = body.candidates[0]
        .content
        .parts
        .iter()
        .find_map(|p| p.function_call.as_ref())
        .unwrap();

    // Args must be a JSON object, not a string
    assert!(fc.args.is_object());
}

#[tokio::test]
async fn spec_gemini_streaming_only_last_chunk_has_finish_reason() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent?alt=sse",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();
    let chunks: Vec<SpecGenerateContentResponse> = parse_sse_data(&body)
        .iter()
        .map(|data| serde_json::from_str(data).unwrap())
        .collect();

    if chunks.len() > 1 {
        // Intermediate chunks should NOT have finish_reason
        for chunk in &chunks[..chunks.len() - 1] {
            assert!(
                chunk.candidates[0].finish_reason.is_none(),
                "intermediate chunks should not have finishReason"
            );
        }
    }
    // Last chunk has finish_reason
    assert!(!chunks.is_empty(), "no streaming chunks received");
    let last = chunks.last().unwrap();
    assert!(
        last.candidates[0].finish_reason.is_some(),
        "last chunk must have finishReason"
    );
}

#[tokio::test]
async fn spec_gemini_accepts_extra_request_fields() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}],
            "generationConfig": {
                "temperature": 0.7,
                "topP": 0.9,
                "topK": 40,
                "maxOutputTokens": 100
            },
            "safetySettings": []
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: SpecGenerateContentResponse = resp.json().await.unwrap();
    assert_eq!(
        body.candidates[0].content.parts[0].text.as_deref(),
        Some("world")
    );
}
