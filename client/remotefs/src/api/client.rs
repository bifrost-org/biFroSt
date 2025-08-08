use chrono::offset;
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
/// Converte permessi da formato stringa a numero ottale stringa
fn format_permissions(perm: &str) -> String {
    // Se è già in formato ottale valido (3 cifre), restituiscilo
    if perm.len() == 3 && perm.chars().all(|c| c.is_ascii_digit() && c <= '7') {
        return perm.to_string();
    }
    
    // Conversione da formato simbolico rwx a ottale
    if perm.len() == 9 && (perm.starts_with('r') || perm.starts_with('-')) {
        return symbolic_to_octal(perm);
    }
    
    // Se è un numero decimale, convertilo in ottale
    if let Ok(decimal_perm) = perm.parse::<u32>() {
        // Se è già in formato ottale (cifre <= 7), restituiscilo
        if decimal_perm <= 777 && decimal_perm.to_string().chars().all(|c| c <= '7') {
            return format!("{:03}", decimal_perm);
        }
        // Altrimenti converte da decimale a ottale
        return format!("{:03o}", decimal_perm);
    }
    
    // Conversioni per formati comuni
    match perm {
        "rw-r--r--" => "644",
        "rwxr-xr-x" => "755", 
        "rw-------" => "600",
        "rwxrwxrwx" => "777",
        "r--r--r--" => "444",
        "rwxrwxr-x" => "775",
        _ => "644", // Fallback sicuro
    }.to_string()
}

/// Converte permessi simbolici (rwxrwxrwx) in ottale
fn symbolic_to_octal(symbolic: &str) -> String {
    let mut octal = 0;
    
    // Owner (primi 3 caratteri)
    if symbolic.chars().nth(0) == Some('r') { octal += 400; }
    if symbolic.chars().nth(1) == Some('w') { octal += 200; }
    if symbolic.chars().nth(2) == Some('x') { octal += 100; }
    
    // Group (caratteri 3-5)
    if symbolic.chars().nth(3) == Some('r') { octal += 40; }
    if symbolic.chars().nth(4) == Some('w') { octal += 20; }
    if symbolic.chars().nth(5) == Some('x') { octal += 10; }
    
    // Other (caratteri 6-8)
    if symbolic.chars().nth(6) == Some('r') { octal += 4; }
    if symbolic.chars().nth(7) == Some('w') { octal += 2; }
    if symbolic.chars().nth(8) == Some('x') { octal += 1; }
    
    format!("{:03o}", octal)
}
/// Converte datetime ISO in formato richiesto dal server
fn format_datetime(iso_datetime: &str) -> String {
    // Prova a parsare il datetime ISO
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso_datetime) {
        // Converti in UTC e formatta nel formato richiesto
        dt.with_timezone(&chrono::Utc)
            .format("%Y-%m-%dT%H:%M:%S.000Z")
            .to_string()
    } else {
        // Fallback: genera datetime corrente nel formato giusto
        chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S.000Z")
            .to_string()
    }
}

// Funzioni helper per gestire i path
fn remove_last_part(path: &str) -> String {
    if path == "/" {
        return "/".to_string();
    }

    // Rimuovi trailing slash se presente
    let clean_path = path.trim_end_matches('/');

    // Se il path inizia con '/', trova l'ultimo '/'
    if let Some(last_slash) = clean_path.rfind('/') {
        if last_slash == 0 {
            // Se l'ultimo slash è all'inizio, siamo nella root
            "/".to_string()
        } else {
            clean_path[..last_slash].to_string()
        }
    } else {
        // Nessun slash trovato, restituisci root
        "/".to_string()
    }
}

fn take_last_part(path: &str) -> String {
    if path == "/" {
        return "".to_string();
    }

    // Rimuovi trailing slash se presente
    let clean_path = path.trim_end_matches('/');

    // Trova l'ultimo '/' e prendi tutto quello che segue
    if let Some(last_slash) = clean_path.rfind('/') {
        clean_path[last_slash + 1..].to_string()
    } else {
        // Nessun slash, restituisci l'intero path
        clean_path.to_string()
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
        println!(
            "🔍 [METADATA] Inizio get_file_metadata per path: '{}'",
            path
        );

        // ✅ CASO SPECIALE: ROOT DIRECTORY
        if path == "/" {
            println!("🏠 [METADATA] Root directory richiesta - generando metadati sintetici");
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

        // ✅ STRATEGIA CORRETTA: Separa parent directory e nome file
        let parent_path = remove_last_part(path);
        let file_name = take_last_part(path);

        println!(
            "🔍 [METADATA] Cerco file '{}' nella directory '{}'",
            file_name, parent_path
        );

        // Lista la directory padre
        let parent_listing = self.list_directory(&parent_path).await?;

        // Cerca il file specifico nella lista
        if let Some(found_file) = parent_listing.files.iter().find(|f| f.name == file_name) {
            println!(
                "✅ [METADATA] File '{}' trovato nella directory '{}'!",
                file_name, parent_path
            );
            let mut result = found_file.clone();
            result.name = path.to_string(); // Mantieni il path completo
            return Ok(result);
        }

        println!(
            "❌ [METADATA] File '{}' non trovato nella directory '{}'",
            file_name, parent_path
        );
        Err(ClientError::NotFound {
            path: path.to_string(),
        })
    }

    pub async fn list_directory(&self, path: &str) -> Result<DirectoryListing, ClientError> {
        println!("📁 [LIST_DIR] Inizio list_directory per path: '{}'", path);

        // Costruisci URL - gestisci correttamente la root
        let url = if path == "/" {
            format!("{}/list/", self.base_url)
        } else {
            self.build_url("/list", Some(path))
        };

        println!("📁 [LIST_DIR] URL costruito: {}", url);

        let headers = self.auth_headers();

        let response = match self
            .http_client
            .get(&url)
            .headers(headers)
            .timeout(self.timeout)
            .send()
            .await
        {
            Ok(r) => {
                println!("✅ [LIST_DIR] Risposta ricevuta: status={}", r.status());
                r
            }
            Err(e) => {
                println!("❌ [LIST_DIR] Errore nell'invio della richiesta: {}", e);
                return Err(ClientError::Http(e));
            }
        };

        // Gestisci errori HTTP
        if !response.status().is_success() {
            let status_code = response.status().as_u16();
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());

            println!("❌ [LIST_DIR] Errore HTTP {}: {}", status_code, message);

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

        // Deserializza direttamente come Vec<MetaFile>
        let files: Vec<MetaFile> = match response.json::<Vec<MetaFile>>().await {
            Ok(f) => {
                println!("✅ [LIST_DIR] Parsing completato: {} file trovati", f.len());
                f
            }
            Err(e) => {
                println!("❌ [LIST_DIR] Errore nel parsing della risposta: {:?}", e);
                return Err(ClientError::Http(e));
            }
        };

        // Log dettagli dei file ricevuti
        for (i, file) in files.iter().enumerate() {
            println!(
                "  {}. '{}' ({}, {} bytes, kind: {:?})",
                i + 1,
                file.name,
                match file.kind {
                    FileKind::Directory => "DIR",
                    FileKind::RegularFile => "FILE",
                    FileKind::Symlink => "SYMLINK",
                    FileKind::Hardlink => "HARDLINK",
                },
                file.size,
                file.kind
            );
        }

        // Crea DirectoryListing - MANTIENI i nomi originali dall'API
        let directory_listing = DirectoryListing { files };

        println!("✅ [LIST_DIR] Completato con successo per path: '{}'", path);
        Ok(directory_listing)
    }


    // Leggi contenuto file con support per offset e size
    pub async fn read_file(
        &self,
        path: &str,
        offset: Option<u64>,
        size: Option<u64>,
    ) -> Result<FileContent, ClientError> {
        println!(
            "📖 [READ_FILE] path: '{}', offset: {:?}, size: {:?}",
            path, offset, size
        );

        // 1. Costruisci URL base
        let mut url = self.build_url("/files", Some(path));

        // 2. Aggiungi query parameters per offset e size
        let mut query_params = Vec::new();

        if let Some(offset) = offset {
            query_params.push(format!("offset={}", offset));
        }

        if let Some(size) = size {
            query_params.push(format!("size={}", size));
        }

        // Aggiungi query parameters all'URL se presenti
        if !query_params.is_empty() {
            url = format!("{}?{}", url, query_params.join("&"));
        }

        println!("📖 [READ_FILE] URL finale: {}", url);

        let response = self
            .http_client
            .get(&url)
            .headers(self.auth_headers())
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| {
                println!("❌ [READ_FILE] Errore nell'invio della richiesta: {}", e);
                ClientError::Http(e)
            })?;

        let status = response.status();
        println!("📖 [READ_FILE] Risposta ricevuta: status={}", status);

        // 2. Gestisci risposta binaria, non JSON
        if response.status().is_success() {
            let data = response.bytes().await.map_err(ClientError::Http)?.to_vec();
            println!("✅ [READ_FILE] Letti {} bytes", data.len());
            Ok(FileContent { data })
        } else {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            println!(
                "❌ [READ_FILE] Errore HTTP {}: {}",
                status.as_u16(),
                message
            );
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
 let mut metadata_map = serde_json::Map::new();

metadata_map.insert("size".to_string(), json!(data_size));
metadata_map.insert("perm".to_string(), json!(format_permissions(&write_request.perm)));
metadata_map.insert("mtime".to_string(), json!(format_datetime(&write_request.mtime)));
metadata_map.insert("atime".to_string(), json!(format_datetime(&write_request.atime)));
metadata_map.insert("ctime".to_string(), json!(format_datetime(&write_request.ctime)));
metadata_map.insert("crtime".to_string(), json!(format_datetime(&write_request.crtime)));
metadata_map.insert("kind".to_string(), json!(write_request.kind.to_string()));
metadata_map.insert("mode".to_string(), json!(write_request.mode.to_string()));

// ✅ Aggiungi newPath solo se non è None
if let Some(ref new_path) = write_request.new_path {
    metadata_map.insert("newPath".to_string(), json!(new_path));
}

// ✅ Aggiungi refPath solo se non è None  
if let Some(ref ref_path) = write_request.ref_path {
    metadata_map.insert("refPath".to_string(), json!(ref_path));
}

if let Some(ref offset) = write_request.offset {
    metadata_map.insert("offset".to_string(), json!(offset));
}

let metadata_json = serde_json::Value::Object(metadata_map);
        println!("🔍 [METADATA] Metadati preparati: {}", metadata_json);

        // Converti metadati in stringa JSON
        let metadata_str =
            serde_json::to_string(&metadata_json).map_err(ClientError::Serialization)?;

        println!("🔍 [FORM] Preparazione form multipart...");

        // Crea form multipart - IMPORTANTE: usa i nomi campo corretti
        let form = reqwest::multipart::Form::new()
            // Campo "metadata" come testo JSON
            .text("metadata", metadata_str.clone())
            // Campo "content" come parte binaria
            .part(
                "content",
                reqwest::multipart::Part::bytes(write_request.data.clone().unwrap_or_default())
                    .file_name("file") // Aggiungi filename se necessario
                    .mime_str("application/octet-stream")
                    .map_err(ClientError::Http)?,
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

        println!(
            "✅ [RESPONSE] Risposta ricevuta: status={}",
            response.status()
        );

        // ✅ AGGIUNGI DEBUG DETTAGLIATO
        if !response.status().is_success() {
            let status_code = response.status().as_u16();
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "No response body".to_string());

            println!(
                "❌ [WRITE_FILE] Errore HTTP {}: {}",
                status_code, error_body
            );

            // Debug dettagli della richiesta per 400 Bad Request
            if status_code == 400 || status_code == 404 {
                println!("Metadata JSON inviato:");
                println!("  {}", metadata_str);

                if let Some(data) = &write_request.data {
                    println!("  💾 Data length: {} bytes", data.len());
                    if data.len() <= 100 {
                        println!("  💾 Data content: {:?}", String::from_utf8_lossy(data));
                    }
                } else {
                    println!("  💾 Data: None");
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
                403 | 401 => ClientError::PermissionDenied(error_body),
                _ => ClientError::Server {
                    status: status_code,
                    message: error_body,
                },
            });
        } else {
            println!("✅ [WRITE_FILE] Richiesta completata con successo");
        }

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
