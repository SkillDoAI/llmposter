use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Response, StatusCode};
use axum::response::IntoResponse;
use tokio::time::sleep;

use crate::failure;
use crate::fixture::match_fixture;
use crate::format::responses;
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

    let (model, user_message) = match responses::extract_request_info(&json_body) {
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
        Some(Provider::Responses),
    ) {
        Some(f) => f,
        None => {
            if state.verbose {
                eprintln!(
                    "[llmposter] POST /v1/responses → no match (model='{}', msg='{:.50}')",
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
        eprintln!("[llmposter] POST /v1/responses → fixture matched");
    }

    // Handle error fixtures
    if let Some(ref err) = fixture.error {
        let status = StatusCode::from_u16(err.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        return (
            status,
            [(header::CONTENT_TYPE, "application/json")],
            failure::build_error_body(err.status, &err.message),
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

        // For tool calls in streaming, send as SSE events via mpsc channel
        if let Some(ref tool_calls) = response.tool_calls {
            let tc_pairs: Vec<(&str, serde_json::Value)> = tool_calls
                .iter()
                .map(|tc| (tc.name.as_str(), tc.arguments.clone()))
                .collect();
            let resp = responses::build_tool_call_response(
                &state.id_gen,
                &model,
                &tc_pairs,
                &user_message,
            );
            let mut resp_json = serde_json::to_value(&resp).unwrap();
            // Add type fields
            let mut completed_json = resp_json.clone();
            completed_json["type"] = serde_json::json!("response.completed");
            let completed_str = serde_json::to_string(&completed_json).unwrap();
            resp_json["type"] = serde_json::json!("response.created");
            resp_json["status"] = serde_json::json!("in_progress");
            resp_json["output"] = serde_json::json!([]);
            resp_json["usage"]["output_tokens"] = serde_json::json!(0);
            resp_json["usage"]["total_tokens"] = resp_json["usage"]["input_tokens"].clone();
            let created_str = serde_json::to_string(&resp_json).unwrap();

            // Build full lifecycle event sequence for tool-call streaming
            let mut frames = vec![format!(
                "event: response.created\ndata: {}\n\n",
                created_str
            )];
            // Add output_item.added (empty initial) + output_item.done (full) for each tool call
            for (i, item) in resp.output.iter().enumerate() {
                // added event: item with initial state (no arguments, in_progress)
                let mut initial_item = item.clone();
                if let Some(obj) = initial_item.as_object_mut() {
                    obj.remove("arguments");
                    obj.insert("status".to_string(), serde_json::json!("in_progress"));
                }
                frames.push(format!(
                    "event: response.output_item.added\ndata: {}\n\n",
                    serde_json::json!({
                        "type": "response.output_item.added",
                        "output_index": i,
                        "item": initial_item,
                    })
                ));
                // function_call_arguments.delta — send full arguments in one delta
                let args = item
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::json!(""));
                let item_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                frames.push(format!(
                    "event: response.function_call_arguments.delta\ndata: {}\n\n",
                    serde_json::json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": item_id,
                        "call_id": call_id,
                        "output_index": i,
                        "delta": args,
                    })
                ));
                // function_call_arguments.done
                frames.push(format!(
                    "event: response.function_call_arguments.done\ndata: {}\n\n",
                    serde_json::json!({
                        "type": "response.function_call_arguments.done",
                        "item_id": item_id,
                        "call_id": call_id,
                        "output_index": i,
                        "arguments": args,
                    })
                ));
                // done event: full item
                frames.push(format!(
                    "event: response.output_item.done\ndata: {}\n\n",
                    serde_json::json!({
                        "type": "response.output_item.done",
                        "output_index": i,
                        "item": item,
                    })
                ));
            }
            frames.push(format!(
                "event: response.completed\ndata: {}\n\n",
                completed_str
            ));
            frames.push("event: response.done\ndata: {\"type\":\"response.done\"}\n\n".to_string());

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

        let events = responses::build_stream_events(
            &state.id_gen,
            &model,
            content,
            chunk_size,
            &user_message,
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
            let resp = responses::build_tool_call_response(
                &state.id_gen,
                &model,
                &tc_pairs,
                &user_message,
            );
            let json = serde_json::to_string(&resp).unwrap();
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                json,
            )
                .into_response();
        }

        let resp = responses::build_response(&state.id_gen, &model, content, &user_message);
        let json = serde_json::to_string(&resp).unwrap();
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            json,
        )
            .into_response()
    }
}
