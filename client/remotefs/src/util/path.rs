use users::get_current_username;

pub fn get_current_user() -> String {
    let username_osstr = get_current_username().expect("Cannot get current username");
    let username = username_osstr.to_string_lossy();
    username.to_string()
}
