//! Finder for Windows.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod browser;
mod files;
mod icons;
mod theme;

use browser::{Browser, Collapse, Expand, GoBack, GoForward, GoUp, Open, SelectNext, SelectPrev};
use gpui::{
    App, AssetSource, Bounds, Focusable, KeyBinding, SharedString, TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowOptions, prelude::*, px,
    size,
};
use std::borrow::Cow;

#[derive(rust_embed::RustEmbed)]
#[folder = "assets/icons"]
#[prefix = "icons/"]
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(<Self as rust_embed::Embed>::get(path).map(|f| f.data))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(<Self as rust_embed::Embed>::iter().filter(|p| p.starts_with(path)).map(|p| p.to_string().into()).collect())
    }
}

fn main() {
    gpui_platform::application().with_assets(Assets).run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("down", SelectNext, Some("Browser")),
            KeyBinding::new("up", SelectPrev, Some("Browser")),
            KeyBinding::new("right", Expand, Some("Browser")),
            KeyBinding::new("left", Collapse, Some("Browser")),
            // Finder's keys, with Ctrl for Command, and Explorer's.
            KeyBinding::new("ctrl-down", Open, Some("Browser")),
            KeyBinding::new("enter", Open, Some("Browser")),
            KeyBinding::new("ctrl-up", GoUp, Some("Browser")),
            KeyBinding::new("alt-up", GoUp, Some("Browser")),
            KeyBinding::new("ctrl-[", GoBack, Some("Browser")),
            KeyBinding::new("ctrl-]", GoForward, Some("Browser")),
            KeyBinding::new("alt-left", GoBack, Some("Browser")),
            KeyBinding::new("alt-right", GoForward, Some("Browser")),
            KeyBinding::new("backspace", GoBack, Some("Browser")),
        ]);
        let font: SharedString = theme::ui_font(&cx.text_system().all_font_names()).into();
        let bounds = Bounds::centered(None, size(px(theme::WINDOW_W), px(theme::WINDOW_H)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions { title: Some("Finder".into()), appears_transparent: true, traffic_light_position: None }),
                    window_background: WindowBackgroundAppearance::MicaBackdrop,
                    window_min_size: Some(size(px(theme::WINDOW_MIN_W), px(theme::WINDOW_MIN_H))),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| Browser::new(font, cx));
                    window.focus(&view.focus_handle(cx), cx);
                    view
                },
            )
            .expect("open window");
        let _ = window;
        cx.activate(true);
    });
}
