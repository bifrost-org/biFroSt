use std::fs;

use remotefs::api::client::RemoteClient;
use remotefs::api::models::UserKeys;
use remotefs::config::settings::Config;
use users::get_current_username;

pub async fn run() {
    let config = Config::from_file().expect("Loading configuration failed");

    println!("\nBegin registration:");

    let username_osstr = get_current_username().expect("Cannot get current username");
    let username = username_osstr.to_string_lossy();
    println!("  Current user detected: {}", username);

    let client = RemoteClient::new(&config);

    let user_keys: UserKeys = client
        .user_registration(&username)
        .await
        .expect("User registration failed");

    println!("  User '{}' successfully registered", username);

    // keys will be saved in .bifrost folder

    let mut dir = dirs::home_dir().expect("Cannot find home directory");
    dir.push(".bifrost");
    fs::create_dir_all(&dir).expect("Failed to create .bifrost directory");

    let mut api_key_path = dir.clone();
    api_key_path.push("api_key");
    fs::write(&api_key_path, user_keys.api_key).expect("Failed to write api_key file");

    let mut secret_key_path = dir.clone();
    secret_key_path.push("secret_key");
    fs::write(&secret_key_path, user_keys.secret_key).expect("Failed to write secret_key file");

    println!("  Keys saved in {}", dir.display());
    println!("Registration complete!")
}
