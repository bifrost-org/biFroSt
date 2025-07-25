use libc::remove;
use serde_json::json;

use crate::api::models::*;
use crate::config::settings::Config;
use std::time::Duration;

pub struct RemoteClient {
    base_url: String,             // URL del server (es. "http://localhost:8080")
    auth_token: Option<String>,   // Token JWT per autenticazione
    http_client: reqwest::Client, // Client HTTP per le richieste
    timeout: Duration,            // Timeout per le richieste
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

fn remove_last_part(path: &str) -> String {
    if let Some(pos) = path.rfind('/') {
        path[..pos].to_string()
    } else {
        "/".to_string() // Se non c'è '/', ritorna la root
    }
}

fn take_last_part(path: &str) -> String {
    if let Some(pos) = path.rfind('/') {
        path[pos + 1..].to_string()
    } else {
        path.to_string() // Se non c'è '/', ritorna l'intero path
    }
}

impl RemoteClient {
    pub fn new(config: &Config) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            base_url: config.server_full_url(),
            auth_token: None,
            http_client,
            timeout: config.timeout,
        }
    }

// Costruisce URL completo per un endpoint con path parameter opzionale
fn build_url(&self, base_route: &str, path_param: Option<&str>) -> String {
    match path_param {
        Some(param) => {
            // Se c'è un path parameter, codificalo completamente
            let encoded_param = urlencoding::encode(param.trim_start_matches('/'));
            format!("{}{}/{}", self.base_url, base_route, encoded_param)
        }
        None => {
            // Se non c'è path parameter, usa solo il base route
            format!("{}{}", self.base_url, base_route)
        }
    }
}

    async fn handle_response<T>(&self, response: reqwest::Response) -> Result<T, ClientError>
    where
        T: serde::de::DeserializeOwned,
    {
        let status = response.status();

        if status.is_success() {
            Ok(response.json().await?)
        } else {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            Err(self.map_http_error(status.as_u16(), message))
        }
    }

    // Gestisce risposte senza dati (solo success/error)
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

    // Mappa errori HTTP a errori specifici
    fn map_http_error(&self, status: u16, message: String) -> ClientError {
        match status {
            404 => ClientError::NotFound {
                path: "Unknown".to_string(),
            },
            401 | 403 => ClientError::PermissionDenied(message),
            _ => ClientError::Server { status, message },
        }
    }

    // Crea headers con autenticazione
    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();

        if let Some(token) = &self.auth_token {
            let auth_value = format!("Bearer {}", token);
            headers.insert(reqwest::header::AUTHORIZATION, auth_value.parse().unwrap());
        }

        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );

        headers
    }

    // Ottieni metadati di un singolo file/directory
    pub async fn get_file_metadata(&self, path: &str) -> Result<MetaFile, ClientError> {
    let last_part = take_last_part(path);
    let parent_path = remove_last_part(path);
    
    // Usa la nuova firma di build_url
    let url = self.build_url("/list", Some(&parent_path));

        let response = self
            .http_client
            .get(&url)
            .headers(self.auth_headers())
            .send()
            .await?;

        let directory_listing: Result<DirectoryListing, ClientError> =
            self.handle_response(response).await;

        match directory_listing {
            Ok(dir) => {
                let file = dir.files.iter().find(|f| f.name == last_part);
                if file.is_none() {
                    return Err(ClientError::NotFound {
                        path: path.to_string(),
                    });
                }

                let mut ret = file.unwrap().clone();
                ret.name = path.to_string(); // Aggiorna il nome con il path completo
                
                Ok(ret)
            }
            Err(ClientError::NotFound { path }) => Err(ClientError::NotFound { path }),
            Err(err) => Err(ClientError::Server {
                status: 500,
                message: format!("Failed to list directory: {}", err),
            }),
        }
    }

pub async fn list_directory(&self, path: &str) -> Result<DirectoryListing, ClientError> {
    
    let url = self.build_url("/list", Some(path));

    let headers = self.auth_headers();

    let response = match self
        .http_client
        .get(&url)
        .headers(headers)
        .send()
        .await {
            Ok(r) => {
                println!("✅ [RESPONSE] Risposta ricevuta: status={}", r.status());
                r
            },
            Err(e) => {
                println!("❌ [ERROR] Errore nell'invio della richiesta: {}", e);
                return Err(ClientError::Http(e));
            }
        };

    // Deserializza direttamente come Vec<MetaFile> invece di DirectoryListing
    let files: Vec<MetaFile> = match response.json::<Vec<MetaFile>>().await {
        Ok(f) => {
            println!("✅ [PARSING] Parsing completato: {} file trovati", f.len());
            f
        },
        Err(e) => {
            println!("❌ [ERROR] Errore nel parsing della risposta: {:?}", e);
            return Err(ClientError::Http(e));
        }
    };
    
    // Crea DirectoryListing dal Vec<MetaFile>
    let mut directory_listing = DirectoryListing { files };
    
    for (i, file) in directory_listing.files.iter_mut().enumerate() {
        let old_name = file.name.clone();
        
        // Costruisci il path completo
        let full_path = if path == "/" {
            format!("/{}", file.name)
        } else {
            format!("{}/{}", path, file.name)
        };
        
        file.name = full_path;
        println!("  - File[{}]: {} -> {}", i, old_name, file.name);
    }
    
    println!("✅ [COMPLETATO] Funzione list_directory completata con successo");
    Ok(directory_listing)
}
    // Leggi contenuto file
    pub async fn read_file(&self, path: &str) -> Result<FileContent, ClientError> {
        // 1. Codifica correttamente il path

    let url = self.build_url("/files", Some(path));

        let response = self
            .http_client
            .get(&url)
            .headers(self.auth_headers())
            .send()
            .await?;

        let status = response.status();

        // 2. Gestisci risposta binaria, non JSON
        if response.status().is_success() {
            Ok(FileContent {
                data: response.bytes().await?.to_vec(),
            })
        } else {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            Err(self.map_http_error(status.as_u16(), message))
        }
    }

    // Scrivi file (usando multipart/form-data come richiesto dall'API)
pub async fn write_file(&self, write_request: &WriteRequest) -> Result<(), ClientError> {
    println!("🔍 [INIZIO] write_file con path={}", write_request.path);
    
    // Codifica il path per route parameter
    let url = self.build_url("/files", Some(&write_request.path));
    
    println!("🔍 [URL] URL costruito: {}", url);

    // Prepara il JSON dei metadati (includi newPath se necessario)
    let data_size = write_request.data.as_ref().map_or(0, |d| d.len());
    println!("🔍 [DATA] Dimensione dati: {} bytes", data_size);
    
    let metadata = json!({
        "size": data_size,
        "permissions": write_request.permissions_octal.clone().unwrap_or_else(|| "644".to_string()),
        "lastModified": write_request.last_modified.clone().unwrap_or_else(||
            chrono::Utc::now().to_rfc3339()),
        "newPath": write_request.new_path.clone()
    });

    println!("🔍 [METADATA] Metadati preparati: {}", metadata);

    // Converti metadati in stringa JSON
    let metadata_str = serde_json::to_string(&metadata)
        .map_err(ClientError::Serialization)?;

    println!("🔍 [FORM] Preparazione form multipart...");
    
    // Crea form multipart - IMPORTANTE: usa i nomi campo corretti
    let form = reqwest::multipart::Form::new()
        // Campo "metadata" come testo JSON
        .text("metadata", metadata_str)
        // Campo "content" come parte binaria
        .part(
            "content",
            reqwest::multipart::Part::bytes(write_request.data.clone().unwrap_or_default())
                .file_name("file") // Aggiungi filename se necessario
                .mime_str("application/octet-stream")
                .map_err(ClientError::Http)?
        );

    println!("✅ [FORM] Form multipart creato");

    // Headers - NON includere Content-Type (reqwest lo gestisce automaticamente)
    let mut headers = self.auth_headers();
    headers.remove(reqwest::header::CONTENT_TYPE);
    println!("🔍 [HEADERS] Headers finali: {:?}", headers);

    println!("🔍 [REQUEST] Invio richiesta HTTP PUT...");
    let response = self
        .http_client
        .put(&url)
        .headers(headers)
        .multipart(form)
        .send()
        .await
        .map_err(ClientError::Http)?;

    println!("✅ [RESPONSE] Risposta ricevuta: status={}", response.status());

    self.handle_empty_response(response).await
}
    // Crea directory
    pub async fn create_directory(&self, path: &str) -> Result<(), ClientError> {
    let url = self.build_url("/mkdir", Some(path));

        let response = self
            .http_client
            .post(&url)
            .headers(self.auth_headers())
            .send()
            .await?;

        self.handle_empty_response(response).await
    }

    // Elimina file o directory
    pub async fn delete(&self, path: &str) -> Result<(), ClientError> {
    let url = self.build_url("/files", Some(path));

        let response = self
            .http_client
            .delete(&url)
            .headers(self.auth_headers())
            .send()
            .await?;

        self.handle_empty_response(response).await
    }
}
