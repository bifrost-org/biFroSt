use clap::{Parser, Subcommand};
mod commands;

#[derive(Parser)]
#[command(name = "bifrost")]
#[command(
    about = "A remote filesystem client",
    long_about = r#"
        Bifrost is a remote filesystem client that communicates with a server via HTTPS.
        It supports:
        • User registration and authentication via HMAC
        • Remote read/write operations
    "#
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Config,
    Register,
    Start,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Config => {
            commands::config::run().await;
        }
        Commands::Register => {
            commands::register::run().await;
        }
        Commands::Start => {
            commands::start::run().await;
        }
    }
}
