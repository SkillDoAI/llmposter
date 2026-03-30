use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::Response;

use super::{ProviderHandler, StreamOutput};
use crate::format::responses;
use crate::format::Provider;
use crate::server::AppState;

struct ResponsesHandler;

impl ProviderHandler for ResponsesHandler {
    fn provider(&self) -> Provider {
        Provider::Responses
    }
    fn route_label(&self) -> &str {
        "/v1/responses"
    }
    fn extract_request_info(&self, body: &serde_json::Value) -> Result<(String, String), String> {
        responses::extract_request_info(body)
    }
    fn default_stop_reason(&self) -> &str {
        "stop"
    }
    fn build_response(
        &self,
        state: &AppState,
        model: &str,
        content: &str,
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> String {
        let mut resp = responses::build_response(&state.id_gen, model, content, prompt);
        // Responses API uses "status" instead of "finish_reason".
        // Map non-default stop_reason to "incomplete" status + emit incomplete_details.
        if has_explicit_reason && stop_reason != "stop" {
            resp.status = "incomplete".to_string();
            resp.incomplete_details = Some(serde_json::json!({"reason": stop_reason}));
        }
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
        let mut resp =
            responses::build_tool_call_response(&state.id_gen, model, tool_calls, prompt);
        if has_explicit_reason && stop_reason != "stop" {
            resp.status = "incomplete".to_string();
            resp.incomplete_details = Some(serde_json::json!({"reason": stop_reason}));
        }
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
        has_explicit_reason: bool,
    ) -> StreamOutput {
        let mut events =
            responses::build_stream_events(&state.id_gen, model, content, chunk_size, prompt);
        // Override status in nested response envelope if stop_reason is explicit
        if has_explicit_reason && stop_reason != "stop" {
            for (_event_type, data) in &mut events {
                if let Some(resp) = data.get_mut("response") {
                    if resp.get("status").and_then(|v| v.as_str()) == Some("completed") {
                        resp["status"] = serde_json::json!("incomplete");
                        resp["incomplete_details"] = serde_json::json!({"reason": stop_reason});
                    }
                }
            }
        }
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
        let mut resp =
            responses::build_tool_call_response(&state.id_gen, model, tool_calls, prompt);
        // Clone for in_progress BEFORE setting incomplete_details — the real API
        // only emits incomplete_details on the final response.completed event.
        let resp_json_for_progress = serde_json::to_value(&resp).unwrap();
        if has_explicit_reason && stop_reason != "stop" {
            resp.status = "incomplete".to_string();
            resp.incomplete_details = Some(serde_json::json!({"reason": stop_reason}));
        }
        let resp_json = serde_json::to_value(&resp).unwrap();
        let mut seq_counter: u64 = 0;

        // Build in_progress envelope (from pre-incomplete snapshot)
        let mut in_progress_resp = resp_json_for_progress;
        in_progress_resp["status"] = serde_json::json!("in_progress");
        in_progress_resp["output"] = serde_json::json!([]);
        in_progress_resp["usage"]["output_tokens"] = serde_json::json!(0);
        in_progress_resp["usage"]["total_tokens"] =
            in_progress_resp["usage"]["input_tokens"].clone();

        let mut frames = Vec::new();

        // response.created — nested envelope
        frames.push(format!(
            "event: response.created\ndata: {}\n\n",
            serde_json::json!({
                "type": "response.created",
                "response": in_progress_resp.clone(),
                "sequence_number": responses::next_seq(&mut seq_counter),
            })
        ));

        // response.in_progress
        frames.push(format!(
            "event: response.in_progress\ndata: {}\n\n",
            serde_json::json!({
                "type": "response.in_progress",
                "response": in_progress_resp,
                "sequence_number": responses::next_seq(&mut seq_counter),
            })
        ));

        for (i, item) in resp.output.iter().enumerate() {
            let item_id = match item.get("id").and_then(|v| v.as_str()) {
                Some(v) => v,
                None => continue, // skip malformed output items
            };
            let call_id = match item.get("call_id").and_then(|v| v.as_str()) {
                Some(v) => v,
                None => continue,
            };
            let args_str = match item.get("arguments").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => continue,
            };

            // output_item.added — keep arguments in the item (don't strip)
            let mut added_item = item.clone();
            if let Some(obj) = added_item.as_object_mut() {
                obj.insert("status".to_string(), serde_json::json!("in_progress"));
            }
            frames.push(format!(
                "event: response.output_item.added\ndata: {}\n\n",
                serde_json::json!({
                    "type": "response.output_item.added",
                    "output_index": i,
                    "item": added_item,
                    "sequence_number": responses::next_seq(&mut seq_counter),
                })
            ));

            // function_call_arguments.delta
            frames.push(format!(
                "event: response.function_call_arguments.delta\ndata: {}\n\n",
                serde_json::json!({
                    "type": "response.function_call_arguments.delta",
                    "item_id": item_id,
                    "call_id": call_id,
                    "output_index": i,
                    "delta": args_str,
                    "sequence_number": responses::next_seq(&mut seq_counter),
                })
            ));

            // function_call_arguments.done
            frames.push(format!(
                "event: response.function_call_arguments.done\ndata: {}\n\n",
                serde_json::json!({
                    "type": "response.function_call_arguments.done",
                    "item_id": item_id,
                    "call_id": call_id,
                    "output_index": i,
                    "arguments": args_str,
                    "sequence_number": responses::next_seq(&mut seq_counter),
                })
            ));

            // output_item.done — full completed item
            frames.push(format!(
                "event: response.output_item.done\ndata: {}\n\n",
                serde_json::json!({
                    "type": "response.output_item.done",
                    "output_index": i,
                    "item": item,
                    "sequence_number": responses::next_seq(&mut seq_counter),
                })
            ));
        }

        // response.completed — nested envelope with final response
        frames.push(format!(
            "event: response.completed\ndata: {}\n\n",
            serde_json::json!({
                "type": "response.completed",
                "response": resp_json,
                "sequence_number": responses::next_seq(&mut seq_counter),
            })
        ));

        StreamOutput::Sse(frames)
    }
}

pub async fn handle(State(state): State<Arc<AppState>>, body: String) -> Response<Body> {
    super::handle_request(&ResponsesHandler, state, body).await
}
