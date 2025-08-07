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
        mount_point: PathBuf::from("/tmp/remotefs_mount31"),
        api_key: None,
        username: None,
        password: None,
        timeout: std::time::Duration::from_secs(60),
    };

    println!("📡 Server: {}", config.server_full_url());
    println!("📁 Mount point: {:?}", config.mount_point);

    // ✅ GESTIONE INTELLIGENTE DIRECTORY MOUNT
    if config.mount_point.exists() {
        println!("📁 Directory mount già esistente");
        
        // Verifica se è già montata
        if is_mounted(&config.mount_point) {
            println!("🔄 Directory già montata, smonto...");
            
            if unmount_filesystem(&config.mount_point) {
                println!("✅ Filesystem smontato con successo");
            } else {
                eprintln!("❌ Impossibile smontare il filesystem");
                eprintln!("💡 Prova manualmente: fusermount -u {:?}", config.mount_point);
                eprintln!("💡 Oppure: umount {:?}", config.mount_point);
                std::process::exit(1);
            }
        }
        
        // Verifica che sia vuota dopo lo smontaggio
        match std::fs::read_dir(&config.mount_point) {
            Ok(entries) => {
                let count = entries.count();
                if count > 0 {
                    println!("🧹 Directory non vuota ({} elementi), pulisco...", count);
                    
                    // Prova a pulire la directory
                    match std::fs::remove_dir_all(&config.mount_point) {
                        Ok(_) => {
                            println!("✅ Directory pulita");
                            std::fs::create_dir_all(&config.mount_point)
                                .expect("Cannot recreate mount dir");
                        }
                        Err(e) => {
                            eprintln!("❌ Impossibile pulire directory: {}", e);
                            eprintln!("💡 Pulisci manualmente: rm -rf {:?}", config.mount_point);
                            std::process::exit(1);
                        }
                    }
                } else {
                    println!("✅ Directory mount vuota e pronta");
                }
            }
            Err(e) => {
                eprintln!("❌ Impossibile leggere directory mount: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        // Crea directory se non esiste
        match std::fs::create_dir_all(&config.mount_point) {
            Ok(_) => println!("✅ Directory di mount creata"),
            Err(e) => {
                eprintln!("❌ Impossibile creare directory di mount: {}", e);
                std::process::exit(1);
            }
        }
    }

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

// ✅ FUNZIONI HELPER PER GESTIONE MOUNT
fn is_mounted(mount_point: &PathBuf) -> bool {
    // Verifica tramite comando mount
    if let Ok(output) = std::process::Command::new("mount").output() {
        let mount_output = String::from_utf8_lossy(&output.stdout);
        let mount_point_str = mount_point.to_string_lossy();
        
        if mount_output.contains(&*mount_point_str) {
            return true;
        }
    }
    
    // Verifica tramite /proc/mounts su Linux
    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
        let mount_point_str = mount_point.to_string_lossy();
        if mounts.contains(&*mount_point_str) {
            return true;
        }
    }
    
    false
}

fn unmount_filesystem(mount_point: &PathBuf) -> bool {
    // Prova prima con fusermount
    if let Ok(output) = std::process::Command::new("fusermount")
        .arg("-u")
        .arg(mount_point)
        .output() 
    {
        if output.status.success() {
            return true;
        }
    }

    // Prova con umount normale
    if let Ok(output) = std::process::Command::new("umount")
        .arg(mount_point)
        .output() 
    {
        if output.status.success() {
            return true;
        }
    }

    // Prova con umount lazy (forza)
    if let Ok(output) = std::process::Command::new("umount")
        .arg("-l")
        .arg(mount_point)
        .output() 
    {
        if output.status.success() {
            return true;
        }
    }

    // Ultima risorsa: prova con sudo (se disponibile)
    if let Ok(output) = std::process::Command::new("sudo")
        .arg("umount")
        .arg("-l")
        .arg(mount_point)
        .output() 
    {
        if output.status.success() {
            return true;
        }
    }

    false
}