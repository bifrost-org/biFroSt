use crate::api::client::{ ClientError, RemoteClient };
use crate::api::models::*;
use crate::fs::attributes::{ self, new_directory_attr, new_file_attr };
use fuser::{
    FileAttr,
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
use std::fs::metadata;
use std::time::{ Duration, SystemTime };

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
}

struct Permissions {
    owner: u32,
    group: u32,
    other: u32,
}

fn parse_permissions(perm_str: &str) -> Permissions {
    match u32::from_str_radix(perm_str, 8) {
        Ok(perms) =>
            Permissions {
                owner: (perms >> 6) & 0o7,
                group: (perms >> 3) & 0o7,
                other: perms & 0o7,
            },
        Err(_) =>
            Permissions {
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
    // Due write lock sempre in conflitto
    // Write lock e read lock sempre in conflitto
    // Due read lock mai in conflitto
    typ1 == libc::F_WRLCK || typ2 == libc::F_WRLCK
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
            open_dirs: HashMap::new(),
            file_locks: HashMap::new(),
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

    fn get_current_attributes(&mut self, ino: u64, path: &str, reply: ReplyAttr) {
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };

        // 1. OTTIENI METADATI FRESCHI DAL SERVER
        match rt.block_on(async { self.client.get_file_metadata(path).await }) {
            Ok(metadata) => {
                // 2. CONVERTI IN ATTRIBUTI FUSE
                let attr = attributes::from_metadata(ino, &metadata);
                let ttl = Duration::from_secs(1); // Cache TTL

                // 3. RESTITUISCI A FUSE
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
        _config: &mut fuser::KernelConfig
    ) -> Result<(), libc::c_int> {
        // 1. Configurazione parametri FUSE per filesystem remoto
        let _ = _config.set_max_write(1024 * 1024); // Buffer scrittura 1MB
        let _ = _config.set_max_readahead(1024 * 1024); // Buffer lettura anticipata 1MB

        // 2. Verifica connessione al server
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        match
            rt.block_on(async {
                // Verifica che il server sia raggiungibile
                match self.client.get_file_metadata("/").await {
                    Ok(_) => Ok(()),
                    Err(ClientError::NotFound { .. }) => Ok(()), // È ok se "/" non esiste come file
                    Err(e) => Err(e),
                }
            })
        {
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

    fn lookup(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        println!("LOOKUPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPP");

        let filename = match name.to_str() {
            Some(s) => s,
            None => {
                log::error!("❌ [LOOKUP] Nome file non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        println!("🔍 [LOOKUP] parent: {}, name: '{}', pid: {}", parent, filename, req.pid());

        // ✅ FILTRO COMANDI SHELL E AUTOCOMPLETE
        const SHELL_COMMANDS: &[&str] = &[
            // Comandi base Unix
            "ls",
            "cat",
            "touch",
            "echo",
            "cp",
            "mv",
            "rm",
            "mkdir",
            "rmdir",
            "grep",
            "find",
            "head",
            "tail",
            "less",
            "more",
            "vi",
            "vim",
            "nano",
            "bash",
            "sh",
            "zsh",
            "pwd",
            "cd",
            "which",
            "whereis",
            "file",
            "stat",
            "clear",
            "history",
            "exit",
            "logout",
            "su",
            "sudo",
            "chmod",
            "chown",
            "tar",
            "gzip",
            "gunzip",
            "unzip",
            "zip",
            "curl",
            "wget",
            "ssh",
            "scp", // Autocomplete comuni
            "Input",
            "Output",
            "input",
            "output",
            "test",
            "Test",
            "tmp",
            "Tmp",
            "bin",
            "usr",
            "etc",
            "var",
            "home",
            "root",
            "opt",
            "proc",
            "sys", // Comandi di sistema
            "ps",
            "top",
            "htop",
            "kill",
            "killall",
            "mount",
            "umount",
            "df",
            "du",
            "free",
            "uname",
            "whoami",
            "id",
            "groups",
            "date",
            "uptime",
            "w",
            "who",
            // Editor e viewer
            "emacs",
            "code",
            "subl",
            "atom",
            "gedit",
            "kate",
            "notepad",
            "view",
        ];

        if SHELL_COMMANDS.contains(&filename) {
            println!("⚠️ [LOOKUP] Comando/autocomplete shell '{}' - ENOENT", filename);
            reply.error(libc::ENOENT);
            return;
        }

        // ✅ GESTIONE DIRECTORY SPECIALI
        if filename == "." {
            println!("🔍 [LOOKUP] Directory corrente '.' richiesta");
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
                    // Fallback per directory corrente
                    let attr = attributes::new_directory_attr(parent, 0o755);
                    let ttl = Duration::from_secs(1);
                    reply.entry(&ttl, &attr, 0);
                    return;
                }
            }
        }

        if filename == ".." {
            println!("🔍 [LOOKUP] Directory padre '..' richiesta");
            let parent_attr = if parent == 1 {
                // Root directory - padre è se stessa
                attributes::new_directory_attr(1, 0o755)
            } else {
                // Calcola inode del padre
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

            let ttl = Duration::from_secs(1);
            reply.entry(&ttl, &parent_attr, 0);
            return;
        }

        // ✅ OTTIENI PATH PADRE
        let parent_path = match self.get_path(parent) {
            Some(path) => path.clone(),
            None => {
                log::error!("❌ [LOOKUP] Directory padre con inode {} non trovata", parent);
                reply.error(libc::ENOENT);
                return;
            }
        };

        // ✅ COSTRUISCI PATH COMPLETO
        let full_path = if parent_path == "/" {
            format!("/{}", filename)
        } else {
            format!("{}/{}", parent_path, filename)
        };

        println!("🔍 [LOOKUP] Path completo: '{}'", full_path);

        // ✅ VERIFICA CACHE LOCALE PRIMA
        if let Some(&existing_inode) = self.path_to_inode.get(&full_path) {
            println!("💾 [LOOKUP] File trovato in cache: inode {}", existing_inode);

            // Verifica che i metadati siano ancora validi (opzionale)
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
                    // File eliminato dal server - rimuovi dalla cache
                    println!("🗑️ [LOOKUP] File eliminato dal server, pulizia cache");
                    self.unregister_inode(existing_inode);
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(e) => {
                    log::error!("❌ [LOOKUP] Errore verifica cache: {}", e);
                    // Usa cache comunque se server non raggiungibile
                    let attr = attributes::new_file_attr(existing_inode, 0, 0o644);
                    let ttl = Duration::from_secs(1);
                    reply.entry(&ttl, &attr, 0);
                    return;
                }
            }
        }

        // ✅ NON IN CACHE - CHIEDI AL SERVER
        println!("🌐 [LOOKUP] File non in cache, interrogo server...");
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
                println!(
                    "✅ [LOOKUP] File trovato sul server: '{}' ({:?}, {} bytes)",
                    full_path,
                    metadata.kind,
                    metadata.size
                );

                // Genera nuovo inode e registra
                let new_inode = self.generate_inode();
                self.register_inode(new_inode, full_path.clone());

                // Converti metadati e restituisci
                let attr = attributes::from_metadata(new_inode, &metadata);
                let ttl = Duration::from_secs(1);
                reply.entry(&ttl, &attr, 0);

                println!("📝 [LOOKUP] Nuovo inode {} registrato per '{}'", new_inode, full_path);
            }
            Err(ClientError::NotFound { .. }) => {
                println!("❌ [LOOKUP] File '{}' non trovato sul server", full_path);
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                log::error!("❌ [LOOKUP] Permesso negato per: {}", full_path);
                reply.error(libc::EACCES);
            }
            Err(e) => {
                log::error!("❌ [LOOKUP] Errore server per '{}': {}", full_path, e);
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
        reply: ReplyAttr
    ) {
        log::debug!(
            "🔧 [SETATTR] ino: {}, mode: {:?}, uid: {:?}, gid: {:?}, size: {:?}, fh: {:?}, flags: {:?}",
            ino,
            mode,
            uid,
            gid,
            size,
            fh,
            flags
        );

        // 1. CONTROLLI PRELIMINARI

        // Directory root è read-only per operazioni di modifica strutturale
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

        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };

        // 2. OTTIENI METADATI ATTUALI
        let current_metadata = match
            rt.block_on(async { self.client.get_file_metadata(&path).await })
        {
            Ok(metadata) => metadata,
            Err(ClientError::NotFound { .. }) => {
                log::error!("❌ [SETATTR] File non trovato sul server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                log::error!("❌ [SETATTR] Errore recupero metadati per '{}': {}", path, e);
                reply.error(libc::EIO);
                return;
            }
        };

        log::debug!("🔍 [SETATTR] Metadati attuali recuperati per: {}", path);

        // 3. GESTIONE OPERAZIONI SUPPORTATE

        // A) TRUNCATE/RESIZE (modifica dimensione file)
        if let Some(new_size) = size {
            log::debug!("📏 [SETATTR] Richiesta modifica dimensione a {} bytes", new_size);

            // Verifica che sia un file regolare (non directory)
            match current_metadata.kind {
                FileKind::Directory => {
                    log::warn!("⚠️ [SETATTR] Tentativo di truncate su directory: {}", path);
                    reply.error(libc::EISDIR);
                    return;
                }
                _ => {
                    // Continua con la logica di truncate
                }
            }

            let current_size = current_metadata.size;
            log::debug!("📏 [SETATTR] Dimensione attuale: {} → nuova: {}", current_size, new_size);

            // Se è la stessa dimensione, non fare nulla
            if new_size == current_size {
                log::debug!("✅ [SETATTR] Dimensione già corretta, nessuna modifica necessaria");
                self.get_current_attributes(ino, &path, reply);
                return;
            }

            let now_iso = chrono::Utc::now().to_rfc3339();

            // Determina operazione ed esegui
            let operation_result = if new_size < current_size {
                // TRUNCATE (riduzione)
                log::debug!("✂️ [SETATTR] Operazione: TRUNCATE (riduzione)");
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
                // EXTEND (espansione)
                log::debug!("📈 [SETATTR] Operazione: EXTEND (espansione)");
                let padding_size = new_size - current_size;
                let padding_data = vec![0u8; padding_size as usize];

                rt.block_on(async { self.client.write_file(
                        &(WriteRequest {
                            offset: None,
                            path: path.clone(),
                            new_path: None,
                            size: padding_size, // ← Size del padding da aggiungere
                            atime: now_iso.clone(),
                            mtime: now_iso.clone(),
                            ctime: now_iso.clone(),
                            crtime: current_metadata.crtime.clone(),
                            kind: current_metadata.kind,
                            ref_path: None,
                            perm: current_metadata.perm.clone(),
                            mode: Mode::Append, // ← Append i null bytes alla fine
                            data: Some(padding_data),
                        })
                    ).await })
            };

            // ✅ GESTISCI IL RISULTATO DELL'OPERAZIONE
            match operation_result {
                Ok(()) => {
                    log::debug!(
                        "✅ [SETATTR] Dimensione modificata con successo a {} bytes",
                        new_size
                    );
                    self.get_current_attributes(ino, &path, reply);
                }
                Err(e) => {
                    log::error!("❌ [SETATTR] Errore modifica dimensione: {}", e);
                    let error_code = match e {
                        ClientError::NotFound { .. } => libc::ENOENT,
                        ClientError::PermissionDenied(_) => libc::EPERM,
                        ClientError::Server { status: 413, .. } => libc::EFBIG, // File troppo grande
                        ClientError::Server { status: 507, .. } => libc::ENOSPC, // Spazio insufficiente
                        _ => libc::EIO,
                    };
                    reply.error(error_code);
                }
            }
            return;
        }

        // B) CHMOD (cambio permessi)
        if let Some(new_mode) = mode {
            log::debug!("🔒 [SETATTR] Richiesta modifica permessi: {:o}", new_mode & 0o777);

            let new_permissions = format!("{:o}", new_mode & 0o777);
            let now_iso = chrono::Utc::now().to_rfc3339();

            let chmod_request = WriteRequest {
                offset: None,
                path: path.clone(),
                new_path: None,
                size: current_metadata.size, // Mantieni dimensione
                atime: current_metadata.atime.clone(), // Mantieni access time
                mtime: current_metadata.mtime.clone(), // Mantieni modification time
                ctime: now_iso, // Aggiorna change time (metadati cambiati)
                crtime: current_metadata.crtime.clone(), // Mantieni creation time
                kind: current_metadata.kind, // Mantieni tipo file
                ref_path: None,
                perm: new_permissions, // Nuovi permessi
                mode: Mode::Write, // Modalità metadata-only
                data: None, // Nessun contenuto, solo metadati
            };

            match rt.block_on(async { self.client.write_file(&chmod_request).await }) {
                Ok(()) => {
                    log::debug!(
                        "✅ [SETATTR] Permessi modificati con successo: {:o}",
                        new_mode & 0o777
                    );
                    self.get_current_attributes(ino, &path, reply);
                }
                Err(e) => {
                    log::error!("❌ [SETATTR] Errore modifica permessi: {}", e);
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

        // C) CHOWN (cambio proprietario) - NON SUPPORTATO su filesystem remoto
        if uid.is_some() || gid.is_some() {
            log::warn!("⚠️ [SETATTR] Cambio uid/gid non supportato su filesystem remoto");
            reply.error(libc::EPERM);
            return;
        }

        // D) TOUCH (modifica timestamp) - IMPLEMENTAZIONE FUTURA
        // Per ora ignoriamo _atime, _mtime, _ctime perché richiedono conversioni complesse
        // da fuser::TimeOrNow a timestamp ISO8601

        // E) FLAGS - NON SUPPORTATO
        if flags.is_some() {
            log::warn!("⚠️ [SETATTR] Cambio flags non supportato");
            reply.error(libc::ENOSYS);
            return;
        }

        // 4. NESSUNA MODIFICA RICONOSCIUTA - RESTITUISCI ATTRIBUTI ATTUALI
        log::debug!("📋 [SETATTR] Nessuna modifica richiesta, restituendo attributi attuali");
        self.get_current_attributes(ino, &path, reply);
    }

fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {
    log::debug!("🔗 [READLINK] ino: {}", ino);

    let path = match self.inode_to_path.get(&ino) {
        Some(p) => p.clone(),
        None => {
            log::error!("❌ [READLINK] Inode {} non trovato", ino);
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
                    log::debug!("🔗 [READLINK] Target originale: '{}'", target);
                    
                    // ✅ FIX: Converti path assoluti in relativi
                    let resolved_target = if target.starts_with('/') {
                        // Path assoluto → rimuovi la / iniziale per renderlo relativo
                        let relative_target = &target[1..];
                        log::debug!("🔗 [READLINK] Convertito path assoluto '{}' in relativo '{}'", target, relative_target);
                        relative_target
                    } else {
                        // Path già relativo → usa così com'è
                        target.as_str()
                    };
                    
                    log::debug!("✅ [READLINK] Target finale: '{}'", resolved_target);
                    reply.data(resolved_target.as_bytes());
                }
                (FileKind::Symlink, _) => {
                    log::error!("❌ [READLINK] Symlink senza target valido: {}", path);
                    reply.error(libc::EIO);
                }
                (file_type, _) => {
                    log::warn!("⚠️ [READLINK] '{}' non è un symlink: {:?}", path, file_type);
                    reply.error(libc::EINVAL);
                }
            }
        }
        Err(ClientError::NotFound { .. }) => {
            log::error!("❌ [READLINK] File non trovato: {}", path);
            reply.error(libc::ENOENT);
        }
        Err(e) => {
            log::error!("❌ [READLINK] Errore server: {}", e);
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
                log::error!("❌ [MKNOD] Directory padre con inode {} non trovata", parent);
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
                    size: 0, // ✅ NON Some(0)
                    atime: chrono::Utc::now().to_rfc3339(),
                    mtime: chrono::Utc::now().to_rfc3339(),
                    ctime: chrono::Utc::now().to_rfc3339(),
                    crtime: chrono::Utc::now().to_rfc3339(),
                    kind: FileKind::RegularFile, // ✅ Specifica tipo file
                    ref_path: None, // ✅ Non è un link
                    perm: (mode & 0o777 & !(umask & 0o777)).to_string(), // ✅ NON permissions_octal
                    mode: Mode::Write, // ✅ Aggiungi mode
                    data: Some(Vec::new()), // ✅ File vuoto
                };

                let create_result = rt.block_on(async {
                    self.client.write_file(&write_request).await
                });

                // 6. GESTISCI RISULTATO CREAZIONE
                match create_result {
                    Ok(()) => {
                        log::debug!("✅ [MKNOD] File creato sul server con successo");

                        // Genera nuovo inode e registra
                        let new_inode = self.generate_inode();
                        self.register_inode(new_inode, full_path.clone());

                        // Ottieni metadati dal server per conferma
                        let metadata_result = rt.block_on(async {
                            self.client.get_file_metadata(&full_path).await
                        });

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
                                let effective_perms = mode & 0o777 & !(umask & 0o777);
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
        reply: ReplyEntry
    ) {
        println!("MKDIRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR");
        log::debug!(
            "📁 [MKDIR] parent: {}, name: {:?}, mode: {:#o}, umask: {:#o}",
            parent,
            name,
            mode,
            umask
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
        let effective_permissions = mode & 0o777 & !(umask & 0o777);
        let permissions_octal = format!("{:o}", effective_permissions);

        log::debug!(
            "🔒 [MKDIR] Permessi: mode={:#o}, umask={:#o}, effective={:#o}",
            mode & 0o777,
            umask & 0o777,
            effective_permissions
        );

        // 6. CREA DIRECTORY SUL SERVER
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        println!("Credo la directory: {}", full_path);
        let create_result = rt.block_on(async { self.client.create_directory(&full_path).await });

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
        log::debug!("🗑️ [UNLINK] parent: {}, name: {:?}", parent, name);

        // 1. VALIDAZIONE INPUT
        let filename = match name.to_str() {
            Some(s) => s,
            None => {
                log::error!("❌ [UNLINK] Nome file non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        // 2. OTTIENI PATH DELLA DIRECTORY PADRE
        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                log::error!("❌ [UNLINK] Directory padre con inode {} non trovata", parent);
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

        log::debug!("🗑️ [UNLINK] Path completo: {}", full_path);

        // 4. VERIFICA CHE IL FILE ESISTA NELLA CACHE
        let file_inode = match self.path_to_inode.get(&full_path) {
            Some(&inode) => inode,
            None => {
                log::warn!("⚠️ [UNLINK] File non trovato nella cache: {}", full_path);
                // Potrebbe esistere sul server ma non in cache - verifica
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
                        log::debug!("📝 [UNLINK] File esiste sul server ma non in cache");
                        // Continua con eliminazione senza inode locale
                    }
                    Err(ClientError::NotFound { .. }) => {
                        reply.error(libc::ENOENT);
                        return;
                    }
                    Err(e) => {
                        log::error!("❌ [UNLINK] Errore verifica esistenza: {}", e);
                        reply.error(libc::EIO);
                        return;
                    }
                }
                0 // Placeholder, file non in cache locale
            }
        };

        // 5. VERIFICA CHE SIA UN FILE (NON DIRECTORY)
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
                        log::warn!("⚠️ [UNLINK] Tentativo di unlink su directory: {}", full_path);
                        reply.error(libc::EISDIR);
                        return;
                    }
                }
                Err(ClientError::NotFound { .. }) => {
                    reply.error(libc::ENOENT);
                    return;
                }
                Err(e) => {
                    log::error!("❌ [UNLINK] Errore verifica tipo file: {}", e);
                    reply.error(libc::EIO);
                    return;
                }
            }
        }

        // 6. VERIFICA CHE IL FILE NON SIA APERTO
        let is_file_open = self.open_files.values().any(|open_file| open_file.path == full_path);
        if is_file_open {
            log::warn!("⚠️ [UNLINK] File ancora aperto: {}", full_path);
            // Su Unix, il file viene eliminato ma rimane accessibile ai processi che lo hanno aperto
            // Per semplicità, blocchiamo l'operazione
            reply.error(libc::EBUSY);
            return;
        }

        // 7. ELIMINA FILE DAL SERVER
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
                log::debug!("✅ [UNLINK] File eliminato dal server con successo");

                // 8. RIMUOVI DALLA CACHE LOCALE
                if file_inode != 0 {
                    self.unregister_inode(file_inode);
                    log::debug!("🗑️ [UNLINK] Inode {} rimosso dalla cache", file_inode);
                }

                reply.ok();
                log::debug!("✅ [UNLINK] Operazione completata per: {}", full_path);
            }
            Err(ClientError::NotFound { .. }) => {
                log::warn!("⚠️ [UNLINK] File già eliminato dal server: {}", full_path);
                // Rimuovi comunque dalla cache locale se presente
                if file_inode != 0 {
                    self.unregister_inode(file_inode);
                }
                reply.ok(); // Su Unix, eliminare un file già eliminato non è un errore
            }
            Err(e) => {
                log::error!("❌ [UNLINK] Errore eliminazione dal server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: fuser::ReplyEmpty) {
        log::debug!("🗂️ [RMDIR] parent: {}, name: {:?}", parent, name);

        // 1. VALIDAZIONE INPUT
        let dirname = match name.to_str() {
            Some(s) => s,
            None => {
                log::error!("❌ [RMDIR] Nome directory non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        // 2. PROTEZIONE DIRECTORY SPECIALI
        if dirname == "." || dirname == ".." {
            log::warn!("⚠️ [RMDIR] Tentativo di eliminare directory speciale: {}", dirname);
            reply.error(libc::EINVAL);
            return;
        }

        // 3. OTTIENI PATH DELLA DIRECTORY PADRE
        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                log::error!("❌ [RMDIR] Directory padre con inode {} non trovata", parent);
                reply.error(libc::ENOENT);
                return;
            }
        };

        // 4. COSTRUISCI PATH COMPLETO
        let full_path = if parent_path == "/" {
            format!("/{}", dirname)
        } else {
            format!("{}/{}", parent_path, dirname)
        };

        log::debug!("🗂️ [RMDIR] Path completo: {}", full_path);

        // 5. PROTEZIONE DIRECTORY ROOT
        if full_path == "/" {
            log::warn!("⚠️ [RMDIR] Tentativo di eliminare directory root");
            reply.error(libc::EBUSY);
            return;
        }

        // 6. VERIFICA CHE LA DIRECTORY ESISTA NELLA CACHE
        let dir_inode = match self.path_to_inode.get(&full_path) {
            Some(&inode) => inode,
            None => {
                log::warn!("⚠️ [RMDIR] Directory non trovata nella cache: {}", full_path);
                // Potrebbe esistere sul server ma non in cache - verifica
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
                            log::warn!("⚠️ [RMDIR] '{}' non è una directory", full_path);
                            reply.error(libc::ENOTDIR);
                            return;
                        }
                        log::debug!("📝 [RMDIR] Directory esiste sul server ma non in cache");
                        // Continua con eliminazione senza inode locale
                    }
                    Err(ClientError::NotFound { .. }) => {
                        reply.error(libc::ENOENT);
                        return;
                    }
                    Err(e) => {
                        log::error!("❌ [RMDIR] Errore verifica esistenza: {}", e);
                        reply.error(libc::EIO);
                        return;
                    }
                }
                0 // Placeholder, directory non in cache locale
            }
        };

        // 7. VERIFICA CHE SIA UNA DIRECTORY (NON FILE)
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
                    log::error!("❌ [RMDIR] Errore verifica tipo directory: {}", e);
                    reply.error(libc::EIO);
                    return;
                }
            }
        }

        // 8. VERIFICA CHE LA DIRECTORY SIA VUOTA
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
            Err(ClientError::NotFound { .. }) => {
                // Directory già inesistente - ok per rmdir
                log::debug!("📝 [RMDIR] Directory già inesistente sul server");
            }
            Err(e) => {
                log::error!("❌ [RMDIR] Errore verifica directory vuota: {}", e);
                reply.error(libc::EIO);
                return;
            }
        }

        // 9. ELIMINA DIRECTORY DAL SERVER
        let delete_result = rt.block_on(async { self.client.delete(&full_path).await });

        match delete_result {
            Ok(()) => {
                log::debug!("✅ [RMDIR] Directory eliminata dal server con successo");

                // 10. RIMUOVI DALLA CACHE LOCALE
                if dir_inode != 0 {
                    self.unregister_inode(dir_inode);
                    log::debug!("🗂️ [RMDIR] Inode {} rimosso dalla cache", dir_inode);
                }

                reply.ok();
                log::debug!("✅ [RMDIR] Operazione completata per: {}", full_path);
            }
            Err(ClientError::NotFound { .. }) => {
                log::warn!("⚠️ [RMDIR] Directory già eliminata dal server: {}", full_path);
                // Rimuovi comunque dalla cache locale se presente
                if dir_inode != 0 {
                    self.unregister_inode(dir_inode);
                }
                reply.ok(); // Su Unix, eliminare una directory già eliminata non è un errore
            }
            Err(e) => {
                log::error!("❌ [RMDIR] Errore eliminazione dal server: {}", e);
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
        log::debug!("🔗 [SYMLINK] parent: {}, name: {:?}, link: {:?}", parent, name, link);

        // 1. VALIDAZIONE INPUT
        let link_name = match name.to_str() {
            Some(s) => s,
            None => {
                log::error!("❌ [SYMLINK] Nome symlink non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        let target_path = match link.to_str() {
            Some(s) => s,
            None => {
                log::error!("❌ [SYMLINK] Path target non valido: {:?}", link);
                reply.error(libc::EINVAL);
                return;
            }
        };

        // 2. OTTIENI PATH DELLA DIRECTORY PADRE
        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                log::error!("❌ [SYMLINK] Directory padre con inode {} non trovata", parent);
                reply.error(libc::ENOENT);
                return;
            }
        };

        // 3. COSTRUISCI PATH COMPLETO DEL SYMLINK
        let symlink_path = if parent_path == "/" {
            format!("/{}", link_name)
        } else {
            format!("{}/{}", parent_path, link_name)
        };

        log::debug!("🔗 [SYMLINK] Creando symlink: '{}' → '{}'", symlink_path, target_path);

        // 4. VERIFICA CHE IL SYMLINK NON ESISTA GIÀ
        if self.path_to_inode.contains_key(&symlink_path) {
            log::warn!("⚠️ [SYMLINK] Symlink già esistente: {}", symlink_path);
            reply.error(libc::EEXIST);
            return;
        }

        // 5. CREA SYMLINK SUL SERVER
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        let now_iso = chrono::Utc::now().to_rfc3339();
        //non ricordo se è corretto
        let symlink_request = WriteRequest {
            offset: None,
            path: symlink_path.clone(),
            new_path: None,
            size: target_path.len() as u64,
            atime: now_iso.clone(),
            mtime: now_iso.clone(),
            ctime: now_iso.clone(),
            crtime: now_iso,
            kind: FileKind::Symlink,
            ref_path: Some(target_path.to_string()), // ← Target del symlink
            perm: "777".to_string(), // Symlink hanno sempre permessi 777
            mode: Mode::Write,
            data: None, // Target come contenuto
        };

        match rt.block_on(async { self.client.write_file(&symlink_request).await }) {
            Ok(()) => {
                log::debug!("✅ [SYMLINK] Symlink creato sul server con successo");

                // 6. GENERA NUOVO INODE E REGISTRA
                let new_inode = self.generate_inode();
                self.register_inode(new_inode, symlink_path.clone());

                // 7. OTTIENI METADATI DAL SERVER PER CONFERMA
                let metadata_result = rt.block_on(async {
                    self.client.get_file_metadata(&symlink_path).await
                });

                match metadata_result {
                    Ok(metadata) => {
                        // Usa metadati reali dal server
                        let attr = attributes::from_metadata(new_inode, &metadata);
                        let ttl = Duration::from_secs(1);
                        reply.entry(&ttl, &attr, 0);

                        log::debug!("✅ [SYMLINK] Entry restituita per inode {}", new_inode);
                    }
                    Err(e) => {
                        reply.error(libc::EIO);
                    }
                }
            }
            Err(e) => {
                log::error!("❌ [SYMLINK] Errore creazione symlink sul server: {}", e);
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
        log::debug!(
            "📝 [RENAME] parent: {}, name: {:?}, newparent: {}, newname: {:?}, flags: {}",
            parent,
            name,
            newparent,
            newname,
            flags
        );

        // 1. VALIDAZIONE INPUT
        let old_filename = match name.to_str() {
            Some(s) => s,
            None => {
                log::error!("❌ [RENAME] Nome file originale non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        let new_filename = match newname.to_str() {
            Some(s) => s,
            None => {
                log::error!("❌ [RENAME] Nuovo nome file non valido: {:?}", newname);
                reply.error(libc::EINVAL);
                return;
            }
        };

        // 2. GESTIONE FLAGS (per ora ignoriamo, ma logghiamo)
        if flags != 0 {
            log::warn!("⚠️ [RENAME] Flags non supportati: {}, procedendo comunque", flags);
        }

        // 3. OTTIENI PATH DELLA DIRECTORY PADRE ORIGINALE
        let old_parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                log::error!(
                    "❌ [RENAME] Directory padre originale con inode {} non trovata",
                    parent
                );
                reply.error(libc::ENOENT);
                return;
            }
        };

        // 4. OTTIENI PATH DELLA NUOVA DIRECTORY PADRE
        let new_parent_path = match self.get_path(newparent) {
            Some(p) => p.clone(),
            None => {
                log::error!(
                    "❌ [RENAME] Nuova directory padre con inode {} non trovata",
                    newparent
                );
                reply.error(libc::ENOENT);
                return;
            }
        };

        // 5. COSTRUISCI PATH COMPLETI
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

        log::debug!("📝 [RENAME] Da: '{}' → A: '{}'", old_path, new_path);

        // 6. PROTEZIONI SPECIALI
        if old_path == "/" {
            log::warn!("⚠️ [RENAME] Tentativo di rinominare directory root");
            reply.error(libc::EBUSY);
            return;
        }

        if
            old_filename == "." ||
            old_filename == ".." ||
            new_filename == "." ||
            new_filename == ".."
        {
            log::warn!("⚠️ [RENAME] Tentativo di rinominare directory speciali");
            reply.error(libc::EINVAL);
            return;
        }

        if old_path == new_path {
            log::debug!("📝 [RENAME] Source e destination identici, operazione completata");
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

        // 7. OTTIENI METADATI DEL FILE ORIGINALE
        let old_metadata = match
            rt.block_on(async { self.client.get_file_metadata(&old_path).await })
        {
            Ok(metadata) => metadata,
            Err(ClientError::NotFound { .. }) => {
                log::error!("❌ [RENAME] File originale non trovato: {}", old_path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                log::error!("❌ [RENAME] Errore verifica file originale: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        // 8. VERIFICA CHE IL FILE NON SIA APERTO
        let file_inode = self.path_to_inode.get(&old_path).copied().unwrap_or(0);
        if file_inode != 0 {
            let is_file_open = self.open_files.values().any(|open_file| open_file.path == old_path);
            if is_file_open {
                log::warn!("⚠️ [RENAME] File ancora aperto: {}", old_path);
                reply.error(libc::EBUSY);
                return;
            }
        }

        // 9. VERIFICA DESTINAZIONE (se esiste, deve essere compatibile)
        if
            let Ok(new_metadata) = rt.block_on(async {
                self.client.get_file_metadata(&new_path).await
            })
        {
            log::debug!("📝 [RENAME] Destinazione esiste, verificando sovrascrittura");

            // Verifica compatibilità dei tipi
            if old_metadata.kind != new_metadata.kind {
                if old_metadata.kind == FileKind::Directory {
                    // Tentativo di sovrascrivere file con directory
                    reply.error(libc::ENOTDIR);
                } else {
                    // Tentativo di sovrascrivere directory con file
                    reply.error(libc::EISDIR);
                }
                return;
            }

            // Se è una directory, deve essere vuota
            if new_metadata.kind == FileKind::Directory {
                match rt.block_on(async { self.client.list_directory(&new_path).await }) {
                    Ok(listing) => {
                        if !listing.files.is_empty() {
                            log::warn!(
                                "⚠️ [RENAME] Directory destinazione non vuota: {}",
                                new_path
                            );
                            reply.error(libc::ENOTEMPTY);
                            return;
                        }
                    }
                    Err(e) => {
                        log::error!("❌ [RENAME] Errore verifica directory vuota: {}", e);
                        reply.error(libc::EIO);
                        return;
                    }
                }
            }
        }

        // 10. ESEGUI RENAME SUL SERVER
        let now_iso = chrono::Utc::now().to_rfc3339();
        let rename_request = WriteRequest {
            offset: None,
            path: old_path.clone(),
            new_path: Some(new_path.clone()),
            size: old_metadata.size, // ✅ Mantieni dimensione originale
            atime: old_metadata.atime.clone(), // ✅ Mantieni access time
            mtime: old_metadata.mtime.clone(), // ✅ Mantieni modification time
            ctime: now_iso.clone(), // ✅ Aggiorna change time
            crtime: old_metadata.crtime.clone(), // ✅ Mantieni creation time
            kind: old_metadata.kind, // ✅ Mantieni tipo file
            ref_path: None, // ✅ Non è symlink operation
            perm: old_metadata.perm.clone(), // ✅ Mantieni permessi
            mode: Mode::Write, // ✅ Specifica operazione rename
            data: None, // ✅ Nessun dato da trasferire
        };

        let rename_result = rt.block_on(async { self.client.write_file(&rename_request).await });

        match rename_result {
            Ok(()) => {
                log::debug!("✅ [RENAME] Rename sul server completato con successo");

                // 11. AGGIORNA CACHE LOCALE
                if file_inode != 0 {
                    // Rimuovi vecchia mappatura
                    self.inode_to_path.remove(&file_inode);
                    self.path_to_inode.remove(&old_path);

                    // Se destinazione esisteva, rimuovi anche quella
                    if let Some(&dest_inode) = self.path_to_inode.get(&new_path) {
                        if dest_inode != file_inode {
                            self.unregister_inode(dest_inode);
                        }
                    }

                    // Aggiungi nuova mappatura
                    self.inode_to_path.insert(file_inode, new_path.clone());
                    self.path_to_inode.insert(new_path.clone(), file_inode);

                    log::debug!(
                        "🔄 [RENAME] Cache aggiornata: inode {} da '{}' a '{}'",
                        file_inode,
                        old_path,
                        new_path
                    );
                }

                reply.ok();
                log::debug!("✅ [RENAME] Operazione completata: '{}' → '{}'", old_path, new_path);
            }
            Err(ClientError::NotFound { .. }) => {
                log::error!("❌ [RENAME] File originale non trovato sul server: {}", old_path);
                reply.error(libc::ENOENT);
            }
            Err(e) => {
                log::error!("❌ [RENAME] Errore rename sul server: {}", e);
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
        log::debug!("🔗 [LINK] ino: {}, newparent: {}, newname: {:?}", ino, newparent, newname);

        // 1. VALIDAZIONE INPUT
        let link_name = match newname.to_str() {
            Some(s) => s,
            None => {
                log::error!("❌ [LINK] Nome hard link non valido: {:?}", newname);
                reply.error(libc::EINVAL);
                return;
            }
        };

        // 2. OTTIENI PATH DEL FILE SORGENTE
        let source_path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                log::error!("❌ [LINK] Inode sorgente {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };

        // 3. OTTIENI PATH DELLA DIRECTORY PADRE DESTINAZIONE
        let parent_path = match self.get_path(newparent) {
            Some(p) => p.clone(),
            None => {
                log::error!("❌ [LINK] Directory padre con inode {} non trovata", newparent);
                reply.error(libc::ENOENT);
                return;
            }
        };

        // 4. COSTRUISCI PATH COMPLETO DEL NUOVO HARD LINK
        let link_path = if parent_path == "/" {
            format!("/{}", link_name)
        } else {
            format!("{}/{}", parent_path, link_name)
        };

        log::debug!("🔗 [LINK] Creando hard link: '{}' → '{}'", link_path, source_path);

        // 5. VERIFICA CHE IL LINK NON ESISTA GIÀ
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

        // 6. OTTIENI METADATI DEL FILE SORGENTE
        let source_metadata = match
            rt.block_on(async { self.client.get_file_metadata(&source_path).await })
        {
            Ok(metadata) => metadata,
            Err(ClientError::NotFound { .. }) => {
                log::error!("❌ [LINK] File sorgente non trovato: {}", source_path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                log::error!("❌ [LINK] Errore verifica file sorgente: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        // 7. VERIFICA CHE SIA UN FILE REGOLARE (NON DIRECTORY O SYMLINK)
        match source_metadata.kind {
            FileKind::RegularFile => {
                log::debug!("✅ [LINK] File sorgente è un file regolare");
            }
            FileKind::Directory => {
                log::warn!("⚠️ [LINK] Impossibile creare hard link su directory: {}", source_path);
                reply.error(libc::EPERM);
                return;
            }
            FileKind::Symlink => {
                log::warn!("⚠️ [LINK] Hard link su symlink non supportato: {}", source_path);
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

        // 8. CREA HARD LINK SUL SERVER
        let now_iso = chrono::Utc::now().to_rfc3339();

        let link_request = WriteRequest {
            offset: None,
            path: link_path.clone(),
            new_path: None,
            size: source_metadata.size, // ✅ Stessa dimensione del file originale
            atime: source_metadata.atime.clone(), // ✅ Mantieni access time
            mtime: source_metadata.mtime.clone(), // ✅ Mantieni modification time
            ctime: now_iso.clone(), // ✅ Aggiorna change time (nuovo link)
            crtime: source_metadata.crtime.clone(), // ✅ Mantieni creation time
            kind: FileKind::Hardlink, // ✅ Stesso tipo file
            ref_path: Some(source_path.clone()), // ✅ Riferimento al file originale
            perm: source_metadata.perm.clone(), // ✅ Stessi permessi
            mode: Mode::Write, // ✅ Modalità hard link
            data: None, // ✅ Nessun contenuto, solo link
        };

        match rt.block_on(async { self.client.write_file(&link_request).await }) {
            Ok(()) => {
                log::debug!("✅ [LINK] Hard link creato sul server con successo");

                // 9. REGISTRA STESSO INODE PER IL NUOVO PATH
                // Hard link condivide lo stesso inode del file originale
                self.inode_to_path.insert(ino, link_path.clone()); // ✅ Aggiorna mapping inode -> path più recente
                self.path_to_inode.insert(link_path.clone(), ino); // ✅ Aggiungi nuovo path -> inode

                log::debug!("🔗 [LINK] Inode {} ora mappato anche a '{}'", ino, link_path);

                // 10. OTTIENI METADATI AGGIORNATI DAL SERVER
                let updated_metadata = match
                    rt.block_on(async { self.client.get_file_metadata(&link_path).await })
                {
                    Ok(metadata) => metadata,
                    Err(e) => {
                        log::error!("❌ [LINK] Errore recupero metadati dopo creazione: {}", e);
                        // Hard link creato ma usa metadati originali
                        source_metadata
                    }
                };

                // 11. RESTITUISCI ENTRY CON STESSO INODE
                let attr = attributes::from_metadata(ino, &updated_metadata);
                let ttl = Duration::from_secs(1);
                reply.entry(&ttl, &attr, 0);

                log::debug!("✅ [LINK] Entry restituita per inode {} (hard link)", ino);
            }
            Err(ClientError::NotFound { .. }) => {
                log::error!(
                    "❌ [LINK] File sorgente non trovato durante creazione: {}",
                    source_path
                );
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                log::error!("❌ [LINK] Permesso negato per creazione hard link");
                reply.error(libc::EPERM);
            }
            Err(e) => {
                log::error!("❌ [LINK] Errore creazione hard link sul server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    //sistemare solo quando ricevo l'errore che non posso perchè non ho l'autorizazzione
    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        println!("📂 [OPEN] INIZIO: ino={}, flags={:#x}", ino, flags);

        // 1. VALIDAZIONE INODE
        let path = match self.inode_to_path.get(&ino) {
            Some(p) => {
                println!("📂 [OPEN] PATH TROVATO: {}", p);
                p.clone()
            }
            None => {
                println!("❌ [OPEN] INODE {} NON TROVATO", ino);
                log::error!("❌ [OPEN] Inode {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };

        println!("📂 [OPEN] PRIMA DI GET_METADATA");

        // 2. VERIFICA ESISTENZA E TIPO FILE SUL SERVER
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                println!("📂 [OPEN] RUNTIME HANDLE OK");
                handle
            }
            Err(_) => {
                println!("📂 [OPEN] CREANDO NUOVO RUNTIME");
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };

        println!("📂 [OPEN] CHIAMANDO BLOCK_ON...");

        let metadata_result = rt.block_on(async {
            println!("📂 [OPEN] DENTRO ASYNC BLOCK");
            let result = self.client.get_file_metadata(&path).await;
            println!("📂 [OPEN] METADATA RESULT: {:?}", result.is_ok());
            result
        });

        println!("📂 [OPEN] DOPO BLOCK_ON");

        let metadata = match metadata_result {
            Ok(metadata) => {
                println!("📂 [OPEN] METADATA OK: {:?}", metadata.kind);
                metadata
            }
            Err(ClientError::NotFound { .. }) => {
                println!("❌ [OPEN] FILE NON TROVATO: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                println!("❌ [OPEN] ERRORE METADATA: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        // Nella funzione open, dopo "METADATA OK"

        println!("📂 [OPEN] METADATA OK: {:?}", metadata.kind);

        // 3. VERIFICA TIPO FILE
        match metadata.kind {
            FileKind::RegularFile => {
                println!("📂 [OPEN] File regolare OK");
            }
            FileKind::Symlink => {
                println!("🔗 [OPEN] Symlink - seguirò il target");
                // Per i symlink, il kernel dovrebbe aver già fatto readlink e lookup del target
                // Ma permettiamo l'apertura diretta
            }
            FileKind::Directory => {
                println!("❌ [OPEN] È una directory");
                reply.error(libc::EISDIR);
                return;
            }
            _ => {
                println!("❌ [OPEN] Tipo file non supportato: {:?}", metadata.kind);
                reply.error(libc::EPERM);
                return;
            }
        }

        println!("📂 [OPEN] TIPO FILE OK");

        // 4. ANALISI FLAGS
        let access_mode = flags & libc::O_ACCMODE;
        let open_flags = flags & !libc::O_ACCMODE;

        println!("📂 [OPEN] ACCESS_MODE: {:#x}", access_mode);
        println!("📂 [OPEN] OPEN_FLAGS: {:#x}", open_flags);

        match access_mode {
            libc::O_RDONLY => println!("📂 [OPEN] MODALITÀ: READ_ONLY"),
            libc::O_WRONLY => println!("📂 [OPEN] MODALITÀ: WRITE_ONLY"),
            libc::O_RDWR => println!("📂 [OPEN] MODALITÀ: READ_WRITE"),
            _ => println!("📂 [OPEN] MODALITÀ: UNKNOWN ({:#x})", access_mode),
        }

        if (open_flags & libc::O_APPEND) != 0 {
            println!("📂 [OPEN] FLAG: O_APPEND RILEVATO");
        }
        if (open_flags & libc::O_CREAT) != 0 {
            println!("📂 [OPEN] FLAG: O_CREAT RILEVATO");
        }
        if (open_flags & libc::O_TRUNC) != 0 {
            println!("📂 [OPEN] FLAG: O_TRUNC RILEVATO");
        }

        println!("📂 [OPEN] PRIMA VERIFICA PERMESSI");

        // 5. VALIDAZIONE PERMESSI DI ACCESSO
        let perms = parse_permissions(&metadata.perm);
        println!("📂 [OPEN] PERMESSI PARSATI: owner={:#o}", perms.owner);

        let effective_perms = perms.owner; // Assumiamo owner per semplicità

        match access_mode {
            libc::O_RDONLY => {
                println!("📖 [OPEN] Verifica permesso lettura...");
                if (effective_perms & 0o4) == 0 {
                    // ✅ FIX: 0o4 invece di 0o400
                    println!("❌ [OPEN] Permesso di lettura negato");
                    reply.error(libc::EACCES);
                    return;
                }
                println!("✅ [OPEN] Permesso lettura OK");
            }
            libc::O_WRONLY => {
                println!("✏️ [OPEN] Verifica permesso scrittura...");
                if (effective_perms & 0o2) == 0 {
                    // ✅ FIX: 0o2 invece di 0o200
                    println!("❌ [OPEN] Permesso di scrittura negato");
                    reply.error(libc::EACCES);
                    return;
                }
                println!("✅ [OPEN] Permesso scrittura OK");
            }
            libc::O_RDWR => {
                println!("📝 [OPEN] Verifica permessi lettura/scrittura...");
                if (effective_perms & 0o6) != 0o6 {
                    // ✅ FIX: 0o6 invece di 0o600
                    println!("❌ [OPEN] Permessi lettura/scrittura insufficienti");
                    reply.error(libc::EACCES);
                    return;
                }
                println!("✅ [OPEN] Permessi lettura/scrittura OK");
            }
            _ => {
                println!("❌ [OPEN] Modalità di accesso non valida: {:#x}", access_mode);
                reply.error(libc::EINVAL);
                return;
            }
        }

        println!("📂 [OPEN] PERMESSI VERIFICATI - CONTINUANDO...");

        // 6. GENERA FILE HANDLE
        let fh = self.next_fh;
        self.next_fh += 1;

        println!("📂 [OPEN] FILE HANDLE GENERATO: {}", fh);

        // 7. REGISTRA FILE APERTO
        self.open_files.insert(fh, OpenFile {
            path: path.clone(),
            flags,
        });

        println!("📂 [OPEN] FILE REGISTRATO IN OPEN_FILES");

        // 8. GESTIONE O_TRUNC
        if (open_flags & libc::O_TRUNC) != 0 && access_mode != libc::O_RDONLY {
            println!("✂️ [OPEN] O_TRUNC rilevato - troncamento file");
            // ... codice troncamento se presente ...
        }

        println!("📂 [OPEN] PRIMA DI REPLY.OPENED");

        // 9. RESTITUISCI FILE HANDLE
        reply.opened(fh, 0);

        println!("📂 [OPEN] COMPLETATO CON SUCCESSO - FH: {}", fh);
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
        reply: ReplyData
    ) {
        log::debug!(
            "📖 [READ] ino: {}, fh: {}, offset: {}, size: {}, flags: {:#x}",
            ino,
            fh,
            offset,
            size,
            flags
        );

        // 1. VALIDAZIONE PARAMETRI
        if offset < 0 {
            log::error!("❌ [READ] Offset negativo: {}", offset);
            reply.error(libc::EINVAL);
            return;
        }

        if size == 0 {
            log::debug!("📖 [READ] Richiesta di lettura 0 bytes - EOF");
            reply.data(&[]);
            return;
        }

        let offset_u64 = offset as u64;
        let size_usize = size as usize;

        // 2. VERIFICA FILE HANDLE
        let open_file = match self.open_files.get(&fh) {
            Some(file) => file,
            None => {
                log::error!("❌ [READ] File handle {} non trovato", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        let path = open_file.path.clone();
        log::debug!("📖 [READ] Path: {}", path);

        // 3. VERIFICA PERMESSI DI LETTURA
        let access_mode = open_file.flags & libc::O_ACCMODE;
        if access_mode == libc::O_WRONLY {
            log::warn!("⚠️ [READ] Tentativo di lettura su file aperto in WRITE-ONLY: {}", path);
            reply.error(libc::EBADF);
            return;
        }

        // 4. OTTIENI METADATI E VERIFICA ESISTENZA
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
                log::error!("❌ [READ] File non trovato sul server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                log::error!("❌ [READ] Errore verifica metadati: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        // 5. VERIFICA TIPO FILE
        match metadata.kind {
            FileKind::RegularFile | FileKind::Symlink => {
                log::debug!("✅ [READ] Tipo file leggibile: {:?}", metadata.kind);
            }
            FileKind::Directory => {
                log::warn!("⚠️ [READ] Tentativo di read su directory: {}", path);
                reply.error(libc::EISDIR);
                return;
            }
            _ => {
                log::warn!("⚠️ [READ] Tipo file non supportato per read: {:?}", metadata.kind);
                reply.error(libc::EPERM);
                return;
            }
        }

        let file_size = metadata.size;

        // 6. GESTIONE OFFSET OLTRE EOF
        if offset_u64 >= file_size {
            log::debug!("📖 [READ] Offset {} >= dimensione file {} - EOF", offset_u64, file_size);
            reply.data(&[]);
            return;
        }

        // 7. CALCOLA DIMENSIONE EFFETTIVA DA LEGGERE
        let bytes_available = file_size - offset_u64;
        let bytes_to_read = std::cmp::min(size_usize as u64, bytes_available);

        log::debug!(
            "📖 [READ] File: {}, size: {}, offset: {}, requested: {}, reading: {}",
            path,
            file_size,
            offset_u64,
            size,
            bytes_to_read
        );

        // 8. GESTIONE LETTURE DI 0 BYTES (EOF raggiunto)
        if bytes_to_read == 0 {
            log::debug!("📖 [READ] EOF raggiunto, 0 bytes da leggere");
            reply.data(&[]);
            return;
        }

        // 9. LEGGI DATI DAL SERVER
        let read_result = rt.block_on(async {
            self.client.read_file(&path, Some(offset_u64), Some(bytes_to_read)).await
        });

        match read_result {
            Ok(read_response) => {
                let data = read_response.data;

                // 10. VALIDAZIONE DATI RICEVUTI
                if data.len() > (bytes_to_read as usize) {
                    log::warn!(
                        "⚠️ [READ] Server ha restituito più dati del richiesto: {} > {}, troncando",
                        data.len(),
                        bytes_to_read
                    );
                    reply.data(&data[..bytes_to_read as usize]);
                } else if data.is_empty() && bytes_to_read > 0 {
                    log::debug!("📖 [READ] Server ha restituito 0 bytes (EOF inaspettato)");
                    reply.data(&[]);
                } else {
                    log::debug!(
                        "✅ [READ] Lettura completata: {} bytes da offset {} per '{}'",
                        data.len(),
                        offset_u64,
                        path
                    );
                    reply.data(&data);
                }
            }
            Err(ClientError::NotFound { .. }) => {
                log::error!("❌ [READ] File eliminato durante la lettura: {}", path);
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                log::error!("❌ [READ] Permesso di lettura negato: {}", path);
                reply.error(libc::EACCES);
            }
            Err(e) => {
                log::error!("❌ [READ] Errore lettura dal server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }
    //ridare un occhio a questa funzione, se non funziona bene
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
        reply: fuser::ReplyWrite
    ) {
        println!("WRITEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE");
        log::debug!(
            "✏️ [WRITE] ino: {}, fh: {}, offset: {}, data.len: {}, write_flags: {:#x}, flags: {:#x}",
            ino,
            fh,
            offset,
            data.len(),
            write_flags,
            flags
        );

        // 1. VALIDAZIONE PARAMETRI
        if offset < 0 {
            log::error!("❌ [WRITE] Offset negativo: {}", offset);
            reply.error(libc::EINVAL);
            return;
        }

        if data.is_empty() {
            log::debug!("✅ [WRITE] Scrittura di 0 bytes - operazione completata");
            reply.written(0);
            return;
        }

        let offset_u64 = offset as u64;
        let data_len = data.len();

        // 2. VERIFICA FILE HANDLE
        let open_file = match self.open_files.get(&fh) {
            Some(file) => file,
            None => {
                log::error!("❌ [WRITE] File handle {} non trovato", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        let path = open_file.path.clone();
        let open_flags = open_file.flags;
        log::debug!("✏️ [WRITE] Path: {}", path);

        // 3. VERIFICA PERMESSI DI SCRITTURA
        let access_mode = open_flags & libc::O_ACCMODE;
        if access_mode == libc::O_RDONLY {
            log::warn!("⚠️ [WRITE] Tentativo di scrittura su file aperto in READ-ONLY: {}", path);
            reply.error(libc::EBADF);
            return;
        }

        // 4. OTTIENI METADATI E VERIFICA ESISTENZA
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
                log::error!("❌ [WRITE] File non trovato sul server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                log::error!("❌ [WRITE] Errore verifica metadati: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        // 5. VERIFICA TIPO FILE
        match metadata.kind {
            FileKind::RegularFile | FileKind::Symlink => {
                log::debug!("✅ [WRITE] Tipo file scrivibile: {:?}", metadata.kind);
            }
            FileKind::Directory => {
                log::warn!("⚠️ [WRITE] Tentativo di write su directory: {}", path);
                reply.error(libc::EISDIR);
                return;
            }
            _ => {
                log::warn!("⚠️ [WRITE] Tipo file non supportato per write: {:?}", metadata.kind);
                reply.error(libc::EPERM);
                return;
            }
        }

        let current_file_size = metadata.size;

        // 6. GESTIONE MODALITÀ APPEND
        let effective_offset = if (open_flags & libc::O_APPEND) != 0 {
            log::debug!(
                "📎 [WRITE] Modalità APPEND: offset {} → {}",
                offset_u64,
                current_file_size
            );
            current_file_size // Scrivi sempre alla fine del file
        } else {
            offset_u64
        };

        // 7. CALCOLA NUOVA DIMENSIONE FILE
        let new_file_size = std::cmp::max(current_file_size, effective_offset + (data_len as u64));

        println!(
            "✏️ [WRITE] File: {}, current_size: {}, offset: {}, effective_offset: {}, data_len: {}, new_size: {}",
            path,
            current_file_size,
            offset_u64,
            effective_offset,
            data_len,
            new_file_size
        );

        // 8. DETERMINA MODALITÀ DI SCRITTURA
        let write_mode = if effective_offset == current_file_size {
            // Scrittura alla fine del file (append)
            Mode::Append
        } else if effective_offset == 0 && (data_len as u64) >= current_file_size {
            // Sovrascrittura completa del file
            Mode::Write
        } else {
            // Scrittura parziale (non supportata direttamente)
            // Dovremmo leggere il file, modificare la porzione e riscrivere tutto
            log::warn!(
                "⚠️ [WRITE] Scrittura parziale non ottimizzata per offset: {}",
                effective_offset
            );
            Mode::Write // Fallback
        };

        // 9. PREPARA RICHIESTA DI SCRITTURA
        let now_iso = chrono::Utc::now().to_rfc3339();
        let write_request = WriteRequest {
            offset: if matches!(write_mode, Mode::WriteAt) {
                Some(effective_offset) // Non serve offset in append
            } else {
                None
            },
            path: path.clone(),
            new_path: None,
            size: if matches!(write_mode, Mode::WriteAt) {
                data.len() as u64
            } else if matches!(write_mode, Mode::Append) {
                println!("appenddddddddddddddddddddd");
                data_len as u64
            } else {
                new_file_size
            },
            atime: metadata.atime.clone(), // Mantieni access time
            mtime: now_iso.clone(), // Aggiorna modification time
            ctime: now_iso.clone(), // Aggiorna change time
            crtime: metadata.crtime.clone(), // Mantieni creation time
            kind: metadata.kind, // Mantieni tipo file
            ref_path: None, // Mantieni ref_path se esiste
            perm: metadata.perm.clone(), // Mantieni permessi
            mode: write_mode.clone(), // Modalità determinata sopra
            data: Some(data.to_vec()), // Dati da scrivere
        };

        // 10. GESTIONE SCRITTURA PARZIALE (se necessaria)
        let final_data = if
            matches!(write_mode, Mode::Write) &&
            effective_offset > 0 &&
            effective_offset < current_file_size
        {
            // Dobbiamo fare una scrittura parziale - leggi file esistente e modifica
            log::debug!("🔄 [WRITE] Eseguendo scrittura parziale...");

            match rt.block_on(async { self.client.read_file(&path, None, None).await }) {
                Ok(existing_content) => {
                    let mut file_data = existing_content.data;

                    // Estendi il file se necessario
                    if file_data.len() < (new_file_size as usize) {
                        file_data.resize(new_file_size as usize, 0);
                    }

                    // Sovrascrivi la porzione richiesta
                    let start_idx = effective_offset as usize;
                    let end_idx = std::cmp::min(start_idx + data.len(), file_data.len());
                    file_data[start_idx..end_idx].copy_from_slice(&data[..end_idx - start_idx]);

                    Some(file_data)
                }
                Err(e) => {
                    log::error!("❌ [WRITE] Errore lettura file per scrittura parziale: {}", e);
                    reply.error(libc::EIO);
                    return;
                }
            }
        } else {
            Some(data.to_vec())
        };

        // 11. AGGIORNA RICHIESTA CON DATI FINALI
        let mut final_request = write_request;
        if let Some(final_data_vec) = final_data {
            final_request.data = Some(final_data_vec);
            final_request.size = new_file_size;
        }

        // 12. ESEGUI SCRITTURA SUL SERVER
        let write_result = rt.block_on(async { self.client.write_file(&final_request).await });

        match write_result {
            Ok(()) => {
                log::debug!(
                    "✅ [WRITE] Scrittura completata: {} bytes scritti per '{}'",
                    data_len,
                    path
                );
                reply.written(data_len as u32);
            }
            Err(ClientError::NotFound { .. }) => {
                log::error!("❌ [WRITE] File eliminato durante la scrittura: {}", path);
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                log::error!("❌ [WRITE] Permesso di scrittura negato: {}", path);
                reply.error(libc::EACCES);
            }
            Err(ClientError::Server { status: 413, .. }) => {
                log::error!("❌ [WRITE] File troppo grande: {}", path);
                reply.error(libc::EFBIG);
            }
            Err(ClientError::Server { status: 507, .. }) => {
                log::error!("❌ [WRITE] Spazio insufficiente sul server: {}", path);
                reply.error(libc::ENOSPC);
            }
            Err(e) => {
                log::error!("❌ [WRITE] Errore scrittura sul server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn flush(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _lock_owner: u64,
        reply: fuser::ReplyEmpty
    ) {
        // Nessun buffering locale = nessuna azione necessaria
        reply.ok();
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        flags: i32,
        lock_owner: Option<u64>,
        flush: bool,
        reply: fuser::ReplyEmpty
    ) {
        log::debug!(
            "🔒 [RELEASE] ino: {}, fh: {}, flags: {:#x}, lock_owner: {:?}, flush: {}",
            ino,
            fh,
            flags,
            lock_owner,
            flush
        );

        // 1. VERIFICA CHE IL FILE HANDLE ESISTA
        let open_file = match self.open_files.get(&fh) {
            Some(file) => file,
            None => {
                log::warn!("⚠️ [RELEASE] File handle {} già rilasciato o inesistente", fh);
                // Non è un errore fatale - restituisci ok comunque
                reply.ok();
                return;
            }
        };

        let path = open_file.path.clone();
        log::debug!("🔒 [RELEASE] Path: {}", path);

        // 2. ESEGUI FLUSH SE RICHIESTO
        if flush {
            log::debug!("💫 [RELEASE] Flush richiesto prima del release");
            // Nel filesystem remoto, tutti i write vanno direttamente al server
            // quindi non c'è buffering locale da svuotare
        }

        // 3. CLEANUP: RIMUOVI FILE HANDLE DALLA CACHE
        if let Some(removed_file) = self.open_files.remove(&fh) {
            log::debug!(
                "✅ [RELEASE] File handle {} rimosso per path: '{}'",
                fh,
                removed_file.path
            );
        }

        // 4. STATISTICHE OPZIONALI
        log::debug!("📊 [RELEASE] File aperti rimanenti: {}", self.open_files.len());

        reply.ok();
    }

    fn fsync(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        datasync: bool,
        reply: fuser::ReplyEmpty
    ) {
        log::debug!("💫 [FSYNC] ino: {}, fh: {}, datasync: {}", ino, fh, datasync);

        // 1. VERIFICA FILE HANDLE VALIDO
        let open_file = match self.open_files.get(&fh) {
            Some(file) => file,
            None => {
                log::error!("❌ [FSYNC] File handle {} non trovato", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        let path = open_file.path.clone();
        log::debug!("💫 [FSYNC] Path: {}", path);

        // 2. VERIFICA PERMESSI
        let access_mode = open_file.flags & libc::O_ACCMODE;
        if access_mode == libc::O_RDONLY {
            log::warn!("⚠️ [FSYNC] File aperto in read-only: {}", path);
            reply.error(libc::EBADF);
            return;
        }

        // 3. NEL FILESYSTEM REMOTO, TUTTI I WRITE SONO GIÀ PERSISTENTI
        // I dati vanno direttamente al server senza buffering locale
        log::debug!("✅ [FSYNC] Filesystem remoto: dati già persistenti sul server");

        // Opzionale: Verifica che il file esista ancora
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        match rt.block_on(async { self.client.get_file_metadata(&path).await }) {
            Ok(_) => {
                log::debug!("✅ [FSYNC] File confermato esistente sul server");
                reply.ok();
            }
            Err(ClientError::NotFound { .. }) => {
                log::error!("❌ [FSYNC] File eliminato durante fsync: {}", path);
                reply.error(libc::ENOENT);
            }
            Err(e) => {
                log::error!("❌ [FSYNC] Errore verifica server: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn opendir(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        log::debug!("📂 [OPENDIR] ino: {}, flags: {:#x}", ino, flags);

        // 1. VALIDAZIONE INODE
        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                log::error!("❌ [OPENDIR] Inode {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };

        log::debug!("📂 [OPENDIR] Path: {}", path);

        // 2. VERIFICA CHE SIA UNA DIRECTORY
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
                log::error!("❌ [OPENDIR] Directory non trovata sul server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                log::error!("❌ [OPENDIR] Errore verifica metadati: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        // 3. VERIFICA TIPO DIRECTORY
        if metadata.kind != FileKind::Directory {
            log::warn!("⚠️ [OPENDIR] '{}' non è una directory: {:?}", path, metadata.kind);
            reply.error(libc::ENOTDIR);
            return;
        }

        // 4. VERIFICA PERMESSI DI LETTURA DIRECTORY
        log::debug!("📂 [OPENDIR] Flags: {:#x}", flags);

        // 5. VERIFICA CHE LA DIRECTORY SIA LEGGIBILE
        match rt.block_on(async { self.client.list_directory(&path).await }) {
            Ok(_) => {
                log::debug!("✅ [OPENDIR] Directory accessibile: {}", path);
            }
            Err(ClientError::PermissionDenied(_)) => {
                log::error!("❌ [OPENDIR] Permesso di lettura negato: {}", path);
                reply.error(libc::EACCES);
                return;
            }
            Err(e) => {
                log::error!("❌ [OPENDIR] Errore accesso directory: {}", e);
                reply.error(libc::EIO);
                return;
            }
        }

        // 6. GENERA DIRECTORY HANDLE
        let dh = self.next_fh;
        self.next_fh += 1;

        // 7. REGISTRA DIRECTORY APERTA
        self.open_dirs.insert(dh, OpenDir {
            path: path.clone(),
            flags, // ← Includi i flags
        });

        log::debug!(
            "✅ [OPENDIR] Directory aperta: path='{}', dh={}, flags={:#x}",
            path,
            dh,
            flags
        );

        // 8. RESTITUISCI DIRECTORY HANDLE
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
        log::debug!("📂 [READDIR] ino: {}, fh: {}, offset: {}", ino, fh, offset);

        // 1. VERIFICA DIRECTORY HANDLE
        let open_dir = match self.open_dirs.get(&fh) {
            Some(dir) => dir,
            None => {
                log::error!("❌ [READDIR] Directory handle {} non trovato", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        let path = open_dir.path.clone();
        log::debug!("📂 [READDIR] Path: {}", path);

        // 2. OTTIENI CONTENUTO DIRECTORY DAL SERVER
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
                log::error!("❌ [READDIR] Directory non trovata sul server: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(ClientError::PermissionDenied(_)) => {
                log::error!("❌ [READDIR] Permesso di lettura negato: {}", path);
                reply.error(libc::EACCES);
                return;
            }
            Err(e) => {
                log::error!("❌ [READDIR] Errore lettura directory: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        // 3. CREA LISTA ENTRIES (includiamo . e ..)
        let mut entries = Vec::new();

        // Entry "." (directory corrente)
        entries.push((ino, FileType::Directory, ".".to_string()));

        // Entry ".." (directory padre)
        let parent_ino = if path == "/" {
            1 // Root directory
        } else {
            // Calcola inode del padre
            let parent_path = std::path::Path
                ::new(&path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "/".to_string());

            self.path_to_inode.get(&parent_path).copied().unwrap_or(1)
        };
        entries.push((parent_ino, FileType::Directory, "..".to_string()));

        // 4. AGGIUNGI FILES DAL SERVER
        for file_entry in listing.files {
            // Costruisci path completo
            let entry_path = if path == "/" {
                format!("/{}", file_entry.name)
            } else {
                format!("{}/{}", path, file_entry.name)
            };

            // Ottieni o genera inode per questo file
            let entry_ino = if let Some(&existing_ino) = self.path_to_inode.get(&entry_path) {
                existing_ino
            } else {
                // Prima volta che vediamo questo file - genera nuovo inode
                let new_ino = self.generate_inode();
                self.register_inode(new_ino, entry_path.clone());
                new_ino
            };

            // Determina tipo file per FUSE
            let file_type = match file_entry.kind {
                FileKind::Directory => FileType::Directory,
                FileKind::RegularFile => FileType::RegularFile,
                FileKind::Symlink => FileType::Symlink,
                FileKind::Hardlink => FileType::RegularFile, // Hard link appare come file normale
                _ => {
                    log::warn!("⚠️ [READDIR] Tipo file non supportato: {:?}", file_entry.kind);
                    FileType::RegularFile // Fallback
                }
            };

            entries.push((entry_ino, file_type, file_entry.name));
        }

        log::debug!("📂 [READDIR] Trovati {} entries totali (inclusi . e ..)", entries.len());

        // 5. GESTIONE OFFSET E PAGINAZIONE
        let start_index = if offset == 0 {
            0
        } else {
            // offset rappresenta l'indice dell'entry successivo da leggere
            offset as usize
        };

        if start_index >= entries.len() {
            log::debug!(
                "📂 [READDIR] Offset {} >= entries totali {}, EOF",
                start_index,
                entries.len()
            );
            reply.ok();
            return;
        }

        // 6. AGGIUNGI ENTRIES AL REPLY
        let mut current_offset = start_index;
        for (entry_ino, file_type, name) in entries.into_iter().skip(start_index) {
            current_offset += 1;

            log::debug!(
                "📁 [READDIR] Entry: ino={}, type={:?}, name='{}', offset={}",
                entry_ino,
                file_type,
                name,
                current_offset
            );

            // Aggiungi entry al buffer di risposta
            let buffer_full = reply.add(
                entry_ino, // inode
                current_offset as i64, // offset per prossima entry
                file_type, // tipo file
                name // nome file
            );

            if buffer_full {
                log::debug!("📂 [READDIR] Buffer pieno, restituendo entries parziali");
                break;
            }
        }

        log::debug!(
            "✅ [READDIR] Completato per directory '{}', ultimo offset: {}",
            path,
            current_offset
        );

        reply.ok();
    }

    fn releasedir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        flags: i32,
        reply: fuser::ReplyEmpty
    ) {
        log::debug!("🔒 [RELEASEDIR] ino: {}, fh: {}, flags: {:#x}", ino, fh, flags);

        // 1. VERIFICA CHE IL DIRECTORY HANDLE ESISTA
        let open_dir = match self.open_dirs.get(&fh) {
            Some(dir) => dir,
            None => {
                log::warn!("⚠️ [RELEASEDIR] Directory handle {} già rilasciato o inesistente", fh);
                // Non è un errore fatale - restituisci ok comunque
                reply.ok();
                return;
            }
        };

        let path = open_dir.path.clone();
        log::debug!("🔒 [RELEASEDIR] Path: {}", path);

        // 2. CLEANUP: RIMUOVI DIRECTORY HANDLE DALLA CACHE
        if let Some(removed_dir) = self.open_dirs.remove(&fh) {
            log::debug!(
                "✅ [RELEASEDIR] Directory handle {} rilasciata per path: '{}'",
                fh,
                removed_dir.path
            );
        }

        // 3. STATISTICHE OPZIONALI
        log::debug!("📊 [RELEASEDIR] Directory aperte rimanenti: {}", self.open_dirs.len());

        log::debug!("✅ [RELEASEDIR] Operazione completata per: {}", path);

        reply.ok();
    }

    fn fsyncdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        datasync: bool,
        reply: fuser::ReplyEmpty
    ) {
        log::debug!("💫📂 [FSYNCDIR] ino: {}, fh: {}, datasync: {}", ino, fh, datasync);

        // 1. VERIFICA DIRECTORY HANDLE VALIDO
        let open_dir = match self.open_dirs.get(&fh) {
            Some(dir) => dir,
            None => {
                log::error!("❌ [FSYNCDIR] Directory handle {} non trovato", fh);
                reply.error(libc::EBADF);
                return;
            }
        };

        let path = open_dir.path.clone();
        log::debug!("💫📂 [FSYNCDIR] Path: {}", path);

        // 2. VERIFICA CHE SIA EFFETTIVAMENTE UNA DIRECTORY
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
                log::error!("❌ [FSYNCDIR] Directory non trovata: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                log::error!("❌ [FSYNCDIR] Errore verifica metadati: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        if metadata.kind != FileKind::Directory {
            log::error!("❌ [FSYNCDIR] '{}' non è una directory", path);
            reply.error(libc::ENOTDIR);
            return;
        }

        // 3. NEL FILESYSTEM REMOTO: SYNC DIRECTORY SUL SERVER
        log::debug!("✅ [FSYNCDIR] Filesystem remoto: metadati directory già persistenti");

        // Opzione A: Se il server supporta sync esplicito per directory
        // match rt.block_on(async { self.client.sync_directory(&path).await }) { ... }

        // Opzione B: Verifica che la directory sia ancora accessibile
        match rt.block_on(async { self.client.list_directory(&path).await }) {
            Ok(_) => {
                log::debug!("✅ [FSYNCDIR] Directory confermata accessibile sul server");
                reply.ok();
            }
            Err(ClientError::NotFound { .. }) => {
                log::error!("❌ [FSYNCDIR] Directory eliminata durante fsyncdir: {}", path);
                reply.error(libc::ENOENT);
            }
            Err(ClientError::PermissionDenied(_)) => {
                log::error!("❌ [FSYNCDIR] Permesso negato per directory: {}", path);
                reply.error(libc::EACCES);
            }
            Err(e) => {
                log::error!("❌ [FSYNCDIR] Errore verifica directory: {}", e);
                reply.error(libc::EIO);
            }
        }
    }

    fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: fuser::ReplyStatfs) {
        // Simula 1TB con 50% libero
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
            0
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
        reply: fuser::ReplyEmpty
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
        reply: fuser::ReplyXattr
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
        log::debug!("[Not Implemented] listxattr(ino: {:#x?}, size: {})", ino, size);
        reply.error(libc::ENOSYS);
    }

    fn removexattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        name: &OsStr,
        reply: fuser::ReplyEmpty
    ) {
        log::debug!("[Not Implemented] removexattr(ino: {:#x?}, name: {:?})", ino, name);
        reply.error(libc::ENOSYS);
    }

    fn access(&mut self, _req: &Request<'_>, ino: u64, mask: i32, reply: fuser::ReplyEmpty) {
        log::debug!("🔍 [ACCESS] ino: {}, mask: {:#x}", ino, mask);

        // 1. OTTIENI PATH DAL INODE
        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                log::error!("❌ [ACCESS] Inode {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };

        log::debug!("🔍 [ACCESS] Path: {}, mask: {:#x}", path, mask);

        // 2. DECODIFICA MASK
        let check_exist =
            mask == libc::F_OK || (mask & (libc::R_OK | libc::W_OK | libc::X_OK)) != 0;
        let check_read = (mask & libc::R_OK) != 0;
        let check_write = (mask & libc::W_OK) != 0;
        let check_exec = (mask & libc::X_OK) != 0;

        log::debug!(
            "🔍 [ACCESS] Verifiche: exist={}, read={}, write={}, exec={}",
            check_exist,
            check_read,
            check_write,
            check_exec
        );

        // 3. OTTIENI METADATI DAL SERVER
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
                log::error!("❌ [ACCESS] File non trovato: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(ClientError::PermissionDenied(_)) => {
                log::error!("❌ [ACCESS] Permesso negato per metadati: {}", path);
                reply.error(libc::EACCES);
                return;
            }
            Err(e) => {
                log::error!("❌ [ACCESS] Errore verifica esistenza: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        // 4. VERIFICA ESISTENZA (F_OK)
        if mask == libc::F_OK {
            log::debug!("✅ [ACCESS] File esiste: {}", path);
            reply.ok();
            return;
        }

        // 5. PARSING PERMESSI DAL SERVER
        let perms = parse_permissions(&metadata.perm);

        log::debug!(
            "🔍 [ACCESS] Permessi file: {}, parsed: owner={:#o}, group={:#o}, other={:#o}",
            metadata.perm,
            perms.owner,
            perms.group,
            perms.other
        );

        // 6. DETERMINA PERMESSI UTENTE (semplificato per filesystem remoto)
        // In un filesystem reale dovresti controllare uid/gid dell'utente
        let effective_perms = perms.owner; // Assumi che siamo sempre owner

        // 7. VERIFICA PERMESSI RICHIESTI
        let mut access_denied = false;

        if check_read && (effective_perms & 0o400) == 0 {
            log::warn!("⚠️ [ACCESS] Permesso lettura negato per: {}", path);
            access_denied = true;
        }

        if check_write && (effective_perms & 0o200) == 0 {
            log::warn!("⚠️ [ACCESS] Permesso scrittura negato per: {}", path);
            access_denied = true;
        }

        if check_exec && (effective_perms & 0o100) == 0 {
            log::warn!("⚠️ [ACCESS] Permesso esecuzione negato per: {}", path);
            access_denied = true;
        }

        // 8. VERIFICA TIPO FILE PER ESECUZIONE
        if check_exec && metadata.kind == FileKind::Directory {
            // Directory: esecuzione = attraversamento
            log::debug!("🔍 [ACCESS] Directory: permesso esecuzione = attraversamento");
        } else if check_exec && metadata.kind != FileKind::RegularFile {
            log::warn!("⚠️ [ACCESS] Tipo file non eseguibile: {:?}", metadata.kind);
            access_denied = true;
        }

        // 9. RISPOSTA FINALE
        if access_denied {
            reply.error(libc::EACCES);
        } else {
            log::debug!("✅ [ACCESS] Tutti i permessi verificati per: {}", path);
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
        println!("CREAAAATEEEEEEEEEEE");
        log::debug!(
            "🆕 [CREATE] parent: {}, name: {:?}, mode: {:#o}, umask: {:#o}, flags: {:#x}",
            parent,
            name,
            mode,
            umask,
            flags
        );

        // 1. VALIDAZIONE INPUT
        let filename = match name.to_str() {
            Some(s) => s,
            None => {
                log::error!("❌ [CREATE] Nome file non valido: {:?}", name);
                reply.error(libc::EINVAL);
                return;
            }
        };

        // 2. OTTIENI PATH DELLA DIRECTORY PADRE
        let parent_path = match self.get_path(parent) {
            Some(p) => p.clone(),
            None => {
                log::error!("❌ [CREATE] Directory padre con inode {} non trovata", parent);
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

        log::debug!("🆕 [CREATE] Path completo: {}", full_path);

        // 4. VERIFICA CHE IL FILE NON ESISTA GIÀ
        if self.path_to_inode.contains_key(&full_path) {
            log::warn!("⚠️ [CREATE] File già esistente: {}", full_path);
            reply.error(libc::EEXIST);
            return;
        }

        // 5. CALCOLA PERMESSI EFFETTIVI
        let effective_permissions = mode & 0o777 & !(umask & 0o777);
        let effective_permissions_str = format!("{:o}", effective_permissions);

        // 6. ANALISI FLAGS DI APERTURA
        let access_mode = flags & libc::O_ACCMODE;
        let open_flags = flags & !libc::O_ACCMODE;

        log::debug!(
            "🆕 [CREATE] Permessi: {:#o}, Access mode: {:#x}, Open flags: {:#x}",
            effective_permissions,
            access_mode,
            open_flags
        );

        // 7. CREA FILE SUL SERVER
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
            new_path: None, // ✅ AGGIUNGI QUESTO
            size: 0, // File vuoto inizialmente
            atime: now_iso.clone(),
            mtime: now_iso.clone(),
            ctime: now_iso.clone(),
            crtime: now_iso,
            kind: FileKind::RegularFile,
            ref_path: None, // ✅ AGGIUNGI QUESTO
            perm: effective_permissions_str, // ✅ FIX: usa la variabile corretta
            mode: Mode::Write,
            data: Some(Vec::new()), // File vuoto
        };

        // 8. GESTIONE TRUNCATE FLAG
        if (open_flags & libc::O_TRUNC) != 0 {
            log::debug!("✂️ [CREATE] Flag O_TRUNC rilevato (redundante su file nuovo)");
            // Su file nuovo, O_TRUNC è ridondante
        }

        match rt.block_on(async { self.client.write_file(&create_request).await }) {
            Ok(()) => {
                log::debug!("✅ [CREATE] File creato sul server con successo");

                // 9. GENERA NUOVO INODE E REGISTRA
                let new_inode = self.generate_inode();
                self.register_inode(new_inode, full_path.clone());

                // 10. GENERA FILE HANDLE PER APERTURA
                let fh = self.next_fh;
                self.next_fh += 1;

                // 11. REGISTRA FILE APERTO
                self.open_files.insert(fh, OpenFile {
                    path: full_path.clone(),
                    flags,
                });

                // 12. OTTIENI METADATI DAL SERVER
                let metadata_result = rt.block_on(async {
                    self.client.get_file_metadata(&full_path).await
                });

                match metadata_result {
                    Ok(metadata) => {
                        // Usa metadati reali dal server
                        let attr = attributes::from_metadata(new_inode, &metadata);
                        let ttl = Duration::from_secs(1);

                        log::debug!(
                            "✅ [CREATE] File creato e aperto: path='{}', ino={}, fh={}",
                            full_path,
                            new_inode,
                            fh
                        );

                        reply.created(&ttl, &attr, 0, fh, 0);
                    }
                    Err(e) => {
                        log::error!("❌ [CREATE] Errore recupero metadati: {}", e);
                        // File creato ma usa attributi base
                        let attr = new_file_attr(new_inode, 0, effective_permissions);
                        let ttl = Duration::from_secs(1);
                        reply.created(&ttl, &attr, 0, fh, 0);
                    }
                }
            }
            Err(e) => {
                log::error!("❌ [CREATE] Errore creazione file sul server: {}", e);
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
        reply: fuser::ReplyLock
    ) {
        log::debug!(
            "🔒 [GETLK] ino: {}, range: {}-{}, type: {}, pid: {}",
            ino,
            start,
            end,
            typ,
            pid
        );

        // Verifica file handle
        if !self.open_files.contains_key(&fh) {
            reply.error(libc::EBADF);
            return;
        }

        // Cerca conflitti con lock esistenti
        if let Some(locks) = self.file_locks.get(&ino) {
            for existing_lock in locks {
                // Verifica sovrapposizione di range
                if ranges_overlap(start, end, existing_lock.start, existing_lock.end) {
                    // Verifica conflitto di tipo
                    if locks_conflict(typ, existing_lock.typ) {
                        log::debug!(
                            "⚠️ [GETLK] Conflitto trovato con lock {} di pid {}",
                            existing_lock.typ,
                            existing_lock.pid
                        );
                        reply.locked(
                            existing_lock.start,
                            existing_lock.end,
                            existing_lock.typ,
                            existing_lock.pid
                        );
                        return;
                    }
                }
            }
        }

        // Nessun conflitto trovato
        log::debug!("✅ [GETLK] Nessun conflitto, lock disponibile");
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
        reply: fuser::ReplyEmpty
    ) {
        log::debug!(
            "🔒 [SETLK] ino: {}, range: {}-{}, type: {}, pid: {}, sleep: {}",
            ino,
            start,
            end,
            typ,
            pid,
            sleep
        );

        // Verifica file handle
        if !self.open_files.contains_key(&fh) {
            reply.error(libc::EBADF);
            return;
        }

        match typ {
            libc::F_UNLCK => {
                // Rimuovi lock esistenti
                if let Some(locks) = self.file_locks.get_mut(&ino) {
                    locks.retain(|lock| {
                        !(
                            lock.lock_owner == lock_owner &&
                            ranges_overlap(start, end, lock.start, lock.end)
                        )
                    });
                }
                log::debug!("✅ [SETLK] Lock rilasciato");
                reply.ok();
            }
            libc::F_RDLCK | libc::F_WRLCK => {
                // Verifica conflitti
                if let Some(locks) = self.file_locks.get(&ino) {
                    for existing_lock in locks {
                        if
                            ranges_overlap(start, end, existing_lock.start, existing_lock.end) &&
                            locks_conflict(typ, existing_lock.typ) &&
                            existing_lock.lock_owner != lock_owner
                        {
                            if sleep {
                                // In un'implementazione reale, dovresti mettere il processo in attesa
                                log::warn!(
                                    "⚠️ [SETLK] Lock bloccante non implementato completamente"
                                );
                                reply.error(libc::ENOSYS);
                            } else {
                                log::debug!("❌ [SETLK] Lock conflict, non-blocking");
                                reply.error(libc::EAGAIN);
                            }
                            return;
                        }
                    }
                }

                // Aggiungi nuovo lock
                let new_lock = FileLock {
                    typ,
                    start,
                    end,
                    pid,
                    lock_owner,
                };

                self.file_locks.entry(ino).or_insert_with(Vec::new).push(new_lock);

                log::debug!("✅ [SETLK] Lock acquisito: type={}", typ);
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
        reply: fuser::ReplyBmap
    ) {
        log::debug!("🗺️ [BMAP] ino: {}, blocksize: {}, idx: {}", ino, blocksize, idx);

        // 1. VERIFICA CHE IL FILE ESISTA
        let path = match self.inode_to_path.get(&ino) {
            Some(p) => p.clone(),
            None => {
                log::error!("❌ [BMAP] Inode {} non trovato", ino);
                reply.error(libc::ENOENT);
                return;
            }
        };

        // 2. OTTIENI METADATI DEL FILE
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
                log::error!("❌ [BMAP] File non trovato: {}", path);
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                log::error!("❌ [BMAP] Errore metadati: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        // 3. VERIFICA TIPO FILE
        if metadata.kind != FileKind::RegularFile {
            log::warn!("⚠️ [BMAP] bmap solo supportato per file regolari");
            reply.error(libc::EPERM);
            return;
        }

        // 4. CALCOLA NUMERO TOTALE DI BLOCCHI
        let file_size = metadata.size;
        let blocks_in_file = (file_size + (blocksize as u64) - 1) / (blocksize as u64);

        // 5. VERIFICA CHE IL BLOCCO RICHIESTO ESISTA
        if idx >= blocks_in_file {
            log::debug!("📍 [BMAP] Blocco {} oltre EOF (file ha {} blocchi)", idx, blocks_in_file);
            reply.error(libc::ENXIO);
            return;
        }

        // 6. SIMULA MAPPATURA SEQUENZIALE
        // Per filesystem remoto, simula che i blocchi siano sequenziali
        // Usiamo l'inode come "base address" e aggiungiamo l'offset del blocco
        let simulated_physical_block = ino * 1000 + idx;

        log::debug!(
            "✅ [BMAP] File: {}, logical_block: {} → physical_block: {} (simulato)",
            path,
            idx,
            simulated_physical_block
        );

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
        log::debug!(
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
        reply: fuser::ReplyLseek
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
        reply: fuser::ReplyWrite
    ) {
        log::debug!(
            "📋 [COPY_FILE_RANGE] in: ino={}, fh={}, offset={}, out: ino={}, fh={}, offset={}, len={}",
            ino_in,
            fh_in,
            offset_in,
            ino_out,
            fh_out,
            offset_out,
            len
        );

        // 1. VALIDAZIONE PARAMETRI
        if offset_in < 0 || offset_out < 0 {
            log::error!("❌ [COPY_FILE_RANGE] Offset negativi non supportati");
            reply.error(libc::EINVAL);
            return;
        }

        if len == 0 {
            log::debug!("✅ [COPY_FILE_RANGE] Nulla da copiare");
            reply.written(0);
            return;
        }

        // 2. VERIFICA FILE HANDLES
        let source_file = match self.open_files.get(&fh_in) {
            Some(file) => file,
            None => {
                log::error!("❌ [COPY_FILE_RANGE] File handle sorgente {} non trovato", fh_in);
                reply.error(libc::EBADF);
                return;
            }
        };

        let dest_file = match self.open_files.get(&fh_out) {
            Some(file) => file,
            None => {
                log::error!("❌ [COPY_FILE_RANGE] File handle destinazione {} non trovato", fh_out);
                reply.error(libc::EBADF);
                return;
            }
        };

        // 3. VERIFICA PERMESSI
        let source_access = source_file.flags & libc::O_ACCMODE;
        let dest_access = dest_file.flags & libc::O_ACCMODE;

        if source_access == libc::O_WRONLY {
            log::error!("❌ [COPY_FILE_RANGE] File sorgente non leggibile");
            reply.error(libc::EBADF);
            return;
        }

        if dest_access == libc::O_RDONLY {
            log::error!("❌ [COPY_FILE_RANGE] File destinazione non scrivibile");
            reply.error(libc::EBADF);
            return;
        }

        // 4. ESEGUI COPIA CON READ + WRITE
        let rt = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle,
            Err(_) => {
                let runtime = tokio::runtime::Runtime::new().expect("Failed to create runtime");
                runtime.handle().clone()
            }
        };
        let chunk_size = std::cmp::min(len, 1024 * 1024); // Max 1MB per chunk

        // Leggi dal file sorgente
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
                log::error!("❌ [COPY_FILE_RANGE] Errore lettura sorgente: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        let bytes_read = source_data.len() as u64;
        let bytes_to_copy = std::cmp::min(len, bytes_read);

        if bytes_to_copy == 0 {
            log::debug!("📋 [COPY_FILE_RANGE] EOF raggiunto in sorgente");
            reply.written(0);
            return;
        }

        // Ottieni metadati destinazione per merge
        let dest_metadata = match
            rt.block_on(async { self.client.get_file_metadata(&dest_file.path).await })
        {
            Ok(metadata) => metadata,
            Err(e) => {
                log::error!("❌ [COPY_FILE_RANGE] Errore metadati destinazione: {}", e);
                reply.error(libc::EIO);
                return;
            }
        };

        // Scrivi nel file destinazione
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
                log::debug!("✅ [COPY_FILE_RANGE] Copiati {} bytes", bytes_to_copy);
                reply.written(bytes_to_copy as u32);
            }
            Err(e) => {
                log::error!("❌ [COPY_FILE_RANGE] Errore scrittura: {}", e);
                reply.error(libc::EIO);
            }
        }
    }
}
