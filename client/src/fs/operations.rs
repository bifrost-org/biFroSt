use crate::api::client::{ ClientError, RemoteClient };
use crate::api::models::*;
use crate::fs::attributes::{ self, new_directory_attr, new_file_attr };
use fuser::{
    FileType,
    Filesystem,
    ReplyAttr,
    ReplyData,
    ReplyDirectory,
    ReplyEntry,
    ReplyOpen,
    Request,
};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::time::{ Duration, SystemTime };

#[allow(unused_macros)]
macro_rules! debug_println {
    ($($arg:tt)*) => {
        println!($($arg)*);
    };
}

pub struct RemoteFileSystem {
    inode_to_path: HashMap<u64, String>,
    path_to_inode: HashMap<String, u64>,
    next_inode: u64,
    client: RemoteClient,
    open_files: HashMap<u64, OpenFile>,
    next_fh: u64,
    open_dirs: HashMap<u64, OpenDir>,

}


struct OpenDir {
    path: String,
}

struct OpenFile {
    path: String,
    flags: i32,
    write_buffer: Vec<u8>,
    buffer_dirty: bool,
}

struct Permissions {
    owner: u32,
}

fn parse_permissions(perm_str: &str) -> Permissions {
    match u32::from_str_radix(perm_str, 8) {
        Ok(perms) =>
            Permissions {
                owner: (perms >> 6) & 0o7,
            },
        Err(_) =>
            Permissions {
                owner: 0o6,
            },
    }
}


impl RemoteFileSystem {
    pub fn new(client: RemoteClient) -> Self {
        let mut fs = Self {
            inode_to_path: HashMap::new(),
            path_to_inode: HashMap::new(),
            next_inode: 2,
            client,
            open_files: HashMap::new(),
            next_fh: 1,
            open_dirs: HashMap::new(),
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
            let ttl = Duration::from_secs(300);

                reply.attr(&ttl, &attr);
            }
            Err(_) => {
                reply.error(libc::EIO);
            }
        }
    }
}

impl Filesystem for RemoteFileSystem {
    fn init(
        &mut self,
        _req: &Request<'_>,
        _config: &mut fuser::KernelConfig
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
        match
            rt.block_on(async {
                match self.client.get_file_metadata("/").await {
                    Ok(_) => Ok(()),
                    Err(ClientError::NotFound { .. }) => Ok(()),
                    Err(e) => Err(e),
                }
            })
        {
            Ok(_) => {
                debug_println!("RemoteFileSystem: connection to server verified");

                let _ = rt.block_on(async {
                    if let Ok(listing) = self.client.list_directory("/").await {
                        debug_println!(
                            "RemoteFileSystem: root directory preloaded with {} elements",
                            listing.files.len()
                        );

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
            Err(e) => {
                debug_println!("RemoteFileSystem: error connecting to server: {}", e);
                Err(libc::EIO)
            }
        }
    }

    fn destroy(&mut self) {}

    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let filename = match name.to_str() {
            Some(s) => s,
            None => {
                debug_println!("❌ [LOOKUP] Invalid file name: {:?}", name);
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
                    let ttl = Duration::from_secs(300);
                    reply.entry(&ttl, &attr, 0);
                    return;
                }
                Err(_) => {
                    let attr = attributes::new_directory_attr(parent, 0o755);
                    let ttl = Duration::from_secs(300);
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
                    std::path::Path
                        ::new(&parent_path)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or("/".to_string())
                };

                let grandparent_ino = self.path_to_inode
                    .get(&grandparent_path)
                    .copied()
                    .unwrap_or(1);
                attributes::new_directory_attr(grandparent_ino, 0o755)
            };

            let ttl = Duration::from_secs(300);
            reply.entry(&ttl, &parent_attr, 0);
            return;
        }

        let parent_path = match self.get_path(parent) {
            Some(path) => path.clone(),
            None => {
                debug_println!("❌ [LOOKUP] Parent directory with inode {} not found", parent);
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
                    let ttl = Duration::from_secs(300);
                    reply.entry(&ttl, &attr, 0);
                    return;
                }
                Err(ClientError::NotFound { .. }) => {
                    self.unregister_inode(existing_inode);
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(e) => {
                    debug_println!("❌ [LOOKUP] Error verifying cache: {}", e);

                    let attr = attributes::new_file_attr(existing_inode, 0, 0o644);
                    let ttl = Duration::from_secs(300);
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
        let metadata_result = rt.block_on(async {
            self.client.get_file_metadata(&full_path).await
        });

        match metadata_result {
            Ok(metadata) => {
                let new_inode = self.generate_inode();
                self.register_inode(new_inode, full_path.clone());

                let attr = attributes::from_metadata(new_inode, &metadata);
                let ttl = Duration::from_secs(300);
                reply.entry(&ttl, &attr, 0);
            }
            Err(ClientError::NotFound { .. }) => {
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                reply.error(libc::EACCES);
            }
            Err(_e) => {
                reply.error(libc::EIO);
            }
        }
    }

    fn forget(&mut self, _req: &Request<'_>, _ino: u64, _nlookup: u64) {}

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        if ino == 1 {
            let attr = attributes::new_directory_attr(1, 0o755);
            let ttl = Duration::from_secs(300);
            reply.attr(&ttl, &attr);
            return;
        }

        let path = match self.inode_to_path.get(&ino) {
            Some(p) => {
                p.clone()
            }
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
                let ttl = Duration::from_secs(300);
                reply.attr(&ttl, &attr);
            }
            Err(ClientError::NotFound { .. }) => {
                reply.error(libc::ENOENT);
            }
            Err(e) => {
                debug_println!("RemoteFileSystem: error getattr({}): {}", path, e);
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
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        flags: Option<u32>,
        reply: ReplyAttr
    ) {



        if ino == 1 {
            debug_println!("⚠️ [SETATTR] Try to modify root directory");
            reply.error(libc::EPERM);
            return;
        }

        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                debug_println!("❌ [SETATTR] Inode {} not found", ino);
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


        let current_metadata = match
            rt.block_on(async { self.client.get_file_metadata(&path).await })
        {
            Ok(metadata) => metadata,
            Err(ClientError::NotFound { .. }) => {
                debug_println!("❌ [SETATTR] File not found on server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                debug_println!("❌ [SETATTR] Error retrieving metadata for '{}': {}", path, e);
                reply.error(libc::EIO);
                return;
            }
        };




        if let Some(new_size) = size {

            match current_metadata.kind {
                FileKind::Directory => {
                    debug_println!("⚠️ [SETATTR] Try to open directory: {}", path);
                    reply.error(libc::EISDIR);
                    return;
                }
                _ => {

                }
            }

            let current_size = current_metadata.size;

            if new_size == current_size {
                self.get_current_attributes(ino, &path, reply);
                return;
            }

            let now_iso = chrono::Utc::now().to_rfc3339();

            let operation_result = if new_size < current_size {

                rt.block_on(async { self.client.write_file(
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
                        })
                    ).await })
            } else {

                let padding_size = new_size - current_size;
                let padding_data = vec![0u8; padding_size as usize];

                rt.block_on(async { self.client.write_file(
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
                        })
                    ).await })
            };


            match operation_result {
                Ok(()) => {
                    self.get_current_attributes(ino, &path, reply);
                }
                Err(e) => {
                    debug_println!("❌ [SETATTR] Error modifying size: {}", e);
                    let error_code = match e {
                        ClientError::NotFound { .. } => libc::ENOENT,
                        ClientError::PermissionDenied(_) => libc::EPERM,
                        ClientError::Server { status: 413, .. } => libc::EFBIG,
                        ClientError::Server { status: 507, .. } => libc::ENOSPC,
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
                    debug_println!("❌ [SETATTR] Error modifying permissions: {}", e);
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
            debug_println!("⚠️ [SETATTR] Change of uid/gid not supported on remote filesystem");
            reply.error(libc::EPERM);
            return;
        }


        if _atime.is_some() || _mtime.is_some() || _ctime.is_some() {
            self.get_current_attributes(ino, &path, reply);
            return;
        }


        if flags.is_some() {
            debug_println!("⚠️ [SETATTR] Change flags not supported");
            reply.error(libc::ENOSYS);
            return;
        }


        self.get_current_attributes(ino, &path, reply);
    }

    fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {

        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                debug_println!("❌ [READLINK] Inode {} not found", ino);
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
                match (metadata.kind, &metadata.ref_path) {
                    (FileKind::Symlink, Some(target)) if !target.is_empty() => {
                        reply.data(target.as_bytes());
                    }
                    (FileKind::Symlink, _) => {
                        debug_println!("❌ [READLINK] Symlink with invalid target: {}", path);
                        reply.error(libc::EIO);
                    }
                    (FileKind::RegularFile, _) => {
                        reply.error(libc::EINVAL);
                    }
                    (FileKind::Directory, _) => {
                        debug_println!("❌ [READLINK] Attempting readlink on directory: {}", path);
                        reply.error(libc::EINVAL);
                    }
                    (FileKind::Hardlink, _) => {
                        debug_println!("❌ [READLINK] Attempting readlink on hardlink: {}", path);
                        reply.error(libc::EINVAL);
                    }
                }
            }
            Err(ClientError::NotFound { .. }) => {
                debug_println!("❌ [READLINK] File not found: {}", path);
                reply.error(libc::ENOENT);
            }
            Err(e) => {
                debug_println!("❌ [READLINK] Server error: {}", e);
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
        reply: ReplyEntry
    ) {
        let filename = match name.to_str() {
            Some(s) => s,
            None => {
                debug_println!("❌ [MKNOD] Invalid file name: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                debug_println!("❌ [MKNOD] Parent directory with inode {} not found", parent);
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
            debug_println!("⚠️ [MKNOD] File exists: {}", full_path);
            reply.error(libc::EEXIST);
            return;
        }

        let file_type = mode & libc::S_IFMT;

        match file_type {
            libc::S_IFREG => {
                let rt = match tokio::runtime::Handle::try_current() {
                    Ok(handle) => handle,
                    Err(_) => {
                        let runtime = tokio::runtime::Runtime
                            ::new()
                            .expect("Failed to create runtime");
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

                let create_result = rt.block_on(async {
                    self.client.write_file(&write_request).await
                });

                match create_result {
                    Ok(()) => {
                        debug_println!("✅ [MKNOD] File created on server successfully");


                        let new_inode = self.generate_inode();
                        self.register_inode(new_inode, full_path.clone());


                        let metadata_result = rt.block_on(async {
                            self.client.get_file_metadata(&full_path).await
                        });

                        match metadata_result {
                            Ok(metadata) => {

                                let attr = attributes::from_metadata(new_inode, &metadata);
                                let ttl = Duration::from_secs(300);
                                reply.entry(&ttl, &attr, 0);

                                
                            }
                            Err(e) => {
                                debug_println!(
                                    "❌ [MKNOD] Error retrieving metadata after creation: {}",
                                    e
                                );

                                let effective_perms = mode & 0o777 & !(umask & 0o777);
                                let attr = new_file_attr(new_inode, 0, effective_perms);
                                let ttl = Duration::from_secs(300);
                                reply.entry(&ttl, &attr, 0);
                            }
                        }
                    }
                    Err(e) => {
                        debug_println!("❌ [MKNOD] Error creating file on server: {}", e);
                        match e {
                            ClientError::NotFound { .. } => reply.error(libc::ENOENT),
                            _ => reply.error(libc::EIO),
                        }
                    }
                }
            }
            libc::S_IFIFO => {

                debug_println!("⚠️ [MKNOD] Named pipe not supported: {}", full_path);
                reply.error(libc::EPERM);
            }
            libc::S_IFCHR => {

                debug_println!(
                    "⚠️ [MKNOD] Character device not supported: {} (rdev: {})",
                    full_path,
                    rdev
                );
                reply.error(libc::EPERM);
            }
            libc::S_IFBLK => {

                debug_println!(
                    "⚠️ [MKNOD] Block device not supported: {} (rdev: {})",
                    full_path,
                    rdev
                );
                reply.error(libc::EPERM);
            }
            libc::S_IFSOCK => {

                debug_println!("⚠️ [MKNOD] Socket not supported: {}", full_path);
                reply.error(libc::EPERM);
            }
            _ => {

                debug_println!("❌ [MKNOD] Unknown file type: {:#o}", file_type);
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
        reply: ReplyEntry
    ) {

        let dirname = match name.to_str() {
            Some(s) => s,
            None => {
                debug_println!("❌ [MKDIR] Invalid directory name: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };


        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                debug_println!("❌ [MKDIR] Parent directory with inode {} not found", parent);
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
            debug_println!("⚠️ [MKDIR] Directory already exists: {}", full_path);
            reply.error(libc::EEXIST);
            return;
        }


        let effective_permissions = mode & 0o777 & !(umask & 0o777);



        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
    // removed noisy confirmation log
        let create_result = rt.block_on(async { self.client.create_directory(&full_path).await });

        match create_result {
            Ok(()) => {



                let new_inode = self.generate_inode();
                self.register_inode(new_inode, full_path.clone());


                let metadata_result = rt.block_on(async {
                    self.client.get_file_metadata(&full_path).await
                });

                match metadata_result {
                    Ok(metadata) => {

                        let attr = attributes::from_metadata(new_inode, &metadata);
                        let ttl = Duration::from_secs(300);
                        reply.entry(&ttl, &attr, 0);

                    }
                    Err(e) => {
                        debug_println!("❌ [MKDIR] Error retrieving metadata after creation: {}", e);

                        let attr = new_directory_attr(new_inode, effective_permissions);
                        let ttl = Duration::from_secs(300);
                        reply.entry(&ttl, &attr, 0);
                    }
                }
            }
            Err(e) => {
                debug_println!("❌ [MKDIR] Error on creating directory on server: {}", e);
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
                debug_println!("❌ [UNLINK] Invalid file name: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };


        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                debug_println!("❌ [UNLINK] Parent directory with inode {} not found", parent);
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
                debug_println!("⚠️ [UNLINK] File not found in cache: {}", full_path);

                let rt = match tokio::runtime::Handle::try_current() {
                    Ok(handle) => handle,
                    Err(_) => {
                        let runtime = tokio::runtime::Runtime
                            ::new()
                            .expect("Failed to create runtime");
                        runtime.handle().clone()
                    }
                };
                match rt.block_on(async { self.client.get_file_metadata(&full_path).await }) {
                    Ok(_) => {
                    }
                    Err(ClientError::NotFound { .. }) => {
                        reply.error(libc::ENOENT);
                        return;
                    }
                    Err(e) => {
                        debug_println!("❌ [UNLINK] Error on verifying existence: {}", e);
                        reply.error(libc::EIO);
                        return;
                    }
                }
                0
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
                        debug_println!("⚠️ [UNLINK] Attempt to unlink a directory: {}", full_path);
                        reply.error(libc::EISDIR);
                        return;
                    }
                }
                Err(ClientError::NotFound { .. }) => {
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(e) => {
                    debug_println!("❌ [UNLINK] Error on verifying file type: {}", e);
                    reply.error(libc::EIO);
                    return;
                }
            }
        }


        let is_file_open = self.open_files.values().any(|open_file| open_file.path == full_path);
        if is_file_open {
            debug_println!("⚠️ [UNLINK] File still open: {}", full_path);


            reply.error(libc::EBUSY);
            return;
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
                debug_println!("⚠️ [UNLINK] File already deleted on server: {}", full_path);

                self.remove_path_mapping(&full_path);
                reply.ok();
            }
            Err(e) => {
                debug_println!("❌ [UNLINK] Error on deletion from server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {


        let dirname = match name.to_str() {
            Some(s) => s,
            None => {
                debug_println!("❌ [RMDIR] Invalid directory name: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };


        if dirname == "." || dirname == ".." {
            debug_println!("⚠️ [RMDIR] Attempt to delete special directory: {}", dirname);
            reply.error(libc::EINVAL);
            return;
        }


        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                debug_println!("❌ [RMDIR] Parent directory with inode {} not found", parent);
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
            debug_println!("⚠️ [RMDIR] Attempt to delete root directory");
            reply.error(libc::EBUSY);
            return;
        }


        let dir_inode = match self.path_to_inode.get(&full_path) {
            Some(&inode) => inode,
            None => {
                debug_println!("⚠️ [RMDIR] Directory not found in cache: {}", full_path);

                let rt = match tokio::runtime::Handle::try_current() {
                    Ok(handle) => handle,
                    Err(_) => {
                        let runtime = tokio::runtime::Runtime
                            ::new()
                            .expect("Failed to create runtime");
                        runtime.handle().clone()
                    }
                };
                match rt.block_on(async { self.client.get_file_metadata(&full_path).await }) {
                    Ok(metadata) => {
                        if metadata.kind != FileKind::Directory {
                            debug_println!("⚠️ [RMDIR] '{}' is not a directory", full_path);
                            reply.error(libc::ENOTDIR);
                            return;
                        }


                    }
                    Err(ClientError::NotFound { .. }) => {
                        reply.error(libc::ENOENT);
                        return;
                    }
                    Err(e) => {
                        debug_println!("❌ [RMDIR] Error on verifying existence: {}", e);
                        reply.error(libc::EIO);
                        return;
                    }
                }
                0
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
                        debug_println!("⚠️ [RMDIR] Attempt to rmdir a file: {}", full_path);
                        reply.error(libc::ENOTDIR);
                        return;
                    }
                }
                Err(ClientError::NotFound { .. }) => {
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(e) => {
                    debug_println!("❌ [RMDIR] Error on verifying directory type: {}", e);
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
                    debug_println!(
                        "⚠️ [RMDIR] Directory not empty: {} ({} elementi)",
                        full_path,
                        listing.files.len()
                    );
                    reply.error(libc::ENOTEMPTY);
                    return;
                }
            }
            Err(ClientError::NotFound { .. }) => {

                debug_println!("📝 [RMDIR] Directory already missing on server");
            }
            Err(e) => {
                debug_println!("❌ [RMDIR] Error on verifying empty directory: {}", e);
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
                debug_println!("⚠️ [RMDIR] Directory already deleted from server: {}", full_path);

                if dir_inode != 0 {
                    self.unregister_inode(dir_inode);
                }
                reply.ok();
            }
            Err(e) => {
                debug_println!("❌ [RMDIR] Error on deletion from server: {}", e);
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
        reply: ReplyEntry
    ) {


        let link_name = match name.to_str() {
            Some(s) => s,
            None => {
                debug_println!("❌ [SYMLINK] Invalid symlink name: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        let target_path = match link.to_str() {
            Some(s) => s,
            None => {
                debug_println!("❌ [SYMLINK] Invalid target path: {:?}", link);
                reply.error(libc::EINVAL);
                return;
            }
        };


        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                debug_println!("❌ [SYMLINK] Parent directory with inode {} not found", parent);
                reply.error(libc::ENOENT);
                return;
            }
        };



        let symlink_path = if parent_path == "/" {
            format!("/{}", link_name)
        } else {
            format!("{}/{}", parent_path, link_name)
        };


        if self.path_to_inode.contains_key(&symlink_path) {
            debug_println!("⚠️ [SYMLINK] Symlink already esistente: {}", symlink_path);
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

        match rt.block_on(async { self.client.write_file(&symlink_request).await }) {
            Ok(()) => {


                let new_inode = self.generate_inode();
                self.register_inode(new_inode, symlink_path.to_string().clone());


                let metadata_result = rt.block_on(async {
                    self.client.get_file_metadata(&symlink_path).await
                });

                match metadata_result {
                    Ok(metadata) => {

                        let attr = attributes::from_metadata(new_inode, &metadata);
                        let ttl = Duration::from_secs(300);
                        reply.entry(&ttl, &attr, 0);


                    }
                    Err(_) => {
                        reply.error(libc::EIO);
                    }
                }
            }
            Err(e) => {
                debug_println!("❌ [SYMLINK] Error on creating symlink on server: {}", e);
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
        reply: fuser::ReplyEmpty
    ) {


        let old_filename = match name.to_str() {
            Some(s) => s,
            None => {
                debug_println!("❌ [RENAME] Original file name invalid: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        let new_filename = match newname.to_str() {
            Some(s) => s,
            None => {
                debug_println!("❌ [RENAME] New file name invalid: {:?}", newname);
                reply.error(libc::EINVAL);
                return;
            }
        };


        if flags != 0 {
            debug_println!("⚠️ [RENAME] Flags not supported: {}, proceeding anyway", flags);
        }


        let old_parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                debug_println!(
                    "❌ [RENAME] Original parent directory with inode {} not found",
                    parent
                );
                reply.error(libc::ENOENT);
                return;
            }
        };


        let new_parent_path = match self.get_path(newparent) {
            Some(p) => p.clone(),
            None => {
                debug_println!(
                    "❌ [RENAME] Nuova directory padre con inode {} not found",
                    newparent
                );
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
            debug_println!("⚠️ [RENAME] Attempt to rename root directory");
            reply.error(libc::EBUSY);
            return;
        }

        if
            old_filename == "." ||
            old_filename == ".." ||
            new_filename == "." ||
            new_filename == ".."
        {
            debug_println!("⚠️ [RENAME] Attempt to rename special directories");
            reply.error(libc::EINVAL);
            return;
        }

        if old_path == new_path {
            reply.ok();
            return;
        }

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };


        let old_metadata = match
            rt.block_on(async { self.client.get_file_metadata(&old_path).await })
        {
            Ok(metadata) => metadata,
            Err(ClientError::NotFound { .. }) => {
                debug_println!("❌ [RENAME] Original file not found: {}", old_path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                debug_println!("❌ [RENAME] Error on verifying original file: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };


        let file_inode = self.path_to_inode.get(&old_path).copied().unwrap_or(0);
        if file_inode != 0 {
            let is_file_open = self.open_files.values().any(|open_file| open_file.path == old_path);
            if is_file_open {
                debug_println!("⚠️ [RENAME] File still open: {}", old_path);
                reply.error(libc::EBUSY);
                return;
            }
        }


        if
            let Ok(new_metadata) = rt.block_on(async {
                self.client.get_file_metadata(&new_path).await
            })
        {



            if old_metadata.kind != new_metadata.kind {
                if old_metadata.kind == FileKind::Directory {

                    reply.error(libc::ENOTDIR);
                } else {

                    reply.error(libc::EISDIR);
                }
                return;
            }


            if new_metadata.kind == FileKind::Directory {
                match rt.block_on(async { self.client.list_directory(&new_path).await }) {
                    Ok(listing) => {
                        if !listing.files.is_empty() {
                            debug_println!(
                                "⚠️ [RENAME] Destination directory not empty: {}",
                                new_path
                            );
                            reply.error(libc::ENOTEMPTY);
                            return;
                        }
                    }
                    Err(e) => {
                        debug_println!("❌ [RENAME] Error on verifying empty directory: {}", e);
                        reply.error(libc::EIO);
                        return;
                    }
                }
            }
        }


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

        let rename_result = rt.block_on(async { self.client.write_file(&rename_request).await });

        match rename_result {
            Ok(()) => {


                if file_inode != 0 {

                    self.inode_to_path.remove(&file_inode);
                    self.path_to_inode.remove(&old_path);


                    if let Some(&dest_inode) = self.path_to_inode.get(&new_path) {
                        if dest_inode != file_inode {
                            self.unregister_inode(dest_inode);
                        }
                    }


                    self.inode_to_path.insert(file_inode, new_path.clone());
                    self.path_to_inode.insert(new_path.clone(), file_inode);

                    debug_println!(
                        "🔄 [RENAME] Cache updated: inode {} from '{}' to '{}'",
                        file_inode,
                        old_path,
                        new_path
                    );
                }

                reply.ok();
            }
            Err(ClientError::NotFound { .. }) => {
                debug_println!("❌ [RENAME] File originale not found sul server: {}", old_path);
                reply.error(libc::ENOENT);
            }
            Err(e) => {
                debug_println!("❌ [RENAME] Error rename sul server: {}", e);
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
        reply: ReplyEntry
    ) {


        let link_name = match newname.to_str() {
            Some(s) => s,
            None => {
                debug_println!("❌ [LINK] Name hard link invalid: {:?}", newname);
                reply.error(libc::EINVAL);
                return;
            }
        };


        let source_path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                debug_println!("❌ [LINK] Inode sorgente {} not found", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };


        let parent_path = match self.get_path(newparent) {
            Some(p) => p.clone(),
            None => {
                debug_println!("❌ [LINK] Parent directory with inode {} not found", newparent);
                reply.error(libc::ENOENT);
                return;
            }
        };



        let link_path = if parent_path == "/" {
            format!("/{}", link_name)
        } else {
            format!("{}/{}", parent_path, link_name)
        };


        if self.path_to_inode.contains_key(&link_path) {
            debug_println!("⚠️ [LINK] Hard link already esistente: {}", link_path);
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


        let source_metadata = match
            rt.block_on(async { self.client.get_file_metadata(&source_path).await })
        {
            Ok(metadata) => metadata,
            Err(ClientError::NotFound { .. }) => {
                debug_println!("❌ [LINK] Source file not found: {}", source_path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                debug_println!("❌ [LINK] Error on verifying source file: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };


        match source_metadata.kind {
            FileKind::RegularFile => {
            }
            FileKind::Directory => {
                debug_println!("⚠️ [LINK] Impossible to create hard link on directory: {}", source_path);
                reply.error(libc::EPERM);
                return;
            }
            FileKind::Symlink => {
                debug_println!("⚠️ [LINK] Hard link on symlink not supported: {}", source_path);
                reply.error(libc::EPERM);
                return;
            }
            _ => {
                debug_println!(
                    "⚠️ [LINK] Type of file not supported for hard link: {:?}",
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



                let updated_metadata = match
                    rt.block_on(async { self.client.get_file_metadata(&link_path).await })
                {
                    Ok(metadata) => metadata,
                    Err(e) => {
                        debug_println!("❌ [LINK] Error on retrieving metadata after creation: {}", e);

                        source_metadata
                    }
                };


                let attr = attributes::from_metadata(ino, &updated_metadata);
                let ttl = Duration::from_secs(300);
                reply.entry(&ttl, &attr, 0);

            }
            Err(ClientError::NotFound { .. }) => {
                debug_println!(
                    "❌ [LINK] File sorgente not found durante creazione: {}",
                    source_path
                );
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                debug_println!("❌ [LINK] Permission denied for hard link creation");
                reply.error(libc::EPERM);
            }
            Err(e) => {
                debug_println!("❌ [LINK] Error on hard link creation on server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }


    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {


        let path = match self.inode_to_path.get(&ino) {
            Some(p) => {
                p.clone()
            }
            None => {
                debug_println!("❌ [OPEN] Inode {} not found", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };



        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle
            }
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };


        let metadata_result = rt.block_on(async {
            let result = self.client.get_file_metadata(&path).await;
            result
        });


        let metadata = match metadata_result {
            Ok(metadata) => {
                metadata
            }
            Err(ClientError::NotFound { .. }) => {
                debug_println!("❌ [OPEN] File Not Found: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                debug_println!("❌ [OPEN] Error on metadata: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };





        match metadata.kind {
            FileKind::RegularFile => {
            }
            FileKind::Symlink => {
            }
            FileKind::Directory => {
                debug_println!("❌ [OPEN] È una directory");
                reply.error(libc::EISDIR);
                return;
            }
            _ => {
                debug_println!("❌ [OPEN] Tipo file non supportato: {:?}", metadata.kind);
                reply.error(libc::EPERM);
                return;
            }
        }



        let access_mode = flags & libc::O_ACCMODE;



        let perms = parse_permissions(&metadata.perm);

        let effective_perms = perms.owner;

        match access_mode {
            libc::O_RDONLY => {
                if (effective_perms & 0o4) == 0 {

                    debug_println!("❌ [OPEN] Read permission denied");
                    reply.error(libc::EACCES);
                    return;
                }
            }
            libc::O_WRONLY => {
                if (effective_perms & 0o2) == 0 {

                    debug_println!("❌ [OPEN] Write permission denied");
                    reply.error(libc::EACCES);
                    return;
                }
            }
            libc::O_RDWR => {
                if (effective_perms & 0o6) != 0o6 {

                    debug_println!("❌ [OPEN] Insufficient read/write permissions");
                    reply.error(libc::EACCES);
                    return;
                }
            }
            _ => {
                debug_println!("❌ [OPEN] Invalid access mode: {:#x}", access_mode);
                reply.error(libc::EINVAL);
                return;
            }
        }



        let fh = self.next_fh;
        self.next_fh += 1;



        self.open_files.insert(fh, OpenFile {
            path: path.clone(),
            flags,
            write_buffer: Vec::new(),
            buffer_dirty: false,
        });




        reply.opened(fh, 0);

    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData
    ) {



        if offset < 0 {
            debug_println!("❌ [READ] Offset negativo: {}", offset);
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
                debug_println!("❌ [READ] File handle {} not found", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        let path = open_file.path.clone();


        let access_mode = open_file.flags & libc::O_ACCMODE;
        if access_mode == libc::O_WRONLY {
            debug_println!("⚠️ [READ] Attempt to read on file opened in WRITE-ONLY: {}", path);
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
                debug_println!("❌ [READ] File not found on server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                debug_println!("❌ [READ] Error on metadata verification: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };


        match metadata.kind {
            FileKind::RegularFile | FileKind::Symlink => {
            }
            FileKind::Directory => {
                debug_println!("⚠️ [READ] Attempt to read su directory: {}", path);
                reply.error(libc::EISDIR);
                return;
            }
            _ => {
                debug_println!("⚠️ [READ] Type of file not supported for read: {:?}", metadata.kind);
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
            self.client.read_file(&path, Some(offset_u64), Some(bytes_to_read)).await
        });

        match read_result {
            Ok(read_response) => {
                let data = read_response.data;


                if data.len() > (bytes_to_read as usize) {
                    debug_println!(
                        "⚠️ [READ] Server has returned more data than requested: {} > {}, truncating",
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
                debug_println!("❌ [READ] File deleted during read: {}", path);
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                debug_println!("❌ [READ] Read permission denied: {}", path);
                reply.error(libc::EACCES);
            }
            Err(e) => {
                debug_println!("❌ [READ] Error on server read: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: fuser::ReplyWrite
    ) {


        if offset < 0 {
            debug_println!("❌ [WRITE] Negative offset: {}", offset);
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
                debug_println!("❌ [WRITE] File handle {} not found", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        let path = open_file.path.clone();
        let open_flags = open_file.flags;


        let access_mode = open_flags & libc::O_ACCMODE;
        if access_mode == libc::O_RDONLY {
            debug_println!("⚠️ [WRITE] Attempt to write in READ-ONLY: {}", path);
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
                debug_println!("❌ [WRITE] File not found on server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                debug_println!("❌ [WRITE] Error on metadata verification: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };


        match metadata.kind {
            FileKind::RegularFile | FileKind::Symlink => {
            }
            FileKind::Directory => {
                debug_println!("⚠️ [WRITE] Attempt to write su directory: {}", path);
                reply.error(libc::EISDIR);
                return;
            }
            _ => {
                debug_println!("⚠️ [WRITE] Type of file not supported for write: {:?}", metadata.kind);
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
        let (write_mode, final_data) = if effective_offset == current_file_size+file1 as u64 {



            (Mode::Append, data.to_vec())
        } else if effective_offset == 0 && (data_len as u64) >= current_file_size {
            (Mode::Write, data.to_vec())
        } else {
                    (Mode::Write, data.to_vec())

        };

        match write_mode {
            Mode::Append => {

                let open_file = self.open_files.get_mut(&fh);
                if let Some(file) = open_file {
                    file.write_buffer.extend_from_slice(&final_data);
                    file.buffer_dirty = true;
                }
                reply.written(final_data.len() as u32);
                return;
            }
            _ => {


                let now_iso1 = chrono::Utc::now().to_rfc3339();

                let open_file = self.open_files.get_mut(&fh);
                let file = if open_file.is_some() {
                    open_file.unwrap()
                } else {
                    debug_println!("❌ [WRITE] File handle {} not found", fh);
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

                let write_result1 = rt.block_on(async {
                    self.client.write_file(&write_request1).await
                });

                if let Err(e) = write_result1 {
                   debug_println!("❌ [WRITE] Error on file write: {}", e);
                    reply.error(libc::EIO);
                    return;
                }

                file.buffer_dirty = false;
                file.write_buffer.clear();
            }}
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

                reply.written(data_len as u32);
            }
            Err(ClientError::NotFound { .. }) => {
                debug_println!("❌ [WRITE] File not found during write: {}", path);
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                debug_println!("❌ [WRITE] Write permission denied: {}", path);
                reply.error(libc::EACCES);
            }
            Err(ClientError::Server { status: 413, .. }) => {
                debug_println!("❌ [WRITE] File too large: {}", path);
                reply.error(libc::EFBIG);
            }
            Err(ClientError::Server { status: 507, .. }) => {
                debug_println!("❌ [WRITE] Insufficient space on server: {}", path);
                reply.error(libc::ENOSPC);
            }
            Err(e) => {
                debug_println!("❌ [WRITE] Error on server write: {}", e);
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
        reply: fuser::ReplyEmpty
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
                debug_println!("❌ [WRITE] File handle {} not found", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        if open_file.write_buffer.is_empty() {
            reply.ok();
            return;
        }

        let metadata = match
            rt.block_on(async { self.client.get_file_metadata(&open_file.path).await })
        {
            Ok(metadata) => metadata,
            Err(ClientError::NotFound { .. }) => {
                debug_println!("❌ [WRITE] File not found on server: {}", open_file.path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                debug_println!("❌ [WRITE] Error on metadata verification: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        if open_file.buffer_dirty {
            let now_iso1 = chrono::Utc::now().to_rfc3339();

            let open_file = self.open_files.get_mut(&fh);
            let _file: &mut OpenFile = if open_file.is_some() {
                open_file.unwrap()
            } else {
                debug_println!("❌ [WRITE] File handle {} not found", fh);
                reply.error(libc::EBADF);
                return;
            };

            let file = if let Some(f) = self.open_files.get_mut(&fh) {
                f
            } else {
                debug_println!("❌ [WRITE] File handle {} not found", fh);
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

            let write_result1 = rt.block_on(async {
                self.client.write_file(&write_request1).await
            });

            if let Err(e) = write_result1 {
                debug_println!("❌ [WRITE] Error on file write: {}", e);
                reply.error(libc::EIO);
                return;
            }
        }

        reply.ok()
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty
    ) {


        let _open_file = match self.open_files.get(&fh) {
            Some(file) => file,
            None => {
                debug_println!("⚠️ [RELEASE] File handle {} already rilasciato o inesistente", fh);

                reply.ok();
                return;
            }
        };









        reply.ok();
    }

    fn fsync(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _datasync: bool,
        reply: fuser::ReplyEmpty
    ) {


        let open_file = match self.open_files.get(&fh) {
            Some(file) => file,
            None => {
                debug_println!("❌ [FSYNC] File handle {} not found", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        let path = open_file.path.clone();


        let access_mode = open_file.flags & libc::O_ACCMODE;
        if access_mode == libc::O_RDONLY {
            debug_println!("⚠️ [FSYNC] File opened in read-only: {}", path);
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
                debug_println!("❌ [FSYNC] File not found during fsync: {}", path);
                reply.error(libc::ENOENT);
            }
            Err(e) => {
                debug_println!("❌ [FSYNC] Error on server check: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn opendir(&mut self, _req: &Request<'_>, ino: u64, _flags: i32, reply: ReplyOpen) {


        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                debug_println!("❌ [OPENDIR] Inode {} not found", ino);
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
                debug_println!("❌ [OPENDIR] Directory not found on server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                debug_println!("❌ [OPENDIR] Error on metadata check: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };


        if metadata.kind != FileKind::Directory {
            debug_println!("⚠️ [OPENDIR] '{}' is not a directory: {:?}", path, metadata.kind);
            reply.error(libc::ENOTDIR);
            return;
        }


        match rt.block_on(async { self.client.list_directory(&path).await }) {
            Ok(_) => {
            }
            Err(ClientError::PermissionDenied(_)) => {
                debug_println!("❌ [OPENDIR] Read permission denied: {}", path);
                reply.error(libc::EACCES);
                return;
            }
            Err(e) => {
                debug_println!("❌ [OPENDIR] Error on directory access: {}", e);
                reply.error(libc::EIO);
                return;
            }
        }


        let dh = self.next_fh;
        self.next_fh += 1;


        self.open_dirs.insert(dh, OpenDir {
            path: path.clone(),
        });




        reply.opened(dh, 0);
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        mut reply: ReplyDirectory
    ) {


        let open_dir = match self.open_dirs.get(&fh) {
            Some(dir) => dir,
            None => {
                debug_println!("❌ [READDIR] Directory handle {} not found", fh);
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
                debug_println!("❌ [READDIR] Directory not found sul server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(ClientError::PermissionDenied(_)) => {
                debug_println!("❌ [READDIR] Read permission denied: {}", path);
                reply.error(libc::EACCES);
                return;
            }
            Err(e) => {
                debug_println!("❌ [READDIR] Error on directory read: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };


        let mut entries = Vec::new();


        entries.push((ino, FileType::Directory, ".".to_string()));


        let parent_ino = if path == "/" {
            1
        } else {

            let parent_path = std::path::Path
                ::new(&path)
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
                FileKind::Hardlink => FileType::RegularFile,
            };

            entries.push((entry_ino, file_type, file_entry.name));
        }



        let start_index = if offset == 0 {
            0
        } else {

            offset as usize
        };

        if start_index >= entries.len() {

            reply.ok();
            return;
        }


        let mut current_offset = start_index;
        for (entry_ino, file_type, name) in entries.into_iter().skip(start_index) {
            current_offset += 1;




            let buffer_full = reply.add(
                entry_ino,
                current_offset as i64,
                file_type,
                name
            );

            if buffer_full {
                break;
            }
        }



        reply.ok();
    }

    fn releasedir(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _flags: i32,
        reply: fuser::ReplyEmpty
    ) {


        let _open_dir = match self.open_dirs.get(&fh) {
            Some(dir) => dir,
            None => {
                debug_println!("⚠️ [RELEASEDIR] Directory handle {} already rilasciato o inesistente", fh);

                reply.ok();
                return;
            }
        };


        reply.ok();
    }

    fn fsyncdir(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _datasync: bool,
        reply: fuser::ReplyEmpty
    ) {


        let open_dir = match self.open_dirs.get(&fh) {
            Some(dir) => dir,
            None => {
                debug_println!("❌ [FSYNCDIR] Directory handle {} not found", fh);
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
                debug_println!("❌ [FSYNCDIR] Directory not found: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                debug_println!("❌ [FSYNCDIR] Error verifica metadati: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        if metadata.kind != FileKind::Directory {
            debug_println!("❌ [FSYNCDIR] '{}' non è una directory", path);
            reply.error(libc::ENOTDIR);
            return;
        }







        match rt.block_on(async { self.client.list_directory(&path).await }) {
            Ok(_) => {
                reply.ok();
            }
            Err(ClientError::NotFound { .. }) => {
                debug_println!("❌ [FSYNCDIR] Directory not found during fsyncdir: {}", path);
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                debug_println!("❌ [FSYNCDIR] Permission denied for directory: {}", path);
                reply.error(libc::EACCES);
            }
            Err(e) => {
                debug_println!("❌ [FSYNCDIR] Error on directory check: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: fuser::ReplyStatfs) {

        let total_blocks = 268435456u64;
        let free_blocks = 134217728u64;
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
            0
        );
    }

    fn setxattr(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _name: &OsStr,
        _value: &[u8],
        _flags: i32,
        _position: u32,
        reply: fuser::ReplyEmpty
    ) {

        reply.error(libc::ENOSYS);
    }

    fn getxattr(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _name: &OsStr,
        _size: u32,
        reply: fuser::ReplyXattr
    ) {

        reply.error(libc::ENOSYS);
    }

    fn listxattr(&mut self, _req: &Request<'_>, ino: u64, size: u32, reply: fuser::ReplyXattr) {
        debug_println!("[Not Implemented] listxattr(ino: {:#x?}, size: {})", ino, size);
        reply.error(libc::ENOSYS);
    }

    fn removexattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        name: &OsStr,
        reply: fuser::ReplyEmpty
    ) {
        debug_println!("[Not Implemented] removexattr(ino: {:#x?}, name: {:?})", ino, name);
        reply.error(libc::ENOSYS);
    }

    fn access(&mut self, _req: &Request<'_>, ino: u64, mask: i32, reply: fuser::ReplyEmpty) {


        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                debug_println!("❌ [ACCESS] Inode {} not found", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };



        let _check_exist =
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
                debug_println!("❌ [ACCESS] File not found: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(ClientError::PermissionDenied(_)) => {
                debug_println!("❌ [ACCESS] Permission denied for metadata: {}", path);
                reply.error(libc::EACCES);
                return;
            }
            Err(e) => {
                debug_println!("❌ [ACCESS] Error verifica esistenza: {}", e);
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

        if check_read && (effective_perms & 0o400) == 0 {
            debug_println!("⚠️ [ACCESS] Permesso lettura negato per: {}", path);
            access_denied = true;
        }

        if check_write && (effective_perms & 0o200) == 0 {
            debug_println!("⚠️ [ACCESS] Permesso scrittura negato per: {}", path);
            access_denied = true;
        }

        if check_exec && (effective_perms & 0o100) == 0 {
            debug_println!("⚠️ [ACCESS] Permesso esecuzione negato per: {}", path);
            access_denied = true;
        }


        if check_exec && metadata.kind == FileKind::Directory {

            debug_println!("🔍 [ACCESS] Directory: permesso esecuzione = attraversamento");
        } else if check_exec && metadata.kind != FileKind::RegularFile {
            debug_println!("⚠️ [ACCESS] Tipo file non eseguibile: {:?}", metadata.kind);
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
        reply: fuser::ReplyCreate
    ) {



        let filename = match name.to_str() {
            Some(s) => s,
            None => {
                debug_println!("❌ [CREATE] Invalid file name: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };


        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                debug_println!("❌ [CREATE] Parent directory with inode {} not found", parent);
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
            debug_println!("⚠️ [CREATE] File already esistente: {}", full_path);
            reply.error(libc::EEXIST);
            return;
        }


        let effective_permissions = mode & 0o777 & !(umask & 0o777);
        let effective_permissions_str = format!("{:o}", effective_permissions);






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


        match rt.block_on(async { self.client.write_file(&create_request).await }) {
            Ok(()) => {


                let new_inode = self.generate_inode();
                self.register_inode(new_inode, full_path.clone());


                let fh = self.next_fh;
                self.next_fh += 1;


                self.open_files.insert(fh, OpenFile {
                    path: full_path.clone(),
                    flags,
                    write_buffer: Vec::new(),
                    buffer_dirty: false,
                });


                let metadata_result = rt.block_on(async {
                    self.client.get_file_metadata(&full_path).await
                });

                match metadata_result {
                    Ok(metadata) => {

                        let attr = attributes::from_metadata(new_inode, &metadata);
                        let ttl = Duration::from_secs(300);



                        reply.created(&ttl, &attr, 0, fh, 0);
                    }
                    Err(e) => {
                        debug_println!("❌ [CREATE] Error recupero metadati: {}", e);

                        let attr = new_file_attr(new_inode, 0, effective_permissions);
                        let ttl = Duration::from_secs(300);
                        reply.created(&ttl, &attr, 0, fh, 0);
                    }
                }
            }
            Err(e) => {
                debug_println!("❌ [CREATE] Error creating file on server: {}", e);
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
        _ino: u64,
        _fh: u64,
        _lock_owner: u64,
        _start: u64,
        _end: u64,
        _typ: i32,
        _pid: u32,
        reply: fuser::ReplyLock
    ) {
        reply.locked(0, 0, libc::F_UNLCK, 0);
    }

    fn setlk(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _lock_owner: u64,
        _start: u64,
        _end: u64,
        _typ: i32,
        _pid: u32,
        _sleep: bool,
        reply: fuser::ReplyEmpty
    ) {
        reply.ok();
    }

    fn bmap(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        blocksize: u32,
        idx: u64,
        reply: fuser::ReplyBmap
    ) {

        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                debug_println!("❌ [BMAP] Inode {} not found", ino);
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
                debug_println!("❌ [BMAP] File not found: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                debug_println!("❌ [BMAP] Error metadati: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };


        if metadata.kind != FileKind::RegularFile {
            debug_println!("⚠️ [BMAP] bmap solo supportato per file regolari");
            reply.error(libc::EPERM);
            return;
        }


        let file_size = metadata.size;
        let blocks_in_file = (file_size + (blocksize as u64) - 1) / (blocksize as u64);


        if idx >= blocks_in_file {
            debug_println!("📍 [BMAP] Block {} beyond EOF (file has {} blocks)", idx, blocks_in_file);
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
        reply: fuser::ReplyIoctl
    ) {
        debug_println!(
            "[Not Implemented] ioctl(ino: {:#x?}, fh: {}, flags: {}, cmd: {}, \
            in_data.len(): {}, out_size: {})",
            ino,
            fh,
            flags,
            cmd,
            in_data.len(),
            out_size
        );
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
        reply: fuser::ReplyEmpty
    ) {
        debug_println!(
            "[Not Implemented] fallocate(ino: {:#x?}, fh: {}, offset: {}, \
            length: {}, mode: {})",
            ino,
            fh,
            offset,
            length,
            mode
        );
        reply.error(libc::ENOSYS);
    }

    fn lseek(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        whence: i32,
        reply: fuser::ReplyLseek
    ) {
        debug_println!(
            "[Not Implemented] lseek(ino: {:#x?}, fh: {}, offset: {}, whence: {})",
            ino,
            fh,
            offset,
            whence
        );
        reply.error(libc::ENOSYS);
    }

    fn copy_file_range(
        &mut self,
        _req: &Request<'_>,
        _ino_in: u64,
        fh_in: u64,
        offset_in: i64,
        _ino_out: u64,
        fh_out: u64,
        offset_out: i64,
        len: u64,
        _flags: u32,
        reply: fuser::ReplyWrite
    ) {



        if offset_in < 0 || offset_out < 0 {
            debug_println!("❌ [COPY_FILE_RANGE] Negative offset not allowed");
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
                debug_println!("❌ [COPY_FILE_RANGE] File handle source {} not found", fh_in);
                reply.error(libc::EBADF);
                return;
            }
        };

        let dest_file = match self.open_files.get(&fh_out) {
            Some(file) => file,
            None => {
                debug_println!("❌ [COPY_FILE_RANGE] File handle destination {} not found", fh_out);
                reply.error(libc::EBADF);
                return;
            }
        };


        let source_access = source_file.flags & libc::O_ACCMODE;
        let dest_access = dest_file.flags & libc::O_ACCMODE;

        if source_access == libc::O_WRONLY {
            debug_println!("❌ [COPY_FILE_RANGE] Source file not opened for reading");
            reply.error(libc::EBADF);
            return;
        }

        if dest_access == libc::O_RDONLY {
            debug_println!("❌ [COPY_FILE_RANGE] File destinatione not writable");
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
        let chunk_size = std::cmp::min(len, 1024 * 1024);


        let source_data = match
            rt.block_on(async {
                self.client.read_file(
                    &source_file.path,
                    Some(offset_in as u64),
                    Some(chunk_size)
                ).await
            })
        {
            Ok(data) => data.data,
            Err(e) => {
                debug_println!("❌ [COPY_FILE_RANGE] Error on source read: {}", e);
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


        let dest_metadata = match
            rt.block_on(async { self.client.get_file_metadata(&dest_file.path).await })
        {
            Ok(metadata) => metadata,
            Err(e) => {
                debug_println!("❌ [COPY_FILE_RANGE] Error on destination metadata: {}", e);
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
                debug_println!("❌ [COPY_FILE_RANGE] Error on write: {}", e);
                reply.error(libc::EIO);
            }
        }
    }
}
