use crate::api::client::{ClientError, RemoteClient};
use crate::api::models::*;
use crate::fs::attributes::{self, new_directory_attr, new_file_attr};
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
    fn init(
        &mut self,
        _req: &Request<'_>,
        _config: &mut fuser::KernelConfig,
    ) -> Result<(), libc::c_int> {
        // 1. Configurazione parametri FUSE per filesystem remoto
        let _ = _config.set_max_write(1024 * 1024); // Buffer scrittura 1MB
        let _ = _config.set_max_readahead(1024 * 1024); // Buffer lettura anticipata 1MB

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
                        println!(
                            "RemoteFileSystem: precaricata directory root con {} elementi",
                            listing.files.len()
                        );

                        // Registra file nella cache degli inode
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
            let attr = attributes::new_directory_attr(1, 0o755);
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
        let metadata_result = rt.block_on(async { self.client.get_file_metadata(&path).await });

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
    //COMPLETARLO perchè non so come inviare la richiesta di modifica, aspettare emanuele
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
            "🔧 [SETATTR] ino: {}, mode: {:?}, size: {:?}",
            ino,
            mode,
            size
        );

        // 1. CONTROLLI PRELIMINARI

        // Directory root è read-only
        if ino == 1 {
            log::warn!("⚠️ [SETATTR] Tentativo di modificare directory root");
            reply.error(libc::EPERM);
            return;
        }

        // Ottieni il path dall'inode
        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                log::error!("❌ [SETATTR] Inode {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };

        log::debug!("🔧 [SETATTR] Path: {}", path);

        let rt = tokio::runtime::Handle::current();
        //ASPETTARE CHE EMANUELE MI DICA COME STRUTTURARE IL CAMBIO DI ATTRIBUTI SUL SERVER

        // 2. GESTIONE OPERAZIONI SUPPORTATE
        /*
        // A) TRUNCATE (ridimensionamento file)
        if let Some(new_size) = size {
            rt.block_on(async self.client.write_file())
            self.handle_truncate(ino, &path, new_size, reply);
            return;
        }

        // B) CHMOD (cambio permessi)
        if let Some(new_mode) = mode {
            self.handle_chmod(ino, &path, new_mode, reply);
            return;
        }

        // 3. OPERAZIONI NON SUPPORTATE
        if uid.is_some() || gid.is_some() {
            log::warn!("⚠️ [SETATTR] Cambio uid/gid non supportato");
            reply.error(libc::EPERM);
            return;
        }

        if flags.is_some() {
            log::warn!("⚠️ [SETATTR] Cambio flags non supportato");
            reply.error(libc::ENOSYS);
            return;
        }

        // 4. NESSUNA MODIFICA - RESTITUISCI ATTRIBUTI ATTUALI
        self.get_current_attributes(ino, &path, reply);
        */
    }
    //COMPLETARLO perchè non so come capire se è un link, aspettare emanuele
    fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {
        /*
        // 1. VALIDAZIONE INODE
        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                log::error!("❌ [READLINK] Inode {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };


        // 2. OTTIENI METADATI DAL SERVER
        let rt = tokio::runtime::Handle::current();
        let metadata_result = rt.block_on(async { self.client.get_file_metadata(&path).await });

        match metadata_result {
            Ok(metadata) => {
                // 3. VERIFICA CHE SIA UN LINK SIMBOLICO
                if !metadata.is_symlink {
                    log::warn!("⚠️ [READLINK] '{}' non è un link simbolico", path);
                    reply.error(libc::EINVAL);
                    return;
                }

                // 4. OTTIENI IL TARGET DEL SYMLINK
                match metadata.symlink_target {
                    Some(target) => {
                        log::debug!("✅ [READLINK] Symlink '{}' → '{}'", path, target);
                        reply.data(target.as_bytes());
                    }
                    None => {
                        log::error!("❌ [READLINK] Target mancante per symlink '{}'", path);
                        reply.error(libc::EIO);
                    }
                }
            }
            Err(ClientError::NotFound { .. }) => {
                log::error!("❌ [READLINK] Symlink '{}' non trovato sul server", path);
                reply.error(libc::ENOENT);
            }
            Err(e) => {
                log::error!("❌ [READLINK] Errore server per '{}': {}", path, e);
                reply.error(libc::EIO);
            }
        }
        */
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
        // 1. VALIDAZIONE INPUT
        let filename = match name.to_str() {
            Some(s) => s,
            None => {
                log::error!("❌ [MKNOD] Nome file non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        // 2. OTTIENI PATH DELLA DIRECTORY PADRE
        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                log::error!(
                    "❌ [MKNOD] Directory padre con inode {} non trovata",
                    parent
                );
                reply.error(libc::ENOENT);
                return;
            }
        };

        // 3. COSTRUISCI PATH COMPLETO
        let full_path = if parent_path == "/" {
            format!("/{}", filename)
        } else {
            format!("{}/{}", parent_path, filename)
        };

        log::debug!("🔧 [MKNOD] Path completo: {}", full_path);

        // 4. VERIFICA CHE IL FILE NON ESISTA GIÀ
        if self.path_to_inode.contains_key(&full_path) {
            log::warn!("⚠️ [MKNOD] File già esistente: {}", full_path);
            reply.error(libc::EEXIST);
            return;
        }

        // 5. DETERMINA TIPO DI NODO DA CREARE
        let file_type = mode & libc::S_IFMT;

        match file_type {
            libc::S_IFREG => {
                // FILE REGOLARE - Supportato
                log::debug!("📄 [MKNOD] Creazione file regolare: {}", full_path);
                let rt = tokio::runtime::Handle::current();

                let write_request = WriteRequest {
                    path: full_path.clone(), // ← Clone per usarlo dopo
                    new_path: None,
                    data: Some(Vec::new()),
                    size: Some(0),
                    permissions_octal: Some(((mode & 0o777) & !(umask & 0o777)).to_string()),
                    last_modified: Some(chrono::Utc::now().to_rfc3339()),
                };

                let create_result =
                    rt.block_on(async { self.client.write_file(&write_request).await });

                // 6. GESTISCI RISULTATO CREAZIONE
                match create_result {
                    Ok(()) => {
                        log::debug!("✅ [MKNOD] File creato sul server con successo");

                        // Genera nuovo inode e registra
                        let new_inode = self.generate_inode();
                        self.register_inode(new_inode, full_path.clone());

                        // Ottieni metadati dal server per conferma
                        let metadata_result =
                            rt.block_on(async { self.client.get_file_metadata(&full_path).await });

                        match metadata_result {
                            Ok(metadata) => {
                                // Usa metadati reali dal server
                                let attr = attributes::from_metadata(new_inode, &metadata);
                                let ttl = Duration::from_secs(1);
                                reply.entry(&ttl, &attr, 0);

                                log::debug!("✅ [MKNOD] Entry restituita per inode {}", new_inode);
                            }
                            Err(e) => {
                                log::error!(
                                    "❌ [MKNOD] Errore recupero metadati dopo creazione: {}",
                                    e
                                );
                                // File creato ma metadati non disponibili - usa attributi base
                                let effective_perms = (mode & 0o777) & !(umask & 0o777);
                                let attr = new_file_attr(new_inode, 0, effective_perms);
                                let ttl = Duration::from_secs(1);
                                reply.entry(&ttl, &attr, 0);
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("❌ [MKNOD] Errore creazione file sul server: {}", e);
                        match e {
                            ClientError::NotFound { .. } => reply.error(libc::ENOENT),
                            _ => reply.error(libc::EIO),
                        }
                    }
                }
            }
            libc::S_IFIFO => {
                // NAMED PIPE/FIFO - Non supportato su filesystem remoto
                log::warn!("⚠️ [MKNOD] Named pipe non supportato: {}", full_path);
                reply.error(libc::EPERM);
            }
            libc::S_IFCHR => {
                // CHARACTER DEVICE - Non supportato su filesystem remoto
                log::warn!(
                    "⚠️ [MKNOD] Character device non supportato: {} (rdev: {})",
                    full_path,
                    rdev
                );
                reply.error(libc::EPERM);
            }
            libc::S_IFBLK => {
                // BLOCK DEVICE - Non supportato su filesystem remoto
                log::warn!(
                    "⚠️ [MKNOD] Block device non supportato: {} (rdev: {})",
                    full_path,
                    rdev
                );
                reply.error(libc::EPERM);
            }
            libc::S_IFSOCK => {
                // SOCKET - Non supportato su filesystem remoto
                log::warn!("⚠️ [MKNOD] Socket non supportato: {}", full_path);
                reply.error(libc::EPERM);
            }
            _ => {
                // TIPO SCONOSCIUTO
                log::error!("❌ [MKNOD] Tipo file sconosciuto: {:#o}", file_type);
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
    log::debug!(
        "📁 [MKDIR] parent: {}, name: {:?}, mode: {:#o}, umask: {:#o}",
        parent, name, mode, umask
    );
    
    // 1. VALIDAZIONE INPUT
    let dirname = match name.to_str() {
        Some(s) => s,
        None => {
            log::error!("❌ [MKDIR] Nome directory non valido: {:?}", name);
            reply.error(libc::EINVAL);
            return;
        }
    };
    
    // 2. OTTIENI PATH DELLA DIRECTORY PADRE
    let parent_path = match self.get_path(parent) {
        Some(p) => p.clone(),
        None => {
            log::error!("❌ [MKDIR] Directory padre con inode {} non trovata", parent);
            reply.error(libc::ENOENT);
            return;
        }
    };
    
    // 3. COSTRUISCI PATH COMPLETO
    let full_path = if parent_path == "/" {
        format!("/{}", dirname)
    } else {
        format!("{}/{}", parent_path, dirname)
    };
    
    log::debug!("📁 [MKDIR] Path completo: {}", full_path);
    
    // 4. VERIFICA CHE LA DIRECTORY NON ESISTA GIÀ
    if self.path_to_inode.contains_key(&full_path) {
        log::warn!("⚠️ [MKDIR] Directory già esistente: {}", full_path);
        reply.error(libc::EEXIST);
        return;
    }
    
    // 5. CALCOLA PERMESSI EFFETTIVI
    let effective_permissions = (mode & 0o777) & !(umask & 0o777);
    let permissions_octal = format!("{:o}", effective_permissions);
    
    log::debug!("🔒 [MKDIR] Permessi: mode={:#o}, umask={:#o}, effective={:#o}", 
               mode & 0o777, umask & 0o777, effective_permissions);
    
    // 6. CREA DIRECTORY SUL SERVER
    let rt = tokio::runtime::Handle::current();
    let create_result = rt.block_on(async {
        self.client.write_file(&WriteRequest { path: full_path.clone(), new_path: (None), data: (None), size: Some(4096),permissions_octal: Some(permissions_octal), last_modified: Some(chrono::Utc::now().to_rfc3339()), }).await
    });
    
    match create_result {
        Ok(()) => {
            log::debug!("✅ [MKDIR] Directory creata sul server con successo");
            
            // 7. GENERA NUOVO INODE E REGISTRA
            let new_inode = self.generate_inode();
            self.register_inode(new_inode, full_path.clone());
            
            // 8. OTTIENI METADATI DAL SERVER PER CONFERMA
            let metadata_result = rt.block_on(async {
                self.client.get_file_metadata(&full_path).await
            });
            
            match metadata_result {
                Ok(metadata) => {
                    // Usa metadati reali dal server
                    let attr = attributes::from_metadata(new_inode, &metadata);
                    let ttl = Duration::from_secs(1);
                    reply.entry(&ttl, &attr, 0);
                    
                    log::debug!("✅ [MKDIR] Entry restituita per inode {}", new_inode);
                }
                Err(e) => {
                    log::error!("❌ [MKDIR] Errore recupero metadati dopo creazione: {}", e);
                    // Directory creata ma metadati non disponibili - usa attributi base)
                    let attr = new_directory_attr(new_inode, effective_permissions);
                    let ttl = Duration::from_secs(1);
                    reply.entry(&ttl, &attr, 0);
                }
            }
        }
        Err(e) => {
            log::error!("❌ [MKDIR] Errore creazione directory sul server: {}", e);
            match e {
                ClientError::NotFound { .. } => reply.error(libc::ENOENT),
                _ => reply.error(libc::EIO),
            }
        }
    }
}

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        log::debug!(
            "[Not Implemented] unlink(parent: {:#x?}, name: {:?})",
            parent,
            name,
        );
        reply.error(libc::ENOSYS);
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        log::debug!(
            "[Not Implemented] rmdir(parent: {:#x?}, name: {:?})",
            parent,
            name,
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
            parent,
            name,
            link,
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
            parent,
            name,
            newparent,
            newname,
            flags,
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
            ino,
            newparent,
            newname
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
            ino,
            fh,
            offset,
            size,
            flags,
            lock_owner
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

    fn flush(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        lock_owner: u64,
        reply: fuser::ReplyEmpty,
    ) {
        log::debug!(
            "[Not Implemented] flush(ino: {:#x?}, fh: {}, lock_owner: {:?})",
            ino,
            fh,
            lock_owner
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

    fn fsync(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        datasync: bool,
        reply: fuser::ReplyEmpty,
    ) {
        log::debug!(
            "[Not Implemented] fsync(ino: {:#x?}, fh: {}, datasync: {})",
            ino,
            fh,
            datasync
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
            ino,
            fh,
            offset
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
            ino,
            fh,
            offset
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
            ino,
            fh,
            datasync
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
            ino,
            name,
            flags,
            position
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
            ino,
            name,
            size
        );
        reply.error(libc::ENOSYS);
    }

    fn listxattr(&mut self, _req: &Request<'_>, ino: u64, size: u32, reply: fuser::ReplyXattr) {
        log::debug!(
            "[Not Implemented] listxattr(ino: {:#x?}, size: {})",
            ino,
            size
        );
        reply.error(libc::ENOSYS);
    }

    fn removexattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        name: &OsStr,
        reply: fuser::ReplyEmpty,
    ) {
        log::debug!(
            "[Not Implemented] removexattr(ino: {:#x?}, name: {:?})",
            ino,
            name
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
            parent,
            name,
            mode,
            umask,
            flags
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
            ino,
            fh,
            lock_owner,
            start,
            end,
            typ,
            pid
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
            ino,
            fh,
            lock_owner,
            start,
            end,
            typ,
            pid,
            sleep
        );
        reply.error(libc::ENOSYS);
    }

    fn bmap(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        blocksize: u32,
        idx: u64,
        reply: fuser::ReplyBmap,
    ) {
        log::debug!(
            "[Not Implemented] bmap(ino: {:#x?}, blocksize: {}, idx: {})",
            ino,
            blocksize,
            idx,
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
        reply: fuser::ReplyLseek,
    ) {
        log::debug!(
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
            ino_in,
            fh_in,
            offset_in,
            ino_out,
            fh_out,
            offset_out,
            len,
            flags
        );
        reply.error(libc::ENOSYS);
    }
}
