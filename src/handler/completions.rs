//! Legacy text completions handler (`POST /v1/completions`).

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::Response;

use crate::format::completions;
use crate::format::Provider;
use crate::server::AppState;

use super::{ProviderHandler, StreamOutput};

struct CompletionsHandler;

impl ProviderHandler for CompletionsHandler {
    fn provider(&self) -> Provider {
        Provider::OpenAI
    }

    fn route_label(&self) -> &str {
        "/v1/completions"
    }

    fn extract_request_info(&self, body: &serde_json::Value) -> Result<(String, String), String> {
        completions::extract_request_info(body)
    }

    fn default_stop_reason(&self) -> &str {
        "stop"
    }

    fn build_response(
        &self,
        state: &AppState,
        model: &str,
        content: &str,
        _prompt: &str,
        stop_reason: &str,
        _has_explicit_reason: bool,
    ) -> String {
        let resp = completions::build_response(&state.id_gen, model, content, _prompt, stop_reason);
        serde_json::to_string(&resp).unwrap_or_else(|_| "{}".to_string())
    }

    fn build_tool_call_response(
        &self,
        state: &AppState,
        model: &str,
        _tool_calls: &[(&str, serde_json::Value)],
        prompt: &str,
        stop_reason: &str,
        _has_explicit_reason: bool,
    ) -> String {
        // Legacy completions API has no tool calls — return empty text.
        self.build_response(state, model, "", prompt, stop_reason, false)
    }

    fn build_refusal_response(
        &self,
        state: &AppState,
        model: &str,
        reason: &str,
        prompt: &str,
    ) -> String {
        // Return the refusal text as plain completion content.
        self.build_response(state, model, reason, prompt, "stop", false)
    }

    fn build_stream_frames(
        &self,
        state: &AppState,
        model: &str,
        content: &str,
        chunk_size: usize,
        _prompt: &str,
        _stop_reason: &str,
        _has_explicit_reason: bool,
    ) -> StreamOutput {
        let id = state.id_gen.next_completions();
        let chunks = completions::build_stream_chunks(&id, model, content, chunk_size);
        let mut frames: Vec<String> = chunks
            .iter()
            .map(|c| {
                format!(
                    "data: {}\n\n",
                    serde_json::to_string(c).unwrap_or_else(|_| "{}".to_string())
                )
            })
            .collect();
        frames.push("data: [DONE]\n\n".to_string());
        StreamOutput::Sse(frames)
    }

    fn build_tool_call_stream_frames(
        &self,
        state: &AppState,
        model: &str,
        _tool_calls: &[(&str, serde_json::Value)],
        _chunk_size: usize,
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> StreamOutput {
        // Legacy completions API has no tool calls — stream empty text.
        self.build_stream_frames(
            state,
            model,
            "",
            20,
            prompt,
            stop_reason,
            has_explicit_reason,
        )
    }
}

pub async fn handle(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: String,
) -> Response<Body> {
    let headers = super::header_map_to_lowercase(&headers);
    let mut resp = super::handle_request(&CompletionsHandler, state, headers, body).await;
    resp.extensions_mut().insert(Provider::OpenAI);
    resp
}
