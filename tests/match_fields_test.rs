//! Integration tests for the richer match fields introduced in
//! v0.4.6: header, system_prompt, temperature, metadata, tool_schema.

use llmposter::{Fixture, ServerBuilder};

async fn post_openai(
    url: &str,
    body: serde_json::Value,
    extra_header: Option<(&str, &str)>,
) -> u16 {
    let client = reqwest::Client::new();
    let mut req = client
        .post(format!("{}/v1/chat/completions", url))
        .json(&body);
    if let Some((name, value)) = extra_header {
        req = req.header(name, value);
    }
    req.send().await.unwrap().status().as_u16()
}

#[tokio::test]
async fn should_match_on_header_value() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_header("x-tenant", "acme")
                .respond_with_content("acme-tenant"),
        )
        .build()
        .await
        .unwrap();

    let body = serde_json::json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "hi"}]
    });

    // Missing header → 404 (no match)
    assert_eq!(post_openai(&server.url(), body.clone(), None).await, 404);

    // Wrong header → 404
    assert_eq!(
        post_openai(&server.url(), body.clone(), Some(("x-tenant", "globex"))).await,
        404
    );

    // Matching header → 200
    assert_eq!(
        post_openai(&server.url(), body, Some(("x-tenant", "acme"))).await,
        200
    );
}

#[tokio::test]
async fn should_match_on_system_prompt_openai() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_system_prompt("You are a pirate")
                .respond_with_content("yarr"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // No system message → no match
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // System message that matches → 200
    let resp: serde_json::Value = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "You are a pirate. Speak like one."},
                {"role": "user", "content": "hello"}
            ]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["choices"][0]["message"]["content"], "yarr");
}

#[tokio::test]
async fn should_match_on_anthropic_top_level_system_string() {
    // Anthropic's system prompt is a top-level string (or array of text
    // blocks) on the request body, not a message entry.
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_system_prompt("helpful assistant")
                .respond_with_content("OK"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "system": "You are a helpful assistant that answers questions.",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_match_on_exact_temperature() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_temperature(0.7)
                .respond_with_content("warm"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Wrong temperature → no match
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "temperature": 0.2,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Exact match → 200
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "temperature": 0.7,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_match_on_temperature_range() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_temperature_range(Some(0.5), Some(1.0))
                .respond_with_content("in range"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Below range
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "temperature": 0.2,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // In range (inclusive)
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "temperature": 0.5,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Above range
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "temperature": 1.5,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_match_on_metadata_entry() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_metadata("customer_id", "cust-42")
                .respond_with_content("known customer"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Different metadata
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "metadata": {"customer_id": "cust-99"},
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Matching metadata
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "metadata": {"customer_id": "cust-42"},
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_match_on_tool_schema_openai_and_anthropic() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_tool_schema("get_weather")
                .respond_with_content("weather tool detected"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // OpenAI tools format: tools[].function.name
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [
                {"type": "function", "function": {"name": "get_weather", "parameters": {}}}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Anthropic tools format: tools[].name
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"name": "get_weather", "input_schema": {"type": "object"}}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_match_on_gemini_tool_schema() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_tool_schema("get_weather")
                .respond_with_content("weather"),
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "tools": [{
                "functionDeclarations": [
                    {"name": "get_weather", "description": "..."}
                ]
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_reject_fixture_with_blank_header_name() {
    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_header("   ", "value")
                .respond_with_content("ok"),
        )
        .build()
        .await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("header name must not be blank"),
        "unexpected: {err}"
    );
}

#[tokio::test]
async fn should_prefer_high_priority_fixture_regardless_of_file_order() {
    // The catch-all with `priority: 0` comes first in file order, but
    // the `priority: 100` specific fixture should win.
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("low-priority match"),
        )
        .fixture(
            Fixture::new()
                .match_user_message("hello world")
                .with_priority(100)
                .respond_with_content("high-priority specific"),
        )
        .build()
        .await
        .unwrap();

    let body: serde_json::Value = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "high-priority specific"
    );
}

#[tokio::test]
async fn should_use_catch_all_only_when_no_other_fixture_matches() {
    let server = ServerBuilder::new()
        // Catch-all listed FIRST in file order — should still be
        // reached last per the catch_all semantics.
        .fixture(
            Fixture::new()
                .as_catch_all()
                .respond_with_content("catch-all fallback"),
        )
        .fixture(
            Fixture::new()
                .match_user_message("specific")
                .respond_with_content("specific match"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Request that matches the specific fixture.
    let body: serde_json::Value = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "specific"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "specific match");

    // Request that only the catch-all handles.
    let body: serde_json::Value = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "anything else"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "catch-all fallback"
    );
}

#[tokio::test]
async fn should_reject_fixture_with_inverted_temperature_range() {
    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_temperature_range(Some(1.0), Some(0.5))
                .respond_with_content("ok"),
        )
        .build()
        .await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("range inverted"), "unexpected: {err}");
}
