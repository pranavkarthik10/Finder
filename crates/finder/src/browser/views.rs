//! The icon, column and gallery views (the list view is in browser.rs).

use super::*;
use std::ops::Range;

const ICON: f32 = 64.;
const CELL_W: f32 = 104.;
const CELL_H: f32 = 116.;
const GRID_PAD: f32 = 18.;
const GRID_GAP: f32 = 12.;
const COLUMN_W: f32 = 235.;
const PREVIEW_W: f32 = 260.;
const STRIP_H: f32 = 118.;
/// One filmstrip tile, gap included.
const TILE: f32 = 86.;

/// One column of the column view: a folder's entries, by name.
pub(super) struct Column {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    pub sel: Option<usize>,
    pub scroll: UniformListScrollHandle,
}

impl Column {
    pub fn open(path: PathBuf) -> Column {
        let mut entries = files::list(&path).unwrap_or_default();
        files::sort(&mut entries, SortKey::Name, true);
        Column { path, entries, sel: None, scroll: UniformListScrollHandle::new() }
    }
}

/// An icon's label: names too long for two lines lose their middle, as in
/// Finder ("anki_2024_10_0…762.png"), so the extension stays visible.
fn icon_label(name: &str) -> String {
    const MAX: usize = 24;
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= MAX {
        return name.to_string();
    }
    let tail = 10;
    let head: String = chars[..MAX - tail - 1].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{head}\u{2026}{end}")
}

impl Browser {
    /// Width left for the view between the sidebar and the inspector.
    fn content_width(&self, window: &Window) -> f32 {
        window.viewport_size().width / px(1.) - t::SIDEBAR_W - t::INSPECTOR_W
    }

    /// Move the selection by `delta` (rows in the list, cells in the grid
    /// and filmstrip).
    pub(super) fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as isize - 1;
        let ix = match self.selected {
            Some(i) => (i as isize + delta).clamp(0, last),
            None if delta > 0 => 0,
            None => last,
        };
        self.select(ix as usize, cx);
    }

    /// Keep the selection in view, however the current view lays it out.
    pub(super) fn reveal(&mut self, ix: usize) {
        match self.shown_view() {
            ViewMode::Icons => self.scroll.scroll_to_item(ix / self.grid_cols.max(1), ScrollStrategy::Nearest),
            ViewMode::Gallery => {
                let x = ix as f32 * TILE - (self.strip_width - TILE) / 2.;
                self.strip_scroll.set_offset(gpui::point(px(-x.max(0.)), px(0.)));
            }
            _ => self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest),
        }
    }

    /// The view on screen: search results always show as a list.
    pub(super) fn shown_view(&self) -> ViewMode {
        if self.search.is_some() { ViewMode::List } else { self.view }
    }

    pub(super) fn set_view(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        self.view = mode;
        // Only the list shows folders opened in place.
        if mode != ViewMode::List && self.search.is_none() && self.rows.iter().any(|r| r.depth > 0) {
            let sel = self.selected_entry().map(|e| e.path);
            let cur = self.current.clone();
            self.load(cur);
            self.selected = sel.and_then(|p| self.rows.iter().position(|r| r.entry.path == p));
        }
        if mode == ViewMode::Gallery && self.selected.is_none() && !self.rows.is_empty() {
            self.selected = Some(0);
        }
        if let Some(i) = self.selected {
            self.reveal(i);
        }
        cx.notify();
    }

    /// What the inspector, path bar and status bar describe.
    pub(super) fn selected_entry(&self) -> Option<Entry> {
        if self.shown_view() == ViewMode::Columns {
            return self.columns.iter().rev().find_map(|c| c.sel.map(|i| c.entries[i].clone()));
        }
        self.selected.and_then(|i| self.rows.get(i)).map(|r| r.entry.clone())
    }

    pub(super) fn main_view(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.error.is_some() || self.shown_view() == ViewMode::List {
            return self.list(window, cx).into_any_element();
        }
        match self.view {
            ViewMode::Icons => self.icon_grid(window, cx),
            ViewMode::Columns => self.column_view(window, cx),
            _ => self.gallery(window, cx),
        }
    }

    // Icons

    fn icon_grid(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let w = self.content_width(window);
        let cols = (((w - 2. * GRID_PAD + GRID_GAP) / (CELL_W + GRID_GAP)).floor() as usize).max(1);
        self.grid_cols = cols;
        let rows = self.rows.len().div_ceil(cols);
        uniform_list("grid", rows, cx.processor(move |this, range: Range<usize>, window, cx| range.map(|r| this.grid_row(r, cols, window, cx)).collect::<Vec<_>>()))
            .flex_1()
            .track_scroll(&self.scroll)
            .into_any_element()
    }

    fn grid_row(&mut self, r: usize, cols: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let focused = self.focus.is_focused(window);
        let mut row = div().w_full().h(px(CELL_H + GRID_GAP)).flex().gap(px(GRID_GAP)).px(px(GRID_PAD)).pt(px(GRID_GAP));
        for ix in r * cols..((r + 1) * cols).min(self.rows.len()) {
            let e = self.rows[ix].entry.clone();
            let src = self.icon_source(&e, 128, cx);
            let sel = self.is_selected(ix);
            let pictured = !e.is_dir && icons::has_thumbnail(&e.path);
            let rename = self.rename_box(&e.path);
            row = row.child(
                div()
                    .id(("cell", ix))
                    .w(px(CELL_W))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .size(px(ICON + 10.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .when(sel, |d| d.bg(t::icon_selected()))
                            .child(img(src).size(px(ICON)).object_fit(gpui::ObjectFit::Contain)),
                    )
                    .children(rename)
                    .when(self.renaming.as_ref().is_none_or(|r| r.path != e.path), |d| d.child(
                        div()
                            .max_w(px(CELL_W))
                            .px(px(5.))
                            .rounded(px(4.))
                            .text_size(px(12.))
                            .line_height(px(15.))
                            .text_center()
                            .line_clamp(2)
                            .when(sel, |d| d.bg(if focused { t::selection() } else { t::selection_inactive() }))
                            .when(sel && focused, |d| d.text_color(gpui::white()))
                            .child(icon_label(&e.name)),
                    ))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        if ev.click_count() >= 2 { this.open_row(ix, cx) } else { this.click_select(ix, &ev.modifiers(), cx) }
                    })),
            );
        }
        row.into_any_element()
    }

    /// FINDER_SELECT=name selects an entry at startup (testing).
    pub(super) fn select_by_name(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.shown_view() == ViewMode::Columns {
            self.ensure_columns();
            if let Some(ix) = self.columns[0].entries.iter().position(|e| *e.name == *name) {
                self.col_select(0, ix, cx);
            }
        } else if let Some(ix) = self.rows.iter().position(|r| *r.entry.name == *name) {
            self.select(ix, cx);
        }
    }

    // Columns

    /// Columns start at the current folder; navigating elsewhere resets them.
    fn ensure_columns(&mut self) {
        if self.columns.first().map(|c| &c.path) != Some(&self.current) {
            self.columns = vec![Column::open(self.current.clone())];
            self.col_focus = 0;
        }
    }

    fn column_view(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        self.ensure_columns();
        let mut strip = div().id("columns").flex_1().min_h_0().flex().overflow_x_scroll().track_scroll(&self.col_scroll);
        for k in 0..self.columns.len() {
            let n = self.columns[k].entries.len();
            let list = uniform_list(("column", k), n, cx.processor(move |this, range: Range<usize>, window, cx| this.column_rows(k, range, window, cx)))
                .size_full()
                .track_scroll(&self.columns[k].scroll);
            strip = strip.child(div().w(px(COLUMN_W)).h_full().flex_none().py(px(4.)).border_r_1().border_color(t::separator()).child(list));
        }
        if let Some(e) = self.columns.last().and_then(|c| c.sel.map(|i| c.entries[i].clone())).filter(|e| !e.is_dir) {
            strip = strip.child(self.column_preview(e, window, cx));
        }
        strip.into_any_element()
    }

    fn column_rows(&mut self, k: usize, range: Range<usize>, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let active = k == self.col_focus && self.focus.is_focused(window);
        range
            .map(|ix| {
                let e = self.columns[k].entries[ix].clone();
                let src = self.icon_source(&e, 32, cx);
                let sel = self.columns[k].sel == Some(ix);
                let (fg, fg2): (gpui::Hsla, gpui::Hsla) =
                    if sel && active { (gpui::white(), gpui::white().opacity(0.85)) } else { (t::text().into(), t::text_secondary().into()) };
                let row = div()
                    .id(("crow", ix))
                    .flex_1()
                    .min_w_0()
                    .h(px(t::ROW_H))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(5.))
                    .when(sel, |d| d.bg(if active { t::selection() } else { t::selection_inactive() }))
                    .text_color(fg)
                    .child(img(src).size(px(t::ROW_ICON)).flex_none())
                    .child(div().flex_1().min_w_0().truncate().child(e.name.clone()))
                    .when(e.is_dir, |d| d.child(sym("chevron.right@semibold", 8., fg2)))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.col_select(k, ix, cx);
                        if ev.click_count() >= 2 {
                            this.col_open(cx);
                        }
                    }));
                div().w_full().px(px(6.)).flex().child(row).into_any_element()
            })
            .collect()
    }

    fn col_select(&mut self, k: usize, ix: usize, cx: &mut Context<Self>) {
        self.columns.truncate(k + 1);
        let c = &mut self.columns[k];
        c.sel = Some(ix);
        c.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        self.col_focus = k;
        let e = &c.entries[ix];
        if e.is_dir {
            let p = e.path.clone();
            self.columns.push(Column::open(p));
        }
        // Keep the newest column (or the preview) in view.
        self.col_scroll.scroll_to_item(self.columns.len());
        cx.notify();
    }

    /// Up and down within the focused column.
    pub(super) fn col_step(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.ensure_columns();
        let k = self.col_focus.min(self.columns.len() - 1);
        let n = self.columns[k].entries.len() as isize;
        if n == 0 {
            return;
        }
        let ix = match self.columns[k].sel {
            Some(i) => (i as isize + delta).clamp(0, n - 1),
            None => 0,
        };
        self.col_select(k, ix as usize, cx);
    }

    /// Right: into the selected folder's column.
    pub(super) fn col_right(&mut self, cx: &mut Context<Self>) {
        let k = self.col_focus;
        let into_dir = self.columns.get(k).and_then(|c| c.sel.map(|i| c.entries[i].is_dir)).unwrap_or(false);
        if into_dir && self.columns.get(k + 1).is_some_and(|c| !c.entries.is_empty()) {
            self.col_select(k + 1, 0, cx);
        }
    }

    /// Left: back to the parent column, keeping its selection.
    pub(super) fn col_left(&mut self, cx: &mut Context<Self>) {
        let k = self.col_focus;
        if k == 0 {
            return;
        }
        self.columns[k].sel = None;
        self.columns.truncate(k + 1);
        self.col_focus = k - 1;
        cx.notify();
    }

    /// Open what's selected in the focused column.
    pub(super) fn col_open(&mut self, cx: &mut Context<Self>) {
        let Some(e) = self.columns.get(self.col_focus).and_then(|c| c.sel.map(|i| c.entries[i].clone())) else { return };
        if e.is_dir {
            self.navigate(e.path, cx);
        } else {
            let _ = std::process::Command::new("explorer").arg(&e.path).spawn();
        }
    }

    fn column_preview(&mut self, e: Entry, _window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let pictured = icons::has_thumbnail(&e.path);
        let src = self.icon_source(&e, 512, cx);
        div()
            .w(px(PREVIEW_W))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .p(px(16.))
            .gap(px(4.))
            .child(div().h(px(200.)).w_full().flex().items_center().justify_center().child(img(src).when(pictured, |i| i.w(px(220.)).h(px(190.))).when(!pictured, |i| i.size(px(128.))).object_fit(gpui::ObjectFit::Contain)))
            .child(div().mt(px(8.)).text_size(px(t::TEXT_SIZE)).font_weight(gpui::FontWeight::SEMIBOLD).text_center().child(e.name.clone()))
            .child(div().text_size(px(t::SMALL_SIZE)).text_color(t::text_secondary()).child(format!("{} - {}", e.kind, files::format_size(&e))))
            .into_any_element()
    }

    // Gallery

    fn gallery(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.selected.is_none() && !self.rows.is_empty() {
            self.selected = Some(0);
        }
        let w = self.content_width(window);
        self.strip_width = w;
        let preview = match self.selected.and_then(|i| self.rows.get(i)).map(|r| r.entry.clone()) {
            Some(e) => {
                let pictured = !e.is_dir && icons::has_thumbnail(&e.path);
                let src = self.icon_source(&e, if pictured { 1024 } else { 256 }, cx);
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(24.))
                    .child(img(src).when(pictured, |i| i.size_full()).when(!pictured, |i| i.size(px(256.))).object_fit(gpui::ObjectFit::Contain))
                    .into_any_element()
            }
            None => div().flex_1().into_any_element(),
        };
        // Only the tiles in view are built; spacers stand in for the rest.
        let n = self.rows.len();
        let off = -(self.strip_scroll.offset().x / px(1.));
        let first = ((off / TILE).floor().max(0.) as usize).min(n);
        let last = (first + (w / TILE).ceil() as usize + 2).min(n);
        let focused = self.focus.is_focused(window);
        let mut strip = div()
            .id("filmstrip")
            .h(px(STRIP_H))
            .flex_none()
            .flex()
            .items_center()
            .px(px(12.))
            .overflow_x_scroll()
            .track_scroll(&self.strip_scroll)
            .border_t_1()
            .border_color(t::separator())
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .child(div().w(px(first as f32 * TILE)).h(px(1.)).flex_none());
        for i in first..last {
            let e = self.rows[i].entry.clone();
            let src = self.icon_source(&e, 128, cx);
            let sel = self.selected == Some(i);
            let pictured = !e.is_dir && icons::has_thumbnail(&e.path);
            strip = strip.child(
                div()
                    .id(("tile", i))
                    .w(px(TILE))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .size(px(74.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .when(sel, |d| d.bg(if focused { t::selection() } else { t::selection_inactive() }))
                            .child(img(src).size(px(64.)).object_fit(gpui::ObjectFit::Contain)),
                    )
                    .child(div().w(px(TILE - 6.)).text_size(px(t::SMALL_SIZE)).text_center().truncate().child(e.name.clone()))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        if ev.click_count() >= 2 { this.open_row(i, cx) } else { this.select(i, cx) }
                    })),
            );
        }
        strip = strip.child(div().w(px((n - last) as f32 * TILE)).h(px(1.)).flex_none());
        div().flex_1().min_h_0().flex().flex_col().child(preview).child(strip).into_any_element()
    }
}
