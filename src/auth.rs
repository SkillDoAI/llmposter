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
    pub url: String,
    pub client_id: String,
    pub client_secret: String,
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
    /// Tokens that were explicitly exhausted via expires_after_uses.
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
    pub fn new() -> Self {
        Self {
            tokens: RwLock::new(HashMap::new()),
            exhausted: RwLock::new(std::collections::HashSet::new()),
            #[cfg(feature = "oauth")]
            oauth_introspect: RwLock::new(None),
        }
    }

    /// Add a token. `max_uses` of `None` = unlimited.
    pub fn add_token(&self, token: &str, max_uses: Option<u64>) {
        self.tokens
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token.to_string(), max_uses);
    }

    /// Check token validity and decrement use count.
    /// Returns `Valid`, `Exhausted` (deny-listed), or `Unknown` (not a hardcoded token).
    pub fn check_and_use(&self, token: &str) -> TokenStatus {
        // Check deny-list first (exhausted or revoked tokens).
        // Only needs a read lock — no contention with tokens.
        if self
            .exhausted
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains(token)
        {
            return TokenStatus::Exhausted;
        }

        // Scope tokens write-lock tightly: drop it before touching exhausted.
        let (result, exhausted_token) = {
            let mut tokens = self.tokens.write().unwrap_or_else(|e| e.into_inner());
            match tokens.get_mut(token) {
                Some(Some(remaining)) if *remaining > 0 => {
                    *remaining -= 1;
                    if *remaining == 0 {
                        tokens.remove(token);
                        (TokenStatus::Valid, Some(token.to_string()))
                    } else {
                        (TokenStatus::Valid, None)
                    }
                }
                Some(Some(_)) => {
                    tokens.remove(token);
                    (TokenStatus::Exhausted, Some(token.to_string()))
                }
                Some(None) => (TokenStatus::Valid, None),
                None => (TokenStatus::Unknown, None),
            }
        }; // tokens lock dropped here

        if let Some(t) = exhausted_token {
            self.exhausted
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .insert(t);
        }
        result
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
                    false
                }
            }
            Err(_) => false,
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
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

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
}
