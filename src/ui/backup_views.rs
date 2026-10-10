//! Window-owned backup tasks share one saved configuration across manual and scheduled runs.

use super::*;
use cash_counter::backup_service::{
    BackupFrequency, BackupMode, BackupService, BackupSettings, BackupStatus,
};
use std::time::Duration;

#[derive(Default)]
pub(super) struct Backups {
    pub status: Option<BackupStatus>,
    pub loading: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
}

impl RegisterApp {
    pub(super) fn load_backup_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.access != Access::Authenticated || self.backups.loading || self.backup_working {
            return;
        }
        self.backups.loading = true;
        let path = self.path.clone();
        let work = cx
            .background_executor()
            .spawn(async move { BackupService::open(&path?).and_then(|service| service.status()) });
        self.backup_settings_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.backups.loading = false;
                match result {
                    Ok(status) => {
                        view.backups.error = None;
                        view.backups.status = Some(status);
                        view.discard_backup_settings(window, cx);
                    }
                    Err(error) => {
                        view.backups.error = Some(format!(
                            "{error} Open Settings and retry loading backup preferences."
                        ))
                    }
                }
                view.start_backup_timer(window, cx);
                view.run_configured_backup(true, window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn start_backup_timer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.backup_timer.is_some() {
            return;
        }
        self.backup_timer = Some(cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(30))
                    .await;
                let stop = view.update_in(cx, |view, window, cx| {
                    if view.access != Access::Authenticated {
                        return true;
                    }
                    if view.updates.installing {
                        return false;
                    }
                    if view.backups.status.is_none() {
                        view.load_backup_settings(window, cx);
                    } else {
                        view.run_configured_backup(true, window, cx);
                    }
                    false
                });
                if stop.unwrap_or(true) {
                    break;
                }
            }
        }));
    }

    pub(super) fn discard_backup_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(status) = &self.backups.status else {
            return;
        };
        self.backup_automatic = status.settings.mode == BackupMode::Automatic;
        let directory = status
            .settings
            .directory
            .as_ref()
            .map(|path| display_backup_path(path))
            .unwrap_or_default();
        self.backup_directory
            .update(cx, |input, cx| input.set_value(directory, window, cx));
        let frequency = status.settings.frequency.label();
        self.backup_frequency.update(cx, |select, cx| {
            select.set_selected_value(&frequency, window, cx)
        });
        cx.notify();
    }

    pub(super) fn backup_settings_dirty(&self, cx: &App) -> bool {
        let Some(status) = &self.backups.status else {
            return false;
        };
        let mut saved = status.settings.clone();
        saved.directory = saved
            .directory
            .as_ref()
            .map(|path| PathBuf::from(display_backup_path(path)));
        self.edited_backup_settings(cx)
            .map_or(true, |settings| settings != saved)
    }

    fn edited_backup_settings(&self, cx: &App) -> Result<BackupSettings, String> {
        let directory = self.backup_directory.read(cx).value();
        let directory = directory.trim();
        if self.backup_automatic && directory.is_empty() {
            return Err("Choose a backup folder before enabling automatic backups.".into());
        }
        let frequency = match self.backup_frequency.read(cx).selected_value().copied() {
            Some("Hourly") => BackupFrequency::Hourly,
            Some("Every 6 hours") => BackupFrequency::EverySixHours,
            Some("Daily") => BackupFrequency::Daily,
            Some("Weekly") => BackupFrequency::Weekly,
            _ => return Err("Choose how often to back up.".into()),
        };
        Ok(BackupSettings {
            directory: (!directory.is_empty()).then(|| PathBuf::from(directory)),
            mode: if self.backup_automatic {
                BackupMode::Automatic
            } else {
                BackupMode::Manual
            },
            frequency,
        })
    }

    pub(super) fn toggle_backup_mode(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.backups.loading || self.backup_working {
            return;
        }
        self.backup_automatic = !self.backup_automatic;
        cx.notify();
    }

    pub(super) fn choose_backup_directory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy
            || self.backups.loading
            || self.backup_working
            || self.access != Access::Authenticated
        {
            return;
        }
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose backup folder".into()),
        });
        self.backups.loading = true;
        self.backup_settings_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = picker.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.backups.loading = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            view.backup_directory.update(cx, |input, cx| input.set_value(path.to_string_lossy().into_owned(), window, cx));
                            view.backups.error = None;
                        }
                    }
                    Ok(Ok(None)) => {},
                    _ => view.backups.error = Some("Could not open the folder picker. Enter the folder path or try Browse folder again.".into()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn save_backup_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy
            || self.backups.loading
            || self.backup_working
            || self.access != Access::Authenticated
        {
            return;
        }
        let settings = match self.edited_backup_settings(cx) {
            Ok(settings) => settings,
            Err(error) => {
                self.backups.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.backups.loading = true;
        self.backups.error = None;
        self.backups.notice = None;
        let path = self.path.clone();
        let work = cx.background_executor().spawn(async move {
            BackupService::open(&path?).and_then(|mut service| {
                service.save_settings(&settings, chrono::Utc::now().timestamp())
            })
        });
        self.backup_settings_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.backups.loading = false;
                match result {
                    Ok(status) => {
                        view.backups.status = Some(status);
                        view.discard_backup_settings(window, cx);
                        view.backups.notice = Some("Backup settings saved.".into());
                        view.run_configured_backup(true, window, cx);
                    }
                    Err(error) => view.backups.error = Some(error),
                }
                view.resume_ready_update(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn run_configured_backup(
        &mut self,
        automatic: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.access != Access::Authenticated
            || self.backup_working
            || self.backups.loading
            || self.busy
            || self.updates.installing
        {
            return;
        }
        let Some(status) = &self.backups.status else {
            if !automatic {
                self.load_backup_settings(window, cx);
            }
            return;
        };
        let now = chrono::Utc::now().timestamp();
        if automatic
            && (status.settings.mode != BackupMode::Automatic
                || status.next_due_at.is_none_or(|due| due > now))
        {
            return;
        }
        if !automatic && self.backup_settings_dirty(cx) {
            self.backups.error =
                Some("Save or discard your backup changes before backing up.".into());
            cx.notify();
            return;
        }
        if status.settings.directory.is_none() {
            self.backups.error =
                Some("Choose a backup folder in Settings and save it before backing up.".into());
            cx.notify();
            return;
        }
        let path = self.path.clone();
        let refresh_draft = !self.backup_settings_dirty(cx);
        self.backup_working = true;
        self.backups.error = None;
        if !automatic {
            self.backups.notice = None;
        }
        let work = cx.background_executor().spawn(async move {
            let mut service = BackupService::open(&path?)?;
            let result = service.run_backup(automatic, now);
            Ok::<_, String>((result, service.status()))
        });
        self.backup_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.backup_working = false;
                match result {
                    Ok((run, status)) => {
                        match status {
                            Ok(status) => {
                                view.backups.status = Some(status);
                                if refresh_draft {
                                    view.discard_backup_settings(window, cx);
                                }
                            }
                            Err(error) => view.backups.error = Some(error),
                        }
                        match run {
                            Ok(Some(summary)) => view.backups.notice = Some(format!("Verified backup saved to {} ({} bytes).", display_backup_path(&summary.path), summary.size_bytes)),
                            Ok(None) if !automatic => view.backups.notice = Some("Another instance is backing up. Wait for it to finish, then retry.".into()),
                            Ok(None) => {},
                            Err(error) => view.backups.error = Some(error),
                        }
                    }
                    Err(error) => view.backups.error = Some(error),
                }
                view.resume_ready_update(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }
}

pub(super) fn display_backup_path(path: &std::path::Path) -> String {
    let value = path.to_string_lossy();
    if cfg!(windows) {
        if let Some(network) = value.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{network}");
        }
        if let Some(local) = value.strip_prefix(r"\\?\") {
            return local.to_owned();
        }
    }
    value.into_owned()
}
