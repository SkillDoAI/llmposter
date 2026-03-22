//! Anthropic Messages API spec compliance tests.
//!
//! Spec: https://docs.anthropic.com/en/api/messages
//! Streaming: https://docs.anthropic.com/en/api/messages-streaming
//!
//! Golden structs in `types::anthropic` are the source of truth.

use super::*;
use types::anthropic::*;

/// Parse SSE body into (event_type, data_json) pairs for Anthropic events.
fn parse_anthropic_sse(body: &str) -> Vec<(String, String)> {
    let mut events = Vec::new();
    let mut current_event = String::new();
    let mut current_data = String::new();

    for line in body.lines() {
        if line.starts_with("event: ") {
            current_event = line.trim_start_matches("event: ").to_string();
            current_data.clear(); // defensive: discard stale data from incomplete block
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
    // Flush final event if body doesn't end with blank line
    if !current_event.is_empty() {
        events.push((current_event, current_data));
    }
    events
}

// ===========================================================================
// Shape compliance — non-streaming
// ===========================================================================

#[tokio::test]
async fn spec_anthropic_non_streaming_text_response_shape() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: SpecMessagesResponse = resp.json().await.unwrap();

    assert!(!body.id.is_empty(), "id must be non-empty");
    assert_eq!(body.msg_type, "message");
    assert_eq!(body.role, "assistant");
    assert!(!body.model.is_empty());
    assert!(!body.content.is_empty());

    // First content block should be text
    match &body.content[0] {
        SpecContentBlock::Text { text, .. } => {
            assert_eq!(text, "world");
        }
        _ => panic!("expected text content block"),
    }

    // Usage — must include cache token fields per latest spec
    assert!(body.usage.input_tokens > 0);
    assert!(body.usage.output_tokens > 0);
    // Cache fields are always emitted (0 when caching isn't used)
    assert_eq!(body.usage.cache_creation_input_tokens, 0);
    assert_eq!(body.usage.cache_read_input_tokens, 0);
}

#[tokio::test]
async fn spec_anthropic_non_streaming_tool_use_response_shape() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "weather"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: SpecMessagesResponse = resp.json().await.unwrap();

    // Should have a tool_use content block
    let has_tool_use = body
        .content
        .iter()
        .any(|c| matches!(c, SpecContentBlock::ToolUse { .. }));
    assert!(has_tool_use, "must have tool_use content block");

    // Check tool_use shape
    for block in &body.content {
        if let SpecContentBlock::ToolUse { id, name, input } = block {
            assert!(!id.is_empty());
            assert_eq!(name, "get_weather");
            // Input is a JSON object, not a string (unlike OpenAI)
            assert!(input.is_object(), "tool_use input must be a JSON object");
            assert_eq!(input["location"], "SF");
        }
    }
}

// ===========================================================================
// Shape compliance — streaming
// ===========================================================================

#[tokio::test]
async fn spec_anthropic_streaming_text_response_shape() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let events = parse_anthropic_sse(&body);

    assert!(!events.is_empty(), "must have streaming events");

    // Verify event types can be deserialized into spec structs
    for (event_type, data) in &events {
        match event_type.as_str() {
            "ping" => {
                let _: SpecPingEvent = serde_json::from_str(data).unwrap();
            }
            "message_start" => {
                let evt: SpecMessageStartEvent = serde_json::from_str(data).unwrap();
                assert_eq!(evt.event_type, "message_start");
                assert_eq!(evt.message.role, "assistant");
                assert_eq!(evt.message.msg_type, "message");
            }
            "content_block_start" => {
                let _: SpecContentBlockStartEvent = serde_json::from_str(data).unwrap();
            }
            "content_block_delta" => {
                let _: SpecContentBlockDeltaEvent = serde_json::from_str(data).unwrap();
            }
            "content_block_stop" => {
                let _: SpecContentBlockStopEvent = serde_json::from_str(data).unwrap();
            }
            "message_delta" => {
                let _: SpecMessageDeltaEvent = serde_json::from_str(data).unwrap();
            }
            "message_stop" => {
                let _: SpecMessageStopEvent = serde_json::from_str(data).unwrap();
            }
            other => panic!("unexpected event type: {}", other),
        }
    }
}

#[tokio::test]
async fn spec_anthropic_streaming_tool_use_response_shape() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "weather"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let events = parse_anthropic_sse(&body);

    // Must have content_block_start with tool_use type
    let has_tool_start = events
        .iter()
        .any(|(et, data)| et == "content_block_start" && data.contains("tool_use"));
    assert!(
        has_tool_start,
        "streaming tool call must have content_block_start with tool_use"
    );

    // Must have input_json_delta
    let has_json_delta = events
        .iter()
        .any(|(et, data)| et == "content_block_delta" && data.contains("input_json_delta"));
    assert!(
        has_json_delta,
        "streaming tool call must have input_json_delta"
    );
}

// ===========================================================================
// Semantic compliance
// ===========================================================================

#[tokio::test]
async fn spec_anthropic_stop_reason_end_turn_for_text() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecMessagesResponse = resp.json().await.unwrap();
    assert_eq!(body.stop_reason.as_deref(), Some("end_turn"));
}

#[tokio::test]
async fn spec_anthropic_stop_reason_tool_use_for_tools() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "weather"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecMessagesResponse = resp.json().await.unwrap();
    assert_eq!(body.stop_reason.as_deref(), Some("tool_use"));
}

#[tokio::test]
async fn spec_anthropic_type_field_is_message() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecMessagesResponse = resp.json().await.unwrap();
    assert_eq!(body.msg_type, "message");
    assert_eq!(body.role, "assistant");
}

#[tokio::test]
async fn spec_anthropic_streaming_event_order() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();
    let events = parse_anthropic_sse(&body);
    let event_types: Vec<&str> = events.iter().map(|(et, _)| et.as_str()).collect();

    // Verify ordering: ping → message_start → content_block_start →
    // content_block_delta(s) → content_block_stop → message_delta → message_stop
    assert!(
        event_types.len() >= 4,
        "expected at least 4 streaming events, got {}",
        event_types.len()
    );
    assert_eq!(event_types[0], "ping", "first event must be ping");
    assert_eq!(
        event_types[1], "message_start",
        "second event must be message_start"
    );

    // Must end with message_delta then message_stop
    let len = event_types.len();
    assert_eq!(event_types[len - 1], "message_stop");
    assert_eq!(event_types[len - 2], "message_delta");
}

#[tokio::test]
async fn spec_anthropic_streaming_message_start_has_usage() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();
    let events = parse_anthropic_sse(&body);

    let (_, data) = events
        .iter()
        .find(|(et, _)| et == "message_start")
        .expect("must have message_start");

    let evt: SpecMessageStartEvent = serde_json::from_str(data).unwrap();
    assert!(evt.message.usage.input_tokens > 0);
    assert_eq!(evt.message.usage.output_tokens, 0); // 0 at start
                                                    // Cache fields must be present in streaming too
    assert_eq!(evt.message.usage.cache_creation_input_tokens, 0);
    assert_eq!(evt.message.usage.cache_read_input_tokens, 0);
}

#[tokio::test]
async fn spec_anthropic_tool_use_input_is_object() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "weather"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecMessagesResponse = resp.json().await.unwrap();
    for block in &body.content {
        if let SpecContentBlock::ToolUse { input, .. } = block {
            // Anthropic sends tool input as a JSON object (not a string like OpenAI)
            assert!(
                input.is_object(),
                "tool_use input must be JSON object, not string"
            );
        }
    }
}

#[tokio::test]
async fn spec_anthropic_id_format() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecMessagesResponse = resp.json().await.unwrap();
    assert!(
        body.id.starts_with("msg_") || body.id.starts_with("msg-"),
        "Anthropic message ID should start with 'msg_' or 'msg-', got: {}",
        body.id
    );
}

#[tokio::test]
async fn spec_anthropic_accepts_extra_request_fields() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}],
            "temperature": 0.7,
            "top_p": 0.9,
            "top_k": 40,
            "system": "You are a helpful assistant",
            "metadata": {"user_id": "test-123"}
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: SpecMessagesResponse = resp.json().await.unwrap();
    match &body.content[0] {
        SpecContentBlock::Text { text, .. } => assert_eq!(text, "world"),
        _ => panic!("expected text"),
    }
}

// ===========================================================================
// Error response compliance
// ===========================================================================

#[tokio::test]
async fn spec_anthropic_error_429_shape() {
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
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "rate-limited",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 429);
    // Check headers before consuming response body
    assert!(resp.headers().get("x-request-id").is_some());
    let body: SpecAnthropicErrorResponse = resp.json().await.unwrap();
    assert_eq!(body.resp_type, "error");
    assert_eq!(body.error.error_type, "rate_limit_error");
    assert_eq!(body.error.message, "Rate limit exceeded");
}

#[tokio::test]
async fn spec_anthropic_error_500_shape() {
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
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "broken",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 500);
    let body: SpecAnthropicErrorResponse = resp.json().await.unwrap();
    assert_eq!(body.resp_type, "error");
    assert_eq!(body.error.error_type, "api_error");
}

#[tokio::test]
async fn spec_anthropic_error_400_shape() {
    let (server, client) = server_with_text("hello", "world").await;

    // Missing messages field
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({ "model": "claude-sonnet-4-6" }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: SpecAnthropicErrorResponse = resp.json().await.unwrap();
    assert_eq!(body.resp_type, "error");
    assert_eq!(body.error.error_type, "invalid_request_error");
}

// ===========================================================================
// Response headers
// ===========================================================================

#[tokio::test]
async fn spec_anthropic_request_id_header() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}]
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

// ===========================================================================
// Streaming message_delta compliance
// ===========================================================================

#[tokio::test]
async fn spec_anthropic_streaming_message_delta_has_full_usage() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();
    let events = parse_anthropic_sse(&body);

    let (_, data) = events
        .iter()
        .find(|(et, _)| et == "message_delta")
        .expect("must have message_delta");

    let evt: SpecMessageDeltaEvent = serde_json::from_str(data).unwrap();
    // message_delta usage must include all token fields with correct values
    assert!(evt.usage.input_tokens > 0, "input_tokens must be positive");
    assert!(
        evt.usage.output_tokens > 0,
        "output_tokens must be positive"
    );
    assert_eq!(evt.usage.cache_creation_input_tokens, 0);
    assert_eq!(evt.usage.cache_read_input_tokens, 0);
}
