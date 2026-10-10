//! Restore work pauses local writes and requires a new sign-in after replacing credentials.

use super::backup_views::display_backup_path;
use super::*;
use cash_counter::restore::{resolve_backup, restore_database};

impl RegisterApp {
    fn restore_available(&self) -> bool {
        self.access == Access::Authenticated
            && !self.busy
            && !self.backup_working
            && !self.backups.loading
            && !self.restore_working
            && !self.updates.installing
    }

    pub(super) fn choose_restore_source(
        &mut self,
        folder: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.restore_available() {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: !folder,
            directories: folder,
            multiple: false,
            prompt: Some(
                if folder {
                    "Choose backup folder"
                } else {
                    "Choose backup file"
                }
                .into(),
            ),
        });
        self.backups.loading = true;
        self.restore_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = picker.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.backups.loading = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            view.restore_source.update(cx, |input, cx| {
                                input.set_value(path.to_string_lossy().into_owned(), window, cx)
                            });
                            view.backups.error = None;
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => {
                        view.backups.error = Some(
                            "Could not open the backup picker. Try again or enter the path.".into(),
                        )
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn use_last_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.restore_available() {
            return;
        }
        if let Some(path) = self
            .backups
            .status
            .as_ref()
            .and_then(|status| status.last_backup_path.as_ref())
        {
            self.restore_source.update(cx, |input, cx| {
                input.set_value(display_backup_path(path), window, cx)
            });
            self.backups.error = None;
            cx.notify();
        }
    }

    pub(super) fn request_restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.restore_available() {
            return;
        }
        if self.failed_operation.is_some() || self.has_update_draft(cx) || self.dashboard.working {
            self.backups.error = Some(
                "Save or discard your changes and wait for active work to finish before restoring."
                    .into(),
            );
            cx.notify();
            return;
        }
        let selection = self.restore_source.read(cx).value().trim().to_owned();
        if selection.is_empty() {
            self.backups.error = Some("Choose a backup file or folder first.".into());
            cx.notify();
            return;
        }
        let path = self.path.clone();
        let work = cx.background_executor().spawn(async move {
            let path = path?;
            resolve_backup(&PathBuf::from(selection), &path).map(|backup| (path, backup))
        });
        self.busy = true;
        self.restore_working = true;
        self.backups.error = None;
        self.backups.notice = None;
        self.restore_task = Some(cx.spawn_in(window, async move |view, cx| {
            let (path, backup) = match work.await {
                Ok(paths) => paths,
                Err(error) => {
                    let _ = view.update_in(cx, |view, _, cx| view.finish_restore_error(error, cx));
                    return;
                }
            };
            let confirmation = view.update_in(cx, |_, window, cx| {
                window.prompt(PromptLevel::Warning, "Restore this backup?",
                    Some(&format!("Restore from {}. This replaces current ledger data, company details and administrator accounts. A safety copy will be saved first. You will need to sign in again.", display_backup_path(&backup))),
                    &[PromptButton::cancel("Cancel"), PromptButton::new("Restore data")], cx)
            });
            let Ok(confirmation) = confirmation else { return; };
            if confirmation.await != Ok(1) {
                let _ = view.update_in(cx, |view, window, cx| {
                    view.busy = false;
                    view.restore_working = false;
                    view.resume_ready_update(window, cx);
                    cx.notify();
                });
                return;
            }
            let service = match view.update_in(cx, |view, _, _| {
                view.clear_dashboard();
                view.service.clone()
            }) {
                Ok(service) => service,
                Err(_) => return,
            };
            let result = cx.background_executor().spawn(async move {
                let mut service = service.lock().map_err(|_| "Storage is unavailable. Restart the application.".to_string())?;
                // Closing this connection lets SQLite obtain an exclusive restore lock.
                *service = None;
                restore_database(&path, &backup)
            }).await;
            let _ = view.update_in(cx, |view, window, cx| {
                match result {
                    Ok(summary) => {
                        view.busy = false;
                        view.restore_working = false;
                        view.logout(window, cx);
                        view.mode = FormMode::Entry;
                        view.name.update(cx, |input, cx| input.set_value("", window, cx));
                        view.profile = None;
                        view.access = Access::Loading;
                        view.username.update(cx, |input, cx| input.set_value("", window, cx));
                        view.notice = Some(format!("Data restored. Sign in with the account from the backup. Your previous data is saved at {}.", display_backup_path(&summary.safety_backup)));
                        view.load_company(window, cx);
                    }
                    Err(error) => view.finish_restore_error(error, cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn finish_restore_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.busy = false;
        self.restore_working = false;
        self.backups.error = Some(error);
        cx.notify();
    }
}
