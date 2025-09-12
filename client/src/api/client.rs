use reqwest::header::HeaderMap;
use serde_json::json;

use crate::api::models::*;
use crate::config::settings::Config;
use crate::util::auth::{ExtraItem, UserKeys};
use crate::util::date::format_datetime;
use crate::util::fs::format_permissions;
use crate::util::path::{get_file_name, get_parent_path};
use std::time::Duration;

use moka::sync::Cache as MokaCache;

#[allow(unused_macros)]
macro_rules! debug_println {
    ($($arg:tt)*) => {
        println!($($arg)*); // leave the comment to enable debug logs
    };
}


#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("Server error: {status} - {message}")]
    Server { status: u16, message: String },

    #[error("File not found: {path}")]
    NotFound { path: String },

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub struct RemoteClient {
    base_url: String,
    http_client: reqwest::Client,
    user_keys: UserKeys,
    timeout: Duration,
    pub path_mounting: String,
    cache: MokaCache<String, DirectoryListing>,
}

impl RemoteClient {
    pub fn new(config: &Config, user_keys: Option<UserKeys>) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            base_url: config.server_full_url(),
            http_client,
            user_keys: user_keys.unwrap_or(UserKeys {
                api_key: String::new(),
                secret_key: String::new(),
            }),
            timeout: config.timeout,
            path_mounting: config.mount_point.to_string_lossy().to_string(),
            cache: MokaCache::builder()
                .time_to_live(Duration::from_secs(3*60))
                .time_to_idle(Duration::from_secs(3*60))
                .build(),
        }
    }

    fn build_path(&self, base: &str, extra: Option<&str>) -> String {
        match extra {
            Some(p) if !p.is_empty() => {
                let encoded = urlencoding::encode(p.trim_start_matches('/'));
                format!("{}/{}", base.trim_end_matches('/'), encoded)
            }
            _ => base.trim_end_matches('/').to_string(),
        }
    }

    fn build_url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }


    async fn handle_empty_response(&self, response: reqwest::Response) -> Result<(), ClientError> {
        let status = response.status();

        if status.is_success() {
            Ok(())
        } else {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            Err(self.map_http_error(status.as_u16(), message))
        }
    }

    fn map_http_error(&self, status: u16, message: String) -> ClientError {
        match status {
            404 => ClientError::NotFound {
                path: "Unknown".to_string(),
            },
            401 | 403 => ClientError::PermissionDenied(message),
            _ => ClientError::Server { status, message },
        }
    }

    fn get_headers(
        &self,
        method: &str,
        route_path: &str,
        extra_header: Option<&str>,
        extra_to_be_hashed: Option<Vec<ExtraItem>>,
    ) -> HeaderMap {
        let timestamp = UserKeys::generate_timestamp().to_string();
        let nonce = UserKeys::generate_nonce();

        let hmac_message = self.user_keys.build_hmac_message(
            method,
            route_path,
            {
                let mut v: Vec<&str> = vec![&timestamp, &nonce];
                if let Some(extra) = extra_header {
                    v.push(extra);
                }
                v
            },
            extra_to_be_hashed,
        );

        let final_headers: HeaderMap =
            self.user_keys
                .get_auth_headers(&hmac_message, &timestamp.to_string(), &nonce);
        final_headers
    }

    // Obtain metadata for a single file/directory
    pub async fn get_file_metadata(&self, path: &str) -> Result<MetaFile, ClientError> {
        if path == "/" {
            let now_iso = chrono::Utc::now().to_rfc3339();
            return Ok(MetaFile {
                name: "/".to_string(),
                size: 4096,
                atime: now_iso.clone(),
                mtime: now_iso.clone(),
                ctime: now_iso.clone(),
                crtime: now_iso,
                kind: FileKind::Directory,
                perm: "755".to_string(),
                nlink: 2,
                ref_path: None,
            });
        }

        let parent_path = get_parent_path(path);
        let file_name = get_file_name(path);

        let parent_listing = self.list_directory(&parent_path).await?;

        if let Some(found_file) = parent_listing.files.iter().find(|f| f.name == file_name) {
            let mut result = found_file.clone();
            result.name = path.to_string(); // Mantieni il path completo
            return Ok(result);
        }

        Err(ClientError::NotFound {
            path: path.to_string(),
        })
    }

    pub async fn list_directory(&self, path: &str) -> Result<DirectoryListing, ClientError> {
        let route_path = self.build_path("/list", Some(path));
        let url = self.build_url(&route_path);
        
        
        let headers = self.get_headers("GET", &route_path, None, None);
        
        match self.cache.get(path) {
            Some(cached_response) => {
                return Ok(cached_response.clone());
            }
            None => {
                debug_println!("Metadati requested from server for path: {}", path);
            }
        }

        let response = match self
            .http_client
            .get(&url)
            .headers(headers)
            .timeout(self.timeout)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                debug_println!("❌ [LIST_DIR] Error on sending request: {}", e);
                return Err(ClientError::Http(e));
            }
        };

        // Manage HTTP errors
        if !response.status().is_success() {
            let status_code = response.status().as_u16();
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            debug_println!("{}",message);
            return Err(match status_code {
                404 => ClientError::NotFound {
                    path: path.to_string(),
                },
                403 | 401 => ClientError::PermissionDenied(message),
                _ => ClientError::Server {
                    status: status_code,
                    message,
                },
            });
        }

        let files: Vec<MetaFile> = match response.json::<Vec<MetaFile>>().await {
            Ok(f) => f,
            Err(e) => {
                return Err(ClientError::Http(e));
            }
        };

        let directory_listing = DirectoryListing { files };

        self.cache.insert(path.to_string(), directory_listing.clone());

        Ok(directory_listing)
    }

    pub async fn read_file(
        &self,
        path: &str,
        offset: Option<u64>,
        size: Option<u64>,
    ) -> Result<FileContent, ClientError> {
        let route_path = self.build_path("/files", Some(path));
        let url = self.build_url(&route_path);

        let mut headers;
        // Note: offset and size should be always present
        if let Some(off) = offset {
            let range_value = if let Some(sz) = size {
                format!("bytes={}-{}", off, off + sz - 1)
            } else {
                // from the offset to the end of the file
                format!("bytes={}-", off)
            };
            headers = self.get_headers("GET", &route_path, Some(&range_value), None);
            headers.insert("Range", range_value.parse().expect("Invalid Range header"));
            debug_println!("🔍 [HEADERS] Range: {}", range_value);
        } else {
            headers = self.get_headers("GET", &route_path, None, None);
        }
        // without offset, the server should return the entire file

        let response = self
            .http_client
            .get(&url)
            .headers(headers)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| {
                debug_println!("❌ [READ_FILE] Error on sending request: {}", e);
                ClientError::Http(e)
            })?;

        let status = response.status();
        debug_println!("📖 [READ_FILE] Response received: status={}", status);

        if response.status().is_success() {
            let data = response.bytes().await.map_err(ClientError::Http)?.to_vec();

            Ok(FileContent { data })
        } else {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            debug_println!(
                "❌ [READ_FILE] Errore HTTP {}: {}",
                status.as_u16(),
                message
            );
            Err(self.map_http_error(status.as_u16(), message))
        }
    }

    // Write file (using multipart/form-data as required by the API)
    pub async fn write_file(&self, write_request: &WriteRequest) -> Result<(), ClientError> {
        
        self.cache.invalidate(&get_parent_path(&write_request.path));


        let route_path = self.build_path("/files", Some(&write_request.path));
        let url = self.build_url(&route_path);

        match write_request.kind {
            FileKind::Symlink | FileKind::Hardlink => {
                if write_request.ref_path.is_none() {
                    debug_println!("❌ [WRITE_FILE] refPath mancante per link");
                    return Err(ClientError::Server {
                        status: 400,
                        message: "refPath required for link types".into(),
                    });
                }
            }
            _ => {}
        }

        let has_content = write_request
            .data
            .as_ref()
            .map(|d| !d.is_empty())
            .unwrap_or(false);
        match write_request.mode {
            Mode::Write => {

                if has_content
                    && (write_request.size as usize) != write_request.data.as_ref().unwrap().len()
                {
                    debug_println!("❌ [WRITE_FILE] Size declared ≠ content length (write)");
                    return Err(ClientError::Server {
                        status: 400,
                        message: "Declared size does not match content length (write)".into(),
                    });
                }
            }
            Mode::Append => {
                if !has_content {
                    debug_println!("❌ [WRITE_FILE] Content richiesto in append");
                    return Err(ClientError::Server {
                        status: 400,
                        message: "Content required for append".into(),
                    });
                }
                if (write_request.size as usize) != write_request.data.as_ref().unwrap().len() {
                    debug_println!("❌ [WRITE_FILE] Size declared ≠ content length (append)");
                    return Err(ClientError::Server {
                        status: 400,
                        message: "Declared size does not match content length (append)".into(),
                    });
                }
            }
            Mode::WriteAt => {
                if !has_content {
                    debug_println!("❌ [WRITE_FILE] Content required in write_at");
                    return Err(ClientError::Server {
                        status: 400,
                        message: "Content required for write_at".into(),
                    });
                }
                if write_request.offset.is_none() {
                    debug_println!("❌ [WRITE_FILE] Offset required in write_at");
                    return Err(ClientError::Server {
                        status: 400,
                        message: "Offset required for write_at".into(),
                    });
                }
                if (write_request.size as usize) != write_request.data.as_ref().unwrap().len() {
                    debug_println!("❌ [WRITE_FILE] Size declared ≠ content length (write_at)");
                    return Err(ClientError::Server {
                        status: 400,
                        message: "Declared size does not match content length (write_at)".into(),
                    });
                }
            }
            Mode::Truncate => {
                // Ignore content as per spec
                if has_content {
                    debug_println!("ℹ️ [WRITE_FILE] Content ignored in truncate");
                }
                // size = final requested size
            }
        }

        let effective_size: u64 = match write_request.mode {
            Mode::Truncate => write_request.size, // final requested size
            Mode::Write | Mode::Append | Mode::WriteAt => {
                if has_content {
                    write_request.data.as_ref().unwrap().len() as u64
                } else {
                    // metadata-only update: use declared size (already validated above)
                    write_request.size
                }
            }
        };

        let send_data: Vec<u8> = match write_request.kind {
            FileKind::Symlink | FileKind::Hardlink => {
                if has_content {
                    debug_println!("ℹ️ [WRITE_FILE] Content ignorato per link");
                }
                Vec::new()
            }
            _ => write_request.data.clone().unwrap_or_default(),
        };

        // METADATA JSON

        let mut metadata_map = serde_json::Map::new();
        metadata_map.insert("size".to_string(), json!(effective_size));
        metadata_map.insert(
            "perm".to_string(),
            json!(format_permissions(&write_request.perm)),
        );
        metadata_map.insert(
            "mtime".to_string(),
            json!(format_datetime(&write_request.mtime)),
        );
        metadata_map.insert(
            "atime".to_string(),
            json!(format_datetime(&write_request.atime)),
        );
        metadata_map.insert(
            "ctime".to_string(),
            json!(format_datetime(&write_request.ctime)),
        );
        metadata_map.insert(
            "crtime".to_string(),
            json!(format_datetime(&write_request.crtime)),
        );
        metadata_map.insert("kind".to_string(), json!(write_request.kind.to_string()));
        metadata_map.insert("mode".to_string(), json!(write_request.mode.to_string()));

        if let Some(ref new_path) = write_request.new_path {
            metadata_map.insert("newPath".to_string(), json!(new_path));
        }
        if let Some(ref ref_path) = write_request.ref_path {
            metadata_map.insert("refPath".to_string(), json!(ref_path));
        }
        if let Some(ref offset) = write_request.offset {
            if matches!(write_request.mode, Mode::WriteAt) {
                metadata_map.insert("offset".to_string(), json!(offset));
            }
        }

        let metadata_json = serde_json::Value::Object(metadata_map);

        let metadata_str =
            serde_json::to_string(&metadata_json).map_err(ClientError::Serialization)?;

        let include_content = !send_data.is_empty();

        let extra_items = if include_content {
            Some(vec![
                ExtraItem::Text(&metadata_str),
                ExtraItem::Bytes(&send_data),
            ])
        } else {
            Some(vec![ExtraItem::Text(&metadata_str)])
        };
        let mut headers = self.get_headers("PUT", &route_path, None, extra_items);
        // Headers - NOT include Content-Type (reqwest handles it automatically)
        headers.remove(reqwest::header::CONTENT_TYPE);

        let mut form = reqwest::multipart::Form::new().text("metadata", metadata_str.clone());
        if include_content {
            form = form.part(
                "content",
                reqwest::multipart::Part::bytes(send_data)
                    .file_name("file")
                    .mime_str("application/octet-stream")
                    .map_err(ClientError::Http)?,
            );
        } 

        let response = self
            .http_client
            .put(&url)
            .headers(headers)
            .multipart(form)
            .send()
            .await
            .map_err(ClientError::Http)?;

        let status_code = response.status().as_u16();

        if !(200..=299).contains(&status_code) {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "No response body".to_string());

            debug_println!(
                "❌ [WRITE_FILE] Errore HTTP {}: {}",
                status_code, error_body
            );

            // Debug dettagli della richiesta per 400 Bad Request
            if status_code == 400 || status_code == 404 {
                debug_println!("Metadata JSON inviato:");
                debug_println!("  {}", metadata_str);

                if let Some(data) = &write_request.data {
                    debug_println!("  💾 Data length: {} bytes", data.len());
                    if data.len() <= 100 {
                        debug_println!("  💾 Data content: {:?}", String::from_utf8_lossy(data));
                    }
                } else {
                    debug_println!("  💾 Data: None");
                }
            }

            return Err(match status_code {
                400 => ClientError::Server {
                    status: status_code,
                    message: format!("Bad Request: {}", error_body),
                },
                404 => ClientError::NotFound {
                    path: write_request.path.clone(),
                },
                401 | 403 => ClientError::PermissionDenied(error_body),
                409 => ClientError::Server {
                    status: status_code,
                    message: "Conflict".into(),
                },
                _ => ClientError::Server {
                    status: status_code,
                    message: error_body,
                },
            });
        }

        self.handle_empty_response(response).await
    }

    // Create directory
    pub async fn create_directory(&self, path: &str) -> Result<(), ClientError> {
        let route_path = self.build_path("/mkdir", Some(path));
        let url = self.build_url(&route_path);
        

        self.cache.invalidate(&get_parent_path(&path)); //invalidate the father entries


        let headers = self.get_headers("POST", &route_path, None, None);

        let response = self.http_client.post(&url).headers(headers).send().await?;

        self.handle_empty_response(response).await
    }

    // Delete file or directory
    pub async fn delete(&self, path: &str) -> Result<(), ClientError> {
        let route_path = self.build_path("/files", Some(path));
        let url = self.build_url(&route_path);

        self.cache.invalidate(&get_parent_path(&path));

        let headers = self.get_headers("DELETE", &route_path, None, None);

        let response = self
            .http_client
            .delete(&url)
            .headers(headers)
            .send()
            .await?;

        self.handle_empty_response(response).await
    }

    // user registration
    pub async fn user_registration(&self, username: String) -> Result<UserKeys, ClientError> {
        let route_path = self.build_path("/users", None);
        let url = self.build_url(&route_path);

        let request_body = RegisterRequest { username };

        let response = self
            .http_client
            .post(&url)
            .json(&request_body)
            .timeout(self.timeout)
            .send()
            .await?;

        if response.status().is_success() {
            let keys: UserKeys = response.json().await.map_err(ClientError::Http)?;
            Ok(keys)
        } else {
            let status_code = response.status().as_u16();
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            Err(self.map_http_error(status_code, message))
        }
    }
}
