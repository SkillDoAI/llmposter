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
        _has_explicit_reason: bool,
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
    fn build_refusal_response(
        &self,
        state: &AppState,
        model: &str,
        reason: &str,
        prompt: &str,
    ) -> String {
        let resp = openai::build_refusal_response(&state.id_gen, model, reason, prompt);
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
        _chunk_size: usize,
        _prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> StreamOutput {
        let id = state.id_gen.next_openai();
        let created = openai::unix_timestamp();
        let fingerprint = Some(openai::SYSTEM_FINGERPRINT.to_string());
        let tc_outputs: Vec<openai::ToolCallOutput> = tool_calls
            .iter()
            .enumerate()
            .map(|(i, (name, args))| openai::ToolCallOutput {
                index: Some(i as u32),
                id: format!("call_llmposter_{}", state.id_gen.next_tool_call_counter()),
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
                created,
                model: model.to_string(),
                system_fingerprint: fingerprint.clone(),
                service_tier: Some("default".to_string()),
                choices: vec![openai::ChunkChoice {
                    index: 0,
                    delta: openai::Delta {
                        role: Some("assistant".to_string()),
                        content: None,
                        tool_calls: None,
                        refusal: None,
                    },
                    finish_reason: None,
                    logprobs: None,
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
                created,
                model: model.to_string(),
                system_fingerprint: fingerprint.clone(),
                service_tier: None,
                choices: vec![openai::ChunkChoice {
                    index: 0,
                    delta: openai::Delta {
                        role: None,
                        content: None,
                        tool_calls: Some(tc_outputs),
                        refusal: None,
                    },
                    finish_reason: None,
                    logprobs: None,
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
                created,
                model: model.to_string(),
                system_fingerprint: fingerprint,
                service_tier: None,
                choices: vec![openai::ChunkChoice {
                    index: 0,
                    delta: openai::Delta {
                        role: None,
                        content: None,
                        tool_calls: None,
                        refusal: None,
                    },
                    finish_reason: Some(if has_explicit_reason {
                        stop_reason.to_string()
                    } else {
                        "tool_calls".to_string()
                    }),
                    logprobs: None,
                }],
            })
            .unwrap()
        ));
        frames.push("data: [DONE]\n\n".to_string());
        StreamOutput::Sse(frames)
    }
}

/// Axum handler — delegates to the generic request handler with openai-specific logic.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: String,
) -> Response<Body> {
    let headers = super::header_map_to_lowercase(&headers);
    let mut resp = super::handle_request(&OpenAIHandler, state, headers, body).await;
    resp.extensions_mut().insert(Provider::OpenAI);
    resp
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::http::StatusCode;

    use crate::fixture::{Fixture, FixtureError};
    use crate::format::IdGenerator;
    use crate::server::AppState;

    /// Exercises the graceful Err fallback in handle_request when a fixture has an
    /// invalid HTTP header name (bypassing validate() by constructing directly).
    #[tokio::test]
    async fn should_return_500_for_invalid_header_name_in_error_fixture() {
        let fixture = Fixture {
            error: Some(FixtureError {
                status: 429,
                message: "rate limit".to_string(),
                headers: HashMap::from([("invalid header name!".to_string(), "v".to_string())]),
            }),
            ..Fixture::new()
        };
        let state = Arc::new(AppState {
            fixtures: std::sync::RwLock::new(crate::server::FixtureSet::new(vec![Arc::new(
                fixture,
            )])),
            id_gen: IdGenerator::new(),
            verbose: false,
            request_counter: Default::default(),
            chaos_counter: Default::default(),
            auth: None,
            scenarios: Default::default(),
            captured_requests: Default::default(),
            capture_capacity: None,
            boot_instant: std::time::Instant::now(),
            boot_epoch_ms: 0,
            #[cfg(feature = "ui")]
            ui_tx: None,
        });
        let body = r#"{"model":"gpt-4","messages":[{"role":"user","content":"hi"}]}"#;
        let resp = super::super::handle_request(
            &super::OpenAIHandler,
            state,
            std::collections::HashMap::new(),
            body.to_string(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Exercises the "Fixture has neither response nor error" fallback when a fixture
    /// matches but has no response or error set (bypassing validate()).
    #[tokio::test]
    async fn should_return_500_for_fixture_without_response_or_error() {
        let fixture = Fixture::new(); // no response, no error — just a catch-all match
        let state = Arc::new(AppState {
            fixtures: std::sync::RwLock::new(crate::server::FixtureSet::new(vec![Arc::new(
                fixture,
            )])),
            id_gen: IdGenerator::new(),
            verbose: false,
            request_counter: Default::default(),
            chaos_counter: Default::default(),
            auth: None,
            scenarios: Default::default(),
            captured_requests: Default::default(),
            capture_capacity: None,
            boot_instant: std::time::Instant::now(),
            boot_epoch_ms: 0,
            #[cfg(feature = "ui")]
            ui_tx: None,
        });
        let body = r#"{"model":"gpt-4","messages":[{"role":"user","content":"hi"}]}"#;
        let resp = super::super::handle_request(
            &super::OpenAIHandler,
            state,
            std::collections::HashMap::new(),
            body.to_string(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
