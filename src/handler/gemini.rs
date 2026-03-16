use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, Response, StatusCode};
use axum::response::IntoResponse;

use super::{ProviderHandler, StreamOutput};
use crate::failure;
use crate::format::gemini;
use crate::format::Provider;
use crate::server::AppState;

struct GeminiHandler {
    model_from_url: String,
    action: String,
    is_sse: bool,
}

impl ProviderHandler for GeminiHandler {
    fn provider(&self) -> Provider {
        Provider::Gemini
    }
    fn build_error_body(&self, status: u16, message: &str) -> String {
        // Gemini uses {"error": {"code": N, "message": "...", "status": "STATUS_NAME"}}
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
    fn route_label(&self) -> &str {
        // We can't dynamically format here, but the verbose logging in
        // handle_request uses this. Gemini's axum handler does its own
        // verbose logging for the model:action path, but the generic
        // handler's no-match / matched messages use this label.
        "/v1beta/models/*"
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
    ) -> String {
        let mut resp = gemini::build_response(content, prompt);
        // Gemini only overrides finish_reason if explicitly set in fixture
        if stop_reason != "STOP" {
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
    fn build_stream_frames(
        &self,
        _state: &AppState,
        _model: &str,
        content: &str,
        chunk_size: usize,
        prompt: &str,
        stop_reason: &str,
    ) -> StreamOutput {
        let mut chunks = gemini::build_stream_chunks(content, chunk_size, prompt);
        // Apply finish_reason override to last chunk if explicitly set
        if stop_reason != "STOP" {
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

pub async fn handle(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    body: String,
) -> Response<Body> {
    // Parse path: e.g. "gemini-pro:generateContent" or "gemini-pro:streamGenerateContent"
    let (model, action) = match path.rsplit_once(':') {
        Some((m, a)) => (m.to_string(), a.to_string()),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                failure::build_error_body(400, "Invalid path: expected {model}:{action}"),
            )
                .into_response();
        }
    };

    if action != "generateContent" && action != "streamGenerateContent" {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            failure::build_error_body(
                400,
                &format!(
                    "Unknown action '{}': expected generateContent or streamGenerateContent",
                    action
                ),
            ),
        )
            .into_response();
    }

    let is_sse =
        action == "streamGenerateContent" && query.get("alt").map(|v| v.as_str()) == Some("sse");

    let handler = GeminiHandler {
        model_from_url: model,
        action,
        is_sse,
    };

    super::handle_request(&handler, state, body).await
}
