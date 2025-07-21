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

    // Costruisce URL completo per un endpoint
    fn build_url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
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
        let url = self.build_url(&format!("/metadata{}", path));

        let response = self
            .http_client
            .get(&url)
            .headers(self.auth_headers())
            .send()
            .await?;

        self.handle_response(response).await
    }

    // Lista contenuto directory
    pub async fn list_directory(&self, path: &str) -> Result<DirectoryListing, ClientError> {
        let url = self.build_url(&format!("/list{}", path));

        let response = self
            .http_client
            .get(&url)
            .headers(self.auth_headers())
            .send()
            .await?;

        self.handle_response(response).await
    }

    // Leggi contenuto file
    pub async fn read_file(
        &self,
        read_request: &ReadRequest,
    ) -> Result<FileContent, ClientError> {
        let mut url = self.build_url(&format!("/files{}", read_request.path));


        // ELIMINARE I QUERY PARAMETERS PRENDERE TUTTO IL FILE
        // Aggiungi parametri query se specificati
        if read_request.offset.is_some() || read_request.size.is_some() {
            let mut query_params = Vec::new();
            if let Some(offset) = read_request.offset {
                query_params.push(format!("offset={}", offset));
            }
            if let Some(size) = read_request.size {
                query_params.push(format!("size={}", size));
            }
            url = format!("{}?{}", url, query_params.join("&"));
        }

        let response = self
            .http_client
            .get(&url)
            .headers(self.auth_headers())
            .send()
            .await?;

        self.handle_response(response).await
    }

    // Scrivi file
    pub async fn write_file(&self, write_request: &WriteRequest) -> Result<(), ClientError> {
        let url = self.build_url(&format!("/files{}", write_request.path));

        let response = self
            .http_client
            .put(&url)
            .headers(self.auth_headers())
            .json(write_request)
            .send()
            .await?;

        self.handle_empty_response(response).await
    }

    // Crea directory
    pub async fn create_directory(&self, create_request: &CreateDirectoryRequest) -> Result<(), ClientError> {
        let url = self.build_url(&format!("/mkdir{}", create_request.path));


        let response = self
            .http_client
            .post(&url)
            .headers(self.auth_headers())
            .json(&create_request)
            .send()
            .await?;

        self.handle_empty_response(response).await
    }

    // Elimina file o directory
    pub async fn delete(&self, delete_request: &DeleteRequest) -> Result<(), ClientError> {
        let url = self.build_url(&format!("/files{}", delete_request.path));

        let response = self
            .http_client
            .delete(&url)
            .headers(self.auth_headers())
            .json(delete_request)
            .send()
            .await?;

        self.handle_empty_response(response).await
    }
}
