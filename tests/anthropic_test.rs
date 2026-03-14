use llmposter::fixture::{FailureConfig, FixtureResponse, ToolCall};
use llmposter::{Fixture, Provider, ServerBuilder};

#[tokio::test]
async fn should_return_anthropic_messages_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from Claude mock!"),
        )
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["type"], "message");
    assert_eq!(body["role"], "assistant");
    assert_eq!(body["content"][0]["type"], "text");
    assert_eq!(body["content"][0]["text"], "Hi from Claude mock!");
    assert_eq!(body["stop_reason"], "end_turn");
    assert!(body["id"].as_str().unwrap().starts_with("msg-llmposter-"));
    assert!(body["usage"]["input_tokens"].as_u64().unwrap() > 0);
    assert!(body["usage"]["output_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn should_return_400_for_unparseable_anthropic_request() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .header("content-type", "application/json")
        .body("not json")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_stream_anthropic_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hello world")
                .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await;

    let client = reqwest::Client::new();
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
    assert_eq!(
        resp.headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "text/event-stream"
    );

    let body = resp.text().await.unwrap();
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: content_block_delta"));
    assert!(body.contains("event: message_stop"));
}

#[tokio::test]
async fn should_handle_array_content_format() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("array content")
                .respond_with_content("got it"),
        )
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": [{"type": "text", "text": "array content"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_return_anthropic_tool_use_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "London", "unit": "celsius"}),
                }]),
        )
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "What's the weather in London?"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["type"], "message");
    assert_eq!(body["role"], "assistant");
    assert_eq!(body["stop_reason"], "tool_use");

    let content = body["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "tool_use");
    assert_eq!(content[0]["name"], "get_weather");
    assert_eq!(content[0]["id"], "toolu_llmposter_1");
    assert!(content[0]["input"].is_object());
    assert_eq!(content[0]["input"]["location"], "London");
    assert_eq!(content[0]["input"]["unit"], "celsius");
}

#[tokio::test]
async fn should_stream_anthropic_tool_use_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "Paris"}),
                }])
                .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "weather in Paris"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "text/event-stream"
    );

    let body = resp.text().await.unwrap();
    assert!(body.contains("event: message_start"));
    assert!(body.contains("event: message_stop"));
    assert!(body.contains("tool_use"));
    assert!(body.contains("get_weather"));
}

#[tokio::test]
async fn should_simulate_latency_on_anthropic() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("delayed anthropic")
                .with_failure(FailureConfig {
                    latency_ms: Some(200),
                    corrupt_body: None,
                    truncate_after_chunks: None,
                    disconnect_after_ms: None,
                }),
        )
        .build()
        .await;

    let client = reqwest::Client::new();
    let start = std::time::Instant::now();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["content"][0]["text"], "delayed anthropic");
    assert!(
        elapsed >= std::time::Duration::from_millis(180),
        "Expected at least 180ms delay, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn should_stream_anthropic_with_latency_between_chunks() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("Hello world test")
                .with_streaming(Some(50), Some(5)),
        )
        .build()
        .await;

    let client = reqwest::Client::new();
    let start = std::time::Instant::now();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let elapsed = start.elapsed();

    assert!(body.contains("event: message_stop"));
    // "Hello world test" = 16 chars, chunk_size 5 = 4 deltas
    // Plus content_block_start, content_block_stop, message_start, message_delta, message_stop = ~9 events
    // At least several events with 50ms latency
    assert!(
        elapsed >= std::time::Duration::from_millis(150),
        "Expected at least 150ms for streaming with latency, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn should_match_first_fixture_via_http_anthropic() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("first match"),
        )
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("second match"),
        )
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["content"][0]["text"], "first match");
}

#[tokio::test]
async fn should_not_match_openai_fixture_on_anthropic_endpoint() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("openai only")
                .for_provider(Provider::OpenAI),
        )
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_match_anthropic_provider_fixture_on_anthropic_endpoint() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("anthropic matched")
                .for_provider(Provider::Anthropic),
        )
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["content"][0]["text"], "anthropic matched");
}

#[tokio::test]
async fn should_match_model_filter_via_http_anthropic() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_model("claude-sonnet")
                .respond_with_content("sonnet response"),
        )
        .build()
        .await;

    let client = reqwest::Client::new();

    // Should match claude-sonnet-4-6
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["content"][0]["text"], "sonnet response");

    // Should NOT match claude-haiku
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-haiku-3",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_use_custom_stop_reason_anthropic() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content: Some("hit max tokens".to_string()),
                tool_calls: None,
                stop_reason: Some("max_tokens".to_string()),
                finish_reason: None,
            }),
            ..Fixture::new()
        })
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["stop_reason"], "max_tokens");
    assert_eq!(body["content"][0]["text"], "hit max tokens");
}

#[tokio::test]
async fn should_return_429_for_error_fixture_anthropic() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("rate limit")
                .with_error(429, "Rate limit exceeded"),
        )
        .build()
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "trigger rate limit please"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 429);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["message"], "Rate limit exceeded");
}
