#![cfg(feature = "record")]
use llmposter::{Fixture, ServerBuilder, VcrMode};

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
