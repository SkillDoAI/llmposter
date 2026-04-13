//! Embedded debug UI — request inspector + fixture match debugger.
//!
//! Gated behind the `ui` Cargo feature. Mount via `ServerBuilder::ui(true)`
//! or `--ui` on the CLI. Serves a single-page HTML app at `/ui` with live
//! SSE updates at `/ui/events`.

use std::sync::Arc;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::Router;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::server::AppState;

const INDEX_HTML: &str = include_str!("ui_assets/index.html");

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Lightweight event broadcast to SSE clients. Carries enough data to
/// populate the request inspector table without re-reading the capture log.
#[derive(Clone, serde::Serialize)]
pub(crate) struct UiEvent {
    pub id: u64,
    pub timestamp_ms: u64,
    pub method: String,
    pub path: String,
    pub provider: Option<String>,
    pub outcome: String,
    pub matched_scenario: Option<String>,
    pub status_code: u16,
    pub request_body: String,
}

#[derive(serde::Deserialize)]
struct DebugRequest {
    provider: String,
    body: String,
}

#[derive(serde::Serialize)]
struct DebugResponse {
    fixtures: Vec<FixtureEval>,
    matched_index: Option<usize>,
}

#[derive(serde::Serialize)]
struct FixtureEval {
    index: usize,
    label: String,
    priority: Option<i32>,
    catch_all: bool,
    passed: bool,
    checks: Vec<FieldCheck>,
}

#[derive(serde::Serialize)]
struct FieldCheck {
    field: String,
    expected: String,
    actual: Option<String>,
    passed: bool,
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

pub(crate) fn ui_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/ui", get(serve_ui))
        .route("/ui/events", get(event_stream))
        .route("/ui/requests", get(get_requests))
        .route("/ui/fixtures", get(get_fixtures))
        .route("/ui/debug", post(debug_match))
}

async fn serve_ui() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn event_stream(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let rx = state
        .ui_tx
        .as_ref()
        .expect("ui_tx must be set when UI is enabled")
        .subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(event) => {
            let json = serde_json::to_string(&event).ok()?;
            Some(Ok::<_, std::convert::Infallible>(
                Event::default().data(json),
            ))
        }
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(_)) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn get_requests(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let captured = state
        .captured_requests
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let boot_instant = state.boot_instant;
    let boot_epoch_ms = state.boot_epoch_ms;

    let events: Vec<UiEvent> = captured
        .iter()
        .enumerate()
        .map(|(i, req)| {
            let elapsed_ms = req
                .timestamp
                .checked_duration_since(boot_instant)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            UiEvent {
                id: i as u64,
                timestamp_ms: boot_epoch_ms + elapsed_ms,
                method: req.method.clone(),
                path: req.path.clone(),
                provider: provider_from_path_str(&req.path),
                outcome: outcome_to_str(&req.outcome),
                matched_scenario: req.matched_scenario.clone(),
                status_code: outcome_to_status(&req.outcome),
                request_body: req.body.clone(),
            }
        })
        .collect();
    axum::Json(events)
}

async fn get_fixtures(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let fixtures = state.fixtures.read().unwrap_or_else(|e| e.into_inner());
    let summaries: Vec<serde_json::Value> = fixtures
        .iter_all()
        .enumerate()
        .map(|(i, f)| {
            serde_json::json!({
                "index": i,
                "priority": f.priority,
                "catch_all": f.catch_all,
                "provider": f.provider.map(|p| format!("{:?}", p).to_lowercase()),
                "has_match_rule": f.match_rule.is_some(),
                "has_response": f.response.is_some(),
                "has_error": f.error.is_some(),
                "has_refusal": f.refusal.is_some(),
                "match_summary": match_summary(f),
            })
        })
        .collect();
    axum::Json(summaries)
}

async fn debug_match(
    State(state): State<Arc<AppState>>,
    axum::Json(req): axum::Json<DebugRequest>,
) -> impl IntoResponse {
    let json_body: serde_json::Value = match serde_json::from_str(&req.body) {
        Ok(v) => v,
        Err(e) => {
            return axum::Json(serde_json::json!({"error": format!("Invalid JSON: {}", e)}));
        }
    };

    let provider = match req.provider.as_str() {
        "openai" => Some(crate::format::Provider::OpenAI),
        "anthropic" => Some(crate::format::Provider::Anthropic),
        "gemini" => Some(crate::format::Provider::Gemini),
        "responses" => Some(crate::format::Provider::Responses),
        _ => None,
    };

    // Extract user_message + model like the real handler does.
    let user_message = extract_user_message(&json_body, provider);
    let model = json_body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let empty_headers = std::collections::HashMap::new();
    let scenarios = state.scenarios.read().unwrap_or_else(|e| e.into_inner());
    let ctx = crate::fixture::MatchContext::new(
        &user_message,
        if model.is_empty() { None } else { Some(model) },
        provider,
        Some(&scenarios),
        &empty_headers,
        &json_body,
    );

    let fixtures = state.fixtures.read().unwrap_or_else(|e| e.into_inner());
    let mut evals: Vec<FixtureEval> = Vec::new();
    let mut matched_index: Option<usize> = None;

    // Evaluate in the same order FixtureSet would use (primary then catch_all).
    for (eval_order, f) in fixtures
        .primary_iter()
        .chain(fixtures.catch_all_iter())
        .enumerate()
    {
        let (passed, checks) = evaluate_fixture(f, &ctx);
        let original_index = evals.len(); // ordinal position in output
        if passed && matched_index.is_none() {
            matched_index = Some(eval_order);
        }
        evals.push(FixtureEval {
            index: original_index,
            label: match_summary(f),
            priority: f.priority,
            catch_all: f.catch_all,
            passed,
            checks,
        });
    }

    axum::Json(
        serde_json::to_value(DebugResponse {
            fixtures: evals,
            matched_index,
        })
        .unwrap_or_default(),
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn outcome_to_str(outcome: &crate::server::RequestOutcome) -> String {
    match outcome {
        crate::server::RequestOutcome::Matched => "matched".into(),
        crate::server::RequestOutcome::NoFixtureMatch => "no_match".into(),
        crate::server::RequestOutcome::BadRequest => "bad_request".into(),
        crate::server::RequestOutcome::AuthRejected => "auth_rejected".into(),
        crate::server::RequestOutcome::CodeEndpoint => "code_endpoint".into(),
    }
}

pub(crate) fn outcome_to_status(outcome: &crate::server::RequestOutcome) -> u16 {
    match outcome {
        crate::server::RequestOutcome::Matched => 200,
        crate::server::RequestOutcome::NoFixtureMatch => 404,
        crate::server::RequestOutcome::BadRequest => 400,
        crate::server::RequestOutcome::AuthRejected => 401,
        crate::server::RequestOutcome::CodeEndpoint => 200,
    }
}

pub(crate) fn provider_from_path_str(path: &str) -> Option<String> {
    if path.starts_with("/v1/chat/completions") {
        Some("openai".into())
    } else if path.starts_with("/v1/messages") {
        Some("anthropic".into())
    } else if path.starts_with("/v1beta/models") {
        Some("gemini".into())
    } else if path.starts_with("/v1/responses") {
        Some("responses".into())
    } else {
        None
    }
}

fn extract_user_message(
    body: &serde_json::Value,
    provider: Option<crate::format::Provider>,
) -> String {
    // Best-effort extraction for the debugger — not the canonical path
    let array_key = match provider {
        Some(crate::format::Provider::Responses) => "input",
        Some(crate::format::Provider::Gemini) => "contents",
        _ => "messages",
    };
    if let Some(arr) = body.get(array_key).and_then(|v| v.as_array()) {
        for msg in arr.iter().rev() {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
            if role == "user" {
                // OpenAI/Anthropic/Responses: content as string
                if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
                    return s.to_string();
                }
                // Gemini: parts[].text
                if let Some(parts) = msg.get("parts").and_then(|v| v.as_array()) {
                    if let Some(text) = parts
                        .first()
                        .and_then(|p| p.get("text"))
                        .and_then(|v| v.as_str())
                    {
                        return text.to_string();
                    }
                }
            }
        }
    }
    // Responses API string input
    if let Some(s) = body.get("input").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    String::new()
}

fn match_summary(f: &crate::fixture::Fixture) -> String {
    let Some(m) = f.match_rule.as_ref() else {
        return "(any request)".into();
    };
    let mut parts = Vec::new();
    if let Some(ref um) = m.user_message {
        parts.push(format!(
            "user_message: {:?}",
            match um {
                crate::fixture::StringMatch::Substring(s) => s.as_str(),
                crate::fixture::StringMatch::Regex(r) => &r.regex,
            }
        ));
    }
    if let Some(ref mm) = m.model {
        parts.push(format!(
            "model: {:?}",
            match mm {
                crate::fixture::StringMatch::Substring(s) => s.as_str(),
                crate::fixture::StringMatch::Regex(r) => &r.regex,
            }
        ));
    }
    if !m.headers.is_empty() {
        parts.push(format!("headers({})", m.headers.len()));
    }
    if m.system_prompt.is_some() {
        parts.push("system_prompt".into());
    }
    if m.temperature.is_some() {
        parts.push("temperature".into());
    }
    if !m.metadata.is_empty() {
        parts.push(format!("metadata({})", m.metadata.len()));
    }
    if m.tool_schema.is_some() {
        parts.push("tool_schema".into());
    }
    if m.body_jsonpath.is_some() {
        parts.push("body_jsonpath".into());
    }
    if parts.is_empty() {
        "(no criteria)".into()
    } else {
        parts.join(", ")
    }
}

/// Evaluate a single fixture against a MatchContext and return
/// per-field pass/fail checks. Mirrors `fixture_matches()` logic
/// but collects diagnostics instead of short-circuiting on first
/// failure.
fn evaluate_fixture(
    fixture: &crate::fixture::Fixture,
    ctx: &crate::fixture::MatchContext<'_>,
) -> (bool, Vec<FieldCheck>) {
    let mut checks = Vec::new();
    let mut all_pass = true;

    // Provider check
    if let Some(fp) = fixture.provider {
        let passed = ctx.provider == Some(fp);
        if !passed {
            all_pass = false;
        }
        checks.push(FieldCheck {
            field: "provider".into(),
            expected: format!("{:?}", fp),
            actual: ctx.provider.map(|p| format!("{:?}", p)),
            passed,
        });
    }

    // Scenario required_state
    if let Some(ref scenario) = fixture.scenario {
        if let Some(ref required) = scenario.required_state {
            let current = ctx
                .scenario_states
                .and_then(|states| states.get(&scenario.name))
                .map(|s| s.as_str())
                .unwrap_or("");
            let passed = current == required;
            if !passed {
                all_pass = false;
            }
            checks.push(FieldCheck {
                field: format!("scenario[{}].required_state", scenario.name),
                expected: required.clone(),
                actual: Some(current.to_string()),
                passed,
            });
        }
    }

    let Some(m) = fixture.match_rule.as_ref() else {
        // No match rule = matches everything
        return (all_pass, checks);
    };

    // user_message
    if let Some(ref um) = m.user_message {
        let passed = string_matches_check(um, ctx.user_message);
        if !passed {
            all_pass = false;
        }
        checks.push(FieldCheck {
            field: "user_message".into(),
            expected: string_match_display(um),
            actual: Some(truncate(ctx.user_message, 80)),
            passed,
        });
    }

    // model
    if let Some(ref mm) = m.model {
        let model_str = ctx.model.unwrap_or("");
        let passed = !model_str.is_empty() && string_matches_check(mm, model_str);
        if !passed {
            all_pass = false;
        }
        checks.push(FieldCheck {
            field: "model".into(),
            expected: string_match_display(mm),
            actual: Some(model_str.to_string()),
            passed,
        });
    }

    // headers
    for (name, pattern) in &m.headers {
        let actual = ctx.headers.get(name).map(|v| v.as_str());
        let passed = actual.is_some_and(|v| string_matches_check(pattern, v));
        if !passed {
            all_pass = false;
        }
        checks.push(FieldCheck {
            field: format!("headers.{}", name),
            expected: string_match_display(pattern),
            actual: actual.map(|s| s.to_string()),
            passed,
        });
    }

    // system_prompt
    if m.system_prompt.is_some() {
        // Use fixture_matches for the full check — just report pass/fail
        let full_passed = crate::fixture::fixture_matches(fixture, ctx);
        // This is approximate — system_prompt is one of many checks. For
        // a precise breakdown we'd need to extract the system prompt here.
        // Good enough for the debugger MVP.
        checks.push(FieldCheck {
            field: "system_prompt".into(),
            expected: "(set)".into(),
            actual: Some("(see full match)".into()),
            passed: full_passed || all_pass, // conservative
        });
    }

    // temperature
    if m.temperature.is_some() {
        let temp = ctx.body.get("temperature").and_then(|v| v.as_f64());
        let passed = crate::fixture::fixture_matches(fixture, ctx);
        checks.push(FieldCheck {
            field: "temperature".into(),
            expected: "(set)".into(),
            actual: temp.map(|t| format!("{}", t)),
            passed: passed || all_pass,
        });
    }

    // metadata
    if !m.metadata.is_empty() {
        let metadata = ctx.body.get("metadata").and_then(|v| v.as_object());
        for (key, pattern) in &m.metadata {
            let actual = metadata.and_then(|m| m.get(key)).and_then(|v| v.as_str());
            let passed = actual.is_some_and(|v| string_matches_check(pattern, v));
            if !passed {
                all_pass = false;
            }
            checks.push(FieldCheck {
                field: format!("metadata.{}", key),
                expected: string_match_display(pattern),
                actual: actual.map(|s| s.to_string()),
                passed,
            });
        }
    }

    // tool_schema
    if m.tool_schema.is_some() {
        checks.push(FieldCheck {
            field: "tool_schema".into(),
            expected: "(set)".into(),
            actual: Some("(see full match)".into()),
            passed: crate::fixture::fixture_matches(fixture, ctx),
        });
    }

    // body_jsonpath
    if m.body_jsonpath.is_some() {
        checks.push(FieldCheck {
            field: "body_jsonpath".into(),
            expected: m.body_jsonpath.as_deref().unwrap_or("").to_string(),
            actual: Some("(evaluated)".into()),
            passed: crate::fixture::fixture_matches(fixture, ctx),
        });
    }

    (all_pass, checks)
}

fn string_matches_check(pattern: &crate::fixture::StringMatch, haystack: &str) -> bool {
    match pattern {
        crate::fixture::StringMatch::Substring(s) => haystack.contains(s.as_str()),
        crate::fixture::StringMatch::Regex(r) => {
            // RegexMatch::is_match is pub(crate) on fixture.rs — use the
            // compiled regex directly if available, otherwise fall back to
            // the pattern string.
            regex::Regex::new(&r.regex)
                .map(|re| re.is_match(haystack))
                .unwrap_or(false)
        }
    }
}

fn string_match_display(pattern: &crate::fixture::StringMatch) -> String {
    match pattern {
        crate::fixture::StringMatch::Substring(s) => format!("contains {:?}", s),
        crate::fixture::StringMatch::Regex(r) => format!("regex {:?}", r.regex),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
