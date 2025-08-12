use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub server_url: String,
    pub port: u16,
    pub timeout: Duration,
    pub mount_point: PathBuf,
    pub api_key: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            server_url: "http://localhost".to_string(),
            port: 8080,
            mount_point: PathBuf::from("/mnt/bifrost"),
            timeout: Duration::from_secs(60),
            api_key: None,
        }
    }
}

impl Config {
    fn default_path() -> &'static Path {
        Path::new("config.toml")
    }

    // load configuration from file
    pub fn from_file() -> Result<Self, ConfigError> {
        let content =
            std::fs::read_to_string(Self::default_path()).map_err(|e| ConfigError::FileRead(e))?;

        let config: Config =
            toml::from_str(&content).map_err(|e| ConfigError::Parse(e.to_string()))?;

        config.validate()?;
        Ok(config)
    }

    // save actual configuration to file
    pub fn save_to_file(&self) -> Result<(), ConfigError> {
        let content =
            toml::to_string_pretty(self).map_err(|e| ConfigError::Serialize(e.to_string()))?;

        std::fs::write(Self::default_path(), content).map_err(|e| ConfigError::FileWrite(e))?;

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

    pub fn server_full_url(&self) -> String {
        format!("{}:{}", self.server_url, self.port)
    }
}
