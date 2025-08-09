use clap::{Parser, Subcommand};
mod commands;

#[derive(Parser)]
#[command(name = "bifrost")]
#[command(about = "A remote filesystem", long_about = None)] // TODO:
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Register,
    Start,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Register => {
            commands::register::run().await;
        }
        Commands::Start => {
            commands::start::run().await;
        }
    }
}
