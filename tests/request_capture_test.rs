//! Tests for the expanded request capture API — v0.4.5 added
//! `RequestOutcome` and extended capture to cover malformed JSON, 400s,
//! auth-rejected requests, and the `/code/{status}` echo endpoint.

use llmposter::{Fixture, RequestOutcome, ServerBuilder};

#[tokio::test]
async fn should_capture_matched_request_with_outcome_matched() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("hi"),
        )
        .build()
        .await
        .unwrap();

    reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    let reqs = server.get_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].outcome, RequestOutcome::Matched);
    assert!(reqs[0].was_matched());
    assert!(reqs[0].body.contains("hello"));
}

#[tokio::test]
async fn should_capture_bad_json_request_with_outcome_bad_request() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("content-type", "application/json")
        .body("{{{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    let reqs = server.get_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].outcome, RequestOutcome::BadRequest);
    assert!(!reqs[0].was_matched());
    assert_eq!(reqs[0].body, "{{{not json");
}

#[tokio::test]
async fn should_capture_no_fixture_match_with_outcome_no_fixture_match() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("hi"),
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "unmatched prompt"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    let reqs = server.get_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].outcome, RequestOutcome::NoFixtureMatch);
    assert!(reqs[0].body.contains("unmatched prompt"));
}

#[tokio::test]
async fn should_capture_auth_rejected_request_with_outcome_auth_rejected() {
    let server = ServerBuilder::new()
        .with_bearer_token("valid-token")
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("authorization", "Bearer wrong-token")
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    let reqs = server.get_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].outcome, RequestOutcome::AuthRejected);
    assert_eq!(reqs[0].path, "/v1/chat/completions");
    assert_eq!(reqs[0].method, "POST");
    // Locks in the documented contract: auth rejection captures path +
    // outcome but NOT the request body (the middleware deliberately
    // does not buffer the body to stay off the hot path).
    assert_eq!(
        reqs[0].body, "",
        "auth-rejected captures should not buffer the request body"
    );
}

#[tokio::test]
async fn should_capture_code_endpoint_hit_with_outcome_code_endpoint() {
    let server = ServerBuilder::new().build().await.unwrap();

    let resp = reqwest::get(format!("{}/code/503", server.url()))
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);

    let reqs = server.get_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].outcome, RequestOutcome::CodeEndpoint);
    assert_eq!(reqs[0].path, "/code/503");
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(reqs[0].body, "");
}

#[tokio::test]
async fn should_capture_gemini_real_model_action_path_not_wildcard() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();

    reqwest::Client::new()
        .post(format!(
            "{}/v1beta/models/gemini-2.0-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();

    let reqs = server.get_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(
        reqs[0].path, "/v1beta/models/gemini-2.0-pro:generateContent",
        "real request path should be captured, not the router wildcard"
    );
    assert_eq!(reqs[0].outcome, RequestOutcome::Matched);
}

#[tokio::test]
async fn should_capture_gemini_invalid_action_as_bad_request() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!(
            "{}/v1beta/models/gemini-pro:totallyFakeAction",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    let reqs = server.get_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].outcome, RequestOutcome::BadRequest);
    assert_eq!(
        reqs[0].path, "/v1beta/models/gemini-pro:totallyFakeAction",
        "bad action request should capture the real path"
    );
    assert!(reqs[0].body.contains("hi"));
}

#[tokio::test]
async fn should_capture_invalid_code_endpoint_as_bad_request() {
    let server = ServerBuilder::new().build().await.unwrap();

    let resp = reqwest::get(format!("{}/code/999", server.url()))
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    let reqs = server.get_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(
        reqs[0].outcome,
        RequestOutcome::BadRequest,
        "invalid /code/<out-of-range> should capture as BadRequest, not CodeEndpoint"
    );
}

#[tokio::test]
async fn should_capture_non_numeric_code_endpoint_as_bad_request() {
    // `/code/abc` used to slip past capture entirely because axum's
    // `Path<u16>` extractor rejected the request before the handler
    // ran. The handler now takes `Path<String>` and parses the
    // number manually so non-numeric paths still reach capture.
    let server = ServerBuilder::new().build().await.unwrap();

    let resp = reqwest::get(format!("{}/code/abc", server.url()))
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    let reqs = server.get_requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].outcome, RequestOutcome::BadRequest);
    assert_eq!(reqs[0].path, "/code/abc");
    assert_eq!(reqs[0].method, "GET");
}

#[tokio::test]
async fn should_cap_captured_requests_at_configured_capacity() {
    // capture_capacity=3: the log should never grow past 3 entries, and
    // the oldest entries drop FIFO when newer ones arrive.
    let server = ServerBuilder::new()
        .capture_capacity(3)
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    for i in 0..5 {
        client
            .post(format!("{}/v1/chat/completions", server.url()))
            .json(&serde_json::json!({
                "model": "gpt-4",
                "messages": [{"role": "user", "content": format!("msg-{i}")}]
            }))
            .send()
            .await
            .unwrap();
    }

    let reqs = server.get_requests();
    assert_eq!(reqs.len(), 3, "capture log should be capped at 3");
    // Oldest 2 are dropped; the newest 3 survive.
    assert!(reqs[0].body.contains("msg-2"));
    assert!(reqs[1].body.contains("msg-3"));
    assert!(reqs[2].body.contains("msg-4"));
}

#[tokio::test]
async fn should_disable_capture_when_capacity_is_zero() {
    let server = ServerBuilder::new()
        .capture_capacity(0)
        .fixture(Fixture::new().respond_with_content("ok"))
        .build()
        .await
        .unwrap();

    reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(server.get_requests().len(), 0);
}

#[tokio::test]
async fn should_capture_mixed_outcomes_in_chronological_order() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("match me")
                .respond_with_content("hit"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // 1) Matched
    client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "match me"}]
        }))
        .send()
        .await
        .unwrap();
    // 2) NoFixtureMatch
    client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "nope"}]
        }))
        .send()
        .await
        .unwrap();
    // 3) CodeEndpoint
    reqwest::get(format!("{}/code/418", server.url()))
        .await
        .unwrap();

    let reqs = server.get_requests();
    assert_eq!(reqs.len(), 3);
    assert_eq!(reqs[0].outcome, RequestOutcome::Matched);
    assert_eq!(reqs[1].outcome, RequestOutcome::NoFixtureMatch);
    assert_eq!(reqs[2].outcome, RequestOutcome::CodeEndpoint);
}
