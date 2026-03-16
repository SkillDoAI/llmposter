use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, Response, StatusCode};
use axum::response::IntoResponse;
use tokio::time::sleep;

use crate::failure;
use crate::fixture::match_fixture;
use crate::format::gemini;
use crate::format::Provider;
use crate::server::AppState;

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

    let is_streaming = action == "streamGenerateContent";
    let is_sse = is_streaming && query.get("alt").map(|v| v.as_str()) == Some("sse");

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

    let (model, user_message) = match gemini::extract_request_info(&json_body, Some(&model)) {
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

    let fixture = match match_fixture(
        &state.fixtures,
        &user_message,
        Some(&model),
        Some(Provider::Gemini),
    ) {
        Some(f) => f,
        None => {
            if state.verbose {
                eprintln!(
                    "[llmposter] POST /v1beta/models/{}:{} → no match (model='{}', msg='{:.50}')",
                    model, action, model, user_message
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
        eprintln!(
            "[llmposter] POST /v1beta/models/{}:{} → fixture matched",
            model, action
        );
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

        // For tool calls in streaming, use mpsc channel with failure simulation
        if let Some(ref tool_calls) = response.tool_calls {
            let tc_pairs: Vec<(&str, serde_json::Value)> = tool_calls
                .iter()
                .map(|tc| (tc.name.as_str(), tc.arguments.clone()))
                .collect();
            let mut resp = gemini::build_tool_call_response(&tc_pairs, &user_message);
            if let Some(reason) = response
                .finish_reason
                .as_deref()
                .or(response.stop_reason.as_deref())
            {
                if let Some(c) = resp.candidates.first_mut() {
                    c.finish_reason = Some(reason.to_string());
                }
            }
            let json = serde_json::to_string(&resp).unwrap();

            if is_sse {
                let frames = vec![format!("data: {}\n\n", json)];
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
                    .unwrap();
            } else {
                // JSON array — single element, apply truncation and disconnect
                if truncate_after == Some(0) {
                    return (
                        StatusCode::OK,
                        [(header::CONTENT_TYPE, "application/json")],
                        "[]".to_string(),
                    )
                        .into_response();
                }
                // Apply disconnect simulation: if disconnect_after_ms is shorter
                // than latency, the response is never sent
                if let Some(ms) = disconnect_after_ms {
                    if latency > 0 && ms < latency {
                        // Disconnect fires before the response would be sent
                        sleep(Duration::from_millis(ms)).await;
                        return (
                            StatusCode::OK,
                            [(header::CONTENT_TYPE, "application/json")],
                            "[]".to_string(),
                        )
                            .into_response();
                    }
                    if ms == 0 {
                        return (
                            StatusCode::OK,
                            [(header::CONTENT_TYPE, "application/json")],
                            "[]".to_string(),
                        )
                            .into_response();
                    }
                }
                if latency > 0 {
                    sleep(Duration::from_millis(latency)).await;
                }
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json")],
                    format!("[{}]", json),
                )
                    .into_response();
            }
        }

        let mut chunks = gemini::build_stream_chunks(content, chunk_size, &user_message);
        // Apply finish_reason override to last streaming chunk
        if let Some(last) = chunks.last_mut() {
            if let Some(candidate) = last.candidates.first_mut() {
                if let Some(reason) = response
                    .finish_reason
                    .as_deref()
                    .or(response.stop_reason.as_deref())
                {
                    candidate.finish_reason = Some(reason.to_string());
                }
            }
        }

        if is_sse {
            // SSE format: data: {json}\n\n
            let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(32);

            tokio::spawn(async move {
                let start = std::time::Instant::now();

                for (sent, chunk) in chunks.iter().enumerate() {
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
            // Default Gemini streaming: JSON array of chunks
            let mut truncated_chunks: Vec<&gemini::GenerateContentResponse> = Vec::new();
            let start = std::time::Instant::now();

            for (i, chunk) in chunks.iter().enumerate() {
                // Yield to allow elapsed time to advance for disconnect checks
                tokio::task::yield_now().await;

                if let Some(ms) = disconnect_after_ms {
                    if start.elapsed() >= Duration::from_millis(ms) {
                        break;
                    }
                }

                if let Some(max) = truncate_after {
                    if i as u32 >= max {
                        break;
                    }
                }

                truncated_chunks.push(chunk);

                if latency > 0 {
                    sleep(Duration::from_millis(latency)).await;
                }
            }

            let json = serde_json::to_string(&truncated_chunks).unwrap();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                json,
            )
                .into_response()
        }
    } else {
        // Non-streaming: generateContent

        // Handle tool calls
        if let Some(ref tool_calls) = response.tool_calls {
            let tc_pairs: Vec<(&str, serde_json::Value)> = tool_calls
                .iter()
                .map(|tc| (tc.name.as_str(), tc.arguments.clone()))
                .collect();
            let mut resp = gemini::build_tool_call_response(&tc_pairs, &user_message);
            if let Some(reason) = response
                .finish_reason
                .as_deref()
                .or(response.stop_reason.as_deref())
            {
                if let Some(c) = resp.candidates.first_mut() {
                    c.finish_reason = Some(reason.to_string());
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

        let mut resp = gemini::build_response(content, &user_message);
        // finish_reason takes priority over stop_reason (alias)
        if let Some(reason) = response
            .finish_reason
            .as_deref()
            .or(response.stop_reason.as_deref())
        {
            if let Some(candidate) = resp.candidates.first_mut() {
                candidate.finish_reason = Some(reason.to_string());
            }
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
