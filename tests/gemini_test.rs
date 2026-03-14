use llmposter::{Fixture, ServerBuilder};

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
