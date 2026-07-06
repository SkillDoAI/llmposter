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
async fn should_reject_proxy_url_with_embedded_credentials() {
    let err = ServerBuilder::new()
        .vcr_mode(VcrMode::Record)
        .record_file(temp_cassette("proxy_creds"))
        .proxy_openai("http://user:secret@127.0.0.1:9999/")
        .build()
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("credentials"), "got: {}", msg);
    assert!(
        !msg.contains("secret"),
        "build error must not echo the credential: {}",
        msg
    );
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

/// Minimal raw HTTP upstream answering every request with a fixed JSON
/// body. Needed because llmposter's own embeddings mock JOINS array
/// input and always answers with exactly ONE data entry — it can never
/// produce the multi-entry `data` array a real provider returns for
/// multi-input requests.
async fn spawn_raw_json_upstream(body: &'static str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                // Drain the request: headers, then content-length body bytes.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                let header_end = loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
                let content_length = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                while buf.len() < header_end + content_length {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    format!("http://{}", addr)
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

#[tokio::test]
async fn should_record_embeddings() {
    let upstream = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("embed me")
                .respond_with_embedding(vec![0.5, 0.5]),
        )
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("embeddings_on_miss");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!("{}/v1/embeddings", vcr.url());
    let req = serde_json::json!({"model": "text-embedding-3-small", "input": "embed me"});

    // First request: miss → forwarded upstream, recorded, relayed.
    let resp = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["data"][0]["embedding"], serde_json::json!([0.5, 0.5]));
    assert_eq!(upstream.request_count(), 1);
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(
        fixtures[0]
            .response
            .as_ref()
            .unwrap()
            .embedding
            .as_ref()
            .unwrap(),
        &vec![0.5, 0.5]
    );

    // Second identical request replays in-memory — no further upstream call.
    let resp2 = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
    let body2: serde_json::Value = resp2.json().await.unwrap();
    assert_eq!(body2["data"][0]["embedding"], serde_json::json!([0.5, 0.5]));
    assert_eq!(upstream.request_count(), 1, "replay must be in-memory");

    let outcomes: Vec<RequestOutcome> = vcr.get_requests().iter().map(|r| r.outcome).collect();
    assert_eq!(
        outcomes,
        vec![RequestOutcome::Recorded, RequestOutcome::Matched]
    );
}

#[tokio::test]
async fn should_pass_through_multi_input_embeddings_unrecorded() {
    // Two-entry data array — what a real provider returns for a
    // two-string input array. The fixture schema stores ONE vector, so
    // this must pass through verbatim and record nothing.
    let upstream_body = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.1]},{"object":"embedding","index":1,"embedding":[0.2]}],"model":"text-embedding-3-small","usage":{"prompt_tokens":4,"total_tokens":4}}"#;
    let upstream_url = spawn_raw_json_upstream(upstream_body).await;
    let cassette = fresh_cassette("embeddings_multi_input");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream_url)
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/embeddings", vcr.url()))
        .json(&serde_json::json!({
            "model": "text-embedding-3-small",
            "input": ["first thing", "second thing"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let relayed = resp.text().await.unwrap();
    assert_eq!(relayed, upstream_body, "multi-entry body relayed verbatim");

    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert!(
        fixtures.is_empty(),
        "multi-input response is never recorded"
    );
    let captured = vcr.get_requests();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].outcome, RequestOutcome::Recorded);
    assert_eq!(captured[0].status_code, 200);
}

#[tokio::test]
async fn should_record_all_mode_embeddings() {
    let upstream = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("embed all")
                .respond_with_embedding(vec![0.25, 0.75]),
        )
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("embeddings_record_all");
    // The VCR server OWNS a matching fixture, but mode Record ignores it.
    let vcr = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("embed all")
                .respond_with_embedding(vec![9.0, 9.0]),
        )
        .vcr_mode(VcrMode::Record)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/embeddings", vcr.url()))
        .json(&serde_json::json!({"model": "text-embedding-3-small", "input": "embed all"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["data"][0]["embedding"],
        serde_json::json!([0.25, 0.75]),
        "Record mode bypasses local fixtures — response comes from upstream"
    );
    assert_eq!(upstream.request_count(), 1);

    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(
        fixtures[0]
            .response
            .as_ref()
            .unwrap()
            .embedding
            .as_ref()
            .unwrap(),
        &vec![0.25, 0.75]
    );
}

#[tokio::test]
async fn should_return_502_for_embeddings_when_upstream_unreachable() {
    let cassette = fresh_cassette("embeddings_unreachable");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai("http://127.0.0.1:1")
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let token = "sk-super-secret-bearer-value";
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/embeddings", vcr.url()))
        .header("authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({"model": "text-embedding-3-small", "input": "hello?"}))
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

// --- Streaming record tests: the SSE tee relays frames to the client ---
// --- while a spawned task buffers, reassembles, and persists — the -----
// --- recording lands asynchronously, so poll with wait_until. ----------

async fn wait_until(mut cond: impl FnMut() -> bool, what: &str) {
    for _ in 0..300 {
        if cond() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {}", what);
}

#[tokio::test]
async fn should_record_openai_stream_and_replay() {
    let upstream = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("stream me")
                .respond_with_content("streamed answer")
                .with_streaming(None, Some(5)),
        )
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("openai_stream");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!("{}/v1/chat/completions", vcr.url());
    let mut req = openai_chat_body("stream me");
    req["stream"] = serde_json::json!(true);

    // First request: miss → streamed through frame-by-frame.
    let resp = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .starts_with("text/event-stream"),
        "upstream SSE content-type relayed"
    );
    let text = resp.text().await.unwrap();
    assert!(text.contains("data: "), "SSE frames relayed: {}", text);
    assert!(text.contains("data: [DONE]"), "DONE relayed: {}", text);

    // The recording lands from the spawned task after the stream ends.
    wait_until(|| vcr.fixture_count() == 1, "openai stream recording").await;
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(fixtures[0].priority, Some(-1));
    assert_eq!(
        fixtures[0].response.as_ref().unwrap().content.as_deref(),
        Some("streamed answer"),
        "chunked deltas reassembled into the full content"
    );

    // Second streamed request replays from the recorded fixture.
    let resp2 = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
    let text2 = resp2.text().await.unwrap();
    assert!(
        text2.contains("streamed answer"),
        "replayed SSE carries the full content: {}",
        text2
    );
    assert_eq!(upstream.request_count(), 1, "replay stays in-memory");
}

#[tokio::test]
async fn should_record_anthropic_stream() {
    let upstream = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("claude stream")
                .respond_with_content("claude streamed reply")
                .with_streaming(None, Some(6)),
        )
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("anthropic_stream");
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
        "stream": true,
        "messages": [{"role": "user", "content": "claude stream"}]
    });

    let resp = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(
        text.contains("event: message_stop"),
        "client sees the full anthropic stream: {}",
        text
    );

    wait_until(|| vcr.fixture_count() == 1, "anthropic stream recording").await;
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(
        fixtures[0].response.as_ref().unwrap().content.as_deref(),
        Some("claude streamed reply")
    );

    // Replay from the recorded fixture.
    let resp2 = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
    let text2 = resp2.text().await.unwrap();
    assert!(text2.contains("event: message_stop"), "replay: {}", text2);
    assert_eq!(upstream.request_count(), 1);
}

#[tokio::test]
async fn should_record_anthropic_tool_stream() {
    let upstream = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_tool_calls(vec![ToolCall {
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "SF"}),
        }]))
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("anthropic_tool_stream");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_anthropic(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/messages", vcr.url()))
        .json(&serde_json::json!({
            "model": "claude-test",
            "max_tokens": 64,
            "stream": true,
            "messages": [{"role": "user", "content": "weather in SF?"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(text.contains("input_json_delta"), "tool stream: {}", text);

    wait_until(|| vcr.fixture_count() == 1, "anthropic tool recording").await;
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
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
        "partial_json fragments reassembled into intact arguments"
    );
}

#[tokio::test]
async fn should_record_gemini_sse_stream() {
    let upstream = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("gemini streamed"))
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("gemini_sse_stream");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_gemini(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!(
        "{}/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse&key=whatever",
        vcr.url()
    );
    let req = serde_json::json!({
        "contents": [{"role": "user", "parts": [{"text": "hello gemini"}]}]
    });

    let resp = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(text.contains("data: "), "gemini SSE relayed: {}", text);

    wait_until(|| vcr.fixture_count() == 1, "gemini SSE recording").await;
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(
        fixtures[0].response.as_ref().unwrap().content.as_deref(),
        Some("gemini streamed")
    );

    // Replay from the recorded fixture.
    let resp2 = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
    let text2 = resp2.text().await.unwrap();
    assert!(text2.contains("gemini streamed"), "replay: {}", text2);
    assert_eq!(upstream.request_count(), 1);
}

#[tokio::test]
async fn should_pass_through_gemini_json_array_stream_unrecorded() {
    let upstream = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("array streamed"))
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("gemini_json_array");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_gemini(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    // No ?alt=sse — Gemini's default JSON-array stream shape.
    let resp = reqwest::Client::new()
        .post(format!(
            "{}/v1beta/models/gemini-2.5-flash:streamGenerateContent?key=whatever",
            vcr.url()
        ))
        .json(&serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "hello array"}]}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.is_array(), "JSON-array stream relayed as an array");

    // Settle: give a (wrong) async recording a chance to land, then
    // assert nothing did — the JSON-array shape is out of capture scope.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(vcr.fixture_count(), 0, "JSON-array stream never records");
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert!(fixtures.is_empty(), "cassette stays empty");
}

#[tokio::test]
async fn should_record_responses_stream() {
    let upstream = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("responses streamed"))
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("responses_stream");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!("{}/v1/responses", vcr.url());
    let req = serde_json::json!({
        "model": "gpt-test",
        "input": "stream responses",
        "stream": true
    });

    let resp = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(
        text.contains("response.completed"),
        "completed event relayed: {}",
        text
    );

    wait_until(|| vcr.fixture_count() == 1, "responses stream recording").await;
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(
        fixtures[0].response.as_ref().unwrap().content.as_deref(),
        Some("responses streamed"),
        "content extracted from the response.completed event"
    );

    // Replay from the recorded fixture.
    let resp2 = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
    assert_eq!(upstream.request_count(), 1);
}

#[tokio::test]
async fn should_record_completions_stream() {
    let upstream = ServerBuilder::new()
        .fixture(Fixture::new().respond_with_content("legacy streamed"))
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("completions_stream");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let url = format!("{}/v1/completions", vcr.url());
    let req = serde_json::json!({
        "model": "davinci-test",
        "prompt": "legacy stream",
        "stream": true
    });

    let resp = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(text.contains("data: [DONE]"), "completions SSE: {}", text);

    wait_until(|| vcr.fixture_count() == 1, "completions stream recording").await;
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(
        fixtures[0].response.as_ref().unwrap().content.as_deref(),
        Some("legacy streamed"),
        "text fragments reassembled"
    );

    // Replay from the recorded fixture.
    let resp2 = client.post(&url).json(&req).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
    assert_eq!(upstream.request_count(), 1);
}

#[tokio::test]
async fn should_not_record_truncated_stream() {
    // Upstream truncates after 2 SSE frames — the client sees the
    // truncated stream, and the missing [DONE] sentinel means the
    // recording is discarded.
    let mut truncated = Fixture::new()
        .match_user_message("truncate me")
        .respond_with_content("this content never fully arrives")
        .with_streaming(None, Some(4));
    truncated.failure = Some(FailureConfig {
        truncate_after_frames: Some(2),
        ..Default::default()
    });
    let upstream = ServerBuilder::new()
        .fixture(truncated)
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("truncated_stream");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::RecordOnMiss)
        .proxy_openai(&upstream.url())
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let mut req = openai_chat_body("truncate me");
    req["stream"] = serde_json::json!(true);
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", vcr.url()))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(
        !text.contains("[DONE]"),
        "client sees the truncated stream verbatim: {}",
        text
    );

    // The tee task pushes its capture entry strictly AFTER the persist
    // decision, so the capture landing proves the (non-)recording is
    // final — no sleep race.
    wait_until(|| vcr.request_count() == 1, "truncated stream capture").await;
    assert_eq!(vcr.fixture_count(), 0, "truncated stream never records");
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert!(fixtures.is_empty(), "cassette stays empty");
}

#[tokio::test]
async fn should_relay_rate_limit_headers() {
    let upstream = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .with_error_headers(
                    429,
                    "slow down",
                    [
                        ("x-ratelimit-remaining-requests", "3"),
                        ("anthropic-ratelimit-requests-remaining", "5"),
                    ],
                )
                .unwrap(),
        )
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("rate_limit_headers");
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
    assert_eq!(resp.status(), 429);
    assert_eq!(
        resp.headers()
            .get("x-ratelimit-remaining-requests")
            .and_then(|v| v.to_str().ok()),
        Some("3"),
        "upstream x-ratelimit-* family relayed (real budget, not mock values)"
    );
    assert_eq!(
        resp.headers()
            .get("anthropic-ratelimit-requests-remaining")
            .and_then(|v| v.to_str().ok()),
        Some("5"),
        "upstream anthropic-ratelimit-* family relayed"
    );
}

#[tokio::test]
async fn should_preserve_upstream_x_request_id_on_relayed_response() {
    // The upstream response carries its own x-request-id (set here via an
    // error fixture's headers map); the relay must NOT clobber it with a
    // llmposter-generated one — it is the only correlation handle back to
    // the provider's logs.
    let upstream = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .with_error_headers(500, "exploded", [("x-request-id", "upstream-req-id-123")])
                .unwrap(),
        )
        .build()
        .await
        .unwrap();
    let cassette = fresh_cassette("upstream_request_id");
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
    assert_eq!(resp.status(), 500);
    assert_eq!(
        resp.headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok()),
        Some("upstream-req-id-123"),
        "upstream x-request-id survives both the upstream middleware and the relay"
    );
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
// --- Raw streaming upstream: hand-rolled chunked SSE so tests can -------
// --- control exactly how the stream ends (clean terminal chunk vs -------
// --- abrupt close) and how large it grows. ------------------------------

/// Spawn a raw HTTP upstream that answers every request with a chunked
/// `text/event-stream` body: one chunk per frame, `frame_delay_ms`
/// between frames. `clean_end` sends the terminating zero chunk;
/// `false` closes the socket mid-stream instead (a transport error for
/// the downstream reader).
async fn spawn_raw_sse_upstream(
    frames: Vec<String>,
    frame_delay_ms: u64,
    clean_end: bool,
) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let frames = frames.clone();
            tokio::spawn(async move {
                // Drain the request head + content-length body.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                let header_end = loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
                let content_length = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                while buf.len() < header_end + content_length {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                if sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                          transfer-encoding: chunked\r\n\r\n",
                    )
                    .await
                    .is_err()
                {
                    return;
                }
                for (i, frame) in frames.iter().enumerate() {
                    if i > 0 && frame_delay_ms > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(frame_delay_ms)).await;
                    }
                    let chunk = format!("{:x}\r\n{}\r\n", frame.len(), frame);
                    if sock.write_all(chunk.as_bytes()).await.is_err() {
                        return; // downstream hung up — stop streaming
                    }
                }
                if clean_end {
                    let _ = sock.write_all(b"0\r\n\r\n").await;
                }
                let _ = sock.shutdown().await;
            });
        }
    });
    format!("http://{}", addr)
}

/// A minimal complete OpenAI SSE stream: `n` content deltas, a stop
/// frame, then the `[DONE]` sentinel.
fn openai_sse_frames(n: usize) -> Vec<String> {
    let mut frames: Vec<String> = (0..n)
        .map(|i| {
            format!(
                "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"part{} \"}},\"finish_reason\":null}}]}}\n\n",
                i
            )
        })
        .collect();
    frames.push(
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n"
            .to_string(),
    );
    frames.push("data: [DONE]\n\n".to_string());
    frames
}

#[tokio::test]
async fn should_relay_but_not_record_stream_past_capture_cap() {
    // A well-formed stream larger than the 16 MiB capture cap: the
    // recording is abandoned, but the RELAY must deliver every byte to
    // the connected client, including the [DONE] sentinel.
    let padding = "x".repeat(1024 * 1024);
    let mut frames: Vec<String> = (0..17).map(|_| format!("data: {}\n\n", padding)).collect();
    frames.push("data: [DONE]\n\n".to_string());
    let upstream = spawn_raw_sse_upstream(frames, 0, true).await;

    let cassette = fresh_cassette("cap_relay");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::Record)
        .proxy_openai(&upstream)
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let mut req = openai_chat_body("giant stream");
    req["stream"] = serde_json::json!(true);
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", vcr.url()))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(
        text.len() > 16 * 1024 * 1024,
        "full body relayed past the cap: {} bytes",
        text.len()
    );
    assert!(
        text.ends_with("data: [DONE]\n\n"),
        "stream relayed to the end"
    );

    wait_until(|| vcr.request_count() == 1, "cap-exceeded capture").await;
    assert_eq!(vcr.fixture_count(), 0, "over-cap stream never records");
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert!(fixtures.is_empty(), "cassette stays empty");
}

#[tokio::test]
async fn should_surface_mid_stream_upstream_failure_and_not_record() {
    // Upstream dies mid-stream (no terminal chunk): the tee must inject
    // a REAL transport error for the client — not a clean-looking end —
    // and must never record the partial stream.
    let frames = vec![
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"par\"},\"finish_reason\":null}]}\n\n".to_string(),
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"tial\"},\"finish_reason\":null}]}\n\n".to_string(),
    ];
    let upstream = spawn_raw_sse_upstream(frames, 0, false).await;

    let cassette = fresh_cassette("midstream_error");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::Record)
        .proxy_openai(&upstream)
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let mut req = openai_chat_body("doomed stream");
    req["stream"] = serde_json::json!(true);
    let mut resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", vcr.url()))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "headers arrive before the failure");
    let mut saw_error = false;
    loop {
        match resp.chunk().await {
            Ok(Some(_)) => continue,
            Ok(None) => break,
            Err(_) => {
                saw_error = true;
                break;
            }
        }
    }
    assert!(
        saw_error,
        "client must see a transport error, not a clean end"
    );

    wait_until(|| vcr.request_count() == 1, "mid-stream failure capture").await;
    assert_eq!(vcr.fixture_count(), 0, "failed stream never records");
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert!(fixtures.is_empty(), "cassette stays empty");
}

#[tokio::test]
async fn should_finish_recording_after_client_disconnects_mid_stream() {
    // The client hangs up after the first frame; the tee keeps draining
    // the upstream so the recording still completes.
    let upstream = spawn_raw_sse_upstream(openai_sse_frames(5), 30, true).await;

    let cassette = fresh_cassette("client_disconnect_salvage");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::Record)
        .proxy_openai(&upstream)
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let mut req = openai_chat_body("salvage me");
    req["stream"] = serde_json::json!(true);
    let mut resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", vcr.url()))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let first = resp.chunk().await.unwrap();
    assert!(first.is_some(), "at least one frame reaches the client");
    drop(resp); // client disconnects mid-stream

    wait_until(|| vcr.fixture_count() == 1, "salvaged recording").await;
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert_eq!(fixtures.len(), 1);
    assert_eq!(
        fixtures[0].response.as_ref().unwrap().content.as_deref(),
        Some("part0 part1 part2 part3 part4 "),
        "the FULL upstream stream is recorded despite the disconnect"
    );
}

#[tokio::test]
async fn should_stop_draining_when_client_gone_and_cap_exceeded() {
    // Client disconnects AND the salvage buffer blows the 16 MiB cap:
    // with nothing left to relay or record, the tee must stop draining
    // the (effectively endless) upstream instead of pulling forever.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        // Drain the request head; body is small enough to arrive with it.
        let mut tmp = [0u8; 4096];
        let _ = sock.read(&mut tmp).await;
        let _ = sock
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                  transfer-encoding: chunked\r\n\r\n",
            )
            .await;
        let frame = format!("data: {}\n\n", "y".repeat(1024 * 1024));
        let chunk = format!("{:x}\r\n{}\r\n", frame.len(), frame);
        // Stream "forever" — until the tee drops the connection.
        loop {
            if sock.write_all(chunk.as_bytes()).await.is_err() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let _ = done_tx.send(());
    });

    let cassette = fresh_cassette("cap_break_drain");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::Record)
        .proxy_openai(&format!("http://{}", addr))
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let mut req = openai_chat_body("endless stream");
    req["stream"] = serde_json::json!(true);
    let mut resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", vcr.url()))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let first = resp.chunk().await.unwrap();
    assert!(first.is_some());
    drop(resp); // client gone; upstream keeps pumping toward the cap

    // The upstream write loop errors out once the tee hangs up — proof
    // the drain stopped at the cap instead of running forever.
    tokio::time::timeout(std::time::Duration::from_secs(30), done_rx)
        .await
        .expect("tee must drop the upstream connection after the cap")
        .unwrap();
    assert_eq!(vcr.fixture_count(), 0, "nothing recorded");
    let fixtures = llmposter::fixture::load_yaml_file(&cassette).unwrap();
    assert!(fixtures.is_empty(), "cassette stays empty");
}

#[tokio::test]
async fn should_return_502_when_upstream_body_read_fails() {
    // Non-streaming: upstream promises 10000 bytes but closes after a
    // fragment — reading the body fails after the 200 head, and the
    // client gets the provider-shaped 502.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut tmp = [0u8; 4096];
                let _ = sock.read(&mut tmp).await;
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                          content-length: 10000\r\n\r\n{\"partial\":",
                    )
                    .await;
                let _ = sock.shutdown().await;
            });
        }
    });

    let cassette = fresh_cassette("body_read_fails");
    let vcr = ServerBuilder::new()
        .vcr_mode(VcrMode::Record)
        .proxy_openai(&format!("http://{}", addr))
        .record_file(&cassette)
        .build()
        .await
        .unwrap();

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", vcr.url()))
        .json(&openai_chat_body("short body"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 502);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unreachable"),
        "502 names the upstream failure: {}",
        body
    );
    assert_eq!(vcr.fixture_count(), 0, "nothing recorded");
}

// --- build()'s cassette default fallback (no record_file given) ---------

#[tokio::test]
async fn should_default_cassette_next_to_file_source() {
    let dir = std::env::temp_dir().join(format!(
        "llmposter_cassette_default_file_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("fixtures.yaml");
    std::fs::write(
        &file,
        "fixtures:\n  - match:\n      user_message: hi\n    response:\n      content: hello",
    )
    .unwrap();

    let server = ServerBuilder::new()
        .load_yaml(&file)
        .unwrap()
        .vcr_mode(VcrMode::Record)
        .build()
        .await
        .unwrap();
    assert!(
        dir.join("recorded.yaml").exists(),
        "cassette defaults to recorded.yaml NEXT TO the fixture file"
    );
    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn should_default_cassette_inside_dir_source() {
    let dir = std::env::temp_dir().join(format!(
        "llmposter_cassette_default_dir_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("fixtures.yaml"),
        "fixtures:\n  - match:\n      user_message: hi\n    response:\n      content: hello",
    )
    .unwrap();

    let server = ServerBuilder::new()
        .load_yaml_dir(&dir)
        .unwrap()
        .vcr_mode(VcrMode::Record)
        .build()
        .await
        .unwrap();
    assert!(
        dir.join("recorded.yaml").exists(),
        "cassette defaults to recorded.yaml INSIDE the fixture directory"
    );
    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn should_default_cassette_to_cwd_with_no_sources() {
    // With no fixture sources at all the cassette falls back to
    // ./recorded.yaml. cargo sets the test cwd to the crate root, and
    // changing cwd is process-global (unsafe with parallel tests), so
    // this test creates and removes the file in place. The Drop guard
    // cleans up even if an assertion panics.
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file("recorded.yaml");
        }
    }
    let _ = std::fs::remove_file("recorded.yaml"); // stale artifact from a crashed run
    let _guard = Cleanup;

    let server = ServerBuilder::new()
        .vcr_mode(VcrMode::Record)
        .build()
        .await
        .unwrap();
    assert!(
        std::path::Path::new("recorded.yaml").exists(),
        "cassette defaults to ./recorded.yaml when no fixture source exists"
    );
    drop(server);
}
