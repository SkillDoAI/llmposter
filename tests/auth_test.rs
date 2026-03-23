#[cfg(feature = "oauth")]
use llmposter::server::OAuthConfig;
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

#[cfg(feature = "oauth")]
#[tokio::test]
async fn should_accept_oauth_issued_token_on_llm_endpoint() {
    let server = ServerBuilder::new()
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

    // Get token via client_credentials grant
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

    // Use the OAuth-issued token on an LLM endpoint
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("Authorization", format!("Bearer {}", access_token))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn should_reject_revoked_oauth_token() {
    let server = ServerBuilder::new()
        .with_oauth_defaults()
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let oauth_url = server.oauth_url().unwrap();
    let (client_id, client_secret) = server.oauth_client_credentials().await.unwrap();

    // Get token via client_credentials grant
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
    let token_body: serde_json::Value = token_resp.json().await.unwrap();
    let access_token = token_body["access_token"].as_str().unwrap();

    // Revoke the token via oauth-mock (requires Basic auth)
    let revoke_resp = client
        .post(format!("{}/revoke", oauth_url))
        .basic_auth(&client_id, Some(&client_secret))
        .form(&[("token", access_token)])
        .send()
        .await
        .unwrap();
    assert_eq!(revoke_resp.status(), 200);

    // Token should now be rejected on the LLM endpoint
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("Authorization", format!("Bearer {}", access_token))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn should_accept_oauth_with_custom_client_config() {
    let server = ServerBuilder::new()
        .with_oauth(OAuthConfig {
            client_id: "my-app".to_string(),
            client_secret: "s3cret".to_string(),
            ..OAuthConfig::default()
        })
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let oauth_url = server.oauth_url().expect("OAuth URL should be set");

    // Get token with custom credentials
    let token_resp = client
        .post(format!("{}/token", oauth_url))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", "my-app"),
            ("client_secret", "s3cret"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(token_resp.status(), 200);
    let token_body: serde_json::Value = token_resp.json().await.unwrap();
    let access_token = token_body["access_token"]
        .as_str()
        .expect("must have access_token");

    // Use the token on an LLM endpoint
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("Authorization", format!("Bearer {}", access_token))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn should_still_accept_hardcoded_bearer_token_when_oauth_enabled() {
    let server = ServerBuilder::new()
        .with_oauth_defaults()
        .with_bearer_token("static-key")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Hardcoded token should work
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("Authorization", "Bearer static-key")
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn should_reject_random_token_when_oauth_enabled() {
    let server = ServerBuilder::new()
        .with_oauth_defaults()
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Random token not issued by oauth-mock should be rejected
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("Authorization", "Bearer not-a-real-token")
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn should_support_with_oauth_custom_config() {
    let server = ServerBuilder::new()
        .with_oauth(OAuthConfig {
            client_id: "custom-id".to_string(),
            client_secret: "custom-secret".to_string(),
            ..OAuthConfig::default()
        })
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();
    assert!(server.oauth_url().is_some());
    // Debug output should work
    let debug = format!("{:?}", server);
    assert!(debug.contains("MockServer"));
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn should_return_none_for_approve_device_code_without_oauth() {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token("tok")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();
    // No OAuth configured, so oauth_url returns None
    assert!(server.oauth_url().is_none());
}

#[tokio::test]
async fn should_restore_access_when_re_added_after_exhaustion() {
    use llmposter::auth::TokenStatus;
    let auth = llmposter::AuthState::new();
    auth.add_token("tok", Some(1));
    assert_eq!(auth.check_and_use("tok"), TokenStatus::Valid); // use 1, exhausted
    assert_eq!(auth.check_and_use("tok"), TokenStatus::Exhausted);
    // Re-adding the same token clears the deny-list
    auth.add_token("tok", None);
    assert_eq!(auth.check_and_use("tok"), TokenStatus::Valid); // restored
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn should_return_none_credentials_without_oauth() {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token("tok")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();
    assert!(server.oauth_client_credentials().await.is_none());
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn should_error_approve_device_code_without_oauth() {
    let server = ServerBuilder::new()
        .with_auth(true)
        .with_bearer_token("tok")
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();
    assert!(server.approve_device_code("fake").await.is_err());
}

#[cfg(feature = "oauth")]
#[tokio::test]
async fn should_approve_device_code_with_oauth_enabled() {
    let server = ServerBuilder::new()
        .with_oauth_defaults()
        .fixture(Fixture::new().respond_with_content("hello"))
        .build()
        .await
        .unwrap();

    // approve_device_code with a non-existent user_code should return an error
    // from the OAuth server (not from the "OAuth not configured" branch).
    let result = server.approve_device_code("nonexistent").await;
    assert!(result.is_err());
    // The error should come from oauth-mock, NOT "OAuth not configured"
    let err_msg = result.unwrap_err().to_string();
    assert!(
        !err_msg.contains("OAuth not configured"),
        "Expected oauth-mock error, got: {}",
        err_msg
    );
}
