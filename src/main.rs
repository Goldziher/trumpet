use clap::Parser;
use trumpet::error::CliFormatter;

/// Agent nexus daemon.
#[derive(Parser)]
#[command(name = "trumpet", about = "Agent nexus daemon")]
enum Cli {
    /// Start the trumpet daemon in the foreground.
    Serve,
    /// Show daemon status and connected agents.
    Status,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let result = match Cli::parse() {
        Cli::Serve => trumpet::cli::run_serve().await,
        Cli::Status => trumpet::cli::run_status().await,
    };

    if let Err(ref err) = result {
        eprintln!("{}", CliFormatter(err));
        std::process::exit(1);
    }
}
