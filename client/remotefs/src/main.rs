use std::path::PathBuf;
use remotefs::config::settings::Config;

fn main() {
    // Crea un file di configurazione di esempio
    let config_path = PathBuf::from("config.toml");
    
    // Prova a caricare la configurazione
    match Config::from_file(&config_path) {
        Ok(config) => {
            println!("Configuration loaded successfully!");
            println!("Server URL: {}", config.server_full_url());
            println!("Mount point: {:?}", config.mount_point);
            println!("Has authentication: {}", config.has_auth());
        },
        Err(e) => {
            println!("Error loading config: {}", e);
            
            // Crea una configurazione di default e salvala
            println!("Creating default configuration...");
            let default_config = Config::default();
            
            match default_config.save_to_file(&config_path) {
                Ok(_) => println!("Default config saved to {:?}", config_path),
                Err(save_err) => println!("Error saving default config: {}", save_err),
            }
        }
    }
}