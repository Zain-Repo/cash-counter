use super::{Access, FormMode, Page, RegisterApp};
use gpui::{AppContext, Entity, TestAppContext, VisualTestContext, WindowOptions, px, size};
use gpui_component::Root;
use std::path::PathBuf;

fn workspace(cx: &mut TestAppContext, path: PathBuf) -> (Entity<RegisterApp>, VisualTestContext) {
    let mut service = cash_counter::company::CompanyService::open(&path).unwrap();
    service
        .setup(cash_counter::company::SetupCompany {
            profile: cash_counter::company::CompanyProfile {
                name: "Test Company".into(),
                ..Default::default()
            },
            username: "admin".into(),
            password: "test administrator password".into(),
        })
        .unwrap();
    cx.update(gpui_component::init);
    let mut app = None;
    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            let view = cx.new(|cx| RegisterApp::with_path(Ok(path), window, cx));
            app = Some(view.clone());
            cx.new(|cx| Root::new(view, window, cx))
        })
        .unwrap()
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| {
        let _ = window.draw(cx);
    });
    visual.run_until_parked();
    app.as_ref()
        .unwrap()
        .update_in(&mut visual, |app, window, cx| {
            assert!(app.access == Access::Login);
            assert!(app.snapshot.is_none());
            app.username
                .update(cx, |input, cx| input.set_value("admin", window, cx));
            app.password.update(cx, |input, cx| {
                input.set_value("test administrator password", window, cx)
            });
            app.submit_access(window, cx);
        });
    visual.run_until_parked();
    (app.unwrap(), visual)
}

#[gpui::test]
fn logout_removes_ledger_and_blocks_mutations(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (app, mut cx) = workspace(cx, directory.path().join("logout.sqlite3"));
    app.update_in(&mut cx, |app, window, cx| {
        assert!(app.access == Access::Authenticated);
        app.page = Page::Settings;
        app.logout(window, cx);
        assert!(app.access == Access::Login);
        assert!(app.snapshot.is_none());
        assert!(app.report_data.is_none());
        app.run(
            cash_counter::register::Operation::CreateRegister {
                name: "Blocked".into(),
            },
            window,
            cx,
        );
        assert!(!app.busy);
        assert!(app.password.read(cx).value().is_empty());
    });
}

#[gpui::test]
fn first_run_requires_company_administrator_and_sign_in(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("onboarding.sqlite3");
    cx.update(gpui_component::init);
    let mut app = None;
    let window = cx.update(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            let view = cx.new(|cx| RegisterApp::with_path(Ok(path.clone()), window, cx));
            app = Some(view.clone());
            cx.new(|cx| Root::new(view, window, cx))
        })
        .unwrap()
    });
    let app = app.unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    for height in [420., 600.] {
        visual.simulate_resize(size(px(860.), px(height)));
        visual.update(|window, cx| {
            let _ = window.draw(cx);
        });
        app.update_in(&mut visual, |app, _, _| {
            assert!(app.access == Access::Company);
            assert!(app.access_scroll.max_offset().y > px(0.));
            let maximum = app.access_scroll.max_offset();
            app.access_scroll
                .set_offset(gpui::point(px(0.), -maximum.y));
            assert_eq!(app.access_scroll.offset().y, -maximum.y);
            let content = app.access_scroll.bounds_for_item(0).unwrap();
            let viewport = app.access_scroll.bounds();
            assert!(content.bottom() + app.access_scroll.offset().y <= viewport.bottom() + px(1.));
        });
    }
    app.update_in(&mut visual, |app, window, cx| {
        assert!(app.access == Access::Company);
        app.submit_access(window, cx);
        assert!(app.error.is_some());
        app.company.update(cx, |input, cx| {
            input.set_value("Example Company", window, cx)
        });
        app.submit_access(window, cx);
        assert!(app.access == Access::Administrator);
        app.username
            .update(cx, |input, cx| input.set_value("admin", window, cx));
        app.password.update(cx, |input, cx| {
            input.set_value("administrator test password", window, cx)
        });
        app.confirmation.update(cx, |input, cx| {
            input.set_value("different password", window, cx)
        });
        app.submit_access(window, cx);
        assert!(app.error.is_some());
        assert!(!app.busy);
        app.confirmation.update(cx, |input, cx| {
            input.set_value("administrator test password", window, cx)
        });
        app.submit_access(window, cx);
    });
    visual.run_until_parked();
    app.update_in(&mut visual, |app, window, cx| {
        assert!(app.access == Access::Login);
        assert!(app.snapshot.is_none());
        assert!(app.password.read(cx).value().is_empty());
        app.password.update(cx, |input, cx| {
            input.set_value("administrator test password", window, cx)
        });
        app.submit_access(window, cx);
    });
    visual.run_until_parked();
    app.update_in(&mut visual, |app, window, cx| {
        assert!(app.access == Access::Authenticated);
        assert!(app.snapshot.is_some());
        app.page = Page::Settings;
        app.company.update(cx, |input, cx| {
            input.set_value("Renamed Company", window, cx)
        });
        app.rules.update(cx, |input, cx| {
            input.set_value("Count cash twice", window, cx)
        });
        app.save_settings(window, cx);
    });
    visual.run_until_parked();
    app.update_in(&mut visual, |app, _, _| {
        assert!(app.error.is_none());
        assert_eq!(app.profile.as_ref().unwrap().name, "Renamed Company");
    });
    assert_eq!(
        cash_counter::company::CompanyService::open(&path)
            .unwrap()
            .profile()
            .unwrap()
            .unwrap()
            .rules,
        "Count cash twice"
    );
    for width in [860., 1280.] {
        visual.simulate_resize(size(px(width), px(820.)));
        visual.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }
}

#[gpui::test]
fn forms_save_cash_and_require_review_before_closing(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (app, mut cx) = workspace(cx, directory.path().join("ui.sqlite3"));
    app.update_in(&mut cx, |app, window, cx| {
        assert!(!app.busy);
        assert!(app.mode == FormMode::Setup);
        app.name
            .update(cx, |input, cx| input.set_value("Front desk", window, cx));
        app.submit(window, cx);
    });
    cx.run_until_parked();
    app.update_in(&mut cx, |app, window, cx| {
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(app.mode == FormMode::Open);
        app.amount
            .update(cx, |input, cx| input.set_value("100.00", window, cx));
        app.submit(window, cx);
    });
    cx.run_until_parked();
    app.update_in(&mut cx, |app, window, cx| {
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(app.mode == FormMode::Entry);
        app.amount
            .update(cx, |input, cx| input.set_value("25.50", window, cx));
        app.reason
            .update(cx, |input, cx| input.set_value("Cash sale", window, cx));
        app.submit(window, cx);
        // A repeated submit while saving must not queue a second write.
        app.submit(window, cx);
    });
    cx.run_until_parked();
    app.update_in(&mut cx, |app, window, cx| {
        assert_eq!(
            app.snapshot
                .as_ref()
                .unwrap()
                .balance
                .as_ref()
                .unwrap()
                .expected,
            12550
        );
        assert_eq!(app.snapshot.as_ref().unwrap().entries.len(), 1);
        assert!(app.amount.read(cx).value().is_empty());
        app.change_mode(FormMode::Close, window, cx);
        app.count
            .update(cx, |input, cx| input.set_value("125.00", window, cx));
        app.submit(window, cx);
        assert_eq!(app.closing_review, Some(12500));
        assert!(!app.busy);
        assert!(
            !app.snapshot
                .as_ref()
                .unwrap()
                .session
                .as_ref()
                .unwrap()
                .closed
        );
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    app.update_in(&mut cx, |app, window, cx| app.submit(window, cx));
    cx.run_until_parked();
    app.update_in(&mut cx, |app, window, cx| {
        assert!(
            app.snapshot
                .as_ref()
                .unwrap()
                .session
                .as_ref()
                .unwrap()
                .closed
        );
        assert_eq!(
            app.snapshot
                .as_ref()
                .unwrap()
                .balance
                .as_ref()
                .unwrap()
                .difference,
            Some(-50)
        );
        app.page = Page::Reports;
        app.generate_report(window, cx);
    });
    cx.run_until_parked();
    app.update_in(&mut cx, |app, _, _| {
        assert!(app.error.is_none(), "{:?}", app.error);
        let report = app.report_data.as_ref().unwrap();
        assert_eq!(report.sessions.len(), 1);
        assert_eq!(report.sessions[0].expected, 12550);
        assert_eq!(report.movements.len(), 1);
        assert_eq!(report.sessions[0].difference, Some(-50));
    });
    for width in [860., 1280.] {
        cx.simulate_resize(size(px(width), px(820.)));
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }
}

#[gpui::test]
fn invalid_form_preserves_input_without_writing(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (app, mut cx) = workspace(cx, directory.path().join("invalid.sqlite3"));
    app.update_in(&mut cx, |app, window, cx| {
        app.submit(window, cx);
        assert!(app.error.is_some());
        assert!(!app.busy);
        assert!(app.snapshot.as_ref().unwrap().registers.is_empty());
        app.name
            .update(cx, |input, cx| input.set_value("Desk", window, cx));
        app.submit(window, cx);
    });
    cx.run_until_parked();
    app.update_in(&mut cx, |app, window, cx| {
        app.amount
            .update(cx, |input, cx| input.set_value("1.234", window, cx));
        app.submit(window, cx);
        assert!(app.error.is_some());
        assert!(!app.busy);
        assert_eq!(app.amount.read(cx).value(), "1.234");
        assert!(app.snapshot.as_ref().unwrap().sessions.is_empty());
    });
}

#[gpui::test]
fn updates_wait_for_unsaved_drafts_and_failed_writes(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (app, mut cx) = workspace(cx, directory.path().join("updates.sqlite3"));
    app.update_in(&mut cx, |app, window, cx| {
        // Test workspaces do not check a release service or launch installers.
        assert!(!app.updates.enabled);
        assert!(app.update_task.is_none());
        assert!(app.can_restart_for_update(cx));
        app.updates.working = true;
        assert!(!app.can_restart_for_update(cx));
        app.updates.working = false;
        app.name.update(cx, |input, cx| {
            input.set_value("Unsaved register", window, cx)
        });
        assert!(!app.can_restart_for_update(cx));
        app.name
            .update(cx, |input, cx| input.set_value("", window, cx));
        app.mode = FormMode::Entry;
        app.reason.update(cx, |input, cx| {
            input.set_value("Unsaved cash reason", window, cx)
        });
        assert!(!app.can_restart_for_update(cx));
        app.reason
            .update(cx, |input, cx| input.set_value("", window, cx));
        app.mode = FormMode::Open;
        app.date
            .update(cx, |input, cx| input.set_value("2020-01-01", window, cx));
        assert!(!app.can_restart_for_update(cx));
        app.mode = FormMode::Close;
        app.closing_review = Some(0);
        assert!(!app.can_restart_for_update(cx));
        app.closing_review = None;
        app.mode = FormMode::Entry;
        app.page = Page::Settings;
        app.rules.update(cx, |input, cx| {
            input.set_value("Unsaved policy", window, cx)
        });
        assert!(!app.can_restart_for_update(cx));
        app.populate_settings(window, cx);
        assert!(app.can_restart_for_update(cx));
        app.busy = true;
        assert!(!app.can_restart_for_update(cx));
        app.busy = false;
        app.failed_operation = Some(cash_counter::register::Operation::Refresh);
        assert!(!app.can_restart_for_update(cx));
        app.failed_operation = None;
        app.access = Access::Company;
        assert!(!app.can_restart_for_update(cx));
        app.access = Access::Administrator;
        assert!(!app.can_restart_for_update(cx));
        app.access = Access::Login;
        assert!(!app.can_restart_for_update(cx));
        app.username
            .update(cx, |input, cx| input.set_value("", window, cx));
        assert!(app.can_restart_for_update(cx));
        app.password
            .update(cx, |input, cx| input.set_value("login draft", window, cx));
        assert!(!app.can_restart_for_update(cx));
        app.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        app.access = Access::Authenticated;
    });
    for width in [860., 1280.] {
        cx.simulate_resize(size(px(width), px(820.)));
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }
}
