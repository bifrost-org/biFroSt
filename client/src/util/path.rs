pub fn get_parent_path(path: &str) -> String {
    if path == "/" {
        return "/".to_string();
    }

    let clean_path = path.trim_end_matches('/');

    if let Some(last_slash) = clean_path.rfind('/') {
        if last_slash == 0 {
            "/".to_string()
        } else {
            clean_path[..last_slash].to_string()
        }
    } else {
        "/".to_string()
    }
}

pub fn get_file_name(path: &str) -> String {
    if path == "/" {
        return "".to_string();
    }

    let clean_path = path.trim_end_matches('/');

    if let Some(last_slash) = clean_path.rfind('/') {
        clean_path[last_slash + 1..].to_string()
    } else {
        clean_path.to_string()
    }
}
