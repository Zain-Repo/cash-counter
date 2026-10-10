//! Shared desktop update preferences.
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct UpdatePreference {
    automatic: bool,
}

/// Resolve the established desktop update preference file.
pub fn update_preference_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CASH_LEDGER_UPDATE_PREFERENCES") {
        if path.is_empty() {
            return Err("The configured update preferences path is empty.".into());
        }
        return Ok(PathBuf::from(path));
    }
    directories::ProjectDirs::from("ca", "Local", "Cash Register")
        .map(|directories| directories.data_local_dir().join("updates.json"))
        .ok_or_else(|| "Could not find the update preferences directory.".into())
}

/// Read automatic-update policy, defaulting to enabled only when absent.
pub fn read_preference(path: &Path) -> Result<bool, String> {
    match std::fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(_) => {
            return Err(
                "Could not read update preferences. Automatic installation is paused.".into(),
            );
        }
        Ok(metadata) if metadata.len() > 4096 => {
            return Err(
                "Update preferences are invalid. Choose your update preference again.".into(),
            );
        }
        Ok(_) => {}
    }
    let bytes = std::fs::read(path)
        .map_err(|_| "Could not read update preferences. Automatic installation is paused.")?;
    serde_json::from_slice::<UpdatePreference>(&bytes)
        .map(|preference| preference.automatic)
        .map_err(|_| "Update preferences are invalid. Choose your update preference again.".into())
}

/// Atomically persist automatic-update policy.
pub fn write_preference(path: &Path, automatic: bool) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("The update preferences path is invalid.")?;
    std::fs::create_dir_all(parent).map_err(|_| "Could not save update preferences.")?;
    let temporary = parent.join(format!("updates-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "Could not save update preferences.")?;
        let bytes = serde_json::to_vec(&UpdatePreference { automatic })
            .map_err(|_| "Could not save update preferences.")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Could not save update preferences.")?;
        drop(file);
        std::fs::rename(&temporary, path).map_err(|_| "Could not save update preferences.")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(str::to_string)
}
