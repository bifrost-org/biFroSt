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

    #[serde(rename = "atime")]
    pub atime: String,

    #[serde(rename = "mtime")]
    pub mtime: String,

    #[serde(rename = "ctime")]
    pub ctime: String,

    #[serde(rename = "crtime")]
    pub crtime: String,

    #[serde(rename = "kind")]
    pub kind: FileKind,

    #[serde(rename = "perm")]
    pub perm: String,

    #[serde(rename = "nlink")]
    pub nlink: u32,

    #[serde(rename = "refPath")]
    pub ref_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")] // ← Aggiunto per gestire snake_case
pub enum FileKind {
    #[serde(rename = "regular_file")]
    RegularFile,
    #[serde(rename = "directory")]
    Directory,
    #[serde(rename = "soft_link")]
    Symlink,
    #[serde(rename = "hard_link")]
    Hardlink,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryListing {
    pub files: Vec<MetaFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContent {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteRequest {
    pub path: String,
    pub new_path: Option<String>,
    pub size: u64,
    pub atime: String,
    pub mtime: String,
    pub ctime: String,
    pub crtime: String,
    pub kind: FileKind,
    pub ref_path: Option<String>,
    pub perm: String,
    pub mode: Mode,
    pub data: Option<Vec<u8>>,
    pub offset: Option<u64>,
}

impl FileKind {
    pub fn to_string(&self) -> String {
        match self {
            FileKind::RegularFile => "regular_file".to_string(),
            FileKind::Directory => "directory".to_string(),
            FileKind::Symlink => "soft_link".to_string(),
            FileKind::Hardlink => "hard_link".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Mode {
    #[serde(rename = "write")]
    Write,
    #[serde(rename = "append")]
    Append,
    #[serde(rename = "write_at")]
    WriteAt,
    #[serde(rename = "truncate")]
    Truncate,
}

impl Mode {
    pub fn to_string(&self) -> String {
        match self {
            Mode::Write => "write".to_string(),
            Mode::Append => "append".to_string(),
            Mode::WriteAt => "write_at".to_string(),
            Mode::Truncate => "truncate".to_string(),
        }
    }
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

#[derive(Serialize)]
pub struct RegisterRequest {
    pub username: String,
}
