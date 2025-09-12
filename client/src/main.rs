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
        #[arg(long = "enable-autorun", short ='e')]
        enable_autorun: bool,
    },
    Stop {
        #[arg(long = "disable-autorun", short = 'd')]
        disable_autorun: bool,
    },
}


fn main() {
    let cli = Cli::parse();



    // Se è start con detached, daemonizza PRIMA del runtime
    if let Commands::Start { detached: true, .. } = &cli.command {
        let cwd = std::env::current_dir().expect("cannot get current dir");

        let daemonize = Daemonize::new()
            // Mantieni la working dir corrente per non rompere path relativi (config, ecc.)
            .working_directory(&cwd);


        if let Err(e) = daemonize.start() {
            eprintln!("daemonize failed: {}", e); // visibile sul terminale
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
            Commands::Start { detached, enable_autorun } => {
                // Non fare più daemonize qui dentro.
                // Aggiungi una stampa subito per verificare i log:
                println!("entering start::run (detached={}) pid={}", detached, std::process::id());
                commands::start::run(enable_autorun).await;
            }
            Commands::Stop { disable_autorun } => {
                commands::stop::run(disable_autorun).await;
            }
        }
    });
}