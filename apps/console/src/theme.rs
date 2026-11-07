//! A dark HUD theme: layered near-black panels with neon status colours, after the palettes of
//! play.staratlas.com (panels, gold) and z.ink (green, teal, red), with GitHub dark's text colours.
//! Colours only, no branding.

use std::borrow::Cow;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, px, rgb};

/// Headings and labels: Tomorrow (SIL OFL 1.1, `assets/fonts/OFL.txt`), as z.ink uses.
pub const LABEL_FONT: &str = "Tomorrow";

/// Registers the bundled Tomorrow weights. On failure labels fall back to the system font.
pub fn load_fonts(cx: &App) {
    let fonts = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/Tomorrow-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Tomorrow-Medium.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Tomorrow-SemiBold.ttf").as_slice()),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("warning: loading the Tomorrow font: {e:#}");
    }
}

/// Window and deepest surfaces.
pub const BG: u32 = 0x0014_131a;
/// Panels: keys, detail, table header.
pub const PANEL: u32 = 0x001b_1a22;
/// Raised: cards, inputs, alternate rows.
pub const RAISED: u32 = 0x0022_2129;
pub const BORDER: u32 = 0x002d_2b32;
pub const BORDER_STRONG: u32 = 0x0040_4046;
/// GitHub dark's off-white and muted grey: softer than pure white on near-black.
pub const TEXT: u32 = 0x00e6_edf3;
pub const MUTED: u32 = 0x0091_98a1;
pub const GREEN: u32 = 0x005b_f19b;
pub const TEAL: u32 = 0x003f_d8ea;
pub const GOLD: u32 = 0x00ff_be4d;
pub const RED: u32 = 0x00f9_4e56;
/// Dark text on a neon fill.
const ON_ACCENT: u32 = 0x0006_1a1d;

pub fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// Switches to dark mode, then overrides its colours (a second update, as `Theme::update` asks).
pub fn apply(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |t| {
        t.radius = px(2.0);
        t.radius_lg = px(4.0);
        t.shadow = false;
        let k = &mut t.colors;
        k.background = c(BG);
        k.foreground = c(TEXT);
        k.border = c(BORDER);
        k.window_border = c(BORDER);
        k.muted = c(RAISED);
        k.muted_foreground = c(MUTED);
        k.accent = c(TEAL).opacity(0.12);
        k.accent_foreground = c(TEXT);
        k.primary = c(GREEN);
        k.primary_hover = c(GREEN).opacity(0.85);
        k.primary_active = c(GREEN).opacity(0.7);
        k.primary_foreground = c(ON_ACCENT);
        k.secondary = c(RAISED);
        k.secondary_hover = c(BORDER);
        k.secondary_active = c(BORDER_STRONG);
        k.secondary_foreground = c(TEXT);
        k.success = c(GREEN);
        k.success_foreground = c(ON_ACCENT);
        k.warning = c(GOLD);
        k.warning_foreground = c(ON_ACCENT);
        k.danger = c(RED);
        k.danger_foreground = c(TEXT);
        k.info = c(TEAL);
        k.info_foreground = c(ON_ACCENT);
        k.link = c(TEAL);
        k.link_hover = c(GREEN);
        k.link_active = c(GREEN);
        k.input = c(BORDER_STRONG);
        k.ring = c(TEAL);
        k.caret = c(GREEN);
        k.selection = c(TEAL).opacity(0.25);
        k.popover = c(PANEL);
        k.popover_foreground = c(TEXT);
        k.title_bar = c(BG);
        k.title_bar_border = c(BORDER);
        k.progress_bar = c(GREEN);
        k.scrollbar = c(BG).opacity(0.0);
        k.scrollbar_thumb = c(BORDER_STRONG);
        k.scrollbar_thumb_hover = c(MUTED);
        k.table = c(BG);
        k.table_even = c(PANEL).opacity(0.6);
        k.table_head = c(PANEL);
        k.table_head_foreground = c(MUTED);
        k.table_hover = c(TEAL).opacity(0.06);
        k.table_active = c(TEAL).opacity(0.14);
        k.table_active_border = c(TEAL);
        k.table_row_border = c(BORDER).opacity(0.5);
        k.list = c(BG);
        k.list_hover = c(TEAL).opacity(0.06);
        k.list_active = c(TEAL).opacity(0.14);
        k.list_active_border = c(TEAL);
        k.button = c(RAISED);
        k.button_hover = c(BORDER);
        k.button_active = c(BORDER_STRONG);
        k.button_foreground = c(TEXT);
        k.button_primary = c(GREEN);
        k.button_primary_hover = c(GREEN).opacity(0.85);
        k.button_primary_active = c(GREEN).opacity(0.7);
        k.button_primary_foreground = c(ON_ACCENT);
        k.green = c(GREEN);
        k.cyan = c(TEAL);
        k.red = c(RED);
        k.yellow = c(GOLD);
    });
}
