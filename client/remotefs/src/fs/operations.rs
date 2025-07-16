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
    // 1. LOOKUP - Cerca file/directory per nome
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
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
        let metadata_result = rt.block_on(async {
            self.client.get_file_metadata(&full_path).await
        });

        match metadata_result {
            Ok(metadata) => {
                // ✅ File esiste - usa metadati reali
                let new_inode = self.generate_inode();
                self.register_inode(new_inode, full_path);
                
                // Usa metadati del server per creare attributi
                let attr = attributes::from_metadata(new_inode, &metadata);
                let ttl = Duration::from_secs(1);
                reply.entry(&ttl, &attr, 0);
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


    // 2. GETATTR - Ottieni attributi di un file
    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        if ino == 1 {
            // Root directory
            let attr = attributes::new_directory_attr(1);
            let ttl = Duration::from_secs(1);
            reply.attr(&ttl, &attr);
            return;
        }

        let path = match self.get_path(ino) {
            Some(p) => p.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // TODO: Chiedi metadati al server
        // Per ora ritorna attributi di default
        let attr = attributes::new_file_attr(ino, 1024);
        let ttl = Duration::from_secs(1);
        reply.attr(&ttl, &attr);
    }

    // 3. READDIR - Leggi contenuto directory
    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let path = match self.get_path(ino) {
            Some(p) => p.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        // Entry base per ogni directory
        if offset == 0 {
            reply.add(ino, 0, FileType::Directory, ".");
            reply.add(1, 1, FileType::Directory, "..");
        }

        // TODO: Chiedi lista file al server
        // Per ora simula alcuni file
        if offset <= 2 {
            reply.add(self.generate_inode(), 2, FileType::RegularFile, "test.txt");
        }

        reply.ok();
    }

    // 4. OPEN - Apri file
    fn open(&mut self, _req: &Request, ino: u64, flags: i32, reply: ReplyOpen) {
        let path = match self.get_path(ino) {
            Some(p) => p.clone(),
            None => {
                reply.error(libc::ENOENT);
                return;
            }
        };

        let fh = self.next_fh;
        self.next_fh += 1;

        self.open_files.insert(fh, OpenFile { path, flags });

        reply.opened(fh, 0);
    }

    // 5. READ - Leggi dati da file
    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let open_file = match self.open_files.get(&fh) {
            Some(f) => f,
            None => {
                reply.error(libc::EBADF);
                return;
            }
        };

        // TODO: Chiedi dati al server
        // Per ora ritorna dati finti
        let data = b"Hello from remote filesystem!";
        let start = offset as usize;
        let end = std::cmp::min(start + size as usize, data.len());

        if start < data.len() {
            reply.data(&data[start..end]);
        } else {
            reply.data(&[]);
        }
    }

    // 6. RELEASE - Chiudi file
    fn release(
        &mut self,
        _req: &Request,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: fuser::ReplyEmpty,
    ) {
        self.open_files.remove(&fh);
        reply.ok();
    }
}
