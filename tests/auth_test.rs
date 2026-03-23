use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn should_pass_without_auth_enabled() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("hello"))
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
}

#[tokio::test]
async fn should_reject_missing_token_when_auth_enabled() {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token("valid-token")
        .fixture(Fixture::new().respond_with_content("hello"))
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

    assert_eq!(resp.status(), 401);
    // 401 responses must still carry x-request-id
    assert!(resp.headers().get("x-request-id").is_some());
}

#[tokio::test]
async fn should_accept_valid_token() {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token("valid-token")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("Authorization", "Bearer valid-token")
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_reject_wrong_token() {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token("valid-token")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("Authorization", "Bearer wrong-token")
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn should_expire_token_after_n_uses() {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token_uses("short-lived", 2)
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    for i in 0..2 {
        let resp = client
            .post(format!("{}/v1/chat/completions", server.url()))
            .header("Authorization", "Bearer short-lived")
            .json(&serde_json::json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "use {} should succeed", i + 1);
    }

    // Third use should be rejected — token expired
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("Authorization", "Bearer short-lived")
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn should_return_anthropic_401_for_messages_endpoint() {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token("valid")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .header("Authorization", "Bearer wrong")
        .json(&serde_json::json!({
            "model": "claude",
            "max_tokens": 100,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "authentication_error");
}

#[tokio::test]
async fn should_return_gemini_401_for_generate_content() {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token("valid")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .header("Authorization", "Bearer wrong")
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], 401);
    assert_eq!(body["error"]["status"], "UNAUTHENTICATED");
}
