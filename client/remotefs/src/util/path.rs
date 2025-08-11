pub fn get_parent_path(path: &str) -> String {
    if path == "/" {
        return "/".to_string();
    }

    // Rimuovi trailing slash se presente
    let clean_path = path.trim_end_matches('/');

    // Se il path inizia con '/', trova l'ultimo '/'
    if let Some(last_slash) = clean_path.rfind('/') {
        if last_slash == 0 {
            // Se l'ultimo slash è all'inizio, siamo nella root
            "/".to_string()
        } else {
            clean_path[..last_slash].to_string()
        }
    } else {
        // Nessun slash trovato, restituisci root
        "/".to_string()
    }
}

pub fn get_file_name(path: &str) -> String {
    if path == "/" {
        return "".to_string();
    }

    // Rimuovi trailing slash se presente
    let clean_path = path.trim_end_matches('/');

    // Trova l'ultimo '/' e prendi tutto quello che segue
    if let Some(last_slash) = clean_path.rfind('/') {
        clean_path[last_slash + 1..].to_string()
    } else {
        // Nessun slash, restituisci l'intero path
        clean_path.to_string()
    }
}
