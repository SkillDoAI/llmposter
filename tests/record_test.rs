#![cfg(feature = "record")]
use llmposter::{FailureConfig, Fixture, RequestOutcome, ServerBuilder, ToolCall, VcrMode};

fn temp_cassette(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("llmposter_record_int_tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{}_{}.yaml", name, std::process::id()))
}

#[tokio::test]
async fn should_reject_record_mode_with_auth() {
    let err = ServerBuilder::new()
        .with_bearer_token("tok")
        .vcr_mode(VcrMode::RecordOnMiss)
        .record_file(temp_cassette("auth_reject"))
        .build()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("auth"), "got: {}", err);
}

#[tokio::test]
async fn should_reject_invalid_redact_pattern() {
    let err = ServerBuilder::new()
        .vcr_mode(VcrMode::Record)
        .record_file(temp_cassette("bad_redact"))
        .redact("([unclosed")
        .build()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("redact"), "got: {}", err);
}

#[tokio::test]
async fn should_reject_record_mode_on_non_loopback_bind_without_optin() {
    let err = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .record_file(temp_cassette("bind_reject"))
        .bind("0.0.0.0:0")
        .build()
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("loopback") && msg.contains("allow_remote_record"),
        "got: {}",
        msg
    );
}

#[tokio::test]
async fn should_allow_non_loopback_record_bind_with_optin() {
    let server = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .record_file(temp_cassette("bind_optin"))
        .bind("0.0.0.0:0")
        .allow_remote_record(true)
        .build()
        .await
        .unwrap();
    drop(server);
}

#[tokio::test]
async fn should_reject_proxy_url_with_bad_scheme() {
    let err = ServerBuilder::new()
        .vcr_mode(VcrMode::Record)
        .record_file(temp_cassette("bad_scheme"))
        .proxy_openai("ftp://example.com")
        .build()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("http"), "got: {}", err);
}

#[tokio::test]
async fn should_create_pristine_cassette_and_load_existing_entries_at_build() {
    let path = temp_cassette("build_load");
    let _ = std::fs::remove_file(&path);
    let server = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .record_file(&path)
        .build()
        .await
        .unwrap();
    assert!(path.exists());
    assert_eq!(server.fixture_count(), 0);
    drop(server);
    std::fs::write(&path, "fixtures:\n- match:\n    user_message: \"prior\"\n    model: \"m\"\n  provider: openai\n  priority: -1\n  response:\n    content: \"from cassette\"\n").unwrap();
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hand")
                .respond_with_content("handwritten"),
        )
        .vcr_mode(VcrMode::RecordOnMiss)
        .record_file(&path)
        .build()
        .await
        .unwrap();
    assert_eq!(server.fixture_count(), 2);
}

#[tokio::test]
async fn should_not_double_load_cassette_inside_dir_source_with_unnormalized_paths() {
    // Relative paths on purpose: "./<dir>" as the dir source and
    // "<dir>/recorded.yaml" as the record file spell the same location
    // differently — component-wise starts_with would miss the overlap.
    let dir_name = format!("target/llmposter_record_dblload_{}", std::process::id());
    std::fs::create_dir_all(&dir_name).unwrap();
    let cassette = format!("{}/recorded.yaml", dir_name);
    std::fs::write(&cassette, "fixtures:\n- match:\n    user_message: \"prior\"\n    model: \"m\"\n  provider: openai\n  priority: -1\n  response:\n    content: \"from cassette\"\n").unwrap();
    let server = ServerBuilder::new()
        .load_yaml_dir(std::path::Path::new(&format!("./{}", dir_name)))
        .unwrap()
        .vcr_mode(VcrMode::RecordOnMiss)
        .record_file(&cassette)
        .build()
        .await
        .unwrap();
    assert_eq!(
        server.fixture_count(),
        1,
        "cassette inside a dir source must not be double-loaded"
    );
    drop(server);
    let _ = std::fs::remove_dir_all(&dir_name);
}

#[tokio::test]
async fn should_load_cassette_in_subdir_of_dir_source_explicitly() {
    // load_yaml_dir is NON-recursive: a cassette in a SUBdirectory of a
    // dir source is never read by the flat scan, so it must be loaded
    // (and registered for reload) via the explicit record_file path.
    let base = std::env::temp_dir().join(format!("llmposter_record_subdir_{}", std::process::id()));
    let subdir = base.join("cassettes");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&subdir).unwrap();
    std::fs::write(
        base.join("hand.yaml"),
        "fixtures:\n- match:\n    user_message: \"hand\"\n  response:\n    content: \"handwritten\"\n",
    )
    .unwrap();
    let cassette = subdir.join("recorded.yaml");
    std::fs::write(&cassette, "fixtures:\n- match:\n    user_message: \"prior\"\n    model: \"m\"\n  provider: openai\n  priority: -1\n  response:\n    content: \"from cassette\"\n").unwrap();
    let server = ServerBuilder::new()
        .load_yaml_dir(&base)
        .unwrap()
        .vcr_mode(VcrMode::RecordOnMiss)
        .record_file(&cassette)
        .build()
        .await
        .unwrap();
    assert_eq!(
        server.fixture_count(),
        2,
        "subdir cassette entries must load via the explicit path (dir scan is flat)"
    );
    drop(server);
    let _ = std::fs::remove_dir_all(&base);
}

// --- Record-path end-to-end tests (two-server pattern): a plain -------
// --- llmposter instance plays "upstream provider", a second VCR -------
// --- instance proxies to it. Non-streaming persist is synchronous -----
// --- (inline await before responding), so no polling is needed. -------

/// Fresh cassette path for a record-path test: removes any leftover from
/// a previous run of the same pid so dedupe seeding starts empty.
fn fresh_cassette(name: &str) -> std::path::PathBuf {
    let path = temp_cassette(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn openai_chat_body(message: &str) -> serde_json::Value {
    serde_json::json!({
        "model": "gpt-test",
        "messages": [{"role": "user", "content": message}]
    })
}

#[tokio::test]
async fn should_record_on_miss_then_replay_openai() {
    let upstream = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("capital of France")
                .respond_with_content("Paris."),
        )
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("on_miss_replay");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!("{}/v1/chat/completions", vcr.url());
    let req = openai_chat_body("capital of France");

    // First request: miss → forwarded upstream, recorded, relayed.
    let resp = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "Paris.");
    assert_eq!(upstream.request_count(), 1);
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(fixtures[0].priority, Some(-1));

    // Second identical request: served from the in-memory recorded
    // fixture — the upstream must NOT be hit again.
    let resp2 = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
    let body2: serde_json::Value = resp2.json().await.unwrap();
    assert_eq!(body2["choices"][0]["message"]["content"], "Paris.");
    assert_eq!(upstream.request_count(), 1, "replay must be in-memory");

    let outcomes: Vec<RequestOutcome> = vcr.get_requests().iter().map(|r| r.outcome).collect();
    assert_eq!(
        outcomes,
        vec![RequestOutcome::Recorded, RequestOutcome::Matched]
    );
}

#[tokio::test]
async fn should_record_all_bypass_local_fixtures() {
    let upstream = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("capital of France")
                .respond_with_content("Paris."),
        )
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("record_all_bypass");
    // The VCR server OWNS a matching fixture, but mode Record ignores it.
    let vcr = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("capital of France")
                .respond_with_content("LOCAL"),
        )
        .vcr_mode(VcrMode::Record)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!("{}/v1/chat/completions", vcr.url());
    let req = openai_chat_body("capital of France");

    for _ in 0..2 {
        let resp = client.post(&url).json(&req).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(
            body["choices"][0]["message"]["content"], "Paris.",
            "Record mode bypasses local fixtures — responses come from upstream"
        );
    }
    assert_eq!(upstream.request_count(), 2, "every request forwards");
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1, "identical prompts dedupe to one entry");
}

#[tokio::test]
async fn should_record_anthropic_tool_calls() {
    let upstream = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "SF"}),
        }]))
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("anthropic_tools");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_anthropic(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!("{}/v1/messages", vcr.url());
    let req = serde_json::json!({
        "model": "claude-test",
        "max_tokens": 64,
        "messages": [{"role": "user", "content": "weather in SF?"}]
    });

    let resp = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1);
    let calls = fixtures[0]
        .response
        .as_ref()
        .unwrap()
        .tool_calls
        .as_ref()
        .unwrap();
    assert_eq!(calls[0].name, "get_weather");
    assert_eq!(
        calls[0].arguments["city"], "SF",
        "arguments recorded as an object"
    );

    // Replay: the recorded fixture serves a tool_use block.
    let resp2 = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
    let body2: serde_json::Value = resp2.json().await.unwrap();
    let block = &body2["content"][0];
    assert_eq!(block["type"], "tool_use");
    assert_eq!(block["name"], "get_weather");
    assert_eq!(upstream.request_count(), 1);
}

#[tokio::test]
async fn should_record_gemini_via_url_model() {
    let upstream = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("gemini says hi"))
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("gemini_url_model");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_gemini(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!(
        "{}/v1beta/models/gemini-2.5-flash:generateContent?key=whatever",
        vcr.url()
    );
    let req = serde_json::json!({
        "contents": [{"role": "user", "parts": [{"text": "hello gemini"}]}]
    });

    let resp = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "gemini says hi"
    );

    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(
        fixtures[0].match_rule.as_ref().unwrap().model,
        Some(llmposter::fixture::StringMatch::Substring(
            "gemini-2.5-flash".to_string()
        )),
        "recorded model comes from the URL segment"
    );

    // Replay from the recorded fixture.
    let resp2 = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
    let body2: serde_json::Value = resp2.json().await.unwrap();
    assert_eq!(
        body2["candidates"][0]["content"]["parts"][0]["text"],
        "gemini says hi"
    );
    assert_eq!(upstream.request_count(), 1);
}

#[tokio::test]
async fn should_record_responses_api() {
    let upstream = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("resp text"))
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("responses_api");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!("{}/v1/responses", vcr.url());
    let req = serde_json::json!({"model": "gpt-test", "input": "ping responses"});

    let resp = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(
        fixtures[0].response.as_ref().unwrap().content.as_deref(),
        Some("resp text"),
        "output_text content extracted from the Responses API shape"
    );

    let resp2 = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
    assert_eq!(upstream.request_count(), 1);
}

#[tokio::test]
async fn should_record_legacy_completions() {
    let upstream = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("legacy done"))
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("legacy_completions");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!("{}/v1/completions", vcr.url());
    let req = serde_json::json!({"model": "davinci-test", "prompt": "legacy hi"});

    let resp = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(
        fixtures[0].response.as_ref().unwrap().content.as_deref(),
        Some("legacy done")
    );

    // Replay from the recorded fixture.
    let resp2 = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
    let body2: serde_json::Value = resp2.json().await.unwrap();
    assert_eq!(body2["choices"][0]["text"], "legacy done");
    assert_eq!(upstream.request_count(), 1);
}

#[tokio::test]
async fn should_pass_through_upstream_errors_unrecorded() {
    let upstream = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .with_error_headers(429, "rate limited", [("retry-after", "7")])
                .unwrap(),
        )
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("upstream_error");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    // Fetch the error DIRECTLY from the upstream first — the relayed
    // body must be byte-identical to it.
    let direct_body = client
        .post(format!("{}/v1/chat/completions", upstream.url()))
        .json(&openai_chat_body("anything"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    let resp = client
        .post(format!("{}/v1/chat/completions", vcr.url()))
        .json(&openai_chat_body("anything"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 429, "upstream error passes through");
    assert_eq!(
        resp.headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()),
        Some("7"),
        "upstream retry-after must survive the relay (client backoff logic)"
    );
    let relayed_body = resp.text().await.unwrap();
    assert_eq!(
        relayed_body, direct_body,
        "passthrough body must equal the upstream error JSON verbatim"
    );

    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert!(
        fixtures.is_empty(),
        "a 429 must not be immortalized in the cassette"
    );
    let captured = vcr.get_requests();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].outcome, RequestOutcome::Recorded);
    assert_eq!(captured[0].status_code, 429);
}

#[tokio::test]
async fn should_record_only_once_for_concurrent_identical_misses() {
    // Upstream latency (~300ms) forces the two identical misses to
    // overlap: both check the fixture set before either has persisted,
    // so BOTH forward — then the dedupe set collapses them to a single
    // cassette entry and a single live fixture.
    let mut slow = Fixture::new()
        .match_user_message("slow question")
        .respond_with_content("slow answer");
    slow.failure = Some(FailureConfig {
        latency_ms: Some(300),
        ..Default::default()
    });
    let upstream = ServerBuilder::new().fixture(slow).build().await.unwrap();
    let cassette = fresh_cassette("concurrent_miss");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!("{}/v1/chat/completions", vcr.url());
    let req = openai_chat_body("slow question");

    let (r1, r2) = tokio::join!(
        client.post(&url).json(&req).send(),
        client.post(&url).json(&req).send()
    );
    assert_eq!(r1.unwrap().status(), 200);
    assert_eq!(r2.unwrap().status(), 200);
    assert_eq!(
        upstream.request_count(),
        2,
        "both overlapping misses forward"
    );
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1, "dedupe collapses to one cassette entry");
    assert_eq!(vcr.fixture_count(), 1, "one live recorded fixture");

    // Third request replays in-memory — no further upstream call.
    let r3 = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(r3.status(), 200);
    let body3: serde_json::Value = r3.json().await.unwrap();
    assert_eq!(body3["choices"][0]["message"]["content"], "slow answer");
    assert_eq!(upstream.request_count(), 2, "replay stays in-memory");
}

#[tokio::test]
async fn should_pass_through_non_json_2xx_unrecorded() {
    // corrupt_body makes the upstream return HTTP 200 text/plain
    // "overloaded" — a 2xx that is NOT JSON. The recorder must relay it
    // verbatim and record nothing.
    let mut corrupt = Fixture::new().respond_with_content("ignored");
    corrupt.failure = Some(FailureConfig {
        corrupt_body: Some(true),
        ..Default::default()
    });
    let upstream = ServerBuilder::new().fixture(corrupt).build().await.unwrap();
    let cassette = fresh_cassette("non_json_2xx");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", vcr.url()))
        .json(&openai_chat_body("anything"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .starts_with("text/plain"),
        "upstream content-type relayed"
    );
    assert_eq!(resp.text().await.unwrap(), "overloaded", "body verbatim");
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert!(fixtures.is_empty(), "non-JSON 2xx is never recorded");
}

#[tokio::test]
async fn should_pass_through_unextractable_200_unrecorded() {
    // A refusal-shaped 200 has neither text content nor tool calls —
    // nothing the fixture schema can replay. Pass through, record nothing.
    let upstream = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_refusal("safety policy"))
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("unextractable_200");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", vcr.url()))
        .json(&openai_chat_body("do something bad"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["choices"][0]["message"]["refusal"], "safety policy",
        "refusal body relayed verbatim"
    );
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert!(fixtures.is_empty(), "unextractable 200 is never recorded");
}

#[tokio::test]
async fn should_return_502_when_upstream_unreachable() {
    let cassette = fresh_cassette("upstream_unreachable");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai("http://127.0.0.1:1")
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let token = "sk-super-secret-bearer-value";
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", vcr.url()))
        .header("authorization", format!("Bearer {}", token))
        .json(&openai_chat_body("hello?"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 502);
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("unreachable"),
        "502 body should name the failure: {}",
        body
    );
    assert!(
        !body.contains(token),
        "502 body must never echo auth material: {}",
        body
    );
}

#[tokio::test]
async fn should_prefer_handwritten_fixture_on_miss_mode() {
    let upstream = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("UPSTREAM"))
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("prefer_handwritten");
    let vcr = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("local hit")
                .respond_with_content("LOCAL"),
        )
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", vcr.url()))
        .json(&openai_chat_body("local hit"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "LOCAL");
    assert_eq!(upstream.request_count(), 0, "match never forwards");
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert!(fixtures.is_empty(), "cassette stays empty on a local hit");
}

#[tokio::test]
async fn should_redact_recorded_content() {
    let upstream = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("the key is sk-secret123 done"))
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("redact_content");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .redact(r"sk-[A-Za-z0-9]+")
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!("{}/v1/chat/completions", vcr.url());
    let req = openai_chat_body("what is the key");

    // First response relays the upstream body; the CASSETTE is redacted.
    let resp = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let cassette_text = std::fs::read_to_string(&cassette).unwrap();
    assert!(
        cassette_text.contains("[REDACTED]"),
        "cassette: {}",
        cassette_text
    );
    assert!(
        !cassette_text.contains("sk-secret123"),
        "cassette must not hold the secret: {}",
        cassette_text
    );

    // Replay serves the redacted content.
    let resp2 = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
    let body2: serde_json::Value = resp2.json().await.unwrap();
    let content = body2["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(content.contains("[REDACTED]"), "replay: {}", content);
    assert!(!content.contains("sk-secret123"), "replay: {}", content);
    assert_eq!(upstream.request_count(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn should_create_cassette_with_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let path = temp_cassette("perms");
    let _ = std::fs::remove_file(&path);
    let _server = ServerBuilder::new()
        .vcr_mode(VcrMode::Record)
        .record_file(&path)
        .build()
        .await
        .unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "cassette should be owner-only, got {:o}",
        mode
    );
}
