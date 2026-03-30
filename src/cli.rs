use clap::Parser;
use std::io::Write;
use std::path::PathBuf;

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

    /// Port to listen on (default: 2112)
    #[arg(short, long, default_value = "2112")]
    pub port: u16,

    /// Bind address (supports IPv4 and IPv6)
    #[arg(short, long, default_value = "127.0.0.1")]
    pub bind: String,

    /// Verbose logging to stderr
    #[arg(short, long)]
    pub verbose: bool,
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
    let fixtures = if cli.fixtures.is_dir() {
        crate::fixture::load_yaml_dir(&cli.fixtures)?
    } else {
        crate::fixture::load_yaml_file(&cli.fixtures)?
    };

    if cli.validate {
        if fixtures.is_empty() {
            return Err("No fixtures found — nothing to validate".into());
        }
        // validate() is already called by load_yaml_dir/load_yaml_file during loading.
        // If we got here without error, all fixtures passed validation.
        writeln!(out, "Validated {} fixtures successfully", fixtures.len())?;
        return Ok(None);
    }

    if fixtures.is_empty() {
        writeln!(
            out,
            "Warning: no fixtures loaded from {}",
            cli.fixtures.display()
        )?;
    }

    let bind_addr = if let Ok(_sa) = cli.bind.parse::<std::net::SocketAddr>() {
        // Already a full socket address (e.g. "0.0.0.0:8080") — use as-is, ignore --port
        cli.bind.clone()
    } else if let Some((host, port)) = cli.bind.rsplit_once(':') {
        // hostname:port (e.g. "localhost:8080") — use as-is if port is valid
        if !host.is_empty() && !host.contains(':') && port.parse::<u16>().is_ok() {
            cli.bind.clone()
        } else {
            format!("{}:{}", cli.bind, cli.port)
        }
    } else if let Ok(ip) = cli.bind.parse::<std::net::IpAddr>() {
        match ip {
            std::net::IpAddr::V6(_) => format!("[{}]:{}", cli.bind, cli.port),
            std::net::IpAddr::V4(_) => format!("{}:{}", cli.bind, cli.port),
        }
    } else {
        // Bare hostname (e.g. "localhost")
        format!("{}:{}", cli.bind, cli.port)
    };

    let server = crate::ServerBuilder::new()
        .fixtures(fixtures)
        .bind(&bind_addr)
        .verbose(cli.verbose)
        .build()
        .await?;

    writeln!(out, "llmposter listening on {}", server.url())?;
    writeln!(out, "Press Ctrl+C to stop")?;
    Ok(Some(server))
}
