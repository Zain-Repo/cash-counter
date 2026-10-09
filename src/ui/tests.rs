use super::{FormMode, RegisterApp};
use gpui::{AppContext, Entity, TestAppContext, VisualTestContext, WindowOptions, px, size};
use gpui_component::Root;
use std::path::PathBuf;

fn workspace(cx: &mut TestAppContext, path: PathBuf) -> (Entity<RegisterApp>, VisualTestContext) {
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
    (app.unwrap(), visual)
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
    app.update_in(&mut cx, |app, _, _| {
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
