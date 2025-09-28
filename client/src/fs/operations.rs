#![allow(warnings)]

use crate::api::client::{ClientError, RemoteClient};
use crate::api::models::*;
use crate::fs::attributes::{self, new_directory_attr, new_file_attr};
use fuser::consts::FOPEN_DIRECT_IO;
use fuser::{
    FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen, Request,
};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::time::{Duration, SystemTime};

const STREAM_WRITE: usize = 4 * 1024 * 1024; // 4MB
pub struct RemoteFileSystem {
    inode_to_path: HashMap<u64, String>,
    path_to_inode: HashMap<String, u64>,
    next_inode: u64,

    client: RemoteClient,

    open_files: HashMap<u64, OpenFile>,
    next_fh: u64,

    open_dirs: HashMap<u64, OpenDir>,
    file_locks: HashMap<u64, Vec<FileLock>>, // inode -> locks
}

struct FileLock {
    typ: i32,
    start: u64,
    end: u64,
    pid: u32,
    lock_owner: u64,
}

struct OpenDir {
    path: String,
    flags: i32,
}

struct OpenFile {
    path: String,
    flags: i32,
    write_buffer: Vec<u8>,
    buffer_dirty: bool, // points out if the buffer must be flushed
}

struct Permissions {
    owner: u32,
    group: u32,
    other: u32,
}

fn parse_permissions(perm_str: &str) -> Permissions {
    match u32::from_str_radix(perm_str, 8) {
        Ok(perms) => Permissions {
            owner: (perms >> 6) & 0o7,
            group: (perms >> 3) & 0o7,
            other: perms & 0o7,
        },
        Err(_) => Permissions {
            owner: 0o6, // Default read+write
            group: 0o4, // Default read
            other: 0o4, // Default read
        },
    }
}

fn ranges_overlap(start1: u64, end1: u64, start2: u64, end2: u64) -> bool {
    start1 <= end2 && start2 <= end1
}

fn locks_conflict(typ1: i32, typ2: i32) -> bool {
    typ1 == libc::F_WRLCK || typ2 == libc::F_WRLCK
}

impl RemoteFileSystem {
    pub fn new(client: RemoteClient) -> Self {
        let mut fs = Self {
            inode_to_path: HashMap::new(),
            path_to_inode: HashMap::new(),
            next_inode: 2, // 1 is reserved for root
            client,
            open_files: HashMap::new(),
            next_fh: 1,
            open_dirs: HashMap::new(),
            file_locks: HashMap::new(),
        };

        fs.inode_to_path.insert(1, "/".to_string());
        fs.path_to_inode.insert("/".to_string(), 1);

        fs
    }

    fn generate_inode(&mut self) -> u64 {
        let inode = self.next_inode;
        self.next_inode += 1;
        inode
    }

    fn get_path(&self, inode: u64) -> Option<&String> {
        self.inode_to_path.get(&inode)
    }

    fn register_inode(&mut self, inode: u64, path: String) {
        self.inode_to_path.insert(inode, path.clone());
        self.path_to_inode.insert(path, inode);
    }

    fn unregister_inode(&mut self, inode: u64) {
        if let Some(path) = self.inode_to_path.remove(&inode) {
            self.path_to_inode.remove(&path);
        }
    }

    fn remove_path_mapping(&mut self, path: &str) {
        if let Some(inode) = self.path_to_inode.remove(path) {
            if let Some(current) = self.inode_to_path.get(&inode).cloned() {
                if current == path {
                    if let Some((alt_path, _)) =
                        self.path_to_inode.iter().find(|(_, &ino)| ino == inode)
                    {
                        self.inode_to_path.insert(inode, alt_path.clone());
                    } else {
                        self.inode_to_path.remove(&inode);
                    }
                }
            }
        }
    }

    fn get_current_attributes(&mut self, ino: u64, path: &str, reply: ReplyAttr) {
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };

        match rt.block_on(async { self.client.get_file_metadata(path).await }) {
            Ok(metadata) => {
                let attr = attributes::from_metadata(ino, &metadata);
                let ttl = Duration::from_secs(1); // Cache TTL

                reply.attr(&ttl, &attr);
            }
            Err(e) => {
                reply.error(libc::EIO);
            }
        }
    }
}

impl Filesystem for RemoteFileSystem {
    fn init(
        &mut self,
        _req: &Request<'_>,
        _config: &mut fuser::KernelConfig,
    ) -> Result<(), libc::c_int> {
        let _ = _config.set_max_write(1024 * 1024);
        let _ = _config.set_max_readahead(1024 * 1024);

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        match rt.block_on(async {
            match self.client.get_file_metadata("/").await {
                Ok(_) => Ok(()),
                Err(ClientError::NotFound { .. }) => Ok(()),
                Err(e) => Err(e),
            }
        }) {
            Ok(_) => {
                let _ = rt.block_on(async {
                    if let Ok(listing) = self.client.list_directory("/").await {
                        for entry in listing.files {
                            if !self.path_to_inode.contains_key(&entry.name) {
                                let new_inode = self.generate_inode();
                                self.register_inode(new_inode, entry.name);
                            }
                        }
                    }
                });

                Ok(())
            }
            Err(e) => Err(libc::EIO),
        }
    }

    fn destroy(&mut self) {}

    fn lookup(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let filename = match name.to_str() {
            Some(s) => s,
            None => {
                eprintln!("❌ [LOOKUP] Nome file non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        if filename == "." {
            let parent_path = self.get_path(parent).cloned().unwrap_or("/".to_string());
            let rt = match tokio::runtime::Handle::try_current() {
                Ok(handle) => handle,
                Err(_) => {
                    let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                    runtime.handle().clone()
                }
            };

            match rt.block_on(async { self.client.get_file_metadata(&parent_path).await }) {
                Ok(metadata) => {
                    let attr = attributes::from_metadata(parent, &metadata);
                    let ttl = Duration::from_secs(1);
                    reply.entry(&ttl, &attr, 0);
                    return;
                }
                Err(_) => {
                    let attr = attributes::new_directory_attr(parent, 0o755);
                    let ttl = Duration::from_secs(1);
                    reply.entry(&ttl, &attr, 0);
                    return;
                }
            }
        }

        if filename == ".." {
            let parent_attr = if parent == 1 {
                attributes::new_directory_attr(1, 0o755)
            } else {
                let parent_path = self.get_path(parent).cloned().unwrap_or("/".to_string());
                let grandparent_path = if parent_path == "/" {
                    "/".to_string()
                } else {
                    std::path::Path::new(&parent_path)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or("/".to_string())
                };

                let grandparent_ino = self
                    .path_to_inode
                    .get(&grandparent_path)
                    .copied()
                    .unwrap_or(1);
                attributes::new_directory_attr(grandparent_ino, 0o755)
            };

            let ttl = Duration::from_secs(1);
            reply.entry(&ttl, &parent_attr, 0);
            return;
        }

        let parent_path = match self.get_path(parent) {
            Some(path) => path.clone(),
            None => {
                eprintln!(
                    "❌ [LOOKUP] Directory padre con inode {} non trovata",
                    parent
                );
                reply.error(libc::ENOENT);
                return;
            }
        };

        let full_path = if parent_path == "/" {
            format!("/{}", filename)
        } else {
            format!("{}/{}", parent_path, filename)
        };

        if let Some(&existing_inode) = self.path_to_inode.get(&full_path) {
            let rt = match tokio::runtime::Handle::try_current() {
                Ok(handle) => handle,
                Err(_) => {
                    let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                    runtime.handle().clone()
                }
            };
            match rt.block_on(async { self.client.get_file_metadata(&full_path).await }) {
                Ok(metadata) => {
                    let attr = attributes::from_metadata(existing_inode, &metadata);
                    let ttl = Duration::from_secs(1);
                    reply.entry(&ttl, &attr, 0);
                    return;
                }
                Err(ClientError::NotFound { .. }) => {
                    self.unregister_inode(existing_inode);
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(e) => {
                    eprintln!("❌ [LOOKUP] Errore verifica cache: {}", e);
                    let attr = attributes::new_file_attr(existing_inode, 0, 0o644);
                    let ttl = Duration::from_secs(1);
                    reply.entry(&ttl, &attr, 0);
                    return;
                }
            }
        }

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        let metadata_result =
            rt.block_on(async { self.client.get_file_metadata(&full_path).await });

        match metadata_result {
            Ok(metadata) => {
                let new_inode = self.generate_inode();
                self.register_inode(new_inode, full_path.clone());

                let attr = attributes::from_metadata(new_inode, &metadata);
                let ttl = Duration::from_secs(1);
                reply.entry(&ttl, &attr, 0);
            }
            Err(ClientError::NotFound { .. }) => {
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                reply.error(libc::EACCES);
            }
            Err(e) => {
                reply.error(libc::EIO);
            }
        }
    }

    fn forget(&mut self, _req: &Request<'_>, _ino: u64, _nlookup: u64) {}

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        if ino == 1 {
            let attr = attributes::new_directory_attr(1, 0o755);
            let ttl = Duration::from_secs(1);
            reply.attr(&ttl, &attr);
            return;
        }

        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };

        let metadata_result = rt.block_on(async { self.client.get_file_metadata(&path).await });

        match metadata_result {
            Ok(metadata) => {
                let attr = attributes::from_metadata(ino, &metadata);

                let ttl = Duration::from_secs(1);
                reply.attr(&ttl, &attr);
            }
            Err(ClientError::NotFound { .. }) => {
                reply.error(libc::ENOENT);
            }
            Err(e) => {
                reply.error(libc::EIO);
            }
        }
    }
    fn setattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>,
        _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<SystemTime>,
        fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        if _atime.is_some() || _mtime.is_some() || _ctime.is_some() {}

        if ino == 1 {
            log::warn!("⚠️ [SETATTR] Tentativo di modificare directory root");
            reply.error(libc::EPERM);
            return;
        }

        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                eprintln!("❌ [SETATTR] Inode {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };

        let current_metadata =
            match rt.block_on(async { self.client.get_file_metadata(&path).await }) {
                Ok(metadata) => metadata,
                Err(ClientError::NotFound { .. }) => {
                    eprintln!("❌ [SETATTR] File non trovato sul server: {}", path);
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(e) => {
                    eprintln!(
                        "❌ [SETATTR] Errore recupero metadati per '{}': {}",
                        path, e
                    );
                    reply.error(libc::EIO);
                    return;
                }
            };

        if let Some(new_size) = size {
            match current_metadata.kind {
                FileKind::Directory => {
                    log::warn!("⚠️ [SETATTR] Tentativo di truncate su directory: {}", path);
                    reply.error(libc::EISDIR);
                    return;
                }
                _ => {}
            }

            let current_size = current_metadata.size;

            if new_size == current_size {
                self.get_current_attributes(ino, &path, reply);
                return;
            }

            let now_iso = chrono::Utc::now().to_rfc3339();

            let operation_result = if new_size < current_size {
                rt.block_on(async {
                    self.client
                        .write_file(
                            &(WriteRequest {
                                offset: None,
                                path: path.clone(),
                                new_path: None,
                                size: new_size,
                                atime: now_iso.clone(),
                                mtime: now_iso.clone(),
                                ctime: now_iso.clone(),
                                crtime: current_metadata.crtime.clone(),
                                kind: current_metadata.kind,
                                ref_path: None,
                                perm: current_metadata.perm.clone(),
                                mode: Mode::Truncate,
                                data: None,
                            }),
                        )
                        .await
                })
            } else {
                let padding_size = new_size - current_size;
                let padding_data = vec![0u8; padding_size as usize];

                rt.block_on(async {
                    self.client
                        .write_file(
                            &(WriteRequest {
                                offset: None,
                                path: path.clone(),
                                new_path: None,
                                size: padding_size,
                                atime: now_iso.clone(),
                                mtime: now_iso.clone(),
                                ctime: now_iso.clone(),
                                crtime: current_metadata.crtime.clone(),
                                kind: current_metadata.kind,
                                ref_path: None,
                                perm: current_metadata.perm.clone(),
                                mode: Mode::Append,
                                data: Some(padding_data),
                            }),
                        )
                        .await
                })
            };

            match operation_result {
                Ok(()) => {
                    self.get_current_attributes(ino, &path, reply);
                }
                Err(e) => {
                    eprintln!("❌ [SETATTR] Errore modifica dimensione: {}", e);
                    let error_code = match e {
                        ClientError::NotFound { .. } => libc::ENOENT,
                        ClientError::PermissionDenied(_) => libc::EPERM,
                        ClientError::Server { status: 413, .. } => libc::EFBIG, // File too big
                        ClientError::Server { status: 507, .. } => libc::ENOSPC, // No space left on device
                        _ => libc::EIO,
                    };
                    reply.error(error_code);
                }
            }
            return;
        }

        if let Some(new_mode) = mode {
            let new_permissions = format!("{:o}", new_mode & 0o777);
            let now_iso = chrono::Utc::now().to_rfc3339();

            let chmod_request = WriteRequest {
                offset: None,
                path: path.clone(),
                new_path: None,
                size: current_metadata.size,
                atime: current_metadata.atime.clone(),
                mtime: current_metadata.mtime.clone(),
                ctime: now_iso,
                crtime: current_metadata.crtime.clone(),
                kind: current_metadata.kind,
                ref_path: None,
                perm: new_permissions,
                mode: Mode::Write,
                data: None,
            };

            match rt.block_on(async { self.client.write_file(&chmod_request).await }) {
                Ok(()) => {
                    self.get_current_attributes(ino, &path, reply);
                }
                Err(e) => {
                    eprintln!("❌ [SETATTR] Errore modifica permessi: {}", e);
                    let error_code = match e {
                        ClientError::NotFound { .. } => libc::ENOENT,
                        ClientError::PermissionDenied(_) => libc::EPERM,
                        _ => libc::EIO,
                    };
                    reply.error(error_code);
                }
            }
            return;
        }

        if uid.is_some() || gid.is_some() {
            log::warn!("⚠️ [SETATTR] Cambio uid/gid non supportato su filesystem remoto");
            reply.error(libc::EPERM);
            return;
        }
        if _atime.is_some() || _mtime.is_some() || _ctime.is_some() {
            fn ton_to_rfc3339(t: Option<fuser::TimeOrNow>, fallback_iso: &str) -> String {
                match t {
                    Some(fuser::TimeOrNow::Now) => chrono::Utc::now().to_rfc3339(),
                    Some(_) => fallback_iso.to_string(),
                    None => fallback_iso.to_string(),
                }
            }
            let new_atime = ton_to_rfc3339(_atime, &current_metadata.atime);
            let new_mtime = ton_to_rfc3339(_mtime, &current_metadata.mtime);
            let new_ctime = match _ctime {
                Some(st) => {
                    let dt: chrono::DateTime<chrono::Utc> = st.into();
                    dt.to_rfc3339()
                }
                None => current_metadata.ctime.clone(),
            };

            let touch_req = WriteRequest {
                offset: None,
                path: path.clone(),
                new_path: None,
                size: current_metadata.size,
                atime: new_atime,
                mtime: new_mtime,
                ctime: new_ctime,
                crtime: current_metadata.crtime.clone(),
                kind: current_metadata.kind,
                ref_path: current_metadata.ref_path.clone(),
                perm: current_metadata.perm.clone(),
                mode: Mode::Write,
                data: None,
            };

            match rt.block_on(async { self.client.write_file(&touch_req).await }) {
                Ok(()) => self.get_current_attributes(ino, &path, reply),
                Err(_) => reply.error(libc::EIO),
            }
            return;
        }

        if flags.is_some() {
            log::warn!("⚠️ [SETATTR] Cambio flags non supportato");
            reply.error(libc::ENOSYS);
            return;
        }

        self.get_current_attributes(ino, &path, reply);
    }

    fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {
        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                eprintln!("❌ [READLINK] Inode {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };

        match rt.block_on(async { self.client.get_file_metadata(&path).await }) {
            Ok(metadata) => {
                println!("Metadata: {:?}", metadata);
                match (metadata.kind, &metadata.ref_path) {
                    (FileKind::Symlink, Some(target)) if !target.is_empty() => {
                        reply.data(target.as_bytes());
                    }
                    (FileKind::Symlink, _) => {
                        println!("❌ [READLINK] Symlink senza target valido: {}", path);
                        reply.error(libc::EIO);
                    }
                    (FileKind::RegularFile, _) => {
                        reply.error(libc::EINVAL);
                    }
                    (FileKind::Directory, _) => {
                        println!("❌ [READLINK] Tentativo di readlink su directory: {}", path);
                        reply.error(libc::EINVAL);
                    }
                    (FileKind::Hardlink, _) => {
                        println!("❌ [READLINK] Tentativo di readlink su hardlink: {}", path);
                        reply.error(libc::EINVAL);
                    }
                }
            }
            Err(ClientError::NotFound { .. }) => {
                eprintln!("❌ [READLINK] File non trovato: {}", path);
                reply.error(libc::ENOENT);
            }
            Err(e) => {
                eprintln!("❌ [READLINK] Errore server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn mknod(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        rdev: u32,
        reply: ReplyEntry,
    ) {
        let filename = match name.to_str() {
            Some(s) => s,
            None => {
                eprintln!("❌ [MKNOD] Nome file non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                eprintln!(
                    "❌ [MKNOD] Directory padre con inode {} non trovata",
                    parent
                );
                reply.error(libc::ENOENT);
                return;
            }
        };

        let full_path = if parent_path == "/" {
            format!("/{}", filename)
        } else {
            format!("{}/{}", parent_path, filename)
        };

        if self.path_to_inode.contains_key(&full_path) {
            log::warn!("⚠️ [MKNOD] File già esistente: {}", full_path);
            reply.error(libc::EEXIST);
            return;
        }

        let file_type = mode & libc::S_IFMT;

        match file_type {
            libc::S_IFREG => {
                let rt = match tokio::runtime::Handle::try_current() {
                    Ok(handle) => handle,
                    Err(_) => {
                        let runtime =
                            tokio::runtime::Runtime::new().expect("Failed to create runtime");
                        runtime.handle().clone()
                    }
                };

                let write_request = WriteRequest {
                    offset: None,
                    path: full_path.clone(),
                    new_path: None,
                    size: 0,
                    atime: chrono::Utc::now().to_rfc3339(),
                    mtime: chrono::Utc::now().to_rfc3339(),
                    ctime: chrono::Utc::now().to_rfc3339(),
                    crtime: chrono::Utc::now().to_rfc3339(),
                    kind: FileKind::RegularFile,
                    ref_path: None,
                    perm: (mode & 0o777 & !(umask & 0o777)).to_string(),
                    mode: Mode::Write,
                    data: Some(Vec::new()),
                };

                let create_result =
                    rt.block_on(async { self.client.write_file(&write_request).await });

                match create_result {
                    Ok(()) => {
                        let new_inode = self.generate_inode();
                        self.register_inode(new_inode, full_path.clone());

                        let metadata_result =
                            rt.block_on(async { self.client.get_file_metadata(&full_path).await });

                        match metadata_result {
                            Ok(metadata) => {
                                let attr = attributes::from_metadata(new_inode, &metadata);
                                let ttl = Duration::from_secs(1);
                                reply.entry(&ttl, &attr, 0);
                            }
                            Err(e) => {
                                eprintln!(
                                    "❌ [MKNOD] Errore recupero metadati dopo creazione: {}",
                                    e
                                );
                                let effective_perms = mode & 0o777 & !(umask & 0o777);
                                let attr = new_file_attr(new_inode, 0, effective_perms);
                                let ttl = Duration::from_secs(1);
                                reply.entry(&ttl, &attr, 0);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ [MKNOD] Errore creazione file sul server: {}", e);
                        match e {
                            ClientError::NotFound { .. } => reply.error(libc::ENOENT),
                            _ => reply.error(libc::EIO),
                        }
                    }
                }
            }
            libc::S_IFIFO => {
                log::warn!("⚠️ [MKNOD] Named pipe non supportato: {}", full_path);
                reply.error(libc::EPERM);
            }
            libc::S_IFCHR => {
                log::warn!(
                    "⚠️ [MKNOD] Character device non supportato: {} (rdev: {})",
                    full_path,
                    rdev
                );
                reply.error(libc::EPERM);
            }
            libc::S_IFBLK => {
                log::warn!(
                    "⚠️ [MKNOD] Block device non supportato: {} (rdev: {})",
                    full_path,
                    rdev
                );
                reply.error(libc::EPERM);
            }
            libc::S_IFSOCK => {
                log::warn!("⚠️ [MKNOD] Socket non supportato: {}", full_path);
                reply.error(libc::EPERM);
            }
            _ => {
                eprintln!("❌ [MKNOD] Tipo file sconosciuto: {:#o}", file_type);
                reply.error(libc::EINVAL);
            }
        }
    }

    fn mkdir(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        let dirname = match name.to_str() {
            Some(s) => s,
            None => {
                eprintln!("❌ [MKDIR] Nome directory non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                eprintln!(
                    "❌ [MKDIR] Directory padre con inode {} non trovata",
                    parent
                );
                reply.error(libc::ENOENT);
                return;
            }
        };

        let full_path = if parent_path == "/" {
            format!("/{}", dirname)
        } else {
            format!("{}/{}", parent_path, dirname)
        };

        if self.path_to_inode.contains_key(&full_path) {
            log::warn!("⚠️ [MKDIR] Directory già esistente: {}", full_path);
            reply.error(libc::EEXIST);
            return;
        }

        let effective_permissions = mode & 0o777 & !(umask & 0o777);
        let permissions_octal = format!("{:o}", effective_permissions);

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };

        let create_result = rt.block_on(async { self.client.create_directory(&full_path).await });

        match create_result {
            Ok(()) => {
                let new_inode = self.generate_inode();
                self.register_inode(new_inode, full_path.clone());

                let metadata_result =
                    rt.block_on(async { self.client.get_file_metadata(&full_path).await });

                match metadata_result {
                    Ok(metadata) => {
                        let attr = attributes::from_metadata(new_inode, &metadata);
                        let ttl = Duration::from_secs(1);
                        reply.entry(&ttl, &attr, 0);
                    }
                    Err(e) => {
                        eprintln!("❌ [MKDIR] Errore recupero metadati dopo creazione: {}", e);
                        let attr = new_directory_attr(new_inode, effective_permissions);
                        let ttl = Duration::from_secs(1);
                        reply.entry(&ttl, &attr, 0);
                    }
                }
            }
            Err(e) => {
                eprintln!("❌ [MKDIR] Errore creazione directory sul server: {}", e);
                match e {
                    ClientError::NotFound { .. } => reply.error(libc::ENOENT),
                    _ => reply.error(libc::EIO),
                }
            }
        }
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        let filename = match name.to_str() {
            Some(s) => s,
            None => {
                eprintln!("❌ [UNLINK] Nome file non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                eprintln!(
                    "❌ [UNLINK] Directory padre con inode {} non trovata",
                    parent
                );
                reply.error(libc::ENOENT);
                return;
            }
        };

        let full_path = if parent_path == "/" {
            format!("/{}", filename)
        } else {
            format!("{}/{}", parent_path, filename)
        };

        let file_inode = match self.path_to_inode.get(&full_path) {
            Some(&inode) => inode,
            None => {
                log::warn!("⚠️ [UNLINK] File non trovato nella cache: {}", full_path);
                let rt = match tokio::runtime::Handle::try_current() {
                    Ok(handle) => handle,
                    Err(_) => {
                        let runtime =
                            tokio::runtime::Runtime::new().expect("Failed to create runtime");
                        runtime.handle().clone()
                    }
                };
                match rt.block_on(async { self.client.get_file_metadata(&full_path).await }) {
                    Ok(_) => {}
                    Err(ClientError::NotFound { .. }) => {
                        reply.error(libc::ENOENT);
                        return;
                    }
                    Err(e) => {
                        eprintln!("❌ [UNLINK] Errore verifica esistenza: {}", e);
                        reply.error(libc::EIO);
                        return;
                    }
                }
                0 // Placeholder, file not in local cache
            }
        };

        if file_inode != 0 {
            let rt = match tokio::runtime::Handle::try_current() {
                Ok(handle) => handle,
                Err(_) => {
                    let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                    runtime.handle().clone()
                }
            };
            match rt.block_on(async { self.client.get_file_metadata(&full_path).await }) {
                Ok(metadata) => {
                    if metadata.kind == FileKind::Directory {
                        log::warn!(
                            "⚠️ [UNLINK] Tentativo di unlink su directory: {}",
                            full_path
                        );
                        reply.error(libc::EISDIR);
                        return;
                    }
                }
                Err(ClientError::NotFound { .. }) => {
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(e) => {
                    eprintln!("❌ [UNLINK] Errore verifica tipo file: {}", e);
                    reply.error(libc::EIO);
                    return;
                }
            }
        }

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        let delete_result = rt.block_on(async { self.client.delete(&full_path).await });

        match delete_result {
            Ok(()) => {
                self.remove_path_mapping(&full_path);

                reply.ok();
            }
            Err(ClientError::NotFound { .. }) => {
                log::warn!("⚠️ [UNLINK] File già eliminato dal server: {}", full_path);
                self.remove_path_mapping(&full_path);
                reply.ok();
            }
            Err(e) => {
                eprintln!("❌ [UNLINK] Errore eliminazione dal server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        let dirname = match name.to_str() {
            Some(s) => s,
            None => {
                eprintln!("❌ [RMDIR] Nome directory non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        if dirname == "." || dirname == ".." {
            log::warn!(
                "⚠️ [RMDIR] Tentativo di eliminare directory speciale: {}",
                dirname
            );
            reply.error(libc::EINVAL);
            return;
        }

        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                eprintln!(
                    "❌ [RMDIR] Directory padre con inode {} non trovata",
                    parent
                );
                reply.error(libc::ENOENT);
                return;
            }
        };

        let full_path = if parent_path == "/" {
            format!("/{}", dirname)
        } else {
            format!("{}/{}", parent_path, dirname)
        };

        if full_path == "/" {
            log::warn!("⚠️ [RMDIR] Tentativo di eliminare directory root");
            reply.error(libc::EBUSY);
            return;
        }

        let dir_inode = match self.path_to_inode.get(&full_path) {
            Some(&inode) => inode,
            None => {
                log::warn!(
                    "⚠️ [RMDIR] Directory non trovata nella cache: {}",
                    full_path
                );
                let rt = match tokio::runtime::Handle::try_current() {
                    Ok(handle) => handle,
                    Err(_) => {
                        let runtime =
                            tokio::runtime::Runtime::new().expect("Failed to create runtime");
                        runtime.handle().clone()
                    }
                };
                match rt.block_on(async { self.client.get_file_metadata(&full_path).await }) {
                    Ok(metadata) => {
                        if metadata.kind != FileKind::Directory {
                            log::warn!("⚠️ [RMDIR] '{}' non è una directory", full_path);
                            reply.error(libc::ENOTDIR);
                            return;
                        }
                    }
                    Err(ClientError::NotFound { .. }) => {
                        reply.error(libc::ENOENT);
                        return;
                    }
                    Err(e) => {
                        eprintln!("❌ [RMDIR] Errore verifica esistenza: {}", e);
                        reply.error(libc::EIO);
                        return;
                    }
                }
                0 // Placeholder, directory non in local cache
            }
        };

        if dir_inode != 0 {
            let rt = match tokio::runtime::Handle::try_current() {
                Ok(handle) => handle,
                Err(_) => {
                    let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                    runtime.handle().clone()
                }
            };
            match rt.block_on(async { self.client.get_file_metadata(&full_path).await }) {
                Ok(metadata) => {
                    if metadata.kind != FileKind::Directory {
                        log::warn!("⚠️ [RMDIR] Tentativo di rmdir su file: {}", full_path);
                        reply.error(libc::ENOTDIR);
                        return;
                    }
                }
                Err(ClientError::NotFound { .. }) => {
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(e) => {
                    eprintln!("❌ [RMDIR] Errore verifica tipo directory: {}", e);
                    reply.error(libc::EIO);
                    return;
                }
            }
        }

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        match rt.block_on(async { self.client.list_directory(&full_path).await }) {
            Ok(listing) => {
                if !listing.files.is_empty() {
                    log::warn!(
                        "⚠️ [RMDIR] Directory non vuota: {} ({} elementi)",
                        full_path,
                        listing.files.len()
                    );
                    reply.error(libc::ENOTEMPTY);
                    return;
                }
            }
            Err(ClientError::NotFound { .. }) => {}
            Err(e) => {
                eprintln!("❌ [RMDIR] Errore verifica directory vuota: {}", e);
                reply.error(libc::EIO);
                return;
            }
        }

        let delete_result = rt.block_on(async { self.client.delete(&full_path).await });

        match delete_result {
            Ok(()) => {
                if dir_inode != 0 {
                    self.unregister_inode(dir_inode);
                }

                reply.ok();
            }
            Err(ClientError::NotFound { .. }) => {
                log::warn!(
                    "⚠️ [RMDIR] Directory già eliminata dal server: {}",
                    full_path
                );
                if dir_inode != 0 {
                    self.unregister_inode(dir_inode);
                }
                reply.ok();
            }
            Err(e) => {
                eprintln!("❌ [RMDIR] Errore eliminazione dal server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn symlink(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        link: &std::path::Path,
        reply: ReplyEntry,
    ) {
        println!(
            "🔗 [SYMLINK] parent: {}, name: {:?}, link: {:?}",
            parent, name, link
        );

        let link_name = match name.to_str() {
            Some(s) => s,
            None => {
                eprintln!("❌ [SYMLINK] Nome symlink non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        let target_path = match link.to_str() {
            Some(s) => s,
            None => {
                eprintln!("❌ [SYMLINK] Path target non valido: {:?}", link);
                reply.error(libc::EINVAL);
                return;
            }
        };

        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                eprintln!(
                    "❌ [SYMLINK] Directory padre con inode {} non trovata",
                    parent
                );
                reply.error(libc::ENOENT);
                return;
            }
        };

        let symlink_path = if parent_path == "/" {
            format!("/{}", link_name)
        } else {
            format!("{}/{}", parent_path, link_name)
        };

        println!("🔗 [SYMLINK] Target risolto: '{}'", symlink_path);

        println!(
            "🔗 [SYMLINK] Creando symlink: '{}' → '{}'",
            link_name, target_path
        );

        if self.path_to_inode.contains_key(&symlink_path) {
            log::warn!("⚠️ [SYMLINK] Symlink già esistente: {}", symlink_path);
            reply.error(libc::EEXIST);
            return;
        }

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        let now_iso = chrono::Utc::now().to_rfc3339();

        let symlink_request = WriteRequest {
            offset: None,
            path: symlink_path.to_string().clone(),
            new_path: None,
            size: target_path.len() as u64,
            atime: now_iso.clone(),
            mtime: now_iso.clone(),
            ctime: now_iso.clone(),
            crtime: now_iso,
            kind: FileKind::Symlink,
            ref_path: Some(target_path.to_string().clone()),
            perm: "777".to_string(),
            mode: Mode::Write,
            data: None,
        };
        println!(
            "🔗 [SYMLINK] Creando symlink: '{}' → '{}'",
            link_name, target_path
        );

        match rt.block_on(async { self.client.write_file(&symlink_request).await }) {
            Ok(()) => {
                let new_inode = self.generate_inode();
                self.register_inode(new_inode, symlink_path.to_string().clone());

                let metadata_result =
                    rt.block_on(async { self.client.get_file_metadata(&symlink_path).await });

                match metadata_result {
                    Ok(metadata) => {
                        let attr = attributes::from_metadata(new_inode, &metadata);
                        let ttl = Duration::from_secs(1);
                        reply.entry(&ttl, &attr, 0);
                    }
                    Err(e) => {
                        reply.error(libc::EIO);
                    }
                }
            }
            Err(e) => {
                eprintln!("❌ [SYMLINK] Errore creazione symlink sul server: {}", e);
                match e {
                    ClientError::NotFound { .. } => reply.error(libc::ENOENT),
                    ClientError::PermissionDenied(_) => reply.error(libc::EPERM),
                    _ => reply.error(libc::EIO),
                }
            }
        }
    }

    fn rename(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        flags: u32,
        reply: fuser::ReplyEmpty,
    ) {
        let old_filename = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };
        let new_filename = match newname.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        if flags != 0 {
            log::warn!("⚠️ [RENAME] Flags non supportati: {}", flags);
        }
        if old_filename == "."
            || old_filename == ".."
            || new_filename == "."
            || new_filename == ".."
        {
            reply.error(libc::EINVAL);
            return;
        }

        let old_parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };
        let new_parent_path = match self.get_path(newparent) {
            Some(p) => p.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let old_path = if old_parent_path == "/" {
            format!("/{}", old_filename)
        } else {
            format!("{}/{}", old_parent_path, old_filename)
        };
        let new_path = if new_parent_path == "/" {
            format!("/{}", new_filename)
        } else {
            format!("{}/{}", new_parent_path, new_filename)
        };

        if old_path == "/" {
            reply.error(libc::EBUSY);
            return;
        }
        if old_path == new_path {
            reply.ok();
            return;
        }

        // Runtime
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(h) => h,
            Err(_) => {
                let r = tokio::runtime::Runtime::new().expect("rt");
                r.handle().clone()
            }
        };

        // Source metadata
        let old_metadata =
            match rt.block_on(async { self.client.get_file_metadata(&old_path).await }) {
                Ok(m) => m,
                Err(ClientError::NotFound { .. }) => {
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(_) => {
                    reply.error(libc::EIO);
                    return;
                }
            };

        let dest_metadata_opt = rt
            .block_on(async { self.client.get_file_metadata(&new_path).await })
            .ok();
        if let Some(dest_md) = &dest_metadata_opt {
            if dest_md.kind != old_metadata.kind {
                reply.error(if old_metadata.kind == FileKind::Directory {
                    libc::ENOTDIR
                } else {
                    libc::EISDIR
                });
                return;
            }
            if dest_md.kind == FileKind::Directory {
                match rt.block_on(async { self.client.list_directory(&new_path).await }) {
                    Ok(listing) => {
                        if !listing.files.is_empty() {
                            reply.error(libc::ENOTEMPTY);
                            return;
                        }
                    }
                    Err(_) => {
                        reply.error(libc::EIO);
                        return;
                    }
                }
            }
        }

        // Rename request
        let now_iso = chrono::Utc::now().to_rfc3339();
        let rename_request = WriteRequest {
            offset: None,
            path: old_path.clone(),
            new_path: Some(new_path.clone()),
            size: old_metadata.size,
            atime: old_metadata.atime.clone(),
            mtime: old_metadata.mtime.clone(),
            ctime: now_iso.clone(),
            crtime: old_metadata.crtime.clone(),
            kind: old_metadata.kind,
            ref_path: None,
            perm: old_metadata.perm.clone(),
            mode: Mode::Write,
            data: None,
        };

        let file_inode = self.path_to_inode.get(&old_path).copied().unwrap_or(0);

        match rt.block_on(async { self.client.write_file(&rename_request).await }) {
            Ok(()) => {
                if let Some(&dest_inode) = self.path_to_inode.get(&new_path) {
                    if dest_inode != file_inode {
                        self.unregister_inode(dest_inode);
                    }
                }

                if file_inode != 0 {
                    self.inode_to_path.remove(&file_inode);
                    self.path_to_inode.remove(&old_path);
                    self.inode_to_path.insert(file_inode, new_path.clone());
                    self.path_to_inode.insert(new_path.clone(), file_inode);

                    // If directory: update sons
                    if old_metadata.kind == FileKind::Directory {
                        let mut updates = Vec::new();
                        for (ino, p) in self.inode_to_path.iter() {
                            if p.starts_with(&old_path) && *ino != file_inode {
                                // build new path
                                let suffix = &p[old_path.len()..];
                                let mut np = new_path.clone();
                                np.push_str(suffix);
                                updates.push((*ino, np));
                            }
                        }
                        for (ino, np) in updates {
                            self.path_to_inode.remove(&self.inode_to_path[&ino]);
                            self.inode_to_path.insert(ino, np.clone());
                            self.path_to_inode.insert(np, ino);
                        }
                    }

                    for of in self.open_files.values_mut() {
                        if of.path == old_path {
                            of.path = new_path.clone();
                        } else if old_metadata.kind == FileKind::Directory
                            && of.path.starts_with(&old_path)
                        {
                            let suffix = &of.path[old_path.len()..];
                            let mut np = new_path.clone();
                            np.push_str(suffix);
                            of.path = np;
                        }
                    }
                } else {
                    let new_inode = self.generate_inode();
                    self.register_inode(new_inode, new_path.clone());
                }

                reply.ok();
            }
            Err(ClientError::NotFound { .. }) => {
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                reply.error(libc::EACCES);
            }
            Err(_) => {
                reply.error(libc::EIO);
            }
        }
    }
    fn link(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        newparent: u64,
        newname: &OsStr,
        reply: ReplyEntry,
    ) {
        println!(
            "🔗 [LINK] ino: {}, newparent: {}, newname: {:?}",
            ino, newparent, newname
        );

        let link_name = match newname.to_str() {
            Some(s) => s,
            None => {
                eprintln!("❌ [LINK] Nome hard link non valido: {:?}", newname);
                reply.error(libc::EINVAL);
                return;
            }
        };

        let source_path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                eprintln!("❌ [LINK] Inode sorgente {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };

        let parent_path = match self.get_path(newparent) {
            Some(p) => p.clone(),
            None => {
                eprintln!(
                    "❌ [LINK] Directory padre con inode {} non trovata",
                    newparent
                );
                reply.error(libc::ENOENT);
                return;
            }
        };

        println!(
            "Parent path: {}, Link name: {}, Source path: {:?}",
            parent_path, link_name, source_path
        );

        let link_path = if parent_path == "/" {
            format!("/{}", link_name)
        } else {
            format!("{}/{}", parent_path, link_name)
        };
        println!("Richiesta con {}", link_path);

        if self.path_to_inode.contains_key(&link_path) {
            log::warn!("⚠️ [LINK] Hard link già esistente: {}", link_path);
            reply.error(libc::EEXIST);
            return;
        }

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };

        let source_metadata =
            match rt.block_on(async { self.client.get_file_metadata(&source_path).await }) {
                Ok(metadata) => metadata,
                Err(ClientError::NotFound { .. }) => {
                    eprintln!("❌ [LINK] File sorgente non trovato: {}", source_path);
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(e) => {
                    eprintln!("❌ [LINK] Errore verifica file sorgente: {}", e);
                    reply.error(libc::EIO);
                    return;
                }
            };

        match source_metadata.kind {
            FileKind::RegularFile => {}
            FileKind::Directory => {
                log::warn!(
                    "⚠️ [LINK] Impossibile creare hard link su directory: {}",
                    source_path
                );
                reply.error(libc::EPERM);
                return;
            }
            FileKind::Symlink => {
                log::warn!(
                    "⚠️ [LINK] Hard link su symlink non supportato: {}",
                    source_path
                );
                reply.error(libc::EPERM);
                return;
            }
            _ => {
                log::warn!(
                    "⚠️ [LINK] Tipo file non supportato per hard link: {:?}",
                    source_metadata.kind
                );
                reply.error(libc::EPERM);
                return;
            }
        }

        let now_iso = chrono::Utc::now().to_rfc3339();

        let link_request = WriteRequest {
            offset: None,
            path: link_path.clone(),
            new_path: None,
            size: source_metadata.size,
            atime: source_metadata.atime.clone(),
            mtime: source_metadata.mtime.clone(),
            ctime: now_iso.clone(),
            crtime: source_metadata.crtime.clone(),
            kind: FileKind::Hardlink,
            ref_path: Some(source_path.clone()),
            perm: source_metadata.perm.clone(),
            mode: Mode::Write,
            data: None,
        };

        match rt.block_on(async { self.client.write_file(&link_request).await }) {
            Ok(()) => {
                self.path_to_inode.insert(link_path.clone(), ino);

                let updated_metadata =
                    match rt.block_on(async { self.client.get_file_metadata(&link_path).await }) {
                        Ok(metadata) => metadata,
                        Err(e) => {
                            eprintln!("❌ [LINK] Errore recupero metadati dopo creazione: {}", e);
                            source_metadata
                        }
                    };

                let attr = attributes::from_metadata(ino, &updated_metadata);
                let ttl = Duration::from_secs(1);
                reply.entry(&ttl, &attr, 0);
            }
            Err(ClientError::NotFound { .. }) => {
                eprintln!(
                    "❌ [LINK] File sorgente non trovato durante creazione: {}",
                    source_path
                );
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                eprintln!("❌ [LINK] Permesso negato per creazione hard link");
                reply.error(libc::EPERM);
            }
            Err(e) => {
                eprintln!("❌ [LINK] Errore creazione hard link sul server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                eprintln!("❌ [OPEN] Inode {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(h) => h,
            Err(_) => {
                let r = tokio::runtime::Runtime::new().expect("rt");
                r.handle().clone()
            }
        };

        let access_mode = flags & libc::O_ACCMODE;
        let create_flag = (flags & libc::O_CREAT) != 0;
        let excl_flag = (flags & libc::O_EXCL) != 0;
        let trunc_flag = (flags & libc::O_TRUNC) != 0;

        let metadata_result = rt.block_on(async { self.client.get_file_metadata(&path).await });

        let metadata = match metadata_result {
            Ok(m) => m,
            Err(ClientError::NotFound { .. }) => {
                if create_flag {
                    let now_iso = chrono::Utc::now().to_rfc3339();
                    let create_req = WriteRequest {
                        offset: None,
                        path: path.clone(),
                        new_path: None,
                        size: 0,
                        atime: now_iso.clone(),
                        mtime: now_iso.clone(),
                        ctime: now_iso.clone(),
                        crtime: now_iso,
                        kind: FileKind::RegularFile,
                        ref_path: None,
                        perm: "644".to_string(), // default
                        mode: Mode::Write,
                        data: Some(Vec::new()),
                    };
                    if let Err(e) = rt.block_on(async { self.client.write_file(&create_req).await })
                    {
                        eprintln!("❌ [OPEN] Creazione fallita {}: {}", path, e);
                        reply.error(libc::EIO);
                        return;
                    }

                    match rt.block_on(async { self.client.get_file_metadata(&path).await }) {
                        Ok(m2) => m2,
                        Err(_) => {
                            reply.error(libc::EIO);
                            return;
                        }
                    }
                } else {
                    reply.error(libc::ENOENT);
                    return;
                }
            }
            Err(e) => {
                eprintln!("❌ [OPEN] Errore metadati {}: {}", path, e);
                reply.error(libc::EIO);
                return;
            }
        };

        let perms = parse_permissions(&metadata.perm);
        let owner_bits = perms.owner;

        match access_mode {
            libc::O_RDONLY => {
                if (owner_bits & 0o4) == 0 {
                    reply.error(libc::EACCES);
                    return;
                }
            }
            libc::O_WRONLY => {
                if (owner_bits & 0o2) == 0 {
                    reply.error(libc::EACCES);
                    return;
                }
            }
            libc::O_RDWR => {
                if (owner_bits & 0o6) != 0o6 {
                    reply.error(libc::EACCES);
                    return;
                }
            }
            _ => {
                reply.error(libc::EINVAL);
                return;
            }
        }

        if create_flag && excl_flag {
            if metadata.size >= 0 {
                reply.error(libc::EEXIST);
                return;
            }
        }

        if trunc_flag && access_mode != libc::O_RDONLY {
            let now_iso = chrono::Utc::now().to_rfc3339();
            let trunc_req = WriteRequest {
                offset: None,
                path: path.clone(),
                new_path: None,
                size: 0,
                atime: metadata.atime.clone(),
                mtime: now_iso.clone(),
                ctime: now_iso,
                crtime: metadata.crtime.clone(),
                kind: metadata.kind,
                ref_path: metadata.ref_path.clone(),
                perm: metadata.perm.clone(),
                mode: Mode::Truncate,
                data: None,
            };
            if let Err(e) = rt.block_on(async { self.client.write_file(&trunc_req).await }) {
                eprintln!("❌ [OPEN] Truncate fallito {}: {}", path, e);
                reply.error(libc::EIO);
                return;
            }
        }

        let fh = self.next_fh;
        self.next_fh += 1;
        self.open_files.insert(
            fh,
            OpenFile {
                path: path.clone(),
                flags,
                write_buffer: Vec::new(),
                buffer_dirty: false,
            },
        );

        reply.opened(fh, 0);
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        flags: i32,
        lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        if offset < 0 {
            eprintln!("❌ [READ] Offset negativo: {}", offset);
            reply.error(libc::EINVAL);
            return;
        }

        if size == 0 {
            reply.data(&[]);
            return;
        }

        let offset_u64 = offset as u64;
        let size_usize = size as usize;

        let open_file = match self.open_files.get(&fh) {
            Some(file) => file,
            None => {
                eprintln!("❌ [READ] File handle {} non trovato", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        let path = open_file.path.clone();

        let access_mode = open_file.flags & libc::O_ACCMODE;
        if access_mode == libc::O_WRONLY {
            log::warn!(
                "⚠️ [READ] Tentativo di lettura su file aperto in WRITE-ONLY: {}",
                path
            );
            reply.error(libc::EBADF);
            return;
        }

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        let metadata = match rt.block_on(async { self.client.get_file_metadata(&path).await }) {
            Ok(metadata) => metadata,
            Err(ClientError::NotFound { .. }) => {
                eprintln!("❌ [READ] File non trovato sul server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                eprintln!("❌ [READ] Errore verifica metadati: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        match metadata.kind {
            FileKind::RegularFile | FileKind::Symlink => {}
            FileKind::Directory => {
                log::warn!("⚠️ [READ] Tentativo di read su directory: {}", path);
                reply.error(libc::EISDIR);
                return;
            }
            _ => {
                log::warn!(
                    "⚠️ [READ] Tipo file non supportato per read: {:?}",
                    metadata.kind
                );
                reply.error(libc::EPERM);
                return;
            }
        }

        let file_size = metadata.size;

        if offset_u64 >= file_size {
            reply.data(&[]);
            return;
        }

        let bytes_available = file_size - offset_u64;
        let bytes_to_read = std::cmp::min(size_usize as u64, bytes_available);

        if bytes_to_read == 0 {
            reply.data(&[]);
            return;
        }

        let read_result = rt.block_on(async {
            self.client
                .read_file(&path, Some(offset_u64), Some(bytes_to_read))
                .await
        });

        match read_result {
            Ok(read_response) => {
                let data = read_response.data;

                if data.len() > (bytes_to_read as usize) {
                    log::warn!(
                        "⚠️ [READ] Server ha restituito più dati del richiesto: {} > {}, troncando",
                        data.len(),
                        bytes_to_read
                    );
                    reply.data(&data[..bytes_to_read as usize]);
                } else if data.is_empty() && bytes_to_read > 0 {
                    reply.data(&[]);
                } else {
                    reply.data(&data);
                }
            }
            Err(ClientError::NotFound { .. }) => {
                eprintln!("❌ [READ] File eliminato durante la lettura: {}", path);
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                eprintln!("❌ [READ] Permesso di lettura negato: {}", path);
                reply.error(libc::EACCES);
            }
            Err(e) => {
                eprintln!("❌ [READ] Errore lettura dal server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }
    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        write_flags: u32,
        flags: i32,
        lock_owner: Option<u64>,
        reply: fuser::ReplyWrite,
    ) {
        if offset < 0 {
            eprintln!("❌ [WRITE] Offset negativo: {}", offset);
            reply.error(libc::EINVAL);
            return;
        }

        if data.is_empty() {
            reply.written(0);
            return;
        }

        let offset_u64 = offset as u64;
        let data_len = data.len();

        let open_file = match self.open_files.get(&fh) {
            Some(file) => file,
            None => {
                eprintln!("❌ [WRITE] File handle {} non trovato", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        let path = open_file.path.clone();
        let open_flags = open_file.flags;

        let access_mode = open_flags & libc::O_ACCMODE;
        if access_mode == libc::O_RDONLY {
            log::warn!(
                "⚠️ [WRITE] Tentativo di scrittura su file aperto in READ-ONLY: {}",
                path
            );
            reply.error(libc::EBADF);
            return;
        }

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };

        let metadata = match rt.block_on(async { self.client.get_file_metadata(&path).await }) {
            Ok(metadata) => metadata,
            Err(ClientError::NotFound { .. }) => {
                eprintln!("❌ [WRITE] File non trovato sul server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                eprintln!("❌ [WRITE] Errore verifica metadati: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        match metadata.kind {
            FileKind::RegularFile | FileKind::Symlink => {}
            FileKind::Directory => {
                log::warn!("⚠️ [WRITE] Tentativo di write su directory: {}", path);
                reply.error(libc::EISDIR);
                return;
            }
            _ => {
                log::warn!(
                    "⚠️ [WRITE] Tipo file non supportato per write: {:?}",
                    metadata.kind
                );
                reply.error(libc::EPERM);
                return;
            }
        }

        let current_file_size = metadata.size;

        let effective_offset = if (open_flags & libc::O_APPEND) != 0 {
            current_file_size
        } else {
            offset_u64
        };

        let file1 = self.open_files.get_mut(&fh).unwrap().write_buffer.len();
        let (write_mode, final_data) = if effective_offset == current_file_size + (file1 as u64) {
            (Mode::Append, data.to_vec())
        } else if effective_offset == 0 && (data_len as u64) >= current_file_size {
            (Mode::Write, data.to_vec())
        } else {
            (Mode::Write, data.to_vec())
        };

        let open_file = self.open_files.get_mut(&fh);
        let file = open_file.unwrap();

        if write_mode == Mode::Append && file.write_buffer.len() < STREAM_WRITE {
            let open_file = self.open_files.get_mut(&fh);
            if let Some(file) = open_file {
                file.write_buffer.extend_from_slice(&final_data);

                file.buffer_dirty = true;
            }
            reply.written(final_data.len() as u32);
            return;
        }

        let now_iso1 = chrono::Utc::now().to_rfc3339();

        let open_file = self.open_files.get_mut(&fh);
        let file = if open_file.is_some() {
            open_file.unwrap()
        } else {
            eprintln!("❌ [WRITE] File handle {} non trovato", fh);
            reply.error(libc::EBADF);
            return;
        };

        if !file.write_buffer.is_empty() {
            let write_request1 = WriteRequest {
                offset: None,
                path: path.clone(),
                new_path: None,
                size: file.write_buffer.len() as u64,
                atime: metadata.atime.clone(),
                mtime: now_iso1.clone(),
                ctime: now_iso1,
                crtime: metadata.crtime.clone(),
                kind: metadata.kind.clone(),
                ref_path: metadata.ref_path.clone(),
                perm: metadata.perm.clone(),
                mode: Mode::Append,
                data: Some(file.write_buffer.clone()),
            };

            let write_result1 =
                rt.block_on(async { self.client.write_file(&write_request1).await });

            if let Err(e) = write_result1 {
                eprintln!("❌ [WRITE] Errore scrittura file: {}", e);
                reply.error(libc::EIO);
                return;
            }

            file.buffer_dirty = false;
            file.write_buffer.clear();
        }

        let now_iso = chrono::Utc::now().to_rfc3339();
        let write_request = WriteRequest {
            offset: if matches!(write_mode, Mode::WriteAt) {
                Some(offset as u64)
            } else {
                None
            },
            path: path.clone(),
            new_path: None,
            size: final_data.len() as u64,
            atime: metadata.atime.clone(),
            mtime: now_iso.clone(),
            ctime: now_iso,
            crtime: metadata.crtime.clone(),
            kind: metadata.kind,
            ref_path: metadata.ref_path.clone(),
            perm: metadata.perm.clone(),
            mode: write_mode,
            data: Some(final_data),
        };

        let write_result = rt.block_on(async { self.client.write_file(&write_request).await });

        match write_result {
            Ok(()) => {
                match write_request.mode {
                    Mode::Append => {}
                    Mode::Write => {
                        if effective_offset != 0 || (data_len as u64) < current_file_size {
                        } else {
                        }
                    }
                    _ => {}
                }

                reply.written(data_len as u32);
            }
            Err(ClientError::NotFound { .. }) => {
                eprintln!("❌ [WRITE] File eliminato durante la scrittura: {}", path);
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                eprintln!("❌ [WRITE] Permesso di scrittura negato: {}", path);
                reply.error(libc::EACCES);
            }
            Err(ClientError::Server { status: 413, .. }) => {
                eprintln!("❌ [WRITE] File troppo grande: {}", path);
                reply.error(libc::EFBIG);
            }
            Err(ClientError::Server { status: 507, .. }) => {
                eprintln!("❌ [WRITE] Spazio insufficiente sul server: {}", path);
                reply.error(libc::ENOSPC);
            }
            Err(e) => {
                eprintln!("❌ [WRITE] Errore scrittura sul server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }
    fn flush(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _lock_owner: u64,
        reply: fuser::ReplyEmpty,
    ) {
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };

        let open_file = match self.open_files.get(&fh) {
            Some(file) => file,
            None => {
                eprintln!("❌ [WRITE] File handle {} non trovato", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        if open_file.write_buffer.is_empty() {
            reply.ok();
            return;
        }

        let metadata =
            match rt.block_on(async { self.client.get_file_metadata(&open_file.path).await }) {
                Ok(metadata) => metadata,
                Err(ClientError::NotFound { .. }) => {
                    eprintln!("❌ [WRITE] File non trovato sul server: {}", open_file.path);
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(e) => {
                    eprintln!("❌ [WRITE] Errore verifica metadati: {}", e);
                    reply.error(libc::EIO);
                    return;
                }
            };

        if open_file.buffer_dirty {
            let now_iso1 = chrono::Utc::now().to_rfc3339();

            let open_file = self.open_files.get_mut(&fh);
            let file = if open_file.is_some() {
                open_file.unwrap()
            } else {
                eprintln!("❌ [WRITE] File handle {} non trovato", fh);
                reply.error(libc::EBADF);
                return;
            };

            let file = if let Some(f) = self.open_files.get_mut(&fh) {
                f
            } else {
                eprintln!("❌ [WRITE] File handle {} non trovato", fh);
                reply.error(libc::EBADF);
                return;
            };

            let write_request1 = WriteRequest {
                offset: None,
                path: file.path.clone(),
                new_path: None,
                size: file.write_buffer.len() as u64,
                atime: metadata.atime.clone(),
                mtime: now_iso1.clone(),
                ctime: now_iso1,
                crtime: metadata.crtime.clone(),
                kind: metadata.kind.clone(),
                ref_path: metadata.ref_path.clone(),
                perm: metadata.perm.clone(),
                mode: Mode::Append,
                data: Some(file.write_buffer.clone()),
            };

            let write_result1 =
                rt.block_on(async { self.client.write_file(&write_request1).await });

            if let Err(e) = write_result1 {
                eprintln!("❌ [WRITE] Errore scrittura file: {}", e);
                reply.error(libc::EIO);
                return;
            }
        }

        reply.ok()
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        flags: i32,
        lock_owner: Option<u64>,
        flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        let open_file = match self.open_files.get(&fh) {
            Some(file) => file,
            None => {
                log::warn!(
                    "⚠️ [RELEASE] File handle {} già rilasciato o inesistente",
                    fh
                );
                reply.ok();
                return;
            }
        };

        let path = open_file.path.clone();

        if flush {}

        if let Some(removed_file) = self.open_files.remove(&fh) {}

        reply.ok();
    }

    fn fsync(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        datasync: bool,
        reply: fuser::ReplyEmpty,
    ) {
        let open_file = match self.open_files.get(&fh) {
            Some(file) => file,
            None => {
                eprintln!("❌ [FSYNC] File handle {} non trovato", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        let path = open_file.path.clone();

        let access_mode = open_file.flags & libc::O_ACCMODE;
        if access_mode == libc::O_RDONLY {
            log::warn!("⚠️ [FSYNC] File aperto in read-only: {}", path);
            reply.error(libc::EBADF);
            return;
        }

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        match rt.block_on(async { self.client.get_file_metadata(&path).await }) {
            Ok(_) => {
                reply.ok();
            }
            Err(ClientError::NotFound { .. }) => {
                eprintln!("❌ [FSYNC] File eliminato durante fsync: {}", path);
                reply.error(libc::ENOENT);
            }
            Err(e) => {
                eprintln!("❌ [FSYNC] Errore verifica server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn opendir(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                eprintln!("❌ [OPENDIR] Inode {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        let metadata = match rt.block_on(async { self.client.get_file_metadata(&path).await }) {
            Ok(metadata) => metadata,
            Err(ClientError::NotFound { .. }) => {
                eprintln!("❌ [OPENDIR] Directory non trovata sul server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                eprintln!("❌ [OPENDIR] Errore verifica metadati: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        if metadata.kind != FileKind::Directory {
            log::warn!(
                "⚠️ [OPENDIR] '{}' non è una directory: {:?}",
                path,
                metadata.kind
            );
            reply.error(libc::ENOTDIR);
            return;
        }

        match rt.block_on(async { self.client.list_directory(&path).await }) {
            Ok(_) => {}
            Err(ClientError::PermissionDenied(_)) => {
                eprintln!("❌ [OPENDIR] Permesso di lettura negato: {}", path);
                reply.error(libc::EACCES);
                return;
            }
            Err(e) => {
                eprintln!("❌ [OPENDIR] Errore accesso directory: {}", e);
                reply.error(libc::EIO);
                return;
            }
        }

        let dh = self.next_fh;
        self.next_fh += 1;

        self.open_dirs.insert(
            dh,
            OpenDir {
                path: path.clone(),
                flags,
            },
        );

        reply.opened(dh, 0);
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let open_dir = match self.open_dirs.get(&fh) {
            Some(dir) => dir,
            None => {
                eprintln!("❌ [READDIR] Directory handle {} non trovato", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        let path = open_dir.path.clone();

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        let listing_result = rt.block_on(async { self.client.list_directory(&path).await });

        let listing = match listing_result {
            Ok(listing) => listing,
            Err(ClientError::NotFound { .. }) => {
                eprintln!("❌ [READDIR] Directory non trovata sul server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(ClientError::PermissionDenied(_)) => {
                eprintln!("❌ [READDIR] Permesso di lettura negato: {}", path);
                reply.error(libc::EACCES);
                return;
            }
            Err(e) => {
                eprintln!("❌ [READDIR] Errore lettura directory: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        let mut entries = Vec::new();

        entries.push((ino, FileType::Directory, ".".to_string()));

        let parent_ino = if path == "/" {
            1 // Root directory
        } else {
            let parent_path = std::path::Path::new(&path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "/".to_string());

            self.path_to_inode.get(&parent_path).copied().unwrap_or(1)
        };
        entries.push((parent_ino, FileType::Directory, "..".to_string()));

        for file_entry in listing.files {
            let entry_path = if path == "/" {
                format!("/{}", file_entry.name)
            } else {
                format!("{}/{}", path, file_entry.name)
            };

            let entry_ino = if let Some(&existing_ino) = self.path_to_inode.get(&entry_path) {
                existing_ino
            } else {
                let new_ino = self.generate_inode();
                self.register_inode(new_ino, entry_path.clone());
                new_ino
            };

            let file_type = match file_entry.kind {
                FileKind::Directory => FileType::Directory,
                FileKind::RegularFile => FileType::RegularFile,
                FileKind::Symlink => FileType::Symlink,
                FileKind::Hardlink => FileType::RegularFile, // Hard link appears as normal files
                _ => {
                    log::warn!(
                        "⚠️ [READDIR] Tipo file non supportato: {:?}",
                        file_entry.kind
                    );
                    FileType::RegularFile // Fallback
                }
            };

            entries.push((entry_ino, file_type, file_entry.name));
        }

        let start_index = if offset == 0 { 0 } else { offset as usize };

        if start_index >= entries.len() {
            reply.ok();
            return;
        }

        let mut current_offset = start_index;
        for (entry_ino, file_type, name) in entries.into_iter().skip(start_index) {
            current_offset += 1;

            let buffer_full = reply.add(entry_ino, current_offset as i64, file_type, name);

            if buffer_full {
                break;
            }
        }

        reply.ok();
    }

    fn releasedir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        flags: i32,
        reply: fuser::ReplyEmpty,
    ) {
        let open_dir = match self.open_dirs.get(&fh) {
            Some(dir) => dir,
            None => {
                log::warn!(
                    "⚠️ [RELEASEDIR] Directory handle {} già rilasciato o inesistente",
                    fh
                );
                reply.ok();
                return;
            }
        };

        let path = open_dir.path.clone();

        if let Some(removed_dir) = self.open_dirs.remove(&fh) {}

        reply.ok();
    }

    fn fsyncdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        datasync: bool,
        reply: fuser::ReplyEmpty,
    ) {
        let open_dir = match self.open_dirs.get(&fh) {
            Some(dir) => dir,
            None => {
                eprintln!("❌ [FSYNCDIR] Directory handle {} non trovato", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        let path = open_dir.path.clone();

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        let metadata = match rt.block_on(async { self.client.get_file_metadata(&path).await }) {
            Ok(metadata) => metadata,
            Err(ClientError::NotFound { .. }) => {
                eprintln!("❌ [FSYNCDIR] Directory non trovata: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                eprintln!("❌ [FSYNCDIR] Errore verifica metadati: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        if metadata.kind != FileKind::Directory {
            eprintln!("❌ [FSYNCDIR] '{}' non è una directory", path);
            reply.error(libc::ENOTDIR);
            return;
        }

        match rt.block_on(async { self.client.list_directory(&path).await }) {
            Ok(_) => {
                reply.ok();
            }
            Err(ClientError::NotFound { .. }) => {
                eprintln!(
                    "❌ [FSYNCDIR] Directory eliminata durante fsyncdir: {}",
                    path
                );
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                eprintln!("❌ [FSYNCDIR] Permesso negato per directory: {}", path);
                reply.error(libc::EACCES);
            }
            Err(e) => {
                eprintln!("❌ [FSYNCDIR] Errore verifica directory: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: fuser::ReplyStatfs) {
        let total_blocks = 268435456u64; // 1TB / 4KB
        let free_blocks = 134217728u64; // 512GB / 4KB
        let available_blocks = free_blocks;
        let total_inodes = 1000000u64;
        let free_inodes = total_inodes - (self.path_to_inode.len() as u64);

        reply.statfs(
            total_blocks,
            free_blocks,
            available_blocks,
            free_inodes,
            total_inodes,
            4096,
            255,
            0,
        );
    }

    fn setxattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        name: &OsStr,
        _value: &[u8],
        flags: i32,
        position: u32,
        reply: fuser::ReplyEmpty,
    ) {
        reply.error(libc::ENOSYS);
    }

    fn getxattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        name: &OsStr,
        size: u32,
        reply: fuser::ReplyXattr,
    ) {
        reply.error(libc::ENOSYS);
    }

    fn listxattr(&mut self, _req: &Request<'_>, ino: u64, size: u32, reply: fuser::ReplyXattr) {
        reply.error(libc::ENOSYS);
    }

    fn removexattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        name: &OsStr,
        reply: fuser::ReplyEmpty,
    ) {
        reply.error(libc::ENOSYS);
    }

    fn access(&mut self, _req: &Request<'_>, ino: u64, mask: i32, reply: fuser::ReplyEmpty) {
        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                eprintln!("❌ [ACCESS] Inode {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };

        let check_exist =
            mask == libc::F_OK || (mask & (libc::R_OK | libc::W_OK | libc::X_OK)) != 0;
        let check_read = (mask & libc::R_OK) != 0;
        let check_write = (mask & libc::W_OK) != 0;
        let check_exec = (mask & libc::X_OK) != 0;

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        let metadata = match rt.block_on(async { self.client.get_file_metadata(&path).await }) {
            Ok(metadata) => metadata,
            Err(ClientError::NotFound { .. }) => {
                eprintln!("❌ [ACCESS] File non trovato: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(ClientError::PermissionDenied(_)) => {
                eprintln!("❌ [ACCESS] Permesso negato per metadati: {}", path);
                reply.error(libc::EACCES);
                return;
            }
            Err(e) => {
                eprintln!("❌ [ACCESS] Errore verifica esistenza: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        if mask == libc::F_OK {
            reply.ok();
            return;
        }

        let perms = parse_permissions(&metadata.perm);

        let effective_perms = perms.owner;

        let mut access_denied = false;

        if check_read && (effective_perms & 0o4) == 0 {
            reply.error(libc::EACCES);
            return;
        }
        if check_write && (effective_perms & 0o2) == 0 {
            reply.error(libc::EACCES);
            return;
        }
        if check_exec && (effective_perms & 0o1) == 0 && metadata.kind != FileKind::Directory {
            reply.error(libc::EACCES);
            return;
        }

        if check_exec && metadata.kind == FileKind::Directory {
        } else if check_exec && metadata.kind != FileKind::RegularFile {
            log::warn!("⚠️ [ACCESS] Tipo file non eseguibile: {:?}", metadata.kind);
            access_denied = true;
        }

        if access_denied {
            reply.error(libc::EACCES);
        } else {
            reply.ok();
        }
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
        reply: fuser::ReplyCreate,
    ) {
        let filename = match name.to_str() {
            Some(s) => s,
            None => {
                eprintln!("❌ [CREATE] Nome file non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                eprintln!(
                    "❌ [CREATE] Directory padre con inode {} non trovata",
                    parent
                );
                reply.error(libc::ENOENT);
                return;
            }
        };

        let full_path = if parent_path == "/" {
            format!("/{}", filename)
        } else {
            format!("{}/{}", parent_path, filename)
        };

        if self.path_to_inode.contains_key(&full_path) {
            log::warn!("⚠️ [CREATE] File già esistente: {}", full_path);
            reply.error(libc::EEXIST);
            return;
        }

        let effective_permissions = mode & 0o777 & !(umask & 0o777);
        let effective_permissions_str = format!("{:o}", effective_permissions);

        let access_mode = flags & libc::O_ACCMODE;
        let open_flags = flags & !libc::O_ACCMODE;

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        let now_iso = chrono::Utc::now().to_rfc3339();

        let create_request = WriteRequest {
            offset: None,
            path: full_path.clone(),
            new_path: None,
            size: 0,
            atime: now_iso.clone(),
            mtime: now_iso.clone(),
            ctime: now_iso.clone(),
            crtime: now_iso,
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: effective_permissions_str,
            mode: Mode::Write,
            data: Some(Vec::new()),
        };

        if (open_flags & libc::O_TRUNC) != 0 {}

        match rt.block_on(async { self.client.write_file(&create_request).await }) {
            Ok(()) => {
                let new_inode = self.generate_inode();
                self.register_inode(new_inode, full_path.clone());

                let fh = self.next_fh;
                self.next_fh += 1;

                self.open_files.insert(
                    fh,
                    OpenFile {
                        path: full_path.clone(),
                        flags,
                        write_buffer: Vec::new(),
                        buffer_dirty: false,
                    },
                );

                let metadata_result =
                    rt.block_on(async { self.client.get_file_metadata(&full_path).await });

                match metadata_result {
                    Ok(metadata) => {
                        let attr = attributes::from_metadata(new_inode, &metadata);
                        let ttl = Duration::from_secs(1);

                        reply.created(&ttl, &attr, 0, fh, FOPEN_DIRECT_IO);
                    }
                    Err(e) => {
                        eprintln!("❌ [CREATE] Errore recupero metadati: {}", e);
                        let attr = new_file_attr(new_inode, 0, effective_permissions);
                        let ttl = Duration::from_secs(1);
                        reply.created(&ttl, &attr, 0, fh, 0);
                    }
                }
            }
            Err(e) => {
                eprintln!("❌ [CREATE] Errore creazione file sul server: {}", e);
                match e {
                    ClientError::NotFound { .. } => reply.error(libc::ENOENT),
                    ClientError::PermissionDenied(_) => reply.error(libc::EPERM),
                    _ => reply.error(libc::EIO),
                }
            }
        }
    }
    fn getlk(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        lock_owner: u64,
        start: u64,
        end: u64,
        typ: i32,
        pid: u32,
        reply: fuser::ReplyLock,
    ) {
        if !self.open_files.contains_key(&fh) {
            reply.error(libc::EBADF);
            return;
        }

        if let Some(locks) = self.file_locks.get(&ino) {
            for existing_lock in locks {
                if ranges_overlap(start, end, existing_lock.start, existing_lock.end) {
                    if locks_conflict(typ, existing_lock.typ) {
                        reply.locked(
                            existing_lock.start,
                            existing_lock.end,
                            existing_lock.typ,
                            existing_lock.pid,
                        );
                        return;
                    }
                }
            }
        }

        reply.locked(0, 0, libc::F_UNLCK, 0);
    }

    fn setlk(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        lock_owner: u64,
        start: u64,
        end: u64,
        typ: i32,
        pid: u32,
        sleep: bool,
        reply: fuser::ReplyEmpty,
    ) {
        if !self.open_files.contains_key(&fh) {
            reply.error(libc::EBADF);
            return;
        }

        match typ {
            libc::F_UNLCK => {
                if let Some(locks) = self.file_locks.get_mut(&ino) {
                    locks.retain(|lock| {
                        !(lock.lock_owner == lock_owner
                            && ranges_overlap(start, end, lock.start, lock.end))
                    });
                }

                reply.ok();
            }
            libc::F_RDLCK | libc::F_WRLCK => {
                if let Some(locks) = self.file_locks.get(&ino) {
                    for existing_lock in locks {
                        if ranges_overlap(start, end, existing_lock.start, existing_lock.end)
                            && locks_conflict(typ, existing_lock.typ)
                            && existing_lock.lock_owner != lock_owner
                        {
                            if sleep {
                                log::warn!(
                                    "⚠️ [SETLK] Lock bloccante non implementato completamente"
                                );
                                reply.error(libc::ENOSYS);
                            } else {
                                eprintln!("❌ [SETLK] Lock conflict, non-blocking");
                                reply.error(libc::EAGAIN);
                            }
                            return;
                        }
                    }
                }

                let new_lock = FileLock {
                    typ,
                    start,
                    end,
                    pid,
                    lock_owner,
                };

                self.file_locks
                    .entry(ino)
                    .or_insert_with(Vec::new)
                    .push(new_lock);

                reply.ok();
            }
            _ => {
                reply.error(libc::EINVAL);
            }
        }
    }

    fn bmap(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        blocksize: u32,
        idx: u64,
        reply: fuser::ReplyBmap,
    ) {
        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                eprintln!("❌ [BMAP] Inode {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        let metadata = match rt.block_on(async { self.client.get_file_metadata(&path).await }) {
            Ok(metadata) => metadata,
            Err(ClientError::NotFound { .. }) => {
                eprintln!("❌ [BMAP] File non trovato: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                eprintln!("❌ [BMAP] Errore metadati: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        if metadata.kind != FileKind::RegularFile {
            log::warn!("⚠️ [BMAP] bmap solo supportato per file regolari");
            reply.error(libc::EPERM);
            return;
        }

        let file_size = metadata.size;
        let blocks_in_file = (file_size + (blocksize as u64) - 1) / (blocksize as u64);

        if idx >= blocks_in_file {
            reply.error(libc::ENXIO);
            return;
        }

        let simulated_physical_block = ino * 1000 + idx;

        reply.bmap(simulated_physical_block);
    }

    fn ioctl(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        flags: u32,
        cmd: u32,
        in_data: &[u8],
        out_size: u32,
        reply: fuser::ReplyIoctl,
    ) {
        reply.error(libc::ENOSYS);
    }

    fn fallocate(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        length: i64,
        mode: i32,
        reply: fuser::ReplyEmpty,
    ) {
        reply.error(libc::ENOSYS);
    }

    fn lseek(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        whence: i32,
        reply: fuser::ReplyLseek,
    ) {
        reply.error(libc::ENOSYS);
    }

    fn copy_file_range(
        &mut self,
        _req: &Request<'_>,
        ino_in: u64,
        fh_in: u64,
        offset_in: i64,
        ino_out: u64,
        fh_out: u64,
        offset_out: i64,
        len: u64,
        flags: u32,
        reply: fuser::ReplyWrite,
    ) {
        if offset_in < 0 || offset_out < 0 {
            eprintln!("❌ [COPY_FILE_RANGE] Offset negativi non supportati");
            reply.error(libc::EINVAL);
            return;
        }

        if len == 0 {
            reply.written(0);
            return;
        }

        let source_file = match self.open_files.get(&fh_in) {
            Some(file) => file,
            None => {
                eprintln!(
                    "❌ [COPY_FILE_RANGE] File handle sorgente {} non trovato",
                    fh_in
                );
                reply.error(libc::EBADF);
                return;
            }
        };

        let dest_file = match self.open_files.get(&fh_out) {
            Some(file) => file,
            None => {
                eprintln!(
                    "❌ [COPY_FILE_RANGE] File handle destinazione {} non trovato",
                    fh_out
                );
                reply.error(libc::EBADF);
                return;
            }
        };

        let source_access = source_file.flags & libc::O_ACCMODE;
        let dest_access = dest_file.flags & libc::O_ACCMODE;

        if source_access == libc::O_WRONLY {
            eprintln!("❌ [COPY_FILE_RANGE] File sorgente non leggibile");
            reply.error(libc::EBADF);
            return;
        }

        if dest_access == libc::O_RDONLY {
            eprintln!("❌ [COPY_FILE_RANGE] File destinazione non scrivibile");
            reply.error(libc::EBADF);
            return;
        }

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        let chunk_size = std::cmp::min(len, 1024 * 1024); // Max 1MB per chunk

        let source_data = match rt.block_on(async {
            self.client
                .read_file(&source_file.path, Some(offset_in as u64), Some(chunk_size))
                .await
        }) {
            Ok(data) => data.data,
            Err(e) => {
                eprintln!("❌ [COPY_FILE_RANGE] Errore lettura sorgente: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        let bytes_read = source_data.len() as u64;
        let bytes_to_copy = std::cmp::min(len, bytes_read);

        if bytes_to_copy == 0 {
            reply.written(0);
            return;
        }

        let dest_metadata =
            match rt.block_on(async { self.client.get_file_metadata(&dest_file.path).await }) {
                Ok(metadata) => metadata,
                Err(e) => {
                    eprintln!("❌ [COPY_FILE_RANGE] Errore metadati destinazione: {}", e);
                    reply.error(libc::EIO);
                    return;
                }
            };

        let now_iso = chrono::Utc::now().to_rfc3339();
        let write_request = WriteRequest {
            offset: None,
            path: dest_file.path.clone(),
            new_path: None,
            size: std::cmp::max(dest_metadata.size, (offset_out as u64) + bytes_to_copy),
            atime: dest_metadata.atime.clone(),
            mtime: now_iso.clone(),
            ctime: now_iso,
            crtime: dest_metadata.crtime.clone(),
            kind: dest_metadata.kind,
            ref_path: None,
            perm: dest_metadata.perm.clone(),
            mode: Mode::Write,
            data: Some(source_data[..bytes_to_copy as usize].to_vec()),
        };

        match rt.block_on(async { self.client.write_file(&write_request).await }) {
            Ok(()) => {
                reply.written(bytes_to_copy as u32);
            }
            Err(e) => {
                eprintln!("❌ [COPY_FILE_RANGE] Errore scrittura: {}", e);
                reply.error(libc::EIO);
            }
        }
    }
}
