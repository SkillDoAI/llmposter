//! Tests for `--diagnostics` mode — exercises every match field path in
//! `evaluate_fixture_fields` so the 404 nearest-match output covers each.

use llmposter::{Fixture, Provider, ServerBuilder};

/// Helper: send a request and return the parsed 404 body.
async fn send_and_get_404(server_url: &str, body: serde_json::Value) -> serde_json::Value {
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server_url))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    resp.json().await.unwrap()
}

#[tokio::test]
async fn should_include_user_message_pass_and_model_fail() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .match_model("gpt-4")
                .respond_with_content("sunny"),
        )
        .diagnostics(true)
        .build()
        .await
        .unwrap();

    let body = send_and_get_404(
        &server.url(),
        serde_json::json!({"model": "claude", "messages": [{"role": "user", "content": "weather"}]}),
    )
    .await;
    let fields = body["error"]["nearest_match"]["fields"].as_array().unwrap();
    let um = fields
        .iter()
        .find(|f| f["field"] == "user_message")
        .unwrap();
    assert_eq!(um["passed"], true);
    let m = fields.iter().find(|f| f["field"] == "model").unwrap();
    assert_eq!(m["passed"], false);
}

#[tokio::test]
async fn should_evaluate_header_match_in_diagnostics() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .match_header("x-tenant", "acme")
                .respond_with_content("ok"),
        )
        .diagnostics(true)
        .build()
        .await
        .unwrap();

    let body = send_and_get_404(
        &server.url(),
        serde_json::json!({"model": "m", "messages": [{"role": "user", "content": "hello"}]}),
    )
    .await;
    let fields = body["error"]["nearest_match"]["fields"].as_array().unwrap();
    let h = fields.iter().find(|f| f["field"] == "headers").unwrap();
    assert_eq!(h["passed"], false);
}

#[tokio::test]
async fn should_evaluate_system_prompt_match_in_diagnostics() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hi")
                .match_system_prompt("pirate")
                .respond_with_content("arrr"),
        )
        .diagnostics(true)
        .build()
        .await
        .unwrap();

    let body = send_and_get_404(
        &server.url(),
        serde_json::json!({
            "model": "m",
            "messages": [
                {"role": "system", "content": "you are helpful"},
                {"role": "user", "content": "hi"}
            ]
        }),
    )
    .await;
    let fields = body["error"]["nearest_match"]["fields"].as_array().unwrap();
    let sp = fields
        .iter()
        .find(|f| f["field"] == "system_prompt")
        .unwrap();
    assert_eq!(sp["passed"], false);
}

#[tokio::test]
async fn should_evaluate_temperature_exact_in_diagnostics() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hi")
                .match_temperature(0.7)
                .respond_with_content("ok"),
        )
        .diagnostics(true)
        .build()
        .await
        .unwrap();

    let body = send_and_get_404(
        &server.url(),
        serde_json::json!({
            "model": "m",
            "temperature": 0.5,
            "messages": [{"role": "user", "content": "hi"}]
        }),
    )
    .await;
    let fields = body["error"]["nearest_match"]["fields"].as_array().unwrap();
    let t = fields.iter().find(|f| f["field"] == "temperature").unwrap();
    assert_eq!(t["passed"], false);
}

#[tokio::test]
async fn should_evaluate_metadata_in_diagnostics() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hi")
                .match_metadata("tier", "gold")
                .respond_with_content("ok"),
        )
        .diagnostics(true)
        .build()
        .await
        .unwrap();

    // Request has no metadata at all — metadata match should fail.
    let body = send_and_get_404(
        &server.url(),
        serde_json::json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    let fields = body["error"]["nearest_match"]["fields"].as_array().unwrap();
    let md = fields.iter().find(|f| f["field"] == "metadata").unwrap();
    assert_eq!(md["passed"], false);
}

#[tokio::test]
async fn should_evaluate_tool_schema_in_diagnostics() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hi")
                .match_tool_schema("get_weather")
                .respond_with_content("ok"),
        )
        .diagnostics(true)
        .build()
        .await
        .unwrap();

    let body = send_and_get_404(
        &server.url(),
        serde_json::json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    let fields = body["error"]["nearest_match"]["fields"].as_array().unwrap();
    let ts = fields.iter().find(|f| f["field"] == "tool_schema").unwrap();
    assert_eq!(ts["passed"], false);
}

#[tokio::test]
async fn should_evaluate_provider_in_diagnostics() {
    let mut anthropic_fixture = Fixture::new()
        .match_user_message("hi")
        .respond_with_content("ok");
    anthropic_fixture.provider = Some(Provider::Anthropic);

    let server = ServerBuilder::new()
        .fixture(anthropic_fixture)
        .diagnostics(true)
        .build()
        .await
        .unwrap();

    // Hit the openai endpoint — provider field should fail.
    let body = send_and_get_404(
        &server.url(),
        serde_json::json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    let fields = body["error"]["nearest_match"]["fields"].as_array().unwrap();
    let p = fields.iter().find(|f| f["field"] == "provider").unwrap();
    assert_eq!(p["passed"], false);
}

#[tokio::test]
async fn should_evaluate_scenario_required_state_in_diagnostics() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hi")
                .with_scenario("flow", Some("after_first"), None)
                .respond_with_content("ok"),
        )
        .diagnostics(true)
        .build()
        .await
        .unwrap();

    // Scenario state is empty by default — required_state check fails.
    let body = send_and_get_404(
        &server.url(),
        serde_json::json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]}),
    )
    .await;
    let fields = body["error"]["nearest_match"]["fields"].as_array().unwrap();
    let s = fields
        .iter()
        .find(|f| f["field"] == "scenario.required_state")
        .unwrap();
    assert_eq!(s["passed"], false);
}

#[tokio::test]
async fn should_evaluate_combined_user_message_and_model_diagnostic() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .match_model("gpt-4")
                .respond_with_content("sunny"),
        )
        .diagnostics(true)
        .build()
        .await
        .unwrap();

    let body = send_and_get_404(
        &server.url(),
        serde_json::json!({"model": "gpt-4", "messages": [{"role": "user", "content": "world"}]}),
    )
    .await;
    let nm = &body["error"]["nearest_match"];
    assert_eq!(nm["pass_count"], 1);
    assert_eq!(nm["total_fields"], 2);
    let summary = nm["summary"].as_str().unwrap();
    assert!(summary.contains("user_message"));
    assert!(summary.contains("model"));
}
