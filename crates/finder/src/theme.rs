//! Finder's look: macOS 26 (Tahoe) light appearance, in points, measured
//! from Tahoe screenshots (see docs/design-notes.md). One place for every
//! color, size and font, so the app can be tuned without touching views.

use gpui::{BoxShadow, Hsla, Rgba, hsla, point, px, rgb, rgba};

/// Preferred UI fonts, first installed wins: SF Pro (loaded from the local
/// fonts folder) for testing, Inter as the shippable stand-in, then
/// Windows' own.
pub const FONTS: &[&str] = &["SF Pro Text", "SF Pro", "Inter", "Segoe UI Variable Text", "Segoe UI"];

pub fn ui_font(installed: &[String]) -> &'static str {
    FONTS.iter().find(|f| installed.iter().any(|i| i == *f)).copied().unwrap_or("Segoe UI")
}

// Window
pub const WINDOW_W: f32 = 1180.;
pub const WINDOW_H: f32 = 720.;
pub const WINDOW_MIN_W: f32 = 760.;
pub const WINDOW_MIN_H: f32 = 420.;
pub const TOOLBAR_H: f32 = 52.;
/// Behind the floating sidebar (Mica shows through it).
pub fn window_bg() -> Rgba {
    rgba(0xF1F2F200)
}

// Traffic lights
pub const LIGHT_D: f32 = 14.;
pub const LIGHT_GAP: f32 = 8.;
/// From the sidebar panel's top-left corner.
pub const LIGHTS_X: f32 = 11.;
pub const LIGHTS_Y: f32 = 11.;
pub fn light_close() -> Rgba {
    rgb(0xFF5F57)
}
pub fn light_min() -> Rgba {
    rgb(0xFEBC2E)
}
pub fn light_max() -> Rgba {
    rgb(0x28C840)
}
pub fn light_inactive() -> Rgba {
    rgb(0xDDDDDD)
}
pub fn light_border() -> Rgba {
    rgba(0x00000014)
}
pub fn light_glyph() -> Rgba {
    rgba(0x00000099)
}

// Sidebar: a floating glass panel inset from the window edges.
pub const SIDEBAR_W: f32 = 206.;
pub const SIDEBAR_MARGIN: f32 = 8.;
pub const SIDEBAR_RADIUS: f32 = 18.;
pub const SIDEBAR_ROW_H: f32 = 30.;
pub const SIDEBAR_INSET: f32 = 8.;
/// SF Symbols' point size in the sidebar.
pub const SIDEBAR_SYMBOL: f32 = 15.;
pub fn sidebar_bg() -> Rgba {
    rgba(0xF9F9FAF2)
}
pub fn sidebar_border() -> Rgba {
    rgba(0x0000000D)
}
pub fn sidebar_selected() -> Rgba {
    rgba(0x0000000F)
}
pub fn sidebar_header() -> Rgba {
    rgba(0x00000066)
}
pub fn sidebar_icon() -> Rgba {
    rgba(0x000000B3)
}
pub fn accent() -> Rgba {
    rgb(0x0A7AFF)
}
pub fn sidebar_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow { color: hsla(0., 0., 0., 0.06), offset: point(px(0.), px(1.)), blur_radius: px(6.), spread_radius: px(0.), inset: false }]
}

// Content
pub const ROW_H: f32 = 22.;
pub const ROW_INSET: f32 = 10.;
pub const ROW_RADIUS: f32 = 6.;
pub const ROW_ICON: f32 = 16.;
pub const INDENT: f32 = 18.;
pub const HEADER_H: f32 = 28.;
pub const PATH_BAR_H: f32 = 26.;
pub const STATUS_BAR_H: f32 = 26.;
pub const INSPECTOR_W: f32 = 320.;
pub const DATE_W: f32 = 200.;
pub const SIZE_W: f32 = 80.;
pub const KIND_W: f32 = 140.;
pub fn content_bg() -> Rgba {
    rgb(0xFFFFFF)
}
pub fn zebra() -> Rgba {
    rgb(0xF4F5F5)
}
pub fn selection() -> Rgba {
    rgb(0x0056DF)
}
pub fn selection_inactive() -> Rgba {
    rgb(0xDCDCDC)
}
pub fn inspector_bg() -> Rgba {
    rgb(0xFFFFFF)
}
pub fn separator() -> Rgba {
    rgba(0x00000017)
}
pub fn bar_bg() -> Rgba {
    rgb(0xF9F9F9)
}

// Text
pub const TEXT_SIZE: f32 = 13.;
pub const SMALL_SIZE: f32 = 11.;
pub const HEADER_SIZE: f32 = 12.;
pub const TITLE_SIZE: f32 = 15.;
pub fn text() -> Rgba {
    rgba(0x000000DE)
}
pub fn text_secondary() -> Rgba {
    rgba(0x00000080)
}
pub fn text_tertiary() -> Rgba {
    rgba(0x00000047)
}

// Toolbar controls: Liquid Glass capsules floating over the content.
pub const CAPSULE_H: f32 = 36.;
pub const CAPSULE_GAP: f32 = 10.;
/// SF Symbols' point size in the toolbar.
pub const TOOL_SYMBOL: f32 = 15.;
pub fn glass() -> Rgba {
    rgba(0xFFFFFFEB)
}
pub fn glass_border() -> Rgba {
    rgba(0x0000000A)
}
pub fn glass_pressed() -> Rgba {
    rgba(0x00000012)
}
pub fn glass_shadow() -> Vec<BoxShadow> {
    vec![
        BoxShadow { color: hsla(0., 0., 0., 0.07), offset: point(px(0.), px(2.)), blur_radius: px(12.), spread_radius: px(0.), inset: false },
        BoxShadow { color: hsla(0., 0., 0., 0.04), offset: point(px(0.), px(0.)), blur_radius: px(1.), spread_radius: px(0.), inset: false },
    ]
}
pub fn glass_icon() -> Hsla {
    hsla(0., 0., 0.12, 1.)
}
