//! One Finder window: sidebar, toolbar, list view, inspector, path bar.

use crate::files::{self, Entry, SortKey};
use crate::icons;
use crate::theme as t;
use gpui::{
    AnyElement, App, ClickEvent, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, MouseButton, ParentElement, Render,
    ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, UniformListScrollHandle, Window, WindowControlArea, actions, div, img,
    prelude::*, px, svg, uniform_list,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

actions!(finder, [SelectNext, SelectPrev, Expand, Collapse, Open, GoBack, GoForward, GoUp]);

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
        Some(Place { name: "Applications".into(), icon: "icons/applications.svg", path: Some(PathBuf::from(r"C:\Program Files")) }),
        known("Desktop", "icons/desktop.svg", "Desktop"),
        known("Documents", "icons/document.svg", "Documents"),
        known("Downloads", "icons/download.svg", "Downloads"),
        known("Developer", "icons/hammer.svg", "code"),
    ];
    let user = h.file_name().map_or("Home".into(), |n| n.to_string_lossy().into_owned());
    let mut locations = vec![known("OneDrive", "icons/cloud.svg", "OneDrive"), Some(Place { name: user.into(), icon: "icons/house.svg", path: Some(h.clone()) })];
    for d in fsearch::walk::drives() {
        let root = PathBuf::from(format!("{}:\\", d as char));
        locations.push(Some(Place { name: files::display_name(&root).into(), icon: "icons/drive.svg", path: Some(root) }));
    }
    locations.push(Some(Place { name: "Network".into(), icon: "icons/globe.svg", path: None }));
    locations.push(Some(Place { name: "Recycle Bin".into(), icon: "icons/trash.svg", path: None }));
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
        };
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
        if !r.entry.is_dir || r.expanded {
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

    // Keyboard

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = Some(ix);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let ix = self.selected.map_or(0, |i| (i + 1).min(self.rows.len() - 1));
        self.select(ix, cx);
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
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

fn icon(path: &'static str, size: f32, color: impl Into<gpui::Hsla>) -> impl IntoElement {
    svg().path(path).size(px(size)).flex_none().text_color(color.into())
}

/// A Liquid Glass capsule holding toolbar buttons.
fn capsule(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(t::CAPSULE_H))
        .flex()
        .items_center()
        .px(px(3.))
        .gap(px(1.))
        .rounded_full()
        .bg(t::glass())
        .border_1()
        .border_color(t::glass_border())
        .shadow(vec![gpui::BoxShadow {
            color: t::glass_shadow(),
            offset: gpui::point(px(0.), px(1.)),
            blur_radius: px(4.),
            spread_radius: px(0.),
            inset: false,
        }])
}

/// One icon button inside a capsule.
fn tool_button(id: impl Into<gpui::ElementId>, path: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(t::CAPSULE_H - 8.))
        .min_w(px(36.))
        .px(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .when(on, |d| d.bg(t::glass_pressed()))
        .hover(|s| s.bg(t::glass_pressed()))
        .child(icon(path, t::TOOL_ICON, t::text()))
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
                        .path(glyph)
                        .size(px(t::LIGHT_D - 4.))
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
            .child(light("close", t::light_close(), "icons/tl-close.svg", WindowControlArea::Close))
            .child(light("min", t::light_min(), "icons/tl-min.svg", WindowControlArea::Min))
            .child(light("max", t::light_max(), "icons/tl-max.svg", WindowControlArea::Max))
    }

    fn sidebar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div()
            .id("sidebar-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_y_scroll()
            .px(px(t::SIDEBAR_INSET))
            .pb(px(12.));
        for (si, s) in self.sections.iter().enumerate() {
            col = col.child(
                div()
                    .h(px(26.))
                    .mt(px(if si == 0 { 0. } else { 10. }))
                    .px(px(8.))
                    .flex()
                    .items_end()
                    .pb(px(5.))
                    .text_size(px(t::SMALL_SIZE))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(t::sidebar_header())
                    .child(s.title),
            );
            for (pi, p) in s.places.iter().enumerate() {
                let on = p.path.as_deref() == Some(self.current.as_path());
                let path = p.path.clone();
                col = col.child(
                    div()
                        .id(("place", si * 100 + pi))
                        .h(px(t::SIDEBAR_ROW_H))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .rounded(px(8.))
                        .when(on, |d| d.bg(t::sidebar_selected()))
                        .child(icon(p.icon, t::SIDEBAR_ICON, t::sidebar_icon()))
                        .child(div().text_size(px(t::TEXT_SIZE)).when(on, |d| d.font_weight(gpui::FontWeight::SEMIBOLD)).truncate().child(p.name.clone()))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(p) = &path {
                                this.navigate(p.clone(), cx);
                            }
                        })),
                );
            }
        }
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
        let title = files::display_name(&self.current);
        let view_btn = |id: &'static str, path: &'static str, mode: ViewMode, cx: &mut Context<Self>| {
            tool_button(id, path, self.view == mode).on_click(cx.listener(move |this, _, _, cx| {
                this.view = mode;
                cx.notify();
            }))
        };
        div()
            .h(px(t::TOOLBAR_H))
            .flex_none()
            .flex()
            .items_center()
            .px(px(10.))
            .gap(px(t::CAPSULE_GAP))
            .window_control_area(WindowControlArea::Drag)
            .child(
                capsule("nav")
                    .child(
                        tool_button("back", "icons/chevron-left.svg", false)
                            .when(self.back.is_empty(), |d| d.opacity(0.35))
                            .on_click(cx.listener(|this, _, w, cx| this.go_back(&GoBack, w, cx))),
                    )
                    .child(div().w(px(1.)).h(px(18.)).bg(t::separator()))
                    .child(
                        tool_button("forward", "icons/chevron-right.svg", false)
                            .when(self.forward.is_empty(), |d| d.opacity(0.35))
                            .on_click(cx.listener(|this, _, w, cx| this.go_forward(&GoForward, w, cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px(px(6.))
                    .text_size(px(t::TITLE_SIZE))
                    .font_weight(gpui::FontWeight::BOLD)
                    .truncate()
                    .child(title),
            )
            .child(capsule("airdrop-cap").child(tool_button("airdrop", "icons/airdrop.svg", false)))
            .child(
                capsule("views")
                    .child(view_btn("v-icons", "icons/grid.svg", ViewMode::Icons, cx))
                    .child(view_btn("v-list", "icons/list.svg", ViewMode::List, cx))
                    .child(view_btn("v-columns", "icons/columns.svg", ViewMode::Columns, cx))
                    .child(view_btn("v-gallery", "icons/gallery.svg", ViewMode::Gallery, cx)),
            )
            .child(
                capsule("group-cap").child(
                    tool_button("group", "icons/group.svg", false).gap(px(2.)).child(icon("icons/chevron-down.svg", 9., t::text())),
                ),
            )
            .child(
                capsule("actions")
                    .child(tool_button("share", "icons/share.svg", false))
                    .child(tool_button("tags", "icons/tag.svg", false))
                    .child(tool_button("more", "icons/ellipsis.svg", false)),
            )
            .child(
                capsule("search")
                    .w(px(190.))
                    .px(px(12.))
                    .gap(px(6.))
                    .child(icon("icons/search.svg", 15., t::text_secondary()))
                    .child(div().text_size(px(t::TEXT_SIZE)).text_color(t::text_tertiary()).child("Search")),
            )
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
                .border_l_1()
                .border_color(t::separator())
                .when(on, |d| d.font_weight(gpui::FontWeight::BOLD).text_color(t::text()))
                .child(div().flex_1().truncate().child(label))
                .when(on, |d| d.child(icon(if self.sort.1 { "icons/chevron-up.svg" } else { "icons/chevron-down.svg" }, 9., t::text_secondary())))
                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(key, cx)))
        };
        div()
            .h(px(t::HEADER_H))
            .flex_none()
            .flex()
            .items_center()
            .mx(px(t::ROW_INSET))
            .border_b_1()
            .border_color(t::separator())
            .text_size(px(t::HEADER_SIZE))
            .text_color(t::text_secondary())
            .child(col("h-name", "Name", SortKey::Name, cx).flex_1().border_l_0().pl(px(30.)))
            .child(col("h-date", "Date Modified", SortKey::Modified, cx).w(px(170.)))
            .child(col("h-size", "Size", SortKey::Size, cx).w(px(90.)))
            .child(col("h-kind", "Kind", SortKey::Kind, cx).w(px(150.)))
    }

    fn render_rows(&mut self, range: std::ops::Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let focused = self.focus.is_focused(window);
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
                let (fg, fg2) = if sel && focused { (gpui::white().into(), gpui::rgba(0xFFFFFFCC)) } else { (t::text(), t::text_secondary()) };
                let indent = r.depth as f32 * t::INDENT;
                div().w_full().px(px(t::ROW_INSET)).flex().child(
                    div()
                    .id(("row", ix))
                    .h(px(t::ROW_H))
                    .flex_1()
                    .min_w_0()
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
                                    .w(px(16.))
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(r.entry.is_dir, |d| {
                                        d.child(
                                            svg()
                                                .path("icons/disclosure.svg")
                                                .size(px(10.))
                                                .text_color(if sel && focused { gpui::rgba(0xFFFFFFCC) } else { t::text_secondary() })
                                                .when(r.expanded, |s| s.with_transformation(gpui::Transformation::rotate(gpui::radians(std::f32::consts::FRAC_PI_2)))),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.toggle(ix, cx);
                                        }))
                                    }),
                            )
                            .child(img(src).size(px(t::ROW_ICON)).flex_none().mx(px(6.)))
                            .child(div().flex_1().min_w_0().truncate().child(r.entry.name.clone())),
                    )
                    .child(div().w(px(170.)).px(px(9.)).truncate().text_color(fg2).child(files::format_date(r.entry.modified)))
                    .child(div().w(px(90.)).px(px(9.)).flex().justify_end().text_color(fg2).child(files::format_size(&r.entry)))
                    .child(div().w(px(150.)).px(px(9.)).truncate().text_color(fg2).child(r.entry.kind.clone()))
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        if e.click_count() >= 2 {
                            this.open_row(ix, cx);
                        } else {
                            this.select(ix, cx);
                        }
                    }))
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
        let mut crumbs: Vec<PathBuf> = self.current.ancestors().map(Path::to_path_buf).collect();
        crumbs.reverse();
        let n = crumbs.len();
        let mut bar = div()
            .id("path-bar")
            .h(px(t::PATH_BAR_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(12.))
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
                        .child(if is_root { img("icons/disk.svg").size(px(13.)).into_any_element() } else { img("icons/folder.svg").size(px(13.)).into_any_element() })
                        .child(name)
                        .on_click(cx.listener(move |this, _, _, cx| this.navigate(p.clone(), cx))),
                )
                .when(i + 1 < n, |d| d.child(icon("icons/chevron-right.svg", 8., t::text_tertiary())));
        }
        bar
    }

    /// "24 items, 58.9 GB available".
    fn status_bar(&self) -> impl IntoElement {
        let n = self.rows.iter().filter(|r| r.depth == 0).count();
        let items = if n == 1 { "1 item".to_string() } else { format!("{n} items") };
        let free = files::free_space(&self.current).map(|b| format!(", {} available", files::format_bytes(b))).unwrap_or_default();
        div()
            .h(px(t::STATUS_BAR_H))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .border_t_1()
            .border_color(t::separator())
            .text_size(px(t::SMALL_SIZE))
            .text_color(t::text_secondary())
            .child(format!("{items}{free}"))
    }

    fn inspector(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let e = self.selected.and_then(|i| self.rows.get(i)).map(|r| r.entry.clone()).or_else(|| Entry::from_path(self.current.clone()));
        let mut col = div()
            .w(px(t::INSPECTOR_W))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .px(px(16.))
            .pt(px(16.))
            .pb(px(10.))
            .bg(t::inspector_bg())
            .border_l_1()
            .border_color(t::separator())
            .text_size(px(t::SMALL_SIZE));
        if let Some(e) = e {
            let info = |label: &'static str, value: String| {
                div()
                    .flex()
                    .justify_between()
                    .gap(px(8.))
                    .py(px(3.))
                    .child(div().text_color(t::text_secondary()).child(label))
                    .child(div().text_color(t::text()).truncate().child(value))
            };
            let size = if e.is_dir { String::new() } else { format!(" - {}", files::format_size(&e)) };
            col = col
                .child(div().h(px(190.)).flex().items_center().justify_center().child(img(self.icon_source(&e, 256, cx)).max_w(px(190.)).max_h(px(190.)).when(e.is_dir || !icons::has_thumbnail(&e.path), |i| i.size(px(170.)))))
                .child(div().mt(px(14.)).text_size(px(t::TEXT_SIZE)).font_weight(gpui::FontWeight::BOLD).truncate().child(e.name.clone()))
                .child(div().text_color(t::text_secondary()).child(format!("{}{size}", e.kind)))
                .child(div().mt(px(14.)).mb(px(4.)).font_weight(gpui::FontWeight::BOLD).child("Information"))
                .child(info("Created", files::format_date_long(e.created)))
                .child(info("Modified", files::format_date_long(e.modified)))
                .child(info("Last opened", files::format_date_long(e.accessed)))
                .child(div().mt(px(14.)).mb(px(4.)).font_weight(gpui::FontWeight::BOLD).child("Tags"))
                .child(div().text_color(t::text_tertiary()).child("Add Tags…"));
        }
        col.child(div().flex_1()).child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(3.))
                .text_color(t::text_secondary())
                .child(icon("icons/ellipsis-circle.svg", 18., t::text_secondary()))
                .child("More…"),
        )
    }
}

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .on_mouse_down(MouseButton::Navigate(gpui::NavigationDirection::Back), cx.listener(|this, _, w, cx| this.go_back(&GoBack, w, cx)))
            .on_mouse_down(MouseButton::Navigate(gpui::NavigationDirection::Forward), cx.listener(|this, _, w, cx| this.go_forward(&GoForward, w, cx)))
            .size_full()
            .flex()
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
                    .flex_col()
                    .bg(t::content_bg())
                    .child(self.toolbar(cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .child(div().flex_1().min_w_0().flex().flex_col().child(self.list(window, cx)).child(self.path_bar(cx)).child(self.status_bar()))
                            .child(self.inspector(cx)),
                    ),
            )
    }
}
