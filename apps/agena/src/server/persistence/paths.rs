use std::path::PathBuf;

/// Agena server state has one current location and one database name.
pub(crate) const SERVER_DB_FILE: &str = "agena.db";

pub(crate) fn server_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("AGENA_SERVER_DATA_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    crate::server::path_utils::config_home_dir().join("agena")
}

pub(crate) fn server_state_db_path() -> PathBuf {
    server_data_dir().join(SERVER_DB_FILE)
}
