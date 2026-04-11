//! Tests for the `refusal:` fixture block (v0.4.5). Each provider gets
//! its own native refusal shape; clients that branch on safety refusal
//! outcomes can exercise that path without building a fake upstream.

use llmposter::{Fixture, ServerBuilder};

#[tokio::test]
async fn should_return_openai_refusal_with_message_refusal_field() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("how to")
                .respond_with_refusal("I cannot help with that request."),
        )
        .build()
        .await
        .unwrap();

    let body: serde_json::Value = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "how to do bad thing"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(
        body["choices"][0]["message"]["refusal"],
        "I cannot help with that request."
    );
    assert!(body["choices"][0]["message"]["content"].is_null());
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
}

#[tokio::test]
async fn should_return_anthropic_refusal_with_refusal_stop_reason() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("bad")
                .respond_with_refusal("I cannot help with that."),
        )
        .build()
        .await
        .unwrap();

    let body: serde_json::Value = reqwest::Client::new()
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "bad thing"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["stop_reason"], "refusal");
    assert_eq!(body["content"][0]["type"], "text");
    assert_eq!(body["content"][0]["text"], "I cannot help with that.");
}

#[tokio::test]
async fn should_return_gemini_refusal_with_empty_candidates_and_block_reason() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("bad")
                .respond_with_refusal("Prompt violates safety policy."),
        )
        .build()
        .await
        .unwrap();

    let body: serde_json::Value = reqwest::Client::new()
        .post(format!(
            "{}/v1beta/models/gemini-2.0-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "bad request"}]}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["candidates"].as_array().unwrap().len(), 0);
    assert_eq!(body["promptFeedback"]["blockReason"], "SAFETY");
    assert_eq!(
        body["promptFeedback"]["blockReasonMessage"],
        "Prompt violates safety policy."
    );
}

#[tokio::test]
async fn should_return_responses_refusal_with_refusal_content_part() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("bad")
                .respond_with_refusal("Refusing this request."),
        )
        .build()
        .await
        .unwrap();

    let body: serde_json::Value = reqwest::Client::new()
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4.1",
            "input": "bad prompt"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["status"], "completed");
    let output = &body["output"][0];
    assert_eq!(output["type"], "message");
    let part = &output["content"][0];
    assert_eq!(part["type"], "refusal");
    assert_eq!(part["refusal"], "Refusing this request.");
}

#[tokio::test]
async fn should_reject_fixture_combining_refusal_and_response() {
    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("hi")
                .respond_with_refusal("no"),
        )
        .build()
        .await;
    assert!(result.is_err(), "refusal + response should fail validation");
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("mutually exclusive"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn should_reject_fixture_combining_refusal_and_error() {
    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .with_error(429, "rate limited")
                .respond_with_refusal("no"),
        )
        .build()
        .await;
    assert!(result.is_err());
    assert!(format!("{}", result.unwrap_err()).contains("mutually exclusive"));
}

#[tokio::test]
async fn should_reject_fixture_combining_refusal_and_failure() {
    use llmposter::fixture::FailureConfig;
    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .with_failure(FailureConfig {
                    latency_ms: Some(100),
                    ..Default::default()
                })
                .respond_with_refusal("no"),
        )
        .build()
        .await;
    assert!(result.is_err());
    assert!(format!("{}", result.unwrap_err()).contains("mutually exclusive"));
}

#[tokio::test]
async fn should_reject_fixture_with_blank_refusal_reason() {
    let result = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_refusal("   "))
        .build()
        .await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("blank"), "unexpected error: {msg}");
}

#[tokio::test]
async fn should_reject_streaming_request_against_refusal_fixture_with_400() {
    // Refusal fixtures return a non-streaming JSON body only; a client
    // that asks for `stream: true` gets a 400 with a clear message
    // rather than an SSE-parser-mismatching `application/json` body.
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("bad")
                .respond_with_refusal("cannot help"),
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "stream": true,
            "messages": [{"role": "user", "content": "bad prompt"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("do not currently support streaming"),
        "unexpected error: {body}"
    );
}

#[tokio::test]
async fn should_capture_refused_request_as_matched_outcome() {
    use llmposter::RequestOutcome;
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("bad")
                .respond_with_refusal("no."),
        )
        .build()
        .await
        .unwrap();

    reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "bad thing"}]
        }))
        .send()
        .await
        .unwrap();

    let reqs = server.get_requests();
    assert_eq!(reqs.len(), 1);
    // A refusal fixture is still a matched fixture.
    assert_eq!(reqs[0].outcome, RequestOutcome::Matched);
    assert!(reqs[0].was_matched());
}
