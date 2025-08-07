use fuser::{mount2, MountOption};
use remotefs::api::client::RemoteClient;
use remotefs::config::settings::Config;
use remotefs::fs::operations::RemoteFileSystem;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    println!("🚀 Avvio RemoteFS...");

    // Configurazione
    let config = Config {
        server_url: "https://bifrost.oberon-server.it".to_string(),
        port: 443,
        mount_point: PathBuf::from("/tmp/remotefs_mount32"),
        api_key: None,
        username: None,
        password: None,
        timeout: std::time::Duration::from_secs(60),
    };

    println!("📡 Server: {}", config.server_full_url());
    println!("📁 Mount point: {:?}", config.mount_point);

    // ✅ LOGICA SEMPLIFICATA SECONDO LE TUE SPECIFICHE
    prepare_mount_point(&config.mount_point);

    // ✅ FILESYSTEM E MOUNT
    let filesystem = RemoteFileSystem::new(RemoteClient::new(&config));
    println!("✅ Filesystem inizializzato");

    let options = [
        MountOption::RW,
        MountOption::FSName("remotefs".to_string()),
        MountOption::DefaultPermissions,
    ];

    println!("🔧 Montaggio filesystem...");
    println!("📋 Per testare: ls {:?}", config.mount_point);
    println!("🛑 Premi Ctrl+C per terminare");

    // ✅ MOUNT DIRETTO CON spawn_blocking
    let mount_point_clone = config.mount_point.clone();
    
    let mount_task = tokio::task::spawn_blocking(move || {
        println!("📡 Avvio mount2 in spawn_blocking...");
        mount2(filesystem, &mount_point_clone, &options)
    });

    // ✅ ATTENDI RISULTATO
    match mount_task.await {
        Ok(Ok(())) => println!("✅ Mount terminato"),
        Ok(Err(e)) => eprintln!("❌ Errore mount: {}", e),
        Err(e) => eprintln!("❌ Errore task: {}", e),
    }
}

fn prepare_mount_point(mount_point: &PathBuf) {
    println!("🔍 Preparazione mount point: {:?}", mount_point);
    
    // Estrai directory padre e nome directory
    let parent_dir = mount_point.parent().unwrap_or(std::path::Path::new("/"));
    let dir_name = mount_point.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    
    println!("📁 Directory padre: {:?}", parent_dir);
    println!("📁 Nome directory: {}", dir_name);
    
    // Verifica se la directory padre esiste
    if !parent_dir.exists() {
        eprintln!("❌ Directory padre non esiste: {:?}", parent_dir);
        std::process::exit(1);
    }
    
    // Controlla se il mount point è contenuto nella directory padre
    let mount_point_exists = check_if_mount_point_exists_in_parent(parent_dir, dir_name);
    
    if mount_point_exists {
        println!("📁 Mount point trovato nella directory padre");
        
        // Unmount + rimozione
        println!("🔄 Eseguo umount -l {:?}", mount_point);
        let _ = std::process::Command::new("umount")
            .arg(mount_point)
            .output();
        
        println!("🗑️ Eseguo rmdir {:?}", mount_point);
        let _ = std::process::Command::new("rmdir")
            .arg(mount_point)
            .output();
        
        /*
        // ✅ FORZA INVALIDAZIONE CACHE DIRECTORY PADRE
        println!("🧹 Forza invalidazione cache directory padre...");
        invalidate_directory_cache(parent_dir);
        */
        // Attendi stabilizzazione più lunga
        std::thread::sleep(std::time::Duration::from_millis(1000));
    } else {
        println!("📁 Mount point non trovato nella directory padre");
    }
    
    // Crea directory mount
    println!("📁 Creo directory mount: {:?}", mount_point);
    match std::fs::create_dir_all(mount_point) {
        Ok(_) => {
            println!("✅ Directory mount creata");
            
            // ✅ FORZA INVALIDAZIONE CACHE DOPO CREAZIONE
            invalidate_directory_cache(parent_dir);
        }
        Err(e) => {
            eprintln!("❌ Errore creazione directory: {}", e);
            std::process::exit(1);
        }
    }
}

// ✅ FUNZIONE PER INVALIDARE CACHE DIRECTORY
fn invalidate_directory_cache(dir_path: &std::path::Path) {
    println!("🧹 Invalidazione cache per: {:?}", dir_path);
    
    // Metodo 1: sync per forzare flush filesystem
    let _ = std::process::Command::new("sync").output();
    
    // Metodo 2: touch directory per aggiornare timestamp
    let _ = std::process::Command::new("touch")
        .arg(dir_path)
        .output();
    
    // Metodo 3: ls directory per forzare refresh cache
    let _ = std::process::Command::new("ls")
        .arg("-la")
        .arg(dir_path)
        .output();
    
    // Metodo 4: drop cache VFS (richiede root, ma proviamo)
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg("echo 2 > /proc/sys/vm/drop_caches 2>/dev/null || true")
        .output();
    
    println!("✅ Cache invalidation completata");
}
// ✅ VERIFICA ESISTENZA NELLA DIRECTORY PADRE
fn check_if_mount_point_exists_in_parent(parent_dir: &std::path::Path, dir_name: &str) -> bool {
    println!("🔍 Cerco '{}' in {:?}", dir_name, parent_dir);
    
    // Metodo 1: Lettura directory normale
    match std::fs::read_dir(parent_dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                if let Some(entry_name) = entry.file_name().to_str() {
                    if entry_name == dir_name {
                        println!("  ✅ Trovato '{}' tramite read_dir", dir_name);
                        return true;
                    }
                }
            }
            println!("  ❌ Non trovato '{}' tramite read_dir", dir_name);
        }
        Err(e) => {
            println!("  ⚠️ Errore read_dir: {}", e);
        }
    }
    
    // Metodo 2: Comando ls come fallback
    match std::process::Command::new("ls")
        .arg("-1")  // Una colonna
        .arg(parent_dir)
        .output()
    {
        Ok(output) if output.status.success() => {
            let ls_output = String::from_utf8_lossy(&output.stdout);
            for line in ls_output.lines() {
                if line.trim() == dir_name {
                    println!("  ✅ Trovato '{}' tramite ls", dir_name);
                    return true;
                }
            }
            println!("  ❌ Non trovato '{}' tramite ls", dir_name);
        }
        Ok(output) => {
            println!("  ⚠️ ls fallito: {}", String::from_utf8_lossy(&output.stderr));
        }
        Err(e) => {
            println!("  ⚠️ Errore comando ls: {}", e);
        }
    }
    
    // Metodo 3: Test diretto del path
    let full_path = parent_dir.join(dir_name);
    if full_path.exists() {
        println!("  ✅ Trovato '{}' tramite path exists", dir_name);
        return true;
    }
    
    println!("  ❌ '{}' non trovato con nessun metodo", dir_name);
    false
}