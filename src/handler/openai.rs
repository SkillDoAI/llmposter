use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Response, StatusCode};
use axum::response::IntoResponse;
use tokio::time::sleep;

use crate::failure;
use crate::fixture::match_fixture;
use crate::format::openai;
use crate::format::Provider;
use crate::server::AppState;

pub async fn handle(State(state): State<Arc<AppState>>, body: String) -> Response<Body> {
    let json_body: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                failure::build_error_body(400, "Invalid JSON in request body"),
            )
                .into_response();
        }
    };

    let (model, user_message) = match openai::extract_request_info(&json_body) {
        Ok(info) => info,
        Err(msg) => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                failure::build_error_body(400, &msg),
            )
                .into_response();
        }
    };

    let is_streaming = json_body["stream"].as_bool().unwrap_or(false);

    let fixture = match match_fixture(
        &state.fixtures,
        &user_message,
        Some(&model),
        Some(Provider::OpenAI),
    ) {
        Some(f) => f,
        None => {
            if state.verbose {
                eprintln!(
                    "[llmposter] POST /v1/chat/completions → no match (model='{}', msg='{:.50}')",
                    model, user_message
                );
            }
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "application/json")],
                failure::build_no_match_body(&model, &user_message),
            )
                .into_response();
        }
    };

    if state.verbose {
        eprintln!("[llmposter] POST /v1/chat/completions → fixture matched");
    }

    // Handle error fixtures
    if let Some(ref err) = fixture.error {
        let status = StatusCode::from_u16(err.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        return (
            status,
            [(header::CONTENT_TYPE, "application/json")],
            failure::build_error_body(status.as_u16(), &err.message),
        )
            .into_response();
    }

    let response = match fixture.response.as_ref() {
        Some(r) => r,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                failure::build_error_body(500, "Fixture has neither response nor error"),
            )
                .into_response();
        }
    };
    let content = response.content.as_deref().unwrap_or("");
    // Support both finish_reason (OpenAI term) and stop_reason (Anthropic term) as aliases
    let finish_reason = response
        .finish_reason
        .as_deref()
        .or(response.stop_reason.as_deref())
        .unwrap_or("stop");

    // Handle failure: latency
    if let Some(ref fail) = fixture.failure {
        if let Some(ms) = fail.latency_ms {
            sleep(Duration::from_millis(ms)).await;
        }

        // Handle failure: corrupt body
        if fail.corrupt_body == Some(true) {
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/plain")],
                "overloaded".to_string(),
            )
                .into_response();
        }
    }

    if is_streaming {
        let chunk_size = fixture
            .streaming
            .as_ref()
            .and_then(|s| s.chunk_size)
            .unwrap_or(20);
        let latency = fixture
            .streaming
            .as_ref()
            .and_then(|s| s.latency)
            .unwrap_or(0);
        let truncate_after = fixture
            .failure
            .as_ref()
            .and_then(|f| f.truncate_after_chunks);
        let disconnect_after_ms = fixture.failure.as_ref().and_then(|f| f.disconnect_after_ms);

        // For tool calls in streaming, use ChatCompletionChunk format with delta.tool_calls
        // Uses mpsc channel for proper truncation/disconnect support
        if let Some(ref tool_calls) = response.tool_calls {
            let id = state.id_gen.next_openai();
            let tc_outputs: Vec<openai::ToolCallOutput> = tool_calls
                .iter()
                .enumerate()
                .map(|(i, tc)| openai::ToolCallOutput {
                    index: Some(i as u32), // Streaming: index is required
                    id: format!("call_llmposter_{}", i + 1),
                    call_type: "function".to_string(),
                    function: openai::FunctionCall {
                        name: tc.name.clone(),
                        arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                    },
                })
                .collect();

            // Build the tool-call chunks as SSE frames
            let mut frames: Vec<String> = Vec::new();
            // First chunk: role only (per OpenAI streaming protocol)
            frames.push(format!(
                "data: {}\n\n",
                serde_json::to_string(&openai::ChatCompletionChunk {
                    id: id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    model: model.clone(),
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
                    model: model.clone(),
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
            frames.push(format!(
                "data: {}\n\n",
                serde_json::to_string(&openai::ChatCompletionChunk {
                    id,
                    object: "chat.completion.chunk".to_string(),
                    model: model.clone(),
                    choices: vec![openai::ChunkChoice {
                        index: 0,
                        delta: openai::Delta {
                            role: None,
                            content: None,
                            tool_calls: None,
                        },
                        finish_reason: Some(
                            if response.finish_reason.is_some() || response.stop_reason.is_some() {
                                finish_reason.to_string()
                            } else {
                                "tool_calls".to_string()
                            }
                        ),
                    }],
                })
                .unwrap()
            ));
            frames.push("data: [DONE]\n\n".to_string());

            let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(32);

            tokio::spawn(async move {
                let start = std::time::Instant::now();

                for (sent, frame) in frames.into_iter().enumerate() {
                    tokio::task::yield_now().await;

                    if let Some(ms) = disconnect_after_ms {
                        if start.elapsed() >= Duration::from_millis(ms) {
                            return;
                        }
                    }
                    if let Some(max) = truncate_after {
                        if sent as u32 >= max {
                            return;
                        }
                    }

                    if tx.send(Ok(frame)).await.is_err() {
                        return;
                    }

                    if latency > 0 {
                        sleep(Duration::from_millis(latency)).await;
                    }
                }
            });

            let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .header(header::CACHE_CONTROL, "no-cache")
                .header(header::CONNECTION, "keep-alive")
                .body(Body::from_stream(stream))
                .expect("static SSE response headers");
        }

        let id = state.id_gen.next_openai();
        let mut chunks = openai::build_stream_chunks(&id, &model, content, chunk_size);
        // Apply finish_reason override to the final chunk
        if let Some(last) = chunks.last_mut() {
            if let Some(choice) = last.choices.first_mut() {
                if choice.finish_reason.is_some() {
                    choice.finish_reason = Some(finish_reason.to_string());
                }
            }
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(32);

        tokio::spawn(async move {
            let start = std::time::Instant::now();

            for (sent, chunk) in chunks.iter().enumerate() {
                // Yield to allow elapsed time to advance for disconnect checks
                tokio::task::yield_now().await;

                if let Some(ms) = disconnect_after_ms {
                    if start.elapsed() >= Duration::from_millis(ms) {
                        return;
                    }
                }

                if let Some(max) = truncate_after {
                    if sent as u32 >= max {
                        return;
                    }
                }

                let data = format!("data: {}\n\n", serde_json::to_string(chunk).unwrap());
                if tx.send(Ok(data)).await.is_err() {
                    return;
                }

                if latency > 0 {
                    sleep(Duration::from_millis(latency)).await;
                }
            }

            let _ = tx.send(Ok("data: [DONE]\n\n".to_string())).await;
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let body = Body::from_stream(stream);

        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive")
            .body(body)
            .expect("static SSE response headers")
    } else {
        // Handle tool calls
        if let Some(ref tool_calls) = response.tool_calls {
            let tc_pairs: Vec<(&str, serde_json::Value)> = tool_calls
                .iter()
                .map(|tc| (tc.name.as_str(), tc.arguments.clone()))
                .collect();
            let mut resp =
                openai::build_tool_call_response(&state.id_gen, &model, &tc_pairs, &user_message);
            // Only override if fixture explicitly sets finish_reason/stop_reason
            if response.finish_reason.is_some() || response.stop_reason.is_some() {
                if let Some(choice) = resp.choices.first_mut() {
                    choice.finish_reason = finish_reason.to_string();
                }
            }
            let json = serde_json::to_string(&resp).unwrap();
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                json,
            )
                .into_response();
        }

        let mut resp = openai::build_response(&state.id_gen, &model, content, &user_message);
        if let Some(choice) = resp.choices.first_mut() {
            choice.finish_reason = finish_reason.to_string();
        }
        let json = serde_json::to_string(&resp).unwrap();
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            json,
        )
            .into_response()
    }
}
