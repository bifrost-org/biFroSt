use std::path::PathBuf;
use remotefs::{api::client::RemoteClient, config::settings::Config, fs::operations::RemoteFileSystem};
use fuser::mount2;

#[tokio::main]
async fn main() {
    // Configurazione standard
    let config = Config {
        server_url: "http://192.168.56.1".to_string(),
        port: 3000,
        mount_point: PathBuf::from("/tmp/remotefs_mount2"),
        api_key: None,
        username: None,
        password: None,
        timeout: std::time::Duration::from_secs(60),
    };

    println!("🚀 Avvio RemoteFS...");
    println!("📡 Server: {}", config.server_full_url());
    println!("📁 Mount point: {:?}", config.mount_point);

    // Crea la directory di mount se non esiste
    if !config.mount_point.exists() {
        std::fs::create_dir_all(&config.mount_point)
            .expect("Impossibile creare directory di mount");
        println!("✅ Directory di mount creata");
    }
    
    // Crea il filesystem
    let filesystem = RemoteFileSystem::new(RemoteClient::new(&config));
    println!("✅ Filesystem inizializzato");

    use fuser::MountOption;

    // Opzioni di mount FUSE
    let options = [
        MountOption::RW,
        MountOption::FSName("remotefs".to_string()),
    ];

    println!("🔧 Montaggio filesystem su {:?}...", config.mount_point);
    
    // Clona il mount point prima del move
    let mount_point_clone = config.mount_point.clone();
    
    // Spawna il mount in un task bloccante
    let mount_result = tokio::task::spawn_blocking(move || {
        mount2(filesystem, &config.mount_point, &options)
    }).await;

    match mount_result {
        Ok(Ok(())) => {
            println!("✅ Filesystem montato con successo!");
            println!("🔍 Puoi ora esplorare: {:?}", mount_point_clone);
            println!("🔄 Il processo rimarrà attivo per mantenere il mount...");
            
            // Mantieni il processo attivo
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
        Ok(Err(e)) => {
            eprintln!("❌ Errore nel montaggio: {}", e);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("❌ Errore nel task di mount: {}", e);
            std::process::exit(1);
        }
    }
}