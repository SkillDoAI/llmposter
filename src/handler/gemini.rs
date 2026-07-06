use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, Response, StatusCode};
use axum::response::IntoResponse;

use super::{ProviderHandler, StreamOutput};
use crate::format::gemini;
use crate::format::Provider;
use crate::server::AppState;

/// Build Gemini-style error JSON without needing a full GeminiHandler instance.
fn gemini_error_body(status: u16, message: &str) -> String {
    let status_name = match status {
        400 => "INVALID_ARGUMENT",
        401 => "UNAUTHENTICATED",
        403 => "PERMISSION_DENIED",
        404 => "NOT_FOUND",
        429 => "RESOURCE_EXHAUSTED",
        500 => "INTERNAL",
        503 => "UNAVAILABLE",
        _ => "UNKNOWN",
    };
    serde_json::json!({
        "error": {
            "code": status,
            "message": message,
            "status": status_name
        }
    })
    .to_string()
}

struct GeminiHandler {
    model_from_url: String,
    action: String,
    is_sse: bool,
    /// Pre-formatted `/v1beta/models/{model}:{action}` path. Stored so
    /// `route_label()` can return the *real* incoming URI instead of the
    /// router wildcard pattern (previously the only thing visible to the
    /// request capture API).
    real_path: String,
    /// Query string forwarded upstream in record mode — only `alt` and
    /// `key`, with values percent-encoded so a decoded value can't
    /// smuggle extra params upstream. `None` when neither was sent.
    #[cfg_attr(not(feature = "record"), allow(dead_code))]
    forward_query: Option<String>,
}

/// Percent-encode a URL query component: everything except ASCII
/// alphanumerics and `-_.~` (the RFC 3986 unreserved set) is `%XX`-escaped.
/// Tiny on purpose — not worth a `url` crate dependency.
fn percent_encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

impl ProviderHandler for GeminiHandler {
    fn provider(&self) -> Provider {
        Provider::Gemini
    }
    fn build_error_body(&self, status: u16, message: &str) -> String {
        gemini_error_body(status, message)
    }
    fn route_label(&self) -> &str {
        // Pre-formatted in the axum entry point from the real incoming
        // URL, so captured requests show e.g.
        // `/v1beta/models/gemini-pro:generateContent` instead of the
        // router wildcard.
        &self.real_path
    }
    fn extract_request_info(&self, body: &serde_json::Value) -> Result<(String, String), String> {
        gemini::extract_request_info(body, Some(&self.model_from_url))
    }
    fn is_streaming(&self, _body: &serde_json::Value) -> bool {
        self.action == "streamGenerateContent"
    }
    #[cfg(feature = "record")]
    fn forward_path_and_query(&self) -> (String, Option<String>) {
        // real_path is the pre-formatted `/v1beta/models/{model}:{action}`.
        (self.real_path.clone(), self.forward_query.clone())
    }
    fn default_stop_reason(&self) -> &str {
        "STOP"
    }
    fn build_response(
        &self,
        _state: &AppState,
        _model: &str,
        content: &str,
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> String {
        let mut resp = gemini::build_response(content, prompt);
        // Gemini only overrides finish_reason if explicitly set in fixture
        if has_explicit_reason {
            if let Some(candidate) = resp.candidates.first_mut() {
                candidate.finish_reason = Some(stop_reason.to_string());
            }
        }
        serde_json::to_string(&resp).unwrap()
    }
    fn build_tool_call_response(
        &self,
        _state: &AppState,
        _model: &str,
        tool_calls: &[(&str, serde_json::Value)],
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> String {
        let mut resp = gemini::build_tool_call_response(tool_calls, prompt);
        if has_explicit_reason {
            if let Some(c) = resp.candidates.first_mut() {
                c.finish_reason = Some(stop_reason.to_string());
            }
        }
        serde_json::to_string(&resp).unwrap()
    }
    fn build_refusal_response(
        &self,
        _state: &AppState,
        _model: &str,
        reason: &str,
        prompt: &str,
    ) -> String {
        let resp = gemini::build_refusal_response(reason, prompt);
        serde_json::to_string(&resp).unwrap()
    }
    fn streaming_is_sse(&self) -> bool {
        self.is_sse
    }
    fn build_stream_frames(
        &self,
        _state: &AppState,
        _model: &str,
        content: &str,
        chunk_size: usize,
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> StreamOutput {
        let mut chunks = gemini::build_stream_chunks(content, chunk_size, prompt);
        // Apply finish_reason override to last chunk if explicitly set in fixture.
        // build_stream_chunks always returns ≥1 chunk, each with 1 candidate.
        if has_explicit_reason {
            let last = chunks.last_mut().expect("build_stream_chunks is non-empty");
            last.candidates
                .first_mut()
                .expect("chunk always has 1 candidate")
                .finish_reason = Some(stop_reason.to_string());
        }

        if self.is_sse {
            let frames = chunks
                .iter()
                .map(|c| format!("data: {}\n\n", serde_json::to_string(c).unwrap()))
                .collect();
            StreamOutput::Sse(frames)
        } else {
            let frames = chunks
                .iter()
                .map(|c| serde_json::to_string(c).unwrap())
                .collect();
            StreamOutput::JsonArray(frames)
        }
    }
    fn build_tool_call_stream_frames(
        &self,
        _state: &AppState,
        _model: &str,
        tool_calls: &[(&str, serde_json::Value)],
        _chunk_size: usize,
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> StreamOutput {
        let mut resp = gemini::build_tool_call_response(tool_calls, prompt);
        if has_explicit_reason {
            if let Some(c) = resp.candidates.first_mut() {
                c.finish_reason = Some(stop_reason.to_string());
            }
        }
        let json = serde_json::to_string(&resp).unwrap();

        if self.is_sse {
            StreamOutput::Sse(vec![format!("data: {}\n\n", json)])
        } else {
            StreamOutput::JsonArray(vec![json])
        }
    }
}

/// Axum handler — delegates to the generic request handler with gemini-specific logic.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: axum::http::HeaderMap,
    body: String,
) -> Response<Body> {
    let headers = super::header_map_to_lowercase(&headers);
    // Parse path: e.g. "gemini-pro:generateContent" or "gemini-pro:streamGenerateContent"
    // Helper that stamps the `Provider::Gemini` extension on every
    // response from this entry point, so the `add_response_headers`
    // middleware can identify the provider without string-matching
    // the URI path.
    fn with_provider(mut resp: Response<Body>) -> Response<Body> {
        resp.extensions_mut().insert(Provider::Gemini);
        resp
    }

    let (model, action) = match path.rsplit_once(':') {
        Some((m, a)) => (m.to_string(), a.to_string()),
        None => {
            crate::handler::capture_non_matched(
                &state,
                "POST",
                "/v1beta/models/<invalid>",
                &body,
                crate::server::RequestOutcome::BadRequest,
            );
            return with_provider(
                (
                    StatusCode::BAD_REQUEST,
                    [(header::CONTENT_TYPE, "application/json")],
                    gemini_error_body(400, "Invalid path: expected {model}:{action}"),
                )
                    .into_response(),
            );
        }
    };

    // Reject pathological model segments — real Gemini models only
    // contain ASCII alphanumerics, `.`, `-`, and `_`, AND must have
    // at least one alphanumeric character (so `.` / `..` / `---` are
    // all rejected). Without this check, a request to e.g.
    // `/v1beta/models/../../etc:generateContent` would flow the
    // traversal-ish string into capture logs, fixture matches, and
    // error messages unchanged. The capture log uses a fixed
    // placeholder path on rejection so the raw invalid segment
    // never makes it into `CapturedRequest::path` either.
    let model_bytes_valid = !model.is_empty()
        && model.bytes().any(|b| b.is_ascii_alphanumeric())
        && model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_');
    if !model_bytes_valid {
        crate::handler::capture_non_matched(
            &state,
            "POST",
            "/v1beta/models/<invalid>:<invalid>",
            &body,
            crate::server::RequestOutcome::BadRequest,
        );
        return with_provider(
            (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                gemini_error_body(
                    400,
                    "Invalid model name: must contain at least one ASCII \
                     alphanumeric character and only '.', '-', '_' as \
                     separators",
                ),
            )
                .into_response(),
        );
    }

    if action != "generateContent" && action != "streamGenerateContent" {
        let captured_path = format!("/v1beta/models/{}:{}", model, action);
        crate::handler::capture_non_matched(
            &state,
            "POST",
            &captured_path,
            &body,
            crate::server::RequestOutcome::BadRequest,
        );
        return with_provider(
            (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                gemini_error_body(
                    400,
                    &format!(
                        "Unknown action '{}': expected generateContent or streamGenerateContent",
                        action
                    ),
                ),
            )
                .into_response(),
        );
    }

    let is_sse =
        action == "streamGenerateContent" && query.get("alt").map(|v| v.as_str()) == Some("sse");

    // Record-mode forward query: ONLY `alt` and `key` survive, values
    // percent-encoded. Anything else the client sent stays local.
    let forward_query = {
        let parts: Vec<String> = ["alt", "key"]
            .iter()
            .filter_map(|name| {
                query
                    .get(*name)
                    .map(|v| format!("{}={}", name, percent_encode_component(v)))
            })
            .collect();
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("&"))
        }
    };

    let real_path = format!("/v1beta/models/{}:{}", model, action);
    let handler = GeminiHandler {
        model_from_url: model,
        action,
        is_sse,
        real_path,
        forward_query,
    };

    with_provider(super::handle_request(&handler, state, headers, body).await)
}

#[cfg(test)]
mod gemini_handler_tests {
    use super::*;

    #[test]
    fn should_percent_encode_query_component_so_values_cannot_smuggle_params() {
        // A VALUE of `a&b=c` must round-trip encoded: `key=a%26b%3Dc`,
        // never `key=a&b=c` (which would inject a second param upstream).
        assert_eq!(percent_encode_component("a&b=c"), "a%26b%3Dc");
        assert_eq!(
            format!("key={}", percent_encode_component("a&b=c")),
            "key=a%26b%3Dc"
        );
        // Unreserved characters pass through untouched.
        assert_eq!(percent_encode_component("AZaz09-_.~"), "AZaz09-_.~");
        // Non-ASCII is escaped byte-wise.
        assert_eq!(percent_encode_component("é"), "%C3%A9");
    }
}
