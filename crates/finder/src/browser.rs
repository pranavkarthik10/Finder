//! One Finder window: sidebar, toolbar, list view, inspector, path bar.

use crate::files::{self, Entry, SortKey};
use crate::assets::symbol;
use crate::icons;
use crate::text_input::{self, TextInput};
use crate::theme as t;
use gpui::{
    AnyElement, App, ClickEvent, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, MouseButton, ParentElement, Render,
    ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, UniformListScrollHandle, Window, WindowControlArea, actions, div, img,
    prelude::*, px, svg, uniform_list,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

actions!(finder, [SelectNext, SelectPrev, Expand, Collapse, Open, GoBack, GoForward, GoUp, FocusSearch, EndSearch]);

/// Most results a search shows.
const SEARCH_LIMIT: usize = 1000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Icons,
    List,
    Columns,
    Gallery,
}

/// One visible row of the (expandable) list.
struct Row {
    entry: Entry,
    depth: u16,
    expanded: bool,
}

struct Place {
    name: SharedString,
    icon: &'static str,
    path: Option<PathBuf>,
}

struct Section {
    title: &'static str,
    places: Vec<Place>,
}

pub struct Browser {
    font: SharedString,
    current: PathBuf,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    rows: Vec<Row>,
    selected: Option<usize>,
    sort: (SortKey, bool),
    view: ViewMode,
    sections: Vec<Section>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    error: Option<String>,
    icons: HashMap<icons::Key, Icon>,
    search_input: gpui::Entity<TextInput>,
    /// The whole-disk index (fsearch), in-process.
    engine: Option<fsearch::Engine>,
    /// Set while the search field has text: the list shows results.
    search: Option<Search>,
    /// Bumped per query; results of older ones are dropped.
    search_gen: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    ThisPc,
    Folder,
}

struct Search {
    /// The folder the search began in (the "Folder" scope).
    root: PathBuf,
    scope: Scope,
    took: Option<Duration>,
    capped: bool,
    /// Why there are no results (still indexing, a bad query).
    error: Option<String>,
    /// Sorted by a column; else by relevance.
    sorted: bool,
}

enum Icon {
    Loading,
    Ready(Arc<gpui::RenderImage>),
    Failed,
}

impl Focusable for Browser {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".into()))
}

fn sections() -> Vec<Section> {
    let h = home();
    let known = |name: &str, icon, sub: &str| {
        let p = h.join(sub);
        p.is_dir().then(|| Place { name: SharedString::from(name.to_string()), icon, path: Some(p) })
    };
    let favorites = [
        Some(Place { name: "Applications".into(), icon: "applications", path: Some(PathBuf::from(r"C:\Program Files")) }),
        known("Desktop", "menubar.dock.rectangle", "Desktop"),
        known("Documents", "document", "Documents"),
        known("Downloads", "arrow.down.circle", "Downloads"),
        known("Developer", "hammer", "code"),
    ];
    let user = h.file_name().map_or("Home".into(), |n| n.to_string_lossy().into_owned());
    let mut locations = vec![known("OneDrive", "icloud", "OneDrive"), Some(Place { name: user.into(), icon: "house", path: Some(h.clone()) })];
    for d in fsearch::walk::drives() {
        let root = PathBuf::from(format!("{}:\\", d as char));
        locations.push(Some(Place { name: files::display_name(&root).into(), icon: "internaldrive", path: Some(root) }));
    }
    locations.push(Some(Place { name: "Network".into(), icon: "network", path: None }));
    locations.push(Some(Place { name: "Recycle Bin".into(), icon: "trash", path: None }));
    vec![
        Section { title: "Favorites", places: favorites.into_iter().flatten().collect() },
        Section { title: "Locations", places: locations.into_iter().flatten().collect() },
    ]
}

impl Browser {
    pub fn new(font: SharedString, cx: &mut Context<Self>) -> Browser {
        let start = home().join("Desktop");
        let mut b = Browser {
            font,
            current: PathBuf::new(),
            back: Vec::new(),
            forward: Vec::new(),
            rows: Vec::new(),
            selected: None,
            sort: (SortKey::Kind, true),
            view: ViewMode::List,
            sections: sections(),
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            error: None,
            icons: HashMap::new(),
            search_input: cx.new(|cx| TextInput::new("Search", cx)),
            engine: fsearch::Engine::start(fsearch::Options { dir: fsearch::sys::data_dir(), home: fsearch::sys::home(), skip: None }).ok(),
            search: None,
            search_gen: 0,
        };
        cx.subscribe(&b.search_input, |this, _, _: &text_input::Changed, cx| this.query_changed(cx)).detach();
        b.load(if start.is_dir() { start } else { home() });
        b
    }

    // Navigation

    fn load(&mut self, dir: PathBuf) {
        match files::list(&dir) {
            Ok(mut v) => {
                files::sort(&mut v, self.sort.0, self.sort.1);
                self.rows = v.into_iter().map(|entry| Row { entry, depth: 0, expanded: false }).collect();
                self.error = None;
            }
            Err(e) => {
                self.rows.clear();
                self.error = Some(e.to_string());
            }
        }
        self.current = dir;
        self.selected = None;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    fn navigate(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        self.end_search(cx);
        if dir == self.current {
            return;
        }
        self.back.push(std::mem::take(&mut self.current));
        self.forward.clear();
        self.load(dir);
        cx.notify();
    }

    fn go_back(&mut self, _: &GoBack, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.back.pop() {
            self.forward.push(std::mem::take(&mut self.current));
            self.load(p);
            cx.notify();
        }
    }

    fn go_forward(&mut self, _: &GoForward, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.forward.pop() {
            self.back.push(std::mem::take(&mut self.current));
            self.load(p);
            cx.notify();
        }
    }

    fn go_up(&mut self, _: &GoUp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = self.current.parent() {
            let (from, to) = (self.current.clone(), p.to_path_buf());
            self.navigate(to, cx);
            // Land on the folder we came out of.
            self.selected = self.rows.iter().position(|r| r.entry.path == from);
            if let Some(i) = self.selected {
                self.scroll.scroll_to_item(i, ScrollStrategy::Center);
            }
        }
    }

    /// Open a row: folders in place, files with their default app.
    fn open_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(r) = self.rows.get(ix) else { return };
        if r.entry.is_dir {
            let p = r.entry.path.clone();
            self.navigate(p, cx);
        } else {
            let _ = std::process::Command::new("explorer").arg(&r.entry.path).spawn();
        }
    }

    fn set_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        self.sort = if self.sort.0 == key { (key, !self.sort.1) } else { (key, true) };
        if let Some(s) = &mut self.search {
            s.sorted = true;
            let sel = self.selected.map(|i| self.rows[i].entry.path.clone());
            let mut v: Vec<Entry> = self.rows.drain(..).map(|r| r.entry).collect();
            files::sort(&mut v, self.sort.0, self.sort.1);
            self.rows = v.into_iter().map(|entry| Row { entry, depth: 0, expanded: false }).collect();
            self.selected = sel.and_then(|p| self.rows.iter().position(|r| r.entry.path == p));
            cx.notify();
            return;
        }
        let sel = self.selected.map(|i| self.rows[i].entry.path.clone());
        let cur = self.current.clone();
        let expanded: Vec<PathBuf> = self.rows.iter().filter(|r| r.expanded).map(|r| r.entry.path.clone()).collect();
        self.load(cur);
        // Re-open what was open, top-down so nested folders follow.
        for p in expanded {
            if let Some(i) = self.rows.iter().position(|r| r.entry.path == p) {
                self.expand(i);
            }
        }
        self.selected = sel.and_then(|p| self.rows.iter().position(|r| r.entry.path == p));
        cx.notify();
    }

    // Expanding folders in place

    fn expand(&mut self, ix: usize) {
        let r = &self.rows[ix];
        if !r.entry.is_dir || r.expanded || self.search.is_some() {
            return;
        }
        let depth = r.depth + 1;
        let mut kids = files::list(&r.entry.path).unwrap_or_default();
        files::sort(&mut kids, self.sort.0, self.sort.1);
        self.rows[ix].expanded = true;
        self.rows.splice(ix + 1..ix + 1, kids.into_iter().map(|entry| Row { entry, depth, expanded: false }));
    }

    fn collapse(&mut self, ix: usize) {
        let depth = self.rows[ix].depth;
        let end = (ix + 1..self.rows.len()).find(|&j| self.rows[j].depth <= depth).unwrap_or(self.rows.len());
        self.rows.drain(ix + 1..end);
        self.rows[ix].expanded = false;
        if let Some(s) = self.selected
            && s > ix
        {
            self.selected = if s < end { Some(ix) } else { Some(s - (end - ix - 1)) };
        }
    }

    fn toggle(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.rows[ix].expanded { self.collapse(ix) } else { self.expand(ix) }
        cx.notify();
    }

    /// The picture for an entry at `px` pixels: our folder, else the
    /// shell's icon or thumbnail once loaded (the generic one meanwhile).
    fn icon_source(&mut self, e: &Entry, px: u32, cx: &mut Context<Self>) -> gpui::ImageSource {
        if e.is_dir {
            return e.icon().into();
        }
        let key = icons::key(&e.path, px);
        match self.icons.get(&key) {
            Some(Icon::Ready(img)) => return gpui::ImageSource::Render(img.clone()),
            Some(_) => return e.icon().into(),
            None => {}
        }
        self.icons.insert(key.clone(), Icon::Loading);
        let path = e.path.clone();
        cx.spawn(async move |this, cx| {
            let img = cx.background_spawn(async move { icons::load(&path, px) }).await;
            let _ = this.update(cx, |b, cx| {
                b.icons.insert(key, img.map_or(Icon::Failed, Icon::Ready));
                cx.notify();
            });
        })
        .detach();
        e.icon().into()
    }

    // Search

    fn query_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.search_input.read(cx).text().trim().to_string();
        if text.is_empty() {
            if self.search.take().is_some() {
                let cur = self.current.clone();
                self.load(cur);
                cx.notify();
            }
            return;
        }
        if self.search.is_none() {
            self.search = Some(Search { root: self.current.clone(), scope: Scope::ThisPc, took: None, capped: false, error: None, sorted: false });
        }
        self.run_search(text, cx);
    }

    fn run_search(&mut self, text: String, cx: &mut Context<Self>) {
        let (Some(engine), Some(s)) = (self.engine.clone(), self.search.as_mut()) else { return };
        let mut q = match fsearch::Query::parse(&text, &fsearch::sys::home()) {
            Ok(q) => q,
            Err(e) => {
                s.error = Some(e);
                cx.notify();
                return;
            }
        };
        q.limit = SEARCH_LIMIT;
        if s.scope == Scope::Folder {
            q.scope = Some(fsearch::sys::index_path(&s.root));
        }
        self.search_gen += 1;
        let generation = self.search_gen;
        cx.spawn(async move |this, cx| {
            let t = Instant::now();
            let res = cx.background_spawn(async move { engine.search(&q) }).await;
            let took = t.elapsed();
            let _ = this.update(cx, |b, cx| b.show_results(generation, text, res, took, cx));
        })
        .detach();
    }

    fn show_results(&mut self, generation: u64, text: String, res: Result<Vec<fsearch::Found>, String>, took: Duration, cx: &mut Context<Self>) {
        if generation != self.search_gen {
            return;
        }
        let Some(s) = self.search.as_mut() else { return };
        match res {
            Ok(found) => {
                s.error = None;
                s.took = Some(took);
                s.capped = found.len() >= SEARCH_LIMIT;
                let mut v: Vec<Entry> = found
                    .into_iter()
                    .map(|f| {
                        let name = f.path.file_name().map_or_else(|| f.path.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
                        let modified = Some(UNIX_EPOCH + Duration::from_secs(f.mtime as u64));
                        Entry::from_parts(name, f.path, f.kind & 3 == fsearch::walk::KIND_DIR, f.size, modified)
                    })
                    .collect();
                if s.sorted {
                    files::sort(&mut v, self.sort.0, self.sort.1);
                }
                self.rows = v.into_iter().map(|entry| Row { entry, depth: 0, expanded: false }).collect();
                self.selected = None;
                self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
            Err(e) => {
                // Still indexing: ask again shortly.
                let retry = e.starts_with("indexing");
                s.error = Some(e);
                if retry {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(Duration::from_secs(1)).await;
                        let _ = this.update(cx, |b, cx| {
                            if b.search_gen == generation && b.search.is_some() {
                                b.run_search(text, cx);
                            }
                        });
                    })
                    .detach();
                }
            }
        }
        cx.notify();
    }

    fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        if let Some(s) = &mut self.search
            && s.scope != scope
        {
            s.scope = scope;
            let text = self.search_input.read(cx).text().trim().to_string();
            self.run_search(text, cx);
        }
    }

    /// Leave search: clear the field, back to the folder.
    fn end_search(&mut self, cx: &mut Context<Self>) {
        if self.search.take().is_some() {
            self.search_gen += 1;
            self.search_input.update(cx, |i, cx| i.set_text("", cx));
            let cur = self.current.clone();
            self.load(cur);
            cx.notify();
        }
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        let h = self.search_input.focus_handle(cx);
        window.focus(&h, cx);
        cx.notify();
    }

    fn escape(&mut self, _: &EndSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.end_search(cx);
        window.focus(&self.focus, cx);
    }

    // Keyboard

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = Some(ix);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        // From the search field too: the arrow moves into the results.
        window.focus(&self.focus, cx);
        if self.rows.is_empty() {
            return;
        }
        let ix = self.selected.map_or(0, |i| (i + 1).min(self.rows.len() - 1));
        self.select(ix, cx);
    }

    fn select_prev(&mut self, _: &SelectPrev, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        if self.rows.is_empty() {
            return;
        }
        let ix = self.selected.map_or(self.rows.len() - 1, |i| i.saturating_sub(1));
        self.select(ix, cx);
    }

    fn expand_selected(&mut self, _: &Expand, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.selected {
            self.expand(i);
            cx.notify();
        }
    }

    /// Left arrow: fold the folder, or else go to its parent row.
    fn collapse_selected(&mut self, _: &Collapse, _: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.selected else { return };
        if self.rows[i].expanded {
            self.collapse(i);
        } else if self.rows[i].depth > 0 {
            let d = self.rows[i].depth;
            if let Some(p) = (0..i).rev().find(|&j| self.rows[j].depth < d) {
                self.selected = Some(p);
                self.scroll.scroll_to_item(p, ScrollStrategy::Nearest);
            }
        }
        cx.notify();
    }

    fn open_selected(&mut self, _: &Open, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.selected {
            self.open_row(i, cx);
        }
    }
}


// Pieces

/// An SF Symbol (or its stand-in) at `pt` points: symbols sit in a square
/// 1.25 em frame, so this is the box that shows them at that size.
fn sym(name: &str, pt: f32, color: impl Into<gpui::Hsla>) -> gpui::Svg {
    svg().path(symbol(name)).size(px(pt * 1.25)).flex_none().text_color(color.into())
}

/// A Liquid Glass capsule floating over the content, holding buttons.
fn capsule(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(t::CAPSULE_H))
        .flex_none()
        .flex()
        .items_center()
        .px(px(4.))
        .gap(px(2.))
        .rounded_full()
        .bg(t::glass())
        .border(px(0.5))
        .border_color(t::glass_border())
        .shadow(t::glass_shadow())
}

/// One icon button inside a capsule.
fn tool_button(id: impl Into<gpui::ElementId>, name: &str, on: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(t::CAPSULE_H - 8.))
        .min_w(px(38.))
        .px(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .when(on, |d| d.bg(t::glass_pressed()))
        .hover(|s| s.bg(t::glass_pressed()))
        .child(sym(name, t::TOOL_SYMBOL, t::glass_icon()))
}

impl Browser {
    fn traffic_lights(&self, window: &Window) -> impl IntoElement {
        let active = window.is_window_active();
        let light = |id: &'static str, color: gpui::Rgba, glyph: &'static str, area: WindowControlArea| {
            div()
                .id(id)
                .size(px(t::LIGHT_D))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(if active { color } else { t::light_inactive() })
                .border(px(0.5))
                .border_color(t::light_border())
                .window_control_area(area)
                .child(
                    svg()
                        .path(symbol(glyph))
                        .size(px(t::LIGHT_D * 0.8))
                        .text_color(gpui::transparent_black())
                        .group_hover("lights", |s| s.text_color(t::light_glyph())),
                )
        };
        div()
            .group("lights")
            .absolute()
            .top(px(t::LIGHTS_Y))
            .left(px(t::LIGHTS_X))
            .flex()
            .gap(px(t::LIGHT_GAP))
            .child(light("close", t::light_close(), "xmark@semibold", WindowControlArea::Close))
            .child(light("min", t::light_min(), "minus@semibold", WindowControlArea::Min))
            .child(light("max", t::light_max(), "plus@semibold", WindowControlArea::Max))
    }

    fn sidebar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div().id("sidebar-scroll").flex().flex_col().flex_1().min_h_0().overflow_y_scroll().px(px(t::SIDEBAR_INSET)).pb(px(12.));
        for (si, s) in self.sections.iter().enumerate() {
            col = col.child(
                div()
                    .h(px(28.))
                    .mt(px(if si == 0 { 0. } else { 8. }))
                    .px(px(10.))
                    .flex()
                    .flex_none()
                    .items_end()
                    .pb(px(6.))
                    .text_size(px(t::SMALL_SIZE))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(t::sidebar_header())
                    .child(s.title),
            );
            for (pi, p) in s.places.iter().enumerate() {
                let on = p.path.as_deref() == Some(self.current.as_path());
                let path = p.path.clone();
                let tint = if on { t::accent() } else { t::sidebar_icon() };
                col = col.child(
                    div()
                        .id(("place", si * 100 + pi))
                        .h(px(t::SIDEBAR_ROW_H))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .px(px(8.))
                        .rounded(px(10.))
                        .when(on, |d| d.bg(t::sidebar_selected()))
                        .child(div().w(px(22.)).flex().justify_center().child(sym(p.icon, t::SIDEBAR_SYMBOL, tint)))
                        .child(div().text_size(px(t::TEXT_SIZE)).when(on, |d| d.text_color(t::accent())).truncate().child(p.name.clone()))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(p) = &path {
                                this.navigate(p.clone(), cx);
                            }
                        })),
                );
            }
        }
        // Flush with the window's edges (macOS 27 dropped Tahoe's floating
        // panel), divided from the content by a hairline.
        div()
            .relative()
            .w(px(t::SIDEBAR_W))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(t::sidebar_bg())
            .border_r_1()
            .border_color(t::separator())
            // Title-bar strip: drags the window.
            .child(div().h(px(t::TOOLBAR_H)).flex_none().window_control_area(WindowControlArea::Drag))
            .child(col)
            .child(self.traffic_lights(window))
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = match &self.search {
            Some(s) if s.scope == Scope::Folder => format!("Searching \u{201c}{}\u{201d}", files::display_name(&s.root)),
            Some(_) => "Searching \u{201c}This PC\u{201d}".to_string(),
            None => files::display_name(&self.current),
        };
        let view_btn = |id: &'static str, name: &'static str, mode: ViewMode, cx: &mut Context<Self>| {
            tool_button(id, name, self.view == mode).on_click(cx.listener(move |this, _, _, cx| {
                this.view = mode;
                cx.notify();
            }))
        };
        div()
            .h(px(t::TOOLBAR_H))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(12.))
            .pr(px(12.))
            .gap(px(t::CAPSULE_GAP))
            .window_control_area(WindowControlArea::Drag)
            .child(
                capsule("nav")
                    .child(
                        tool_button("back", "chevron.left@medium", false)
                            .when(self.back.is_empty(), |d| d.opacity(0.3))
                            .on_click(cx.listener(|this, _, w, cx| this.go_back(&GoBack, w, cx))),
                    )
                    .child(div().w(px(1.)).h(px(16.)).bg(t::separator()))
                    .child(
                        tool_button("forward", "chevron.right@medium", false)
                            .when(self.forward.is_empty(), |d| d.opacity(0.3))
                            .on_click(cx.listener(|this, _, w, cx| this.go_forward(&GoForward, w, cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px(px(4.))
                    .text_size(px(t::TITLE_SIZE))
                    .font_weight(gpui::FontWeight::BOLD)
                    .truncate()
                    .child(title),
            )
            .child(
                capsule("views")
                    .child(view_btn("v-icons", "square.grid.2x2", ViewMode::Icons, cx))
                    .child(view_btn("v-list", "list.bullet", ViewMode::List, cx))
                    .child(view_btn("v-columns", "rectangle.split.3x1", ViewMode::Columns, cx))
                    .child(view_btn("v-gallery", "squares.below.rectangle", ViewMode::Gallery, cx)),
            )
            .child(
                capsule("group-cap").child(
                    tool_button("group", "square.grid.3x1.below.line.grid.1x2", false)
                        .gap(px(1.))
                        .child(sym("chevron.down@semibold", 8., t::glass_icon())),
                ),
            )
            .child(
                capsule("actions")
                    .child(tool_button("share", "square.and.arrow.up", false))
                    .child(tool_button("tags", "tag", false))
                    .child(tool_button("more", "ellipsis", false)),
            )
    }

    /// The inspector column's own strip of toolbar: the search field.
    fn search_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.search_input.focus_handle(cx).is_focused(window);
        let has_text = !self.search_input.read(cx).text().is_empty();
        div().h(px(t::TOOLBAR_H)).flex_none().flex().items_center().px(px(12.)).window_control_area(WindowControlArea::Drag).child(
            capsule("search")
                .flex_1()
                .px(px(10.))
                .gap(px(5.))
                .when(focused, |d| d.border_2().border_color(t::focus_ring()))
                .child(sym("magnifyingglass", 13., t::text_secondary()))
                .child(self.search_input.clone())
                .when(has_text, |d| {
                    d.child(
                        div()
                            .id("clear-search")
                            .child(sym("xmark.circle.fill", 13., t::text_tertiary()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.end_search(cx);
                                let h = this.search_input.focus_handle(cx);
                                window.focus(&h, cx);
                            })),
                    )
                })
                .on_click(cx.listener(|this, _, window, cx| this.focus_search(&FocusSearch, window, cx))),
        )
    }

    /// "Search: This PC | "Folder"", under the toolbar while searching.
    fn scope_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.search.as_ref().expect("searching");
        let button = |id: &'static str, label: String, on: bool, scope: Scope, cx: &mut Context<Self>| {
            div()
                .id(id)
                .px(px(9.))
                .py(px(3.))
                .rounded(px(6.))
                .when(on, |d| d.bg(t::sidebar_selected()).text_color(t::text()).font_weight(gpui::FontWeight::SEMIBOLD))
                .hover(|d| d.bg(t::glass_pressed()))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.set_scope(scope, cx)))
        };
        let plain = |id: &'static str, label: &'static str| div().id(id).px(px(9.)).py(px(3.)).rounded(px(6.)).hover(|d| d.bg(t::glass_pressed())).child(label);
        div()
            .h(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.))
            .px(px(14.))
            .text_size(px(t::HEADER_SIZE))
            .text_color(t::text_secondary())
            .child(div().mr(px(4.)).child("Search:"))
            .child(button("scope-pc", "This PC".into(), s.scope == Scope::ThisPc, Scope::ThisPc, cx))
            .child(button("scope-folder", format!("\u{201c}{}\u{201d}", files::display_name(&s.root)), s.scope == Scope::Folder, Scope::Folder, cx))
            .child(div().flex_1())
            .child(plain("save-search", "Save"))
            .child(plain("add-criteria", "+"))
    }

    fn column_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let col = |id: &'static str, label: &'static str, key: SortKey, cx: &mut Context<Self>| {
            let on = self.sort.0 == key;
            div()
                .id(id)
                .h_full()
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(8.))
                .when(on, |d| d.font_weight(gpui::FontWeight::SEMIBOLD).text_color(t::text()))
                .child(div().flex_1().truncate().child(label))
                .when(on, |d| d.child(sym(if self.sort.1 { "chevron.up@semibold" } else { "chevron.down@semibold" }, 8., t::text_secondary())))
                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(key, cx)))
        };
        let divider = || div().w(px(1.)).h(px(14.)).flex_none().bg(t::separator());
        div()
            .h(px(t::HEADER_H))
            .flex_none()
            .flex()
            .items_center()
            .px(px(t::ROW_INSET))
            .border_b_1()
            .border_color(t::separator())
            .text_size(px(t::HEADER_SIZE))
            .text_color(t::text_secondary())
            .child(col("h-name", "Name", SortKey::Name, cx).flex_1().pl(px(46.)))
            .child(divider())
            .child(col("h-date", "Date Modified", SortKey::Modified, cx).w(px(t::DATE_W - 1.)))
            .child(divider())
            .child(col("h-size", "Size", SortKey::Size, cx).w(px(t::SIZE_W - 1.)))
            .child(divider())
            .child(col("h-kind", "Kind", SortKey::Kind, cx).w(px(t::KIND_W - 1.)))
    }

    fn render_rows(&mut self, range: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let focused = self.focus.is_focused(window);
        let searching = self.search.is_some();
        range
            .map(|ix| {
                // Past the last row: Finder keeps the stripes going.
                if ix >= self.rows.len() {
                    return div()
                        .id(("filler", ix))
                        .w_full()
                        .h(px(t::ROW_H))
                        .px(px(t::ROW_INSET))
                        .child(div().size_full().rounded(px(t::ROW_RADIUS)).when(ix % 2 == 1, |d| d.bg(t::zebra())))
                        .on_click(cx.listener(|this, _, window, cx| {
                            window.focus(&this.focus, cx);
                            this.selected = None;
                            cx.notify();
                        }))
                        .into_any_element();
                }
                let entry = self.rows[ix].entry.clone();
                let src = self.icon_source(&entry, 32, cx);
                let r = &self.rows[ix];
                let sel = self.selected == Some(ix);
                let lit = sel && focused;
                let (fg, fg2): (gpui::Hsla, gpui::Hsla) =
                    if lit { (gpui::white(), gpui::white().opacity(0.85)) } else { (t::text().into(), t::text_secondary().into()) };
                let indent = r.depth as f32 * t::INDENT;
                div()
                    .w_full()
                    .px(px(t::ROW_INSET))
                    .flex()
                    .child(
                        div()
                            .id(("row", ix))
                            .flex_1()
                            .min_w_0()
                            .h(px(t::ROW_H))
                            .flex()
                            .items_center()
                            .rounded(px(t::ROW_RADIUS))
                            .when(ix % 2 == 1, |d| d.bg(t::zebra()))
                            .when(sel, |d| d.bg(if focused { t::selection() } else { t::selection_inactive() }))
                            .text_size(px(t::TEXT_SIZE))
                            .text_color(fg)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .pl(px(4. + indent))
                                    .child(
                                        div()
                                            .id(("disclosure", ix))
                                            .w(px(18.))
                                            .h_full()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .when(r.entry.is_dir && !searching, |d| {
                                                d.child(sym(if r.expanded { "chevron.down@semibold" } else { "chevron.right@semibold" }, 8., fg2)).on_click(
                                                    cx.listener(move |this, _, _, cx| {
                                                        cx.stop_propagation();
                                                        this.toggle(ix, cx);
                                                    }),
                                                )
                                            }),
                                    )
                                    .child(img(src).size(px(t::ROW_ICON)).flex_none().ml(px(2.)).mr(px(6.)))
                                    .child(div().flex_1().min_w_0().truncate().child(r.entry.name.clone())),
                            )
                            .child(div().w(px(t::DATE_W)).px(px(9.)).truncate().text_color(fg2).child(files::format_date(r.entry.modified)))
                            .child(div().w(px(t::SIZE_W)).px(px(9.)).flex().justify_end().text_color(fg2).child(files::format_size(&r.entry)))
                            .child(div().w(px(t::KIND_W)).px(px(9.)).truncate().text_color(fg2).child(r.entry.kind.clone()))
                            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                                window.focus(&this.focus, cx);
                                if e.click_count() >= 2 {
                                    this.open_row(ix, cx);
                                } else {
                                    this.select(ix, cx);
                                }
                            })),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn list(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fill = (window.viewport_size().height / px(t::ROW_H)).ceil() as usize;
        let body: AnyElement = if let Some(e) = &self.error {
            div().flex_1().flex().items_center().justify_center().text_color(t::text_secondary()).child(e.clone()).into_any_element()
        } else {
            uniform_list("rows", self.rows.len().max(fill), cx.processor(Self::render_rows)).flex_1().track_scroll(&self.scroll).into_any_element()
        };
        div().flex_1().min_h_0().flex().flex_col().child(self.column_header(cx)).child(body)
    }

    fn path_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // While searching it shows where the selected result is.
        let target = match &self.search {
            Some(_) => self.selected.and_then(|i| self.rows.get(i)).map(|r| r.entry.path.clone()),
            None => Some(self.current.clone()),
        };
        let mut crumbs: Vec<PathBuf> = target.iter().flat_map(|p| p.ancestors()).map(Path::to_path_buf).collect();
        crumbs.reverse();
        let n = crumbs.len();
        let mut bar = div()
            .id("path-bar")
            .h(px(t::PATH_BAR_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(3.))
            .px(px(14.))
            .bg(t::bar_bg())
            .border_t_1()
            .border_color(t::separator())
            .text_size(px(t::SMALL_SIZE))
            .text_color(t::text_secondary())
            .overflow_x_hidden();
        for (i, p) in crumbs.into_iter().enumerate() {
            let is_root = p.parent().is_none();
            let name = files::display_name(&p);
            bar = bar
                .child(
                    div()
                        .id(("crumb", i))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(4.))
                        .child(img(if is_root { "icons/disk.svg" } else if p.is_dir() { "icons/folder.svg" } else { "icons/file.svg" }).size(px(14.)))
                        .child(name)
                        .on_click(cx.listener(move |this, _, _, cx| this.navigate(p.clone(), cx))),
                )
                .when(i + 1 < n, |d| d.child(sym("chevron.right@semibold", 7., t::text_tertiary())));
        }
        bar
    }

    /// "24 items, 58.9 GB available".
    fn status_bar(&self) -> impl IntoElement {
        let n = self.rows.iter().filter(|r| r.depth == 0).count();
        let items = match (n, self.selected) {
            (n, Some(_)) => format!("1 of {n} selected"),
            (1, None) => "1 item".to_string(),
            (n, None) => format!("{n} items"),
        };
        if let Some(s) = &self.search {
            let text = match (&s.error, s.took) {
                (Some(e), _) => e.clone(),
                (None, Some(took)) => {
                    let n = self.rows.len();
                    let plus = if s.capped { "+" } else { "" };
                    let sel = if self.selected.is_some() { format!("1 of {n}{plus} selected") } else { format!("{n}{plus} items") };
                    format!("{sel} \u{00b7} {:.1} ms", took.as_secs_f64() * 1e3)
                }
                (None, None) => "Searching\u{2026}".into(),
            };
            return div()
                .h(px(t::STATUS_BAR_H))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .bg(t::bar_bg())
                .text_size(px(t::SMALL_SIZE))
                .text_color(t::text_secondary())
                .child(text);
        }
        let free = files::free_space(&self.current).map(|b| format!(", {} available", files::format_bytes(b))).unwrap_or_default();
        div()
            .h(px(t::STATUS_BAR_H))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .bg(t::bar_bg())
            .text_size(px(t::SMALL_SIZE))
            .text_color(t::text_secondary())
            .child(format!("{items}{free}"))
    }

    fn inspector(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // Search results carry only what the index knows: stat for the rest.
        let e = self
            .selected
            .and_then(|i| self.rows.get(i))
            .map(|r| if r.entry.created.is_none() { Entry::from_path(r.entry.path.clone()).unwrap_or_else(|| r.entry.clone()) } else { r.entry.clone() })
            .or_else(|| Entry::from_path(self.current.clone()));
        let mut col = div().flex_1().min_h_0().flex().flex_col().px(px(16.)).pb(px(10.)).text_size(px(t::TEXT_SIZE));
        if let Some(e) = e {
            let info = |label: &'static str, value: String| {
                div()
                    .flex()
                    .justify_between()
                    .gap(px(8.))
                    .py(px(5.))
                    .border_b_1()
                    .border_color(t::separator())
                    .text_size(px(11.5))
                    .child(div().flex_none().text_color(t::text_secondary()).child(label))
                    .child(div().min_w_0().text_color(t::text()).truncate().child(value))
            };
            let size = if e.is_dir { String::new() } else { format!(" - {}", files::format_size(&e)) };
            let thumb = !e.is_dir && icons::has_thumbnail(&e.path);
            let src = self.icon_source(&e, 512, cx);
            col = col
                .child(
                    div().h(px(220.)).flex_none().flex().items_center().justify_center().child(if thumb {
                        div()
                            .p(px(6.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(t::separator())
                            .child(img(src).max_w(px(230.)).max_h(px(196.)))
                            .into_any_element()
                    } else {
                        img(src).size(px(160.)).into_any_element()
                    }),
                )
                .child(div().mt(px(14.)).text_size(px(t::TITLE_SIZE)).font_weight(gpui::FontWeight::SEMIBOLD).child(e.name.clone()))
                .child(div().text_color(t::text_secondary()).child(format!("{}{size}", e.kind)))
                .child(
                    div()
                        .mt(px(14.))
                        .mb(px(2.))
                        .flex()
                        .justify_between()
                        .child(div().font_weight(gpui::FontWeight::BOLD).child("Information"))
                        .child(div().text_size(px(12.)).text_color(t::accent()).child("Show Less")),
                )
                .child(info("Created", files::format_date_long(e.created)))
                .child(info("Modified", files::format_date_long(e.modified)))
                .child(info("Last opened", files::format_date_long(e.accessed)))
                .child(div().mt(px(14.)).mb(px(6.)).font_weight(gpui::FontWeight::BOLD).child("Tags"))
                .child(div().px(px(6.)).py(px(5.)).rounded(px(5.)).bg(t::bar_bg()).text_color(t::text_tertiary()).child("Add Tags…"));
        }
        col.child(div().flex_1()).child(
            div().flex().justify_center().child(
                div()
                    .id("more")
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(3.))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(8.))
                    .hover(|s| s.bg(t::glass_pressed()))
                    .text_size(px(t::SMALL_SIZE))
                    .text_color(t::text_secondary())
                    .child(sym("ellipsis.circle", 15., t::text_secondary()))
                    .child("More…"),
            ),
        )
    }
}

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // FINDER_APPEARANCE=light|dark overrides the system's, for testing.
        let dark = match std::env::var("FINDER_APPEARANCE").as_deref() {
            Ok("dark") => true,
            Ok("light") => false,
            _ => matches!(window.appearance(), gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark),
        };
        t::set_dark(dark);
        div()
            .id("browser")
            .key_context("Browser")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::expand_selected))
            .on_action(cx.listener(Self::collapse_selected))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_forward))
            .on_action(cx.listener(Self::go_up))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::escape))
            .on_mouse_down(MouseButton::Navigate(gpui::NavigationDirection::Back), cx.listener(|this, _, w, cx| this.go_back(&GoBack, w, cx)))
            .on_mouse_down(MouseButton::Navigate(gpui::NavigationDirection::Forward), cx.listener(|this, _, w, cx| this.go_forward(&GoForward, w, cx)))
            .size_full()
            .flex()
            .bg(t::window_bg())
            .font_family(self.font.clone())
            .text_size(px(t::TEXT_SIZE))
            .text_color(t::text())
            .child(self.sidebar(window, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .bg(t::content_bg())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(self.toolbar(cx))
                            .when(self.search.is_some(), |d| d.child(self.scope_bar(cx)))
                            .child(self.list(window, cx))
                            .child(self.path_bar(cx))
                            .child(self.status_bar()),
                    )
                    .child(
                        div()
                            .w(px(t::INSPECTOR_W))
                            .h_full()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .bg(t::inspector_bg())
                            .border_l_1()
                            .border_color(t::separator())
                            .child(self.search_bar(window, cx))
                            .child(self.inspector(cx)),
                    ),
            )
    }
}
