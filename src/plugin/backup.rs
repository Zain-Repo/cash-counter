use super::*;

impl Server {
    pub(super) fn backup_status(&self) -> Result<Value, RpcError> {
        let s = BackupService::open(&self.database_path)
            .map_err(RpcError::internal)?
            .status()
            .map_err(RpcError::internal)?;
        Ok(backup_json(&s))
    }
    pub(super) fn save_backup_settings(&self, a: &Value) -> Result<Value, RpcError> {
        let mode = match string(a, "mode")? {
            "manual" => BackupMode::Manual,
            "automatic" => BackupMode::Automatic,
            _ => return Err(RpcError::invalid("mode must be manual or automatic")),
        };
        let frequency = match string(a, "frequency")? {
            "hourly" => BackupFrequency::Hourly,
            "six_hours" => BackupFrequency::EverySixHours,
            "daily" => BackupFrequency::Daily,
            "weekly" => BackupFrequency::Weekly,
            _ => return Err(RpcError::invalid("Invalid backup frequency")),
        };
        let mut s = BackupService::open(&self.database_path).map_err(RpcError::internal)?;
        let current = s.status().map_err(RpcError::internal)?;
        let clear_directory = a
            .get("clear_directory")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if clear_directory && a.get("directory").is_some() {
            return Err(RpcError::invalid(
                "Choose either directory or clear_directory=true",
            ));
        }
        let directory = if clear_directory {
            None
        } else {
            optional_string(a, "directory")?
                .map(PathBuf::from)
                .or(current.settings.directory)
        };
        let status = s
            .save_settings(
                &BackupSettings {
                    directory,
                    mode,
                    frequency,
                },
                now(),
            )
            .map_err(RpcError::internal)?;
        Ok(backup_json(&status))
    }
    pub(super) fn run_backup(&self) -> Result<Value, RpcError> {
        let _guard = self
            .background_operation
            .lock()
            .map_err(|_| RpcError::internal("Background operations are unavailable"))?;
        let mut s = BackupService::open(&self.database_path).map_err(RpcError::internal)?;
        let backup = s.run_backup(false, now()).map_err(RpcError::internal)?;
        Ok(backup.map(|b|json!({"created":true,"path":b.path,"size_bytes":b.size_bytes,"elapsed_ms":b.elapsed_ms})).unwrap_or_else(||json!({"created":false})))
    }
}
