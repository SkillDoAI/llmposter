//! Hot-reload tests: fixture swap at runtime via `MockServer::set_fixtures()`,
//! and file-watch mode via `ServerBuilder::watch()`.

use llmposter::{Fixture, ServerBuilder};

async fn post_user_message(url: &str, msg: &str) -> serde_json::Value {
    reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", url))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": msg}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn should_swap_fixtures_via_set_fixtures() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("original"),
        )
        .build()
        .await
        .unwrap();

    let body = post_user_message(&server.url(), "hello").await;
    assert_eq!(body["choices"][0]["message"]["content"], "original");

    // Swap at runtime
    server
        .set_fixtures(vec![Fixture::new()
            .match_user_message("hello")
            .respond_with_content("updated")])
        .expect("valid fixtures should swap");

    let body = post_user_message(&server.url(), "hello").await;
    assert_eq!(body["choices"][0]["message"]["content"], "updated");
}

#[tokio::test]
async fn should_keep_old_fixtures_when_swap_is_invalid() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("original"),
        )
        .build()
        .await
        .unwrap();

    // Swap with a fixture that has neither response nor error — invalid.
    let result = server.set_fixtures(vec![Fixture::new()]);
    assert!(result.is_err(), "invalid swap should error");

    // Old fixture still serving
    let body = post_user_message(&server.url(), "hello").await;
    assert_eq!(body["choices"][0]["message"]["content"], "original");
}

#[tokio::test]
async fn should_swap_empty_fixtures_and_return_404() {
    let server = ServerBuilder::new()
        .fixture(
            Fixture::new()
                .match_user_message("hello")
                .respond_with_content("original"),
        )
        .build()
        .await
        .unwrap();

    server
        .set_fixtures(Vec::new())
        .expect("empty fixtures swap should succeed");

    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", server.url()))
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

/// Creates a unique temp file path and writes the given YAML.
fn write_temp_yaml(name: &str, content: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("llmposter_hot_reload_tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}-{}.yaml", name, std::process::id()));
    std::fs::write(&path, content).unwrap();
    path
}

#[cfg(feature = "watch")]
#[tokio::test]
async fn should_reload_fixtures_on_file_change_when_watching() {
    let path = write_temp_yaml(
        "watch_change",
        "fixtures:\n  - match:\n      user_message: hello\n    response:\n      content: original\n",
    );

    let server = ServerBuilder::new()
        .load_yaml(&path)
        .unwrap()
        .watch(true)
        .build()
        .await
        .unwrap();

    let body = post_user_message(&server.url(), "hello").await;
    assert_eq!(body["choices"][0]["message"]["content"], "original");

    // Modify the file
    std::fs::write(
        &path,
        "fixtures:\n  - match:\n      user_message: hello\n    response:\n      content: reloaded\n",
    )
    .unwrap();

    // Wait for debounce (250ms) + parse + swap
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    let body = post_user_message(&server.url(), "hello").await;
    assert_eq!(body["choices"][0]["message"]["content"], "reloaded");

    let _ = std::fs::remove_file(&path);
}

#[cfg(feature = "watch")]
#[tokio::test]
async fn should_keep_old_fixtures_when_reloaded_yaml_is_invalid() {
    let path = write_temp_yaml(
        "watch_invalid",
        "fixtures:\n  - match:\n      user_message: hello\n    response:\n      content: original\n",
    );

    let server = ServerBuilder::new()
        .load_yaml(&path)
        .unwrap()
        .watch(true)
        .build()
        .await
        .unwrap();

    let body = post_user_message(&server.url(), "hello").await;
    assert_eq!(body["choices"][0]["message"]["content"], "original");

    // Write garbage YAML — reload should fail and old fixtures should keep serving.
    std::fs::write(&path, "fixtures: not a list\n").unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    let body = post_user_message(&server.url(), "hello").await;
    assert_eq!(body["choices"][0]["message"]["content"], "original");

    let _ = std::fs::remove_file(&path);
}

#[cfg(feature = "watch")]
#[tokio::test]
async fn should_log_verbose_reload_on_file_change() {
    let path = write_temp_yaml(
        "watch_verbose",
        "fixtures:\n  - match:\n      user_message: hi\n    response:\n      content: v1\n",
    );

    let server = ServerBuilder::new()
        .load_yaml(&path)
        .unwrap()
        .watch(true)
        .verbose(true)
        .build()
        .await
        .unwrap();

    std::fs::write(
        &path,
        "fixtures:\n  - match:\n      user_message: hi\n    response:\n      content: v2\n",
    )
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    let body = post_user_message(&server.url(), "hi").await;
    assert_eq!(body["choices"][0]["message"]["content"], "v2");

    let _ = std::fs::remove_file(&path);
}

#[cfg(feature = "watch")]
#[tokio::test]
async fn should_keep_old_fixtures_when_reloaded_yaml_has_invalid_fixture() {
    // YAML parses successfully but a fixture inside has no response or error —
    // triggers the `set_fixtures` validation-error branch in reload_and_swap.
    let path = write_temp_yaml(
        "watch_validation",
        "fixtures:\n  - match:\n      user_message: hi\n    response:\n      content: good\n",
    );

    let server = ServerBuilder::new()
        .load_yaml(&path)
        .unwrap()
        .watch(true)
        .build()
        .await
        .unwrap();

    let body = post_user_message(&server.url(), "hi").await;
    assert_eq!(body["choices"][0]["message"]["content"], "good");

    // Empty-match fixture with no response and no error — parses fine, fails validate().
    std::fs::write(&path, "fixtures:\n  - match:\n      user_message: hi\n").unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    // Old fixture still serving.
    let body = post_user_message(&server.url(), "hi").await;
    assert_eq!(body["choices"][0]["message"]["content"], "good");

    let _ = std::fs::remove_file(&path);
}

#[cfg(feature = "watch")]
#[tokio::test]
async fn should_survive_watcher_setup_failure_on_nonexistent_path() {
    // Point the watcher at a path that doesn't exist — the watch() call will
    // fail, the spawn function will log and return, but the server should
    // still come up and serve the in-memory fixtures from load_yaml (which
    // loaded successfully from the initial valid path).
    //
    // To reach `debouncer.watcher().watch(&nonexistent, ...)` we need a source
    // list where the source was deleted between load_yaml and build().
    let path = write_temp_yaml(
        "watch_vanish",
        "fixtures:\n  - match:\n      user_message: hi\n    response:\n      content: loaded\n",
    );
    let builder = ServerBuilder::new().load_yaml(&path).unwrap().watch(true);
    // Delete before build so the watcher setup sees a missing file.
    std::fs::remove_file(&path).unwrap();

    let server = builder.build().await.unwrap();

    // Server still serves the in-memory fixture.
    let body = post_user_message(&server.url(), "hi").await;
    assert_eq!(body["choices"][0]["message"]["content"], "loaded");
}

#[cfg(feature = "watch")]
#[tokio::test]
async fn should_reload_when_watching_a_directory() {
    let dir = std::env::temp_dir().join(format!("llmposter_watch_dir_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.yaml");
    std::fs::write(
        &file,
        "fixtures:\n  - match:\n      user_message: hello\n    response:\n      content: dir-original\n",
    )
    .unwrap();

    let server = ServerBuilder::new()
        .load_yaml_dir(&dir)
        .unwrap()
        .watch(true)
        .build()
        .await
        .unwrap();

    let body = post_user_message(&server.url(), "hello").await;
    assert_eq!(body["choices"][0]["message"]["content"], "dir-original");

    std::fs::write(
        &file,
        "fixtures:\n  - match:\n      user_message: hello\n    response:\n      content: dir-reloaded\n",
    )
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    let body = post_user_message(&server.url(), "hello").await;
    assert_eq!(body["choices"][0]["message"]["content"], "dir-reloaded");

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
async fn should_reload_fixtures_on_sighup() {
    let path = write_temp_yaml(
        "sighup",
        "fixtures:\n  - match:\n      user_message: hello\n    response:\n      content: original\n",
    );

    // No .watch() — SIGHUP is always on for file-backed fixtures.
    let server = ServerBuilder::new()
        .load_yaml(&path)
        .unwrap()
        .build()
        .await
        .unwrap();

    let body = post_user_message(&server.url(), "hello").await;
    assert_eq!(body["choices"][0]["message"]["content"], "original");

    // Rewrite the file, then SIGHUP ourselves to trigger reload.
    std::fs::write(
        &path,
        "fixtures:\n  - match:\n      user_message: hello\n    response:\n      content: sighup-reloaded\n",
    )
    .unwrap();

    // SAFETY: sending SIGHUP to our own PID is a well-defined Unix operation.
    // The signal handler installed by spawn_sighup_handler will re-read the file.
    unsafe {
        libc::kill(libc::getpid(), libc::SIGHUP);
    }

    // Let the signal handler task run + reload complete.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let body = post_user_message(&server.url(), "hello").await;
    assert_eq!(body["choices"][0]["message"]["content"], "sighup-reloaded");

    let _ = std::fs::remove_file(&path);
}
