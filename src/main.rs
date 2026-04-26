use clap::Parser;
use trumpet::cli::{AgentCmd, AuthCmd, ChatCmd, EventsTailArgs, TaskCmd, ToolCmd};
use trumpet::error::CliFormatter;

/// Agent nexus daemon.
#[derive(Parser)]
#[command(name = "trumpet", about = "Agent nexus daemon")]
enum Cli {
    /// Start the trumpet daemon in the foreground.
    Serve,
    /// Start the trumpet daemon in the background.
    Start,
    /// Stop the trumpet daemon.
    Stop,
    /// Show daemon status and connected agents.
    Status,
    /// Manage registered agents.
    Agent {
        #[command(subcommand)]
        cmd: AgentCmd,
    },
    /// Manage A2A tasks.
    Task {
        #[command(subcommand)]
        cmd: TaskCmd,
    },
    /// Manage tools.
    Tool {
        #[command(subcommand)]
        cmd: ToolCmd,
    },
    /// Manage conversations.
    Chat {
        #[command(subcommand)]
        cmd: ChatCmd,
    },
    /// Tail the daemon event stream.
    Events {
        #[command(flatten)]
        args: EventsTailArgs,
    },
    /// Manage daemon authentication.
    Auth {
        #[command(subcommand)]
        cmd: AuthCmd,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let is_serve = matches!(cli, Cli::Serve);

    if !is_serve {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .init();
    }

    let result = match cli {
        Cli::Serve => trumpet::cli::run_serve().await,
        Cli::Start => trumpet::cli::run_start().await,
        Cli::Stop => trumpet::cli::run_stop().await,
        Cli::Status => trumpet::cli::run_status().await,
        Cli::Agent { cmd } => trumpet::cli::agent::run(cmd).await,
        Cli::Task { cmd } => trumpet::cli::task::run(cmd).await,
        Cli::Tool { cmd } => trumpet::cli::tool::run(cmd).await,
        Cli::Chat { cmd } => trumpet::cli::chat::run(cmd).await,
        Cli::Events { args } => trumpet::cli::events::run_tail(args).await,
        Cli::Auth { cmd } => trumpet::cli::auth::run(cmd).await,
    };

    if let Err(ref err) = result {
        eprintln!("{}", CliFormatter(err));
        std::process::exit(1);
    }
}
