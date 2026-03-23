use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::server::AppState;

/// Bearer token state for authentication enforcement.
/// Tracks valid tokens and their remaining uses.
pub struct AuthState {
    tokens: RwLock<HashMap<String, Option<u64>>>,
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
        }
    }

    /// Add a token. `max_uses` of `None` = unlimited.
    pub fn add_token(&self, token: &str, max_uses: Option<u64>) {
        self.tokens
            .write()
            .unwrap()
            .insert(token.to_string(), max_uses);
    }

    /// Check token validity and decrement use count. Returns `true` if valid.
    pub fn check_and_use(&self, token: &str) -> bool {
        let mut tokens = self.tokens.write().unwrap();
        match tokens.get_mut(token) {
            Some(Some(remaining)) if *remaining > 0 => {
                *remaining -= 1;
                if *remaining == 0 {
                    tokens.remove(token);
                }
                true
            }
            Some(Some(_)) => {
                tokens.remove(token);
                false
            }
            Some(None) => true, // unlimited
            None => false,
        }
    }

    /// Revoke a token.
    pub fn revoke(&self, token: &str) {
        self.tokens.write().unwrap().remove(token);
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
        Some(t) if auth.check_and_use(t) => next.run(request).await,
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
        [(header::CONTENT_TYPE, "application/json")],
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
        assert!(state.check_and_use("tok-1"));
    }

    #[test]
    fn should_reject_unknown_token() {
        let state = AuthState::new();
        assert!(!state.check_and_use("unknown"));
    }

    #[test]
    fn should_expire_after_n_uses() {
        let state = AuthState::new();
        state.add_token("tok-1", Some(2));
        assert!(state.check_and_use("tok-1")); // use 1
        assert!(state.check_and_use("tok-1")); // use 2
        assert!(!state.check_and_use("tok-1")); // expired
    }

    #[test]
    fn should_remove_revoked_token() {
        let state = AuthState::new();
        state.add_token("tok-1", None);
        state.revoke("tok-1");
        assert!(!state.check_and_use("tok-1"));
    }

    #[test]
    fn should_accept_unlimited_token_many_times() {
        let state = AuthState::new();
        state.add_token("unlimited", None);
        for _ in 0..100 {
            assert!(state.check_and_use("unlimited"));
        }
    }
}
