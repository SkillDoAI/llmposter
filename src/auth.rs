use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
#[cfg(feature = "oauth")]
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

/// Inner token store, guarded by a single `Mutex` so every mutation
/// or lookup sees a consistent snapshot of `(tokens, exhausted)`.
///
/// Using one lock instead of two separate `RwLock`s eliminates the
/// possibility of an ABBA deadlock across token admin + request
/// dispatch (previously safe via consistent lock ordering, but fragile
/// to future refactors).
#[derive(Default)]
struct TokenStore {
    /// Token → remaining uses (None = unlimited).
    tokens: HashMap<String, Option<u64>>,
    /// Tokens explicitly exhausted / revoked. Deny-list prevents
    /// OAuth fallthrough from bypassing use limits.
    exhausted: HashSet<String>,
}

/// Bearer token state for authentication enforcement.
/// Tracks valid tokens and their remaining uses.
pub struct AuthState {
    store: Mutex<TokenStore>,
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
            store: Mutex::new(TokenStore::default()),
            #[cfg(feature = "oauth")]
            oauth_introspect: RwLock::new(None),
        }
    }

    /// Add a token. `max_uses` of `None` = unlimited.
    /// Clears the token from the deny-list if it was previously exhausted or revoked,
    /// allowing the same token string to be re-issued.
    pub fn add_token(&self, token: &str, max_uses: Option<u64>) {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        store.exhausted.remove(token);
        store.tokens.insert(token.to_string(), max_uses);
    }

    /// Check token validity and decrement use count.
    /// Returns `Valid`, `Exhausted` (deny-listed), or `Unknown` (not a hardcoded token).
    pub fn check_and_use(&self, token: &str) -> TokenStatus {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if store.exhausted.contains(token) {
            return TokenStatus::Exhausted;
        }
        match store.tokens.get_mut(token) {
            Some(Some(remaining)) if *remaining > 0 => {
                *remaining -= 1;
                if *remaining == 0 {
                    store.tokens.remove(token);
                    store.exhausted.insert(token.to_string());
                }
                TokenStatus::Valid
            }
            Some(Some(_)) => {
                store.tokens.remove(token);
                store.exhausted.insert(token.to_string());
                TokenStatus::Exhausted
            }
            Some(None) => TokenStatus::Valid,
            None => TokenStatus::Unknown,
        }
    }

    /// Revoke a token. Atomically removes from tokens and adds to deny-list.
    pub fn revoke(&self, token: &str) {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        store.tokens.remove(token);
        store.exhausted.insert(token.to_string());
    }

    /// Check token validity without consuming a use. The debug UI polls
    /// and holds an SSE stream open, so gating it must not burn
    /// `max_uses` — those are budgeted for LLM calls.
    #[cfg(feature = "ui")]
    pub(crate) fn validate(&self, token: &str) -> TokenStatus {
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if store.exhausted.contains(token) {
            return TokenStatus::Exhausted;
        }
        match store.tokens.get(token) {
            Some(Some(0)) => TokenStatus::Exhausted,
            Some(_) => TokenStatus::Valid,
            None => TokenStatus::Unknown,
        }
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

    // Debug UI gate: same tokens as the LLM endpoints, but validated
    // without consuming `max_uses` (the UI polls), and with a `?token=`
    // query fallback because a browser can't attach an Authorization
    // header to a page load or an EventSource connection.
    #[cfg(feature = "ui")]
    if state.ui_require_auth && (path == "/ui" || path.starts_with("/ui/")) {
        let token = bearer_token(request.headers())
            .map(str::to_string)
            .or_else(|| request.uri().query().and_then(query_token));
        let authorized = match token {
            Some(t) => match auth.validate(&t) {
                TokenStatus::Valid => true,
                TokenStatus::Exhausted => false,
                TokenStatus::Unknown => {
                    #[cfg(feature = "oauth")]
                    {
                        auth.check_oauth_token(&t).await
                    }
                    #[cfg(not(feature = "oauth"))]
                    false
                }
            },
            None => false,
        };
        return if authorized {
            next.run(request).await
        } else {
            ui_auth_error_response(&path)
        };
    }

    // Auth applies only to LLM endpoints — all other routes pass through.
    let is_llm_route = path.starts_with("/v1/") || path.starts_with("/v1beta/");
    if !is_llm_route {
        return next.run(request).await;
    }
    let token = bearer_token(request.headers());

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
                capture_auth_reject(&state, request.method().as_str(), &path);
                auth_error_response(&path)
            }
        }
        _ => {
            capture_auth_reject(&state, request.method().as_str(), &path);
            auth_error_response(&path)
        }
    }
}

/// Extract the bearer token from the Authorization header.
/// RFC 7235: the auth-scheme is case-insensitive.
fn bearer_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            if v.len() > 7 && v[..7].eq_ignore_ascii_case("bearer ") {
                Some(&v[7..])
            } else {
                None
            }
        })
}

/// Extract and percent-decode the `token` query parameter.
#[cfg(feature = "ui")]
fn query_token(query: &str) -> Option<String> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("token=").and_then(percent_decode))
}

/// Minimal percent-decoder for the token query value. `+` decodes to a
/// space per form encoding (matching `URLSearchParams`); malformed
/// escapes and non-UTF-8 results return `None`. Kept local instead of
/// pulling in a URL crate — it only ever sees one query value.
#[cfg(feature = "ui")]
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
                let hi = hex(*bytes.get(i + 1)?)?;
                let lo = hex(*bytes.get(i + 2)?)?;
                out.push(hi * 16 + lo);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// 401 page shown when the debug UI is opened without a token.
#[cfg(feature = "ui")]
const UI_401_HTML: &str = r#"<!doctype html>
<html><head><title>llmposter — authentication required</title></head>
<body style="font-family: system-ui, sans-serif; max-width: 40rem; margin: 4rem auto; line-height: 1.5;">
<h1>Authentication required</h1>
<p>This server runs with bearer auth enabled, so the debug UI is locked too.</p>
<p>Open <code>/ui?token=&lt;your-bearer-token&gt;</code> to sign in. Keep the
<code>?token=</code> parameter in the URL — it's what authenticates reloads.</p>
</body></html>
"#;

/// Build the 401 for gated UI routes: a human-readable HTML hint for
/// the page itself, JSON for the API routes the page's JS calls.
#[cfg(feature = "ui")]
fn ui_auth_error_response(path: &str) -> Response {
    if path == "/ui" {
        (
            StatusCode::UNAUTHORIZED,
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::WWW_AUTHENTICATE, "Bearer realm=\"ui\""),
            ],
            UI_401_HTML,
        )
            .into_response()
    } else {
        let body = serde_json::json!({
            "error": {
                "message": "The debug UI requires a bearer token when auth is enabled. \
                            Send Authorization: Bearer <token> or append ?token=<token>.",
                "type": "authentication_error"
            }
        });
        (
            StatusCode::UNAUTHORIZED,
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::WWW_AUTHENTICATE, "Bearer realm=\"ui\""),
            ],
            body.to_string(),
        )
            .into_response()
    }
}

/// Record an auth-rejected request so `MockServer::get_requests()` shows
/// it alongside matched traffic. The body isn't buffered here — tests
/// that need to diff the rejected body can still read it from their own
/// client side, and avoiding the buffer keeps the auth hot path simple.
fn capture_auth_reject(state: &AppState, method: &str, path: &str) {
    crate::handler::capture_non_matched(
        state,
        method,
        path,
        "",
        crate::server::RequestOutcome::AuthRejected,
    );
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

    #[cfg(feature = "ui")]
    #[test]
    fn should_validate_without_consuming_uses() {
        let state = AuthState::new();
        state.add_token("tok", Some(1));
        // validate() any number of times leaves the single use intact
        assert_eq!(state.validate("tok"), TokenStatus::Valid);
        assert_eq!(state.validate("tok"), TokenStatus::Valid);
        assert_eq!(state.check_and_use("tok"), TokenStatus::Valid);
        assert_eq!(state.check_and_use("tok"), TokenStatus::Exhausted);
    }

    #[cfg(feature = "ui")]
    #[test]
    fn should_validate_exhausted_and_unknown_tokens() {
        let state = AuthState::new();
        assert_eq!(state.validate("nope"), TokenStatus::Unknown);
        state.add_token("tok", None);
        state.revoke("tok");
        assert_eq!(state.validate("tok"), TokenStatus::Exhausted);
        state.add_token("zero", Some(0));
        assert_eq!(state.validate("zero"), TokenStatus::Exhausted);
    }

    #[cfg(feature = "ui")]
    #[test]
    fn should_extract_token_from_query() {
        assert_eq!(query_token("token=abc"), Some("abc".to_string()));
        assert_eq!(query_token("a=1&token=abc&b=2"), Some("abc".to_string()));
        assert_eq!(query_token("a=1&b=2"), None);
        // "atoken=" is a different parameter
        assert_eq!(query_token("atoken=abc"), None);
        assert_eq!(query_token("token="), Some(String::new()));
    }

    #[cfg(feature = "ui")]
    #[test]
    fn should_percent_decode_query_token() {
        // RFC 6750 b64token charset, as encodeURIComponent emits it
        assert_eq!(
            query_token("token=tok%2Bbase64%2Fchars%3D"),
            Some("tok+base64/chars=".to_string())
        );
        // '+' means space in form encoding
        assert_eq!(query_token("token=a+b"), Some("a b".to_string()));
        // Malformed escapes reject rather than mangle
        assert_eq!(query_token("token=%2"), None);
        assert_eq!(query_token("token=%zz"), None);
        // Non-UTF-8 result rejects
        assert_eq!(query_token("token=%FF"), None);
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
            axum::serve(listener, app).await.ok();
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
