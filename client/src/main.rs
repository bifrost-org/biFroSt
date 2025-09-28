use clap::{Parser, Subcommand};
use daemonize::Daemonize;
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
    Start {
        #[arg(long = "detached", short = 'd')]
        detached: bool,
        #[arg(long = "enable-autorun", short = 'e')]
        enable_autorun: bool,
    },
    Stop {
        #[arg(long = "disable-autorun", short = 'd')]
        disable_autorun: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    if let Commands::Start { detached: true, .. } = &cli.command {
        let cwd = std::env::current_dir().expect("cannot get current dir");

        let daemonize = Daemonize::new().working_directory(&cwd);

        if let Err(e) = daemonize.start() {
            eprintln!("daemonize failed: {}", e);
            std::process::exit(1);
        }
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    rt.block_on(async {
        match cli.command {
            Commands::Config => {
                commands::config::run().await;
            }
            Commands::Register => {
                commands::register::run().await;
            }
            Commands::Start {
                detached,
                enable_autorun,
            } => {
                commands::start::run(enable_autorun).await;
            }
            Commands::Stop { disable_autorun } => {
                commands::stop::run(disable_autorun).await;
            }
        }
    });
}
