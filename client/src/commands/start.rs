use std::path::PathBuf;

use fuser::{mount2, MountOption};
use remotefs::{
    api::client::RemoteClient, config::settings::Config, fs::operations::RemoteFileSystem,
    util::auth::UserKeys,
};

pub async fn run() {
    let config = match Config::from_file() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

    println!("🚀 Start bifrost...");
    println!("📡 Server: {}", config.server_full_url());
    println!("📁 Mount point: {:?}", config.mount_point);

    prepare_mount_point(&config.mount_point);

    let user_keys = UserKeys::load_from_files().expect("User keys not found");

    // FILESYSTEM AND MOUNT
    let filesystem = RemoteFileSystem::new(RemoteClient::new(&config, Some(user_keys)));
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
        println!("📡 Start mount2 in spawn_blocking...");
        mount2(filesystem, &mount_point_clone, &options)
    });

    // ✅ WAIT RESULT
    match mount_task.await {
        Ok(Ok(())) => println!("✅ Mount ended"),
        Ok(Err(e)) => eprintln!("❌ Mount error: {}", e),
        Err(e) => eprintln!("❌ Task error: {}", e),
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
    } else {
        println!("📁 Mount point not found in parent directory");
    }

    // Create directory mount
    println!("📁 Create directory mount: {:?}", mount_point);
    match std::fs::create_dir_all(mount_point) {
        Ok(_) => {
            println!("✅ Directory mount created");
        }
        Err(e) => {
            eprintln!("❌ Error in creating directory: {}", e);
            std::process::exit(1);
        }
    }
}

fn check_if_mount_point_exists_in_parent(parent_dir: &std::path::Path, dir_name: &str) -> bool {
    println!("🔍 Searching for '{}' in {:?}", dir_name, parent_dir);

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

    false
}
