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
        // Apply finish_reason override to last chunk if explicitly set in fixture
        if has_explicit_reason {
            if let Some(last) = chunks.last_mut() {
                if let Some(candidate) = last.candidates.first_mut() {
                    candidate.finish_reason = Some(stop_reason.to_string());
                }
            }
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
            let captured_path = format!("/v1beta/models/{}", path);
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
                    gemini_error_body(400, "Invalid path: expected {model}:{action}"),
                )
                    .into_response(),
            );
        }
    };

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

    let real_path = format!("/v1beta/models/{}:{}", model, action);
    let handler = GeminiHandler {
        model_from_url: model,
        action,
        is_sse,
        real_path,
    };

    with_provider(super::handle_request(&handler, state, headers, body).await)
}
