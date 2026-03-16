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
    fn is_streaming(&self, body: &serde_json::Value) -> bool {
        body["stream"].as_bool().unwrap_or(false)
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
        _stop_reason: &str,
        _has_explicit_reason: bool,
    ) -> String {
        let resp = responses::build_response(&state.id_gen, model, content, prompt);
        serde_json::to_string(&resp).unwrap()
    }
    fn build_tool_call_response(
        &self,
        state: &AppState,
        model: &str,
        tool_calls: &[(&str, serde_json::Value)],
        prompt: &str,
        _stop_reason: &str,
        _has_explicit_reason: bool,
    ) -> String {
        let resp = responses::build_tool_call_response(&state.id_gen, model, tool_calls, prompt);
        serde_json::to_string(&resp).unwrap()
    }
    fn build_stream_frames(
        &self,
        state: &AppState,
        model: &str,
        content: &str,
        chunk_size: usize,
        prompt: &str,
        _stop_reason: &str,
        _has_explicit_reason: bool,
    ) -> StreamOutput {
        let events =
            responses::build_stream_events(&state.id_gen, model, content, chunk_size, prompt);
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
        prompt: &str,
        _stop_reason: &str,
        _has_explicit_reason: bool,
    ) -> StreamOutput {
        let resp = responses::build_tool_call_response(&state.id_gen, model, tool_calls, prompt);
        let mut resp_json = serde_json::to_value(&resp).unwrap();
        let mut completed_json = resp_json.clone();
        completed_json["type"] = serde_json::json!("response.completed");
        let completed_str = serde_json::to_string(&completed_json).unwrap();
        resp_json["type"] = serde_json::json!("response.created");
        resp_json["status"] = serde_json::json!("in_progress");
        resp_json["output"] = serde_json::json!([]);
        resp_json["usage"]["output_tokens"] = serde_json::json!(0);
        resp_json["usage"]["total_tokens"] = resp_json["usage"]["input_tokens"].clone();
        let created_str = serde_json::to_string(&resp_json).unwrap();

        let mut frames = vec![format!(
            "event: response.created\ndata: {}\n\n",
            created_str
        )];

        for (i, item) in resp.output.iter().enumerate() {
            // added event
            let mut initial_item = item.clone();
            if let Some(obj) = initial_item.as_object_mut() {
                obj.remove("arguments");
                obj.insert("status".to_string(), serde_json::json!("in_progress"));
            }
            frames.push(format!(
                "event: response.output_item.added\ndata: {}\n\n",
                serde_json::json!({
                    "type": "response.output_item.added",
                    "output_index": i,
                    "item": initial_item,
                })
            ));
            // function_call_arguments.delta
            let args_str = item
                .get("arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let item_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
            frames.push(format!(
                "event: response.function_call_arguments.delta\ndata: {}\n\n",
                serde_json::json!({
                    "type": "response.function_call_arguments.delta",
                    "item_id": item_id,
                    "call_id": call_id,
                    "output_index": i,
                    "delta": args_str,
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
                })
            ));
            // done event
            frames.push(format!(
                "event: response.output_item.done\ndata: {}\n\n",
                serde_json::json!({
                    "type": "response.output_item.done",
                    "output_index": i,
                    "item": item,
                })
            ));
        }
        frames.push(format!(
            "event: response.completed\ndata: {}\n\n",
            completed_str
        ));
        frames.push("event: response.done\ndata: {\"type\":\"response.done\"}\n\n".to_string());

        StreamOutput::Sse(frames)
    }
}

pub async fn handle(State(state): State<Arc<AppState>>, body: String) -> Response<Body> {
    super::handle_request(&ResponsesHandler, state, body).await
}
