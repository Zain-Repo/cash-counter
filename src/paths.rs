use std::path::PathBuf;

/// Resolve the shared SQLite ledger location used by the desktop app and integrations.
pub fn database_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CASH_REGISTER_DB") {
        if path.is_empty() {
            return Err("The configured database path is empty.".into());
        }
        return Ok(PathBuf::from(path));
    }
    directories::ProjectDirs::from("ca", "Local", "Cash Register")
        .map(|directories| directories.data_local_dir().join("register.sqlite3"))
        .ok_or_else(|| "Could not find the local application-data directory.".into())
}
