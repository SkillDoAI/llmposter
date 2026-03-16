use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = llmposter::cli::Cli::parse();

    match llmposter::cli::run(&cli).await {
        Ok(Some(_server)) => {
            // Server is running — wait for Ctrl+C
            if let Err(e) = tokio::signal::ctrl_c().await {
                eprintln!("Signal error: {}", e);
            }
        }
        Ok(None) => {
            // Validate mode — already printed result, just exit
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}
