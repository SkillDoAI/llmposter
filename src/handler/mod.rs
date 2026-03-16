pub mod anthropic;
pub mod gemini;
pub mod openai;
pub mod responses;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Response, StatusCode};
use axum::response::IntoResponse;
use tokio::time::sleep;

use crate::failure;
use crate::fixture::match_fixture;
use crate::format::Provider;
use crate::server::AppState;

/// Streaming output mode for the generic handler.
pub(crate) enum StreamOutput {
    /// SSE frames: `data: ...\n\n` or `event: ...\ndata: ...\n\n`
    Sse(Vec<String>),
    /// Gemini JSON-array streaming: returns a JSON array string directly.
    JsonArray(Vec<String>),
}

/// Each provider implements this trait so the generic handler can delegate
/// format-specific logic while owning all shared boilerplate.
#[allow(clippy::too_many_arguments)]
pub(crate) trait ProviderHandler: Send + Sync {
    fn provider(&self) -> Provider;
    fn route_label(&self) -> &str;
    /// Build a provider-specific error response body.
    /// Default implementation returns OpenAI-style JSON.
    fn build_error_body(&self, status: u16, message: &str) -> String {
        failure::build_error_body(status, message)
    }
    fn extract_request_info(&self, body: &serde_json::Value) -> Result<(String, String), String>;
    fn is_streaming(&self, body: &serde_json::Value) -> bool {
        body["stream"].as_bool().unwrap_or(false)
    }
    fn default_stop_reason(&self) -> &str;
    fn build_response(
        &self,
        state: &AppState,
        model: &str,
        content: &str,
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> String;
    fn build_tool_call_response(
        &self,
        state: &AppState,
        model: &str,
        tool_calls: &[(&str, serde_json::Value)],
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> String;
    fn build_stream_frames(
        &self,
        state: &AppState,
        model: &str,
        content: &str,
        chunk_size: usize,
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> StreamOutput;
    fn build_tool_call_stream_frames(
        &self,
        state: &AppState,
        model: &str,
        tool_calls: &[(&str, serde_json::Value)],
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> StreamOutput;
}

/// Generic request handler — all shared boilerplate lives here.
pub(crate) async fn handle_request(
    handler: &dyn ProviderHandler,
    state: Arc<AppState>,
    body: String,
) -> Response<Body> {
    let json_body: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                handler.build_error_body(400, "Invalid JSON in request body"),
            )
                .into_response();
        }
    };

    let (model, user_message) = match handler.extract_request_info(&json_body) {
        Ok(info) => info,
        Err(msg) => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                handler.build_error_body(400, &msg),
            )
                .into_response();
        }
    };

    let is_streaming = handler.is_streaming(&json_body);

    let fixture = match match_fixture(
        &state.fixtures,
        &user_message,
        Some(&model),
        Some(handler.provider()),
    ) {
        Some(f) => f,
        None => {
            if state.verbose {
                let preview: String = user_message.chars().take(50).collect();
                eprintln!(
                    "[llmposter] POST {} → no match (model='{}', msg='{}...' ({} chars))",
                    handler.route_label(),
                    model,
                    preview,
                    user_message.chars().count()
                );
            }
            let truncated = user_message.chars().count() > 80;
            let preview: String = user_message.chars().take(80).collect();
            let ellipsis = if truncated { "..." } else { "" };
            let msg = format!(
                "No fixture matched: model='{}', user_message='{}{}'",
                model, preview, ellipsis
            );
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "application/json")],
                handler.build_error_body(404, &msg),
            )
                .into_response();
        }
    };

    if state.verbose {
        eprintln!(
            "[llmposter] POST {} → fixture matched",
            handler.route_label()
        );
    }

    // Handle error fixtures
    if let Some(ref err) = fixture.error {
        let status = StatusCode::from_u16(err.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        return (
            status,
            [(header::CONTENT_TYPE, "application/json")],
            handler.build_error_body(status.as_u16(), &err.message),
        )
            .into_response();
    }

    let response = match fixture.response.as_ref() {
        Some(r) => r,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                handler.build_error_body(500, "Fixture has neither response nor error"),
            )
                .into_response();
        }
    };
    let content = response.content.as_deref().unwrap_or("");
    let has_explicit_reason = response.stop_reason.is_some() || response.finish_reason.is_some();
    // Support both finish_reason and stop_reason as aliases
    let stop_reason = response
        .finish_reason
        .as_deref()
        .or(response.stop_reason.as_deref())
        .unwrap_or(handler.default_stop_reason());

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

    let tc_pairs: Option<Vec<(&str, serde_json::Value)>> =
        response.tool_calls.as_ref().map(|tool_calls| {
            tool_calls
                .iter()
                .map(|tc| (tc.name.as_str(), tc.arguments.clone()))
                .collect()
        });

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
            .and_then(|f| f.truncate_after_frames);
        let disconnect_after_ms = fixture.failure.as_ref().and_then(|f| f.disconnect_after_ms);

        let stream_output = if let Some(ref tc) = tc_pairs {
            handler.build_tool_call_stream_frames(
                &state,
                &model,
                tc,
                &user_message,
                stop_reason,
                has_explicit_reason,
            )
        } else {
            handler.build_stream_frames(
                &state,
                &model,
                content,
                chunk_size,
                &user_message,
                stop_reason,
                has_explicit_reason,
            )
        };

        match stream_output {
            StreamOutput::Sse(frames) => {
                stream_sse_frames(frames, latency, truncate_after, disconnect_after_ms).await
            }
            StreamOutput::JsonArray(frames) => {
                stream_json_array(frames, latency, truncate_after, disconnect_after_ms).await
            }
        }
    } else {
        // Non-streaming
        let json = if let Some(ref tc) = tc_pairs {
            handler.build_tool_call_response(
                &state,
                &model,
                tc,
                &user_message,
                stop_reason,
                has_explicit_reason,
            )
        } else {
            handler.build_response(
                &state,
                &model,
                content,
                &user_message,
                stop_reason,
                has_explicit_reason,
            )
        };

        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            json,
        )
            .into_response()
    }
}

/// Stream SSE frames via mpsc channel with truncation/disconnect support.
async fn stream_sse_frames(
    frames: Vec<String>,
    latency: u64,
    truncate_after: Option<u32>,
    disconnect_after_ms: Option<u64>,
) -> Response<Body> {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<String, std::io::Error>>(32);

    tokio::spawn(async move {
        let send_frames = async {
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
                    if let Some(ms) = disconnect_after_ms {
                        let remaining = ms.saturating_sub(start.elapsed().as_millis() as u64);
                        if remaining == 0 {
                            return;
                        }
                        let wait = Duration::from_millis(latency.min(remaining));
                        sleep(wait).await;
                        if start.elapsed() >= Duration::from_millis(ms) {
                            return;
                        }
                    } else {
                        sleep(Duration::from_millis(latency)).await;
                    }
                }
            }
        };

        // When disconnect_after_ms is set, race the frame sender against the deadline
        // to ensure disconnect fires even with zero latency between frames.
        if let Some(ms) = disconnect_after_ms {
            tokio::select! {
                _ = send_frames => {}
                _ = sleep(Duration::from_millis(ms)) => {}
            }
        } else {
            send_frames.await;
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(Body::from_stream(stream))
        .expect("static SSE response headers")
}

/// Stream Gemini JSON-array frames with truncation/disconnect support.
/// Uses `tokio::select!` to race latency sleep against disconnect timer.
async fn stream_json_array(
    frames: Vec<String>,
    latency: u64,
    truncate_after: Option<u32>,
    disconnect_after_ms: Option<u64>,
) -> Response<Body> {
    let mut collected: Vec<String> = Vec::new();
    let start = std::time::Instant::now();

    for (i, frame) in frames.into_iter().enumerate() {
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

        collected.push(frame);

        if latency > 0 {
            if let Some(ms) = disconnect_after_ms {
                let remaining = ms.saturating_sub(start.elapsed().as_millis() as u64);
                if remaining == 0 {
                    break;
                }
                let wait = Duration::from_millis(latency.min(remaining));
                sleep(wait).await;
                if start.elapsed() >= Duration::from_millis(ms) {
                    // Disconnect fired during latency — drop the last buffered frame
                    collected.pop();
                    break;
                }
            } else {
                sleep(Duration::from_millis(latency)).await;
            }
        }
    }

    let json = format!("[{}]", collected.join(","));
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json,
    )
        .into_response()
}
