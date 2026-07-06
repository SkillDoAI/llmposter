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

/// Baseline `Cli` with every feature-gated field at its off/default value.
/// Individual tests override the fields they care about via struct update
/// syntax (`..base_cli(fixtures)`).
fn base_cli(fixtures: PathBuf) -> Cli {
    Cli {
        fixtures,
        validate: false,
        port: 0,
        bind: "127.0.0.1".to_string(),
        verbose: false,
        #[cfg(feature = "watch")]
        watch: false,
        capture_capacity: 1000,
        diagnostics: false,
        #[cfg(feature = "ui")]
        ui: false,
        #[cfg(feature = "record")]
        vcr_mode: llmposter::record::VcrMode::Replay,
        #[cfg(feature = "record")]
        record_file: None,
        #[cfg(feature = "record")]
        proxy_openai: None,
        #[cfg(feature = "record")]
        proxy_anthropic: None,
        #[cfg(feature = "record")]
        proxy_gemini: None,
        #[cfg(feature = "record")]
        redact: Vec::new(),
        #[cfg(feature = "record")]
        allow_remote_record: false,
    }
}

#[tokio::test]
async fn should_validate_good_fixtures() {
    let cli = Cli {
        validate: true,
        ..base_cli(fixtures_dir())
    };
    let result = run(&cli).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none()); // validate returns None (no server)
}

#[tokio::test]
async fn should_fail_validate_empty_dir() {
    let cli = Cli {
        validate: true,
        ..base_cli(empty_dir())
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
    let cli = base_cli(unique_temp_dir("llmposter_cli_test_missing").join("fixtures.yaml"));
    let result = run(&cli).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn should_start_server_and_respond() {
    let cli = base_cli(fixtures_dir());
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

#[cfg(feature = "watch")]
#[tokio::test]
async fn should_start_server_with_watch_flag() {
    let cli = Cli {
        watch: true,
        ..base_cli(fixtures_dir())
    };
    let mut output = Vec::new();
    let result = run_with_output(&cli, &mut output).await;
    assert!(result.is_ok());
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains("Watching"),
        "expected 'Watching' line in output, got: {}",
        text
    );
}

#[cfg(unix)]
#[tokio::test]
async fn should_advertise_sighup_hint_in_cli_output() {
    let cli = base_cli(fixtures_dir());
    let mut output = Vec::new();
    let result = run_with_output(&cli, &mut output).await;
    assert!(result.is_ok());
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains("SIGHUP"),
        "expected SIGHUP hint, got: {}",
        text
    );
}

#[tokio::test]
async fn should_start_server_with_verbose() {
    let cli = Cli {
        verbose: true,
        ..base_cli(fixtures_dir())
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
        validate: true,
        ..base_cli(file)
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
        validate: true,
        ..base_cli(fixtures_dir())
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
    let cli = base_cli(fixtures_dir());
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

    let cli = base_cli(dir);
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
        bind: "::1".to_string(),
        ..base_cli(dir.clone())
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
    let cli = base_cli(dir.clone());
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
        bind: "localhost".to_string(),
        ..base_cli(dir.clone())
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
        port: 9999, // should be ignored when bind is a full socket address
        bind: "127.0.0.1:0".to_string(),
        ..base_cli(dir.clone())
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

#[tokio::test]
async fn should_warn_when_port_ignored_for_socket_addr_bind() {
    let dir = fixtures_dir();
    let cli = Cli {
        port: 5150, // non-default, should trigger warning
        bind: "127.0.0.1:0".to_string(),
        ..base_cli(dir.clone())
    };
    let mut buf = Vec::new();
    let result = run_with_output(&cli, &mut buf).await;
    assert!(result.is_ok());
    let output = String::from_utf8_lossy(&buf);
    assert!(
        output.contains("--port 5150 ignored"),
        "expected port-ignored warning, got: {}",
        output
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn should_accept_hostname_with_port() {
    let dir = fixtures_dir();
    let cli = Cli {
        port: 5150, // non-default, should trigger warning
        bind: "localhost:0".to_string(),
        ..base_cli(dir.clone())
    };
    let mut buf = Vec::new();
    let result = run_with_output(&cli, &mut buf).await;
    assert!(result.is_ok());
    let output = String::from_utf8_lossy(&buf);
    assert!(
        output.contains("--port 5150 ignored"),
        "expected port-ignored warning for hostname:port, got: {}",
        output
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn should_fallback_for_invalid_hostname_port() {
    let dir = fixtures_dir();
    // ":notaport" — rsplit gives host="" which fails the !host.is_empty() check
    let cli = Cli {
        bind: ":notaport".to_string(),
        ..base_cli(dir.clone())
    };
    let mut buf = Vec::new();
    // This will likely fail to bind (":notaport:0" is invalid), but the
    // bind_addr construction path is exercised either way.
    let _ = run_with_output(&cli, &mut buf).await;
    std::fs::remove_dir_all(&dir).ok();
}

// DEFAULT_PORT is 2112.  Tests below use port == 2112 so the "port ignored"
// condition is FALSE — exercising the closing `}` of those if-blocks.

#[tokio::test]
async fn should_not_warn_when_port_matches_default_with_socket_addr_bind() {
    let dir = fixtures_dir();
    let cli = Cli {
        port: 2112, // equals DEFAULT_PORT — condition is false, no warning
        bind: "127.0.0.1:0".to_string(),
        ..base_cli(dir.clone())
    };
    let mut buf = Vec::new();
    let result = run_with_output(&cli, &mut buf).await;
    assert!(result.is_ok());
    let output = String::from_utf8_lossy(&buf);
    assert!(
        !output.contains("--port 2112 ignored"),
        "should NOT warn when port equals default, got: {}",
        output
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn should_not_warn_when_port_matches_default_with_hostname_port() {
    let dir = fixtures_dir();
    let cli = Cli {
        port: 2112, // equals DEFAULT_PORT — condition is false, no warning
        bind: "localhost:0".to_string(),
        ..base_cli(dir.clone())
    };
    let mut buf = Vec::new();
    let result = run_with_output(&cli, &mut buf).await;
    assert!(result.is_ok());
    let output = String::from_utf8_lossy(&buf);
    assert!(
        !output.contains("--port 2112 ignored"),
        "should NOT warn when port equals default, got: {}",
        output
    );
    std::fs::remove_dir_all(&dir).ok();
}

// Writer that always fails — used to exercise the `)?;` error-propagation
// paths inside `writeln!` calls in run_with_output.
struct AlwaysFailWriter;
impl std::io::Write for AlwaysFailWriter {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "simulated write failure",
        ))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// Writer that succeeds for the first N complete `writeln!` calls (detected by
// newline bytes) then fails — used to exercise `)?;` error-propagation paths
// for writeln! calls that appear after the first output line. Only used by the
// unix-only SIGHUP test below; gate the struct + impl with the same cfg to
// avoid dead-code warnings on Windows.
#[cfg(unix)]
struct FailAfterNNewlines {
    completed: usize,
    limit: usize,
}
#[cfg(unix)]
impl std::io::Write for FailAfterNNewlines {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.completed >= self.limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "simulated write failure",
            ));
        }
        // Count newlines to track completed writeln! calls.
        let newlines = buf.iter().filter(|&&b| b == b'\n').count();
        self.completed += newlines;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn should_propagate_write_error_on_empty_fixtures_warning() {
    // With an empty fixture dir the first writeln! is the "no fixtures loaded"
    // warning at line 141-145.  AlwaysFailWriter makes that write fail, so the
    // `)?;` error-propagation path (line 145) is exercised.
    let dir = unique_temp_dir("llmposter_cli_test_fail_write");
    std::fs::write(dir.join("empty.yaml"), "fixtures: []").unwrap();
    let cli = base_cli(dir.clone());
    let mut writer = AlwaysFailWriter;
    let result = run_with_output(&cli, &mut writer).await;
    assert!(result.is_err(), "expected Err from write failure");
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(unix)]
#[tokio::test]
async fn should_propagate_write_error_on_sighup_writeln() {
    // The output sequence for a non-watch server on unix is:
    //   1. "llmposter listening on ..."  ← first writeln (1 newline)
    //   2. "Send SIGHUP (kill -HUP ...) ← second writeln
    //   3. "Press Ctrl+C to stop"
    // FailAfterNNewlines(limit=1) lets the first writeln complete, then fails
    // at the start of the SIGHUP writeln, exercising the `)?;` path on line 178.
    let dir = fixtures_dir();
    let cli = base_cli(dir.clone());
    let mut writer = FailAfterNNewlines {
        completed: 0,
        limit: 1,
    };
    let result = run_with_output(&cli, &mut writer).await;
    assert!(result.is_err(), "expected Err from write failure");
    std::fs::remove_dir_all(&dir).ok();
}

// ===========================================================================
// VCR record/replay CLI flags
// ===========================================================================

#[cfg(feature = "record")]
mod vcr_flags {
    use super::*;
    use clap::Parser;
    use llmposter::record::VcrMode;

    #[test]
    fn should_parse_vcr_flags() {
        let cli = Cli::try_parse_from([
            "llmposter",
            "--fixtures",
            "f.yaml",
            "--vcr-mode",
            "record-on-miss",
            "--redact",
            "a",
            "--redact",
            "b",
            "--proxy-openai",
            "http://x",
            "--allow-remote-record",
        ])
        .expect("should parse VCR flags");

        assert_eq!(cli.vcr_mode, VcrMode::RecordOnMiss);
        assert_eq!(cli.redact, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(cli.proxy_openai, Some("http://x".to_string()));
        assert!(cli.allow_remote_record);
        assert_eq!(cli.proxy_anthropic, None);
        assert_eq!(cli.proxy_gemini, None);
        assert_eq!(cli.record_file, None);
    }

    #[test]
    fn should_default_to_replay_mode() {
        let cli = Cli::try_parse_from(["llmposter", "--fixtures", "f.yaml"])
            .expect("should parse with no VCR flags");
        assert_eq!(cli.vcr_mode, VcrMode::Replay);
    }

    #[tokio::test]
    async fn should_not_mention_vcr_in_output_when_replay_mode() {
        let cli = base_cli(fixtures_dir());
        let mut output = Vec::new();
        let result = run_with_output(&cli, &mut output).await;
        assert!(result.is_ok());
        let text = String::from_utf8(output).unwrap();
        assert!(
            !text.contains("VCR mode"),
            "expected no VCR mode line in replay mode, got: {}",
            text
        );
    }

    #[tokio::test]
    async fn should_start_record_mode_with_default_cassette_next_to_fixtures_file() {
        let dir = unique_temp_dir("llmposter_cli_test_vcr_file");
        let file = dir.join("f.yaml");
        std::fs::write(
            &file,
            "fixtures:\n  - match:\n      user_message: hello\n    response:\n      content: world",
        )
        .unwrap();
        let cli = Cli {
            vcr_mode: VcrMode::RecordOnMiss,
            ..base_cli(file)
        };
        let mut output = Vec::new();
        let result = run_with_output(&cli, &mut output).await;
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        let text = String::from_utf8(output).unwrap();
        assert!(
            text.contains("VCR mode: record-on-miss"),
            "expected VCR mode line, got: {}",
            text
        );
        assert!(
            text.contains("recorded.yaml"),
            "expected cassette path in output, got: {}",
            text
        );
        assert!(
            dir.join("recorded.yaml").exists(),
            "expected cassette file to be created next to fixtures file"
        );
        drop(result.unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn should_place_default_cassette_inside_fixtures_dir() {
        let dir = fixtures_dir();
        let cli = Cli {
            vcr_mode: VcrMode::RecordOnMiss,
            ..base_cli(dir.clone())
        };
        let mut output = Vec::new();
        let result = run_with_output(&cli, &mut output).await;
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        assert!(
            dir.join("recorded.yaml").exists(),
            "expected cassette file to be created inside fixtures dir"
        );
        drop(result.unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn should_wire_proxy_overrides_and_redact_patterns_into_builder() {
        // Exercises the proxy_openai/proxy_anthropic/proxy_gemini/redact
        // wiring branches in run_with_output — asserting only that the
        // server starts successfully with all four set, since the builder
        // itself is responsible for validating/applying them.
        let dir = fixtures_dir();
        let cli = Cli {
            vcr_mode: VcrMode::RecordOnMiss,
            proxy_openai: Some("http://127.0.0.1:9".to_string()),
            proxy_anthropic: Some("http://127.0.0.1:9".to_string()),
            proxy_gemini: Some("http://127.0.0.1:9".to_string()),
            redact: vec!["secret-\\d+".to_string()],
            ..base_cli(dir.clone())
        };
        let mut output = Vec::new();
        let result = run_with_output(&cli, &mut output).await;
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        drop(result.unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn should_refuse_record_mode_on_public_bind_without_flag() {
        let dir = fixtures_dir();
        let cli = Cli {
            bind: "0.0.0.0".to_string(),
            vcr_mode: VcrMode::Record,
            ..base_cli(dir.clone())
        };
        let mut output = Vec::new();
        let result = run_with_output(&cli, &mut output).await;
        assert!(result.is_err(), "expected Err for non-loopback bind");
        assert!(
            result.unwrap_err().to_string().contains("loopback"),
            "expected loopback error message"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn should_allow_record_mode_on_public_bind_with_flag() {
        let dir = fixtures_dir();
        let cli = Cli {
            bind: "0.0.0.0".to_string(),
            vcr_mode: VcrMode::Record,
            allow_remote_record: true,
            ..base_cli(dir.clone())
        };
        let mut output = Vec::new();
        let result = run_with_output(&cli, &mut output).await;
        assert!(
            result.is_ok(),
            "expected Ok with allow_remote_record, got: {:?}",
            result.err()
        );
        drop(result.unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn should_propagate_write_error_on_vcr_mode_writeln() {
        // Output sequence for a record-mode server on unix is:
        //   1. "llmposter listening on ..."       ← first writeln (1 newline)
        //   2. "VCR mode: ... "                    ← second writeln
        //   3. "Send SIGHUP ..."
        //   4. "Press Ctrl+C to stop"
        // FailAfterNNewlines(limit=1) lets the first writeln complete, then
        // fails at the start of the VCR mode writeln.
        let dir = fixtures_dir();
        let cli = Cli {
            vcr_mode: VcrMode::RecordOnMiss,
            ..base_cli(dir.clone())
        };
        let mut writer = FailAfterNNewlines {
            completed: 0,
            limit: 1,
        };
        let result = run_with_output(&cli, &mut writer).await;
        assert!(result.is_err(), "expected Err from write failure");
        std::fs::remove_dir_all(&dir).ok();
    }
}
