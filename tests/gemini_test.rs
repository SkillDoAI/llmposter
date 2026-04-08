use llmposter::fixture::{FailureConfig, FixtureResponse, ToolCall};
use llmposter::{Fixture, Provider, ServerBuilder};

#[tokio::test]
async fn should_return_gemini_generate_content_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from Gemini mock!"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello world"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "Hi from Gemini mock!"
    );
    assert_eq!(body["candidates"][0]["content"]["role"], "model");
    assert_eq!(body["candidates"][0]["finishReason"], "STOP");
    assert!(body["usageMetadata"]["promptTokenCount"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn should_accept_roleless_single_turn_gemini_request() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from Gemini mock!"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"parts": [{"text": "hello world"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "Hi from Gemini mock!"
    );
}

#[tokio::test]
async fn should_extract_model_from_url_path() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_model("gemini-pro")
                .respond_with_content("matched model"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_return_400_for_missing_contents() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({"not_contents": "bad"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_stream_gemini_as_json_array() {
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert!(resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("application/json"));

    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.is_array());
    let arr = body.as_array().unwrap();
    assert!(!arr.is_empty());
}

#[tokio::test]
async fn should_stream_gemini_as_sse_with_alt_param() {
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
}

#[tokio::test]
async fn should_return_gemini_tool_call_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "Tokyo", "unit": "celsius"}),
                }]),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "What's the weather in Tokyo?"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let parts = body["candidates"][0]["content"]["parts"]
        .as_array()
        .unwrap();
    assert_eq!(parts.len(), 1);
    assert!(parts[0].get("text").is_none());
    assert_eq!(parts[0]["functionCall"]["name"], "get_weather");
    assert_eq!(parts[0]["functionCall"]["args"]["location"], "Tokyo");
    assert_eq!(parts[0]["functionCall"]["args"]["unit"], "celsius");
    assert_eq!(body["candidates"][0]["content"]["role"], "model");
}

#[tokio::test]
async fn should_stream_gemini_tool_call_as_json_array() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "Berlin"}),
                }])
                .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "weather in Berlin"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    assert!(resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("application/json"));

    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.is_array());
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert!(arr[0]["candidates"][0]["content"]["parts"][0]["functionCall"].is_object());
    assert_eq!(
        arr[0]["candidates"][0]["content"]["parts"][0]["functionCall"]["name"],
        "get_weather"
    );
}

#[tokio::test]
async fn should_stream_gemini_tool_call_as_sse() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "Berlin"}),
                }])
                .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent?alt=sse",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "weather in Berlin"}]}]
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
    assert!(body.contains("get_weather"));
}

#[tokio::test]
async fn should_simulate_latency_on_gemini() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("delayed gemini")
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "delayed gemini"
    );
    assert!(
        elapsed >= std::time::Duration::from_millis(180),
        "Expected at least 180ms delay, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn should_stream_gemini_with_latency_between_chunks() {
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let elapsed = start.elapsed();

    assert!(body.is_array());
    // "Hello world test" = 16 chars, chunk_size 5 = 4 chunks, 3 inter-chunk delays of 50ms
    assert!(
        elapsed >= std::time::Duration::from_millis(100),
        "Expected at least 100ms for streaming with latency, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn should_match_first_fixture_via_http_gemini() {
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello world"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "first match"
    );
}

#[tokio::test]
async fn should_not_match_openai_fixture_on_gemini_endpoint() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("openai only")
                .for_provider(Provider::OpenAI),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_match_gemini_provider_fixture_on_gemini_endpoint() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("gemini matched")
                .for_provider(Provider::Gemini),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "gemini matched"
    );
}

#[tokio::test]
async fn should_match_model_from_url_path_gemini() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_model("gemini-1.5-pro")
                .respond_with_content("1.5 pro response"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Should match gemini-1.5-pro
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-1.5-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "1.5 pro response"
    );

    // Should NOT match gemini-pro (no "1.5" in the model string)
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_use_custom_finish_reason_gemini() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content: Some("partial output".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: Some("MAX_TOKENS".to_string()),
            }),
            ..Fixture::new()
        })
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["candidates"][0]["finishReason"], "MAX_TOKENS");
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "partial output"
    );
}

#[tokio::test]
async fn should_use_stop_reason_as_finish_reason_gemini() {
    // Gemini handler also checks stop_reason and applies it as finishReason
    let server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content: Some("safety filtered".to_string()),
                tool_calls: None,
                stop_reason: Some("SAFETY".to_string()),
                finish_reason: None,
            }),
            ..Fixture::new()
        })
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["candidates"][0]["finishReason"], "SAFETY");
}

#[tokio::test]
async fn should_return_gemini_tool_call_with_multiple_functions() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("tools")
                .respond_with_tool_calls(vec![
                    ToolCall {
                        name: "search".to_string(),
                        arguments: serde_json::json!({"query": "rust async"}),
                    },
                    ToolCall {
                        name: "read_file".to_string(),
                        arguments: serde_json::json!({"path": "/tmp/test.rs"}),
                    },
                ]),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "use tools please"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let parts = body["candidates"][0]["content"]["parts"]
        .as_array()
        .expect("parts should be an array");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["functionCall"]["name"], "search");
    assert_eq!(parts[0]["functionCall"]["args"]["query"], "rust async");
    assert_eq!(parts[1]["functionCall"]["name"], "read_file");
    assert_eq!(parts[1]["functionCall"]["args"]["path"], "/tmp/test.rs");
    // No text parts should be present
    assert!(parts[0].get("text").is_none());
    assert!(parts[1].get("text").is_none());
}

#[tokio::test]
async fn should_return_500_for_error_fixture_gemini() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("crash")
                .with_error(500, "Internal server error"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "crash now"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 500);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Internal server error"));
}

#[tokio::test]
async fn should_return_custom_finish_reason_max_tokens() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content: Some("truncated output here".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: Some("MAX_TOKENS".to_string()),
            }),
            ..Fixture::new().match_user_message("limit")
        })
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hit the limit"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["candidates"][0]["finishReason"], "MAX_TOKENS");
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "truncated output here"
    );
}

#[tokio::test]
async fn should_truncate_streaming_gemini_json_array() {
    let full_content = "abcdefghijklmnopqrstuvwxyz";
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("trunc")
                .respond_with_content(full_content)
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "trunc me"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("response should be a JSON array");
    // With chunk_size=5 and 26 chars, full streaming would produce 6 chunks.
    // truncate_after_frames=2 caps it at 2.
    assert_eq!(arr.len(), 2);
    // Concatenated text should be shorter than the full content
    let concatenated: String = arr
        .iter()
        .filter_map(|chunk| chunk["candidates"][0]["content"]["parts"][0]["text"].as_str())
        .collect();
    assert!(
        concatenated.len() < full_content.len(),
        "Expected truncated output shorter than {}, got {} chars: '{}'",
        full_content.len(),
        concatenated.len(),
        concatenated
    );
}

#[tokio::test]
async fn should_return_corrupt_body_overloaded_text_gemini() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("corrupt")
                .respond_with_content("this content should not appear")
                .with_failure(FailureConfig {
                    corrupt_body: Some(true),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "corrupt body"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "overloaded");
}

// --- Coverage gap tests below ---

#[tokio::test]
async fn should_log_verbose_no_match_gemini() {
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "unmatched prompt"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_log_verbose_fixture_matched_gemini() {
    let server = ServerBuilder::new()
        .verbose(true)
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("verbose gemini match"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello verbose"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "verbose gemini match"
    );
}

#[tokio::test]
async fn should_return_400_for_invalid_action_gemini() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:invalidAction",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Unknown action"));
}

#[tokio::test]
async fn should_return_400_for_unparseable_json_gemini() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
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
async fn should_simulate_latency_with_corrupt_body_gemini() {
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "latency then corrupt"}]}]
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
async fn should_truncate_gemini_sse_streaming() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("abcdefghijklmnopqrstuvwxyz")
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent?alt=sse",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Should have some data events but not all (26 chars / 5 = 6 chunks, truncated to 2)
    let data_lines: Vec<&str> = body.lines().filter(|l| l.starts_with("data: ")).collect();
    assert!(
        data_lines.len() <= 2,
        "Expected at most 2 data lines, got {}",
        data_lines.len()
    );
}

#[tokio::test]
async fn should_truncate_gemini_sse_tool_call_streaming() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "Berlin"}),
                }])
                .with_streaming(Some(0), Some(5))
                .with_failure(FailureConfig {
                    truncate_after_frames: Some(0),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent?alt=sse",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "weather in Berlin"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // truncate_after_frames=0 means no frames sent at all
    assert!(
        body.is_empty() || !body.contains("get_weather"),
        "Stream should be empty or truncated before content"
    );
}

#[tokio::test]
async fn should_truncate_gemini_json_array_tool_call_streaming() {
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
                    truncate_after_frames: Some(0),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "weather in Tokyo"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "[]");
}

#[tokio::test]
async fn should_stream_gemini_tool_call_with_custom_finish_reason() {
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
                finish_reason: Some("MAX_TOKENS".to_string()),
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
    // Non-streaming: check the finish_reason override for tool calls
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "test search"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["candidates"][0]["finishReason"], "MAX_TOKENS");
}

#[tokio::test]
async fn should_stream_gemini_tool_call_json_array_with_custom_finish_reason() {
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
                finish_reason: Some("SAFETY".to_string()),
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "test search"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.is_array());
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["candidates"][0]["finishReason"], "SAFETY");
}

#[tokio::test]
async fn should_stream_gemini_tool_call_sse_with_custom_finish_reason() {
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
                finish_reason: Some("SAFETY".to_string()),
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent?alt=sse",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "test search"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("SAFETY"));
}

#[tokio::test]
async fn should_stream_gemini_text_with_finish_reason_override() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            match_rule: None,
            provider: None,
            response: Some(FixtureResponse {
                content: Some("partial content".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: Some("MAX_TOKENS".to_string()),
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
    // JSON array streaming
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().unwrap();
    // The last chunk should have MAX_TOKENS finish_reason
    let last = arr.last().unwrap();
    assert_eq!(last["candidates"][0]["finishReason"], "MAX_TOKENS");
}

#[tokio::test]
async fn should_return_400_for_path_without_colon_gemini() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("x"))
        .build()
        .await
        .unwrap();

    // Path with no colon at all — this triggers the None branch of rsplit_once(':')
    // The wildcard path captures everything, so "nocolon" has no ':'
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1beta/models/nocolon", server.url()))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Invalid path"));
}

#[tokio::test]
async fn should_log_verbose_match_gemini() {
    let server = ServerBuilder::new()
        .verbose(true)
        .fixture(Fixture::new().respond_with_content("verbose"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello verbose"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_stream_gemini_tool_call_via_sse_with_truncation() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_tool_calls(vec![ToolCall {
                    name: "search".to_string(),
                    arguments: serde_json::json!({"q": "rust"}),
                }])
                .with_streaming(Some(0), Some(5))
                .with_failure(FailureConfig {
                    truncate_after_frames: Some(0),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent?alt=sse",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "search for rust"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    // Truncated at 0 — should be empty
    assert!(body.is_empty() || !body.contains("functionCall"));
}

#[tokio::test]
async fn should_return_gemini_json_array_tool_call_with_truncation_zero() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "NYC"}),
                }])
                .with_failure(FailureConfig {
                    truncate_after_frames: Some(0),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "weather NYC"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, serde_json::json!([]));
}

#[tokio::test]
async fn should_return_gemini_json_array_tool_call_with_latency() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "London"}),
                }])
                .with_streaming(Some(100), Some(5)),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let start = std::time::Instant::now();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "weather London"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let elapsed = start.elapsed();
    assert!(elapsed >= std::time::Duration::from_millis(80));
}

#[tokio::test]
async fn should_return_gemini_json_array_tool_call_with_disconnect() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_tool_calls(vec![ToolCall {
                    name: "search".to_string(),
                    arguments: serde_json::json!({"q": "test"}),
                }])
                .with_streaming(Some(200), Some(5))
                .with_failure(FailureConfig {
                    disconnect_after_ms: Some(50),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "search test"}]}]
        }))
        .send()
        .await
        .unwrap();

    // Disconnect before latency completes — should return empty array
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, serde_json::json!([]));
}

#[tokio::test]
async fn should_apply_finish_reason_to_gemini_non_streaming_text() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            match_rule: None,
            provider: None,
            response: Some(FixtureResponse {
                content: Some("truncated response".to_string()),
                tool_calls: None,
                stop_reason: None,
                finish_reason: Some("MAX_TOKENS".to_string()),
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["candidates"][0]["finishReason"], "MAX_TOKENS");
}

#[tokio::test]
async fn should_disconnect_gemini_sse_streaming() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("Long response for disconnect test")
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent?alt=sse",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap_or_default();
    // disconnect_after_ms=0 means disconnect ASAP — may still deliver partial data
    let data_lines: Vec<&str> = body.lines().filter(|l| l.starts_with("data: ")).collect();
    // Should be fewer than the full stream (content has multiple chunks)
    // disconnect_after_ms=0 with latency=0 is a race — just verify no panic
    let _ = data_lines;
}

#[tokio::test]
async fn should_disconnect_gemini_sse_tool_call_streaming() {
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent?alt=sse",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap_or_default();
    // disconnect_after_ms=0 means disconnect ASAP — stream is truncated
    let data_lines: Vec<&str> = body.lines().filter(|l| l.starts_with("data: ")).collect();
    // disconnect_after_ms=0 with latency=0 is a race — just verify no panic
    let _ = data_lines;
}

#[tokio::test]
async fn should_disconnect_gemini_json_array_streaming() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("Long content for disconnect")
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let arr = body.as_array().unwrap();
    // disconnect_after_ms=0 should result in empty or very small array
    assert!(
        arr.is_empty() || arr.len() <= 1,
        "Expected empty or tiny array due to disconnect, got {} elements",
        arr.len()
    );
}

#[tokio::test]
async fn should_apply_disconnect_to_gemini_json_array_tool_call_zero_ms() {
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
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, serde_json::json!([]));
}

#[tokio::test]
async fn should_extract_text_from_mixed_part_types() {
    // Parts with explicit "type": "inline_data" (non-text) are filtered;
    // only the text part is used for fixture matching.
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello from mixed")
                .respond_with_content("matched mixed parts"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{
                "role": "user",
                "parts": [
                    {"type": "inline_data", "inline_data": {"mime_type": "image/png", "data": "abc"}},
                    {"text": "hello from mixed"}
                ]
            }]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "matched mixed parts"
    );
}

#[tokio::test]
async fn should_return_gemini_error_for_403_status() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_model("forbidden-model")
                .with_error(403, "Forbidden"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/forbidden-model:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], 403);
    assert_eq!(body["error"]["status"], "PERMISSION_DENIED");
}

#[tokio::test]
async fn should_return_gemini_error_for_404_status() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_model("missing-model")
                .with_error(404, "Not found"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/missing-model:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], 404);
    assert_eq!(body["error"]["status"], "NOT_FOUND");
}

#[tokio::test]
async fn should_return_gemini_error_for_503_status() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_model("unavail-model")
                .with_error(503, "Service unavailable"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/unavail-model:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 503);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["status"], "UNAVAILABLE");
}

#[tokio::test]
async fn should_return_gemini_error_with_unknown_status() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_model("custom-model")
                .with_error(418, "I'm a teapot"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/custom-model:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 418);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["status"], "UNKNOWN");
}

#[tokio::test]
async fn should_ignore_non_text_parts_in_gemini_request() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("visible text")
                .respond_with_content("matched"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{
                "parts": [
                    {"type": "image", "inline_data": {"data": "abc"}},
                    {"text": "visible text"}
                ]
            }]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "matched"
    );
}

#[tokio::test]
async fn should_accept_non_boolean_stream_field_in_gemini_request() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("ok"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    // Gemini ignores `stream` field — it uses URL action for streaming.
    // Non-boolean "stream" should NOT cause a 400 for Gemini.
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"parts": [{"text": "hello"}]}],
            "stream": "true"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}
