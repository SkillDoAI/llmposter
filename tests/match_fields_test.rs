//! Integration tests for the richer match fields introduced in
//! v0.4.6: header, system_prompt, temperature, metadata, tool_schema.

use llmposter::{Fixture, ServerBuilder};

async fn post_openai(
    url: &str,
    body: serde_json::Value,
    extra_header: Option<(&str, &str)>,
) -> u16 {
    let client = reqwest::Client::new();
    let mut req = client
        .post(format!("{}/v1/chat/completions", url))
        .json(&body);
    if let Some((name, value)) = extra_header {
        req = req.header(name, value);
    }
    req.send().await.unwrap().status().as_u16()
}

#[tokio::test]
async fn should_match_on_header_value() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_header("x-tenant", "acme")
                .respond_with_content("acme-tenant"),
        )
        .build()
        .await
        .unwrap();

    let body = serde_json::json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "hi"}]
    });

    // Missing header → 404 (no match)
    assert_eq!(post_openai(&server.url(), body.clone(), None).await, 404);

    // Wrong header → 404
    assert_eq!(
        post_openai(&server.url(), body.clone(), Some(("x-tenant", "globex"))).await,
        404
    );

    // Matching header → 200
    assert_eq!(
        post_openai(&server.url(), body, Some(("x-tenant", "acme"))).await,
        200
    );
}

/// A request can carry multiple values under the same header name
/// (e.g. two `Accept` entries). `header_map_to_lowercase` joins them
/// with `, ` so a substring match on any individual value still
/// hits — a fixture asking for `headers: { accept: "application/json" }`
/// must match a request that sent both `text/html` and
/// `application/json` in separate header lines.
#[tokio::test]
async fn should_match_header_with_multiple_values() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_header("accept", "application/json")
                .respond_with_content("multi-accept"),
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .header("accept", "text/html")
        .header("accept", "application/json")
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_match_on_system_prompt_openai() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_system_prompt("You are a pirate")
                .respond_with_content("yarr"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // No system message → no match
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // System message that matches → 200
    let resp: serde_json::Value = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "You are a pirate. Speak like one."},
                {"role": "user", "content": "hello"}
            ]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["choices"][0]["message"]["content"], "yarr");
}

#[tokio::test]
async fn should_match_on_anthropic_top_level_system_string() {
    // Anthropic's system prompt is a top-level string (or array of text
    // blocks) on the request body, not a message entry.
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_system_prompt("helpful assistant")
                .respond_with_content("OK"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "system": "You are a helpful assistant that answers questions.",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_match_on_exact_temperature() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_temperature(0.7)
                .respond_with_content("warm"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Wrong temperature → no match
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "temperature": 0.2,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Exact match → 200
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "temperature": 0.7,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_match_on_temperature_range() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_temperature_range(Some(0.5), Some(1.0))
                .respond_with_content("in range"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Below range
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "temperature": 0.2,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // In range (inclusive)
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "temperature": 0.5,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Above range
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "temperature": 1.5,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_match_on_metadata_entry() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_metadata("customer_id", "cust-42")
                .respond_with_content("known customer"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Different metadata
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "metadata": {"customer_id": "cust-99"},
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Matching metadata
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "metadata": {"customer_id": "cust-42"},
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_match_on_tool_schema_openai_and_anthropic() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_tool_schema("get_weather")
                .respond_with_content("weather tool detected"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // OpenAI tools format: tools[].function.name
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [
                {"type": "function", "function": {"name": "get_weather", "parameters": {}}}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Anthropic tools format: tools[].name
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"name": "get_weather", "input_schema": {"type": "object"}}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_match_on_gemini_tool_schema() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_tool_schema("get_weather")
                .respond_with_content("weather"),
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "tools": [{
                "functionDeclarations": [
                    {"name": "get_weather", "description": "..."}
                ]
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn should_reject_fixture_with_blank_header_name() {
    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_header("   ", "value")
                .respond_with_content("ok"),
        )
        .build()
        .await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("header name must not be blank"),
        "unexpected: {err}"
    );
}

#[tokio::test]
async fn should_prefer_high_priority_fixture_regardless_of_file_order() {
    // A lower-priority broad match comes first in file order, but
    // the `priority: 100` specific fixture should win the two-pass
    // matcher even though it appears later.
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("low-priority match"),
        )
        .fixture(
            Fixture::new()
                .match_user_message("hello world")
                .with_priority(100)
                .respond_with_content("high-priority specific"),
        )
        .build()
        .await
        .unwrap();

    let body: serde_json::Value = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello world"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "high-priority specific"
    );
}

#[tokio::test]
async fn should_use_catch_all_only_when_no_other_fixture_matches() {
    let server = ServerBuilder::new()
        // Catch-all listed FIRST in file order — should still be
        // reached last per the catch_all semantics.
        .fixture(
            Fixture::new()
                .as_catch_all()
                .respond_with_content("catch-all fallback"),
        )
        .fixture(
            Fixture::new()
                .match_user_message("specific")
                .respond_with_content("specific match"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Request that matches the specific fixture.
    let body: serde_json::Value = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "specific"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "specific match");

    // Request that only the catch-all handles.
    let body: serde_json::Value = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "anything else"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "catch-all fallback"
    );
}

#[tokio::test]
async fn should_reject_fixture_with_inverted_temperature_range() {
    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_temperature_range(Some(1.0), Some(0.5))
                .respond_with_content("ok"),
        )
        .build()
        .await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("range inverted"), "unexpected: {err}");
}

// ---------------------------------------------------------------
// JSONPath body matching
// ---------------------------------------------------------------

#[cfg(feature = "jsonpath")]
#[tokio::test]
async fn should_match_on_body_jsonpath_present() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_body_jsonpath("$.messages[?(@.role == 'system')]")
                .respond_with_content("system-present"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // No system message → no match → 404
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // System message present → match → 200
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "be brief"},
                {"role": "user", "content": "hi"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "system-present");
}

#[cfg(feature = "jsonpath")]
#[tokio::test]
async fn should_match_on_body_jsonpath_deep_field() {
    // Match only when a specific tool definition is present in the request.
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_body_jsonpath("$.tools[?(@.function.name == 'get_weather')]")
                .respond_with_content("weather-tool-seen"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Different tool → no match
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "type": "function",
                "function": {"name": "get_stock_price", "parameters": {}}
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    // Right tool → match
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "type": "function",
                "function": {"name": "get_weather", "parameters": {}}
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[cfg(feature = "jsonpath")]
#[tokio::test]
async fn should_reject_fixture_with_invalid_jsonpath() {
    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_body_jsonpath("$[not-valid")
                .respond_with_content("ok"),
        )
        .build()
        .await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("body_jsonpath is invalid"),
        "unexpected: {err}"
    );
}

#[cfg(feature = "jsonpath")]
#[tokio::test]
async fn should_reject_fixture_with_blank_jsonpath() {
    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_body_jsonpath("   ")
                .respond_with_content("ok"),
        )
        .build()
        .await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("body_jsonpath must not be empty"),
        "unexpected: {err}"
    );
}

// ---------------------------------------------------------------
// Provider-specific request-shape regressions
// ---------------------------------------------------------------

/// Responses API sends the system prompt at the top-level
/// `instructions` string, not as a `role: "system"` message inside
/// `input`. The extractor must check `instructions` first.
#[tokio::test]
async fn should_match_responses_system_prompt_via_instructions_field() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_system_prompt("Be concise")
                .respond_with_content("concise-matched"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [{"role": "user", "content": "hi"}],
            "instructions": "Be concise and helpful"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Gemini puts temperature inside `generationConfig`, not at the
/// top level. The matcher must extract provider-aware.
#[tokio::test]
async fn should_match_gemini_temperature_via_generation_config() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_temperature(0.7)
                .respond_with_content("gemini-temp-matched"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "generationConfig": {
                "temperature": 0.7
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ---------------------------------------------------------------
// Validation error paths
// ---------------------------------------------------------------

#[tokio::test]
async fn should_reject_fixture_with_nan_exact_temperature() {
    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_temperature(f64::NAN)
                .respond_with_content("ok"),
        )
        .build()
        .await;
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("must be a finite number"), "unexpected: {err}");
}

#[tokio::test]
async fn should_reject_fixture_with_nonfinite_temperature_range_bounds() {
    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_temperature_range(Some(f64::NEG_INFINITY), Some(1.0))
                .respond_with_content("ok"),
        )
        .build()
        .await;
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("match.temperature.min must be finite"));

    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_temperature_range(Some(0.0), Some(f64::INFINITY))
                .respond_with_content("ok"),
        )
        .build()
        .await;
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("match.temperature.max must be finite"));
}

#[tokio::test]
async fn should_reject_fixture_with_empty_temperature_range() {
    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_temperature_range(None, None)
                .respond_with_content("ok"),
        )
        .build()
        .await;
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("at least one of min/max"), "unexpected: {err}");
}

#[tokio::test]
async fn should_reject_fixture_with_blank_metadata_key() {
    let result = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_metadata("   ", "value")
                .respond_with_content("ok"),
        )
        .build()
        .await;
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("match.metadata: key must not be blank"));
}

// Duplicate header names differing only in case must be rejected
// by the lowercase-fold step at load time. Exercised via YAML since
// the builder API already lowercases at insert.
#[tokio::test]
async fn should_reject_fixture_with_case_folded_duplicate_headers() {
    let tmp =
        std::env::temp_dir().join(format!("llmposter-dup-headers-{}.yaml", std::process::id()));
    std::fs::write(
        &tmp,
        r#"fixtures:
  - match:
      headers:
        X-Tenant: acme
        x-tenant: globex
    response:
      content: ok
"#,
    )
    .unwrap();

    let result = ServerBuilder::new().load_yaml(&tmp);
    let _ = std::fs::remove_file(&tmp);
    let err = format!("{:?}", result.err().expect("expected load error"));
    assert!(
        err.contains("duplicate header name after case-folding"),
        "unexpected: {err}"
    );
}

// ---------------------------------------------------------------
// Extraction branch coverage
// ---------------------------------------------------------------

/// Anthropic system: array of text blocks (vs legacy string form).
#[tokio::test]
async fn should_match_anthropic_system_content_block_array() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_system_prompt("pirate")
                .respond_with_content("yarr"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", server.url()))
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 10,
            "system": [
                {"type": "text", "text": "You are a pirate"},
                {"type": "text", "text": "Answer only in shanties"}
            ],
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Gemini system prompt via `systemInstruction.parts[*].text`.
#[tokio::test]
async fn should_match_gemini_system_instruction_parts() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_system_prompt("pirate")
                .respond_with_content("arr-matched"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "systemInstruction": {
                "parts": [{"text": "You are a pirate"}, {"text": "talk like one"}]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Responses API system via `input[*]` with `role: system` — fallback
/// path when `instructions` is absent.
#[tokio::test]
async fn should_match_responses_system_via_input_array_fallback() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_system_prompt("Be concise")
                .respond_with_content("concise-input-matched"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/responses", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "input": [
                {"role": "system", "content": "Be concise please"},
                {"role": "user", "content": "hi"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// OpenAI system message with content as an array of text parts
/// (not just plain string).
#[tokio::test]
async fn should_match_openai_system_with_content_parts_array() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_system_prompt("pirate")
                .respond_with_content("yarr-parts"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {
                    "role": "system",
                    "content": [
                        {"type": "text", "text": "You are a pirate"},
                        {"type": "text", "text": "tell tall tales"}
                    ]
                },
                {"role": "user", "content": "hi"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

// ---------------------------------------------------------------
// No-match negative paths (trigger the `return false` branches)
// ---------------------------------------------------------------

#[tokio::test]
async fn should_reject_request_missing_temperature_field() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_temperature(0.7)
                .respond_with_content("ok"),
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

/// OpenAI's metadata spec allows number and boolean values; the
/// matcher coerces them to their JSON scalar form so a fixture
/// written with `"2"` or `"true"` still matches.
#[tokio::test]
async fn should_match_metadata_coerced_from_number_or_bool() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_metadata("priority", "2")
                .respond_with_content("num-coerced"),
        )
        .fixture(
            Fixture::new()
                .match_metadata("active", "true")
                .respond_with_content("bool-coerced"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Integer metadata value
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "metadata": {"priority": 2},
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "num-coerced");

    // Boolean metadata value
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "metadata": {"active": true},
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "bool-coerced");
}

#[tokio::test]
async fn should_reject_metadata_with_object_value() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_metadata("nested", "value")
                .respond_with_content("ok"),
        )
        .build()
        .await
        .unwrap();

    // Nested object metadata value → not coerced → no match → 404
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "metadata": {"nested": {"inner": "value"}},
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_reject_request_missing_metadata_object() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_metadata("tenant", "acme")
                .respond_with_content("ok"),
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_reject_request_when_metadata_key_missing() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_metadata("tenant", "acme")
                .respond_with_content("ok"),
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "metadata": {"other": "field"},
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_reject_tool_schema_when_no_matching_tool_declared() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_tool_schema("nonexistent_tool")
                .respond_with_content("ok"),
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "tools": [{"type": "function", "function": {"name": "get_weather"}}],
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

/// Streaming Gemini fixture with an explicit `stop_reason` that
/// overrides the default — exercises the has_explicit_reason branch
/// in `handler/gemini.rs::build_stream_frames`.
#[tokio::test]
async fn should_override_gemini_streaming_stop_reason() {
    use llmposter::fixture::FixtureResponse;
    let server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content: Some("truncated gemini".to_string()),
                stop_reason: Some("MAX_TOKENS".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        })
        .build()
        .await
        .unwrap();

    let body: String = reqwest::Client::new()
        .post(format!(
            "{}/v1beta/models/gemini-pro:streamGenerateContent?alt=sse",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        body.contains("MAX_TOKENS"),
        "expected MAX_TOKENS stop reason, got:\n{body}"
    );
}

/// Streaming OpenAI fixture with an explicit `finish_reason` that
/// overrides the default — exercises the last-chunk overwrite branch
/// in `handler/openai.rs::build_stream_frames`.
#[tokio::test]
async fn should_override_openai_streaming_finish_reason() {
    use llmposter::fixture::FixtureResponse;
    let server = ServerBuilder::new()
        .fixture(Fixture {
            response: Some(FixtureResponse {
                content: Some("truncated stream".to_string()),
                finish_reason: Some("length".to_string()),
                ..Default::default()
            }),
            ..Fixture::new()
        })
        .build()
        .await
        .unwrap();

    let body: String = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        body.contains("\"finish_reason\":\"length\""),
        "expected length finish_reason, got:\n{body}"
    );
}

/// `/code/429` exercises the middleware branch where the response
/// has no `Provider` extension (the echo route is provider-agnostic),
/// so the rate-limit header insertion falls into the None arm.
#[tokio::test]
async fn should_serve_code_429_without_provider_specific_headers() {
    let server = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("unused"))
        .build()
        .await
        .unwrap();

    let resp = reqwest::get(format!("{}/code/429", server.url()))
        .await
        .unwrap();
    assert_eq!(resp.status(), 429);
    // Common retry-after header still applies to every 429.
    assert_eq!(resp.headers().get("retry-after").unwrap(), "60");
    // Provider-specific rate limit headers are NOT emitted.
    assert!(resp.headers().get("x-ratelimit-limit-requests").is_none());
    assert!(resp
        .headers()
        .get("anthropic-ratelimit-requests-limit")
        .is_none());
}

#[tokio::test]
async fn should_reject_system_prompt_when_pattern_does_not_match_actual_text() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_system_prompt("pirate")
                .respond_with_content("ok"),
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "be a friendly assistant"},
                {"role": "user", "content": "hi"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn should_reject_system_prompt_when_no_system_message_present() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_system_prompt("pirate")
                .respond_with_content("ok"),
        )
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!(
            "{}/v1beta/models/gemini-pro:generateContent",
            server.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

/// `catch_all: true` defers a fixture to the fallback pass, but
/// within that pass `priority` still orders candidates.
#[tokio::test]
async fn should_sort_catch_all_fixtures_by_priority() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .as_catch_all()
                .with_priority(1)
                .respond_with_content("low-pri-catch-all"),
        )
        .fixture(
            Fixture::new()
                .as_catch_all()
                .with_priority(10)
                .respond_with_content("high-pri-catch-all"),
        )
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "high-pri-catch-all"
    );
}
