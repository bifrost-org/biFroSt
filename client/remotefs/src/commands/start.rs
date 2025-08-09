use std::path::PathBuf;

use fuser::{mount2, MountOption};
use remotefs::{
    api::client::RemoteClient, config::settings::Config, fs::operations::RemoteFileSystem,
};

pub async fn run() {
    println!("🚀 Start bifrost...");

    let config = Config::from_file().expect("Loading configuration failed");

    println!("📡 Server: {}", config.server_full_url());
    println!("📁 Mount point: {:?}", config.mount_point);

    prepare_mount_point(&config.mount_point);

    // FILESYSTEM AND MOUNT
    let filesystem = RemoteFileSystem::new(RemoteClient::new(&config));
    println!("✅ Filesystem initialized");

    let options = [
        MountOption::RW,
        MountOption::FSName("bifrost".to_string()),
        MountOption::DefaultPermissions,
    ];

    println!("🔧 Mounting filesystem...");
    println!("📋 To test it: ls {:?}", config.mount_point);
    println!("🛑 Ctrl+C to exit");

    // ✅ Direct mound with spawn_blocking
    let mount_point_clone = config.mount_point.clone();

    let mount_task = tokio::task::spawn_blocking(move || {
        println!("📡 Avvio mount2 in spawn_blocking...");
        mount2(filesystem, &mount_point_clone, &options)
    });

    // ✅ WAIT RESULT
    match mount_task.await {
        Ok(Ok(())) => println!("✅ Mount terminato"),
        Ok(Err(e)) => eprintln!("❌ Errore mount: {}", e),
        Err(e) => eprintln!("❌ Errore task: {}", e),
    }
}

fn prepare_mount_point(mount_point: &PathBuf) {
    println!("🔍 Preparing mount point: {:?}", mount_point);

    // Extract parent directory and name of directory
    let parent_dir = mount_point.parent().unwrap_or(std::path::Path::new("/"));
    let dir_name = mount_point
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");

    println!("📁 Parent directory: {:?}", parent_dir);
    println!("📁 Directory name: {}", dir_name);

    if !parent_dir.exists() {
        eprintln!("❌ Parent directory does not exist: {:?}", parent_dir);
        std::process::exit(1);
    }

    let mount_point_exists = check_if_mount_point_exists_in_parent(parent_dir, dir_name);

    if mount_point_exists {
        println!("📁 Mount point found in parent directory");

        // Unmount + remotion
        println!("🔄 umount -l {:?}", mount_point);
        let _ = std::process::Command::new("umount")
            .arg(mount_point)
            .output();

        println!("🗑️ rmdir {:?}", mount_point);
        let _ = std::process::Command::new("rmdir")
            .arg(mount_point)
            .output();

        // ✅ FORZA INVALIDAZIONE CACHE DIRECTORY PADRE
        // println!("🧹 Forza invalidazione cache directory padre...");
        // invalidate_directory_cache(parent_dir);

        // Attendi stabilizzazione più lunga
        std::thread::sleep(std::time::Duration::from_millis(1000));
    } else {
        println!("📁 Mount point not found in parent directory");
    }

    // Create directory mount
    println!("📁 Create directory mount: {:?}", mount_point);
    match std::fs::create_dir_all(mount_point) {
        Ok(_) => {
            println!("✅ Directory mount created");

            invalidate_directory_cache(parent_dir);
        }
        Err(e) => {
            eprintln!("❌ Error in creating directory: {}", e);
            std::process::exit(1);
        }
    }
}

fn check_if_mount_point_exists_in_parent(parent_dir: &std::path::Path, dir_name: &str) -> bool {
    println!("🔍 Searching for '{}' in {:?}", dir_name, parent_dir);

    // Method 1: reading directory
    match std::fs::read_dir(parent_dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                if let Some(entry_name) = entry.file_name().to_str() {
                    if entry_name == dir_name {
                        println!("  ✅ '{}' found with read_dir", dir_name);
                        return true;
                    }
                }
            }
            println!("  ❌ '{}' not found with read_dir", dir_name);
        }
        Err(e) => {
            println!("  ⚠️ Error read_dir: {}", e);
        }
    }

    // Method 2: ls command as fallback
    match std::process::Command::new("ls")
        .arg("-1") // One column
        .arg(parent_dir)
        .output()
    {
        Ok(output) if output.status.success() => {
            let ls_output = String::from_utf8_lossy(&output.stdout);
            for line in ls_output.lines() {
                if line.trim() == dir_name {
                    println!("  ✅ '{}' found with ls", dir_name);
                    return true;
                }
            }
            println!("  ❌ '{}' found with ls", dir_name);
        }
        Ok(output) => {
            println!(
                "  ⚠️ ls failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Err(e) => {
            println!("  ⚠️ Error ls: {}", e);
        }
    }

    // Method 3: test direct path
    let full_path = parent_dir.join(dir_name);
    if full_path.exists() {
        println!("  ✅ '{}' found with path.exists", dir_name);
        return true;
    }

    println!("  ❌ '{}' not found with any method", dir_name);
    false
}

fn invalidate_directory_cache(dir_path: &std::path::Path) {
    println!("🧹 Invalidazione cache per: {:?}", dir_path);

    // Metodo 1: sync per forzare flush filesystem
    let _ = std::process::Command::new("sync").output();

    // Metodo 2: touch directory per aggiornare timestamp
    let _ = std::process::Command::new("touch").arg(dir_path).output();

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
