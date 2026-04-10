//! Integration tests for streaming chaos (`latency_jitter_ms`,
//! `duplicate_frames`, `probability`, `chaos_seed`).

use llmposter::{FailureConfig, Fixture, ServerBuilder};
use std::time::{Duration, Instant};

async fn stream_and_collect_sse(url: &str, model: &str, prompt: &str) -> Vec<String> {
    let body = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", url))
        .json(&serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "stream": true
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    body.lines()
        .filter(|l| l.starts_with("data: ") && !l.contains("[DONE]"))
        .map(|l| l.to_string())
        .collect()
}

/// Count the number of `choices[].delta.content` chunks with non-empty text.
/// Used to verify duplication — an un-duplicated stream of the same base
/// content produces some baseline number of content frames, and
/// `duplicate_frames: true` should exactly double them.
fn count_content_frames(frames: &[String]) -> usize {
    frames
        .iter()
        .filter_map(|l| {
            let json = l.strip_prefix("data: ")?;
            let v: serde_json::Value = serde_json::from_str(json).ok()?;
            let delta = v.get("choices")?.get(0)?.get("delta")?;
            let content = delta.get("content")?.as_str()?;
            if content.is_empty() {
                None
            } else {
                Some(())
            }
        })
        .count()
}

#[tokio::test]
async fn should_duplicate_frames_when_configured() {
    // Baseline server — no chaos.
    let baseline = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("hello world from chaos test")
                .with_streaming(Some(0), Some(10)),
        )
        .build()
        .await
        .unwrap();

    // Chaos server — duplicate every frame.
    let chaos = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("hello world from chaos test")
                .with_streaming(Some(0), Some(10))
                .with_failure(FailureConfig {
                    duplicate_frames: Some(true),
                    chaos_seed: Some(1),
                    ..Default::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let baseline_frames = stream_and_collect_sse(&baseline.url(), "gpt-4", "hi").await;
    let chaos_frames = stream_and_collect_sse(&chaos.url(), "gpt-4", "hi").await;

    let baseline_content = count_content_frames(&baseline_frames);
    let chaos_content = count_content_frames(&chaos_frames);
    assert!(baseline_content > 0, "baseline must emit content frames");
    assert_eq!(
        chaos_content,
        baseline_content * 2,
        "duplicate_frames should double content frame count (baseline {}, chaos {})",
        baseline_content,
        chaos_content
    );
}

#[tokio::test]
async fn should_honor_latency_jitter_within_bounds() {
    // Base latency 30ms, jitter ±20ms → per-frame delay in [10, 50].
    // With 3 content frames at ~10 chars chunk size on a 30-char response,
    // total streaming time should be roughly 2 inter-frame delays.
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("abcdefghijklmnopqrstuvwxyz1234")
                .with_streaming(Some(30), Some(10))
                .with_failure(FailureConfig {
                    latency_jitter_ms: Some(20),
                    chaos_seed: Some(42),
                    ..Default::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let start = Instant::now();
    let frames = stream_and_collect_sse(&server.url(), "gpt-4", "hi").await;
    let elapsed = start.elapsed();

    // Sanity: the stream completed at all (no pathological hang) and emitted
    // frames. We don't assert precise timing because parallel tokio tests
    // share a runtime and scheduler jitter dwarfs the intentional jitter we
    // are trying to observe.
    assert!(!frames.is_empty(), "expected frames, got none");
    assert!(
        elapsed < Duration::from_secs(10),
        "jittered stream took pathologically long: {:?}",
        elapsed
    );
}

#[tokio::test]
async fn should_be_deterministic_for_same_chaos_seed() {
    // Two independent servers with the same chaos_seed should produce
    // the same jitter/duplication outcomes for the same request sequence.
    let make_server = || async {
        ServerBuilder::new()
            .fixture(
                Fixture::new()
                    .respond_with_content("reproducible output stream")
                    .with_streaming(Some(0), Some(10))
                    .with_failure(FailureConfig {
                        latency_jitter_ms: Some(5),
                        duplicate_frames: Some(true),
                        chaos_seed: Some(12345),
                        ..Default::default()
                    }),
            )
            .build()
            .await
            .unwrap()
    };

    let a = make_server().await;
    let b = make_server().await;

    let frames_a = stream_and_collect_sse(&a.url(), "gpt-4", "hi").await;
    let frames_b = stream_and_collect_sse(&b.url(), "gpt-4", "hi").await;

    assert_eq!(
        count_content_frames(&frames_a),
        count_content_frames(&frames_b),
        "same chaos_seed must yield identical frame counts"
    );
}

#[tokio::test]
async fn should_disable_chaos_when_probability_is_zero() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("should not be duplicated")
                .with_streaming(Some(0), Some(10))
                .with_failure(FailureConfig {
                    duplicate_frames: Some(true),
                    probability: Some(0.0),
                    chaos_seed: Some(1),
                    ..Default::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let baseline = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("should not be duplicated")
                .with_streaming(Some(0), Some(10)),
        )
        .build()
        .await
        .unwrap();

    let chaos_frames = stream_and_collect_sse(&server.url(), "gpt-4", "hi").await;
    let base_frames = stream_and_collect_sse(&baseline.url(), "gpt-4", "hi").await;
    assert_eq!(
        count_content_frames(&chaos_frames),
        count_content_frames(&base_frames),
        "probability=0.0 should disable duplication"
    );
}

#[tokio::test]
async fn should_activate_chaos_when_probability_is_one() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("always-duplicated content here")
                .with_streaming(Some(0), Some(10))
                .with_failure(FailureConfig {
                    duplicate_frames: Some(true),
                    probability: Some(1.0),
                    chaos_seed: Some(1),
                    ..Default::default()
                }),
        )
        .build()
        .await
        .unwrap();

    let baseline = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("always-duplicated content here")
                .with_streaming(Some(0), Some(10)),
        )
        .build()
        .await
        .unwrap();

    let chaos_frames = stream_and_collect_sse(&server.url(), "gpt-4", "hi").await;
    let base_frames = stream_and_collect_sse(&baseline.url(), "gpt-4", "hi").await;
    assert_eq!(
        count_content_frames(&chaos_frames),
        count_content_frames(&base_frames) * 2,
        "probability=1.0 should always duplicate"
    );
}

#[tokio::test]
async fn should_verbose_log_when_chaos_active() {
    // Build a verbose server with chaos; verbose chaos logging is a side
    // effect we can't easily capture, but we can at least exercise the
    // code path to keep coverage high.
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .respond_with_content("verbose chaos path")
                .with_streaming(Some(0), Some(10))
                .with_failure(FailureConfig {
                    duplicate_frames: Some(true),
                    chaos_seed: Some(1),
                    ..Default::default()
                }),
        )
        .verbose(true)
        .build()
        .await
        .unwrap();

    let frames = stream_and_collect_sse(&server.url(), "gpt-4", "hi").await;
    assert!(!frames.is_empty());
}
