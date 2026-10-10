use super::*;
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable, IconName,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    setting::{SelectIndex, SettingGroup, SettingItem, SettingPage, Settings},
    switch::Switch,
    v_flex,
};

impl RegisterApp {
    pub(super) fn settings_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity();
        let company_page = SettingPage::new("Company")
            .icon(IconName::Building2)
            .description("Business details used on reports and receipts.")
            .group(
                SettingGroup::new()
                    .title("Business details")
                    .item(
                        SettingItem::render({
                            let company = self.company.clone();
                            let address = self.address.clone();
                            let disabled = self.busy;
                            move |_, _, _| {
                                v_flex().gap_4().child(
                                    gpui_component::form::v_form()
                                        .child(
                                            gpui_component::form::field()
                                                .label("Company name")
                                                .child(Input::new(&company).disabled(disabled)),
                                        )
                                        .child(
                                            gpui_component::form::field()
                                                .label("Company address")
                                                .child(Input::new(&address).disabled(disabled)),
                                        ),
                                )
                            }
                        })
                        .keywords([
                            "company name",
                            "company address",
                            "business",
                        ]),
                    )
                    .item(self.settings_save_item(
                        &app,
                        "save-company-settings",
                        "Save company settings",
                    )),
            );
        let cash_page = SettingPage::new("Cash handling")
            .icon(IconName::ChartPie)
            .description("Set the guidance and warning shown during cash close.")
            .group(
                SettingGroup::new()
                    .title("Closing guidance")
                    .item(SettingItem::render({
                        let rules = self.rules.clone();
                        let tolerance = self.tolerance.clone();
                        let disabled = self.busy;
                        move |_, _, _| {
                            v_flex()
                                .gap_4()
                                .child(gpui_component::form::v_form()
                                    .child(gpui_component::form::field().label("Cash handling rules").child(Input::new(&rules).disabled(disabled)))
                                    .child(gpui_component::form::field().label("Closing variance warning · CAD").child(Input::new(&tolerance).disabled(disabled))))
                                .child(div().text_sm().child("Warn when the final count differs by more than this amount. Rules are guidance for staff."))
                        }
                    }).keywords(["cash handling", "rules", "closing variance", "warning"]))
                    .item(self.settings_save_item(
                        &app,
                        "save-cash-settings",
                        "Save cash handling settings",
                    )),
            );
        let printing_page = SettingPage::new("Printing")
            .icon(IconName::Settings2)
            .description("Choose the printer and page size for reports.")
            .group(
                SettingGroup::new()
                    .title("Report printing")
                    .item(
                        SettingItem::render({
                            let printer = self.printer.clone();
                            let paper = self.paper.clone();
                            let disabled = self.busy;
                            move |_, _, _| {
                                v_flex()
                                    .gap_4()
                                    .child(
                                        gpui_component::form::v_form().child(
                                            gpui_component::form::field()
                                                .label("Printer name · optional")
                                                .child(Input::new(&printer).disabled(disabled)),
                                        ),
                                    )
                                    .child(
                                        gpui_component::form::v_form().child(
                                            gpui_component::form::field()
                                                .label("Paper size")
                                                .child(
                                                    gpui_component::select::Select::new(&paper)
                                                        .accessibility_label("Paper size")
                                                        .disabled(disabled)
                                                        .w_full(),
                                                ),
                                        ),
                                    )
                            }
                        })
                        .keywords(["printer", "paper size", "printing"]),
                    )
                    .item(self.settings_save_item(
                        &app,
                        "save-printing-settings",
                        "Save printing settings",
                    )),
            );
        let backups_page = self.backups_page(cx);
        let updates_page = self.updates_page(cx);

        div()
            .id("settings-scroll")
            .flex_1()
            .overflow_y_scroll()
            .p_5()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(980.))
                    .min_h(px(470.))
                    .gap_4()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(28.))
                            .child("Settings"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(crate::theme::MUTED_FOREGROUND))
                            .child("Company, cash handling, printing, backups and updates."),
                    )
                    .child(
                        div().w_full().h(px(560.)).child(
                            Settings::new(("workspace-settings", self.settings_generation))
                                .sidebar_width(px(205.))
                                .default_selected_index(SelectIndex {
                                    page_ix: self.settings_initial_page,
                                    group_ix: None,
                                })
                                .pages([
                                    company_page,
                                    cash_page,
                                    printing_page,
                                    backups_page,
                                    updates_page,
                                ]),
                        ),
                    )
                    .child(self.access_feedback()),
            )
            .into_any_element()
    }

    fn settings_save_item(
        &self,
        app: &Entity<RegisterApp>,
        id: &'static str,
        label: &'static str,
    ) -> SettingItem {
        let app = app.clone();
        let busy = self.busy;
        SettingItem::render(move |_, _, _| {
            h_flex().justify_end().child(
                Button::new(id)
                    .primary()
                    .icon(IconName::Check)
                    .label(label)
                    .loading(busy)
                    .disabled(busy)
                    .on_click({
                        let app = app.clone();
                        move |_, window, cx| {
                            cx.update_entity(&app, |this, cx| this.save_settings(window, cx))
                        }
                    }),
            )
        })
        .keywords([label, "save"])
    }

    fn backups_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let disabled = self.busy || self.backups.loading || self.backup_working;
        let directory = self.backup_directory.clone();
        let frequency = self.backup_frequency.clone();
        let app = cx.entity();
        let location_app = app.clone();
        let schedule_app = app.clone();
        let run_app = app.clone();
        let backup_auto = self.backup_automatic;
        let dirty = self.backup_settings_dirty(cx);
        let status = self.backups.status.as_ref();
        let last_success = status
            .and_then(|value| value.last_success_at)
            .map(|timestamp| {
                chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0)
                    .map(|value| {
                        value
                            .with_timezone(&Local)
                            .format("%b %-d, %Y at %-I:%M %p")
                            .to_string()
                    })
                    .unwrap_or_else(|| "Unknown".into())
            });
        let next_due = status.and_then(|value| value.next_due_at).map(|timestamp| {
            chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0)
                .map(|value| {
                    value
                        .with_timezone(&Local)
                        .format("%b %-d, %Y at %-I:%M %p")
                        .to_string()
                })
                .unwrap_or_else(|| "Unknown".into())
        });
        let backup_error = self
            .backups
            .error
            .clone()
            .or_else(|| status.and_then(|value| value.last_error.clone()));
        let notice = self.backups.notice.clone();
        let restore_error = backup_error.clone();
        let loading = self.backups.loading;
        let working = self.backup_working;
        let needs_load = self.backups.status.is_none();
        let load_app = app.clone();
        let restore_source = self.restore_source.clone();
        let restore_source_empty = restore_source.read(cx).value().trim().is_empty();
        let restore_disabled = self.busy || loading || working || self.restore_working;
        let restore_working = self.restore_working;
        let restore_file_app = app.clone();
        let restore_folder_app = app.clone();
        let restore_last_app = app.clone();
        let restore_app = app.clone();
        let has_last_backup = status
            .and_then(|value| value.last_backup_path.as_ref())
            .is_some();
        SettingPage::new("Backups")
            .icon(IconName::Settings2)
            .description("Keep a verified copy of your ledger in a folder you control.")
            .group(
                SettingGroup::new()
                    .title("Backup location and schedule")
                    .description("Automatic backups run while the app is open and signed in. They catch up after sign-in.")
                    .item(SettingItem::render(move |_, _, _| {
                        v_flex()
                            .gap_3()
                            .when(needs_load, |column| {
                                column.child(
                                    Button::new("retry-load-backup-settings")
                                        .label("Load backup settings")
                                        .loading(loading)
                                        .disabled(disabled)
                                        .on_click({
                                            let app = load_app.clone();
                                            move |_, window, cx| {
                                                cx.update_entity(&app, |this, cx| {
                                                    this.load_backup_settings(window, cx)
                                                });
                                            }
                                        }),
                                )
                            })
                            .child(gpui_component::form::v_form().child(gpui_component::form::field().label("Backup folder").child(Input::new(&directory).disabled(disabled))))
                            .child(h_flex().gap_2().flex_wrap()
                                .child(Button::new("browse-backup-folder").label("Browse folder").disabled(disabled)
                                .on_click({ let app = location_app.clone(); move |_, window, cx| { cx.update_entity(&app, |this, cx| this.choose_backup_directory(window, cx)); } }))
                                .child(Button::new("save-backup-settings").primary().label("Save backup settings").loading(loading).disabled(disabled)
                                .on_click({ let app = location_app.clone(); move |_, window, cx| { cx.update_entity(&app, |this, cx| this.save_backup_settings(window, cx)); } }))
                                .child(Button::new("discard-backup-settings").label("Discard changes").disabled(disabled || !dirty)
                                .on_click({ let app = location_app.clone(); move |_, window, cx| { cx.update_entity(&app, |this, cx| this.discard_backup_settings(window, cx)); } })))
                    }).keywords(["backup folder", "backup location", "browse", "save backup settings", "discard", "load backup settings", "retry"])),
            )
            .group(
                SettingGroup::new()
                    .title("Automatic backups")
                    .item(SettingItem::render(move |_, _, _| {
                        v_flex()
                            .gap_3()
                            .child(Switch::new("automatic-backups").label("Automatic backups").checked(backup_auto).disabled(disabled)
                                .on_click({ let app = schedule_app.clone(); move |_, window, cx| { cx.update_entity(&app, |this, cx| this.toggle_backup_mode(window, cx)); } }))
                            .child(gpui_component::form::v_form().child(gpui_component::form::field().label("Backup frequency").child(
                                gpui_component::select::Select::new(&frequency).accessibility_label("Backup frequency").disabled(disabled || !backup_auto).w_full(),
                            )))
                            .child(h_flex().gap_2().flex_wrap()
                                .child(Button::new("backup-now").primary().label("Back up now").loading(working).disabled(disabled || dirty)
                                    .on_click({ let app = run_app.clone(); move |_, window, cx| { cx.update_entity(&app, |this, cx| this.run_configured_backup(false, window, cx)); } })))
                            .children(last_success.as_ref().map(|value| div().text_sm().child(format!("Last successful backup · {value}"))))
                            .children(next_due.as_ref().map(|value| div().text_sm().child(format!("Next automatic backup · {value}"))))
                            .children(notice.clone().map(|value| div().id("backup-notice").role(Role::Status).text_sm().text_color(rgb(crate::theme::SUCCESS)).child(value)))
                            .children(backup_error.clone().map(|value| div().id("backup-error").role(Role::Alert).text_sm().text_color(rgb(0x9a3f32)).child(value)))
                            .when(!backup_auto, |this| this.child(div().text_sm().text_color(rgb(crate::theme::MUTED_FOREGROUND)).child("Automatic backups are off. Back up now creates a one-time manual snapshot.")))
                            .when(dirty, |this| this.child(div().text_sm().text_color(rgb(crate::theme::MUTED_FOREGROUND)).child("Save or discard changes before creating a backup.")))
                            .child(div().text_sm().text_color(rgb(crate::theme::MUTED_FOREGROUND)).child("Snapshots are verified and never replace an existing backup. Administrator credentials are included."))
                    }).keywords(["automatic backup", "schedule", "frequency", "back up now", "last backup", "next backup"])),
            )
            .group(
                SettingGroup::new()
                    .title("Restore data")
                    .description("Restore replaces your ledger, company details and administrator accounts. Current backup preferences are preserved, a safety copy is created first, and you will be signed out.")
                    .item(SettingItem::render(move |_, _, _| {
                        v_flex()
                            .gap_3()
                            .child(gpui_component::form::v_form().child(
                                gpui_component::form::field()
                                    .label("Backup file or folder")
                                    .child(Input::new(&restore_source).disabled(restore_disabled)),
                            ))
                            .child(h_flex().gap_2().flex_wrap()
                                .child(Button::new("choose-restore-file").label("Choose backup file").disabled(restore_disabled)
                                    .on_click({ let app = restore_file_app.clone(); move |_, window, cx| { cx.update_entity(&app, |this, cx| this.choose_restore_source(false, window, cx)); } }))
                                .child(Button::new("choose-restore-folder").label("Choose backup folder").disabled(restore_disabled)
                                    .on_click({ let app = restore_folder_app.clone(); move |_, window, cx| { cx.update_entity(&app, |this, cx| this.choose_restore_source(true, window, cx)); } }))
                                .child(Button::new("use-last-backup").label("Use last backup").disabled(restore_disabled || !has_last_backup)
                                    .on_click({ let app = restore_last_app.clone(); move |_, window, cx| { cx.update_entity(&app, |this, cx| this.use_last_backup(window, cx)); } })))
                            .child(h_flex().justify_end().child(
                                Button::new("restore-data").primary().label("Restore data").loading(restore_working)
                                    .disabled(restore_disabled || restore_source_empty)
                                    .on_click({ let app = restore_app.clone(); move |_, window, cx| { cx.update_entity(&app, |this, cx| this.request_restore(window, cx)); } }),
                            ))
                            .child(div().text_sm().text_color(rgb(crate::theme::MUTED_FOREGROUND)).child("Choose a backup file, or choose a folder to restore its latest generated backup."))
                            .children(restore_error.clone().map(|message| div().id("restore-error").role(Role::Alert).text_sm().text_color(rgb(0x9a3f32)).child(message)))
                    }).keywords(["restore data", "restore backup", "choose backup file", "choose backup folder", "last backup", "ledger", "safety copy"]))
            )
    }

    fn updates_page(&self, cx: &mut Context<Self>) -> SettingPage {
        let app = cx.entity();
        let blocked = !self.updates.enabled || self.updates.working || self.updates.installing;
        let automatic = self.updates.automatic;
        let status = self.updates.status.clone();
        let error = self.updates.error.clone();
        let preference_error = self.updates.preference_error.clone();
        let failure_notice = self.updates.failure_notice.clone();
        let available = self.updates.available.is_some();
        let prepared = self.updates.prepared.is_some();
        let busy = self.busy;
        SettingPage::new("Updates")
            .icon(IconName::Settings2)
            .description("Keep Cash Ledger current while protecting unsaved work.")
            .group(
                SettingGroup::new()
                    .title("Application updates")
                    .item(SettingItem::render(move |_, _, _| {
                        v_flex()
                            .gap_3()
                            .child(div().child(format!("Cash Ledger · Version {}", env!("CARGO_PKG_VERSION"))))
                            .child(div().id("settings-update-status").role(Role::Status).child(gpui_component::alert::Alert::info("settings-update-message", status.clone())))
                            .children(error.clone().map(|message| div().id("update-error").role(Role::Alert).child(gpui_component::alert::Alert::error("update-error-message", message))))
                            .children(preference_error.clone().map(|message| div().id("update-preference-error").role(Role::Alert).child(gpui_component::alert::Alert::warning("update-preference-message", message))))
                            .children(failure_notice.clone().map(|message| div().id("previous-update-error").role(Role::Alert).child(gpui_component::alert::Alert::warning("previous-update-message", message).title("Automatic installation paused"))))
                            .child(Switch::new("automatic-updates").label("Automatic updates").checked(automatic).disabled(blocked)
                                .on_click({ let app = app.clone(); move |_, window, cx| { cx.update_entity(&app, |this, cx| this.toggle_updates(window, cx)); } }))
                            .child(h_flex().gap_2().flex_wrap()
                                .child(Button::new("check-updates").label("Check for updates").disabled(blocked)
                                    .on_click({ let app = app.clone(); move |_, window, cx| { cx.update_entity(&app, |this, cx| this.check_updates(window, cx)); } }))
                                .when(available, |row| row.child(Button::new("install-update").primary()
                                    .label(if prepared { "Install & restart" } else { "Download, install & restart" })
                                    .disabled(blocked || busy)
                                    .on_click({ let app = app.clone(); move |_, window, cx| { cx.update_entity(&app, |this, cx| this.download_update(window, cx)); } }))))
                            .child(div().text_sm().text_color(rgb(crate::theme::MUTED_FOREGROUND)).child("Installed copies check at startup. Automatic updates download, verify and restart when your work is saved. Ledger data stays on this computer."))
                    }).keywords(["updates", "automatic updates", "check for updates", "install update", "restart"])),
            )
    }
}
