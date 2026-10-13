//! One Finder window: sidebar, toolbar, list view, inspector, path bar.

use crate::files::{self, Entry, SortKey};
use crate::assets::symbol;
use crate::icons;
use crate::clipboard;
use crate::ops;
use crate::text_input::{self, TextInput};
use crate::theme as t;
use crate::tags::{Tag, Tags};

mod dnd;
mod menu;
mod quicklook;
mod views;
use gpui::{
    AnyElement, App, ClickEvent, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, MouseButton, ParentElement, Render,
    ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, UniformListScrollHandle, Window, WindowControlArea, actions, div, img,
    prelude::*, px, svg, uniform_list,
};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

actions!(
    finder,
    [SelectNext, SelectPrev, Expand, Collapse, Open, GoBack, GoForward, GoUp, FocusSearch, EndSearch, SelectAll, Rename, NewFolder, Duplicate, Trash, CopyFiles, CutFiles, PasteFiles, CopyPath, Reveal, ShowIcons, ShowList, ShowColumns, ShowGallery, ContextMenu, UndoOp, QuickLook, ShowActionMenu, ShowSortMenu, ShowTagMenu]
);

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
    /// Filters typed into the field, shown as chips before the text:
    /// (key, value) as in `ext:pdf`.
    chips: Vec<(String, String)>,
    /// Column view: the current folder, then each selected folder.
    columns: Vec<views::Column>,
    /// The column the arrow keys move in.
    col_focus: usize,
    col_scroll: gpui::ScrollHandle,
    /// Gallery filmstrip.
    strip_scroll: gpui::ScrollHandle,
    strip_width: f32,
    /// Icon view: cells per row (last laid out).
    grid_cols: usize,
    /// Rows selected besides `selected` (the cursor), by Ctrl/Shift-click.
    marked: BTreeSet<usize>,
    /// Where a Shift-click range starts.
    anchor: Option<usize>,
    /// The name being edited in place.
    renaming: Option<Renaming>,
    /// The last operation's failure, shown in the status bar.
    notice: Option<String>,
    menu: Option<menu::Menu>,
    /// The inspector shows only the modified date (its "Show Less").
    info_collapsed: bool,
    tags: Tags,
    /// FINDER_SCROLL_TO's row, and frames waited so far.
    pending_scroll: Option<(usize, u8)>,
    /// What Ctrl+Z reverses, newest last.
    undo: Vec<Undo>,
    /// The Quick Look panel is up.
    quick_look: bool,
    /// Quick Look's text of the item it shows.
    peek: Option<quicklook::Peek>,
    /// Quick Look's Windows preview handler, for PDFs, video, Office...
    preview: Option<crate::preview_host::PreviewHost>,
    /// Extensions a preview handler takes.
    hostable: HashMap<String, bool>,
    /// Our window (for hosting preview handlers) and its scale factor.
    pub hwnd: Option<isize>,
    scale: f32,
}

/// How to reverse a file operation.
enum Undo {
    Rename { from: PathBuf, to: PathBuf },
    /// Made by us: back to the Recycle Bin.
    Created(Vec<PathBuf>),
    /// (where it is now, where it was).
    Moved(Vec<(PathBuf, PathBuf)>),
    /// Back out of the Recycle Bin.
    Trashed(Vec<PathBuf>),
}

struct Renaming {
    path: PathBuf,
    input: gpui::Entity<TextInput>,
    _subs: Vec<gpui::Subscription>,
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
    /// Showing everything with this tag (from the sidebar), not a query.
    tag: Option<Tag>,
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
            chips: Vec::new(),
            columns: Vec::new(),
            col_focus: 0,
            col_scroll: gpui::ScrollHandle::new(),
            strip_scroll: gpui::ScrollHandle::new(),
            strip_width: 600.,
            grid_cols: 1,
            marked: BTreeSet::new(),
            anchor: None,
            renaming: None,
            notice: None,
            menu: None,
            info_collapsed: false,
            tags: Tags::load(),
            pending_scroll: None,
            undo: Vec::new(),
            quick_look: false,
            peek: None,
            preview: None,
            hostable: HashMap::new(),
            hwnd: None,
            scale: 1.,
        };
        cx.subscribe(&b.search_input, |this, _, _: &text_input::Changed, cx| this.query_changed(cx)).detach();
        cx.subscribe(&b.search_input, |this, _, _: &text_input::Cancelled, cx| this.end_search(cx)).detach();
        cx.subscribe(&b.search_input, |this, _, _: &text_input::BackspaceAtStart, cx| {
            if this.chips.pop().is_some() {
                this.query_changed(cx);
            }
        })
        .detach();
        // FINDER_DIR / FINDER_VIEW pick where and how it opens (testing).
        let start = std::env::var_os("FINDER_DIR").map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or(start);
        b.load(if start.is_dir() { start } else { home() });
        b.view = match std::env::var("FINDER_VIEW").as_deref() {
            Ok("icons") => ViewMode::Icons,
            Ok("columns") => ViewMode::Columns,
            Ok("gallery") => ViewMode::Gallery,
            _ => ViewMode::List,
        };
        if let Ok(name) = std::env::var("FINDER_SELECT") {
            b.select_by_name(&name, cx);
        }
        // FINDER_TAG=Red opens that tag's view, as its sidebar row does.
        if let Some(tag) = std::env::var("FINDER_TAG").ok().and_then(|n| Tag::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(&n))) {
            b.show_tag(tag, cx);
        }
        // FINDER_SCROLL_TO=n scrolls row n (of the list, or of the icon
        // grid) to just under the toolbar, so content runs beneath it, once
        // the window has its size.
        b.pending_scroll = std::env::var("FINDER_SCROLL_TO").ok().and_then(|n| n.parse().ok()).map(|n| (n, 0));
        // FINDER_SEARCH="..." types a search at startup (for testing
        // without driving the keyboard).
        if let Ok(q) = std::env::var("FINDER_SEARCH") {
            b.search_input.update(cx, |i, cx| i.set_text(q, cx));
        }
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
        self.marked.clear();
        self.anchor = None;
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

    /// A column header click: that key, or the other direction of it.
    fn set_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        let ascending = if self.sort.0 == key { !self.sort.1 } else { true };
        self.sort_by(key, ascending, cx);
    }

    pub(super) fn sort_by(&mut self, key: SortKey, ascending: bool, cx: &mut Context<Self>) {
        self.sort = (key, ascending);
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

    // Selection

    pub(crate) fn is_selected(&self, ix: usize) -> bool {
        self.selected == Some(ix) || self.marked.contains(&ix)
    }

    /// Selected paths, in row order.
    fn selection(&self) -> Vec<PathBuf> {
        if self.shown_view() == ViewMode::Columns {
            return self.selected_entry().map(|e| e.path).into_iter().collect();
        }
        let mut ix: BTreeSet<usize> = self.marked.clone();
        ix.extend(self.selected);
        ix.into_iter().filter_map(|i| self.rows.get(i)).map(|r| r.entry.path.clone()).collect()
    }

    /// A click: plain selects, Ctrl toggles, Shift selects a range.
    fn click_select(&mut self, ix: usize, m: &gpui::Modifiers, cx: &mut Context<Self>) {
        if m.control {
            if self.is_selected(ix) {
                self.marked.remove(&ix);
                if self.selected == Some(ix) {
                    self.selected = self.marked.pop_last();
                }
            } else {
                self.marked.extend(self.selected);
                self.selected = Some(ix);
            }
            self.anchor = Some(ix);
            cx.notify();
        } else if m.shift {
            let a = self.anchor.unwrap_or(ix);
            self.marked = (a.min(ix)..=a.max(ix)).filter(|&i| i != ix).collect();
            self.selected = Some(ix);
            cx.notify();
        } else {
            self.select(ix, cx);
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let cur = self.selected.unwrap_or(0);
        self.selected = Some(cur);
        self.marked = (0..self.rows.len()).filter(|&i| i != cur).collect();
        cx.notify();
    }

    // File operations

    /// Re-read the folder, keeping folders opened in place, then select
    /// `select` (if they're there).
    fn refresh(&mut self, select: &[PathBuf], cx: &mut Context<Self>) {
        if self.search.is_some() {
            return;
        }
        let expanded: Vec<PathBuf> = self.rows.iter().filter(|r| r.expanded).map(|r| r.entry.path.clone()).collect();
        let cur = self.current.clone();
        self.load(cur);
        for p in expanded {
            if let Some(i) = self.rows.iter().position(|r| r.entry.path == p) {
                self.expand(i);
            }
        }
        let found: Vec<usize> = select.iter().filter_map(|p| self.rows.iter().position(|r| r.entry.path == *p)).collect();
        if let Some((&last, rest)) = found.split_last() {
            self.selected = Some(last);
            self.marked = rest.iter().copied().collect();
            self.reveal(last);
        }
        self.columns.clear();
        cx.notify();
    }

    /// Run `op` off the UI thread (shell operations show their own
    /// progress and prompts), then refresh selecting `after`.
    fn run_op(&mut self, op: impl FnOnce() -> Result<(), String> + Send + 'static, after: Vec<PathBuf>, undo: Option<Undo>, cx: &mut Context<Self>) {
        self.notice = None;
        cx.spawn(async move |this, cx| {
            let res = cx.background_spawn(async move { op() }).await;
            let _ = this.update(cx, |b, cx| {
                match res {
                    Ok(()) => {
                        if let Some(Undo::Moved(pairs)) = &undo {
                            for (now, was) in pairs {
                                b.tags.moved(was, now);
                            }
                        }
                        b.undo.extend(undo)
                    }
                    Err(e) => b.notice = Some(e),
                }
                b.refresh(&after, cx);
            });
        })
        .detach();
    }

    /// Reverse the last operation.
    fn undo_op(&mut self, _: &UndoOp, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let Some(u) = self.undo.pop() else { return };
        match u {
            Undo::Rename { from, to } => {
                self.tags.moved(&to, &from);
                let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.run_op(move || ops::rename(&to, &name).map(|_| ()), vec![from], None, cx);
            }
            Undo::Created(paths) => self.run_op(move || ops::run(ops::Op::Recycle, &paths, None), Vec::new(), None, cx),
            Undo::Moved(pairs) => {
                for (now, was) in &pairs {
                    self.tags.moved(now, was);
                }
                let back: Vec<PathBuf> = pairs.iter().map(|(_, was)| was.clone()).collect();
                self.run_op(
                    move || {
                        for (now, was) in &pairs {
                            ops::run(ops::Op::Move, std::slice::from_ref(now), was.parent())?;
                        }
                        Ok(())
                    },
                    back,
                    None,
                    cx,
                );
            }
            Undo::Trashed(paths) => {
                let back = paths.clone();
                self.run_op(move || ops::restore(&paths), back, None, cx);
            }
        }
    }

    fn copy_files(&mut self, _: &CopyFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.put_on_clipboard(false, window, cx);
    }

    fn cut_files(&mut self, _: &CutFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.put_on_clipboard(true, window, cx);
    }

    fn put_on_clipboard(&mut self, cut: bool, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selection();
        if !self.focus.is_focused(window) || paths.is_empty() {
            return;
        }
        self.notice = clipboard::put_files(&paths, cut).err();
        cx.notify();
    }

    /// Paste into the current folder: copies, or moves what was cut.
    /// Copies into the folder they came from are named like Finder's
    /// ("name copy"), not Explorer's ("name - Copy").
    fn paste_files(&mut self, _: &PasteFiles, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) || self.search.is_some() {
            return;
        }
        let Some((from, cut)) = clipboard::get_files() else { return };
        let dir = self.current.clone();
        let (same, other): (Vec<PathBuf>, Vec<PathBuf>) = from.into_iter().partition(|p| p.parent() == Some(dir.as_path()));
        if cut && other.is_empty() {
            return; // already here
        }
        let copies: Vec<PathBuf> = if cut { Vec::new() } else { same.iter().map(|p| ops::copy_name(p)).collect() };
        let mut after: Vec<PathBuf> = other.iter().filter_map(|p| p.file_name().map(|n| dir.join(n))).collect();
        let undo = if cut { Undo::Moved(after.iter().cloned().zip(other.iter().cloned()).collect()) } else { Undo::Created(after.iter().chain(&copies).cloned().collect()) };
        after.extend(copies.iter().cloned());
        self.run_op(
            move || {
                if cut {
                    ops::run(ops::Op::Move, &other, Some(&dir))?;
                    clipboard::clear();
                } else {
                    ops::run(ops::Op::Copy, &other, Some(&dir))?;
                    ops::copy_as(&same, &copies)?;
                }
                Ok(())
            },
            after,
            Some(undo),
            cx,
        );
    }

    /// The selected paths as text (Finder's Option-Command-C).
    fn copy_path(&mut self, _: &CopyPath, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.selection();
        if !self.focus.is_focused(window) || paths.is_empty() {
            return;
        }
        let text = paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\r\n");
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
    }

    fn trash(&mut self, _: &Trash, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let paths = self.selection();
        if paths.is_empty() {
            return;
        }
        // Land on the row after the last one removed.
        let next = self.selected.and_then(|i| self.rows.iter().skip(i).find(|r| !paths.contains(&r.entry.path))).map(|r| r.entry.path.clone());
        let undo = Undo::Trashed(paths.clone());
        self.run_op(move || ops::run(ops::Op::Recycle, &paths, None), next.into_iter().collect(), Some(undo), cx);
    }

    fn duplicate(&mut self, _: &Duplicate, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let from = self.selection();
        let to: Vec<PathBuf> = from.iter().map(|p| ops::copy_name(p)).collect();
        if from.is_empty() {
            return;
        }
        let after = to.clone();
        let undo = Undo::Created(to.clone());
        self.run_op(move || ops::copy_as(&from, &to), after, Some(undo), cx);
    }

    fn new_folder(&mut self, _: &NewFolder, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            return;
        }
        match ops::new_folder(&self.current) {
            Ok(p) => {
                self.undo.push(Undo::Created(vec![p.clone()]));
                self.refresh(std::slice::from_ref(&p), cx);
                self.start_rename(p, window, cx);
            }
            Err(e) => {
                self.notice = Some(e);
                cx.notify();
            }
        }
    }

    fn rename_action(&mut self, _: &Rename, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) || !matches!(self.shown_view(), ViewMode::List | ViewMode::Icons) {
            return;
        }
        if let Some(e) = self.selected_entry() {
            self.start_rename(e.path, window, cx);
        }
    }

    /// Edit a name in place: Enter or clicking away saves, Escape doesn't.
    fn start_rename(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // Finder selects the name without its extension.
        let stem = match name.rfind('.') {
            Some(i) if i > 0 && !path.is_dir() => i,
            _ => name.len(),
        };
        let input = cx.new(|cx| {
            let mut i = TextInput::new("", cx);
            i.set_text(name, cx);
            i.select_range(0..stem, cx);
            i
        });
        let subs = vec![
            cx.subscribe_in(&input, window, |this, _, _: &text_input::Confirmed, window, cx| this.finish_rename(true, window, cx)),
            cx.subscribe_in(&input, window, |this, _, _: &text_input::Cancelled, window, cx| this.finish_rename(false, window, cx)),
            cx.on_blur(&input.focus_handle(cx), window, |this, window, cx| this.finish_rename(true, window, cx)),
        ];
        window.focus(&input.focus_handle(cx), cx);
        self.renaming = Some(Renaming { path, input, _subs: subs });
        cx.notify();
    }

    fn finish_rename(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.renaming.take() else { return };
        window.focus(&self.focus, cx);
        let name = r.input.read(cx).text().to_string();
        let unchanged = r.path.file_name().is_some_and(|n| n.to_string_lossy() == name);
        if !save || unchanged {
            cx.notify();
            return;
        }
        match ops::rename(&r.path, &name) {
            Ok(to) => {
                self.notice = None;
                self.tags.moved(&r.path, &to);
                self.undo.push(Undo::Rename { from: r.path.clone(), to: to.clone() });
                self.refresh(&[to], cx);
            }
            Err(e) => {
                self.notice = Some(e);
                cx.notify();
            }
        }
    }

    /// The box a name is edited in, in place of the name.
    fn rename_box(&self, path: &Path) -> Option<AnyElement> {
        let r = self.renaming.as_ref().filter(|r| r.path == path)?;
        Some(
            div()
                .flex_1()
                .min_w_0()
                .h(px(18.))
                .px(px(3.))
                .flex()
                .items_center()
                .rounded(px(3.))
                .bg(t::content_bg())
                .border_1()
                .border_color(t::focus_ring())
                .text_color(t::text())
                .child(r.input.clone())
                .into_any_element(),
        )
    }

    // Search

    fn query_changed(&mut self, cx: &mut Context<Self>) {
        // A filter followed by a space becomes a chip.
        let raw = self.search_input.read(cx).text().to_string();
        let mut words: Vec<&str> = raw.split(' ').collect();
        let open = words.pop().unwrap_or_default();
        let (filters, rest): (Vec<&str>, Vec<&str>) = words.into_iter().filter(|w| !w.is_empty()).partition(|w| chip_of(w).is_some());
        if !filters.is_empty() {
            self.chips.extend(filters.iter().filter_map(|w| chip_of(w)));
            let left = rest.into_iter().chain([open]).collect::<Vec<_>>().join(" ");
            // Re-enters with the filters gone.
            self.search_input.update(cx, |i, cx| i.set_text(left, cx));
            return;
        }
        let typed = raw.trim();
        let text = self.chips.iter().map(|(k, v)| format!("{k}:{v}")).chain((!typed.is_empty()).then(|| typed.to_string())).collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            if self.search.as_ref().is_some_and(|s| s.tag.is_some()) {
                return;
            }
            if self.search.take().is_some() {
                let cur = self.current.clone();
                self.load(cur);
                cx.notify();
            }
            return;
        }
        if self.search.as_ref().is_some_and(|s| s.tag.is_some()) {
            self.search = None;
        }
        if self.search.is_none() {
            self.search = Some(Search { root: self.current.clone(), scope: Scope::ThisPc, took: None, capped: false, error: None, sorted: false, tag: None });
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
        // Finder shows nothing rather than letters scattered across a name.
        q.min_quality(12);
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
                self.marked.clear();
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
            self.query_changed(cx);
        }
    }

    /// Everything tagged `tag`, as Finder's sidebar tags show it.
    pub(super) fn show_tag(&mut self, tag: Tag, cx: &mut Context<Self>) {
        self.chips.clear();
        self.search_gen += 1;
        self.search = Some(Search { root: self.current.clone(), scope: Scope::ThisPc, took: None, capped: false, error: None, sorted: true, tag: Some(tag) });
        self.search_input.update(cx, |i, cx| i.set_text("", cx));
        let mut v: Vec<Entry> = self.tags.tagged(tag).into_iter().filter_map(Entry::from_path).collect();
        files::sort(&mut v, self.sort.0, self.sort.1);
        self.rows = v.into_iter().map(|entry| Row { entry, depth: 0, expanded: false }).collect();
        self.selected = None;
        self.marked.clear();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Windows' share sheet, for the selection.
    pub(super) fn share_selection(&mut self, cx: &mut Context<Self>) {
        let Some(hwnd) = self.hwnd else { return };
        if let Err(e) = crate::share::share(hwnd, self.selection()) {
            self.notice = Some(format!("Couldn't share: {e}"));
            cx.notify();
        }
    }

    /// Toggles `tag` on the selection.
    pub(super) fn toggle_tag(&mut self, tag: Tag, cx: &mut Context<Self>) {
        let paths = self.selection();
        self.tags.toggle(&paths, tag);
        // A tag view drops what no longer has its tag.
        if let Some(t) = self.search.as_ref().and_then(|s| s.tag) {
            self.show_tag(t, cx);
        }
        cx.notify();
    }

    /// Leave search: clear the field, back to the folder.
    fn end_search(&mut self, cx: &mut Context<Self>) {
        self.chips.clear();
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
        if self.quick_look {
            self.quick_look = false;
            cx.notify();
            return;
        }
        self.end_search(cx);
        window.focus(&self.focus, cx);
    }

    // Keyboard

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = Some(ix);
        self.marked.clear();
        self.anchor = Some(ix);
        self.reveal(ix);
        cx.notify();
    }

    // Arrow keys mean what they do in each view: rows in the list, cells
    // in the grid and filmstrip, columns in the column view.

    fn select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        // From the search field too: the arrow moves into the results.
        window.focus(&self.focus, cx);
        match self.shown_view() {
            ViewMode::Columns => self.col_step(1, cx),
            ViewMode::Icons => self.step(self.grid_cols as isize, cx),
            _ => self.step(1, cx),
        }
    }

    fn select_prev(&mut self, _: &SelectPrev, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        match self.shown_view() {
            ViewMode::Columns => self.col_step(-1, cx),
            ViewMode::Icons => self.step(-(self.grid_cols as isize), cx),
            _ => self.step(-1, cx),
        }
    }

    fn expand_selected(&mut self, _: &Expand, _: &mut Window, cx: &mut Context<Self>) {
        match self.shown_view() {
            ViewMode::Columns => return self.col_right(cx),
            ViewMode::Icons | ViewMode::Gallery => return self.step(1, cx),
            ViewMode::List => {}
        }
        if let Some(i) = self.selected {
            self.expand(i);
            cx.notify();
        }
    }

    /// Left arrow: fold the folder, or else go to its parent row.
    fn collapse_selected(&mut self, _: &Collapse, _: &mut Window, cx: &mut Context<Self>) {
        match self.shown_view() {
            ViewMode::Columns => return self.col_left(cx),
            ViewMode::Icons | ViewMode::Gallery => return self.step(-1, cx),
            ViewMode::List => {}
        }
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
        if self.shown_view() == ViewMode::Columns {
            return self.col_open(cx);
        }
        if let Some(i) = self.selected {
            self.open_row(i, cx);
        }
    }
}


/// `ext:pdf` and the like: a filter the field shows as a chip.
fn chip_of(word: &str) -> Option<(String, String)> {
    let (k, v) = word.split_once(':')?;
    let k = k.to_ascii_lowercase();
    let known = ["ext", "kind", "type", "size", "mtime", "modified", "in"];
    (known.contains(&k.as_str()) && !v.is_empty()).then(|| (k, v.to_string()))
}

/// How a chip reads: Finder's attribute name, then the value in words.
fn chip_label(k: &str, v: &str) -> (&'static str, String) {
    match k {
        "ext" => ("KIND", v.split(',').map(|e| files::kind_of(&format!("x.{e}"), false)).collect::<Vec<_>>().join(", ")),
        "kind" => (
            "KIND",
            match v {
                "dir" | "folder" | "d" => "Folder".into(),
                "file" | "f" => "Document".into(),
                "link" | "symlink" | "l" => "Alias".into(),
                v => v.into(),
            },
        ),
        "type" => ("KIND", capitalize(v)),
        "size" => ("SIZE", v.replace('>', "> ").replace('<', "< ").to_uppercase()),
        "mtime" | "modified" => ("MODIFIED", v.replace('<', "within ").replace('>', "before ")),
        "in" => ("IN", v.into()),
        _ => ("", v.into()),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}

// Pieces

/// An SF Symbol (or its stand-in) at `pt` points: symbols sit in a square
/// 1.25 em frame, so this is the box that shows them at that size.
fn sym(name: &str, pt: f32, color: impl Into<gpui::Hsla>) -> gpui::Svg {
    svg().path(symbol(name)).size(px(pt * 1.25)).flex_none().text_color(color.into())
}

/// Liquid Glass filling its (relative) parent: the content beneath it,
/// frosted and bent at the rim.
fn glass_surface(radius: f32) -> impl IntoElement {
    gpui::canvas(|_, _, _| (), move |bounds, _, window, _| window.paint_glass(bounds, px(radius), t::glass_style()))
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

/// A Liquid Glass capsule floating over the content, holding buttons.
fn capsule(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .relative()
        .h(px(t::CAPSULE_H))
        .flex_none()
        .flex()
        .items_center()
        .px(px(4.))
        .gap(px(2.))
        .rounded_full()
        .child(glass_surface(t::CAPSULE_H / 2.))
}

/// Tag colors as overlapping dots, as Finder shows them beside a name.
pub(super) fn tag_dots(tags: &[Tag], d: f32) -> impl IntoElement {
    div().flex().flex_none().children(tags.iter().enumerate().map(move |(i, tag)| {
        div()
            .size(px(d))
            .rounded_full()
            .bg(tag.color())
            .border(px(1.))
            .border_color(t::content_bg())
            .when(i > 0, |el| el.ml(px(-d * 0.4)))
    }))
}

/// Where a button's menu drops from: just below the click.
fn menu_point(e: &ClickEvent) -> gpui::Point<gpui::Pixels> {
    let p = e.position();
    gpui::point(p.x - px(12.), p.y + px(18.))
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
                let on = p.path.as_deref() == Some(self.current.as_path()) && self.search.as_ref().is_none_or(|s| s.tag.is_none());
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
                        .when_some(p.path.clone(), |el, dest| dnd::folder_target(el, dest, cx))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if let Some(p) = &path {
                                this.navigate(p.clone(), cx);
                            }
                        })),
                );
            }
        }
        let showing = self.search.as_ref().and_then(|s| s.tag);
        col = col.child(
            div()
                .h(px(28.))
                .mt(px(8.))
                .px(px(10.))
                .flex()
                .flex_none()
                .items_end()
                .pb(px(6.))
                .text_size(px(t::SMALL_SIZE))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t::sidebar_header())
                .child("Tags"),
        );
        for (i, tag) in Tag::ALL.into_iter().enumerate() {
            let on = showing == Some(tag);
            col = col.child(
                div()
                    .id(("tag", i))
                    .h(px(t::SIDEBAR_ROW_H))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(8.))
                    .rounded(px(10.))
                    .when(on, |d| d.bg(t::sidebar_selected()))
                    .child(div().w(px(22.)).flex().justify_center().child(div().size(px(10.)).rounded_full().bg(tag.color())))
                    .child(div().text_size(px(t::TEXT_SIZE)).when(on, |d| d.text_color(t::accent())).child(tag.name()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.show_tag(tag, cx))),
            );
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
            Some(Search { tag: Some(tag), .. }) => tag.name().to_string(),
            Some(s) if s.scope == Scope::Folder => format!("Searching \u{201c}{}\u{201d}", files::display_name(&s.root)),
            Some(_) => "Searching \u{201c}This PC\u{201d}".to_string(),
            None => files::display_name(&self.current),
        };
        let view_btn = |id: &'static str, name: &'static str, mode: ViewMode, cx: &mut Context<Self>| {
            tool_button(id, name, self.view == mode).on_click(cx.listener(move |this, _, _, cx| {
                this.set_view(mode, cx);
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
                        .on_click(cx.listener(|this, e: &ClickEvent, _, cx| this.open_sort_menu(menu_point(e), cx)))
                        .gap(px(1.))
                        .child(sym("chevron.down@semibold", 8., t::glass_icon())),
                ),
            )
            .child(
                capsule("actions")
                    .child(tool_button("share", "square.and.arrow.up", false).on_click(cx.listener(|this, _, _, cx| this.share_selection(cx))))
                    .child(tool_button("tags", "tag", false).on_click(cx.listener(|this, e: &ClickEvent, _, cx| this.open_tag_menu(menu_point(e), cx))))
                    .child(tool_button("more", "ellipsis", false).on_click(cx.listener(|this, e: &ClickEvent, _, cx| this.open_action_menu(menu_point(e), cx)))),
            )
    }

    /// Everything above the content, floating over it: the toolbar, the
    /// search scope bar and the list's column header, on a veil that
    /// content scrolling beneath them shows through.
    fn toolbar_band(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.error.is_none() && self.shown_view() == ViewMode::List;
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .flex()
            .flex_col()
            .child(
                gpui::canvas(|_, _, _| (), |bounds, _, window, _| window.paint_scroll_edge(bounds, t::scroll_edge(), t::SCROLL_EDGE_BLUR))
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(self.toolbar(cx))
            .when(self.searching_query(), |d| d.child(self.scope_bar(cx)))
            // The column header draws its own hairline beneath it.
            .when(header, |d| d.child(self.column_header(cx)))
            .when(!header, |d| d.child(div().h(px(1.)).flex_none().bg(t::separator())))
    }

    /// A typed search (with its scope bar), rather than a tag view.
    fn searching_query(&self) -> bool {
        self.search.as_ref().is_some_and(|s| s.tag.is_none())
    }

    /// How far the toolbar band reaches down over the content, so views
    /// start their content below it.
    pub(super) fn band_h(&self) -> f32 {
        let header = self.error.is_none() && self.shown_view() == ViewMode::List;
        t::TOOLBAR_H
            + if self.searching_query() { t::SCOPE_BAR_H } else { 0. }
            + if header { t::HEADER_H } else { 1. }
    }

    /// The inspector column's own strip of toolbar: the search field.
    fn search_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.search_input.focus_handle(cx).is_focused(window);
        let has_text = !self.search_input.read(cx).text().is_empty() || !self.chips.is_empty();
        div().h(px(t::TOOLBAR_H)).flex_none().flex().items_center().px(px(12.)).window_control_area(WindowControlArea::Drag).child(
            capsule("search")
                .flex_1()
                .px(px(10.))
                .gap(px(5.))
                .when(focused, |d| d.border_2().border_color(t::focus_ring()))
                .child(sym("magnifyingglass", 13., t::text_secondary()))
                .children(self.chips.iter().enumerate().map(|(i, (k, v))| {
                    let (label, value) = chip_label(k, v);
                    div()
                        .id(("chip", i))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .px(px(6.))
                        .py(px(1.))
                        .rounded(px(5.))
                        .bg(t::glass_pressed())
                        .text_size(px(t::SMALL_SIZE))
                        .child(div().text_size(px(9.)).font_weight(gpui::FontWeight::BOLD).text_color(t::text_secondary()).child(label))
                        .child(div().text_color(t::text()).child(value))
                }))
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
            .h(px(t::SCOPE_BAR_H))
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
                            this.marked.clear();
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Right, cx.listener(|this, e: &gpui::MouseDownEvent, window, cx| this.open_menu(None, e.position, window, cx)))
                        .into_any_element();
                }
                let entry = self.rows[ix].entry.clone();
                let src = self.icon_source(&entry, 32, cx);
                let sel = self.is_selected(ix);
                let drag = self.drag_value(&entry, sel, cx);
                let r = &self.rows[ix];
                let lit = sel && focused;
                let rename = self.rename_box(&entry.path);
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
                                    .child(match rename {
                                        Some(b) => b,
                                        None => div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .items_center()
                                            .gap(px(5.))
                                            .child(div().min_w_0().truncate().child(r.entry.name.clone()))
                                            .child(tag_dots(self.tags.of(&r.entry.path), 8.))
                                            .into_any_element(),
                                    }),
                            )
                            .child(div().w(px(t::DATE_W)).px(px(9.)).truncate().text_color(fg2).child(files::format_date(r.entry.modified)))
                            .child(div().w(px(t::SIZE_W)).px(px(9.)).flex().justify_end().text_color(fg2).child(files::format_size(&r.entry)))
                            .child(div().w(px(t::KIND_W)).px(px(9.)).truncate().text_color(fg2).child(r.entry.kind.clone()))
                            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                                window.focus(&this.focus, cx);
                                if e.click_count() >= 2 {
                                    this.open_row(ix, cx);
                                } else {
                                    this.click_select(ix, &e.modifiers(), cx);
                                }
                            }))
                            .on_mouse_down(MouseButton::Right, cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| this.open_menu(Some(ix), e.position, window, cx)))
                            .map(|el| dnd::drag_source(el, drag))
                            .when(entry.is_dir, |el| dnd::folder_target(el, entry.path.clone(), cx)),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn list(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fill = (window.viewport_size().height / px(t::ROW_H)).ceil() as usize;
        let body: AnyElement = if let Some(e) = &self.error {
            div().flex_1().pt(px(self.band_h())).flex().items_center().justify_center().text_color(t::text_secondary()).child(e.clone()).into_any_element()
        } else {
            uniform_list("rows", self.rows.len().max(fill), cx.processor(Self::render_rows))
                .flex_1()
                .pt(px(self.band_h()))
                .track_scroll(&self.scroll)
                .into_any_element()
        };
        // The column header floats over the rows, in the toolbar band.
        div().flex_1().min_h_0().flex().flex_col().child(body)
    }

    fn path_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // While searching it shows where the selected result is.
        let target = match (&self.search, self.view) {
            (Some(_), _) => self.selected_entry().map(|e| e.path),
            (None, ViewMode::Columns) => Some(self.selected_entry().map_or_else(|| self.current.clone(), |e| e.path)),
            (None, _) => Some(self.current.clone()),
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
                        .when(p.is_dir(), |el| dnd::folder_target(el, p.clone(), cx))
                        .on_click(cx.listener(move |this, _, _, cx| this.navigate(p.clone(), cx))),
                )
                .when(i + 1 < n, |d| d.child(sym("chevron.right@semibold", 7., t::text_tertiary())));
        }
        bar
    }

    /// "24 items, 58.9 GB available".
    fn status_bar(&self) -> impl IntoElement {
        let n = self.rows.iter().filter(|r| r.depth == 0).count();
        if let Some(e) = &self.notice {
            return div()
                .h(px(t::STATUS_BAR_H))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .bg(t::bar_bg())
                .text_size(px(t::SMALL_SIZE))
                .text_color(gpui::rgb(0xE5484D))
                .child(e.clone());
        }
        let picked = self.selection().len();
        let items = match (n, self.selected_entry().map(|_| ())) {
            (n, Some(_)) => format!("{picked} of {n} selected"),
            (1, None) => "1 item".to_string(),
            (n, None) => format!("{n} items"),
        };
        // A tag view isn't a search: it counts like a folder.
        if let Some(s) = self.search.as_ref().filter(|s| s.tag.is_none()) {
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
            .selected_entry()
            .map(|e| if e.created.is_none() { Entry::from_path(e.path.clone()).unwrap_or(e) } else { e })
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
                            .child(div().w(px(230.)).h(px(196.)).flex().items_center().justify_center().overflow_hidden().child(img(src).size_full().object_fit(gpui::ObjectFit::Contain)))
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
                        .child(
                            div()
                                .id("info-toggle")
                                .text_size(px(12.))
                                .text_color(t::accent())
                                .child(if self.info_collapsed { "Show More" } else { "Show Less" })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.info_collapsed = !this.info_collapsed;
                                    cx.notify();
                                })),
                        ),
                )
                .when(!self.info_collapsed, |d| {
                    d.child(info("Created", files::format_date_long(e.created)))
                        .child(info("Modified", files::format_date_long(e.modified)))
                        .child(info("Last opened", files::format_date_long(e.accessed)))
                })
                .when(self.info_collapsed, |d| d.child(info("Modified", files::format_date_long(e.modified))))
                .child(div().mt(px(14.)).mb(px(6.)).font_weight(gpui::FontWeight::BOLD).child("Tags"))
                .child({
                    let tags = self.tags.of(&e.path).to_vec();
                    div()
                        .id("tag-field")
                        .px(px(6.))
                        .py(px(5.))
                        .rounded(px(5.))
                        .bg(t::bar_bg())
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .when(tags.is_empty(), |d| d.text_color(t::text_tertiary()).child("Add Tags…"))
                        .children(tags.into_iter().map(|tag| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .px(px(6.))
                                .rounded(px(5.))
                                .bg(t::glass_pressed())
                                .child(div().size(px(8.)).rounded_full().bg(tag.color()))
                                .child(tag.name())
                        }))
                        .on_click(cx.listener(|this, e: &ClickEvent, _, cx| this.open_tag_menu(menu_point(e), cx)))
                });
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
                    .child("More…")
                    .on_click(cx.listener(|this, e: &ClickEvent, _, cx| this.open_action_menu(menu_point(e), cx))),
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
        self.scale = window.scale_factor();
        if let Some((row, waited)) = self.pending_scroll.take() {
            if waited < 5 {
                self.pending_scroll = Some((row, waited + 1));
                cx.on_next_frame(window, |_, _, cx| cx.notify());
            } else {
                self.scroll.scroll_to_item_strict(row, ScrollStrategy::Top);
            }
        }
        if !self.quick_look && self.preview.is_some() {
            let me = cx.entity();
            cx.defer(move |cx| me.update(cx, |this, cx| this.sync_preview(None, cx)));
        }
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
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::rename_action))
            .on_action(cx.listener(Self::new_folder))
            .on_action(cx.listener(Self::duplicate))
            .on_action(cx.listener(Self::trash))
            .on_action(cx.listener(Self::copy_files))
            .on_action(cx.listener(Self::cut_files))
            .on_action(cx.listener(Self::paste_files))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(Self::reveal_in_explorer))
            .on_action(cx.listener(Self::show_icons))
            .on_action(cx.listener(Self::show_list))
            .on_action(cx.listener(Self::show_columns))
            .on_action(cx.listener(Self::show_gallery))
            .on_action(cx.listener(Self::context_menu_key))
            .on_action(cx.listener(Self::action_menu_key))
            .on_action(cx.listener(Self::sort_menu_key))
            .on_action(cx.listener(Self::tag_menu_key))
            .on_action(cx.listener(Self::undo_op))
            .on_action(cx.listener(Self::toggle_quick_look))
            .on_action(cx.listener(Self::escape))
            .on_mouse_down(MouseButton::Navigate(gpui::NavigationDirection::Back), cx.listener(|this, _, w, cx| this.go_back(&GoBack, w, cx)))
            .on_mouse_down(MouseButton::Navigate(gpui::NavigationDirection::Forward), cx.listener(|this, _, w, cx| this.go_forward(&GoForward, w, cx)))
            .size_full()
            .relative()
            .flex()
            // No fill of its own: the sidebar lets the window's backdrop through.
            .font_family(self.font.clone())
            .text_size(px(t::TEXT_SIZE))
            .text_color(t::text())
            .children(self.render_quick_look(window, cx))
            .children(self.render_menu(cx))
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
                            .relative()
                            .flex()
                            .flex_col()
                            // The view runs to the top; the toolbar band floats over it.
                            .child({
                                let view = self.main_view(window, cx);
                                let el = div().id("view-drop").flex_1().min_h_0().flex().flex_col().child(view);
                                if self.search.is_none() { dnd::folder_target(el, self.current.clone(), cx).into_any_element() } else { el.into_any_element() }
                            })
                            .child(self.path_bar(cx))
                            .child(self.status_bar())
                            .child(self.toolbar_band(cx)),
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
