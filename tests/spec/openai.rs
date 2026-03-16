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
    for chunk in &chunks {
        assert!(!chunk.id.is_empty());
        assert_eq!(chunk.object, "chat.completion.chunk");
        assert!(chunk.created > 0, "every chunk must have created timestamp");
        assert!(!chunk.model.is_empty());
        assert!(!chunk.choices.is_empty());
    }

    // system_fingerprint should be present on first chunk
    assert!(
        chunks[0].system_fingerprint.is_some(),
        "first chunk should have system_fingerprint"
    );

    // service_tier should be present on first chunk
    assert!(
        chunks[0].service_tier.is_some(),
        "first chunk should have service_tier"
    );
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
                // index is a required u64 on SpecToolCallDelta — if it
                // deserialized, it's present. But let's be explicit:
                assert!(tc.index < 100, "tool call delta index should be reasonable");
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
