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
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
            // Tell the client it missed events so the UI can show a
            // visual gap instead of silently pretending everything is
            // fine. The JS side listens for the "lagged" event type.
            let json = serde_json::json!({ "lagged": n }).to_string();
            Some(Ok::<_, std::convert::Infallible>(
                Event::default().event("lagged").data(json),
            ))
        }
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
        .map(|req| {
            let elapsed_ms = req
                .timestamp
                .checked_duration_since(boot_instant)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            UiEvent {
                id: req.capture_id,
                timestamp_ms: boot_epoch_ms + elapsed_ms,
                method: req.method.clone(),
                path: req.path.clone(),
                provider: provider_from_path_str(&req.path),
                outcome: outcome_to_str(&req.outcome),
                matched_scenario: req.matched_scenario.clone(),
                status_code: req.status_code,
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
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({"error": format!("Invalid JSON: {}", e)})),
            )
                .into_response();
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
    // Acquire locks in the same order as handle_request (fixtures →
    // scenarios) to avoid a 3-thread deadlock with concurrent
    // hot-reload + handler + debug requests.
    let fixtures = state.fixtures.read().unwrap_or_else(|e| e.into_inner());
    let scenarios = state.scenarios.read().unwrap_or_else(|e| e.into_inner());
    let ctx = crate::fixture::MatchContext::new(
        &user_message,
        if model.is_empty() { None } else { Some(model) },
        provider,
        Some(&scenarios),
        &empty_headers,
        &json_body,
    );
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
    .into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn outcome_to_str(outcome: &crate::server::RequestOutcome) -> String {
    outcome.label().to_string()
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

    // headers — the debugger form doesn't supply headers, so these
    // checks always fail when ctx.headers is empty. Flag this in the
    // field name so the user knows it's a debugger limitation.
    let headers_simulated = !ctx.headers.is_empty();
    for (name, pattern) in &m.headers {
        let actual = ctx.headers.get(name).map(|v| v.as_str());
        let passed = actual.is_some_and(|v| string_matches_check(pattern, v));
        if !passed {
            all_pass = false;
        }
        let field_label = if headers_simulated {
            format!("headers.{}", name)
        } else {
            format!("headers.{} (not simulated in debugger)", name)
        };
        checks.push(FieldCheck {
            field: field_label,
            expected: string_match_display(pattern),
            actual: actual.map(|s| s.to_string()),
            passed,
        });
    }

    // system_prompt — extract and check independently
    if let Some(ref sp) = m.system_prompt {
        let actual_sp = ctx.system_prompt().map(|s| s.to_string());
        let passed = actual_sp
            .as_deref()
            .is_some_and(|text| string_matches_check(sp, text));
        if !passed {
            all_pass = false;
        }
        checks.push(FieldCheck {
            field: "system_prompt".into(),
            expected: string_match_display(sp),
            actual: actual_sp.map(|s| truncate(&s, 80)),
            passed,
        });
    }

    // temperature — extract provider-aware and check independently
    if let Some(ref tm) = m.temperature {
        let temp = crate::fixture::extract_temperature_for_debug(ctx.body, ctx.provider);
        let passed = temp.is_some_and(|t| match tm {
            crate::fixture::F64Match::Exact(target) => t == *target,
            crate::fixture::F64Match::Range(r) => {
                r.min.is_none_or(|min| t >= min) && r.max.is_none_or(|max| t <= max)
            }
        });
        if !passed {
            all_pass = false;
        }
        checks.push(FieldCheck {
            field: "temperature".into(),
            expected: format!("{:?}", tm),
            actual: temp.map(|t| format!("{}", t)),
            passed,
        });
    }

    // metadata — coerce number/bool to string, matching fixture_matches logic
    if !m.metadata.is_empty() {
        let metadata = ctx.body.get("metadata").and_then(|v| v.as_object());
        for (key, pattern) in &m.metadata {
            let value_str: Option<std::borrow::Cow<str>> =
                metadata.and_then(|m| m.get(key)).and_then(|v| match v {
                    serde_json::Value::String(s) => Some(std::borrow::Cow::Borrowed(s.as_str())),
                    serde_json::Value::Number(n) => Some(std::borrow::Cow::Owned(n.to_string())),
                    serde_json::Value::Bool(b) => Some(std::borrow::Cow::Owned(b.to_string())),
                    _ => None,
                });
            let passed = value_str
                .as_deref()
                .is_some_and(|v| string_matches_check(pattern, v));
            if !passed {
                all_pass = false;
            }
            checks.push(FieldCheck {
                field: format!("metadata.{}", key),
                expected: string_match_display(pattern),
                actual: value_str.map(|s| s.into_owned()),
                passed,
            });
        }
    }

    // tool_schema — extract tool names and check independently
    if let Some(ref ts) = m.tool_schema {
        let names = ctx.tool_names();
        let passed = names.iter().any(|name| string_matches_check(ts, name));
        if !passed {
            all_pass = false;
        }
        checks.push(FieldCheck {
            field: "tool_schema".into(),
            expected: string_match_display(ts),
            actual: if names.is_empty() {
                None
            } else {
                Some(names.join(", "))
            },
            passed,
        });
    }

    // body_jsonpath — evaluate independently via jsonpath
    if let Some(ref path_str) = m.body_jsonpath {
        #[cfg(feature = "jsonpath")]
        let passed = {
            match jsonpath_rust::parser::parse_json_path(path_str) {
                Ok(query) => match jsonpath_rust::query::js_path_process(&query, ctx.body) {
                    Ok(matches) => {
                        !matches.is_empty() && !matches.into_iter().all(|q| q.val().is_null())
                    }
                    Err(_) => false,
                },
                Err(_) => false,
            }
        };
        #[cfg(not(feature = "jsonpath"))]
        let passed = false;
        if !passed {
            all_pass = false;
        }
        checks.push(FieldCheck {
            field: "body_jsonpath".into(),
            expected: path_str.clone(),
            actual: Some("(evaluated)".into()),
            passed,
        });
    }

    (all_pass, checks)
}

fn string_matches_check(pattern: &crate::fixture::StringMatch, haystack: &str) -> bool {
    crate::fixture::string_matches(pattern, haystack)
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
        // Walk backwards from `max` to find a char boundary (avoids
        // panicking on multi-byte UTF-8).
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}
