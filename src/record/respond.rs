//! Record-mode request path: forward the request upstream, persist
//! extractable 2xx responses as fixtures, and relay the upstream
//! response to the client.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Response, StatusCode};
use axum::response::IntoResponse;

use crate::format::Provider;
use crate::handler::{push_captured, ProviderHandler};
use crate::server::{AppState, RequestOutcome};

use super::{extract_for, OpenAiEndpoint, Recorder};

/// Upstream RESPONSE headers copied onto the relayed response when
/// present. `retry-after` keeps client backoff logic working on
/// passed-through 429s (and pre-empts the middleware's `retry-after: 60`
/// fallback, which uses `or_insert`); the request-id pair keeps upstream
/// responses correlatable. The rate-limit header family is deliberately
/// deferred to Task 6.
const RELAY_RESPONSE_HEADERS: &[&str] = &["retry-after", "x-request-id", "request-id"];

/// What [`forward_and_relay`] needs to know about the endpoint being
/// forwarded — built from a [`ProviderHandler`] for the five generic
/// routes, or inline for the standalone embeddings handler.
struct ForwardTarget<'a> {
    path: String,
    query: Option<String>,
    provider: Provider,
    /// OpenAI-endpoint discriminator for extraction; non-OpenAI providers
    /// never consult it.
    endpoint: OpenAiEndpoint,
    is_streaming: bool,
    /// Provider-shaped error body for the 502 exits.
    error_body: &'a (dyn Fn(u16, &str) -> String + Sync),
}

/// Record-mode path for the five [`ProviderHandler`] routes. See
/// [`forward_and_relay`] for the forward/persist/relay semantics.
#[allow(clippy::too_many_arguments)] // mirrors ProviderHandler's allow — the args are the request
pub(crate) async fn record_and_respond(
    recorder: Arc<Recorder>,
    handler: &dyn ProviderHandler,
    state: &Arc<AppState>,
    headers: &HashMap<String, String>,
    body: String,
    json_body: &serde_json::Value,
    model: &str,
    user_message: &str,
) -> Response<Body> {
    let (path, query) = handler.forward_path_and_query();
    forward_and_relay(
        recorder,
        ForwardTarget {
            endpoint: OpenAiEndpoint::from_path(&path),
            path,
            query,
            provider: handler.provider(),
            is_streaming: handler.is_streaming(json_body),
            error_body: &|status, msg| handler.build_error_body(status, msg),
        },
        state,
        headers,
        body,
        model,
        user_message,
    )
    .await
}

/// Record-mode path for the standalone embeddings handler (which does
/// not implement [`ProviderHandler`]): fixed `/v1/embeddings` path,
/// OpenAI provider and error shape, never streaming.
pub(crate) async fn record_and_respond_embeddings(
    recorder: Arc<Recorder>,
    state: &Arc<AppState>,
    headers: &HashMap<String, String>,
    body: String,
    model: &str,
    user_message: &str,
) -> Response<Body> {
    forward_and_relay(
        recorder,
        ForwardTarget {
            path: "/v1/embeddings".to_string(),
            query: None,
            provider: Provider::OpenAI,
            endpoint: OpenAiEndpoint::Embeddings,
            is_streaming: false,
            error_body: &|status, msg| crate::failure::build_error_body(status, msg),
        },
        state,
        headers,
        body,
        model,
        user_message,
    )
    .await
}

/// Forward the request upstream and relay the response.
///
/// - 2xx non-streaming: extract → redact → dedupe → persist to the
///   cassette AND the live fixture set, synchronously, before responding.
/// - 2xx streaming: passed through UNRECORDED (buffered — see below).
/// - Non-2xx: passed through unrecorded (a 429 must not be immortalized).
/// - Transport errors: 502 naming the upstream base — NEVER echoing any
///   header or auth material.
async fn forward_and_relay(
    recorder: Arc<Recorder>,
    target: ForwardTarget<'_>,
    state: &Arc<AppState>,
    headers: &HashMap<String, String>,
    body: String,
    model: &str,
    user_message: &str,
) -> Response<Body> {
    let ForwardTarget {
        path,
        query,
        provider,
        endpoint,
        is_streaming,
        error_body,
    } = target;
    let capture_body = body.clone();
    let upstream_base = recorder.upstream_base(provider).to_string();

    // Shared 502 exit for both transport failures below. `err` is a
    // reqwest error stripped of its URL (`without_url`) so a Gemini
    // `?key=...` can never leak into logs or the response body.
    let bad_gateway = |err: reqwest::Error, capture_body: String| -> Response<Body> {
        let err = err.without_url();
        eprintln!(
            "[llmposter] ERROR: record-mode forward to {} failed: {}",
            upstream_base, err
        );
        push_captured(
            state,
            "POST",
            &path,
            capture_body,
            RequestOutcome::Recorded,
            None,
            502,
        );
        (
            StatusCode::BAD_GATEWAY,
            [(header::CONTENT_TYPE, "application/json")],
            error_body(
                502,
                &format!("upstream {} unreachable: {}", upstream_base, err),
            ),
        )
            .into_response()
    };

    let upstream = match recorder
        .forward(provider, &path, query.as_deref(), headers, body)
        .await
    {
        Ok(resp) => resp,
        Err(e) => return bad_gateway(e, capture_body),
    };

    let status = upstream.status().as_u16();
    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE.as_str())
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let relay_headers: Vec<(&'static str, String)> = RELAY_RESPONSE_HEADERS
        .iter()
        .filter_map(|&name| {
            upstream
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|v| (name, v.to_string()))
        })
        .collect();
    let text = match upstream.text().await {
        Ok(t) => t,
        Err(e) => return bad_gateway(e, capture_body),
    };

    let is_2xx = (200..300).contains(&status);
    if is_2xx && !is_streaming {
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value) => match extract_for(provider, endpoint, &value, model, user_message) {
                Some(rec) => recorder.persist(rec, state).await,
                None => eprintln!(
                    "[llmposter] record: POST {} (model='{}') — no extractable \
                         content in upstream response — passed through, not recorded",
                    path, model
                ),
            },
            Err(_) => eprintln!(
                "[llmposter] record: upstream 2xx body was not JSON — \
                 passed through, not recorded"
            ),
        }
    }
    // Buffered interim: Task 6 replaces this with true stream-through + capture.
    // (2xx streaming responses pass through unrecorded; non-2xx always pass
    // through unrecorded.)

    push_captured(
        state,
        "POST",
        &path,
        capture_body,
        RequestOutcome::Recorded,
        None,
        status,
    );

    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, &content_type);
    for (name, value) in &relay_headers {
        builder = builder.header(*name, value);
    }
    match builder.body(Body::from(text)) {
        Ok(resp) => resp,
        // Unreachable in practice — status and every relayed header came
        // off a parsed upstream response — but never panic in the hot path.
        Err(_) => (
            StatusCode::BAD_GATEWAY,
            [(header::CONTENT_TYPE, "application/json")],
            error_body(502, "upstream response could not be relayed"),
        )
            .into_response(),
    }
}
