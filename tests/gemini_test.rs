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
        .await;

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
async fn should_extract_model_from_url_path() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_model("gemini-pro")
                .respond_with_content("matched model"),
        )
        .build()
        .await;

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
        .await;

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
        .await;

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
        .await;

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
        .await;

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
        .await;

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
        .await;

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
                    truncate_after_chunks: None,
                    disconnect_after_ms: None,
                }),
        )
        .build()
        .await;

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
        .await;

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
        .await;

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
        .await;

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
        .await;

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
        .await;

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
        .await;

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
        .await;

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
        .await;

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
        .await;

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
        .await;

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
                    truncate_after_chunks: Some(2),
                    ..FailureConfig::default()
                }),
        )
        .build()
        .await;

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
    // truncate_after_chunks=2 caps it at 2.
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
        .await;

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
