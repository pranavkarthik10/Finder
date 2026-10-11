//! The right-click menu, drawn like macOS Tahoe's: a rounded glass panel,
//! items highlighted with the accent color, shortcuts on the right.

use super::*;
use gpui::{Action, Point, Pixels, anchored, deferred};

pub(super) enum Item {
    Action { label: SharedString, shortcut: Option<&'static str>, action: Box<dyn Action>, enabled: bool, checked: bool },
    Separator,
}

pub(super) struct Menu {
    pub at: Point<Pixels>,
    pub items: Vec<Item>,
}

fn item(label: &str, shortcut: Option<&'static str>, action: impl Action, enabled: bool) -> Item {
    Item::Action { label: label.to_string().into(), shortcut, action: Box::new(action), enabled, checked: false }
}

fn check(i: Item, on: bool) -> Item {
    match i {
        Item::Action { label, shortcut, action, enabled, .. } => Item::Action { label, shortcut, action, enabled, checked: on },
        sep => sep,
    }
}

impl Browser {
    /// Right-click on row `ix` (None: on the background).
    pub(super) fn open_menu(&mut self, ix: Option<usize>, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        // Right-clicking an item outside the selection selects just it.
        if let Some(ix) = ix
            && !self.is_selected(ix)
        {
            self.select(ix, cx);
        }
        let can_paste = self.search.is_none() && clipboard::get_files().is_some();
        let items = match ix {
            Some(_) => {
                let n = self.selection().len();
                let what = if n == 1 { String::new() } else { format!(" {n} Items") };
                vec![
                    item("Open", Some("Ctrl+\u{2193}"), Open, true),
                    item("Show in Explorer", None, Reveal, true),
                    Item::Separator,
                    item("Move to Recycle Bin", Some("Del"), Trash, true),
                    Item::Separator,
                    item("Rename", Some("F2"), Rename, n == 1),
                    item("Duplicate", Some("Ctrl+D"), Duplicate, true),
                    Item::Separator,
                    item(&format!("Copy{what}"), Some("Ctrl+C"), CopyFiles, true),
                    item("Copy as Pathname", Some("Ctrl+Alt+C"), CopyPath, true),
                    item("Paste Item", Some("Ctrl+V"), PasteFiles, can_paste),
                ]
            }
            None => vec![
                item("New Folder", Some("Ctrl+Shift+N"), NewFolder, self.search.is_none()),
                Item::Separator,
                item("Paste Item", Some("Ctrl+V"), PasteFiles, can_paste),
                Item::Separator,
                check(item("as Icons", None, ShowIcons, true), self.view == ViewMode::Icons),
                check(item("as List", None, ShowList, true), self.view == ViewMode::List),
                check(item("as Columns", None, ShowColumns, true), self.view == ViewMode::Columns),
                check(item("as Gallery", None, ShowGallery, true), self.view == ViewMode::Gallery),
            ],
        };
        self.menu = Some(Menu { at, items });
        cx.notify();
    }

    pub(super) fn render_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        let mut panel = div()
            .id("context-menu")
            .min_w(px(220.))
            .p(px(5.))
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(t::menu_bg())
            .border(px(0.5))
            .border_color(t::glass_border())
            .shadow(t::menu_shadow())
            .text_size(px(t::TEXT_SIZE))
            .text_color(t::text())
            // Clicks inside the panel don't fall through to the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        for (i, it) in menu.items.iter().enumerate() {
            panel = panel.child(match it {
                Item::Separator => div().h(px(1.)).mx(px(10.)).my(px(4.)).bg(t::separator()).into_any_element(),
                Item::Action { label, shortcut, action, enabled, checked } => {
                    let action = action.boxed_clone();
                    let enabled = *enabled;
                    div()
                        .id(("menu-item", i))
                        .h(px(24.))
                        .pl(px(4.))
                        .pr(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(24.))
                        .rounded(px(6.))
                        .when(!enabled, |d| d.text_color(t::text_tertiary()))
                        .when(enabled, |d| d.hover(|s| s.bg(t::selection()).text_color(gpui::white())))
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .items_center()
                                .child(div().w(px(16.)).flex().justify_center().when(*checked, |d| d.child(sym("checkmark@semibold", 9., t::text()))))
                                .child(label.clone()),
                        )
                        .children(shortcut.map(|s| div().text_color(t::text_tertiary()).child(s)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if !enabled {
                                return;
                            }
                            this.menu = None;
                            window.focus(&this.focus, cx);
                            window.dispatch_action(action.boxed_clone(), cx);
                            cx.notify();
                        }))
                        .into_any_element()
                }
            });
        }
        Some(
            deferred(
                // A full-window backdrop: clicking anywhere else closes the menu.
                div()
                    .id("menu-backdrop")
                    .absolute()
                    .inset_0()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.notify();
                        }),
                    )
                    .child(anchored().position(menu.at).snap_to_window_with_margin(px(8.)).child(panel)),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }

    /// Shift+F10 / the menu key: the menu for the selection, by the list.
    pub(super) fn context_menu_key(&mut self, _: &ContextMenu, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.selected;
        let row = ix.map_or(0., |i| i as f32);
        let at = gpui::point(px(t::SIDEBAR_W + 160.), px(t::TOOLBAR_H + t::HEADER_H + (row + 1.) * t::ROW_H));
        self.open_menu(ix, at, window, cx);
    }

    pub(super) fn reveal_in_explorer(&mut self, _: &Reveal, _: &mut Window, _: &mut Context<Self>) {
        if let Some(e) = self.selected_entry() {
            let _ = std::process::Command::new("explorer").arg(format!("/select,{}", e.path.display())).spawn();
        }
    }

    pub(super) fn show_icons(&mut self, _: &ShowIcons, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(ViewMode::Icons, cx);
    }

    pub(super) fn show_list(&mut self, _: &ShowList, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(ViewMode::List, cx);
    }

    pub(super) fn show_columns(&mut self, _: &ShowColumns, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(ViewMode::Columns, cx);
    }

    pub(super) fn show_gallery(&mut self, _: &ShowGallery, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(ViewMode::Gallery, cx);
    }
}
