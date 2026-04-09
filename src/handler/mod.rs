/// Anthropic Messages API handler (`POST /v1/messages`).
pub mod anthropic;
/// Gemini generateContent handler (`POST /v1beta/models/{model}:generateContent`).
pub mod gemini;
/// OpenAI Chat Completions handler (`POST /v1/chat/completions`).
pub mod openai;
/// OpenAI Responses API handler (`POST /v1/responses`).
pub mod responses;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Response, StatusCode};
use axum::response::IntoResponse;
use tokio::time::sleep;

use crate::failure;
use crate::fixture::match_fixture;
use crate::format::Provider;
use crate::server::AppState;

/// Elapsed milliseconds since `start`, capped at u64::MAX.
fn elapsed_ms(start: &Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

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
    /// Return the provider enum variant (OpenAI, Anthropic, Gemini, etc.).
    fn provider(&self) -> Provider;
    /// Return the route path label used for logging and captured requests.
    fn route_label(&self) -> &str;
    /// Build a provider-specific error response body.
    /// Default implementation returns OpenAI-style JSON.
    fn build_error_body(&self, status: u16, message: &str) -> String {
        failure::build_error_body(status, message)
    }
    /// Parse the JSON request body and return `(model, user_message)`.
    /// Returns `Err(message)` if required fields are missing or malformed.
    fn extract_request_info(&self, body: &serde_json::Value) -> Result<(String, String), String>;
    /// Return whether the request asks for streaming.
    /// Default checks `body["stream"]`; Gemini overrides via URL action.
    fn is_streaming(&self, body: &serde_json::Value) -> bool {
        body["stream"].as_bool().unwrap_or(false)
    }
    /// Return the provider's default stop/finish reason (e.g. `"end_turn"`, `"stop"`).
    fn default_stop_reason(&self) -> &str;
    /// Build a complete non-streaming JSON response with text content.
    fn build_response(
        &self,
        state: &AppState,
        model: &str,
        content: &str,
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> String;
    /// Build a complete non-streaming JSON response with tool calls.
    fn build_tool_call_response(
        &self,
        state: &AppState,
        model: &str,
        tool_calls: &[(&str, serde_json::Value)],
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> String;
    /// Split text content into streaming frames (SSE or JSON-array).
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
    /// Split tool calls into streaming frames (SSE or JSON-array).
    fn build_tool_call_stream_frames(
        &self,
        state: &AppState,
        model: &str,
        tool_calls: &[(&str, serde_json::Value)],
        chunk_size: usize,
        prompt: &str,
        stop_reason: &str,
        has_explicit_reason: bool,
    ) -> StreamOutput;
}

/// Generic request handler — all shared boilerplate lives here.
/// `x-request-id` is applied to every response; rate-limit headers are applied on HTTP 429 responses.
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

    // Reject non-boolean stream values — clients sending "true" or 1 would get
    // a silent non-streaming response, masking serialization bugs.
    // Skip for Gemini: streaming is determined by URL action, not a body field.
    if handler.provider() != Provider::Gemini {
        if let Some(sv) = json_body.get("stream") {
            if sv.as_bool().is_none() {
                return (
                    StatusCode::BAD_REQUEST,
                    [(header::CONTENT_TYPE, "application/json")],
                    handler.build_error_body(400, "\"stream\" must be a boolean"),
                )
                    .into_response();
            }
        }
    }
    let is_streaming = handler.is_streaming(&json_body);

    // Match fixture under scenarios write lock (TOCTOU-safe).
    // Extract scenario name inside the lock, capture request AFTER releasing.
    let (fixture, scenario_name) = {
        let mut scenarios = state.scenarios.write().unwrap_or_else(|e| e.into_inner());

        let matched = match_fixture(
            &state.fixtures,
            &user_message,
            Some(&model),
            Some(handler.provider()),
            Some(&scenarios),
        );

        if let Some(f) = matched {
            let name = if let Some(ref scenario) = f.scenario {
                if let Some(ref next_state) = scenario.set_state {
                    scenarios.insert(scenario.name.clone(), next_state.clone());
                }
                Some(scenario.name.clone())
            } else {
                None
            };
            (Some(f), name)
        } else {
            (None, None)
        }
    }; // scenarios lock released here

    // Capture request in a single write — body is moved, not cloned.
    // Scenario name is already resolved, so no second lock acquisition needed.
    state
        .captured_requests
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .push(crate::server::CapturedRequest {
            method: "POST".to_string(),
            path: handler.route_label().to_string(),
            body,
            matched_scenario: scenario_name,
            timestamp: std::time::Instant::now(),
        });

    let fixture = match fixture {
        Some(f) => f,
        None => {
            if state.verbose {
                let char_count = user_message.chars().count();
                let preview: String = user_message.chars().take(50).collect();
                eprintln!(
                    "[llmposter] POST {} → no match (model='{}', msg='{}...' ({} chars))",
                    handler.route_label(),
                    model,
                    preview,
                    char_count
                );
            }
            let msg = format!("No fixture matched for model='{}'", model);
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
        let body = handler.build_error_body(status.as_u16(), &err.message);
        let mut builder = Response::builder().status(status);
        for (name, value) in &err.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let has_content_type = err
            .headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case("content-type"));
        if !has_content_type {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
        }
        return match builder.body(Body::from(body)) {
            Ok(resp) => resp.into_response(),
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                handler.build_error_body(500, "Fixture contains invalid header name or value"),
            )
                .into_response(),
        };
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
    // stop_reason takes precedence (Anthropic-native), finish_reason is the alias
    let stop_reason = response
        .stop_reason
        .as_deref()
        .or(response.finish_reason.as_deref())
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
                chunk_size,
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
        // send_frames has NO internal deadline checks — disconnect is enforced
        // solely by the outer select! so ConnectionReset is always injected.
        let send_frames = async {
            let total = frames.len();
            for (sent, frame) in frames.into_iter().enumerate() {
                tokio::task::yield_now().await;

                if let Some(max) = truncate_after {
                    if sent as u32 >= max {
                        return;
                    }
                }

                if tx.send(Ok(frame)).await.is_err() {
                    return;
                }

                // Sleep between frames, but not after the last one — avoids
                // giving the disconnect timer a window after all content is sent.
                if latency > 0 && sent + 1 < total {
                    sleep(Duration::from_millis(latency)).await;
                }
            }
        };

        // When disconnect_after_ms is set, race the frame sender against the deadline.
        // The biased select! checks the sleep branch first for determinism —
        // if both futures are ready, the disconnect always wins.
        if let Some(ms) = disconnect_after_ms {
            tokio::select! {
                biased;
                _ = sleep(Duration::from_millis(ms)) => {
                    let _ = tx
                        .send(Err(std::io::Error::new(
                            std::io::ErrorKind::ConnectionReset,
                            "llmposter: simulated disconnect",
                        )))
                        .await;
                }
                _ = send_frames => {}
            }
        } else {
            send_frames.await;
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    // No Connection header — axum/hyper manages it per protocol version.
    // Sending Connection: keep-alive is invalid on HTTP/2.
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
        .expect("static SSE response headers")
}

/// Stream Gemini JSON-array frames with truncation/disconnect support.
/// Uses bounded sleep (`latency.min(remaining)`) for disconnect enforcement.
async fn stream_json_array(
    frames: Vec<String>,
    latency: u64,
    truncate_after: Option<u32>,
    disconnect_after_ms: Option<u64>,
) -> Response<Body> {
    let mut collected: Vec<String> = Vec::new();
    let start = Instant::now();

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
                let remaining = ms.saturating_sub(elapsed_ms(&start));
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
