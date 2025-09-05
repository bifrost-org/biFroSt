use std::path::PathBuf;

use fuser::{ mount2, MountOption };
use bifrost::{
    api::client::RemoteClient,
    config::settings::Config,
    fs::operations::RemoteFileSystem,
    util::auth::UserKeys,
};

pub async fn run(detached: bool, enable_service: bool) {



    let config = match Config::from_file() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };

        if enable_service {
            let exe = std::env::current_exe().expect("cannot get current exe path");
            match install_systemd_user_service("bifrost", &exe) {
                Ok(()) => println!("Servizio systemd installato e abilitato (bifrost)."),
                Err(e) => eprintln!("Installazione service fallita: {}", e),
            }
        }





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


fn install_systemd_user_service(service_name: &str, exec: &std::path::Path) -> Result<(), String> {
    use std::io::Write;

    let home = std::env::var("HOME").map_err(|e| format!("HOME not set: {}", e))?;
    let dir = format!("{}/.config/systemd/user", home);
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir failed: {}", e))?;

    let unit_path = format!("{}/{}.service", dir, service_name);
    let tmp_path = format!("{}.tmp", &unit_path);

    // Ensure exec path is absolute
    let exec_path = exec
        .canonicalize()
        .unwrap_or_else(|_| exec.to_path_buf())
        .to_string_lossy()
        .into_owned();

    // Compose a clean unit file. Do NOT daemonize in the ExecStart: systemd manages the process.
    let work_dir = exec.parent().unwrap_or(std::path::Path::new("/")).to_string_lossy().into_owned();
    let content = format!(
        r#"[Unit]
Description=Bifrost RemoteFS Client (user)
After=network.target
# limita i tentativi: al massimo 5 riavvii in 10 minuti
StartLimitIntervalSec=600
StartLimitBurst=5

[Service]
Type=simple
WorkingDirectory={work_dir}
Environment=HOME={home}
ExecStart={exec} start
# restart solo se fallisce (exit ≠ 0), non sempre
Restart=on-failure
# attende 10s prima di provare a riavviare
RestartSec=10
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
"#,
        exec = exec_path,
        home = home,
        work_dir = work_dir,
    );

    // Write atomically: tmp file then rename
    std::fs::write(&tmp_path, content.as_bytes())
        .map_err(|e| format!("write temp unit failed {}: {}", tmp_path, e))?;
    // set permissions to 0644
    let mut perms = std::fs::metadata(&tmp_path)
        .map_err(|e| format!("stat tmp file failed: {}", e))?
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o644);
        std::fs::set_permissions(&tmp_path, perms)
            .map_err(|e| format!("set_permissions failed: {}", e))?;
    }

    std::fs::rename(&tmp_path, &unit_path)
        .map_err(|e| format!("rename unit failed {} -> {}: {}", tmp_path, unit_path, e))?;

    // reload and enable using --user (no sudo)
    let status = std::process::Command::new("systemctl")
        .arg("--user")
        .arg("daemon-reload")
        .status()
        .map_err(|e| format!("systemctl --user daemon-reload failed: {}", e))?;
    if !status.success() {
        return Err("systemctl --user daemon-reload failed".into());
    }
    let status = std::process::Command::new("systemctl")
        .arg("--user")
        .arg("enable")
        .arg("--now")
        .arg(service_name)
        .status()
        .map_err(|e| format!("systemctl --user enable failed: {}", e))?;
    if !status.success() {
        return Err("systemctl --user enable --now failed".into());
    }
    Ok(())
}


pub fn counting_of_processes() -> usize {
    if let Ok(output) = std::process::Command::new("pgrep").arg("bifrost").output() {
        let pids = String::from_utf8_lossy(&output.stdout);
        return pids.lines().count();
    }
    0
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
        let _ = std::process::Command::new("umount").arg(mount_point).output();

        println!("🗑️ rmdir {:?}", mount_point);
        let _ = std::process::Command::new("rmdir").arg(mount_point).output();
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
