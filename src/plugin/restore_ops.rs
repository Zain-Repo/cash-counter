use super::*;
use crate::restore;

impl Server {
    pub(super) fn restore_backup(&mut self, a: &Value) -> Result<Value, RpcError> {
        if a.get("confirm").and_then(Value::as_bool) != Some(true) {
            return Err(RpcError::invalid(
                "Set confirm=true to replace the live ledger from the selected backup",
            ));
        }
        let selection = PathBuf::from(string(a, "path")?);
        let token = string(a, "preview_token")?;
        let Some((expected_token, preview)) = self.restore_preview.take() else {
            return Err(RpcError::invalid(
                "Run restore_preview and review its result first",
            ));
        };
        if token != expected_token
            || selection.canonicalize().ok().as_deref() != Some(preview.backup.as_path())
        {
            return Err(RpcError::invalid(
                "Restore confirmation does not match the latest preview",
            ));
        }
        let current =
            restore::preview_restore(&self.database_path, &selection).map_err(RpcError::invalid)?;
        if current != preview {
            return Err(RpcError::invalid(
                "The selected backup changed after preview. Run restore_preview again.",
            ));
        }
        self.authenticated = false;
        self.authenticated_signal.store(false, Ordering::Release);
        drop(self.database()?);
        let _guard = self
            .background_operation
            .lock()
            .map_err(|_| RpcError::internal("Background operations are unavailable"))?;
        let summary = restore::restore_database(&self.database_path, &preview.backup)
            .map_err(RpcError::internal)?;
        self.authenticated = false;
        self.authenticated_signal.store(false, Ordering::Release);
        self.close_preview = None;
        self.prepared_update = None;
        Ok(
            json!({"restored":true,"restored_from":summary.restored_from,"safety_backup":summary.safety_backup}),
        )
    }
    pub(super) fn restore_preview(&mut self, a: &Value) -> Result<Value, RpcError> {
        let selection = PathBuf::from(string(a, "path")?);
        let preview =
            restore::preview_restore(&self.database_path, &selection).map_err(RpcError::invalid)?;
        let token = uuid::Uuid::new_v4().to_string();
        let result = json!({"preview_token":token,"backup":preview.backup,"size_bytes":preview.size_bytes,"sha256":preview.sha256,"modified_unix_nanos":preview.modified_unix_nanos,"confirm_required":true});
        self.restore_preview = Some((token, preview));
        Ok(result)
    }
}
