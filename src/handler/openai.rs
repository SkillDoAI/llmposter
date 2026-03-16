use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::Response;

use super::{ProviderHandler, StreamOutput};
use crate::format::openai;
use crate::format::Provider;
use crate::server::AppState;

struct OpenAIHandler;

impl ProviderHandler for OpenAIHandler {
    fn provider(&self) -> Provider {
        Provider::OpenAI
    }
    fn route_label(&self) -> &str {
        "/v1/chat/completions"
    }
    fn extract_request_info(&self, body: &serde_json::Value) -> Result<(String, String), String> {
        openai::extract_request_info(body)
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
        stop_reason: &str,
    ) -> String {
        let mut resp = openai::build_response(&state.id_gen, model, content, prompt);
        if let Some(choice) = resp.choices.first_mut() {
            choice.finish_reason = stop_reason.to_string();
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
        let mut resp = openai::build_tool_call_response(&state.id_gen, model, tool_calls, prompt);
        if has_explicit_reason {
            if let Some(choice) = resp.choices.first_mut() {
                choice.finish_reason = stop_reason.to_string();
            }
        }
        serde_json::to_string(&resp).unwrap()
    }
    fn build_stream_frames(
        &self,
        state: &AppState,
        model: &str,
        content: &str,
        chunk_size: usize,
        _prompt: &str,
        stop_reason: &str,
        _has_explicit_reason: bool,
    ) -> StreamOutput {
        let id = state.id_gen.next_openai();
        let mut chunks = openai::build_stream_chunks(&id, model, content, chunk_size);
        if let Some(last) = chunks.last_mut() {
            if let Some(choice) = last.choices.first_mut() {
                if choice.finish_reason.is_some() {
                    choice.finish_reason = Some(stop_reason.to_string());
                }
            }
        }
        let mut frames: Vec<String> = chunks
            .iter()
            .map(|c| format!("data: {}\n\n", serde_json::to_string(c).unwrap()))
            .collect();
        frames.push("data: [DONE]\n\n".to_string());
        StreamOutput::Sse(frames)
    }
    fn build_tool_call_stream_frames(
        &self,
        state: &AppState,
        model: &str,
        tool_calls: &[(&str, serde_json::Value)],
        _prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> StreamOutput {
        let id = state.id_gen.next_openai();
        let tc_outputs: Vec<openai::ToolCallOutput> = tool_calls
            .iter()
            .enumerate()
            .map(|(i, (name, args))| openai::ToolCallOutput {
                index: Some(i as u32),
                id: format!("call_llmposter_{}", i + 1),
                call_type: "function".to_string(),
                function: openai::FunctionCall {
                    name: name.to_string(),
                    arguments: serde_json::to_string(args).unwrap_or_default(),
                },
            })
            .collect();

        let mut frames: Vec<String> = Vec::new();
        // First chunk: role only
        frames.push(format!(
            "data: {}\n\n",
            serde_json::to_string(&openai::ChatCompletionChunk {
                id: id.clone(),
                object: "chat.completion.chunk".to_string(),
                model: model.to_string(),
                choices: vec![openai::ChunkChoice {
                    index: 0,
                    delta: openai::Delta {
                        role: Some("assistant".to_string()),
                        content: None,
                        tool_calls: None,
                    },
                    finish_reason: None,
                }],
            })
            .unwrap()
        ));
        // Second chunk: tool_calls
        frames.push(format!(
            "data: {}\n\n",
            serde_json::to_string(&openai::ChatCompletionChunk {
                id: id.clone(),
                object: "chat.completion.chunk".to_string(),
                model: model.to_string(),
                choices: vec![openai::ChunkChoice {
                    index: 0,
                    delta: openai::Delta {
                        role: None,
                        content: None,
                        tool_calls: Some(tc_outputs),
                    },
                    finish_reason: None,
                }],
            })
            .unwrap()
        ));
        // Third chunk: finish_reason
        frames.push(format!(
            "data: {}\n\n",
            serde_json::to_string(&openai::ChatCompletionChunk {
                id,
                object: "chat.completion.chunk".to_string(),
                model: model.to_string(),
                choices: vec![openai::ChunkChoice {
                    index: 0,
                    delta: openai::Delta {
                        role: None,
                        content: None,
                        tool_calls: None,
                    },
                    finish_reason: Some(if has_explicit_reason {
                        stop_reason.to_string()
                    } else {
                        "tool_calls".to_string()
                    }),
                }],
            })
            .unwrap()
        ));
        frames.push("data: [DONE]\n\n".to_string());
        StreamOutput::Sse(frames)
    }
}

pub async fn handle(State(state): State<Arc<AppState>>, body: String) -> Response<Body> {
    super::handle_request(&OpenAIHandler, state, body).await
}
