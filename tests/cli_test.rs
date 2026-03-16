use llmposter::cli::{Cli, run};
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("llmposter_cli_test");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("test.yaml"),
        "fixtures:\n  - match:\n      user_message: hello\n    response:\n      content: world",
    )
    .unwrap();
    dir
}

fn empty_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("llmposter_cli_test_empty");
    std::fs::create_dir_all(&dir).unwrap();
    // Remove any leftover yaml files
    for entry in std::fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false) {
            std::fs::remove_file(entry.path()).ok();
        }
    }
    dir
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
    assert!(result.unwrap_err().to_string().contains("No fixtures found"));
}

#[tokio::test]
async fn should_fail_nonexistent_path() {
    let cli = Cli {
        fixtures: PathBuf::from("/nonexistent/path/fixtures.yaml"),
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
