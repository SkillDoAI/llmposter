use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::Response;

use super::{ProviderHandler, StreamOutput};
use crate::format::anthropic;
use crate::format::Provider;
use crate::server::AppState;

struct AnthropicHandler;

impl ProviderHandler for AnthropicHandler {
    fn provider(&self) -> Provider {
        Provider::Anthropic
    }
    fn route_label(&self) -> &str {
        "/v1/messages"
    }
    fn build_error_body(&self, status: u16, message: &str) -> String {
        // Anthropic uses {"type": "error", "error": {"type": "<error_type>", "message": "<msg>"}}
        let error_type = match status {
            400 => "invalid_request_error",
            401 => "authentication_error",
            403 => "permission_error",
            404 => "not_found_error",
            429 => "rate_limit_error",
            500 | 502 | 503 => "api_error",
            529 => "overloaded_error",
            _ => "api_error",
        };
        serde_json::json!({
            "type": "error",
            "error": {
                "type": error_type,
                "message": message
            }
        })
        .to_string()
    }
    fn extract_request_info(&self, body: &serde_json::Value) -> Result<(String, String), String> {
        anthropic::extract_request_info(body)
    }
    fn default_stop_reason(&self) -> &str {
        "end_turn"
    }
    fn build_response(
        &self,
        state: &AppState,
        model: &str,
        content: &str,
        prompt: &str,
        stop_reason: &str,
        _has_explicit_reason: bool,
    ) -> String {
        let resp = anthropic::build_response(&state.id_gen, model, content, prompt, stop_reason);
        serde_json::to_string(&resp).unwrap()
    }
    fn build_tool_call_response(
        &self,
        state: &AppState,
        model: &str,
        tool_calls: &[(&str, serde_json::Value)],
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> String {
        let mut resp = anthropic::build_tool_use_response(&state.id_gen, model, tool_calls, prompt);
        if has_explicit_reason {
            resp.stop_reason = Some(stop_reason.to_string());
        }
        serde_json::to_string(&resp).unwrap()
    }
    fn build_refusal_response(
        &self,
        state: &AppState,
        model: &str,
        reason: &str,
        prompt: &str,
    ) -> String {
        let resp = anthropic::build_refusal_response(&state.id_gen, model, reason, prompt);
        serde_json::to_string(&resp).unwrap()
    }
    fn build_stream_frames(
        &self,
        state: &AppState,
        model: &str,
        content: &str,
        chunk_size: usize,
        prompt: &str,
        stop_reason: &str,
        _has_explicit_reason: bool,
    ) -> StreamOutput {
        let events = anthropic::build_stream_events(
            &state.id_gen,
            model,
            content,
            chunk_size,
            prompt,
            stop_reason,
        );
        let frames = events
            .iter()
            .map(|(event_type, data)| {
                format!(
                    "event: {}\ndata: {}\n\n",
                    event_type,
                    serde_json::to_string(data).unwrap()
                )
            })
            .collect();
        StreamOutput::Sse(frames)
    }
    fn build_tool_call_stream_frames(
        &self,
        state: &AppState,
        model: &str,
        tool_calls: &[(&str, serde_json::Value)],
        _chunk_size: usize,
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> StreamOutput {
        let msg_id = state.id_gen.next_anthropic();
        let input_tokens = crate::format::estimate_tokens(prompt);
        let mut output_tokens: u64 = 0;
        let mut frames: Vec<String> = Vec::new();

        // ping
        frames.push("event: ping\ndata: {\"type\":\"ping\"}\n\n".to_string());

        // message_start
        frames.push(format!(
            "event: message_start\ndata: {}\n\n",
            serde_json::json!({
                "type": "message_start",
                "message": {
                    "id": msg_id,
                    "type": "message",
                    "role": "assistant",
                    "model": model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": {
                        "input_tokens": input_tokens,
                        "output_tokens": 0,
                        "cache_creation_input_tokens": 0,
                        "cache_read_input_tokens": 0
                    }
                }
            })
        ));

        // content_block_start + delta + stop for each tool_use
        for (i, (name, args)) in tool_calls.iter().enumerate() {
            let tool_id = format!("toolu_llmposter_{}", state.id_gen.next_tool_call_counter());
            let args_str = serde_json::to_string(args).unwrap_or_default();
            output_tokens += crate::format::estimate_tokens(&args_str);

            frames.push(format!(
                "event: content_block_start\ndata: {}\n\n",
                serde_json::json!({
                    "type": "content_block_start",
                    "index": i,
                    "content_block": {
                        "type": "tool_use",
                        "id": tool_id,
                        "name": name,
                        "input": {}
                    }
                })
            ));
            frames.push(format!(
                "event: content_block_delta\ndata: {}\n\n",
                serde_json::json!({
                    "type": "content_block_delta",
                    "index": i,
                    "delta": {
                        "type": "input_json_delta",
                        "partial_json": args_str
                    }
                })
            ));
            frames.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                serde_json::json!({"type": "content_block_stop", "index": i})
            ));
        }

        // message_delta
        let tc_stop = if has_explicit_reason {
            stop_reason
        } else {
            "tool_use"
        };
        frames.push(format!(
            "event: message_delta\ndata: {}\n\n",
            serde_json::json!({
                "type": "message_delta",
                "delta": {"stop_reason": tc_stop, "stop_sequence": null},
                "usage": {
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0
                }
            })
        ));

        // message_stop
        frames.push(format!(
            "event: message_stop\ndata: {}\n\n",
            serde_json::json!({"type": "message_stop"})
        ));

        StreamOutput::Sse(frames)
    }
}

/// Axum handler — delegates to the generic request handler with anthropic-specific logic.
pub async fn handle(State(state): State<Arc<AppState>>, body: String) -> Response<Body> {
    let mut resp = super::handle_request(&AnthropicHandler, state, body).await;
    resp.extensions_mut().insert(Provider::Anthropic);
    resp
}
