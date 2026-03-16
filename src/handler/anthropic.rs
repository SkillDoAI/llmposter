use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Response, StatusCode};
use axum::response::IntoResponse;
use tokio::time::sleep;

use crate::failure;
use crate::fixture::match_fixture;
use crate::format::anthropic;
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

    let (model, user_message) = match anthropic::extract_request_info(&json_body) {
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
        Some(Provider::Anthropic),
    ) {
        Some(f) => f,
        None => {
            if state.verbose {
                eprintln!(
                    "[llmposter] POST /v1/messages → no match (model='{}', msg='{:.50}')",
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
        eprintln!("[llmposter] POST /v1/messages → fixture matched");
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
    // Support both stop_reason (Anthropic term) and finish_reason (OpenAI term) as aliases
    let stop_reason = response
        .stop_reason
        .as_deref()
        .or(response.finish_reason.as_deref())
        .unwrap_or("end_turn");

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

        // For tool calls in streaming, emit proper Anthropic streaming events
        if let Some(ref tool_calls) = response.tool_calls {
            let msg_id = state.id_gen.next_anthropic();
            let input_tokens = crate::format::estimate_tokens(&user_message);
            let mut output_tokens: u64 = 0;
            let mut frames: Vec<String> = Vec::new();

            // message_start with empty content and null stop_reason
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
                        "usage": {"input_tokens": input_tokens, "output_tokens": 0}
                    }
                })
            ));

            // content_block_start + content_block_delta + content_block_stop for each tool_use
            for (i, tc) in tool_calls.iter().enumerate() {
                let tool_id = format!("toolu_llmposter_{}", i + 1);
                let args_json = &tc.arguments;
                output_tokens += crate::format::estimate_tokens(
                    &serde_json::to_string(args_json).unwrap_or_default(),
                );

                frames.push(format!(
                    "event: content_block_start\ndata: {}\n\n",
                    serde_json::json!({
                        "type": "content_block_start",
                        "index": i,
                        "content_block": {
                            "type": "tool_use",
                            "id": tool_id,
                            "name": tc.name,
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
                            "partial_json": serde_json::to_string(args_json).unwrap_or_default()
                        }
                    })
                ));
                frames.push(format!(
                    "event: content_block_stop\ndata: {}\n\n",
                    serde_json::json!({"type": "content_block_stop", "index": i})
                ));
            }

            // message_delta with stop_reason (default "tool_use" for tool calls)
            let tc_stop = if response.stop_reason.is_some() || response.finish_reason.is_some() {
                stop_reason
            } else {
                "tool_use"
            };
            frames.push(format!(
                "event: message_delta\ndata: {}\n\n",
                serde_json::json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": tc_stop, "stop_sequence": null},
                    "usage": {"output_tokens": output_tokens}
                })
            ));

            // message_stop
            frames.push(format!(
                "event: message_stop\ndata: {}\n\n",
                serde_json::json!({"type": "message_stop"})
            ));

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
            let body = Body::from_stream(stream);

            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .header(header::CACHE_CONTROL, "no-cache")
                .header(header::CONNECTION, "keep-alive")
                .body(body)
                .unwrap();
        }

        let events = anthropic::build_stream_events(
            &state.id_gen,
            &model,
            content,
            chunk_size,
            &user_message,
            stop_reason,
        );

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(32);

        tokio::spawn(async move {
            let start = std::time::Instant::now();

            for (sent, (event_type, data)) in events.iter().enumerate() {
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

                let frame = format!(
                    "event: {}\ndata: {}\n\n",
                    event_type,
                    serde_json::to_string(data).unwrap()
                );
                if tx.send(Ok(frame)).await.is_err() {
                    return;
                }

                if latency > 0 {
                    sleep(Duration::from_millis(latency)).await;
                }
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let body = Body::from_stream(stream);

        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive")
            .body(body)
            .unwrap()
    } else {
        // Handle tool calls
        if let Some(ref tool_calls) = response.tool_calls {
            let tc_pairs: Vec<(&str, serde_json::Value)> = tool_calls
                .iter()
                .map(|tc| (tc.name.as_str(), tc.arguments.clone()))
                .collect();
            let mut resp =
                anthropic::build_tool_use_response(&state.id_gen, &model, &tc_pairs, &user_message);
            // Only override if fixture explicitly sets stop_reason or finish_reason
            if response.stop_reason.is_some() || response.finish_reason.is_some() {
                resp.stop_reason = Some(stop_reason.to_string());
            }
            let json = serde_json::to_string(&resp).unwrap();
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                json,
            )
                .into_response();
        }

        let resp =
            anthropic::build_response(&state.id_gen, &model, content, &user_message, stop_reason);
        let json = serde_json::to_string(&resp).unwrap();
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            json,
        )
            .into_response()
    }
}
