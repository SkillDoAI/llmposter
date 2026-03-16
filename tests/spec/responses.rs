//! OpenAI Responses API spec compliance tests.
//!
//! Spec: https://platform.openai.com/docs/api-reference/responses/object
//!
//! Golden structs in `types::responses` are the source of truth.

use super::*;
use types::responses::*;

// ===========================================================================
// Shape compliance — non-streaming
// ===========================================================================

#[tokio::test]
async fn spec_responses_non_streaming_text_response_shape() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: SpecResponsesResponse = resp.json().await.unwrap();

    assert!(!body.id.is_empty());
    assert_eq!(body.object, "response");
    assert_eq!(body.status, "completed");
    assert!(!body.model.is_empty());
    assert!(!body.output.is_empty());

    // Parse first output item as message
    let item: SpecOutputMessage = serde_json::from_value(body.output[0].clone()).unwrap();
    assert_eq!(item.item_type, "message");
    assert_eq!(item.role, "assistant");
    assert!(!item.content.is_empty());
    assert_eq!(item.content[0].content_type, "output_text");
    assert_eq!(item.content[0].text, "world");

    // Usage
    assert!(body.usage.input_tokens > 0);
    assert!(body.usage.output_tokens > 0);
    assert_eq!(
        body.usage.total_tokens,
        body.usage.input_tokens + body.usage.output_tokens
    );
}

#[tokio::test]
async fn spec_responses_non_streaming_tool_call_response_shape() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "weather"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: SpecResponsesResponse = resp.json().await.unwrap();

    // Find function_call item
    let fc_item = body
        .output
        .iter()
        .find(|o| o.get("type").and_then(|v| v.as_str()) == Some("function_call"))
        .expect("must have function_call output item");

    let fc: SpecFunctionCallItem = serde_json::from_value(fc_item.clone()).unwrap();
    assert_eq!(fc.item_type, "function_call");
    assert!(!fc.id.is_empty());
    assert!(!fc.call_id.is_empty());
    assert_eq!(fc.name, "get_weather");
    // Arguments is a JSON string
    let parsed: serde_json::Value = serde_json::from_str(&fc.arguments).unwrap();
    assert_eq!(parsed["location"], "SF");
}

// ===========================================================================
// Shape compliance — streaming
// ===========================================================================

#[tokio::test]
async fn spec_responses_streaming_text_response_shape() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();

    // Should have response.created and response.completed events
    assert!(
        body.contains("response.created"),
        "must have response.created event"
    );
    assert!(
        body.contains("response.completed") || body.contains("response.done"),
        "must have terminal event"
    );
}

#[tokio::test]
async fn spec_responses_streaming_tool_call_response_shape() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "weather"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();

    // Must have function_call streaming events
    assert!(body.contains("response.output_item.added"));
    assert!(body.contains("response.function_call_arguments"));
}

// ===========================================================================
// Semantic compliance
// ===========================================================================

#[tokio::test]
async fn spec_responses_status_completed_for_text() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecResponsesResponse = resp.json().await.unwrap();
    assert_eq!(body.status, "completed");
}

#[tokio::test]
async fn spec_responses_object_is_response() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecResponsesResponse = resp.json().await.unwrap();
    assert_eq!(body.object, "response");
}

#[tokio::test]
async fn spec_responses_usage_total_equals_sum() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecResponsesResponse = resp.json().await.unwrap();
    assert_eq!(
        body.usage.total_tokens,
        body.usage.input_tokens + body.usage.output_tokens
    );
}

#[tokio::test]
async fn spec_responses_id_format() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecResponsesResponse = resp.json().await.unwrap();
    assert!(
        body.id.starts_with("resp"),
        "Responses API ID should start with 'resp', got: {}",
        body.id
    );
}

#[tokio::test]
async fn spec_responses_tool_call_arguments_are_json_string() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "weather"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecResponsesResponse = resp.json().await.unwrap();
    let fc_item = body
        .output
        .iter()
        .find(|o| o.get("type").and_then(|v| v.as_str()) == Some("function_call"))
        .unwrap();

    let args = fc_item["arguments"]
        .as_str()
        .expect("arguments must be a string");
    let parsed: serde_json::Value = serde_json::from_str(args).unwrap();
    assert_eq!(parsed["location"], "SF");
}

#[tokio::test]
async fn spec_responses_accepts_extra_request_fields() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hello"}],
            "temperature": 0.7,
            "max_output_tokens": 100,
            "metadata": {"session": "test-123"},
            "instructions": "Be helpful"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: SpecResponsesResponse = resp.json().await.unwrap();
    assert_eq!(body.status, "completed");
}
