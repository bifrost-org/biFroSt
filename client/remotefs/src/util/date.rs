// Converts datetime ISO in server requested format
pub fn format_datetime(iso_datetime: &str) -> String {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso_datetime) {
        dt.with_timezone(&chrono::Utc)
            .format("%Y-%m-%dT%H:%M:%S.000Z")
            .to_string()
    } else {
        chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S.000Z")
            .to_string()
    }
}
