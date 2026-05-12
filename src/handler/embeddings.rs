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

    // Fixture matching — use input as user_message for matching.
    let (fixture, fixture_count) = {
        let fixtures = state.fixtures.read().unwrap_or_else(|e| e.into_inner());
        let mut scenarios = state.scenarios.write().unwrap_or_else(|e| e.into_inner());
        let count = fixtures.len();

        let ctx = crate::fixture::MatchContext::new(
            &input,
            Some(&model),
            Some(Provider::OpenAI),
            Some(&scenarios),
            &req_headers,
            &json_body,
        );
        let matched = fixtures.find_match(|f| crate::fixture::fixture_matches(f, &ctx));

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

        let (outcome, status_code) = if let Some(ref f) = arc_fixture {
            let status = f.error.as_ref().map(|e| e.status).unwrap_or(200);
            (RequestOutcome::Matched, status)
        } else {
            (RequestOutcome::NoFixtureMatch, 404)
        };
        crate::handler::push_captured(
            &state,
            "POST",
            "/v1/embeddings",
            body,
            outcome,
            scenario_name,
            status_code,
        );
        (arc_fixture, count)
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
        // No fixture matched — return 404.
        let msg = format!(
            "No fixture matched for model='{}' ({} fixture{} checked)",
            model,
            fixture_count,
            if fixture_count == 1 { "" } else { "s" }
        );
        let mut response = (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            crate::failure::build_error_body(404, &msg),
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
}
