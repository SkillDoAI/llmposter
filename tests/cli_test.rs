use llmposter::cli::{run, run_with_output, Cli};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "{}_{}_{}",
        prefix,
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fixtures_dir() -> PathBuf {
    let dir = unique_temp_dir("llmposter_cli_test");
    std::fs::write(
        dir.join("test.yaml"),
        "fixtures:\n  - match:\n      user_message: hello\n    response:\n      content: world",
    )
    .unwrap();
    dir
}

fn empty_dir() -> PathBuf {
    unique_temp_dir("llmposter_cli_test_empty")
}

#[tokio::test]
async fn should_validate_good_fixtures() {
    let cli = Cli {
        fixtures: fixtures_dir(),
        validate: true,
        port: 0,
        bind: "127.0.0.1".to_string(),
        verbose: false,
    };
    let result = run(&cli).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none()); // validate returns None (no server)
}

#[tokio::test]
async fn should_fail_validate_empty_dir() {
    let cli = Cli {
        fixtures: empty_dir(),
        validate: true,
        port: 0,
        bind: "127.0.0.1".to_string(),
        verbose: false,
    };
    let result = run(&cli).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("No fixtures found"));
}

#[tokio::test]
async fn should_fail_nonexistent_path() {
    let cli = Cli {
        fixtures: unique_temp_dir("llmposter_cli_test_missing").join("fixtures.yaml"),
        validate: false,
        port: 0,
        bind: "127.0.0.1".to_string(),
        verbose: false,
    };
    let result = run(&cli).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn should_start_server_and_respond() {
    let cli = Cli {
        fixtures: fixtures_dir(),
        validate: false,
        port: 0,
        bind: "127.0.0.1".to_string(),
        verbose: false,
    };
    let result = run(&cli).await;
    assert!(result.is_ok());
    let server = result.unwrap().expect("should return server");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "world");
}

#[tokio::test]
async fn should_start_server_with_verbose() {
    let cli = Cli {
        fixtures: fixtures_dir(),
        validate: false,
        port: 0,
        bind: "127.0.0.1".to_string(),
        verbose: true,
    };
    let result = run(&cli).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_some());
}

#[tokio::test]
async fn should_validate_single_file() {
    let dir = fixtures_dir();
    let file = dir.join("test.yaml");
    let cli = Cli {
        fixtures: file,
        validate: true,
        port: 0,
        bind: "127.0.0.1".to_string(),
        verbose: false,
    };
    let result = run(&cli).await;
    assert!(result.is_ok());
}

// ===========================================================================
// Output capture tests using run_with_output
// ===========================================================================

#[tokio::test]
async fn should_output_validated_message() {
    let cli = Cli {
        fixtures: fixtures_dir(),
        validate: true,
        port: 0,
        bind: "127.0.0.1".to_string(),
        verbose: false,
    };
    let mut output = Vec::new();
    let result = run_with_output(&cli, &mut output).await;
    assert!(result.is_ok());

    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains("Validated 1 fixtures successfully"),
        "expected validation message, got: {}",
        text
    );
}

#[tokio::test]
async fn should_output_listening_message() {
    let cli = Cli {
        fixtures: fixtures_dir(),
        validate: false,
        port: 0,
        bind: "127.0.0.1".to_string(),
        verbose: false,
    };
    let mut output = Vec::new();
    let result = run_with_output(&cli, &mut output).await;
    assert!(result.is_ok());

    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains("llmposter listening on"),
        "expected listening message, got: {}",
        text
    );
    assert!(
        text.contains("Press Ctrl+C to stop"),
        "expected Ctrl+C hint, got: {}",
        text
    );
}

#[tokio::test]
async fn should_output_empty_fixtures_warning() {
    // Create a dir with a valid YAML that has no fixtures
    let dir = unique_temp_dir("llmposter_cli_test_warn");
    std::fs::write(dir.join("empty.yaml"), "fixtures: []").unwrap();

    let cli = Cli {
        fixtures: dir,
        validate: false,
        port: 0,
        bind: "127.0.0.1".to_string(),
        verbose: false,
    };
    let mut output = Vec::new();
    let result = run_with_output(&cli, &mut output).await;
    assert!(result.is_ok());

    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains("Warning: no fixtures loaded"),
        "expected warning, got: {}",
        text
    );
    std::fs::remove_dir_all(&cli.fixtures).ok();
}

#[tokio::test]
async fn should_bind_to_ipv6_address() {
    let dir = fixtures_dir();
    let cli = Cli {
        fixtures: dir.clone(),
        validate: false,
        port: 0, // random port
        bind: "::1".to_string(),
        verbose: false,
    };
    let mut output = Vec::new();
    let result = run_with_output(&cli, &mut output).await;
    // Skip gracefully if IPv6 is not available on this host (e.g. Docker, some CI),
    // but fail hard on bind-address formatting bugs (regression guard).
    if let Err(ref e) = result {
        let msg = e.to_string();
        assert!(
            !msg.contains("invalid") && !msg.contains("malformed"),
            "IPv6 bind address was malformed (not a host issue): {msg}"
        );
        eprintln!("skipping: IPv6 not available on this host: {msg}");
        std::fs::remove_dir_all(&dir).ok();
        return;
    }
    let server = result.unwrap().unwrap();
    let url = server.url();
    // IPv6 URL should contain [::1]
    assert!(url.contains("[::1]"), "expected IPv6 URL, got: {}", url);
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn should_warn_on_empty_fixtures_dir() {
    let dir = unique_temp_dir("llmposter_cli_empty");
    // Empty dir — no YAML files
    let cli = Cli {
        fixtures: dir.clone(),
        validate: false,
        port: 0,
        bind: "127.0.0.1".to_string(),
        verbose: false,
    };
    let mut buf = Vec::new();
    let result = run_with_output(&cli, &mut buf).await;
    let output = String::from_utf8_lossy(&buf);
    assert!(
        output.contains("Warning: no fixtures loaded"),
        "expected empty-dir warning, got: {}",
        output
    );
    // Server still starts (just with no fixtures)
    assert!(result.is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn should_accept_non_ip_bind_address() {
    let dir = fixtures_dir();
    let cli = Cli {
        fixtures: dir.clone(),
        validate: false,
        port: 0,
        bind: "localhost".to_string(),
        verbose: false,
    };
    let mut buf = Vec::new();
    let result = run_with_output(&cli, &mut buf).await;
    // "localhost" is not parseable as IpAddr, so hits the fallback format path
    assert!(result.is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn should_accept_socket_address_with_embedded_port() {
    let dir = fixtures_dir();
    let cli = Cli {
        fixtures: dir.clone(),
        validate: false,
        port: 9999, // should be ignored when bind is a full socket address
        bind: "127.0.0.1:0".to_string(),
        verbose: false,
    };
    let mut buf = Vec::new();
    let result = run_with_output(&cli, &mut buf).await;
    assert!(result.is_ok());
    let output = String::from_utf8_lossy(&buf);
    // Port should NOT be 9999 — the embedded :0 means OS-assigned
    assert!(
        !output.contains(":9999"),
        "embedded port should take precedence over --port"
    );
    std::fs::remove_dir_all(&dir).ok();
}
