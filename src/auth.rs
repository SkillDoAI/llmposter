use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::server::AppState;

/// OAuth introspect configuration for validating tokens issued by oauth-mock.
#[cfg(feature = "oauth")]
#[derive(Clone)]
pub(crate) struct OAuthIntrospect {
    /// Token introspection endpoint URL.
    pub url: String,
    /// OAuth client ID for introspection requests.
    pub client_id: String,
    /// OAuth client secret for introspection requests.
    pub client_secret: String,
    /// HTTP client for making introspection requests.
    pub client: reqwest::Client,
}

/// Result of checking a token against the hardcoded token store.
#[derive(Debug, PartialEq)]
pub enum TokenStatus {
    /// Token is valid (accepted).
    Valid,
    /// Token was explicitly exhausted or revoked (deny-listed).
    Exhausted,
    /// Token is not in the hardcoded store (may still be a valid OAuth token).
    Unknown,
}

/// Bearer token state for authentication enforcement.
/// Tracks valid tokens and their remaining uses.
pub struct AuthState {
    tokens: RwLock<HashMap<String, Option<u64>>>,
    /// Tokens explicitly exhausted via `ServerBuilder::with_bearer_token_uses()`.
    /// Prevents OAuth fallthrough from bypassing use limits.
    exhausted: RwLock<std::collections::HashSet<String>>,
    #[cfg(feature = "oauth")]
    oauth_introspect: RwLock<Option<OAuthIntrospect>>,
}

impl Default for AuthState {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthState {
    /// Create a new, empty `AuthState` with no tokens registered.
    pub fn new() -> Self {
        Self {
            tokens: RwLock::new(HashMap::new()),
            exhausted: RwLock::new(std::collections::HashSet::new()),
            #[cfg(feature = "oauth")]
            oauth_introspect: RwLock::new(None),
        }
    }

    /// Add a token. `max_uses` of `None` = unlimited.
    /// Clears the token from the deny-list if it was previously exhausted or revoked,
    /// allowing the same token string to be re-issued.
    pub fn add_token(&self, token: &str, max_uses: Option<u64>) {
        // Hold both locks atomically (tokens → exhausted ordering, consistent
        // with check_and_use and revoke) to prevent a window where the token
        // appears in neither map.
        let mut tokens = self.tokens.write().unwrap_or_else(|e| e.into_inner());
        let mut exhausted = self.exhausted.write().unwrap_or_else(|e| e.into_inner());
        exhausted.remove(token);
        tokens.insert(token.to_string(), max_uses);
    }

    /// Check token validity and decrement use count.
    /// Returns `Valid`, `Exhausted` (deny-listed), or `Unknown` (not a hardcoded token).
    pub fn check_and_use(&self, token: &str) -> TokenStatus {
        // Fast path: read-only check of the deny-list.
        if self
            .exhausted
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains(token)
        {
            return TokenStatus::Exhausted;
        }

        // Acquire both locks when mutation may move a token to the deny-list.
        // Lock ordering (tokens → exhausted) is consistent with revoke().
        let mut tokens = self.tokens.write().unwrap_or_else(|e| e.into_inner());
        match tokens.get_mut(token) {
            Some(Some(remaining)) if *remaining > 0 => {
                *remaining -= 1;
                if *remaining == 0 {
                    tokens.remove(token);
                    // Hold tokens lock while inserting into exhausted to prevent
                    // a TOCTOU gap where the token appears in neither map.
                    self.exhausted
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(token.to_string());
                }
                TokenStatus::Valid
            }
            Some(Some(_)) => {
                tokens.remove(token);
                self.exhausted
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(token.to_string());
                TokenStatus::Exhausted
            }
            Some(None) => TokenStatus::Valid,
            None => {
                // Re-check deny-list under the tokens lock to catch a concurrent
                // revoke() or exhaustion that completed between the fast-path
                // read and acquiring this write lock.
                if self
                    .exhausted
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains(token)
                {
                    TokenStatus::Exhausted
                } else {
                    TokenStatus::Unknown
                }
            }
        }
    }

    /// Revoke a token. Atomically removes from tokens and adds to deny-list.
    pub fn revoke(&self, token: &str) {
        let mut tokens = self.tokens.write().unwrap_or_else(|e| e.into_inner());
        let mut exhausted = self.exhausted.write().unwrap_or_else(|e| e.into_inner());
        tokens.remove(token);
        exhausted.insert(token.to_string());
    }

    /// Set the OAuth introspect configuration for validating oauth-mock tokens.
    #[cfg(feature = "oauth")]
    pub(crate) fn set_oauth_introspect(&self, config: OAuthIntrospect) {
        *self
            .oauth_introspect
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(config);
    }

    /// Validate a token via the oauth-mock introspect endpoint (localhost HTTP call).
    #[cfg(feature = "oauth")]
    pub(crate) async fn check_oauth_token(&self, token: &str) -> bool {
        let config = {
            let guard = self
                .oauth_introspect
                .read()
                .unwrap_or_else(|e| e.into_inner());
            match guard.as_ref() {
                Some(c) => c.clone(),
                None => return false,
            }
        };
        let resp = config
            .client
            .post(&config.url)
            .basic_auth(&config.client_id, Some(&config.client_secret))
            .form(&[("token", token)])
            .send()
            .await;
        match resp {
            Ok(r) => {
                if let Ok(body) = r.json::<serde_json::Value>().await {
                    body.get("active")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                } else {
                    eprintln!(
                        "[llmposter] OAuth introspect: failed to parse response body as JSON"
                    );
                    false
                }
            }
            Err(e) => {
                eprintln!("[llmposter] OAuth introspect request failed: {e}");
                false
            }
        }
    }
}

/// Bearer auth middleware. Skips check if auth is not enabled.
pub(crate) async fn bearer_auth_check(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let auth = match &state.auth {
        Some(a) => a,
        None => return next.run(request).await,
    };

    let path = request.uri().path().to_string();

    // Auth applies only to LLM endpoints — all other routes pass through.
    let is_llm_route = path.starts_with("/v1/") || path.starts_with("/v1beta/");
    if !is_llm_route {
        return next.run(request).await;
    }
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            // RFC 7235: auth-scheme is case-insensitive
            if v.len() > 7 && v[..7].eq_ignore_ascii_case("bearer ") {
                Some(&v[7..])
            } else {
                None
            }
        });

    match token {
        Some(t) => {
            let status = auth.check_and_use(t);
            let is_valid = match status {
                TokenStatus::Valid => true,
                TokenStatus::Exhausted => false,
                TokenStatus::Unknown => {
                    #[cfg(feature = "oauth")]
                    {
                        auth.check_oauth_token(t).await
                    }
                    #[cfg(not(feature = "oauth"))]
                    false
                }
            };
            if is_valid {
                next.run(request).await
            } else {
                auth_error_response(&path)
            }
        }
        _ => auth_error_response(&path),
    }
}

/// Build provider-specific 401 response based on request path.
fn auth_error_response(path: &str) -> Response {
    let body = if path.starts_with("/v1/messages") {
        // Anthropic
        serde_json::json!({
            "type": "error",
            "error": {
                "type": "authentication_error",
                "message": "Invalid bearer token"
            }
        })
    } else if path.starts_with("/v1beta/models") {
        // Gemini
        serde_json::json!({
            "error": {
                "code": 401,
                "message": "Invalid bearer token",
                "status": "UNAUTHENTICATED"
            }
        })
    } else {
        // OpenAI / Responses
        serde_json::json!({
            "error": {
                "message": "Invalid bearer token",
                "type": "authentication_error",
                "param": null,
                "code": "invalid_api_key"
            }
        })
    };
    (
        StatusCode::UNAUTHORIZED,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::WWW_AUTHENTICATE, "Bearer realm=\"api\""),
        ],
        body.to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_accept_valid_token() {
        let state = AuthState::new();
        state.add_token("tok-1", None);
        assert_eq!(state.check_and_use("tok-1"), TokenStatus::Valid);
    }

    #[test]
    fn should_reject_unknown_token() {
        let state = AuthState::new();
        assert_eq!(state.check_and_use("unknown"), TokenStatus::Unknown);
    }

    #[test]
    fn should_expire_after_n_uses() {
        let state = AuthState::new();
        state.add_token("tok-1", Some(2));
        assert_eq!(state.check_and_use("tok-1"), TokenStatus::Valid);
        assert_eq!(state.check_and_use("tok-1"), TokenStatus::Valid);
        assert_eq!(state.check_and_use("tok-1"), TokenStatus::Exhausted);
    }

    #[test]
    fn should_remove_revoked_token() {
        let state = AuthState::new();
        state.add_token("tok-1", None);
        state.revoke("tok-1");
        assert_eq!(state.check_and_use("tok-1"), TokenStatus::Exhausted);
    }

    #[test]
    fn should_accept_unlimited_token_many_times() {
        let state = AuthState::new();
        state.add_token("unlimited", None);
        for _ in 0..100 {
            assert_eq!(state.check_and_use("unlimited"), TokenStatus::Valid);
        }
    }

    #[test]
    fn should_support_default_trait() {
        let state = AuthState::default();
        state.add_token("tok", None);
        assert_eq!(state.check_and_use("tok"), TokenStatus::Valid);
    }

    #[test]
    fn should_reject_zero_use_token() {
        let state = AuthState::new();
        state.add_token("zero", Some(0));
        assert_eq!(state.check_and_use("zero"), TokenStatus::Exhausted);
    }

    #[test]
    fn should_allow_re_add_after_revoke() {
        let state = AuthState::new();
        state.add_token("tok", None);
        state.revoke("tok");
        assert_eq!(state.check_and_use("tok"), TokenStatus::Exhausted);
        // Re-adding should clear the deny-list and restore access
        state.add_token("tok", None);
        assert_eq!(state.check_and_use("tok"), TokenStatus::Valid);
    }

    #[test]
    fn should_allow_re_add_after_exhaustion() {
        let state = AuthState::new();
        state.add_token("tok", Some(1));
        assert_eq!(state.check_and_use("tok"), TokenStatus::Valid);
        assert_eq!(state.check_and_use("tok"), TokenStatus::Exhausted);
        // Re-adding should clear the deny-list and restore access
        state.add_token("tok", Some(2));
        assert_eq!(state.check_and_use("tok"), TokenStatus::Valid);
        assert_eq!(state.check_and_use("tok"), TokenStatus::Valid);
        assert_eq!(state.check_and_use("tok"), TokenStatus::Exhausted);
    }

    #[cfg(feature = "oauth")]
    #[tokio::test]
    async fn should_return_false_when_introspect_unreachable() {
        let state = AuthState::new();
        state.set_oauth_introspect(OAuthIntrospect {
            url: "http://127.0.0.1:1/introspect".to_string(),
            client_id: "test".to_string(),
            client_secret: "secret".to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(100))
                .build()
                .unwrap(),
        });
        assert!(!state.check_oauth_token("any").await);
    }

    #[cfg(feature = "oauth")]
    #[tokio::test]
    async fn should_return_false_when_introspect_not_configured() {
        let state = AuthState::new();
        assert!(!state.check_oauth_token("any").await);
    }

    #[cfg(feature = "oauth")]
    #[tokio::test]
    async fn should_return_false_when_introspect_returns_non_json() {
        use axum::{routing::post, Router};

        // Spin up a tiny server that returns non-JSON on POST
        let app = Router::new().route("/introspect", post(|| async { "this is not json" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let state = AuthState::new();
        state.set_oauth_introspect(OAuthIntrospect {
            url: format!("http://127.0.0.1:{}/introspect", port),
            client_id: "test".to_string(),
            client_secret: "secret".to_string(),
            client: reqwest::Client::new(),
        });
        assert!(!state.check_oauth_token("any").await);
    }
}
