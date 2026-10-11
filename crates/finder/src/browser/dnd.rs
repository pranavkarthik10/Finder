//! Drag and drop: items dragged within the window, and files dropped in
//! from Explorer, onto folders (rows, icons, columns, sidebar, path bar) or
//! onto the view itself (the current folder).

use super::*;
use gpui::{ExternalPaths, ImageSource, StyleRefinement};

/// What a drag carries.
#[derive(Clone)]
pub(super) struct Dragged {
    pub paths: Vec<PathBuf>,
    name: SharedString,
    icon: ImageSource,
}

/// What follows the pointer: the item's icon and name, and a count badge
/// when several are dragged.
struct DragPreview(Dragged);

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let n = self.0.paths.len();
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .opacity(0.85)
            .child(div().relative().child(img(self.0.icon.clone()).size(px(32.))).when(n > 1, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(-6.))
                        .right(px(-8.))
                        .px(px(5.))
                        .rounded_full()
                        .bg(gpui::rgb(0xFF3B30))
                        .text_size(px(11.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(gpui::white())
                        .child(n.to_string()),
                )
            }))
            .child(div().px(px(6.)).py(px(1.)).rounded(px(4.)).bg(t::selection()).text_size(px(12.)).text_color(gpui::white()).child(self.0.name.clone()))
    }
}

/// Paths share a volume (drive letter).
fn same_volume(a: &Path, b: &Path) -> bool {
    let root = |p: &Path| p.components().next().map(|c| c.as_os_str().to_ascii_uppercase());
    root(a) == root(b)
}

impl Browser {
    /// What dragging `e` carries: the whole selection if it's in it.
    pub(super) fn drag_value(&mut self, e: &Entry, selected: bool, cx: &mut Context<Self>) -> Dragged {
        let paths = if selected { self.selection() } else { vec![e.path.clone()] };
        Dragged { paths, name: e.name.clone(), icon: self.icon_source(e, 64, cx) }
    }

    /// Move (or copy, across drives or with Ctrl/Alt held) into `dest`.
    pub(super) fn drop_into(&mut self, dest: PathBuf, paths: Vec<PathBuf>, window: &Window, cx: &mut Context<Self>) {
        // Nothing into itself, its own subfolders, or where it already is.
        let paths: Vec<PathBuf> = paths.into_iter().filter(|p| p.parent() != Some(dest.as_path()) && !dest.starts_with(p)).collect();
        let Some(first) = paths.first() else { return };
        let m = window.modifiers();
        let copy = m.control || m.alt || !same_volume(first, &dest);
        let after: Vec<PathBuf> =
            if dest == self.current { paths.iter().filter_map(|p| p.file_name().map(|n| dest.join(n))).collect() } else { Vec::new() };
        self.run_op(move || ops::run(if copy { ops::Op::Copy } else { ops::Op::Move }, &paths, Some(&dest)), after, cx);
    }
}

/// Make an element accept drops into folder `dest`, highlighting while
/// something hovers over it.
pub(super) fn folder_target<E: InteractiveElement + StatefulInteractiveElement + Styled>(el: E, dest: PathBuf, cx: &mut Context<Browser>) -> E {
    let d2 = dest.clone();
    let highlight = |s: StyleRefinement| s.bg(t::drop_target());
    el.drag_over::<Dragged>(move |s, _, _, _| highlight(s))
        .drag_over::<ExternalPaths>(move |s, _, _, _| highlight(s))
        .on_drop(cx.listener(move |this, d: &Dragged, window, cx| {
            cx.stop_propagation();
            this.drop_into(dest.clone(), d.paths.clone(), window, cx);
        }))
        .on_drop(cx.listener(move |this, d: &ExternalPaths, window, cx| {
            cx.stop_propagation();
            this.drop_into(d2.clone(), d.paths().to_vec(), window, cx);
        }))
}

/// Make an element draggable as the item at `path`.
pub(super) fn drag_source<E: StatefulInteractiveElement>(el: E, value: Dragged) -> E {
    el.on_drag(value, |d, _, _, cx| cx.new(|_| DragPreview(d.clone())))
}
