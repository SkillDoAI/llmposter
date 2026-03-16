use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "llmposter",
    about = "Mock LLM API server — fixture-driven, deterministic responses for testing"
)]
struct Cli {
    /// Path to fixtures directory or YAML file
    #[arg(short, long)]
    fixtures: PathBuf,

    /// Validate fixtures without starting server
    #[arg(long)]
    validate: bool,

    /// Port to listen on (default: 2112)
    #[arg(short, long, default_value = "2112")]
    port: u16,

    /// Bind address (supports IPv4 and IPv6)
    #[arg(short, long, default_value = "127.0.0.1")]
    bind: String,

    /// Verbose logging to stderr
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let fixtures = if cli.fixtures.is_dir() {
        llmposter::fixture::load_yaml_dir(&cli.fixtures)
    } else {
        llmposter::fixture::load_yaml_file(&cli.fixtures)
    };

    let fixtures = match fixtures {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error loading fixtures: {}", e);
            std::process::exit(1);
        }
    };

    if cli.validate {
        if fixtures.is_empty() {
            eprintln!("No fixtures found — nothing to validate");
            std::process::exit(1);
        }
        eprintln!("Validated {} fixtures successfully", fixtures.len());
        return;
    }

    if fixtures.is_empty() {
        eprintln!(
            "Warning: no fixtures loaded from {}",
            cli.fixtures.display()
        );
    }

    let bind_addr = if cli.bind.contains(':') && !cli.bind.starts_with('[') {
        // IPv6 address needs brackets (but don't double-wrap if already bracketed)
        format!("[{}]:{}", cli.bind, cli.port)
    } else {
        format!("{}:{}", cli.bind, cli.port)
    };
    let server = match llmposter::ServerBuilder::new()
        .fixtures(fixtures)
        .bind(&bind_addr)
        .verbose(cli.verbose)
        .build()
        .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error starting server: {}", e);
            std::process::exit(1);
        }
    };

    eprintln!("llmposter listening on {}", server.url());
    eprintln!("Press Ctrl+C to stop");

    // Keep the server alive until Ctrl+C
    if let Err(e) = tokio::signal::ctrl_c().await {
        eprintln!("Signal error: {}", e);
    }
    drop(server);
}
