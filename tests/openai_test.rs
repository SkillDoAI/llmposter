use llmposter::fixture::FailureConfig;
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn should_return_openai_chat_completion() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("Hi from mock!"),
        )
        .build()
        .await;

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
        .await;

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
        .await;

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
        .await;

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
        .await;

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
        .await;

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
        .await;
    let server2 = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("two"))
        .build()
        .await;

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
                    truncate_after_chunks: None,
                    disconnect_after_ms: None,
                }),
        )
        .build()
        .await;

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
                    truncate_after_chunks: Some(2),
                    disconnect_after_ms: None,
                }),
        )
        .build()
        .await;

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
