use llmposter::fixture::{FailureConfig, ToolCall};
use llmposter::{Fixture, Provider, ServerBuilder};

#[tokio::test]
async fn should_return_responses_api_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from Responses mock!"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "response");
    assert_eq!(body["output"][0]["type"], "message");
    assert_eq!(body["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(
        body["output"][0]["content"][0]["text"],
        "Hi from Responses mock!"
    );
    assert!(body["id"].as_str().unwrap().starts_with("resp-llmposter-"));
}

#[tokio::test]
async fn should_handle_string_input() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("string prompt")
                .respond_with_content("got string"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": "string prompt"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_return_400_for_missing_input() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({"model": "gpt-4"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_stream_responses_api() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hello world")
                .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
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
    assert_eq!(
        resp.headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap(),
        "text/event-stream"
    );

    let body = resp.text().await.unwrap();
    assert!(body.contains("event: response.created"));
    assert!(body.contains("event: response.completed"));
}

#[tokio::test]
async fn should_return_error_status_for_503_fixture() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("overload")
                .with_error(503, "Service temporarily unavailable"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "overload me"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 503);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "service_unavailable");
    assert_eq!(body["error"]["message"], "Service temporarily unavailable");
}

#[tokio::test]
async fn should_include_output_index_and_content_index_in_stream_events() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("idx")
                .respond_with_content("check indices")
                .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "idx test"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();

    // Parse each SSE data line and check envelope fields on relevant events
    for line in body.lines() {
        if !line.starts_with("data: ") {
            continue;
        }
        let json_str = &line["data: ".len()..];
        let val: serde_json::Value = serde_json::from_str(json_str).unwrap();

        let event_type = val["type"].as_str().unwrap_or("");
        match event_type {
            "response.output_item.added"
            | "response.content_part.added"
            | "response.output_text.delta"
            | "response.output_text.done"
            | "response.output_item.done" => {
                assert!(
                    val.get("output_index").is_some(),
                    "missing output_index in {}",
                    event_type
                );
            }
            _ => {}
        }

        match event_type {
            "response.content_part.added"
            | "response.output_text.delta"
            | "response.output_text.done" => {
                assert!(
                    val.get("content_index").is_some(),
                    "missing content_index in {}",
                    event_type
                );
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn should_return_corrupt_body_with_overloaded_text() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("corrupt")
                .respond_with_content("ignored content")
                .with_failure(FailureConfig {
                    corrupt_body: Some(true),
                    latency_ms: None,
                    truncate_after_frames: None,
                    disconnect_after_ms: None,
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "corrupt request"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "overloaded");
}

#[tokio::test]
async fn should_return_tool_call_response_non_streaming() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "London"}),
                }]),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "what is the weather in London?"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "response");
    assert_eq!(body["status"], "completed");

    let output = &body["output"];
    assert!(!output.as_array().unwrap().is_empty());
    // Responses API function_call items are flat objects
    assert_eq!(output[0]["type"], "function_call");
    assert_eq!(output[0]["name"], "get_weather");
    assert!(output[0]["id"].as_str().is_some());
    assert!(output[0]["call_id"].as_str().is_some());
    assert!(output[0]["arguments"].as_str().unwrap().contains("London"));
}

#[tokio::test]
async fn should_match_first_fixture_via_http() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("greet")
                .respond_with_content("first fixture wins"),
        )
        .fixture(
            Fixture::new()
                .match_user_message("greet")
                .respond_with_content("second fixture loses"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "greet me"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["output"][0]["content"][0]["text"],
        "first fixture wins"
    );
}

#[tokio::test]
async fn should_stream_responses_tool_call_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "Denver"}),
                }])
                .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "weather in Denver"}],
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
    assert!(body.contains("event: response.created"));
    assert!(body.contains("event: response.completed"));
    assert!(body.contains("function_call"));
    assert!(body.contains("get_weather"));
}

#[tokio::test]
async fn should_simulate_latency_on_responses() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("delayed responses")
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
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["output"][0]["content"][0]["text"], "delayed responses");
    assert!(
        elapsed >= std::time::Duration::from_millis(180),
        "Expected at least 180ms delay, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn should_stream_responses_with_latency_between_chunks() {
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
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let elapsed = start.elapsed();

    assert!(body.contains("event: response.completed"));
    // "Hello world test" = 16 chars, chunk_size 5 = multiple events with 50ms delays
    assert!(
        elapsed >= std::time::Duration::from_millis(150),
        "Expected at least 150ms for streaming with latency, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn should_not_match_anthropic_fixture_on_responses_endpoint() {
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
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_match_responses_provider_fixture_on_responses_endpoint() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("responses matched")
                .for_provider(Provider::Responses),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["output"][0]["content"][0]["text"], "responses matched");
}

#[tokio::test]
async fn should_match_model_filter_via_http_responses() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_model("gpt-4o")
                .respond_with_content("gpt-4o response"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Should match gpt-4o
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4o",
            "input": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["output"][0]["content"][0]["text"], "gpt-4o response");

    // Should NOT match gpt-3.5
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-3.5-turbo",
            "input": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

// --- Coverage gap tests below ---

#[tokio::test]
async fn should_log_verbose_no_match_responses() {
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
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "unmatched prompt"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_log_verbose_fixture_matched_responses() {
    let server = ServerBuilder::new()
        .verbose(true)
        .fixture(Fixture::new().respond_with_content("verbose match"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hello verbose"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["output"][0]["content"][0]["text"], "verbose match");
}

#[tokio::test]
async fn should_return_400_for_unparseable_json_responses() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .header("content-type", "application/json")
        .body("not json at all")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Invalid JSON"));
}

#[tokio::test]
async fn should_return_500_error_fixture_responses() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().with_error(500, "Internal server error"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "trigger error"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 500);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["message"], "Internal server error");
}

#[tokio::test]
async fn should_return_429_error_fixture_responses() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().with_error(429, "Rate limit exceeded"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "trigger rate limit"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 429);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["message"], "Rate limit exceeded");
}

#[tokio::test]
async fn should_simulate_latency_with_corrupt_body_responses() {
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
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "latency then corrupt"}]
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
async fn should_truncate_responses_streaming_text() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content(
                    "This is a very long response that should be truncated before completion",
                )
                .with_streaming(Some(0), Some(5))
                .with_failure(FailureConfig {
                    truncate_after_frames: Some(2),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Truncated: should not have response.completed
    assert!(!body.contains("response.completed"));
}

#[tokio::test]
async fn should_truncate_responses_streaming_tool_call() {
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
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "weather in Tokyo"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Should be truncated - not have all events
    assert!(!body.contains("response.done"));
}

#[tokio::test]
async fn should_return_404_for_no_match_responses() {
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
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "unmatched"}]
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
async fn should_disconnect_responses_streaming_tool_call() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_tool_calls(vec![ToolCall {
                    name: "search".to_string(),
                    arguments: serde_json::json!({"q": "test"}),
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
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(!body.contains("response.done"));
}

#[tokio::test]
async fn should_disconnect_responses_streaming_text() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("A long response for disconnection testing")
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
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(!body.contains("response.completed"));
}

#[tokio::test]
async fn should_map_stop_reason_to_incomplete_status_non_streaming() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("cut")
                .respond_with_content("truncated")
                .with_stop_reason("max_tokens"),
        )
        .build()
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "cut short"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "incomplete");
}

#[tokio::test]
async fn should_map_stop_reason_to_incomplete_status_streaming() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("cut")
                .respond_with_content("truncated")
                .with_stop_reason("max_tokens"),
        )
        .build()
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "cut short"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Parse SSE frames and check the response.completed event has status "incomplete"
    let found = body
        .lines()
        .filter(|l| l.starts_with("data: "))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l["data: ".len()..]).ok())
        .any(|v| {
            v.get("type").and_then(|t| t.as_str()) == Some("response.completed")
                && v.get("response")
                    .and_then(|r| r.get("status"))
                    .and_then(|s| s.as_str())
                    == Some("incomplete")
        });
    assert!(found, "response.completed must have status=incomplete");
}

#[tokio::test]
async fn should_map_stop_reason_to_incomplete_tool_call_non_streaming() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("call")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "fn1".to_string(),
                    arguments: serde_json::json!({"a": 1}),
                }])
                .with_stop_reason("max_tokens"),
        )
        .build()
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "call me"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "incomplete");
}

#[tokio::test]
async fn should_map_stop_reason_to_incomplete_tool_call_streaming() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("call")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "fn1".to_string(),
                    arguments: serde_json::json!({"a": 1}),
                }])
                .with_stop_reason("max_tokens"),
        )
        .build()
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "call me"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let found = body
        .lines()
        .filter(|l| l.starts_with("data: "))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l["data: ".len()..]).ok())
        .any(|v| {
            v.get("type").and_then(|t| t.as_str()) == Some("response.completed")
                && v.get("response")
                    .and_then(|r| r.get("status"))
                    .and_then(|s| s.as_str())
                    == Some("incomplete")
        });
    assert!(found, "response.completed must have status=incomplete");
}

#[tokio::test]
async fn should_emit_incomplete_details_when_status_is_incomplete() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("cut")
                .respond_with_content("truncated")
                .with_stop_reason("max_tokens"),
        )
        .build()
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "cut short"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "incomplete");
    assert_eq!(
        body["incomplete_details"]["reason"], "max_tokens",
        "incomplete_details must carry the stop reason"
    );
}

// Verbose mode tests removed — already covered by
// should_log_verbose_no_match_responses (line 552) and
// should_log_verbose_fixture_matched_responses (line 579).
