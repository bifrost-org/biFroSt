use std::fs;
use std::io::{self, Write};
use std::path::Path;

use bifrost::api::client::RemoteClient;
use bifrost::config::settings::Config;
use bifrost::util::auth::UserKeys;
use bifrost::util::fs::get_current_user;

pub async fn run() {
    let config = match Config::from_file() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    // keys will be saved in ~/.bifrost folder
    let mut dir = dirs::home_dir().expect("Cannot find home directory");
    dir.push(".bifrost");
    fs::create_dir_all(&dir).expect("Failed to create .bifrost directory");

    let mut api_key_path = dir.clone();
    api_key_path.push("api_key");

    let mut secret_key_path = dir.clone();
    secret_key_path.push("secret_key");

    if keys_exist_and_nonempty(&api_key_path, &secret_key_path) {
        if !ask_confirmation("\nKeys already exist. Overwrite them?") {
            println!("Aborted.");
            return;
        }
    }

    println!("\nBegin registration:");

    let username = get_current_user();

    let client = RemoteClient::new(&config, None);

    let user_keys: UserKeys = client
        .user_registration(username.clone())
        .await
        .expect("User registration failed");

    println!("  User '{}' successfully registered", username);

    fs::write(&api_key_path, user_keys.api_key).expect("Failed to write api_key file");
    fs::write(&secret_key_path, user_keys.secret_key).expect("Failed to write secret_key file");

    println!("  Keys saved in `{}`", dir.display());
    println!("Registration complete!")
}

fn keys_exist_and_nonempty(api_key_path: &Path, secret_key_path: &Path) -> bool {
    for path in [api_key_path, secret_key_path] {
        if !path.exists() {
            return false;
        }
        let metadata = fs::metadata(path).ok();
        if metadata.map_or(true, |m| m.len() == 0) {
            return false;
        }
    }
    true
}

fn ask_confirmation(prompt: &str) -> bool {
    print!("{} [y/N]: ", prompt);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}
