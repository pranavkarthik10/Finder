//! Finder for Windows.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod browser;
mod clipboard;
mod files;
mod icons;
mod ops;
mod preview_host;
mod text_input;
mod theme;

use browser::{Browser, Collapse, ContextMenu, CopyFiles, CopyPath, CutFiles, Duplicate, EndSearch, Expand, FocusSearch, GoBack, GoForward, GoUp, NewFolder, Open, PasteFiles, Rename, SelectAll, QuickLook, SelectNext, SelectPrev, Trash, UndoOp};
use gpui::{
    App, Bounds, Focusable, KeyBinding, SharedString, TitlebarOptions, WindowBackgroundAppearance, WindowBounds, WindowOptions, prelude::*, px,
    size,
};

fn main() {
    gpui_platform::application().with_assets(assets::Assets).run(|cx: &mut App| {
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
            KeyBinding::new("ctrl-f", FocusSearch, Some("Browser")),
            KeyBinding::new("escape", EndSearch, Some("Browser")),
            KeyBinding::new("ctrl-a", SelectAll, Some("Browser")),
            KeyBinding::new("f2", Rename, Some("Browser")),
            KeyBinding::new("ctrl-shift-n", NewFolder, Some("Browser")),
            KeyBinding::new("ctrl-d", Duplicate, Some("Browser")),
            KeyBinding::new("delete", Trash, Some("Browser")),
            KeyBinding::new("ctrl-backspace", Trash, Some("Browser")),
            KeyBinding::new("ctrl-c", CopyFiles, Some("Browser")),
            KeyBinding::new("ctrl-x", CutFiles, Some("Browser")),
            KeyBinding::new("ctrl-v", PasteFiles, Some("Browser")),
            KeyBinding::new("ctrl-alt-c", CopyPath, Some("Browser")),
            KeyBinding::new("shift-f10", ContextMenu, Some("Browser")),
            KeyBinding::new("ctrl-z", UndoOp, Some("Browser")),
            KeyBinding::new("space", QuickLook, Some("Browser")),
            KeyBinding::new("ctrl-y", QuickLook, Some("Browser")),
            KeyBinding::new("enter", text_input::Confirm, Some("TextInput")),
            KeyBinding::new("escape", text_input::Cancel, Some("TextInput")),
            // The search field (Ctrl stands in for Command).
            KeyBinding::new("backspace", text_input::Backspace, Some("TextInput")),
            KeyBinding::new("delete", text_input::Delete, Some("TextInput")),
            KeyBinding::new("left", text_input::Left, Some("TextInput")),
            KeyBinding::new("right", text_input::Right, Some("TextInput")),
            KeyBinding::new("shift-left", text_input::SelectLeft, Some("TextInput")),
            KeyBinding::new("shift-right", text_input::SelectRight, Some("TextInput")),
            KeyBinding::new("ctrl-a", text_input::SelectAll, Some("TextInput")),
            KeyBinding::new("ctrl-v", text_input::Paste, Some("TextInput")),
            KeyBinding::new("ctrl-c", text_input::Copy, Some("TextInput")),
            KeyBinding::new("ctrl-x", text_input::Cut, Some("TextInput")),
            KeyBinding::new("home", text_input::Home, Some("TextInput")),
            KeyBinding::new("end", text_input::End, Some("TextInput")),
        ]);
        load_local_fonts(cx);
        let font: SharedString = theme::ui_font(&cx.text_system().all_font_names()).into();
        let bounds = Bounds::centered(None, size(px(theme::WINDOW_W), px(theme::WINDOW_H)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions { title: Some("Finder".into()), appears_transparent: true, traffic_light_position: None }),
                    window_background: WindowBackgroundAppearance::Opaque,
                    window_min_size: Some(size(px(theme::WINDOW_MIN_W), px(theme::WINDOW_MIN_H))),
                    ..Default::default()
                },
                |window, cx| {
                    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                    let hwnd = window.window_handle().ok().and_then(|h| match h.as_raw() {
                        RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
                        _ => None,
                    });
                    let view = cx.new(|cx| {
                        // Light/dark follows Windows, live.
                        cx.observe_window_appearance(window, |_, window, _| window.refresh()).detach();
                        // Quick Look's hosted previews follow the window.
                        cx.observe_window_bounds(window, |_, window, _| window.refresh()).detach();
                        let mut b = Browser::new(font, cx);
                        b.hwnd = hwnd;
                        b
                    });
                    window.focus(&view.focus_handle(cx), cx);
                    view
                },
            )
            .expect("open window");
        // FINDER_ACTIONS="finder::Trash,wait:500,finder::UndoOp" dispatches
        // actions at startup, with pauses (testing without the keyboard).
        if let Ok(list) = std::env::var("FINDER_ACTIONS") {
            let steps: Vec<String> = list.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            cx.spawn(async move |cx| {
                for step in steps {
                    if let Some(ms) = step.strip_prefix("wait:").and_then(|m| m.parse().ok()) {
                        cx.background_executor().timer(std::time::Duration::from_millis(ms)).await;
                        continue;
                    }
                    let _ = cx.update(|cx| match cx.build_action(&step, None) {
                        Ok(action) => {
                            let _ = window.update(cx, |_, window, cx| window.dispatch_action(action, cx));
                        }
                        Err(e) => eprintln!("FINDER_ACTIONS: {step}: {e}"),
                    });
                }
            })
            .detach();
        }
        cx.activate(true);
    });
}

/// Fonts in `%LOCALAPPDATA%\Finder\fonts` (SF Pro for local testing: it
/// may not be bundled), loaded for this process only.
fn load_local_fonts(cx: &mut App) {
    let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("Finder").join("fonts")) else { return };
    let fonts: Vec<std::borrow::Cow<'static, [u8]>> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("otf") || x.eq_ignore_ascii_case("ttf")))
        .filter_map(|e| std::fs::read(e.path()).ok().map(Into::into))
        .collect();
    if !fonts.is_empty() {
        let _ = cx.text_system().add_fonts(fonts);
    }
}
