use super::{Access, Page, tests::workspace};
use cash_counter::backup_service::{BackupMode, BackupService};
use gpui::{TestAppContext, px, size};

#[gpui::test]
fn manual_backup_uses_saved_folder_and_blocks_duplicate_submissions(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("backups");
    std::fs::create_dir(&destination).unwrap();
    let path = directory.path().join("manual.sqlite3");
    let (app, mut visual) = workspace(cx, path.clone());
    app.update_in(&mut visual, |app, window, cx| {
        assert!(app.backups.status.is_some());
        assert!(app.backup_timer.is_some());
        app.backup_directory.update(cx, |input, cx| {
            input.set_value(destination.to_string_lossy().into_owned(), window, cx)
        });
        assert!(!app.can_restart_for_update(cx));
        app.run_configured_backup(false, window, cx);
        assert!(!app.backup_working);
        assert!(app.backups.error.is_some());
        app.save_backup_settings(window, cx);
    });
    visual.run_until_parked();
    app.update_in(&mut visual, |app, window, cx| {
        assert!(app.backups.error.is_none(), "{:?}", app.backups.error);
        assert!(!app.backup_settings_dirty(cx));
        assert!(app.can_restart_for_update(cx));
        app.run_configured_backup(false, window, cx);
        assert!(app.backup_working);
        assert!(!app.can_restart_for_update(cx));
        app.run_configured_backup(false, window, cx);
        app.logout(window, cx);
        assert!(app.access == Access::Authenticated);
    });
    visual.run_until_parked();
    app.update_in(&mut visual, |app, _, _| {
        assert!(!app.backup_working);
        assert!(app.backups.error.is_none(), "{:?}", app.backups.error);
        assert!(
            app.backups
                .status
                .as_ref()
                .unwrap()
                .last_success_at
                .is_some()
        );
    });
    assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 1);
    let status = BackupService::open(&path).unwrap().status().unwrap();
    assert_eq!(status.settings.mode, BackupMode::Manual);
    assert!(
        status
            .last_backup_path
            .unwrap()
            .starts_with(destination.canonicalize().unwrap())
    );
}

#[gpui::test]
fn automatic_settings_run_immediately_and_stop_on_logout(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (app, mut visual) = workspace(cx, directory.path().join("automatic.sqlite3"));
    app.update_in(&mut visual, |app, window, cx| {
        app.backup_directory.update(cx, |input, cx| {
            input.set_value(directory.path().to_string_lossy().into_owned(), window, cx)
        });
        app.toggle_backup_mode(window, cx);
        app.backup_frequency.update(cx, |select, cx| {
            select.set_selected_value(&"Hourly", window, cx)
        });
        app.save_backup_settings(window, cx);
    });
    visual.run_until_parked();
    app.update_in(&mut visual, |app, window, cx| {
        let status = app.backups.status.as_ref().unwrap();
        assert!(app.backups.error.is_none(), "{:?}", app.backups.error);
        assert!(status.last_success_at.is_some());
        assert!(status.next_due_at.unwrap() > status.last_success_at.unwrap());
        app.logout(window, cx);
        assert!(app.access == Access::Login);
        assert!(app.backup_timer.is_none());
        assert!(app.backups.status.is_none());
        assert!(app.backup_directory.read(cx).value().is_empty());
        app.run_configured_backup(true, window, cx);
        assert!(!app.backup_working);
        app.password.update(cx, |input, cx| {
            input.set_value("test administrator password", window, cx)
        });
        app.submit_access(window, cx);
    });
    visual.run_until_parked();
    app.update_in(&mut visual, |app, _, cx| {
        assert!(app.access == Access::Authenticated);
        assert!(app.backup_automatic);
        assert_eq!(
            app.backup_frequency.read(cx).selected_value(),
            Some(&"Hourly")
        );
        assert!(!app.backup_settings_dirty(cx));
    });
}

#[gpui::test]
fn invalid_backup_settings_preserve_draft_and_saved_configuration(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (app, mut visual) = workspace(cx, directory.path().join("invalid-backup.sqlite3"));
    app.update_in(&mut visual, |app, window, cx| {
        app.backup_directory.update(cx, |input, cx| {
            input.set_value("relative-folder", window, cx)
        });
        app.save_backup_settings(window, cx);
    });
    visual.run_until_parked();
    app.update_in(&mut visual, |app, window, cx| {
        assert!(app.backups.error.is_some());
        assert_eq!(app.backup_directory.read(cx).value(), "relative-folder");
        assert!(
            app.backups
                .status
                .as_ref()
                .unwrap()
                .settings
                .directory
                .is_none()
        );
        assert!(!app.can_restart_for_update(cx));
        app.discard_backup_settings(window, cx);
        assert!(!app.backup_settings_dirty(cx));
        app.toggle_backup_mode(window, cx);
        app.save_backup_settings(window, cx);
        assert!(!app.backups.loading);
        assert!(app.backups.error.as_ref().unwrap().contains("folder"));
    });
}

#[gpui::test]
fn folder_picker_selects_directories_and_cancellation_preserves_draft(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (app, mut visual) = workspace(cx, directory.path().join("picker.sqlite3"));
    app.update_in(&mut visual, |app, window, cx| {
        app.choose_backup_directory(window, cx)
    });
    assert!(visual.did_prompt_for_paths());
    visual.simulate_path_prompt_response(|options| {
        assert!(options.directories);
        assert!(!options.files);
        assert!(!options.multiple);
        Some(vec![directory.path().to_path_buf()])
    });
    visual.run_until_parked();
    app.update_in(&mut visual, |app, window, cx| {
        assert_eq!(
            app.backup_directory.read(cx).value().as_ref(),
            directory.path().to_string_lossy()
        );
        app.choose_backup_directory(window, cx);
    });
    visual.simulate_path_prompt_response(|_| None);
    visual.run_until_parked();
    app.update_in(&mut visual, |app, _, cx| {
        assert!(!app.backups.loading);
        assert!(app.backups.error.is_none());
        assert_eq!(
            app.backup_directory.read(cx).value().as_ref(),
            directory.path().to_string_lossy()
        );
    });
}

#[gpui::test]
fn redesigned_settings_pages_render_at_supported_window_sizes(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (app, mut visual) = workspace(cx, directory.path().join("settings-layout.sqlite3"));
    app.update_in(&mut visual, |app, window, cx| {
        app.navigate(Page::Settings, window, cx)
    });
    for (width, height) in [(860., 640.), (1280., 820.)] {
        visual.simulate_resize(size(px(width), px(height)));
        for page in 0..5 {
            app.update_in(&mut visual, |app, _, cx| {
                app.settings_initial_page = page;
                app.settings_generation += 1;
                cx.notify();
            });
            visual.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }
    }
}

#[gpui::test]
fn automatic_backup_timer_resumes_after_update_installation_pause(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("backups");
    std::fs::create_dir(&destination).unwrap();
    let path = directory.path().join("update-pause.sqlite3");
    let (app, mut visual) = workspace(cx, path.clone());
    app.update_in(&mut visual, |app, window, cx| {
        app.backup_directory.update(cx, |input, cx| {
            input.set_value(destination.to_string_lossy().into_owned(), window, cx)
        });
        app.toggle_backup_mode(window, cx);
        app.save_backup_settings(window, cx);
    });
    visual.run_until_parked();
    assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 1);
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE backup_service_state SET next_due_at = 0 WHERE id = 1",
            [],
        )
        .unwrap();
    app.update_in(&mut visual, |app, _, _| {
        app.backups.status.as_mut().unwrap().next_due_at = Some(0);
        app.updates.installing = true;
    });
    visual
        .executor()
        .advance_clock(std::time::Duration::from_secs(30));
    visual.run_until_parked();
    assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 1);
    app.update_in(&mut visual, |app, _, _| app.updates.installing = false);
    visual
        .executor()
        .advance_clock(std::time::Duration::from_secs(30));
    visual.run_until_parked();
    assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 2);
    app.update_in(&mut visual, |app, _, _| {
        assert!(app.backup_timer.is_some());
        assert!(!app.backup_working);
        assert!(app.backups.error.is_none(), "{:?}", app.backups.error);
    });
}
