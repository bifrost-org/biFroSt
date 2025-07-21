use crate::api::client::{ClientError, RemoteClient};
use crate::api::models::*;
use crate::fs::attributes;
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen,
    Request,
};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::time::{Duration, SystemTime};

pub struct RemoteFileSystem {
    // Mappature inode <-> path
    inode_to_path: HashMap<u64, String>,
    path_to_inode: HashMap<String, u64>,
    next_inode: u64,

    // Client per comunicare con server
    client: RemoteClient,

    // File aperti
    open_files: HashMap<u64, OpenFile>,
    next_fh: u64,
}

struct OpenFile {
    path: String,
    flags: i32,
}

impl RemoteFileSystem {
    pub fn new(client: RemoteClient) -> Self {
        let mut fs = Self {
            inode_to_path: HashMap::new(),
            path_to_inode: HashMap::new(),
            next_inode: 2, // 1 è riservato per root
            client,
            open_files: HashMap::new(),
            next_fh: 1,
        };

        // Inode 1 = directory root
        fs.inode_to_path.insert(1, "/".to_string());
        fs.path_to_inode.insert("/".to_string(), 1);

        fs
    }

    // Genera nuovo inode univoco
    fn generate_inode(&mut self) -> u64 {
        let inode = self.next_inode;
        self.next_inode += 1;
        inode
    }

    // Ottieni path da inode
    fn get_path(&self, inode: u64) -> Option<&String> {
        self.inode_to_path.get(&inode)
    }

    // Salva mappatura inode <-> path
    fn register_inode(&mut self, inode: u64, path: String) {
        self.inode_to_path.insert(inode, path.clone());
        self.path_to_inode.insert(path, inode);
    }

    // Rimuovi mappatura
    fn unregister_inode(&mut self, inode: u64) {
        if let Some(path) = self.inode_to_path.remove(&inode) {
            self.path_to_inode.remove(&path);
        }
    }
}


impl Filesystem for RemoteFileSystem {
    fn init(&mut self, _req: &Request<'_>, _config: &mut fuser::KernelConfig) -> Result<(), libc::c_int> {
         // 1. Configurazione parametri FUSE per filesystem remoto
    let _ = _config.set_max_write(1024 * 1024);        // Buffer scrittura 1MB
    let _ = _config.set_max_readahead(1024 * 1024);    // Buffer lettura anticipata 1MB

    // 2. Verifica connessione al server
    let rt = tokio::runtime::Handle::current();
    match rt.block_on(async { 
        // Verifica che il server sia raggiungibile
        match self.client.get_file_metadata("/").await {
            Ok(_) => Ok(()),
            Err(ClientError::NotFound { .. }) => Ok(()), // È ok se "/" non esiste come file
            Err(e) => Err(e),
        }
    }) {
        Ok(_) => {
            println!("RemoteFileSystem: connessione al server verificata");
            
            // 3. Precarica directory root (opzionale)
            let _ = rt.block_on(async {
                if let Ok(listing) = self.client.list_directory("/").await {
                    println!("RemoteFileSystem: precaricata directory root con {} elementi", 
                             listing.files.len());
                    
                    // Registra file nella cache degli inode
                    for entry in listing.files {
                        if !self.path_to_inode.contains_key(&entry.path) {
                            let new_inode = self.generate_inode();
                            self.register_inode(new_inode, entry.path);
                        }
                    }
                }
            });
            
            Ok(())
        },
        Err(e) => {
            eprintln!("RemoteFileSystem: errore connessione al server: {}", e);
            Err(libc::EIO)
        }
    }
    }

    fn destroy(&mut self) {}

    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
                let filename = match name.to_str() {
            Some(s) => s,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        let parent_path = match self.get_path(parent) {
            Some(path) => path.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Costruisci path completo
        let full_path = if parent_path == "/" {
            format!("/{}", filename)
        } else {
            format!("{}/{}", parent_path, filename)
        };

        /*
        // Controlla se già esiste nella cache
        if let Some(&existing_inode) = self.path_to_inode.get(&full_path) {
            // TODO: Chiedi metadati al server per aggiornare
            let attr = attributes::new_file_attr(existing_inode, 1024);
            let ttl = Duration::from_secs(1);
            reply.entry(&ttl, &attr, 0);
            return;
        }
        */

        // 2. NON IN CACHE - CHIEDI AL SERVER SE ESISTE
        // Esegui chiamata async in runtime sincrono
        let rt = tokio::runtime::Handle::current();
        let metadata_result =
            rt.block_on(async { self.client.get_file_metadata(&full_path).await });

        match metadata_result {
            Ok(metadata) => {
                // ✅ File esiste - usa metadati reali
                if let Some(&existing_inode) = self.path_to_inode.get(&full_path) {
                    // File già conosciuto - usa inode esistente
                    let attr = attributes::from_metadata(existing_inode, &metadata);
                    let ttl = Duration::from_secs(1);
                    reply.entry(&ttl, &attr, 0);
                } else {
                    // 2. Solo se è la prima volta, genera nuovo inode
                    let new_inode = self.generate_inode();
                    self.register_inode(new_inode, full_path);

                    let attr = attributes::from_metadata(new_inode, &metadata);
                    let ttl = Duration::from_secs(1);
                    reply.entry(&ttl, &attr, 0);
                }
            }
            Err(ClientError::NotFound { .. }) => {
                // ❌ File non esiste
                reply.error(libc::ENOENT);
            }
            Err(e) => {
                eprintln!("Errore server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn forget(&mut self, _req: &Request<'_>, _ino: u64, _nlookup: u64) {}

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
    // Caso speciale: inode 1 = directory root
    if ino == 1 {
        let attr = attributes::new_directory_attr(1);
        let ttl = Duration::from_secs(1);
        reply.attr(&ttl, &attr);
        return;
    }

    // Per altri inode, ottieni il path
    let path = match self.inode_to_path.get(&ino) {
        Some(p) => p.clone(),
        None => {
            reply.error(libc::ENOENT);
            return;
        }
    };

    // Chiedi metadati al server
    let rt = tokio::runtime::Handle::current();
    let metadata_result = rt.block_on(async { 
        self.client.get_file_metadata(&path).await 
    });

    match metadata_result {
        Ok(metadata) => {
            // Converti metadati dal server in attributi FUSE
            let attr = attributes::from_metadata(ino, &metadata);
            let ttl = Duration::from_secs(1);
            reply.attr(&ttl, &attr);
        }
        Err(ClientError::NotFound { .. }) => {
            // File non esiste più sul server
            reply.error(libc::ENOENT);
        }
        Err(e) => {
            eprintln!("RemoteFileSystem: errore getattr({}): {}", path, e);
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
        log::debug!(
            "[Not Implemented] setattr(ino: {:#x?}, mode: {:?}, uid: {:?}, \
            gid: {:?}, size: {:?}, fh: {:?}, flags: {:?})",
            ino, mode, uid, gid, size, fh, flags
        );
        reply.error(libc::ENOSYS);
    }

    fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {
        log::debug!("[Not Implemented] readlink(ino: {:#x?})", ino);
        reply.error(libc::ENOSYS);
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
        log::debug!(
            "[Not Implemented] mknod(parent: {:#x?}, name: {:?}, mode: {}, \
            umask: {:#x?}, rdev: {})",
            parent, name, mode, umask, rdev
        );
        reply.error(libc::ENOSYS);
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
        log::debug!(
            "[Not Implemented] mkdir(parent: {:#x?}, name: {:?}, mode: {}, umask: {:#x?})",
            parent, name, mode, umask
        );
        reply.error(libc::ENOSYS);
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        log::debug!(
            "[Not Implemented] unlink(parent: {:#x?}, name: {:?})",
            parent, name,
        );
        reply.error(libc::ENOSYS);
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        log::debug!(
            "[Not Implemented] rmdir(parent: {:#x?}, name: {:?})",
            parent, name,
        );
        reply.error(libc::ENOSYS);
    }

    fn symlink(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        link: &std::path::Path,
        reply: ReplyEntry,
    ) {
        log::debug!(
            "[Not Implemented] symlink(parent: {:#x?}, name: {:?}, link: {:?})",
            parent, name, link,
        );
        reply.error(libc::EPERM);
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
        log::debug!(
            "[Not Implemented] rename(parent: {:#x?}, name: {:?}, newparent: {:#x?}, \
            newname: {:?}, flags: {})",
            parent, name, newparent, newname, flags,
        );
        reply.error(libc::ENOSYS);
    }

    fn link(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        newparent: u64,
        newname: &OsStr,
        reply: ReplyEntry,
    ) {
        log::debug!(
            "[Not Implemented] link(ino: {:#x?}, newparent: {:#x?}, newname: {:?})",
            ino, newparent, newname
        );
        reply.error(libc::EPERM);
    }

    fn open(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        reply.opened(0, 0);
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
        log::warn!(
            "[Not Implemented] read(ino: {:#x?}, fh: {}, offset: {}, size: {}, \
            flags: {:#x?}, lock_owner: {:?})",
            ino, fh, offset, size, flags, lock_owner
        );
        reply.error(libc::ENOSYS);
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
        log::debug!(
            "[Not Implemented] write(ino: {:#x?}, fh: {}, offset: {}, data.len(): {}, \
            write_flags: {:#x?}, flags: {:#x?}, lock_owner: {:?})",
            ino,
            fh,
            offset,
            data.len(),
            write_flags,
            flags,
            lock_owner
        );
        reply.error(libc::ENOSYS);
    }

    fn flush(&mut self, _req: &Request<'_>, ino: u64, fh: u64, lock_owner: u64, reply: fuser::ReplyEmpty) {
        log::debug!(
            "[Not Implemented] flush(ino: {:#x?}, fh: {}, lock_owner: {:?})",
            ino, fh, lock_owner
        );
        reply.error(libc::ENOSYS);
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        reply.ok();
    }

    fn fsync(&mut self, _req: &Request<'_>, ino: u64, fh: u64, datasync: bool, reply: fuser::ReplyEmpty) {
        log::debug!(
            "[Not Implemented] fsync(ino: {:#x?}, fh: {}, datasync: {})",
            ino, fh, datasync
        );
        reply.error(libc::ENOSYS);
    }

    fn opendir(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        reply.opened(0, 0);
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        reply: ReplyDirectory,
    ) {
        log::warn!(
            "[Not Implemented] readdir(ino: {:#x?}, fh: {}, offset: {})",
            ino, fh, offset
        );
        reply.error(libc::ENOSYS);
    }

    fn readdirplus(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        reply: fuser::ReplyDirectoryPlus,
    ) {
        log::debug!(
            "[Not Implemented] readdirplus(ino: {:#x?}, fh: {}, offset: {})",
            ino, fh, offset
        );
        reply.error(libc::ENOSYS);
    }

    fn releasedir(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _flags: i32,
        reply: fuser::ReplyEmpty,
    ) {
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
        log::debug!(
            "[Not Implemented] fsyncdir(ino: {:#x?}, fh: {}, datasync: {})",
            ino, fh, datasync
        );
        reply.error(libc::ENOSYS);
    }

    fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: fuser::ReplyStatfs) {
        reply.statfs(0, 0, 0, 0, 0, 512, 255, 0);
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
        log::debug!(
            "[Not Implemented] setxattr(ino: {:#x?}, name: {:?}, flags: {:#x?}, position: {})",
            ino, name, flags, position
        );
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
        log::debug!(
            "[Not Implemented] getxattr(ino: {:#x?}, name: {:?}, size: {})",
            ino, name, size
        );
        reply.error(libc::ENOSYS);
    }

    fn listxattr(&mut self, _req: &Request<'_>, ino: u64, size: u32, reply: fuser::ReplyXattr) {
        log::debug!(
            "[Not Implemented] listxattr(ino: {:#x?}, size: {})",
            ino, size
        );
        reply.error(libc::ENOSYS);
    }

    fn removexattr(&mut self, _req: &Request<'_>, ino: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        log::debug!(
            "[Not Implemented] removexattr(ino: {:#x?}, name: {:?})",
            ino, name
        );
        reply.error(libc::ENOSYS);
    }

    fn access(&mut self, _req: &Request<'_>, ino: u64, mask: i32, reply: fuser::ReplyEmpty) {
        log::debug!("[Not Implemented] access(ino: {:#x?}, mask: {})", ino, mask);
        reply.error(libc::ENOSYS);
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
        log::debug!(
            "[Not Implemented] create(parent: {:#x?}, name: {:?}, mode: {}, umask: {:#x?}, \
            flags: {:#x?})",
            parent, name, mode, umask, flags
        );
        reply.error(libc::ENOSYS);
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
        log::debug!(
            "[Not Implemented] getlk(ino: {:#x?}, fh: {}, lock_owner: {}, start: {}, \
            end: {}, typ: {}, pid: {})",
            ino, fh, lock_owner, start, end, typ, pid
        );
        reply.error(libc::ENOSYS);
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
        log::debug!(
            "[Not Implemented] setlk(ino: {:#x?}, fh: {}, lock_owner: {}, start: {}, \
            end: {}, typ: {}, pid: {}, sleep: {})",
            ino, fh, lock_owner, start, end, typ, pid, sleep
        );
        reply.error(libc::ENOSYS);
    }

    fn bmap(&mut self, _req: &Request<'_>, ino: u64, blocksize: u32, idx: u64, reply: fuser::ReplyBmap) {
        log::debug!(
            "[Not Implemented] bmap(ino: {:#x?}, blocksize: {}, idx: {})",
            ino, blocksize, idx,
        );
        reply.error(libc::ENOSYS);
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
        log::debug!(
            "[Not Implemented] ioctl(ino: {:#x?}, fh: {}, flags: {}, cmd: {}, \
            in_data.len(): {}, out_size: {})",
            ino,
            fh,
            flags,
            cmd,
            in_data.len(),
            out_size,
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
        reply: fuser::ReplyEmpty,
    ) {
        log::debug!(
            "[Not Implemented] fallocate(ino: {:#x?}, fh: {}, offset: {}, \
            length: {}, mode: {})",
            ino, fh, offset, length, mode
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
        reply: fuser::ReplyLseek,
    ) {
        log::debug!(
            "[Not Implemented] lseek(ino: {:#x?}, fh: {}, offset: {}, whence: {})",
            ino, fh, offset, whence
        );
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
        log::debug!(
            "[Not Implemented] copy_file_range(ino_in: {:#x?}, fh_in: {}, \
            offset_in: {}, ino_out: {:#x?}, fh_out: {}, offset_out: {}, \
            len: {}, flags: {})",
            ino_in, fh_in, offset_in, ino_out, fh_out, offset_out, len, flags
        );
        reply.error(libc::ENOSYS);
    }
}
