//! Update tasks have an independent lifecycle so checking never cancels ledger writes.

use super::*;
use cash_counter::updater::{self, AvailableUpdate, PreparedUpdate};
use gpui_component::{
    Disableable,
    alert::Alert,
    button::{Button, ButtonVariants},
    h_flex,
    switch::Switch,
    v_flex,
};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path, time::Duration};

pub(super) struct Updates {
    pub enabled: bool,
    pub automatic: bool,
    pub working: bool,
    pub installing: bool,
    pub status: String,
    pub error: Option<String>,
    preference_error: Option<String>,
    failure_notice: Option<String>,
    failure_pause: bool,
    pub available: Option<AvailableUpdate>,
    pub prepared: Option<PreparedUpdate>,
    preference_path: Result<PathBuf, String>,
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            enabled: false,
            automatic: true,
            working: false,
            installing: false,
            status: "Updates are available in the installed Windows app.".into(),
            error: None,
            preference_error: None,
            failure_notice: None,
            failure_pause: false,
            available: None,
            prepared: None,
            preference_path: update_preference_path(),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct UpdatePreference {
    automatic: bool,
}

fn update_preference_path() -> Result<PathBuf, String> {
    // Retain the established storage namespace when the product display name changes.
    directories::ProjectDirs::from("ca", "Local", "Cash Register")
        .map(|directories| directories.data_local_dir().join("updates.json"))
        .ok_or_else(|| "Could not find the update preferences directory.".into())
}

fn read_preference(path: &Path) -> Result<bool, String> {
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

fn write_preference(path: &Path, automatic: bool) -> Result<(), String> {
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

/// A restart must never interrupt a write, a failed retry payload, or an unsaved draft.
fn restart_is_safe(busy: bool, failed_write: bool, draft: bool, installing: bool) -> bool {
    !busy && !failed_write && !draft && !installing
}

impl RegisterApp {
    pub(super) fn initialize_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.updates.enabled = updater::is_installed();
        if !self.updates.enabled {
            return;
        }
        match self
            .updates
            .preference_path
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|path| read_preference(path))
        {
            Ok(automatic) => self.updates.automatic = automatic,
            Err(error) => {
                self.updates.automatic = false;
                self.updates.preference_error = Some(error);
            }
        }
        if let Some(error) = updater::read_last_failure() {
            self.updates.failure_pause = true;
            self.updates.failure_notice = Some(format!(
                "The previous update failed. Automatic installation is paused; use Download, install & restart to retry. {error}"
            ));
        }
        self.check_updates(window, cx);
    }

    pub(super) fn check_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.updates.enabled || self.updates.working || self.updates.installing {
            return;
        }
        self.updates.working = true;
        self.updates.error = None;
        self.update_idle_task = None;
        self.updates.status = "Checking for updates…".into();
        let work = cx.background_executor().spawn(async { updater::check() });
        self.update_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.updates.working = false;
                match result {
                    Ok(Some(update)) => {
                        if view
                            .updates
                            .available
                            .as_ref()
                            .is_none_or(|previous| previous.version != update.version)
                        {
                            view.updates.prepared = None;
                        }
                        view.updates.status = format!("Version {} is available.", update.version);
                        view.updates.available = Some(update);
                        if view.updates.automatic && !view.updates.failure_pause {
                            view.download_update(window, cx);
                        }
                    }
                    Ok(None) => {
                        view.updates.available = None;
                        view.updates.prepared = None;
                        view.updates.status = "Cash Ledger is up to date.".into();
                    }
                    Err(error) => {
                        view.updates.status =
                            "Could not check for updates. Try Check for updates again.".into();
                        view.updates.error = Some(error);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn download_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.updates.enabled || self.updates.working || self.updates.installing {
            return;
        }
        self.updates.failure_pause = false;
        self.updates.failure_notice = None;
        if self.updates.prepared.is_some() {
            self.install_update(window, cx);
            return;
        }
        let Some(update) = self.updates.available.clone() else {
            return;
        };
        self.updates.working = true;
        self.updates.error = None;
        self.updates.status = format!("Downloading and verifying version {}…", update.version);
        let work = cx
            .background_executor()
            .spawn(async move { updater::download(&update) });
        self.update_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.updates.working = false;
                match result {
                    Ok(prepared) => {
                        view.updates.prepared = Some(prepared);
                        view.updates.status =
                            "Update ready. Finish or save your work before restarting.".into();
                        if view.updates.automatic {
                            view.wait_for_idle_update(window, cx);
                        } else {
                            view.install_update(window, cx);
                        }
                    }
                    Err(error) => {
                        view.updates.status =
                            "Download failed. Your current version is unchanged; try again.".into();
                        view.updates.error = Some(error);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn has_update_draft(&self, cx: &App) -> bool {
        if matches!(
            self.access,
            Access::Loading | Access::Company | Access::Administrator
        ) {
            return true;
        }
        if self.access == Access::Login {
            return !self.username.read(cx).value().is_empty()
                || !self.password.read(cx).value().is_empty();
        }
        let register_draft = match self.mode {
            FormMode::Entry => {
                !self.amount.read(cx).value().is_empty() || !self.reason.read(cx).value().is_empty()
            }
            FormMode::Setup => !self.name.read(cx).value().is_empty(),
            FormMode::Open => {
                !self.amount.read(cx).value().is_empty()
                    || self.date.read(cx).value().as_ref()
                        != Local::now().format("%Y-%m-%d").to_string()
            }
            FormMode::Close => {
                !self.count.read(cx).value().is_empty() || self.closing_review.is_some()
            }
        };
        let settings_draft = self.profile.as_ref().is_some_and(|profile| {
            self.company.read(cx).value().as_ref() != profile.name
                || self.address.read(cx).value().as_ref() != profile.address
                || self.rules.read(cx).value().as_ref() != profile.rules
                || self.printer.read(cx).value().as_ref() != profile.printer_name
                || self.paper.read(cx).selected_value().copied().unwrap_or("") != profile.paper_size
                || self.tolerance.read(cx).value().as_ref()
                    != cash_counter::register::money::format_amount(
                        profile.closing_variance_threshold_cents,
                    )
        });
        let report_draft = self.page == Page::Reports
            && self.report_data.as_ref().is_none_or(|report| {
                self.report_start.read(cx).value().as_ref() != report.filter.start_date
                    || self.report_end.read(cx).value().as_ref() != report.filter.end_date
                    || self.report_register != report.filter.register_id
            });
        register_draft || settings_draft || report_draft
    }

    pub(super) fn can_restart_for_update(&self, cx: &App) -> bool {
        restart_is_safe(
            self.busy || self.updates.working || self.backup_working,
            self.failed_operation.is_some(),
            self.has_update_draft(cx),
            self.updates.installing,
        )
    }

    pub(super) fn resume_ready_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.updates.automatic
            && !self.updates.failure_pause
            && self.updates.prepared.is_some()
            && self.can_restart_for_update(cx)
        {
            self.install_update(window, cx);
        }
    }

    fn wait_for_idle_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.can_restart_for_update(cx) {
            self.install_update(window, cx);
            return;
        }
        // Bounded, window-owned timer: after ten minutes the ready update stays available
        // for a manual restart, without keeping a worker thread or retrying network work.
        self.update_idle_task = Some(cx.spawn_in(window, async move |view, cx| {
            for _ in 0..120 {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                let stop = view.update_in(cx, |view, window, cx| {
                    if !view.updates.automatic
                        || view.updates.failure_pause
                        || view.updates.prepared.is_none()
                        || view.updates.installing
                    {
                        return true;
                    }
                    if view.can_restart_for_update(cx) {
                        view.install_update(window, cx);
                        return true;
                    }
                    false
                });
                if stop.unwrap_or(true) {
                    break;
                }
            }
        }));
    }

    pub(super) fn install_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.updates.enabled || !self.can_restart_for_update(cx) {
            self.updates.status =
                "Update ready. Finish or save your work before restarting.".into();
            cx.notify();
            return;
        }
        let Some(prepared) = self.updates.prepared.clone() else {
            return;
        };
        self.busy = true;
        self.updates.installing = true;
        self.updates.status = "Installing update and restarting Cash Ledger…".into();
        let work = cx
            .background_executor()
            .spawn(async move { updater::launch_update(&prepared) });
        self.update_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = work.await;
            let _ = view.update_in(cx, |view, _, cx| {
                match result {
                    Ok(()) => cx.quit(),
                    Err(error) => {
                        view.busy = false;
                        view.updates.installing = false;
                        view.updates.prepared = None;
                        view.updates.failure_pause = true;
                        view.updates.error = Some(error);
                        view.updates.status = "Could not start installation. Your current version is unchanged; try again.".into();
                        cx.notify();
                    }
                }
            });
        }));
        cx.notify();
    }

    fn toggle_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.updates.working || self.updates.installing {
            return;
        }
        let automatic = !self.updates.automatic;
        match self
            .updates
            .preference_path
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|path| write_preference(path, automatic))
        {
            Ok(()) => {
                self.updates.automatic = automatic;
                self.updates.error = None;
                self.updates.preference_error = None;
                if automatic {
                    self.updates.failure_pause = false;
                    self.updates.failure_notice = None;
                    if self.updates.prepared.is_some() {
                        self.wait_for_idle_update(window, cx);
                    } else if self.updates.available.is_some() {
                        self.download_update(window, cx);
                    }
                }
            }
            Err(error) => self.updates.error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn update_banner(&self) -> impl IntoElement {
        v_flex().children(self.updates.available.as_ref().map(|_| {
            div()
                .id("update-status")
                .role(Role::Status)
                .px_8()
                .py_3()
                .child(Alert::info(
                    "update-banner-message",
                    self.updates.status.clone(),
                ))
        }))
    }

    pub(super) fn updates_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let blocked = !self.updates.enabled || self.updates.working || self.updates.installing;
        v_flex().gap_3().mt_5().pt_5().border_t_1().border_color(rgb(0xdde3dc))
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child("Updates"))
            .child(div().child(format!("Cash Ledger · Version {}", env!("CARGO_PKG_VERSION"))))
            .child(div().id("settings-update-status").role(Role::Status).child(Alert::info("settings-update-message", self.updates.status.clone())))
            .children(self.updates.error.clone().map(|error| {
                div().id("update-error").role(Role::Alert).child(Alert::error("update-error-message", error))
            }))
            .children(self.updates.preference_error.clone().map(|error| {
                div().id("update-preference-error").role(Role::Alert).child(Alert::warning("update-preference-message", error))
            }))
            .children(self.updates.failure_notice.clone().map(|error| {
                div().id("previous-update-error").role(Role::Alert).child(Alert::warning("previous-update-message", error).title("Automatic installation paused"))
            }))
            .child(h_flex().gap_3().flex_wrap()
                .child(Switch::new("automatic-updates").label("Automatic updates")
                    .checked(self.updates.automatic).disabled(blocked)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_updates(window, cx))))
                .child(Button::new("check-updates").label("Check for updates").loading(self.updates.working).disabled(blocked)
                    .on_click(cx.listener(|this, _, window, cx| this.check_updates(window, cx))))
                .children(self.updates.available.as_ref().map(|_| {
                    Button::new("install-update").primary()
                        .label(if self.updates.prepared.is_some() { "Install & restart" } else { "Download, install & restart" })
                        .disabled(blocked || self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.download_update(window, cx)))
                })))
            .child(div().text_sm().text_color(rgb(0x61716a)).child("Installed copies check at startup. Automatic updates download, verify and restart when your work is saved. Ledger data stays on this computer."))
    }
}

#[cfg(test)]
mod tests {
    use super::{read_preference, restart_is_safe, write_preference};

    #[test]
    fn restart_requires_saved_idle_state() {
        assert!(restart_is_safe(false, false, false, false));
        for (busy, failed, draft, installing) in [
            (true, false, false, false),
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, true),
        ] {
            assert!(!restart_is_safe(busy, failed, draft, installing));
        }
    }

    #[test]
    fn preference_defaults_to_automatic_and_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("updates.json");
        assert!(read_preference(&path).unwrap());
        write_preference(&path, false).unwrap();
        assert!(!read_preference(&path).unwrap());
        write_preference(&path, true).unwrap();
        assert!(read_preference(&path).unwrap());
    }

    #[test]
    fn malformed_preferences_do_not_enable_automatic_installation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("updates.json");
        std::fs::write(&path, "invalid").unwrap();
        assert!(read_preference(&path).is_err());
        std::fs::write(&path, vec![b' '; 4097]).unwrap();
        assert!(read_preference(&path).is_err());
    }
}
