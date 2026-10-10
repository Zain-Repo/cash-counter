use gpui::{App, px, rgb};
use gpui_component::{Theme, ThemeMode};

pub(crate) const BACKGROUND: u32 = 0xffffff;
pub(crate) const FOREGROUND: u32 = 0x171717;
pub(crate) const MUTED_SURFACE: u32 = 0xf5f5f5;
pub(crate) const MUTED_FOREGROUND: u32 = 0x737373;
pub(crate) const BORDER: u32 = 0xe5e5e5;
pub(crate) const INPUT: u32 = 0xd4d4d4;
pub(crate) const PRIMARY: u32 = 0x262626;
pub(crate) const PRIMARY_FOREGROUND: u32 = 0xfafafa;
pub(crate) const RING: u32 = 0x737373;
pub(crate) const SUCCESS: u32 = 0x265746;

pub(crate) const CANVAS: u32 = 0xfafafa;

pub(crate) fn setup(cx: &mut App) {
    Theme::change(ThemeMode::Light, None, cx);

    let theme = Theme::global_mut(cx);
    let colors = &mut theme.colors;
    colors.background = rgb(BACKGROUND).into();
    colors.foreground = rgb(FOREGROUND).into();
    colors.border = rgb(BORDER).into();
    colors.input = rgb(INPUT).into();
    colors.muted = rgb(MUTED_SURFACE).into();
    colors.muted_foreground = rgb(MUTED_FOREGROUND).into();
    colors.primary = rgb(PRIMARY).into();
    colors.primary_foreground = rgb(PRIMARY_FOREGROUND).into();
    colors.primary_hover = rgb(0x404040).into();
    colors.primary_active = rgb(0x1f1f1f).into();
    colors.secondary = rgb(MUTED_SURFACE).into();
    colors.secondary_foreground = rgb(FOREGROUND).into();
    colors.secondary_hover = rgb(0xebebeb).into();
    colors.secondary_active = rgb(0xe5e5e5).into();
    colors.accent = rgb(MUTED_SURFACE).into();
    colors.accent_foreground = rgb(FOREGROUND).into();
    colors.ring = rgb(RING).into();
    colors.button = rgb(MUTED_SURFACE).into();
    colors.button_foreground = rgb(FOREGROUND).into();
    colors.button_hover = rgb(0xebebeb).into();
    colors.button_active = rgb(0xe5e5e5).into();
    colors.button_primary = rgb(PRIMARY).into();
    colors.button_primary_foreground = rgb(PRIMARY_FOREGROUND).into();
    colors.button_primary_hover = rgb(0x404040).into();
    colors.button_primary_active = rgb(0x1f1f1f).into();
    colors.button_secondary = colors.secondary;
    colors.button_secondary_foreground = colors.secondary_foreground;
    colors.button_secondary_hover = colors.secondary_hover;
    colors.button_secondary_active = colors.secondary_active;
    colors.popover = rgb(BACKGROUND).into();
    colors.popover_foreground = rgb(FOREGROUND).into();
    colors.list = rgb(BACKGROUND).into();
    colors.list_hover = rgb(MUTED_SURFACE).into();
    colors.list_active = rgb(0xebebeb).into();
    colors.list_active_border = rgb(BORDER).into();
    colors.table = rgb(BACKGROUND).into();
    colors.table_head = rgb(MUTED_SURFACE).into();
    colors.table_even = rgb(0xfafafa).into();
    colors.table_hover = rgb(MUTED_SURFACE).into();
    colors.table_active = rgb(0xebebeb).into();
    colors.table_row_border = rgb(BORDER).into();
    colors.table_active_border = rgb(INPUT).into();
    colors.table_head_foreground = rgb(MUTED_FOREGROUND).into();
    colors.tab = rgb(BACKGROUND).into();
    colors.tab_foreground = rgb(MUTED_FOREGROUND).into();
    colors.tab_active = rgb(BACKGROUND).into();
    colors.tab_active_foreground = rgb(FOREGROUND).into();
    colors.tab_bar = rgb(BACKGROUND).into();
    colors.tab_bar_segmented = rgb(MUTED_SURFACE).into();
    colors.switch = rgb(INPUT).into();
    colors.switch_thumb = rgb(BACKGROUND).into();
    colors.sidebar = rgb(CANVAS).into();
    colors.sidebar_foreground = rgb(FOREGROUND).into();
    colors.sidebar_border = rgb(BORDER).into();
    colors.sidebar_accent = rgb(MUTED_SURFACE).into();
    colors.sidebar_accent_foreground = rgb(FOREGROUND).into();

    // Success feedback keeps its semantic green while primary actions stay neutral.
    colors.success = rgb(SUCCESS).into();
    colors.success_foreground = rgb(0xffffff).into();
    colors.success_hover = rgb(0x1e4637).into();
    colors.success_active = rgb(0x183a2e).into();

    theme.radius = px(8.);
    theme.radius_lg = px(8.);
    theme.font_size = px(14.);
    theme.font_family = "Segoe UI".into();
    theme.tokens = gpui_component::theme::ThemeTokens::from(&theme.colors);
    Theme::sync_base(cx);
}
