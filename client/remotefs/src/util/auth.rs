use anyhow::{bail, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;

use crate::util::path::get_current_user;

type HmacSha256 = Hmac<Sha256>;

#[derive(Deserialize)]
pub struct UserKeys {
    pub api_key: String,
    pub secret_key: String,
    timestamp: i64,
}

impl UserKeys {
    pub fn load_from_files() -> Result<UserKeys> {
        let mut dir = dirs::home_dir().expect("Cannot find user home directory");
        dir.push(".bifrost");

        let api_key_path = dir.join("api_key");
        let secret_key_path = dir.join("secret_key");

        if !api_key_path.exists() || !secret_key_path.exists() {
            bail!("User '{}' is not registered", get_current_user());
        }

        let api_key = fs::read_to_string(&api_key_path)?.trim().to_string();
        let secret_key = fs::read_to_string(&secret_key_path)?.trim().to_string();
        let timestamp = Utc::now().timestamp();

        Ok(UserKeys {
            api_key,
            secret_key,
            timestamp,
        })
    }

    pub fn get_auth_headers(&self, hmac_message: String) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_str(&self.api_key).unwrap());
        headers.insert("x-signature", HeaderValue::from_str(&hmac_message).unwrap());
        headers.insert(
            "x-timestamp",
            HeaderValue::from_str(&self.timestamp.to_string()).unwrap(),
        );
        headers
    }

    pub fn build_hmac_message(&self, method: &str, path: &str, body: Option<&str>) -> String {
        let body_hash = if let Some(content) = body {
            let mut hasher = Sha256::new();
            hasher.update(content.as_bytes());
            format!("{:x}", hasher.finalize())
        } else {
            "".to_string()
        };

        let message = format!(
            "{}\n{}\n{}\n{}",
            method.to_uppercase(),
            path,
            self.timestamp,
            body_hash
        );

        println!("Message: {}", message);

        self.sign_request(message)
    }

    pub fn sign_request(&self, message: String) -> String {
        let mut hmac = HmacSha256::new_from_slice(self.secret_key.as_bytes())
            .expect("HMAC can take key of any size");
        hmac.update(message.as_bytes());
        let result = hmac.finalize();
        let signature_bytes = result.into_bytes();
        hex::encode(signature_bytes)
    }
}

impl Default for UserKeys {
    fn default() -> Self {
        UserKeys::load_from_files().unwrap()
    }
}
