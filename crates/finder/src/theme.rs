//! Finder's look: macOS 26 (Tahoe) light appearance, in points. One place
//! for every color, size and font, so the app can be tuned against real
//! Finder screenshots (and fonts/icons swapped) without touching views.

use gpui::{Hsla, Rgba, rgb, rgba};

/// Preferred UI fonts, first installed wins: SF Pro for local testing,
/// Inter as the shippable stand-in, then Windows' own.
pub const FONTS: &[&str] = &["SF Pro", "SF Pro Text", "Inter", "Segoe UI Variable Text", "Segoe UI"];

pub fn ui_font(installed: &[String]) -> &'static str {
    FONTS.iter().find(|f| installed.iter().any(|i| i == *f)).copied().unwrap_or("Segoe UI")
}

// Window
pub const WINDOW_W: f32 = 1100.;
pub const WINDOW_H: f32 = 700.;
pub const WINDOW_MIN_W: f32 = 760.;
pub const WINDOW_MIN_H: f32 = 420.;
pub const TOOLBAR_H: f32 = 52.;

// Traffic lights
pub const LIGHT_D: f32 = 12.;
pub const LIGHT_GAP: f32 = 8.;
pub const LIGHTS_X: f32 = 20.;
pub const LIGHTS_Y: f32 = (TOOLBAR_H - LIGHT_D) / 2.;
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
    rgb(0xDCDCDC)
}
pub fn light_border() -> Rgba {
    rgba(0x0000001F)
}
pub fn light_glyph() -> Rgba {
    rgba(0x0000008C)
}

// Sidebar
pub const SIDEBAR_W: f32 = 200.;
pub const SIDEBAR_ROW_H: f32 = 28.;
pub const SIDEBAR_INSET: f32 = 10.;
pub const SIDEBAR_ICON: f32 = 17.;
pub fn sidebar_bg() -> Rgba {
    rgba(0xEDEDEFD0)
}
pub fn sidebar_selected() -> Rgba {
    rgba(0x00000017)
}
pub fn sidebar_header() -> Rgba {
    rgba(0x0000005C)
}
pub fn sidebar_icon() -> Rgba {
    rgb(0x0A84FF)
}

// Content
pub const ROW_H: f32 = 24.;
pub const ROW_INSET: f32 = 10.;
pub const ROW_RADIUS: f32 = 6.;
pub const ROW_ICON: f32 = 16.;
pub const INDENT: f32 = 16.;
pub const HEADER_H: f32 = 28.;
pub const PATH_BAR_H: f32 = 24.;
pub const STATUS_BAR_H: f32 = 24.;
pub const INSPECTOR_W: f32 = 250.;
pub fn content_bg() -> Rgba {
    rgb(0xFFFFFF)
}
pub fn zebra() -> Rgba {
    rgb(0xF4F4F5)
}
pub fn selection() -> Rgba {
    rgb(0x0A5CD7)
}
pub fn selection_inactive() -> Rgba {
    rgb(0xDCDCDC)
}
pub fn inspector_bg() -> Rgba {
    rgba(0xF2F2F4F0)
}
pub fn separator() -> Rgba {
    rgba(0x0000001A)
}

// Text
pub const TEXT_SIZE: f32 = 13.;
pub const SMALL_SIZE: f32 = 11.;
pub const HEADER_SIZE: f32 = 11.;
pub const TITLE_SIZE: f32 = 15.;
pub fn text() -> Rgba {
    rgba(0x000000D9)
}
pub fn text_secondary() -> Rgba {
    rgba(0x00000080)
}
pub fn text_tertiary() -> Rgba {
    rgba(0x0000004D)
}

// Toolbar controls: Liquid Glass capsules.
pub const CAPSULE_H: f32 = 34.;
pub const CAPSULE_GAP: f32 = 8.;
pub const TOOL_ICON: f32 = 17.;
pub fn glass() -> Rgba {
    rgba(0xFFFFFFE6)
}
pub fn glass_border() -> Rgba {
    rgba(0x0000000F)
}
pub fn glass_pressed() -> Rgba {
    rgba(0x0000000F)
}
pub fn glass_shadow() -> Hsla {
    gpui::hsla(0., 0., 0., 0.10)
}
