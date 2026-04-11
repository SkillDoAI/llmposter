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
    /// Whether a streaming response should be formatted as Server-Sent
    /// Events (the default for most providers) or as a JSON array
    /// (Gemini's default `streamGenerateContent`). Used when synthesizing
    /// `corrupt_body` responses so an SSE client gets a malformed SSE
    /// frame instead of a text/plain body.
    fn streaming_is_sse(&self) -> bool {
        true
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
    /// Build a safety refusal response body in the provider's native
    /// refusal shape. Used when a matched fixture carries a `refusal`
    /// block instead of `response`.
    fn build_refusal_response(
        &self,
        state: &AppState,
        model: &str,
        reason: &str,
        prompt: &str,
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

/// Flatten an `axum::http::HeaderMap` into a `HashMap` with
/// lowercased keys and UTF-8 decoded values. Invalid UTF-8 values
/// are dropped (treated as if the header wasn't sent).
///
/// Used by the provider axum entry points to pass headers into
/// `handle_request` for fixture matching. Lowercase normalization
/// matches HTTP's case-insensitive header contract.
pub(crate) fn header_map_to_lowercase(
    headers: &axum::http::HeaderMap,
) -> std::collections::HashMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_ascii_lowercase(), v.to_string()))
        })
        .collect()
}

/// Push a `CapturedRequest` into the state's capture log.
///
/// Single construction site for `CapturedRequest` — keeps the
/// `#[non_exhaustive]` struct under one author so future fields land in
/// one place rather than drifting across the matched and non-matched
/// call sites. Also enforces `capture_capacity` FIFO trimming: when the
/// log is at capacity, the oldest entry is dropped to make room.
pub(crate) fn push_captured(
    state: &AppState,
    method: &str,
    path: &str,
    body: String,
    outcome: crate::server::RequestOutcome,
    matched_scenario: Option<String>,
) {
    // `capture_capacity(0)` disables capture entirely — short-circuit
    // BEFORE taking the write lock and constructing the struct so the
    // disable path costs nothing on the hot path.
    if state.capture_capacity == Some(0) {
        return;
    }
    let mut guard = state
        .captured_requests
        .write()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(cap) = state.capture_capacity {
        // FIFO: drop oldest entries until there's room for one more.
        while guard.len() >= cap {
            guard.pop_front();
        }
    }
    guard.push_back(crate::server::CapturedRequest {
        method: method.to_string(),
        path: path.to_string(),
        body,
        outcome,
        matched_scenario,
        timestamp: std::time::Instant::now(),
    });
}

/// Convenience wrapper for early-return paths (bad JSON, failed
/// extraction, auth reject, /code endpoint) that never reach the
/// fixture matcher and therefore never carry a scenario name.
///
/// Short-circuits `capture_capacity == Some(0)` here so the disabled
/// path doesn't pay for the `body.to_string()` allocation AND the
/// subsequent `push_captured` write-lock. Callers only pay for the
/// `&str → String` clone when capture is actually active.
pub(crate) fn capture_non_matched(
    state: &AppState,
    method: &str,
    path: &str,
    body: &str,
    outcome: crate::server::RequestOutcome,
) {
    if state.capture_capacity == Some(0) {
        return;
    }
    push_captured(state, method, path, body.to_string(), outcome, None);
}

/// Generic request handler — all shared boilerplate lives here.
/// `x-request-id` is applied to every response; rate-limit headers are applied on HTTP 429 responses.
pub(crate) async fn handle_request(
    handler: &dyn ProviderHandler,
    state: Arc<AppState>,
    headers: std::collections::HashMap<String, String>,
    body: String,
) -> Response<Body> {
    // Build a 400 response AND capture the request as BadRequest in one
    // place so all three pre-match early-exits stay in lockstep.
    let bad_request = |msg: &str| -> Response<Body> {
        capture_non_matched(
            &state,
            "POST",
            handler.route_label(),
            &body,
            crate::server::RequestOutcome::BadRequest,
        );
        (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            handler.build_error_body(400, msg),
        )
            .into_response()
    };

    let json_body: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return bad_request("Invalid JSON in request body"),
    };

    let (model, user_message) = match handler.extract_request_info(&json_body) {
        Ok(info) => info,
        Err(msg) => return bad_request(&msg),
    };

    // Reject non-boolean stream values — clients sending "true" or 1 would get
    // a silent non-streaming response, masking serialization bugs.
    // Skip for Gemini: streaming is determined by URL action, not a body field.
    if handler.provider() != Provider::Gemini {
        if let Some(sv) = json_body.get("stream") {
            if sv.as_bool().is_none() {
                return bad_request("\"stream\" must be a boolean");
            }
        }
    }
    let is_streaming = handler.is_streaming(&json_body);

    // Match fixture under fixtures read lock (hot-reload-safe) and
    // scenarios write lock (TOCTOU-safe). The capture push happens
    // INSIDE this scope, while the scenarios lock is still held, so
    // the capture order observed by `get_requests()` matches the
    // order in which fixtures were actually matched — concurrent
    // requests racing through the matcher serialize through the
    // same write lock they use to update scenario state. Lock
    // acquisition order (scenarios → captured_requests) matches
    // `MockServer::reset()` so there is no ABBA risk.
    let fixture = {
        let fixtures = state.fixtures.read().unwrap_or_else(|e| e.into_inner());
        let mut scenarios = state.scenarios.write().unwrap_or_else(|e| e.into_inner());

        let ctx = crate::fixture::MatchContext {
            user_message: &user_message,
            model: Some(&model),
            provider: Some(handler.provider()),
            scenario_states: Some(&scenarios),
            headers: &headers,
            body: &json_body,
        };
        let matched = fixtures
            .iter()
            .find(|f| crate::fixture::fixture_matches(f, &ctx));

        let (arc_fixture, scenario_name) = if let Some(f) = matched {
            let name = if let Some(ref scenario) = f.scenario {
                if let Some(ref next_state) = scenario.set_state {
                    scenarios.insert(scenario.name.clone(), next_state.clone());
                }
                Some(scenario.name.clone())
            } else {
                None
            };
            (Some(std::sync::Arc::clone(f)), name)
        } else {
            (None, None)
        };

        let outcome = if arc_fixture.is_some() {
            crate::server::RequestOutcome::Matched
        } else {
            crate::server::RequestOutcome::NoFixtureMatch
        };
        push_captured(
            &state,
            "POST",
            handler.route_label(),
            body,
            outcome,
            scenario_name,
        );
        arc_fixture
    }; // scenarios + fixtures locks released here

    let fixture = match fixture {
        Some(f) => f,
        None => {
            if state.verbose {
                // Intentionally drop the message preview — even
                // truncated prompts can leak PII / secrets into test
                // logs (CI, shared terminals, CRI-O output capture).
                // Char count is enough to correlate with a specific
                // request when you're already looking at the request
                // capture API for the full body.
                let char_count = user_message.chars().count();
                eprintln!(
                    "[llmposter] POST {} → no match (model='{}', msg len={} chars)",
                    handler.route_label(),
                    model,
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

    // Refusal fixtures return a non-streaming refusal-shape body only.
    // Streaming refusals would require per-provider SSE envelope shapes
    // we have not yet implemented; rejecting streaming requests with
    // 400 keeps the wire shape honest — a client with an SSE parser
    // attached would otherwise see `application/json`.
    if let Some(ref refusal) = fixture.refusal {
        if is_streaming {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                handler.build_error_body(
                    400,
                    "refusal fixtures do not currently support streaming — \
                     re-run with `stream: false` or use a regular `response:` fixture",
                ),
            )
                .into_response();
        }
        let body = handler.build_refusal_response(&state, &model, &refusal.reason, &user_message);
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response();
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
    // Content resolution: a plain `content` string wins. A `content_template`
    // is rendered at response time against a small request-derived context.
    // The `templating` feature gates this path; fixture validation rejects
    // `content_template` at load time when the feature is off, so reaching
    // here with a template always means the feature is on.
    #[cfg(feature = "templating")]
    let rendered_template: Option<String> = match response.content_template.as_deref() {
        Some(tmpl) => match crate::templating::render(
            tmpl,
            &response.template_cache,
            &user_message,
            &model,
            handler.provider().as_str(),
            &json_body,
        ) {
            Ok(s) => Some(s),
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(header::CONTENT_TYPE, "application/json")],
                    handler.build_error_body(500, &format!("content_template: {}", e)),
                )
                    .into_response();
            }
        },
        None => None,
    };
    #[cfg(feature = "templating")]
    let content = rendered_template
        .as_deref()
        .or(response.content.as_deref())
        .unwrap_or("");
    #[cfg(not(feature = "templating"))]
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

        // Handle failure: corrupt body.
        //
        // - Non-streaming: text/plain "overloaded" — JSON clients fail to
        //   parse, which is the point.
        // - Streaming SSE: emit a single malformed SSE frame with
        //   text/event-stream so clients testing "mid-stream garbage" see
        //   SSE-shaped corruption instead of a wrong-Content-Type full body.
        // - Streaming JSON-array (Gemini default): keep the text/plain
        //   fallback — clients parse the body as a JSON array and fail.
        if fail.corrupt_body == Some(true) {
            if is_streaming && handler.streaming_is_sse() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    "data: overloaded\n\n".to_string(),
                )
                    .into_response();
            }
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

        // Resolve chaos plan for this request. The chaos counter is advanced
        // whenever the matched fixture has chaos fields configured
        // (`has_chaos() == true`), even if the probability roll ends up
        // returning PASSTHROUGH for this particular request. Requests
        // matching fixtures with no chaos fields at all do not perturb the
        // counter — so a fixed request order against a mixed fixture list
        // still produces a deterministic counter-derived seed sequence.
        let failure_ref = fixture.failure.as_ref();
        let has_chaos = failure_ref.map(|f| f.has_chaos()).unwrap_or(false);
        let chaos_n = if has_chaos {
            state
                .chaos_counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        } else {
            0
        };
        let frame_count = match &stream_output {
            StreamOutput::Sse(v) | StreamOutput::JsonArray(v) => v.len(),
        };
        let plan =
            crate::chaos::ChaosPlan::from_failure(failure_ref, latency, frame_count, chaos_n);
        // Only log when chaos has an observable effect. `plan.active`
        // merely says "the probability roll passed", but a fixture with
        // only `chaos_seed` set (no jitter, no duplication) would still
        // roll active and leave the stream bit-identical to passthrough.
        // The degenerate-config warning at fixture load time covers this
        // at build time; the verbose log stays quiet at request time.
        if state.verbose && plan.active && (plan.duplicate || plan.frame_delays_ms.is_some()) {
            eprintln!("[llmposter] POST {} → chaos active", handler.route_label());
        }

        match stream_output {
            StreamOutput::Sse(frames) => {
                let frames = plan.apply_frame_duplication(frames);
                stream_sse_frames(frames, latency, &plan, truncate_after, disconnect_after_ms).await
            }
            StreamOutput::JsonArray(frames) => {
                let frames = plan.apply_frame_duplication(frames);
                stream_json_array(frames, latency, &plan, truncate_after, disconnect_after_ms).await
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
///
/// Inter-frame delay is read from the [`ChaosPlan`] — when the plan carries
/// no per-frame overrides (the common case), every delay is `base_latency`.
async fn stream_sse_frames(
    frames: Vec<String>,
    base_latency: u64,
    plan: &crate::chaos::ChaosPlan,
    truncate_after: Option<u32>,
    disconnect_after_ms: Option<u64>,
) -> Response<Body> {
    // If the override vector length doesn't match the frame count, log a
    // warning and fall back to the base latency for every frame. This is a
    // belt-and-braces check — the handler always passes a plan built from
    // the same frame count — but protects against future refactors that
    // might let the invariant drift, without panicking in release builds.
    let delays_override = match plan.frame_delays_ms.as_ref() {
        Some(v) if v.len() == frames.len() => Some(v.clone()),
        Some(v) => {
            eprintln!(
                "[llmposter] stream_sse_frames: frame_delays_ms length mismatch \
                 (frames={}, delays={}) — falling back to base latency",
                frames.len(),
                v.len()
            );
            None
        }
        None => None,
    };
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
                let delay = delays_override
                    .as_ref()
                    .and_then(|v| v.get(sent).copied())
                    .unwrap_or(base_latency);
                if delay > 0 && sent + 1 < total {
                    sleep(Duration::from_millis(delay)).await;
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
///
/// Inter-frame delay is read from the [`ChaosPlan`] the same way
/// [`stream_sse_frames`] does. Disconnect enforcement uses a bounded sleep
/// (`delay.min(remaining)`) so the disconnect still fires even when jitter
/// produced a long delay.
async fn stream_json_array(
    frames: Vec<String>,
    base_latency: u64,
    plan: &crate::chaos::ChaosPlan,
    truncate_after: Option<u32>,
    disconnect_after_ms: Option<u64>,
) -> Response<Body> {
    // Same runtime-safe length check as stream_sse_frames: on mismatch,
    // log and fall back to base latency for every frame.
    let delays_override = match plan.frame_delays_ms.as_ref() {
        Some(v) if v.len() == frames.len() => Some(v.as_slice()),
        Some(v) => {
            eprintln!(
                "[llmposter] stream_json_array: frame_delays_ms length mismatch \
                 (frames={}, delays={}) — falling back to base latency",
                frames.len(),
                v.len()
            );
            None
        }
        None => None,
    };
    let mut collected: Vec<String> = Vec::new();
    let start = Instant::now();
    let total = frames.len();

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

        // Mirror `stream_sse_frames`: skip the inter-frame delay after
        // the final frame. Sleeping after the last frame adds a pointless
        // `base_latency` to every JSON-array response and, when combined
        // with `disconnect_after_ms`, can drop an already-buffered final
        // frame via the post-sleep check below.
        if i + 1 >= total {
            break;
        }

        let delay = delays_override
            .and_then(|v| v.get(i).copied())
            .unwrap_or(base_latency);
        if delay > 0 {
            if let Some(ms) = disconnect_after_ms {
                let remaining = ms.saturating_sub(elapsed_ms(&start));
                if remaining == 0 {
                    break;
                }
                let wait = Duration::from_millis(delay.min(remaining));
                sleep(wait).await;
                if start.elapsed() >= Duration::from_millis(ms) {
                    // Disconnect fired during latency — drop the last buffered frame
                    collected.pop();
                    break;
                }
            } else {
                sleep(Duration::from_millis(delay)).await;
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

#[cfg(test)]
mod mod_tests {
    use super::*;
    use crate::chaos::ChaosPlan;

    /// Fabricate a plan whose `frame_delays_ms` length deliberately
    /// doesn't match the frame count. Used to exercise the runtime
    /// length-mismatch fallback paths in both stream helpers.
    fn mismatched_plan() -> ChaosPlan {
        ChaosPlan {
            frame_delays_ms: Some(vec![5, 5]), // only 2 delays
            duplicate: false,
            active: true,
        }
    }

    /// Drain any `Response<Body>` (SSE or JSON-array) to a single `String`.
    async fn collect_body(resp: Response<Body>) -> String {
        use axum::body::to_bytes;
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// Extract the ordered list of `data: ...` payloads from a raw SSE
    /// body. Ignores empty separator lines and `event:` lines.
    fn sse_data_frames(body: &str) -> Vec<&str> {
        body.lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .collect()
    }

    /// Parse a JSON-array response body into its element strings.
    fn json_array_elements(body: &str) -> Vec<String> {
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        v.as_array()
            .unwrap()
            .iter()
            .map(|el| serde_json::to_string(el).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn stream_sse_frames_falls_back_on_length_mismatch() {
        // 3 frames but only 2 delays — handler should log and fall back to
        // base latency for every frame without panicking. Verify ALL 3
        // frames are present in the body; a bug in the fallback would
        // drop the 3rd frame via Vec::get(2).copied() returning None.
        let frames = vec![
            "data: a\n\n".to_string(),
            "data: b\n\n".to_string(),
            "data: c\n\n".to_string(),
        ];
        let resp = stream_sse_frames(frames, 0, &mismatched_plan(), None, None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = collect_body(resp).await;
        assert_eq!(sse_data_frames(&body), vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn stream_json_array_falls_back_on_length_mismatch() {
        // Same invariant for the JSON-array path: all 3 elements should
        // land in the output despite the 2-element delays vec.
        let frames = vec![
            "\"a\"".to_string(),
            "\"b\"".to_string(),
            "\"c\"".to_string(),
        ];
        let resp = stream_json_array(frames, 0, &mismatched_plan(), None, None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = collect_body(resp).await;
        assert_eq!(json_array_elements(&body), vec!["\"a\"", "\"b\"", "\"c\""]);
    }

    #[tokio::test]
    async fn stream_sse_frames_uses_override_when_lengths_match() {
        // Zero per-frame delays (chaos plan override) — both frames should
        // emit cleanly without any inter-frame sleep.
        let frames = vec!["data: a\n\n".to_string(), "data: b\n\n".to_string()];
        let plan = ChaosPlan {
            frame_delays_ms: Some(vec![0, 0]),
            duplicate: false,
            active: true,
        };
        let resp = stream_sse_frames(frames, 100, &plan, None, None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = collect_body(resp).await;
        assert_eq!(sse_data_frames(&body), vec!["a", "b"]);
    }

    #[tokio::test]
    async fn stream_json_array_disconnect_during_latency_drops_last_frame() {
        // Base latency (50ms) is far enough past the disconnect deadline
        // (10ms) that the first frame's inter-frame sleep crosses the
        // deadline — the handler then detects `elapsed >= ms` after the
        // bounded sleep and pops the last-buffered frame. The emitted JSON
        // array must therefore be EMPTY: the first frame was pushed then
        // popped on the same iteration.
        let frames = vec![
            "\"a\"".to_string(),
            "\"b\"".to_string(),
            "\"c\"".to_string(),
        ];
        let plan = ChaosPlan::PASSTHROUGH;
        let resp = stream_json_array(frames, 50, &plan, None, Some(10)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = collect_body(resp).await;
        let elements = json_array_elements(&body);
        // Either zero elements (first frame popped) or at most one
        // (depends on tokio scheduling granularity, but never all three).
        assert!(
            elements.len() < 3,
            "expected disconnect to truncate the stream, got {:?}",
            elements
        );
    }

    #[tokio::test]
    async fn stream_json_array_disconnect_remaining_zero_break() {
        // 15ms per-frame latency, 5ms disconnect — the first frame's
        // bounded sleep (min of 15 and remaining) crosses the deadline,
        // the pop-last branch triggers, and the loop exits.
        let frames = vec![
            "\"a\"".to_string(),
            "\"b\"".to_string(),
            "\"c\"".to_string(),
        ];
        let plan = ChaosPlan::PASSTHROUGH;
        let resp = stream_json_array(frames, 15, &plan, None, Some(5)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = collect_body(resp).await;
        let elements = json_array_elements(&body);
        // Same invariant: the disconnect must truncate the output.
        assert!(
            elements.len() < 3,
            "expected disconnect to truncate the stream, got {:?}",
            elements
        );
    }
}
