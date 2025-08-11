use users::get_current_username;

// Converts permissions from octal to string
pub fn format_permissions(perm: &str) -> String {
    // If it is already a valid octal (3 digits) than return it
    if perm.len() == 3 && perm.chars().all(|c| c.is_ascii_digit() && c <= '7') {
        return perm.to_string();
    }

    // From 'rwx' format to octal string
    if perm.len() == 9 && (perm.starts_with('r') || perm.starts_with('-')) {
        return symbolic_to_octal(perm);
    }

    // From decimal to octal string
    if let Ok(decimal_perm) = perm.parse::<u32>() {
        if decimal_perm <= 777 && decimal_perm.to_string().chars().all(|c| c <= '7') {
            return format!("{:03}", decimal_perm);
        }
        return format!("{:03o}", decimal_perm);
    }

    // ?
    match perm {
        "rw-r--r--" => "644",
        "rwxr-xr-x" => "755",
        "rw-------" => "600",
        "rwxrwxrwx" => "777",
        "r--r--r--" => "444",
        "rwxrwxr-x" => "775",
        _ => "644",
    }
    .to_string()
}

// Converts from 'rwx' format to octal string
pub fn symbolic_to_octal(symbolic: &str) -> String {
    let mut octal = 0;

    // Owner
    if symbolic.chars().nth(0) == Some('r') {
        octal += 400;
    }
    if symbolic.chars().nth(1) == Some('w') {
        octal += 200;
    }
    if symbolic.chars().nth(2) == Some('x') {
        octal += 100;
    }

    // Group
    if symbolic.chars().nth(3) == Some('r') {
        octal += 40;
    }
    if symbolic.chars().nth(4) == Some('w') {
        octal += 20;
    }
    if symbolic.chars().nth(5) == Some('x') {
        octal += 10;
    }

    // Others
    if symbolic.chars().nth(6) == Some('r') {
        octal += 4;
    }
    if symbolic.chars().nth(7) == Some('w') {
        octal += 2;
    }
    if symbolic.chars().nth(8) == Some('x') {
        octal += 1;
    }

    format!("{:03o}", octal)
}

pub fn get_current_user() -> String {
    let username_osstr = get_current_username().expect("Cannot get current username");
    let username = username_osstr.to_string_lossy();
    username.to_string()
}
