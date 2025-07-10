use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub token: String,
    pub expires_in: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub user_id: String,
    pub created_at: String,
    pub expires_at: String,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaFile {
    pub path: String,
    pub size_bytes: u64,
    pub last_modified: String,
    pub permissions_octal: String,
    pub owner_name: String,
    pub type_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryListing {
    pub path: String,
    pub files: Vec<MetaFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContent {
    pub data: Vec<u8>,
    pub offset: u64,
    pub total_size: u64,
    pub content_type: Option<String>,
    pub is_partial: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadRequest {
    pub path: String,
    pub offset: Option<u64>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteRequest {
    pub path: String,
    pub operation: WriteOperation,
    pub data: Option<Vec<u8>>,
    pub offset: Option<u64>,
    pub new_path: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WriteOperation {
    Write,
    Move,
    Append,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteRequest {
    pub path: String,
    pub recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFileRequest {
    pub path: String,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDirectoryRequest {
    pub path: String,
    pub permissions_octal: String,
}