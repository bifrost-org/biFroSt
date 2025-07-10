use std::time::Duration;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub server_url: String,
    pub port: u16, // si potrebbe togliere

    pub mount_point: PathBuf,

    pub timeout: Duration,

    pub username: Option<String>,
    pub password: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    FileRead(#[from] std::io::Error),

    #[error("Failed to write config file: {0}")]
    FileWrite(std::io::Error),

    #[error("Failed to parse config: {0}")]
    Parse(String),

    #[error("Failed to serialize config: {0}")]
    Serialize(String),

    #[error("Configuration validation error: {0}")]
    Validation(String),
}

impl Default for Config {
    fn default() -> Self {
        Config {
            server_url: "http://localhost".to_string(),
            port: 8080,
            mount_point: PathBuf::from("/mnt/remotefs"),
            timeout: Duration::from_secs(60), // 60 seconds
            username: None,
            password: None,
            api_key: None,
        }
    }
}

impl Config {
    /// Carica la configurazione da un file TOML
    pub fn from_file(path: &PathBuf) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|e| ConfigError::FileRead(e))?;
        
        let config: Config =
            toml::from_str(&content).map_err(|e| ConfigError::Parse(e.to_string()))?;
        
        config.validate()?;
        Ok(config)
    }

    /// Salva la configurazione in un file TOML
    pub fn save_to_file(&self, path: &PathBuf) -> Result<(), ConfigError> {
        let content =
            toml::to_string_pretty(self).map_err(|e| ConfigError::Serialize(e.to_string()))?;

        std::fs::write(path, content).map_err(|e| ConfigError::FileWrite(e))?;

        Ok(())
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.server_url.is_empty() {
            return Err(ConfigError::Validation(
                "Server URL cannot be empty".to_string(),
            ));
        }

        if !self.mount_point.is_absolute() {
            return Err(ConfigError::Validation(
                "Mount point must be an absolute path".to_string(),
            ));
        }

        if self.port == 0 {
            return Err(ConfigError::Validation(
                "Port must be greater than 0".to_string(),
            ));
        }

        if self.timeout.is_zero() {
            return Err(ConfigError::Validation(
                "Timeout must be greater than 0".to_string(),
            ));
        }



        Ok(())
    }

        /// Ottieni l'URL completo del server
    pub fn server_full_url(&self) -> String {
        format!("{}:{}", self.server_url, self.port)
    }
    
    /// Controlla se l'autenticazione è configurata
    pub fn has_auth(&self) -> bool {
        self.username.is_some() || self.api_key.is_some()
    }
}
