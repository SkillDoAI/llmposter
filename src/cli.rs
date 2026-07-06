use clap::Parser;
use std::io::Write;
use std::path::PathBuf;

/// Default port the mock server listens on when `--port` is not specified.
const DEFAULT_PORT: u16 = 2112;

#[derive(Parser, Debug)]
#[command(
    name = "llmposter",
    about = "Mock LLM API server — fixture-driven, deterministic responses for testing"
)]
pub struct Cli {
    /// Path to fixtures directory or YAML file
    #[arg(short, long)]
    pub fixtures: PathBuf,

    /// Validate fixtures without starting server
    #[arg(long)]
    pub validate: bool,

    /// Port to listen on
    #[arg(short, long, default_value_t = DEFAULT_PORT)]
    pub port: u16,

    /// Bind address (supports IPv4 and IPv6)
    #[arg(short, long, default_value = "127.0.0.1")]
    pub bind: String,

    /// Verbose logging to stderr
    #[arg(short, long)]
    pub verbose: bool,

    /// Watch the fixtures file/directory and hot-reload on change.
    ///
    /// When a change is detected, the files are re-read and validated. If
    /// validation succeeds, the fixtures are atomically swapped; otherwise
    /// the previous fixtures keep serving and an error is logged to stderr.
    ///
    /// On Unix, `kill -HUP <pid>` also triggers a reload — always on for
    /// file-backed fixtures regardless of this flag.
    #[cfg(feature = "watch")]
    #[arg(short = 'w', long)]
    pub watch: bool,

    /// Maximum number of captured requests to retain in the ring buffer.
    /// Older entries are dropped FIFO when the limit is reached. Defaults
    /// to 1000 for the standalone CLI to prevent unbounded memory growth.
    /// Set to 0 to disable the retention ring (the debug UI live feed
    /// remains active). Library users get unbounded by default
    /// (see `ServerBuilder::capture_capacity`).
    #[arg(long, default_value_t = 1000)]
    pub capture_capacity: usize,

    /// Include nearest-match diagnostics in 404 no-match responses.
    /// When enabled, 404 responses include a nearest_match object showing
    /// which fixture came closest to matching and which fields passed/failed.
    #[arg(long)]
    pub diagnostics: bool,

    /// Enable the embedded debug UI at /ui (request inspector + match debugger).
    #[cfg(feature = "ui")]
    #[arg(long)]
    pub ui: bool,

    /// VCR mode: replay (default, fixtures only), record (proxy everything
    /// upstream and save responses as fixtures), or record-on-miss (proxy
    /// only unmatched requests). Recorded fixtures append to the cassette.
    #[cfg(feature = "record")]
    #[arg(long, value_enum, default_value_t = crate::record::VcrMode::Replay)]
    pub vcr_mode: crate::record::VcrMode,

    /// Cassette file for recorded fixtures.
    /// Default: recorded.yaml inside a --fixtures directory, or next to a --fixtures file.
    #[cfg(feature = "record")]
    #[arg(long)]
    pub record_file: Option<PathBuf>,

    /// Upstream override for OpenAI-format routes (vLLM, Ollama, gateways).
    /// No effect unless --vcr-mode is record or record-on-miss.
    #[cfg(feature = "record")]
    #[arg(long)]
    pub proxy_openai: Option<String>,

    /// Upstream override for /v1/messages.
    /// No effect unless --vcr-mode is record or record-on-miss.
    #[cfg(feature = "record")]
    #[arg(long)]
    pub proxy_anthropic: Option<String>,

    /// Upstream override for Gemini routes.
    /// No effect unless --vcr-mode is record or record-on-miss.
    #[cfg(feature = "record")]
    #[arg(long)]
    pub proxy_gemini: Option<String>,

    /// Regex whose matches are masked as [REDACTED] in recorded response
    /// content and tool-call arguments. Repeatable.
    /// No effect unless --vcr-mode is record or record-on-miss.
    #[cfg(feature = "record")]
    #[arg(long)]
    pub redact: Vec<String>,

    /// Allow record modes on non-loopback binds. Record mode forwards real
    /// API keys over plain HTTP, so this is refused by default — front
    /// llmposter with your own TLS terminator if you need this.
    #[cfg(feature = "record")]
    #[arg(long)]
    pub allow_remote_record: bool,
}

/// Run the CLI with the given options, writing status output to stderr.
/// Returns `Ok(None)` for `--validate`, or `Ok(Some(MockServer))` after startup.
/// The server runs until the returned `MockServer` is dropped.
pub async fn run(cli: &Cli) -> Result<Option<crate::MockServer>, Box<dyn std::error::Error>> {
    run_with_output(cli, &mut std::io::stderr()).await
}

/// Run the CLI with the given options, writing status output to the provided writer.
/// This variant enables tests to capture output.
pub async fn run_with_output(
    cli: &Cli,
    out: &mut (dyn Write + Send),
) -> Result<Option<crate::MockServer>, Box<dyn std::error::Error>> {
    // --validate only parses; skip the builder's source tracking so we don't
    // spawn a watcher / SIGHUP handler for a one-shot validation run.
    if cli.validate {
        let fixtures = if cli.fixtures.is_dir() {
            crate::fixture::load_yaml_dir(&cli.fixtures)?
        } else {
            crate::fixture::load_yaml_file(&cli.fixtures)?
        };
        if fixtures.is_empty() {
            return Err("No fixtures found — nothing to validate".into());
        }
        // validate() is already called by load_yaml_dir/load_yaml_file during loading.
        // If we got here without error, all fixtures passed validation.
        writeln!(out, "Validated {} fixtures successfully", fixtures.len())?;
        return Ok(None);
    }

    let warn_port_ignored = |out: &mut dyn Write,
                             bind_port: &dyn std::fmt::Display,
                             cli_port: u16|
     -> std::io::Result<()> {
        writeln!(
            out,
            "Warning: --port {} ignored because --bind already includes port {}",
            cli_port, bind_port
        )
    };

    let bind_addr = if let Ok(sa) = cli.bind.parse::<std::net::SocketAddr>() {
        if cli.port != DEFAULT_PORT {
            warn_port_ignored(out, &sa.port(), cli.port)?;
        }
        cli.bind.clone()
    } else if let Ok(ip) = cli.bind.parse::<std::net::IpAddr>() {
        match ip {
            std::net::IpAddr::V6(_) => format!("[{}]:{}", cli.bind, cli.port),
            std::net::IpAddr::V4(_) => format!("{}:{}", cli.bind, cli.port),
        }
    } else if let Some((host, port_str)) = cli.bind.rsplit_once(':') {
        if !host.is_empty() && port_str.parse::<u16>().is_ok() {
            if cli.port != DEFAULT_PORT {
                warn_port_ignored(out, &port_str, cli.port)?;
            }
            cli.bind.clone()
        } else {
            format!("{}:{}", cli.bind, cli.port)
        }
    } else {
        // Bare hostname (e.g. "localhost")
        format!("{}:{}", cli.bind, cli.port)
    };

    // Load via the builder so fixture sources are tracked for hot-reload.
    let mut builder = crate::ServerBuilder::new();
    builder = if cli.fixtures.is_dir() {
        builder.load_yaml_dir(&cli.fixtures)?
    } else {
        builder.load_yaml(&cli.fixtures)?
    };

    if builder.fixture_count() == 0 {
        writeln!(
            out,
            "Warning: no fixtures loaded from {}",
            cli.fixtures.display()
        )?;
    }

    #[cfg(feature = "watch")]
    {
        builder = builder.watch(cli.watch);
    }
    #[cfg(feature = "ui")]
    {
        builder = builder.ui(cli.ui);
    }
    #[cfg(feature = "record")]
    let vcr_cassette = if cli.vcr_mode != crate::record::VcrMode::Replay {
        let cassette = cli.record_file.clone().unwrap_or_else(|| {
            if cli.fixtures.is_dir() {
                cli.fixtures.join("recorded.yaml")
            } else {
                cli.fixtures
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join("recorded.yaml")
            }
        });
        builder = builder
            .vcr_mode(cli.vcr_mode)
            .record_file(cassette.clone())
            .allow_remote_record(cli.allow_remote_record);
        if let Some(u) = &cli.proxy_openai {
            builder = builder.proxy_openai(u);
        }
        if let Some(u) = &cli.proxy_anthropic {
            builder = builder.proxy_anthropic(u);
        }
        if let Some(u) = &cli.proxy_gemini {
            builder = builder.proxy_gemini(u);
        }
        for pattern in &cli.redact {
            builder = builder.redact(pattern);
        }
        Some(cassette)
    } else {
        None
    };
    let server = builder
        .bind(&bind_addr)
        .verbose(cli.verbose)
        .capture_capacity(cli.capture_capacity)
        .diagnostics(cli.diagnostics)
        .build()
        .await?;

    writeln!(out, "llmposter listening on {}", server.url())?;
    #[cfg(feature = "record")]
    if let Some(cassette) = &vcr_cassette {
        use clap::ValueEnum;
        let mode_name = cli
            .vcr_mode
            .to_possible_value()
            .expect("VcrMode has no skip_value variants")
            .get_name()
            .to_string();
        writeln!(
            out,
            "VCR mode: {} → recording to {}",
            mode_name,
            cassette.display()
        )?;
    }
    #[cfg(feature = "ui")]
    if cli.ui {
        writeln!(out, "Debug UI at {}/ui", server.url())?;
    }
    #[cfg(feature = "watch")]
    if cli.watch {
        writeln!(out, "Watching {} for changes", cli.fixtures.display())?;
    }
    #[cfg(unix)]
    writeln!(
        out,
        "Send SIGHUP (kill -HUP {}) to reload fixtures",
        std::process::id()
    )?;
    writeln!(out, "Press Ctrl+C to stop")?;
    Ok(Some(server))
}
