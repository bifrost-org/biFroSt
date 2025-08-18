use anyhow::{bail, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use rand::{distr::Alphanumeric, Rng};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;

use crate::util::fs::get_current_user;

type HmacSha256 = Hmac<Sha256>;

#[derive(Deserialize)]
pub struct UserKeys {
    pub api_key: String,
    pub secret_key: String,
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

        Ok(UserKeys {
            api_key,
            secret_key,
        })
    }

    pub fn get_auth_headers(&self, hmac_message: &str, timestamp: &str, nonce: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("X-Api-Key", HeaderValue::from_str(&self.api_key).unwrap());
        headers.insert("X-Signature", HeaderValue::from_str(hmac_message).unwrap());
        headers.insert("X-Timestamp", HeaderValue::from_str(timestamp).unwrap());
        headers.insert("X-Nonce", HeaderValue::from_str(nonce).unwrap());
        headers
    }

    // remember timestamp and nonce
    pub fn build_hmac_message(
        &self,
        method: &str,
        path: &str,
        headers: Vec<&str>,
        extra: Option<Vec<ExtraItem>>,
    ) -> String {
        let extra_hashed = if let Some(extras) = extra {
            let mut hashes = Vec::new();
            for item in extras {
                let hash = match item {
                    ExtraItem::Text(s) => Sha256::digest(s.as_bytes()),
                    ExtraItem::Bytes(b) => Sha256::digest(b),
                };
                hashes.push(format!("{:x}", hash));
            }
            hashes.join("\n")
        } else {
            "".to_string()
        };

        let message = if extra_hashed.is_empty() {
            format!(
                "{}\n{}\n{}",
                method.to_uppercase(),
                path,
                headers.join("\n")
            )
        } else {
            format!(
                "{}\n{}\n{}\n{}",
                method.to_uppercase(),
                path,
                headers.join("\n"),
                extra_hashed
            )
        };

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

    pub fn generate_timestamp() -> i64 {
        Utc::now().timestamp_millis()
    }

    pub fn generate_nonce() -> String {
        rand::rng()
            .sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .collect()
    }
}

pub enum ExtraItem<'a> {
    Text(&'a str),
    Bytes(&'a [u8]),
}
