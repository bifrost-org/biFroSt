use bifrost::config::settings::{Config, ConfigError};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

pub async fn run() {
    let config_path = Config::default_path();

    if config_path.exists() {
        eprintln!(
            "\nConfiguration file already exists at `{}`",
            config_path.display()
        );
        eprintln!("Delete or rename it before creating a new one.");
        return;
    }

    println!("\nBifrost configuration setup:");
    println!("Press ENTER to use the default value (shown in brackets)\n");

    let default_mount = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("bifrostFS");

    let server_url = prompt("Server URL", "https://bifrost.oberon-server.it");
    let port = prompt_parse::<u16>("Port", 443);
    let mount_point = prompt_path(
        "Mount point",
        default_mount.to_str().unwrap_or("/tmp/bifrostFS"),
    );
    let timeout_secs = prompt_parse::<u64>("Timeout in seconds", 60);

    let config = Config {
        server_url,
        port,
        mount_point,
        timeout: Duration::from_secs(timeout_secs),
        api_key: None,
    };

    match config.save_to_file() {
        Ok(_) => {
            println!(
                "\nConfiguration file created at `{}`",
                config_path.display()
            );
        }
        Err(ConfigError::FileWrite(e)) => {
            eprintln!("Failed to write configuration file: {}", e);
        }
        Err(e) => {
            eprintln!("Failed to create configuration file: {}", e);
        }
    }
}

fn prompt(field: &str, default: &str) -> String {
    print!("{} [{}]: ", field, default);
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let trimmed = input.trim();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

fn prompt_parse<T>(field: &str, default: T) -> T
where
    T: std::str::FromStr + Clone + std::fmt::Debug,
{
    loop {
        print!("{} [{:?}]: ", field, default);
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let trimmed = input.trim();

        if trimmed.is_empty() {
            return default;
        } else if let Ok(parsed) = trimmed.parse::<T>() {
            return parsed;
        } else {
            println!("Invalid input, please try again.");
        }
    }
}

fn prompt_path(field: &str, default: &str) -> PathBuf {
    PathBuf::from(prompt(field, default))
}
