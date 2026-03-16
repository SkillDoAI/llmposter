use clap::Parser;
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

/// Run the CLI with the given options. Returns Ok(()) on success.
/// The server runs until the returned MockServer is dropped.
pub async fn run(cli: &Cli) -> Result<Option<crate::MockServer>, Box<dyn std::error::Error>> {
    let fixtures = if cli.fixtures.is_dir() {
        crate::fixture::load_yaml_dir(&cli.fixtures)?
    } else {
        crate::fixture::load_yaml_file(&cli.fixtures)?
    };

    if cli.validate {
        if fixtures.is_empty() {
            return Err("No fixtures found — nothing to validate".into());
        }
        eprintln!("Validated {} fixtures successfully", fixtures.len());
        return Ok(None);
    }

    if fixtures.is_empty() {
        eprintln!(
            "Warning: no fixtures loaded from {}",
            cli.fixtures.display()
        );
    }

    let bind_addr = if cli.bind.contains(':') && !cli.bind.starts_with('[') {
        format!("[{}]:{}", cli.bind, cli.port)
    } else {
        format!("{}:{}", cli.bind, cli.port)
    };

    let server = crate::ServerBuilder::new()
        .fixtures(fixtures)
        .bind(&bind_addr)
        .verbose(cli.verbose)
        .build()
        .await?;

    eprintln!("llmposter listening on {}", server.url());
    Ok(Some(server))
}
