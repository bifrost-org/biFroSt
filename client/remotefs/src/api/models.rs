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
    #[serde(rename = "name")]
    pub name: String,
    
    #[serde(rename = "size")]
    pub size: u64,
    
    #[serde(rename = "lastModified")]
    pub last_modified: String,
    
    #[serde(rename = "permissions")]
    pub permissions_octal: String,
    
    #[serde(rename = "isDirectory")]
    pub is_directory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryListing {
    pub files: Vec<MetaFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContent {
    pub data: Vec<u8>
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteRequest {
    pub path: String,
    pub new_path: Option<String>,
    pub data: Option<Vec<u8>>,
    pub size: Option<u64>,
    pub permissions_octal: Option<String>,
    pub last_modified: Option<String>,
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