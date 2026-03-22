//! OpenAI Responses API spec compliance tests.
//!
//! Spec: https://platform.openai.com/docs/api-reference/responses/object
//!
//! Golden structs in `types::responses` are the source of truth.

use super::*;
use types::openai::SpecErrorResponse;
use types::responses::*;

/// Parse SSE body into (event_type, data_json) pairs for Responses API events.
fn parse_responses_sse(body: &str) -> Vec<(String, String)> {
    let mut events = Vec::new();
    let mut current_event = String::new();
    let mut current_data = String::new();

    for line in body.lines() {
        if line.starts_with("event: ") {
            current_event = line.trim_start_matches("event: ").to_string();
            current_data.clear();
        } else if line.starts_with("data: ") {
            let payload = line.trim_start_matches("data: ");
            if !current_data.is_empty() {
                current_data.push('\n');
            }
            current_data.push_str(payload);
        } else if line.is_empty() {
            if !current_event.is_empty() {
                events.push((current_event.clone(), current_data.clone()));
                current_event.clear();
            }
            current_data.clear();
        }
    }
    if !current_event.is_empty() {
        events.push((current_event, current_data));
    }
    events
}

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
    assert_eq!(item.content[0].text.as_deref(), Some("world"));

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
    let events = parse_responses_sse(&body);

    assert!(!events.is_empty(), "must have streaming events");
    let event_types: Vec<&str> = events.iter().map(|(et, _)| et.as_str()).collect();

    // Verify event ordering per Responses streaming spec
    assert_eq!(event_types[0], "response.created");
    assert_eq!(event_types[1], "response.in_progress");
    assert!(event_types.contains(&"response.output_item.added"));
    assert!(event_types.contains(&"response.content_part.added"));
    assert!(event_types.contains(&"response.output_text.delta"));
    assert!(event_types.contains(&"response.output_text.done"));
    assert!(event_types.contains(&"response.content_part.done"));
    assert!(event_types.contains(&"response.output_item.done"));
    assert_eq!(*event_types.last().unwrap(), "response.completed");

    // Verify response.created has nested envelope with "response" key
    let created_data: serde_json::Value = serde_json::from_str(&events[0].1).unwrap();
    assert_eq!(created_data["type"], "response.created");
    assert!(
        created_data.get("response").is_some(),
        "response.created must have nested 'response' envelope"
    );
    assert_eq!(created_data["response"]["status"], "in_progress");
    assert!(created_data.get("sequence_number").is_some());

    // Verify response.completed has nested envelope
    let completed = events
        .iter()
        .find(|(et, _)| et == "response.completed")
        .unwrap();
    let completed_data: serde_json::Value = serde_json::from_str(&completed.1).unwrap();
    assert_eq!(completed_data["type"], "response.completed");
    assert!(completed_data.get("response").is_some());
    assert_eq!(completed_data["response"]["status"], "completed");

    // Verify delta events have correlation fields
    let delta = events
        .iter()
        .find(|(et, _)| et == "response.output_text.delta")
        .unwrap();
    let delta_data: serde_json::Value = serde_json::from_str(&delta.1).unwrap();
    assert!(
        delta_data.get("item_id").is_some(),
        "delta must have item_id"
    );
    assert!(delta_data.get("output_index").is_some());
    assert!(delta_data.get("content_index").is_some());
    assert!(delta_data.get("sequence_number").is_some());
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
    let events = parse_responses_sse(&body);

    let event_types: Vec<&str> = events.iter().map(|(et, _)| et.as_str()).collect();

    // Verify event ordering
    assert_eq!(event_types[0], "response.created");
    assert_eq!(event_types[1], "response.in_progress");
    assert!(event_types.contains(&"response.output_item.added"));
    assert!(event_types.contains(&"response.function_call_arguments.delta"));
    assert!(event_types.contains(&"response.function_call_arguments.done"));
    assert!(event_types.contains(&"response.output_item.done"));
    assert_eq!(*event_types.last().unwrap(), "response.completed");

    // Verify response.created has nested envelope
    let created_data: serde_json::Value = serde_json::from_str(&events[0].1).unwrap();
    assert!(created_data.get("response").is_some());

    // Verify output_item.added preserves arguments (not stripped)
    let added = events
        .iter()
        .find(|(et, _)| et == "response.output_item.added")
        .unwrap();
    let added_data: serde_json::Value = serde_json::from_str(&added.1).unwrap();
    let item = &added_data["item"];
    assert_eq!(item["type"], "function_call");
    // arguments should be present on the added item
    assert!(
        item.get("arguments").is_some(),
        "output_item.added must preserve arguments"
    );

    // Verify function_call_arguments.done has all fields
    let fc_done = events
        .iter()
        .find(|(et, _)| et == "response.function_call_arguments.done")
        .unwrap();
    let fc_done_data: serde_json::Value = serde_json::from_str(&fc_done.1).unwrap();
    assert!(fc_done_data.get("item_id").is_some());
    assert!(
        fc_done_data.get("call_id").is_some(),
        "function_call_arguments.done must have call_id"
    );
    assert!(fc_done_data.get("output_index").is_some());
    assert!(fc_done_data.get("arguments").is_some());
    assert!(fc_done_data.get("sequence_number").is_some());
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

// ===========================================================================
// Error response compliance
// ===========================================================================

#[tokio::test]
async fn spec_responses_error_429_shape() {
    let server = llmposter::ServerBuilder::new()
        .fixture(
            llmposter::Fixture::new()
                .match_model("rate-limited")
                .with_error(429, "Rate limit exceeded"),
        )
        .build()
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "rate-limited",
            "input": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 429);
    assert!(resp.headers().get("x-request-id").is_some());
    let body: SpecErrorResponse = resp.json().await.unwrap();
    assert_eq!(body.error.error_type, "rate_limit_error");
    assert_eq!(body.error.code.as_deref(), Some("rate_limit_exceeded"));
    assert_eq!(body.error.message, "Rate limit exceeded");
}

#[tokio::test]
async fn spec_responses_error_401_shape() {
    let server = llmposter::ServerBuilder::new()
        .fixture(
            llmposter::Fixture::new()
                .match_model("noauth")
                .with_error(401, "Invalid API key"),
        )
        .build()
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "noauth",
            "input": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
    let body: SpecErrorResponse = resp.json().await.unwrap();
    assert_eq!(body.error.error_type, "authentication_error");
    assert_eq!(body.error.code.as_deref(), Some("invalid_api_key"));
}

#[tokio::test]
async fn spec_responses_error_500_shape() {
    let server = llmposter::ServerBuilder::new()
        .fixture(
            llmposter::Fixture::new()
                .match_model("broken")
                .with_error(500, "Internal error"),
        )
        .build()
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "broken",
            "input": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 500);
    let body: SpecErrorResponse = resp.json().await.unwrap();
    assert_eq!(body.error.error_type, "server_error");
    assert_eq!(body.error.message, "Internal error");
}

#[tokio::test]
async fn spec_responses_error_400_shape() {
    let server = llmposter::ServerBuilder::new()
        .fixture(
            llmposter::Fixture::new()
                .match_model("bad")
                .with_error(400, "Bad request"),
        )
        .build()
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "bad",
            "input": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: SpecErrorResponse = resp.json().await.unwrap();
    assert_eq!(body.error.error_type, "invalid_request_error");
}

// ===========================================================================
// Response headers
// ===========================================================================

#[tokio::test]
async fn spec_responses_request_id_header() {
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
    let request_id = resp
        .headers()
        .get("x-request-id")
        .expect("must have x-request-id")
        .to_str()
        .unwrap();
    assert!(request_id.starts_with("req-llmposter-"));
}

#[tokio::test]
async fn spec_responses_rate_limit_headers_on_429() {
    let server = llmposter::ServerBuilder::new()
        .fixture(
            llmposter::Fixture::new()
                .match_model("rate-limited")
                .with_error(429, "Rate limit exceeded"),
        )
        .build()
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "rate-limited",
            "input": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 429);
    // Responses API uses OpenAI-style rate limit headers
    assert_eq!(
        resp.headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("60")
    );
    assert_eq!(
        resp.headers()
            .get("x-ratelimit-limit-requests")
            .and_then(|v| v.to_str().ok()),
        Some("100")
    );
    assert_eq!(
        resp.headers()
            .get("x-ratelimit-remaining-requests")
            .and_then(|v| v.to_str().ok()),
        Some("0")
    );
    assert_eq!(
        resp.headers()
            .get("x-ratelimit-reset-requests")
            .and_then(|v| v.to_str().ok()),
        Some("1m0s")
    );
    assert!(resp.headers().get("x-request-id").is_some());
}

#[tokio::test]
async fn spec_responses_no_rate_limit_headers_on_200() {
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
    for name in [
        "retry-after",
        "x-ratelimit-limit-requests",
        "x-ratelimit-remaining-requests",
        "x-ratelimit-reset-requests",
    ] {
        assert!(
            resp.headers().get(name).is_none(),
            "200 response should not have {}",
            name
        );
    }
    assert!(resp.headers().get("x-request-id").is_some());
}
