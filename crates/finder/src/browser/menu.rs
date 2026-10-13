//! The right-click menu, drawn like macOS Tahoe's: a rounded glass panel,
//! items highlighted with the accent color, shortcuts on the right.

use super::*;
use gpui::{Action, Point, Pixels, anchored, deferred};
use std::rc::Rc;

type Run = Rc<dyn Fn(&mut Browser, &mut Window, &mut Context<Browser>)>;

pub(super) enum Item {
    Action { label: SharedString, shortcut: Option<&'static str>, action: Box<dyn Action>, enabled: bool, checked: bool },
    /// A choice that carries data, like "sort by Size", with an optional
    /// color dot (a tag's).
    Call { label: SharedString, run: Run, enabled: bool, checked: bool, dot: Option<gpui::Hsla> },
    /// Finder's row of tag colors: click one to toggle it on the selection.
    Swatches(Vec<(Tag, bool)>),
    /// A section title, like "Sort By".
    Header(SharedString),
    Separator,
}

pub(super) struct Menu {
    pub at: Point<Pixels>,
    pub items: Vec<Item>,
}

fn item(label: &str, shortcut: Option<&'static str>, action: impl Action, enabled: bool) -> Item {
    Item::Action { label: label.to_string().into(), shortcut, action: Box::new(action), enabled, checked: false }
}

fn call(label: &str, run: impl Fn(&mut Browser, &mut Window, &mut Context<Browser>) + 'static) -> Item {
    Item::Call { label: label.to_string().into(), run: Rc::new(run), enabled: true, checked: false, dot: None }
}

fn check(i: Item, on: bool) -> Item {
    match i {
        Item::Action { label, shortcut, action, enabled, .. } => Item::Action { label, shortcut, action, enabled, checked: on },
        Item::Call { label, run, enabled, dot, .. } => Item::Call { label, run, enabled, checked: on, dot },
        other => other,
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
        let items = match ix {
            Some(_) => self.selection_items(),
            None => self.folder_items(),
        };
        self.show_menu(at, items, cx);
    }

    pub(super) fn show_menu(&mut self, at: Point<Pixels>, items: Vec<Item>, cx: &mut Context<Self>) {
        self.menu = Some(Menu { at, items });
        cx.notify();
    }

    /// What can be done to the selection.
    fn selection_items(&self) -> Vec<Item> {
        let n = self.selection().len();
        let what = if n == 1 { String::new() } else { format!(" {n} Items") };
        let can_paste = self.search.is_none() && clipboard::get_files().is_some();
        vec![
            item("Open", Some("Ctrl+\u{2193}"), Open, true),
            item("Quick Look", Some("Space"), QuickLook, true),
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
            Item::Separator,
            Item::Swatches(self.tag_states()),
        ]
    }

    /// Each tag, and whether everything selected has it.
    fn tag_states(&self) -> Vec<(Tag, bool)> {
        let sel = self.selection();
        Tag::ALL.into_iter().map(|t| (t, !sel.is_empty() && sel.iter().all(|p| self.tags.of(p).contains(&t)))).collect()
    }

    /// The toolbar's tag button (and the inspector's tag field): each tag,
    /// checked if everything selected has it.
    pub(super) fn open_tag_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let any = !self.selection().is_empty();
        let mut items = vec![Item::Header(if any { "Tags".into() } else { "Select items to tag".into() })];
        for (tag, on) in self.tag_states() {
            items.push(Item::Call {
                label: tag.name().into(),
                run: Rc::new(move |this: &mut Browser, _: &mut Window, cx: &mut Context<Browser>| this.toggle_tag(tag, cx)),
                enabled: any,
                checked: on,
                dot: Some(tag.color()),
            });
        }
        self.show_menu(at, items, cx);
    }

    /// What can be done in the folder itself.
    fn folder_items(&self) -> Vec<Item> {
        let can_paste = self.search.is_none() && clipboard::get_files().is_some();
        vec![
            item("New Folder", Some("Ctrl+Shift+N"), NewFolder, self.search.is_none()),
            Item::Separator,
            item("Paste Item", Some("Ctrl+V"), PasteFiles, can_paste),
            Item::Separator,
            check(item("as Icons", None, ShowIcons, true), self.view == ViewMode::Icons),
            check(item("as List", None, ShowList, true), self.view == ViewMode::List),
            check(item("as Columns", None, ShowColumns, true), self.view == ViewMode::Columns),
            check(item("as Gallery", None, ShowGallery, true), self.view == ViewMode::Gallery),
        ]
    }

    /// The toolbar's "…" (and the inspector's "More…"): Finder's action menu,
    /// for the selection if there is one, else for the folder.
    pub(super) fn open_action_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let mut items = vec![item("New Folder", Some("Ctrl+Shift+N"), NewFolder, self.search.is_none())];
        if self.selected.is_some() {
            items.push(Item::Separator);
            items.extend(self.selection_items());
        } else {
            items.push(item("Paste Item", Some("Ctrl+V"), PasteFiles, self.search.is_none() && clipboard::get_files().is_some()));
            items.push(Item::Separator);
            items.push(call("Show in Explorer", |this, _, _| {
                let _ = std::process::Command::new("explorer").arg(&this.current).spawn();
            }));
            items.push(call("Copy Folder Pathname", |this, _, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(this.current.display().to_string()));
            }));
        }
        self.show_menu(at, items, cx);
    }

    /// The toolbar's group button: how the view is ordered.
    pub(super) fn open_sort_menu(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let (key, ascending) = self.sort;
        let by = |label: &str, k: SortKey| check(call(label, move |this, _, cx| this.sort_by(k, this.sort.1, cx)), key == k);
        let items = vec![
            Item::Header("Sort By".into()),
            by("Name", SortKey::Name),
            by("Kind", SortKey::Kind),
            by("Date Modified", SortKey::Modified),
            by("Size", SortKey::Size),
            Item::Separator,
            check(call("Ascending", |this, _, cx| this.sort_by(this.sort.0, true, cx)), ascending),
            check(call("Descending", |this, _, cx| this.sort_by(this.sort.0, false, cx)), !ascending),
        ];
        self.show_menu(at, items, cx);
    }

    pub(super) fn render_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        let mut panel = div()
            .id("context-menu")
            .min_w(px(220.))
            .p(px(5.))
            .flex()
            .flex_col()
            .relative()
            .rounded(px(t::MENU_RADIUS))
            // Liquid Glass over whatever the menu covers, as in macOS 27.
            .child(
                gpui::canvas(|_, _, _| (), |bounds, _, window, _| window.paint_glass(bounds, px(t::MENU_RADIUS), t::menu_glass()))
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .text_size(px(t::TEXT_SIZE))
            .text_color(t::text())
            // Clicks inside the panel don't fall through to the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        for (i, it) in menu.items.iter().enumerate() {
            panel = panel.child(match it {
                Item::Separator => div().h(px(1.)).mx(px(10.)).my(px(4.)).bg(t::separator()).into_any_element(),
                Item::Header(label) => div()
                    .h(px(22.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .text_size(px(t::SMALL_SIZE))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(t::text_secondary())
                    .child(label.clone())
                    .into_any_element(),
                Item::Swatches(states) => div()
                    .h(px(28.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .children(states.iter().enumerate().map(|(k, &(tag, on))| {
                        div()
                            .id(("swatch", k))
                            .size(px(16.))
                            .rounded_full()
                            .bg(tag.color())
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.border_2().border_color(gpui::white()))
                            .when(on, |d| d.child(sym("checkmark@semibold", 7., gpui::white())))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.menu = None;
                                window.focus(&this.focus, cx);
                                this.toggle_tag(tag, cx);
                            }))
                    }))
                    .into_any_element(),
                Item::Call { label, run, enabled, checked, dot } => {
                    let run = run.clone();
                    let enabled = *enabled;
                    div()
                        .id(("menu-item", i))
                        .h(px(24.))
                        .pl(px(4.))
                        .pr(px(10.))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .when(!enabled, |d| d.text_color(t::text_tertiary()))
                        .when(enabled, |d| d.hover(|s| s.bg(t::selection()).text_color(gpui::white())))
                        .child(div().w(px(16.)).flex().justify_center().when(*checked, |d| d.child(sym("checkmark@semibold", 9., t::text()))))
                        .children(dot.map(|c| div().size(px(10.)).mr(px(7.)).rounded_full().bg(c)))
                        .child(label.clone())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if !enabled {
                                return;
                            }
                            this.menu = None;
                            window.focus(&this.focus, cx);
                            run(this, window, cx);
                            cx.notify();
                        }))
                        .into_any_element()
                }
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

    /// The action menu without a click: under the toolbar's right end.
    pub(super) fn action_menu_key(&mut self, _: &ShowActionMenu, window: &mut Window, cx: &mut Context<Self>) {
        let at = gpui::point(window.viewport_size().width - px(t::INSPECTOR_W + 240.), px(t::TOOLBAR_H - 6.));
        self.open_action_menu(at, cx);
    }

    /// The sort menu without a click: under the toolbar's group button.
    pub(super) fn sort_menu_key(&mut self, _: &ShowSortMenu, window: &mut Window, cx: &mut Context<Self>) {
        let at = gpui::point(window.viewport_size().width - px(t::INSPECTOR_W + 300.), px(t::TOOLBAR_H - 6.));
        self.open_sort_menu(at, cx);
    }

    /// The tag menu without a click: under the toolbar's tag button.
    pub(super) fn tag_menu_key(&mut self, _: &ShowTagMenu, window: &mut Window, cx: &mut Context<Self>) {
        let at = gpui::point(window.viewport_size().width - px(t::INSPECTOR_W + 200.), px(t::TOOLBAR_H - 6.));
        self.open_tag_menu(at, cx);
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
