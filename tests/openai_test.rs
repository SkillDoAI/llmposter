use llmposter::fixture::{FailureConfig, FixtureResponse, ToolCall};
use llmposter::{Fixture, Provider, ServerBuilder};

#[tokio::test]
async fn should_return_openai_chat_completion() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from mock!"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "Hi from mock!");
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
    assert_eq!(body["object"], "chat.completion");
    assert!(body["id"]
        .as_str()
        .unwrap()
        .starts_with("chatcmpl-llmposter-"));
    assert!(body["usage"]["prompt_tokens"].as_u64().unwrap() > 0);
    assert!(body["usage"]["completion_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn should_return_404_when_no_fixture_matches() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("specific")
                .respond_with_content("specific response"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "unmatched"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("No fixture matched"));
}

#[tokio::test]
async fn should_return_error_fixture() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().with_error(429, "Rate limit exceeded"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "anything"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 429);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["message"], "Rate limit exceeded");
}

#[tokio::test]
async fn should_return_400_for_unparseable_json() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("content-type", "application/json")
        .body("not json at all")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_return_400_for_missing_messages() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({"model": "gpt-4"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_stream_openai_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("Hello world")
                .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
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
    assert!(body.contains("data: "));
    assert!(body.contains("data: [DONE]"));
}

#[tokio::test]
async fn should_have_independent_id_counters_per_server() {
    let server1 = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("one"))
        .build()
        .await
        .unwrap();
    let server2 = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("two"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let req_body = serde_json::json!({
        "model": "x",
        "messages": [{"role": "user", "content": "hi"}]
    });

    let resp1: serde_json::Value = client
        .post(format!("{}/v1/chat/completions", server1.url()))
        .json(&req_body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let resp2: serde_json::Value = client
        .post(format!("{}/v1/chat/completions", server2.url()))
        .json(&req_body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp1["id"], "chatcmpl-llmposter-1");
    assert_eq!(resp2["id"], "chatcmpl-llmposter-1");
}

#[tokio::test]
async fn should_simulate_corrupt_body() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("should not appear")
                .with_failure(FailureConfig {
                    latency_ms: None,
                    corrupt_body: Some(true),
                    truncate_after_frames: None,
                    disconnect_after_ms: None,
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(ct.contains("text/plain"));
    let body = resp.text().await.unwrap();
    assert_eq!(body, "overloaded");
}

#[tokio::test]
async fn should_simulate_truncated_stream() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content(
                    "This is a long response that should be truncated after 2 chunks",
                )
                .with_streaming(Some(0), Some(5))
                .with_failure(FailureConfig {
                    latency_ms: None,
                    corrupt_body: None,
                    truncate_after_frames: Some(2),
                    disconnect_after_ms: None,
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("data: "));
    assert!(!body.contains("[DONE]"));
}

#[tokio::test]
async fn should_return_openai_tool_call_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "San Francisco", "unit": "celsius"}),
                }]),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "What's the weather?"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["choices"][0]["finish_reason"], "tool_calls");
    assert!(body["choices"][0]["message"]["content"].is_null());

    let tool_calls = body["choices"][0]["message"]["tool_calls"]
        .as_array()
        .unwrap();
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0]["type"], "function");
    assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
    assert_eq!(tool_calls[0]["id"], "call_llmposter_1");

    let args: serde_json::Value =
        serde_json::from_str(tool_calls[0]["function"]["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(args["location"], "San Francisco");
    assert_eq!(args["unit"], "celsius");
}

#[tokio::test]
async fn should_stream_openai_tool_call_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "NYC"}),
                }])
                .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "weather in NYC"}],
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
    assert!(body.contains("data: "));
    assert!(body.contains("data: [DONE]"));
    assert!(body.contains("get_weather"));
    assert!(body.contains("tool_calls"));
}

#[tokio::test]
async fn should_simulate_latency_on_openai() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("delayed response")
                .with_failure(FailureConfig {
                    latency_ms: Some(200),
                    corrupt_body: None,
                    truncate_after_frames: None,
                    disconnect_after_ms: None,
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let start = std::time::Instant::now();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "delayed response");
    assert!(
        elapsed >= std::time::Duration::from_millis(180),
        "Expected at least 180ms delay, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn should_stream_openai_with_latency_between_chunks() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("Hello world test")
                .with_streaming(Some(50), Some(5)),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let start = std::time::Instant::now();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let elapsed = start.elapsed();

    assert!(body.contains("data: [DONE]"));
    // "Hello world test" is 16 chars, chunk_size 5 = 4 content chunks + 1 final = 5 chunks
    // 4 inter-chunk delays of 50ms = 200ms minimum
    assert!(
        elapsed >= std::time::Duration::from_millis(150),
        "Expected at least 150ms for streaming with latency, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn should_match_first_fixture_via_http_openai() {
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
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "first match");
}

#[tokio::test]
async fn should_not_match_anthropic_fixture_on_openai_endpoint() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("anthropic only")
                .for_provider(Provider::Anthropic),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_match_model_filter_via_http_openai() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_model("gpt-4")
                .respond_with_content("gpt-4 response"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Should match gpt-4
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "gpt-4 response");

    // Should NOT match gpt-3.5
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-3.5-turbo",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_use_custom_finish_reason_openai() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content: Some("truncated output".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: Some("length".to_string()),
            }),
            ..Fixture::new()
        })
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["finish_reason"], "length");
    assert_eq!(body["choices"][0]["message"]["content"], "truncated output");
}

// --- Coverage gap tests below ---

#[tokio::test]
async fn should_log_verbose_no_match_openai() {
    let server = ServerBuilder::new()
        .verbose(true)
        .fixture(
            Fixture::new()
                .match_user_message("specific only")
                .respond_with_content("specific"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "unmatched prompt"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_log_verbose_fixture_matched_openai() {
    let server = ServerBuilder::new()
        .verbose(true)
        .fixture(Fixture::new().respond_with_content("verbose match"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello verbose"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "verbose match");
}

#[tokio::test]
async fn should_return_500_error_fixture_openai() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().with_error(500, "Internal server error"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "trigger error"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 500);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["message"], "Internal server error");
}

#[tokio::test]
async fn should_return_503_error_fixture_openai() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().with_error(503, "Service unavailable"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "trigger"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 503);
}

#[tokio::test]
async fn should_truncate_openai_streaming_tool_call() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "Tokyo"}),
                }])
                .with_streaming(Some(0), Some(5))
                .with_failure(FailureConfig {
                    truncate_after_frames: Some(1),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "weather in Tokyo"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Stream truncated: should not have [DONE]
    assert!(!body.contains("[DONE]"));
}

#[tokio::test]
async fn should_stream_openai_tool_call_with_custom_finish_reason() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            match_rule: None,
            provider: None,
            response: Some(FixtureResponse {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    name: "search".to_string(),
                    arguments: serde_json::json!({"q": "test"}),
                }]),
                stop_reason: None,
                finish_reason: Some("custom_stop".to_string()),
            }),
            error: None,
            failure: None,
            streaming: Some(llmposter::fixture::StreamingConfig {
                latency: Some(0),
                chunk_size: Some(5),
            }),
        })
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "search something"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("custom_stop"));
    assert!(body.contains("[DONE]"));
}

#[tokio::test]
async fn should_simulate_latency_with_corrupt_body_openai() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("never seen")
                .with_failure(FailureConfig {
                    latency_ms: Some(100),
                    corrupt_body: Some(true),
                    truncate_after_frames: None,
                    disconnect_after_ms: None,
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let start = std::time::Instant::now();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "latency then corrupt"}]
        }))
        .send()
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "overloaded");
    assert!(
        elapsed >= std::time::Duration::from_millis(80),
        "Expected latency before corrupt body, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn should_use_stop_reason_as_finish_reason_alias_openai() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content: Some("aliased".to_string()),
                tool_calls: None,
                stop_reason: Some("length".to_string()),
                finish_reason: None,
            }),
            ..Fixture::new()
        })
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["finish_reason"], "length");
}

#[tokio::test]
async fn should_return_tool_call_with_custom_finish_reason_openai() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            match_rule: None,
            provider: None,
            response: Some(FixtureResponse {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    name: "calc".to_string(),
                    arguments: serde_json::json!({"expr": "1+1"}),
                }]),
                stop_reason: None,
                finish_reason: Some("stop".to_string()),
            }),
            error: None,
            failure: None,
            streaming: None,
        })
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "calculate"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
    assert!(body["choices"][0]["message"]["tool_calls"].is_array());
}

#[tokio::test]
async fn should_disconnect_openai_streaming_tool_call() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "Paris"}),
                }])
                .with_streaming(Some(0), Some(5))
                .with_failure(FailureConfig {
                    disconnect_after_ms: Some(0),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "weather in Paris"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Disconnect should prevent full stream
    assert!(!body.contains("[DONE]"));
}

#[tokio::test]
async fn should_disconnect_openai_streaming_text() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("A long response that should be disconnected")
                .with_streaming(Some(0), Some(5))
                .with_failure(FailureConfig {
                    disconnect_after_ms: Some(0),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
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
    assert!(!body.contains("[DONE]"));
}

#[tokio::test]
async fn should_apply_custom_stop_reason_to_non_streaming_tool_call_openai() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            match_rule: None,
            provider: None,
            response: Some(FixtureResponse {
                content: None,
                tool_calls: Some(vec![ToolCall {
                    name: "calculate".to_string(),
                    arguments: serde_json::json!({"expr": "1+1"}),
                }]),
                stop_reason: Some("custom_stop".to_string()),
                finish_reason: None,
            }),
            error: None,
            failure: None,
            streaming: None,
        })
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "compute"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["finish_reason"], "custom_stop");
}

#[tokio::test]
async fn should_disconnect_sse_stream_with_latency() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("A very long response that should be cut short by disconnect")
                .with_streaming(Some(30), Some(5))
                .with_failure(FailureConfig {
                    disconnect_after_ms: Some(200),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // With 30ms latency and 200ms disconnect, expect ~6 chunks before cutoff
    let data_lines: Vec<&str> = body
        .lines()
        .filter(|l| l.starts_with("data: ") && !l.contains("[DONE]"))
        .collect();
    // With 200ms budget and 30ms latency, expect ~6 chunks.
    // Lower bound: at least 1 frame should land before disconnect.
    assert!(
        !data_lines.is_empty(),
        "expected at least 1 chunk before disconnect"
    );
    // Upper bound: full content would produce ~12 chunks at chunk_size=5.
    assert!(
        data_lines.len() < 10,
        "expected truncated stream, got {} data lines",
        data_lines.len()
    );
}
