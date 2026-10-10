use super::{Access, tests::workspace};
use gpui::TestAppContext;

#[gpui::test]
fn restore_does_not_discard_unsaved_cash_entry(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("draft.sqlite");
    let (app, mut visual) = workspace(cx, path.clone());
    let backup = directory.path().join("backup.sqlite");
    cash_counter::backup::backup_database(&path, &backup).unwrap();
    app.update_in(&mut visual, |app, window, cx| {
        app.mode = super::FormMode::Entry;
        app.amount
            .update(cx, |input, cx| input.set_value("25.00", window, cx));
        app.restore_source.update(cx, |input, cx| {
            input.set_value(backup.to_string_lossy().into_owned(), window, cx)
        });
        app.request_restore(window, cx);
        assert!(!app.restore_working);
        assert_eq!(app.amount.read(cx).value(), "25.00");
        assert!(
            app.backups
                .error
                .as_ref()
                .unwrap()
                .contains("Save or discard")
        );
    });
    assert!(!visual.has_pending_prompt());
}

#[gpui::test]
fn restore_picker_supports_files_folders_and_cancel(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (app, mut visual) = workspace(cx, directory.path().join("picker.sqlite"));
    for folder in [false, true] {
        app.update_in(&mut visual, |app, window, cx| {
            app.choose_restore_source(folder, window, cx)
        });
        visual.simulate_path_prompt_response(|options| {
            assert_eq!(options.directories, folder);
            assert_eq!(options.files, !folder);
            assert!(!options.multiple);
            Some(vec![directory.path().to_path_buf()])
        });
        visual.run_until_parked();
    }
    app.update_in(&mut visual, |app, window, cx| {
        app.choose_restore_source(false, window, cx)
    });
    visual.simulate_path_prompt_response(|_| None);
    visual.run_until_parked();
    app.update_in(&mut visual, |app, _, cx| {
        assert_eq!(
            app.restore_source.read(cx).value().as_ref(),
            directory.path().to_string_lossy()
        );
        assert!(!app.backups.loading);
    });
}

#[gpui::test]
fn invalid_restore_keeps_session_and_data(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (app, mut visual) = workspace(cx, directory.path().join("invalid.sqlite"));
    app.update_in(&mut visual, |app, window, cx| {
        app.restore_source.update(cx, |input, cx| {
            input.set_value(
                directory
                    .path()
                    .join("missing.sqlite")
                    .to_string_lossy()
                    .into_owned(),
                window,
                cx,
            )
        });
        app.request_restore(window, cx);
    });
    visual.run_until_parked();
    app.update_in(&mut visual, |app, _, _| {
        assert!(app.access == Access::Authenticated);
        assert!(app.backups.error.is_some());
        assert!(!app.restore_working);
        assert!(!app.busy);
        assert!(app.snapshot.is_some());
    });
    assert!(!visual.has_pending_prompt());
}

#[gpui::test]
fn failed_restore_reopens_current_ledger_for_later_operations(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (app, mut visual) = workspace(cx, directory.path().join("failure.sqlite"));
    let corrupt = directory.path().join("corrupt.sqlite");
    std::fs::write(&corrupt, b"not a SQLite backup").unwrap();
    app.update_in(&mut visual, |app, window, cx| {
        app.restore_source.update(cx, |input, cx| {
            input.set_value(corrupt.to_string_lossy().into_owned(), window, cx)
        });
        app.request_restore(window, cx);
    });
    visual.run_until_parked();
    visual.simulate_prompt_answer("Restore data");
    visual.run_until_parked();
    app.update_in(&mut visual, |app, window, cx| {
        assert!(app.access == Access::Authenticated);
        assert!(app.backups.error.is_some());
        app.run(cash_counter::register::Operation::Refresh, window, cx);
    });
    visual.run_until_parked();
    app.update_in(&mut visual, |app, _, _| {
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(app.snapshot.is_some());
    });
}

#[gpui::test]
fn restore_confirmation_cancel_preserves_current_data(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite");
    let (app, mut visual) = workspace(cx, path.clone());
    let backup = directory.path().join("snapshot.sqlite");
    cash_counter::backup::backup_database(&path, &backup).unwrap();
    app.update_in(&mut visual, |app, window, cx| {
        app.restore_source.update(cx, |input, cx| {
            input.set_value(backup.to_string_lossy().into_owned(), window, cx)
        });
        app.request_restore(window, cx);
        assert!(app.restore_working);
        assert!(!app.can_restart_for_update(cx));
        app.logout(window, cx);
        assert!(app.access == Access::Authenticated);
    });
    visual.run_until_parked();
    assert!(visual.has_pending_prompt());
    visual.simulate_prompt_answer("Cancel");
    visual.run_until_parked();
    app.update_in(&mut visual, |app, _, _| {
        assert!(!app.restore_working);
        assert!(!app.busy);
        assert!(app.access == Access::Authenticated);
    });
    assert_eq!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|file| file.file_name().to_string_lossy().contains("pre-restore"))
            .count(),
        0
    );
}

#[gpui::test]
fn restore_reloads_backup_and_requires_sign_in(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite");
    let (app, mut visual) = workspace(cx, path.clone());
    let backup = directory.path().join("snapshot.sqlite");
    cash_counter::backup::backup_database(&path, &backup).unwrap();
    let service = cash_counter::company::CompanyService::open(&path).unwrap();
    let mut changed = service.profile().unwrap().unwrap();
    changed.name = "Changed after backup".into();
    service.save_settings(&changed).unwrap();
    drop(service);
    app.update_in(&mut visual, |app, window, cx| {
        app.restore_source.update(cx, |input, cx| {
            input.set_value(backup.to_string_lossy().into_owned(), window, cx)
        });
        app.request_restore(window, cx);
    });
    visual.run_until_parked();
    assert!(visual.has_pending_prompt());
    visual.simulate_prompt_answer("Restore data");
    visual.run_until_parked();
    app.update_in(&mut visual, |app, _, _| {
        assert!(app.access == Access::Login, "{:?}", app.error);
        assert_eq!(app.profile.as_ref().unwrap().name, "Test Company");
        assert!(app.snapshot.is_none());
        assert!(app.backup_timer.is_none());
        assert!(app.notice.as_ref().unwrap().contains("previous data"));
    });
}
