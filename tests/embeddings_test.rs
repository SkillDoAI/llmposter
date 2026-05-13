use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn should_return_embedding_with_explicit_vector() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("test")
                .respond_with_embedding(vec![0.1, 0.2, 0.3, 0.4]),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/embeddings", server.url()))
        .json(&serde_json::json!({
            "model": "text-embedding-ada-002",
            "input": "test"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "list");
    assert_eq!(body["model"], "text-embedding-ada-002");
    let emb = body["data"][0]["embedding"].as_array().unwrap();
    assert_eq!(emb.len(), 4);
    assert!(body["usage"]["prompt_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn should_accept_array_input_for_embeddings() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("catch-all"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/embeddings", server.url()))
        .json(&serde_json::json!({
            "model": "text-embedding-ada-002",
            "input": ["first text", "second text"]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let emb = body["data"][0]["embedding"].as_array().unwrap();
    assert_eq!(emb.len(), 1536);
}

#[tokio::test]
async fn should_reject_invalid_json_on_embeddings() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/embeddings", server.url()))
        .body("not valid json")
        .header("content-type", "application/json")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_reject_missing_model_on_embeddings() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/embeddings", server.url()))
        .json(&serde_json::json!({"input": "hello"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_reject_missing_input_on_embeddings() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/embeddings", server.url()))
        .json(&serde_json::json!({"model": "text-embedding-ada-002"}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_reject_non_string_non_array_input_on_embeddings() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/embeddings", server.url()))
        .json(&serde_json::json!({
            "model": "text-embedding-ada-002",
            "input": 42
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_return_error_fixture_via_embeddings() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("err")
                .with_error(500, "Internal error"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/embeddings", server.url()))
        .json(&serde_json::json!({
            "model": "text-embedding-ada-002",
            "input": "err"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 500);
}

#[tokio::test]
async fn should_reject_token_id_array_input_on_embeddings() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/embeddings", server.url()))
        .json(&serde_json::json!({
            "model": "text-embedding-ada-002",
            "input": [1, 2, 3]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn should_honor_dimensions_field_on_fake_embedding() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/embeddings", server.url()))
        .json(&serde_json::json!({
            "model": "text-embedding-3-small",
            "input": "hi",
            "dimensions": 512
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["data"][0]["embedding"].as_array().unwrap().len(), 512);
}

#[tokio::test]
async fn should_clamp_invalid_dimensions_to_default() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();
    // 0 is out of valid range — falls back to 1536.
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/embeddings", server.url()))
        .json(&serde_json::json!({
            "model": "text-embedding-ada-002",
            "input": "hi",
            "dimensions": 0
        }))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["data"][0]["embedding"].as_array().unwrap().len(), 1536);
}

#[tokio::test]
async fn should_apply_custom_error_headers_on_embeddings() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("limited")
                .with_error_headers(429, "Rate limited", [("retry-after", "30")])
                .unwrap(),
        )
        .build()
        .await
        .unwrap();
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/embeddings", server.url()))
        .json(&serde_json::json!({
            "model": "text-embedding-ada-002",
            "input": "limited"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 429);
    assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
}

#[tokio::test]
async fn should_advance_scenario_state_on_embeddings_match() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("first")
                .with_scenario("flow", None, Some("after_first"))
                .respond_with_embedding(vec![0.1]),
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/embeddings", server.url()))
        .json(&serde_json::json!({
            "model": "text-embedding-ada-002",
            "input": "first"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        server.scenario_state("flow").as_deref(),
        Some("after_first")
    );
}

#[tokio::test]
async fn should_include_nearest_match_in_embeddings_404_when_diagnostics_on() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_embedding(vec![0.1]),
        )
        .diagnostics(true)
        .build()
        .await
        .unwrap();
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/embeddings", server.url()))
        .json(&serde_json::json!({
            "model": "text-embedding-ada-002",
            "input": "something else"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.unwrap();
    let nm = &body["error"]["nearest_match"];
    assert!(!nm.is_null());
    assert_eq!(nm["total_fields"], 1);
    let fields = nm["fields"].as_array().unwrap();
    let um = fields
        .iter()
        .find(|f| f["field"] == "user_message")
        .unwrap();
    assert_eq!(um["passed"], false);
}

#[tokio::test]
async fn should_return_404_when_no_embedding_fixture_matches() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("specific")
                .respond_with_embedding(vec![0.1]),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/embeddings", server.url()))
        .json(&serde_json::json!({
            "model": "text-embedding-ada-002",
            "input": "no match"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}
