//! Embeddings handler (`POST /v1/embeddings`).
//!
//! Standalone handler with fixture matching — does not use the full
//! `ProviderHandler` trait since embeddings have no streaming, tool calls,
//! or refusals.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::format::{estimate_tokens, Provider};
use crate::server::{AppState, RequestOutcome};

/// Generate a deterministic fake embedding from the input string.
/// Uses FNV-1a hash as PRNG seed, produces `dims` floats, L2-normalized.
fn generate_fake_embedding(input: &str, dims: usize) -> Vec<f64> {
    let mut seed: u64 = 2166136261;
    for b in input.bytes() {
        seed ^= b as u64;
        seed = seed.wrapping_mul(1099511628211);
    }
    let mut values: Vec<f64> = (0..dims)
        .map(|i| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407_u64.wrapping_add(i as u64));
            ((seed >> 11) as f64 / ((1u64 << 53) as f64)) * 2.0 - 1.0
        })
        .collect();
    let norm = values.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm > 0.0 {
        for v in &mut values {
            *v /= norm;
        }
    }
    values
}

pub async fn handle(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    body: String,
) -> Response<Body> {
    let json_body: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            crate::handler::capture_non_matched(
                &state,
                "POST",
                "/v1/embeddings",
                &body,
                RequestOutcome::BadRequest,
            );
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                crate::failure::build_error_body(400, "Invalid JSON"),
            )
                .into_response();
        }
    };

    let model = match json_body
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(m) => m.to_string(),
        None => {
            crate::handler::capture_non_matched(
                &state,
                "POST",
                "/v1/embeddings",
                &body,
                RequestOutcome::BadRequest,
            );
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                crate::failure::build_error_body(400, "Missing or empty 'model' field"),
            )
                .into_response();
        }
    };

    // Extract input — string or array of strings, joined for matching.
    // Reject array-of-non-string (e.g. token-ID arrays) since they'd silently
    // match against "" and produce a wrong fixture.
    let input = if let Some(s) = json_body.get("input").and_then(|v| v.as_str()) {
        s.to_string()
    } else if let Some(arr) = json_body.get("input").and_then(|v| v.as_array()) {
        if !arr.iter().all(|v| v.is_string()) {
            crate::handler::capture_non_matched(
                &state,
                "POST",
                "/v1/embeddings",
                &body,
                RequestOutcome::BadRequest,
            );
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                crate::failure::build_error_body(
                    400,
                    "'input' array must contain strings (token-ID arrays not supported)",
                ),
            )
                .into_response();
        }
        arr.iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        crate::handler::capture_non_matched(
            &state,
            "POST",
            "/v1/embeddings",
            &body,
            RequestOutcome::BadRequest,
        );
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, "application/json")],
            crate::failure::build_error_body(400, "Missing or invalid 'input' field"),
        )
            .into_response();
    };

    // Lowercase request headers for matching.
    let req_headers = super::header_map_to_lowercase(&headers);

    // --- VCR record mode: bypass fixtures entirely, forward everything. ---
    #[cfg(feature = "record")]
    if let Some(recorder) = crate::record::Recorder::active(&state, crate::record::VcrMode::Record)
    {
        let mut response = crate::record::record_and_respond_embeddings(
            recorder,
            &state,
            &req_headers,
            body,
            &model,
            &input,
        )
        .await;
        response.extensions_mut().insert(Provider::OpenAI);
        return response;
    }
    // --- VCR record-on-miss: Some only when configured. On a miss the
    // request body is threaded OUT of the lock block below (no clone)
    // and handed to the recorder. ---
    #[cfg(feature = "record")]
    let record_on_miss =
        crate::record::Recorder::active(&state, crate::record::VcrMode::RecordOnMiss);

    // Fixture matching — use input as user_message for matching.
    // `record_body` carries request-body ownership out of the lock block
    // when the recorder takes over (always None with the record feature
    // off — `recorder_takes_over` is const false there).
    #[cfg_attr(not(feature = "record"), allow(unused_variables))]
    let (fixture, fixture_count, nearest_hint, record_body) = {
        let fixtures = state.fixtures.read().unwrap_or_else(|e| e.into_inner());
        let mut scenarios = state.scenarios.write().unwrap_or_else(|e| e.into_inner());
        let count = fixtures.len();

        let matched = {
            let ctx = crate::fixture::MatchContext::new(
                &input,
                Some(&model),
                Some(Provider::OpenAI),
                Some(&scenarios),
                &req_headers,
                &json_body,
            );
            fixtures
                .find_match(|f| crate::fixture::fixture_matches(f, &ctx))
                .cloned()
        };

        let (arc_fixture, scenario_name) = if let Some(f) = matched {
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
        };

        // Nearest-match diagnostics — only computed when enabled and no match.
        // Built after scenario update so `scenarios` is no longer mutably borrowed.
        let hint = if arc_fixture.is_none() && state.diagnostics {
            let ctx = crate::fixture::MatchContext::new(
                &input,
                Some(&model),
                Some(Provider::OpenAI),
                Some(&scenarios),
                &req_headers,
                &json_body,
            );
            crate::fixture::evaluate_nearest_match(&fixtures, &ctx)
        } else {
            None
        };

        let (outcome, status_code) = if let Some(ref f) = arc_fixture {
            let status = f.error.as_ref().map(|e| e.status).unwrap_or(200);
            (RequestOutcome::Matched, status)
        } else {
            (RequestOutcome::NoFixtureMatch, 404)
        };
        // When record-on-miss will take over a missed request, skip the
        // capture push here — the recorder pushes later with the REAL
        // upstream status, and a NoFixtureMatch/404 entry would mislead.
        // Mirrored in handler/mod.rs — keep in sync.
        #[cfg(feature = "record")]
        let recorder_takes_over = arc_fixture.is_none() && record_on_miss.is_some();
        #[cfg(not(feature = "record"))]
        let recorder_takes_over = false;
        let record_body = if recorder_takes_over {
            Some(body) // ownership moves to the record-on-miss arm below
        } else {
            crate::handler::push_captured(
                &state,
                "POST",
                "/v1/embeddings",
                body,
                outcome,
                scenario_name,
                status_code,
            );
            None
        };
        (arc_fixture, count, hint, record_body)
    };

    // Determine embedding vector.
    let embedding = if let Some(ref f) = fixture {
        if let Some(ref err) = f.error {
            let status =
                StatusCode::from_u16(err.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let err_body = crate::failure::build_error_body(status.as_u16(), &err.message);
            let mut builder = axum::http::Response::builder().status(status);
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
            let mut response = match builder.body(Body::from(err_body)) {
                Ok(resp) => resp.into_response(),
                Err(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(header::CONTENT_TYPE, "application/json")],
                    crate::failure::build_error_body(
                        500,
                        "Fixture contains invalid header name or value",
                    ),
                )
                    .into_response(),
            };
            response.extensions_mut().insert(Provider::OpenAI);
            return response;
        }
        f.response
            .as_ref()
            .and_then(|r| r.embedding.clone())
            .unwrap_or_else(|| {
                let dims = json_body
                    .get("dimensions")
                    .and_then(|v| v.as_u64())
                    .filter(|n| (1..=8192).contains(n))
                    .map(|n| n as usize)
                    .unwrap_or(1536);
                generate_fake_embedding(&input, dims)
            })
    } else {
        // No fixture matched — record-on-miss forwards upstream instead.
        // Mirrored in handler/mod.rs — keep in sync.
        #[cfg(feature = "record")]
        if let (Some(recorder), Some(rec_body)) = (record_on_miss, record_body) {
            let mut response = crate::record::record_and_respond_embeddings(
                recorder,
                &state,
                &req_headers,
                rec_body,
                &model,
                &input,
            )
            .await;
            response.extensions_mut().insert(Provider::OpenAI);
            return response;
        }
        // No fixture matched — return 404.
        let msg = format!(
            "No fixture matched for model='{}' ({} fixture{} checked)",
            model,
            fixture_count,
            if fixture_count == 1 { "" } else { "s" }
        );
        let body_str = if let Some(hint) = nearest_hint {
            let fields: Vec<serde_json::Value> = hint
                .fields
                .iter()
                .map(|f| serde_json::json!({"field": f.field, "passed": f.passed}))
                .collect();
            serde_json::json!({
                "error": {
                    "message": msg,
                    "type": "not_found_error",
                    "param": null,
                    "code": "not_found",
                    "nearest_match": {
                        "fixture_index": hint.fixture_index,
                        "pass_count": hint.pass_count,
                        "total_fields": hint.total_fields,
                        "summary": hint.summary,
                        "fields": fields
                    }
                }
            })
            .to_string()
        } else {
            crate::failure::build_error_body(404, &msg)
        };
        let mut response = (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            body_str,
        )
            .into_response();
        response.extensions_mut().insert(Provider::OpenAI);
        return response;
    };

    let prompt_tokens = estimate_tokens(&input);
    let resp = serde_json::json!({
        "object": "list",
        "data": [{
            "object": "embedding",
            "embedding": embedding,
            "index": 0
        }],
        "model": model,
        "usage": {
            "prompt_tokens": prompt_tokens,
            "total_tokens": prompt_tokens
        }
    });

    let mut response = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        resp.to_string(),
    )
        .into_response();
    response.extensions_mut().insert(Provider::OpenAI);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_generate_deterministic_embedding() {
        let e1 = generate_fake_embedding("hello", 10);
        let e2 = generate_fake_embedding("hello", 10);
        assert_eq!(e1, e2);
        assert_eq!(e1.len(), 10);
        // L2 norm should be ~1.0
        let norm: f64 = e1.iter().map(|x| x * x).sum::<f64>().sqrt();
        assert!((norm - 1.0).abs() < 1e-10);
    }

    #[test]
    fn should_generate_different_embeddings_for_different_inputs() {
        let e1 = generate_fake_embedding("hello", 10);
        let e2 = generate_fake_embedding("world", 10);
        assert_ne!(e1, e2);
    }

    /// Covers the `if norm > 0.0` false branch: when dims == 0 the values vec is
    /// empty, so norm == 0.0 and the normalization loop is skipped.
    #[test]
    fn should_return_empty_vec_for_zero_dims() {
        let v = generate_fake_embedding("any input", 0);
        assert!(v.is_empty());
    }

    /// Covers the `Err(_)` arm of `builder.body()` in the error-fixture path.
    /// `with_error_headers` validates headers at construction time, so we bypass
    /// validation by constructing `Fixture` + `FixtureError` directly and
    /// inserting a header name containing a null byte — axum's builder sets an
    /// error state when `.header()` receives an invalid name, and the subsequent
    /// `.body()` call returns `Err`, exercising the 500 fallback path.
    #[tokio::test]
    async fn should_return_500_when_error_fixture_has_invalid_header() {
        use std::collections::HashMap;
        use std::sync::Arc;

        use crate::fixture::{Fixture, FixtureError};
        use crate::format::IdGenerator;
        use crate::server::{AppState, FixtureSet};

        let fixture = Fixture {
            error: Some(FixtureError {
                status: 429,
                message: "rate limited".to_string(),
                // null byte makes the header name invalid for axum's builder
                headers: HashMap::from([("bad\x00name".to_string(), "v".to_string())]),
            }),
            ..Fixture::new()
        };

        let state = Arc::new(AppState {
            fixtures: std::sync::RwLock::new(FixtureSet::new(vec![Arc::new(fixture)])),
            id_gen: IdGenerator::new(),
            verbose: false,
            request_counter: Default::default(),
            chaos_counter: Default::default(),
            capture_counter: Default::default(),
            moderation_counter: Default::default(),
            auth: None,
            scenarios: Default::default(),
            captured_requests: Default::default(),
            capture_capacity: None,
            explicit_models: None,
            diagnostics: false,
            boot_instant: std::time::Instant::now(),
            boot_epoch_ms: 0,
            #[cfg(feature = "ui")]
            ui_tx: None,
            #[cfg(feature = "record")]
            recorder: None,
            #[cfg(feature = "ui")]
            ui_require_auth: false,
        });

        let body = r#"{"model":"text-embedding-ada-002","input":"test"}"#;
        let resp = super::handle(
            axum::extract::State(state),
            axum::http::HeaderMap::new(),
            body.to_string(),
        )
        .await;

        assert_eq!(resp.status(), axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }
}
