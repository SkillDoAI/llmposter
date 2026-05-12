use llmposter::fixture::{FailureConfig, ToolCall};
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn should_return_text_completion() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("write a story")
                .respond_with_content("Once upon a time..."),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/completions", server.url()))
        .json(&serde_json::json!({
            "model": "davinci",
            "prompt": "write a story"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "text_completion");
    assert_eq!(body["choices"][0]["text"], "Once upon a time...");
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
    assert_eq!(body["choices"][0]["index"], 0);
    assert!(body["id"].as_str().unwrap().starts_with("cmpl-llmposter-"));
    assert!(body["usage"]["prompt_tokens"].as_u64().unwrap() > 0);
    assert!(body["usage"]["completion_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn should_stream_text_completion() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("stream me")
                .respond_with_content("hello world"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/completions", server.url()))
        .json(&serde_json::json!({
            "model": "davinci",
            "prompt": "stream me",
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("data: "));
    assert!(body.contains("[DONE]"));
    assert!(body.contains("\"text_completion\""));
}

#[tokio::test]
async fn should_return_404_when_no_completion_fixture_matches() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("specific")
                .respond_with_content("ok"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/completions", server.url()))
        .json(&serde_json::json!({
            "model": "davinci",
            "prompt": "no match"
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
async fn should_reject_completion_missing_prompt() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/completions", server.url()))
        .json(&serde_json::json!({"model": "davinci"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_reject_completion_missing_model() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/completions", server.url()))
        .json(&serde_json::json!({"prompt": "hello"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_return_error_fixture_via_completions() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("rate-limit-me")
                .with_error(429, "Rate limited"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/completions", server.url()))
        .json(&serde_json::json!({
            "model": "davinci",
            "prompt": "rate-limit-me"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 429);
}

#[tokio::test]
async fn should_honor_custom_finish_reason_in_completion() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("truncated text")
                .with_finish_reason("length"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/completions", server.url()))
        .json(&serde_json::json!({
            "model": "davinci",
            "prompt": "anything"
        }))
        .send()
        .await
        .unwrap();

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["finish_reason"], "length");
}

#[tokio::test]
async fn should_honor_custom_finish_reason_in_streaming_completion() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("truncated")
                .with_finish_reason("length"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/completions", server.url()))
        .json(&serde_json::json!({
            "model": "davinci",
            "prompt": "anything",
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();
    assert!(body.contains("\"length\""));
}

#[tokio::test]
async fn should_render_tool_call_fixture_as_empty_text_completion() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("tools")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".into(),
                    arguments: serde_json::json!({"location": "Paris"}),
                }]),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/completions", server.url()))
        .json(&serde_json::json!({"model": "davinci", "prompt": "tools"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    // Legacy completions has no tool calls — fixture rendered as empty text.
    assert_eq!(body["choices"][0]["text"], "");
}

#[tokio::test]
async fn should_stream_tool_call_fixture_as_empty_text_completion() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("tools")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".into(),
                    arguments: serde_json::json!({"location": "Paris"}),
                }]),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/completions", server.url()))
        .json(&serde_json::json!({
            "model": "davinci",
            "prompt": "tools",
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(body.contains("[DONE]"));
}

#[tokio::test]
async fn should_render_refusal_fixture_as_text_completion() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hack")
                .respond_with_refusal("I cannot help with that"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/completions", server.url()))
        .json(&serde_json::json!({"model": "davinci", "prompt": "hack"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["text"], "I cannot help with that");
}

#[tokio::test]
async fn should_apply_failure_latency_to_completion() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("slow")
                .respond_with_content("eventually")
                .with_failure(FailureConfig {
                    latency_ms: Some(100),
                    ..Default::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let start = std::time::Instant::now();
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/completions", server.url()))
        .json(&serde_json::json!({
            "model": "davinci",
            "prompt": "slow"
        }))
        .send()
        .await
        .unwrap();
    let elapsed = start.elapsed().as_millis();

    assert_eq!(resp.status(), 200);
    assert!(elapsed >= 90, "expected >=90ms, got {}ms", elapsed);
}
