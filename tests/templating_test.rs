//! Integration tests for response templating (`content_template`).
//!
//! The entire file is gated on the `templating` feature — when compiled
//! without it, the file is empty. Feature-off validation behavior (hard
//! error at fixture load time) is covered by unit tests in
//! `src/fixture.rs`.

#![cfg(feature = "templating")]

use llmposter::fixture::FixtureResponse;
use llmposter::{Fixture, ServerBuilder};

async fn post_openai(url: &str, msg: &str, model: &str) -> serde_json::Value {
    reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", url))
        .json(&serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": msg}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn should_interpolate_user_message_into_response() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content_template: Some("echo: {{ user_message }}".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        })
        .build()
        .await
        .unwrap();

    let body = post_openai(&server.url(), "hello there", "gpt-4").await;
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "echo: hello there"
    );
}

#[tokio::test]
async fn should_interpolate_model_and_provider_into_template() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content_template: Some("{{ provider }}/{{ model }}".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        })
        .build()
        .await
        .unwrap();

    let body = post_openai(&server.url(), "hi", "gpt-4-turbo").await;
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "openai/gpt-4-turbo"
    );
}

#[tokio::test]
async fn should_reach_into_request_json_via_template() {
    let server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content_template: Some("first msg: {{ request.messages[0].role }}".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        })
        .build()
        .await
        .unwrap();

    let body = post_openai(&server.url(), "hi", "gpt-4").await;
    assert_eq!(body["choices"][0]["message"]["content"], "first msg: user");
}

#[tokio::test]
async fn should_return_500_on_template_syntax_error_at_render_time() {
    // Unknown filter is a render-time error, not load-time.
    let server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content_template: Some("{{ user_message | no_such_filter }}".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        })
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 500);
    let text = resp.text().await.unwrap();
    assert!(
        text.contains("content_template") || text.contains("template render"),
        "expected template error in body, got: {}",
        text
    );
}

#[tokio::test]
async fn should_reject_fixture_with_content_and_content_template_both_set() {
    let result = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content: Some("plain".to_string()),
                content_template: Some("{{ user_message }}".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        })
        .build()
        .await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("mutually exclusive"),
        "expected mutually-exclusive error, got: {}",
        msg
    );
}

#[tokio::test]
async fn should_reject_fixture_with_tool_calls_and_content_template() {
    let result = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content_template: Some("{{ user_message }}".to_string()),
                tool_calls: Some(vec![llmposter::ToolCall {
                    name: "get_weather".to_string(),
                    arguments: serde_json::json!({}),
                }]),
                ..Default::default()
            }),
            ..Fixture::new()
        })
        .build()
        .await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("mutually exclusive"),
        "expected mutually-exclusive error, got: {}",
        msg
    );
}

#[tokio::test]
async fn should_reject_fixture_with_invalid_template_syntax() {
    let result = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content_template: Some("{{ unclosed".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        })
        .build()
        .await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("content_template compile error"),
        "expected compile error, got: {}",
        msg
    );
}

#[tokio::test]
async fn should_interpolate_into_streaming_response() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture {
                response: Some(FixtureResponse {
                    content_template: Some("streamed: {{ user_message }}".to_string()),
                    ..Default::default()
                }),
                ..Fixture::new()
            }
            .with_streaming(Some(0), Some(5)),
        )
        .build()
        .await
        .unwrap();

    let body = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "world"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    // SSE data frames must concatenate to the rendered output.
    let mut reassembled = String::new();
    for line in body.lines() {
        if let Some(json) = line.strip_prefix("data: ") {
            if json == "[DONE]" {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(json) {
                if let Some(c) = v["choices"][0]["delta"]["content"].as_str() {
                    reassembled.push_str(c);
                }
            }
        }
    }
    assert_eq!(reassembled, "streamed: world");
}
