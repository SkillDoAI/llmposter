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

use super::{extract_for, reassemble_for, OpenAiEndpoint, Recorder};

/// Upstream RESPONSE headers copied onto the relayed response when
/// present. `retry-after` keeps client backoff logic working on
/// passed-through 429s (and pre-empts the middleware's `retry-after: 60`
/// fallback, which uses `or_insert`); the request-id pair keeps upstream
/// responses correlatable.
const RELAY_RESPONSE_HEADERS: &[&str] = &["retry-after", "x-request-id", "request-id"];

/// Upstream RESPONSE header PREFIXES relayed by lowercase prefix match —
/// the OpenAI and Anthropic rate-limit families, so client throttling
/// logic sees the real upstream budget instead of llmposter's mock values.
const RELAY_RESPONSE_HEADER_PREFIXES: &[&str] = &["x-ratelimit-", "anthropic-ratelimit-"];

/// Cap on the stream tee's capture buffer — symmetric with the
/// request-side `DefaultBodyLimit` (16 MB, server.rs). Exceeding it
/// abandons the recording (`clean = false`), which also bounds the
/// post-client-disconnect salvage drain against an endlessly streaming
/// upstream.
const SALVAGE_BUFFER_CAP: usize = 16 * 1024 * 1024;

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
    /// Whether a streaming response is SSE (`true` for everything except
    /// Gemini's default JSON-array `streamGenerateContent` — SSE only
    /// with `?alt=sse`). Only consulted when `is_streaming` is true.
    sse: bool,
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
            sse: handler.streaming_is_sse(),
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
            sse: true, // never consulted — embeddings never streams
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
/// - 2xx SSE streaming: teed to the client chunk-by-chunk while a spawned
///   task buffers the frames; the task reassembles and persists once the
///   stream ends cleanly, so the client never waits on the recording.
/// - 2xx Gemini JSON-array streaming (no `?alt=sse`): buffered
///   passthrough, UNRECORDED — out of capture scope by spec.
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
        sse,
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
    // `HeaderName::as_str()` is always lowercase, so exact names and
    // prefix matches below are already case-insensitive.
    let relay_headers: Vec<(String, String)> = upstream
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            let name = name.as_str();
            let keep = RELAY_RESPONSE_HEADERS.contains(&name)
                || RELAY_RESPONSE_HEADER_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix));
            if !keep {
                return None;
            }
            value
                .to_str()
                .ok()
                .map(|v| (name.to_string(), v.to_string()))
        })
        .collect();

    let is_2xx = (200..300).contains(&status);

    // --- 2xx SSE stream: tee upstream chunks to the client while a
    // spawned task buffers them; reassemble + persist once the stream
    // ends cleanly. The response returns immediately — the recording
    // happens strictly after the last byte reaches the buffer.
    if is_2xx && is_streaming && sse {
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(32);
        let state = Arc::clone(state);
        let model = model.to_string();
        let user_message = user_message.to_string();
        let task_path = path;
        // Deliberately unowned: the task may still be finishing a salvage
        // persist after MockServer::drop; runtime teardown bounds it in
        // tests.
        tokio::spawn(async move {
            let mut stream = Box::pin(upstream.bytes_stream());
            let mut buf: Vec<u8> = Vec::new();
            let mut clean = true;
            let mut client_connected = true;
            while let Some(item) = tokio_stream::StreamExt::next(&mut stream).await {
                match item {
                    Ok(chunk) => {
                        buf.extend_from_slice(&chunk);
                        if buf.len() > SALVAGE_BUFFER_CAP {
                            // An SSE response this large is not a
                            // recordable fixture, and without the cap a
                            // disconnected client would leave this drain
                            // loop buffering an endless upstream forever.
                            clean = false;
                            break;
                        }
                        if client_connected && tx.send(Ok(chunk)).await.is_err() {
                            // Client hung up — keep draining so the
                            // recording can still complete.
                            client_connected = false;
                        }
                    }
                    Err(e) => {
                        // `without_url` so a Gemini `?key=...` never leaks.
                        eprintln!(
                            "[llmposter] ERROR: record-mode upstream stream for POST {} \
                             failed mid-stream: {}",
                            task_path,
                            e.without_url()
                        );
                        clean = false;
                        // Surface a REAL transport error to the client —
                        // without it, dropping tx would read as a
                        // clean-looking end of a chunked response.
                        let _ = tx
                            .send(Err(std::io::Error::other("upstream stream failed")))
                            .await;
                        break;
                    }
                }
            }
            drop(tx);
            if clean {
                let body = String::from_utf8_lossy(&buf);
                match reassemble_for(provider, endpoint, &body, &model, &user_message) {
                    Some(rec) => recorder.persist(rec, &state).await,
                    None => eprintln!(
                        "[llmposter] record: POST {} (model='{}') — stream had no \
                         extractable content or no completion sentinel — \
                         passed through, not recorded",
                        task_path, model
                    ),
                }
            }
            // ALWAYS pushed from the task — the capture log records what
            // the client actually received, clean or truncated.
            push_captured(
                &state,
                "POST",
                &task_path,
                capture_body,
                RequestOutcome::Recorded,
                None,
                status,
            );
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let mut builder = Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, &content_type)
            .header(header::CACHE_CONTROL, "no-cache");
        for (name, value) in &relay_headers {
            builder = builder.header(name.as_str(), value);
        }
        return match builder.body(Body::from_stream(stream)) {
            Ok(resp) => resp,
            // Unreachable in practice — every relayed header came off a
            // parsed upstream response — but never panic in the hot path.
            Err(_) => (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                error_body(502, "upstream response could not be relayed"),
            )
                .into_response(),
        };
    }

    // Exact-byte passthrough for everything else.
    let bytes = match upstream.bytes().await {
        Ok(b) => b,
        Err(e) => return bad_gateway(e, capture_body),
    };

    if is_2xx && !is_streaming {
        match serde_json::from_slice::<serde_json::Value>(&bytes) {
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
    // Non-2xx and Gemini JSON-array streams (is_streaming && !sse) fall
    // through here unrecorded.

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
        builder = builder.header(name.as_str(), value);
    }
    match builder.body(Body::from(bytes)) {
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
