use llmposter::fixture::ToolCall;
use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn should_advance_scenario_state_on_match() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("start")
                .respond_with_content("step 1")
                .with_scenario("flow", None, Some("started")),
        )
        .fixture(
            Fixture::new()
                .match_user_message("continue")
                .respond_with_content("step 2")
                .with_scenario("flow", Some("started"), Some("completed")),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Before any requests, scenario has no state
    assert_eq!(server.scenario_state("flow"), None);

    // First request: "start" matches, sets state to "started"
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "start"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "step 1");
    assert_eq!(server.scenario_state("flow"), Some("started".to_string()));

    // Second request: "continue" matches because state is "started"
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "continue"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "step 2");
    assert_eq!(server.scenario_state("flow"), Some("completed".to_string()));
}

#[tokio::test]
async fn should_not_match_fixture_when_scenario_state_wrong() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("only after setup")
                .with_scenario("gate", Some("ready"), None),
        )
        .fixture(Fixture::new().respond_with_content("catch-all"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // "hello" won't match the scenario fixture (state is not "ready"), falls to catch-all
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "catch-all");
}

#[tokio::test]
async fn should_simulate_tool_call_loop_with_scenarios() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_tool_calls(vec![ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({"location": "Paris"}),
                }])
                .with_scenario("tool-loop", Some(""), Some("tool_called")),
        )
        .fixture(
            Fixture::new()
                .match_user_message("weather")
                .respond_with_content("It's 22°C and sunny in Paris")
                .with_scenario("tool-loop", Some("tool_called"), Some("done")),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Step 1: Ask about weather → tool call
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "weather in Paris"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["choices"][0]["message"]["tool_calls"].is_array());
    assert_eq!(
        server.scenario_state("tool-loop"),
        Some("tool_called".to_string())
    );

    // Step 2: Send tool result → text response
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "user", "content": "weather in Paris"},
                {"role": "assistant", "tool_calls": [{"id": "1", "function": {"name": "get_weather"}}]},
                {"role": "tool", "content": "tool_result: 22°C sunny"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "It's 22°C and sunny in Paris"
    );
    assert_eq!(server.scenario_state("tool-loop"), Some("done".to_string()));
}

#[tokio::test]
async fn should_capture_requests() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();

    assert_eq!(server.request_count(), 0);

    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    client
        .post(format!("{}/v1/messages", server.url()))
        .header("x-api-key", "test")
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "world"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(server.request_count(), 2);

    let requests = server.get_requests();
    assert_eq!(requests[0].path, "/v1/chat/completions");
    assert!(requests[0].body.contains("hello"));
    assert_eq!(requests[1].path, "/v1/messages");
    assert!(requests[1].body.contains("world"));
}

#[tokio::test]
async fn should_reset_scenarios_and_requests() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok").with_scenario(
            "test",
            None,
            Some("done"),
        ))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(server.scenario_state("test"), Some("done".to_string()));
    assert_eq!(server.request_count(), 1);

    server.reset();

    assert_eq!(server.scenario_state("test"), None);
    assert_eq!(server.request_count(), 0);
}
