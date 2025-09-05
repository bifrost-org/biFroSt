pub async fn run(disabled_autorun: bool) {

    if disabled_autorun {
        let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
        match std::fs::remove_file(format!(
            "{}/.config/systemd/user/bifrost.service",
            std::env::var("HOME").unwrap_or_else(|_| format!("/home/{}", user))
        )) {
            Ok(()) => println!("Servizio systemd disabilitato e rimosso (bifrost)."),
            Err(e) => eprintln!("Rimozione service fallita: {}", e),
        }
    }

    if let Ok(output) = std::process::Command::new("pgrep")
        .arg("bifrost")
        .output()
    {
        let pids = String::from_utf8_lossy(&output.stdout);
        for pid in pids.lines() {
            if let Ok(_) = std::process::Command::new("kill")
                .arg("-9")
                .arg(pid)
                .output()
            {
                println!("🗑️ Terminato processo bifrost con PID {}", pid);
            }
        }
    }
}