//! OpenAI Chat Completions API spec compliance tests.
//!
//! These tests validate that llmposter's OpenAI responses match the real API spec:
//! https://platform.openai.com/docs/api-reference/chat/object
//! https://platform.openai.com/docs/api-reference/chat/streaming
//!
//! Golden structs in `types::openai` are the source of truth — derived directly
//! from the API docs. If our response doesn't deserialize into them, we're
//! out of spec.

use super::*;
use types::openai::*;

// ===========================================================================
// Shape compliance — non-streaming
// ===========================================================================

#[tokio::test]
async fn spec_openai_non_streaming_text_response_shape() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    // This is the core assertion: if deserialization succeeds, every field
    // in the spec struct is present and correctly typed.
    let body: SpecChatCompletion = resp.json().await.unwrap();

    // Required fields are non-empty
    assert!(!body.id.is_empty(), "id must be non-empty");
    assert_eq!(body.object, "chat.completion");
    assert!(body.created > 0, "created must be positive unix timestamp");
    assert!(!body.model.is_empty(), "model must be non-empty");
    assert!(
        !body.choices.is_empty(),
        "choices must have at least one entry"
    );

    // Optional metadata fields we emit
    assert!(
        body.system_fingerprint.is_some(),
        "system_fingerprint should be present"
    );
    assert!(
        body.service_tier.is_some(),
        "service_tier should be present"
    );

    // Choice shape
    let choice = &body.choices[0];
    assert_eq!(choice.index, 0);
    assert_eq!(choice.message.role, "assistant");
    assert_eq!(choice.message.content.as_deref(), Some("world"));
    assert!(
        choice.message.tool_calls.is_none(),
        "text response should not have tool_calls"
    );

    // Usage shape
    assert!(body.usage.prompt_tokens > 0);
    assert!(body.usage.completion_tokens > 0);
}

#[tokio::test]
async fn spec_openai_non_streaming_tool_call_response_shape() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "weather"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: SpecChatCompletion = resp.json().await.unwrap();

    let choice = &body.choices[0];
    assert!(
        choice.message.content.is_none(),
        "tool call response should not have text content"
    );

    let tool_calls = choice
        .message
        .tool_calls
        .as_ref()
        .expect("tool_calls must be present");
    assert!(!tool_calls.is_empty());

    let tc = &tool_calls[0];
    assert!(!tc.id.is_empty(), "tool call id must be non-empty");
    assert_eq!(tc.call_type, "function");
    assert_eq!(tc.function.name, "get_weather");

    // Arguments must be a JSON string (not a raw object)
    let parsed: serde_json::Value =
        serde_json::from_str(&tc.function.arguments).expect("arguments must be valid JSON string");
    assert_eq!(parsed["location"], "SF");
}

// ===========================================================================
// Shape compliance — streaming
// ===========================================================================

#[tokio::test]
async fn spec_openai_streaming_text_response_shape() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let chunks: Vec<SpecChatCompletionChunk> = parse_sse_data(&body)
        .iter()
        .map(|data| serde_json::from_str(data).unwrap())
        .collect();

    assert!(!chunks.is_empty(), "must have at least one chunk");

    // Every chunk must deserialize and have required fields
    for (i, chunk) in chunks.iter().enumerate() {
        assert!(!chunk.id.is_empty());
        assert_eq!(chunk.object, "chat.completion.chunk");
        assert!(chunk.created > 0, "every chunk must have created timestamp");
        assert!(!chunk.model.is_empty());
        assert!(!chunk.choices.is_empty());
        // system_fingerprint must be on ALL chunks per OpenAI streaming spec
        assert!(
            chunk.system_fingerprint.is_some(),
            "chunk {} must have system_fingerprint",
            i
        );
    }

    // service_tier: present on first chunk, absent on rest (matches real API)
    assert!(
        chunks[0].service_tier.is_some(),
        "first chunk should have service_tier"
    );
    for chunk in &chunks[1..] {
        assert!(
            chunk.service_tier.is_none(),
            "non-first chunks should not have service_tier"
        );
    }
}

#[tokio::test]
async fn spec_openai_streaming_tool_call_response_shape() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "weather"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let chunks: Vec<SpecChatCompletionChunk> = parse_sse_data(&body)
        .iter()
        .map(|data| serde_json::from_str(data).unwrap())
        .collect();

    assert!(!chunks.is_empty());

    // Every chunk must have required fields
    for (i, chunk) in chunks.iter().enumerate() {
        assert!(!chunk.id.is_empty(), "chunk {} must have non-empty id", i);
        assert_eq!(chunk.object, "chat.completion.chunk");
        assert!(chunk.created > 0, "chunk {} must have created > 0", i);
        assert!(!chunk.model.is_empty());
        // Note: real API may send empty choices on usage-only final chunk
        // (stream_options.include_usage). We don't emit those yet.
        assert!(!chunk.choices.is_empty());
        // system_fingerprint must be on ALL chunks
        assert!(
            chunk.system_fingerprint.is_some(),
            "chunk {} must have system_fingerprint",
            i
        );
    }

    // service_tier: present on first chunk, absent on rest
    assert!(
        chunks[0].service_tier.is_some(),
        "first chunk should have service_tier"
    );
    for chunk in &chunks[1..] {
        assert!(
            chunk.service_tier.is_none(),
            "non-first chunks should not have service_tier"
        );
    }

    // Find the chunk that has tool_calls in the delta
    let tool_chunk = chunks
        .iter()
        .find(|c| c.choices[0].delta.tool_calls.is_some())
        .expect("must have a chunk with tool_calls delta");

    let tool_calls = tool_chunk.choices[0].delta.tool_calls.as_ref().unwrap();
    let tc = &tool_calls[0];
    // Index is required in streaming tool call deltas
    assert_eq!(tc.index, 0);
}

// ===========================================================================
// Semantic compliance
// ===========================================================================

#[tokio::test]
async fn spec_openai_finish_reason_stop_for_text() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecChatCompletion = resp.json().await.unwrap();
    assert_eq!(body.choices[0].finish_reason, "stop");
}

#[tokio::test]
async fn spec_openai_finish_reason_tool_calls_for_tools() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "weather"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecChatCompletion = resp.json().await.unwrap();
    assert_eq!(body.choices[0].finish_reason, "tool_calls");
}

#[tokio::test]
async fn spec_openai_streaming_first_chunk_has_role() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();
    let chunks: Vec<SpecChatCompletionChunk> = parse_sse_data(&body)
        .iter()
        .map(|data| serde_json::from_str(data).unwrap())
        .collect();

    assert!(!chunks.is_empty(), "no streaming chunks received");
    assert_eq!(
        chunks[0].choices[0].delta.role.as_deref(),
        Some("assistant"),
        "first streaming chunk must have role=assistant"
    );
}

#[tokio::test]
async fn spec_openai_streaming_last_chunk_has_finish_reason() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();
    let chunks: Vec<SpecChatCompletionChunk> = parse_sse_data(&body)
        .iter()
        .map(|data| serde_json::from_str(data).unwrap())
        .collect();

    assert!(!chunks.is_empty(), "no streaming chunks received");
    let last = chunks.last().unwrap();
    assert!(
        last.choices[0].finish_reason.is_some(),
        "last chunk must have finish_reason"
    );
    assert_eq!(last.choices[0].finish_reason.as_deref(), Some("stop"));
}

#[tokio::test]
async fn spec_openai_streaming_ends_with_done_sentinel() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();
    assert!(
        has_done_sentinel(&body),
        "streaming response must end with data: [DONE]"
    );
}

#[tokio::test]
async fn spec_openai_streaming_chunks_have_created_timestamp() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();
    let chunks: Vec<SpecChatCompletionChunk> = parse_sse_data(&body)
        .iter()
        .map(|data| serde_json::from_str(data).unwrap())
        .collect();

    for (i, chunk) in chunks.iter().enumerate() {
        assert!(chunk.created > 0, "chunk {} must have created > 0", i);
    }
}

#[tokio::test]
async fn spec_openai_object_field_is_chat_completion() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecChatCompletion = resp.json().await.unwrap();
    assert_eq!(body.object, "chat.completion");
}

#[tokio::test]
async fn spec_openai_chunk_object_field_is_chat_completion_chunk() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();
    let chunks: Vec<SpecChatCompletionChunk> = parse_sse_data(&body)
        .iter()
        .map(|data| serde_json::from_str(data).unwrap())
        .collect();

    for chunk in &chunks {
        assert_eq!(chunk.object, "chat.completion.chunk");
    }
}

#[tokio::test]
async fn spec_openai_usage_total_equals_prompt_plus_completion() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecChatCompletion = resp.json().await.unwrap();
    assert_eq!(
        body.usage.total_tokens,
        body.usage.prompt_tokens + body.usage.completion_tokens,
        "total_tokens must equal prompt_tokens + completion_tokens"
    );
}

#[tokio::test]
async fn spec_openai_tool_call_arguments_are_json_string() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "weather"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecChatCompletion = resp.json().await.unwrap();
    let args = &body.choices[0].message.tool_calls.as_ref().unwrap()[0]
        .function
        .arguments;

    // Must be a JSON string that parses to an object — not a raw object
    assert!(
        args.starts_with('{'),
        "arguments should be a JSON string starting with '{{'"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(args).expect("arguments must be a parseable JSON string");
    assert!(parsed.is_object());
}

#[tokio::test]
async fn spec_openai_system_fingerprint_present() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecChatCompletion = resp.json().await.unwrap();
    let fp = body
        .system_fingerprint
        .as_ref()
        .expect("system_fingerprint must be present");
    assert!(
        fp.starts_with("fp_"),
        "system_fingerprint should start with 'fp_'"
    );
}

#[tokio::test]
async fn spec_openai_id_format_starts_with_chatcmpl() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecChatCompletion = resp.json().await.unwrap();
    assert!(
        body.id.starts_with("chatcmpl-"),
        "id '{}' should start with 'chatcmpl-'",
        body.id
    );
}

#[tokio::test]
async fn spec_openai_streaming_tool_call_deltas_have_index() {
    let (server, client) = server_with_tool_call(
        "weather",
        "get_weather",
        serde_json::json!({"location": "SF"}),
    )
    .await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "weather"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();
    let chunks: Vec<SpecChatCompletionChunk> = parse_sse_data(&body)
        .iter()
        .map(|data| serde_json::from_str(data).unwrap())
        .collect();

    for chunk in &chunks {
        if let Some(tool_calls) = &chunk.choices[0].delta.tool_calls {
            for tc in tool_calls {
                assert_eq!(tc.index, 0, "single tool call delta should have index 0");
            }
        }
    }
}

// ===========================================================================
// Request tolerance — accept unknown fields silently
// ===========================================================================

#[tokio::test]
async fn spec_openai_accepts_extra_request_fields() {
    let (server, client) = server_with_text("hello", "world").await;

    // Send a request with many real OpenAI parameters we don't use
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}],
            "temperature": 0.7,
            "top_p": 0.9,
            "max_tokens": 100,
            "presence_penalty": 0.0,
            "frequency_penalty": 0.0,
            "n": 1,
            "seed": 42,
            "user": "test-user"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: SpecChatCompletion = resp.json().await.unwrap();
    assert_eq!(body.choices[0].message.content.as_deref(), Some("world"));
}

// ===========================================================================
// Error response compliance
// ===========================================================================

#[tokio::test]
async fn spec_openai_error_429_shape() {
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
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "rate-limited",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 429);
    // Parse as raw Value first to verify param is present-as-null in the JSON
    let raw: serde_json::Value = resp.json().await.unwrap();
    assert!(
        raw["error"].get("param").is_some(),
        "param field must be present in error JSON"
    );
    assert!(raw["error"]["param"].is_null(), "param must be null");
    // Then verify via golden struct
    let body: SpecErrorResponse = serde_json::from_value(raw).unwrap();
    assert_eq!(body.error.message, "Rate limit exceeded");
    assert_eq!(body.error.error_type, "rate_limit_error");
    assert_eq!(body.error.code.as_deref(), Some("rate_limit_exceeded"));
}

#[tokio::test]
async fn spec_openai_error_500_shape() {
    let server = llmposter::ServerBuilder::new()
        .fixture(
            llmposter::Fixture::new()
                .match_model("broken")
                .with_error(500, "Internal server error"),
        )
        .build()
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "broken",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 500);
    let body: SpecErrorResponse = resp.json().await.unwrap();
    assert_eq!(body.error.error_type, "server_error");
    assert_eq!(body.error.code.as_deref(), Some("server_error"));
}

#[tokio::test]
async fn spec_openai_error_400_shape() {
    let (server, client) = server_with_text("hello", "world").await;

    // Send malformed request — missing messages field
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({ "model": "gpt-4" }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    // Parse as raw Value first to verify param is present-as-null in the JSON
    let raw: serde_json::Value = resp.json().await.unwrap();
    assert!(
        raw["error"].get("param").is_some(),
        "param field must be present in error JSON"
    );
    assert!(raw["error"]["param"].is_null(), "param must be null");
    // Then verify via golden struct
    let body: SpecErrorResponse = serde_json::from_value(raw).unwrap();
    assert_eq!(body.error.error_type, "invalid_request_error");
}

// ===========================================================================
// Edge cases
// ===========================================================================

#[tokio::test]
async fn spec_openai_multiple_tool_calls() {
    let server = llmposter::ServerBuilder::new()
        .fixture(
            llmposter::Fixture::new()
                .match_user_message("multi")
                .respond_with_tool_calls(vec![
                    llmposter::ToolCall {
                        name: "get_weather".to_string(),
                        arguments: serde_json::json!({"location": "NYC"}),
                    },
                    llmposter::ToolCall {
                        name: "get_time".to_string(),
                        arguments: serde_json::json!({"timezone": "UTC"}),
                    },
                ]),
        )
        .build()
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "multi"}]
        }))
        .send()
        .await
        .unwrap();

    let body: SpecChatCompletion = resp.json().await.unwrap();
    let tool_calls = body.choices[0].message.tool_calls.as_ref().unwrap();
    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0].function.name, "get_weather");
    assert_eq!(tool_calls[1].function.name, "get_time");
    // Each tool call has a unique ID
    assert_ne!(tool_calls[0].id, tool_calls[1].id);
}

#[tokio::test]
async fn spec_openai_streaming_stop_chunk_has_empty_delta() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();
    let raw_chunks: Vec<serde_json::Value> = parse_sse_data(&body)
        .iter()
        .map(|d| serde_json::from_str(d).unwrap())
        .collect();

    // Find the stop chunk (has finish_reason)
    let stop_chunk = raw_chunks
        .iter()
        .find(|c| c["choices"][0]["finish_reason"].is_string())
        .expect("must have stop chunk");

    // Stop chunk delta should be empty object (no content, no role, no tool_calls)
    let delta = &stop_chunk["choices"][0]["delta"];
    assert!(
        delta.as_object().map(|o| o.is_empty()).unwrap_or(false),
        "stop chunk delta should be empty object {{}}, got: {}",
        delta
    );
}

// ===========================================================================
// Response headers
// ===========================================================================

#[tokio::test]
async fn spec_openai_request_id_header() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let request_id = resp
        .headers()
        .get("x-request-id")
        .expect("must have x-request-id header")
        .to_str()
        .unwrap();
    assert!(
        request_id.starts_with("req-llmposter-"),
        "request-id should start with 'req-llmposter-', got: {}",
        request_id
    );
}

#[tokio::test]
async fn spec_openai_rate_limit_headers_on_429() {
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
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "rate-limited",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 429);

    // Must have rate limit headers with correct default values
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
        Some("60")
    );

    // Must also have x-request-id
    assert!(resp.headers().get("x-request-id").is_some());
}

#[tokio::test]
async fn spec_openai_no_rate_limit_headers_on_200() {
    let (server, client) = server_with_text("hello", "world").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    // Should NOT have any rate limit headers on success
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

    // But should still have x-request-id
    assert!(resp.headers().get("x-request-id").is_some());
}

// ===========================================================================
// OpenAI gap closure tests
// ===========================================================================

#[tokio::test]
async fn spec_openai_empty_content_vs_null() {
    // Test that empty string content produces content: "" (not null)
    let (server, client) = server_with_text("empty", "").await;

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "empty"}]
        }))
        .send()
        .await
        .unwrap();

    let raw: serde_json::Value = resp.json().await.unwrap();
    let content = &raw["choices"][0]["message"]["content"];
    // Empty string fixture should produce content: "" (not null)
    assert!(
        content.is_string(),
        "empty content should serialize as string, got: {}",
        content
    );
    assert_eq!(content.as_str().unwrap(), "");
}
