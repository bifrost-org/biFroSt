use std::path::PathBuf;
use remotefs::api::client::RemoteClient;
use remotefs::config::settings::Config;
use remotefs::fs::operations::RemoteFileSystem;
use fuser::{mount2, MountOption};
use tokio::signal;

#[tokio::main]
async fn main() {
    println!("🚀 Avvio RemoteFS...");
    
    // Configurazione standard
    let config = Config {
        server_url: "https://bifrost.oberon-server.it".to_string(),
        port: 443,
        mount_point: PathBuf::from("/tmp/remotefs_mount11"),
        api_key: None,
        username: None,
        password: None,
        timeout: std::time::Duration::from_secs(60),
    };

    println!("📡 Server: {}", config.server_full_url());
    println!("📁 Mount point: {:?}", config.mount_point);

    // Crea la directory di mount se non esiste
    if !config.mount_point.exists() {
        match std::fs::create_dir_all(&config.mount_point) {
            Ok(_) => println!("✅ Directory di mount creata"),
            Err(e) => {
                eprintln!("❌ Impossibile creare directory di mount: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Verifica che la directory sia vuota (non già montata)
    match std::fs::read_dir(&config.mount_point) {
        Ok(entries) => {
            if entries.count() > 0 {
                eprintln!("⚠️ Directory di mount non vuota. Potrebbe essere già montata.");
                eprintln!("💡 Prova: fusermount -u {:?}", config.mount_point);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("❌ Impossibile leggere directory di mount: {}", e);
            std::process::exit(1);
        }
    }
    
    // Crea il filesystem
    let filesystem = RemoteFileSystem::new(RemoteClient::new(&config));
    println!("✅ Filesystem inizializzato");



    // Opzioni di mount FUSE
    let options = [
        MountOption::RW,
        MountOption::FSName("remotefs".to_string()),
        // ❌ RIMUOVI: MountOption::AllowOther,        // Causa errore senza config
        MountOption::DefaultPermissions, // Usa permessi standard
    ];

    println!("🔧 Montaggio filesystem su {:?}...", config.mount_point);
    
    // Clona il mount point per gestione segnali
    let mount_point_for_cleanup = config.mount_point.clone();
    let mount_point_display = config.mount_point.clone();
    
    // ✅ GESTIONE CTRL+C PER CLEANUP AUTOMATICO
    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(_) => {
                println!("\n🛑 Ricevuto Ctrl+C, smonto il filesystem...");
                
                // Prova diversi metodi di smount
                let cleanup_success = cleanup_mount(&mount_point_for_cleanup);
                
                if cleanup_success {
                    println!("✅ Filesystem smontato correttamente");
                } else {
                    println!("⚠️ Problemi durante lo smount. Potrebbe essere necessario un cleanup manuale:");
                    println!("   fusermount -u {:?}", mount_point_for_cleanup);
                    println!("   sudo umount {:?}", mount_point_for_cleanup);
                }
                
                println!("👋 Uscita...");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("❌ Errore nell'ascolto di Ctrl+C: {}", e);
            }
        }
    });
    
    // Spawna il mount in un task bloccante
    println!("⏳ Avvio mount FUSE...");
    let mount_result = tokio::task::spawn_blocking(move || {
        mount2(filesystem, &config.mount_point, &options)
    }).await;

    match mount_result {
        Ok(Ok(())) => {
            println!("✅ Filesystem montato con successo!");
            println!("🔍 Esplora il filesystem: {:?}", mount_point_display);
            println!("📋 Comandi utili:");
            println!("   ls {:?}", mount_point_display);
            println!("   touch {:?}/test.txt", mount_point_display);
            println!("   echo 'Hello' > {:?}/hello.txt", mount_point_display);
            println!("🔄 Il processo rimarrà attivo per mantenere il mount...");
            println!("💡 Premi Ctrl+C per smontare e uscire");
            
            // Mantieni il processo attivo
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                
                // Controlla periodicamente che il mount sia ancora valido
                if !mount_point_display.exists() {
                    eprintln!("❌ Directory di mount scomparsa!");
                    std::process::exit(1);
                }
            }
        }
        Ok(Err(e)) => {
            eprintln!("❌ Errore nel montaggio FUSE: {}", e);
            eprintln!("💡 Possibili soluzioni:");
            eprintln!("   - Verifica che FUSE sia installato: sudo apt install fuse");
            eprintln!("   - Controlla permessi: sudo usermod -a -G fuse $USER");
            eprintln!("   - Riavvia la sessione dopo aver aggiunto il gruppo");
            eprintln!("   - Verifica che la directory non sia già montata");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("❌ Errore nel task di mount: {}", e);
            std::process::exit(1);
        }
    }
}

/// Funzione helper per pulire il mount con diversi metodi
fn cleanup_mount(mount_point: &PathBuf) -> bool {
    // Metodo 1: fusermount (raccomandato per FUSE)
    if let Ok(output) = std::process::Command::new("fusermount")
        .arg("-u")
        .arg(mount_point)
        .output() {
        if output.status.success() {
            return true;
        }
    }
    
    // Metodo 2: umount standard
    if let Ok(output) = std::process::Command::new("umount")
        .arg(mount_point)
        .output() {
        if output.status.success() {
            return true;
        }
    }
    
    // Metodo 3: umount forzato
    if let Ok(output) = std::process::Command::new("umount")
        .arg("-f")
        .arg(mount_point)
        .output() {
        if output.status.success() {
            return true;
        }
    }
    
    // Metodo 4: lazy umount
    if let Ok(output) = std::process::Command::new("umount")
        .arg("-l")
        .arg(mount_point)
        .output() {
        if output.status.success() {
            return true;
        }
    }
    
    false
}