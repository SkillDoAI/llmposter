//! Auth gating for the embedded debug UI.
//!
//! When bearer auth is enabled, `/ui` and everything under it must
//! require a valid token too — the UI exposes captured request bodies,
//! so leaving it open while the LLM endpoints are locked down would
//! leak all traffic. Browsers can't attach an `Authorization` header
//! to a page load or `EventSource`, so a `?token=` query parameter is
//! accepted as an alternative on UI routes only.
#![cfg(feature = "ui")]

use llmposter::{Fixture, ServerBuilder};

fn chat_body() -> serde_json::Value {
    serde_json::json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "hi"}]
    })
}

#[tokio::test]
async fn should_serve_ui_unauthenticated_when_auth_disabled() {
    let server = ServerBuilder::new()
        .ui(true)
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    for path in ["/ui", "/ui/requests", "/ui/fixtures", "/ui/meta"] {
        let resp = client
            .get(format!("{}{}", server.url(), path))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "GET {} without auth configured", path);
    }
}

#[tokio::test]
async fn should_reject_ui_without_token_when_auth_enabled() {
    let server = ServerBuilder::new()
        .ui(true)
        .with_bearer_token("valid-token")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    for path in [
        "/ui",
        "/ui/requests",
        "/ui/fixtures",
        "/ui/meta",
        "/ui/events",
    ] {
        let resp = client
            .get(format!("{}{}", server.url(), path))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 401, "GET {} must require auth", path);
        // 401s still carry x-request-id like the LLM endpoints
        assert!(resp.headers().get("x-request-id").is_some());
        assert!(resp.headers().get("www-authenticate").is_some());
    }

    let resp = client
        .post(format!("{}/ui/debug", server.url()))
        .json(&serde_json::json!({"provider": "openai", "body": "{}"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401, "POST /ui/debug must require auth");
}

#[tokio::test]
async fn should_reject_ui_with_invalid_token() {
    let server = ServerBuilder::new()
        .ui(true)
        .with_bearer_token("valid-token")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/ui/requests", server.url()))
        .header("Authorization", "Bearer wrong-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    let resp = client
        .get(format!("{}/ui/requests?token=wrong-token", server.url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn should_accept_ui_with_bearer_header() {
    let server = ServerBuilder::new()
        .ui(true)
        .with_bearer_token("valid-token")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    for path in ["/ui", "/ui/requests", "/ui/fixtures", "/ui/meta"] {
        let resp = client
            .get(format!("{}{}", server.url(), path))
            .header("Authorization", "Bearer valid-token")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "GET {} with valid header", path);
    }
}

#[tokio::test]
async fn should_accept_ui_with_token_query_param() {
    let server = ServerBuilder::new()
        .ui(true)
        .with_bearer_token("valid-token")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    for path in ["/ui", "/ui/requests", "/ui/events"] {
        let resp = client
            .get(format!("{}{}?token=valid-token", server.url(), path))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            200,
            "GET {}?token=... with valid token",
            path
        );
    }
}

#[tokio::test]
async fn should_accept_percent_encoded_query_token() {
    // RFC 6750 b64token charset includes '+', '/', and '=' — all of
    // which encodeURIComponent percent-encodes in a query string.
    let server = ServerBuilder::new()
        .ui(true)
        .with_bearer_token("tok+base64/chars=")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "{}/ui/requests?token=tok%2Bbase64%2Fchars%3D",
            server.url()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_not_consume_token_uses_for_ui_requests() {
    let server = ServerBuilder::new()
        .ui(true)
        .with_bearer_token_uses("one-shot", 1)
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    // UI access must not burn the token's single use.
    for _ in 0..3 {
        let resp = client
            .get(format!("{}/ui/requests?token=one-shot", server.url()))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    // The single LLM use is still available...
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("Authorization", "Bearer one-shot")
        .json(&chat_body())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // ...and exactly one: the second LLM call is rejected.
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("Authorization", "Bearer one-shot")
        .json(&chat_body())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn should_reject_exhausted_token_for_ui() {
    let server = ServerBuilder::new()
        .ui(true)
        .with_bearer_token_uses("one-shot", 1)
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    // Burn the token's single use on an LLM call.
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("Authorization", "Bearer one-shot")
        .json(&chat_body())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // The exhausted token no longer opens the UI.
    let resp = client
        .get(format!("{}/ui/requests?token=one-shot", server.url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn should_return_html_401_for_ui_page_and_json_for_ui_api() {
    let server = ServerBuilder::new()
        .ui(true)
        .with_bearer_token("valid-token")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // The page route gets a human-readable HTML hint...
    let resp = client
        .get(format!("{}/ui", server.url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        content_type.starts_with("text/html"),
        "expected text/html on /ui 401, got {}",
        content_type
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("?token="),
        "401 page should explain how to pass a token"
    );

    // ...while the JSON API routes answer in JSON.
    let resp = client
        .get(format!("{}/ui/requests", server.url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        content_type.starts_with("application/json"),
        "expected application/json on /ui/requests 401, got {}",
        content_type
    );
}

#[tokio::test]
async fn should_not_accept_query_token_on_llm_endpoints() {
    // The ?token= escape hatch exists for browser SSE only — the LLM
    // endpoints stay spec-realistic and accept the header exclusively.
    let server = ServerBuilder::new()
        .ui(true)
        .with_bearer_token("valid-token")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1/chat/completions?token=valid-token",
            server.url()
        ))
        .json(&chat_body())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn should_allow_unauthenticated_ui_when_opted_out() {
    // ui_auth(false): the LLM endpoints still enforce tokens, but the
    // UI stays open — for setups where auth exists only to exercise a
    // client's 401 handling on a localhost-bound server.
    let server = ServerBuilder::new()
        .ui(true)
        .ui_auth(false)
        .with_bearer_token("valid-token")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/ui", server.url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "UI must be open after ui_auth(false)");

    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&chat_body())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401, "LLM endpoints must still enforce auth");
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn should_accept_oauth_issued_token_for_ui() {
    let server = ServerBuilder::new()
        .ui(true)
        .with_oauth_defaults()
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let oauth_url = server.oauth_url().expect("OAuth URL should be set");
    let (client_id, client_secret) = server
        .oauth_client_credentials()
        .await
        .expect("should have credentials");

    let token_resp = client
        .post(format!("{}/token", oauth_url))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(token_resp.status(), 200);
    let token_body: serde_json::Value = token_resp.json().await.unwrap();
    let access_token = token_body["access_token"]
        .as_str()
        .expect("must have access_token");

    // OAuth-issued tokens open the UI via header and query param alike.
    let resp = client
        .get(format!("{}/ui/requests", server.url()))
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let resp = client
        .get(format!(
            "{}/ui/requests?token={}",
            server.url(),
            access_token
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}
