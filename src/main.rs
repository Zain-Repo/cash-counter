#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod ui;

use gpui::*;
use gpui_component::{Root, Theme, ThemeMode};
use gpui_component_assets::Assets;

fn main() {
    gpui_platform::application().with_assets(Assets).run(|cx| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Light, None, cx);
        let theme = Theme::global_mut(cx);
        theme.colors.primary = rgb(0x265746).into();
        theme.colors.primary_foreground = rgb(0xffffff).into();
        theme.colors.ring = rgb(0x265746).into();
        theme.radius = px(6.);
        theme.font_size = px(14.);
        theme.font_family = "Candara".into();
        theme.tokens = gpui_component::theme::ThemeTokens::from(&theme.colors);
        Theme::sync_base(cx);
        cx.bind_keys([
            KeyBinding::new("ctrl-enter", ui::SubmitForm, Some("CashRegister")),
            KeyBinding::new("escape", ui::CancelForm, Some("CashRegister")),
        ]);
        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(860.), px(640.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Till — Cash Register".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| ui::RegisterApp::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            },
        );
        if result.is_err() {
            eprintln!("Unable to open the cash-register window.");
            cx.quit();
        }
        cx.activate(true);
    });
}
