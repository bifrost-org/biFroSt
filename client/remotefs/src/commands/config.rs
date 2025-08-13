use remotefs::config::settings::{Config, ConfigError};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

pub async fn run() {
    let relative_config_path = Config::default_path();

    let absolute_config_path = if relative_config_path.is_relative() {
        env::current_dir().unwrap().join(&relative_config_path)
    } else {
        relative_config_path.to_path_buf()
    };

    if relative_config_path.exists() {
        eprintln!(
            "\nConfiguration file already exists at `{}`",
            absolute_config_path.display()
        );
        eprintln!("Delete or rename it before creating a new one.");
        return;
    }

    println!("\nBifrost configuration setup:");
    println!("Press ENTER to use the default value (shown in brackets)\n");

    let server_url = prompt("Server URL", "https://bifrost.oberon-server.it");
    let port = prompt_parse::<u16>("Port", 443);
    let mount_point = prompt_path("Mount point", "/home/oberon/bifrostFS");
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
                absolute_config_path.display()
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
