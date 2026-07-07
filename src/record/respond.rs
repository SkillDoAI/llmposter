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
/// request-side `DefaultBodyLimit` (16 MB, server.rs). The cap bounds
/// only the RECORDING: exceeding it abandons the capture (buffer freed)
/// while the relay to a connected client continues untouched. It also
/// bounds the post-client-disconnect salvage drain against an endlessly
/// streaming upstream.
const SALVAGE_BUFFER_CAP: usize = 16 * 1024 * 1024;

/// `true` when growing the capture buffer by `chunk_len` would blow
/// [`SALVAGE_BUFFER_CAP`] — the tee abandons the recording (never the
/// relay) at that point.
fn exceeds_capture_cap(buf_len: usize, chunk_len: usize) -> bool {
    buf_len.saturating_add(chunk_len) > SALVAGE_BUFFER_CAP
}

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

    // reqwest and axum share the `http` crate, so the upstream's typed
    // status and header values relay verbatim — no fallible re-parse.
    let status_code = upstream.status();
    let status = status_code.as_u16();
    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| axum::http::HeaderValue::from_static("application/json"));
    // `HeaderName::as_str()` is always lowercase, so exact names and
    // prefix matches below are already case-insensitive.
    let relay_headers: Vec<(axum::http::HeaderName, axum::http::HeaderValue)> = upstream
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            let lower = name.as_str();
            let keep = RELAY_RESPONSE_HEADERS.contains(&lower)
                || RELAY_RESPONSE_HEADER_PREFIXES
                    .iter()
                    .any(|prefix| lower.starts_with(prefix));
            keep.then(|| (name.clone(), value.clone()))
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
            let mut recording = true;
            let mut client_connected = true;
            while let Some(item) = tokio_stream::StreamExt::next(&mut stream).await {
                match item {
                    Ok(chunk) => {
                        if recording {
                            if exceeds_capture_cap(buf.len(), chunk.len()) {
                                // Abandon the RECORDING only — the relay
                                // to a connected client continues below.
                                eprintln!(
                                    "[llmposter] record: POST {} — stream exceeded the \
                                     {} MiB capture cap — passed through, not recorded",
                                    task_path,
                                    SALVAGE_BUFFER_CAP / (1024 * 1024)
                                );
                                recording = false;
                                buf.clear();
                                buf.shrink_to_fit();
                            } else {
                                buf.extend_from_slice(&chunk);
                            }
                        }
                        if client_connected && tx.send(Ok(chunk)).await.is_err() {
                            // Client hung up — keep draining so the
                            // recording can still complete.
                            client_connected = false;
                        }
                        if !client_connected && !recording {
                            break; // nothing left to relay OR record
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
            if clean && recording {
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
        let mut resp = relay_response(
            status_code,
            content_type,
            &relay_headers,
            Body::from_stream(stream),
        );
        resp.headers_mut().insert(
            header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        );
        return resp;
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

    relay_response(status_code, content_type, &relay_headers, Body::from(bytes))
}

/// Infallibly assemble the relayed response. Every input is a typed
/// `http` value that came off the parsed upstream response, so there is
/// no fallible builder step — and no unreachable error arm to maintain.
fn relay_response(
    status: StatusCode,
    content_type: axum::http::HeaderValue,
    relay_headers: &[(axum::http::HeaderName, axum::http::HeaderValue)],
    body: Body,
) -> Response<Body> {
    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    for (name, value) in relay_headers {
        resp.headers_mut().append(name.clone(), value.clone());
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_trip_capture_cap_only_when_buffer_would_exceed_it() {
        // At the cap exactly is fine; one byte past it trips.
        assert!(!exceeds_capture_cap(0, SALVAGE_BUFFER_CAP));
        assert!(!exceeds_capture_cap(SALVAGE_BUFFER_CAP - 5, 5));
        assert!(exceeds_capture_cap(SALVAGE_BUFFER_CAP - 5, 6));
        assert!(exceeds_capture_cap(SALVAGE_BUFFER_CAP, 1));
        // Saturating add — no overflow panic on absurd lengths.
        assert!(exceeds_capture_cap(usize::MAX, usize::MAX));
    }
}
