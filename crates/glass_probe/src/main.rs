//! Liquid Glass probe: Finder's toolbar in glass over content that scrolls
//! beneath it, plus a free lens, with keys to tune the optics live.
//!
//! Drag the lens. Keys: R/F refraction, E/W bezel, B/V blur, D dispersion,
//! H highlight, S shadow, T tint, M dark, C clear/regular, 0 reset.
//!
//! GLASS="blur=0,refraction=40,x=300,y=200,scroll=120" sets the starting
//! state, for screenshots without driving the keyboard.

use std::borrow::Cow;

use gpui::{
    App, AssetSource, Bounds, Context, FocusHandle, FontWeight, GlassStyle, Hsla, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollHandle,
    SharedString, Window, WindowBounds, WindowOptions, canvas, div, hsla, img, point, prelude::*,
    px, rgb, size, svg,
};
use rust_embed::Embed;

const WALLPAPER: &str = "C:/Windows/Web/Wallpaper/Windows/img0.jpg";
const TOOLBAR_H: f32 = 52.;
const CAPSULE_H: f32 = 36.;

/// Finder's icons: SF Symbols where extracted locally, else its stand-ins.
#[derive(Embed)]
#[folder = "../finder/assets/icons"]
#[prefix = "icons/"]
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(Self::get(path).map(|f| f.data))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(Self::iter().filter(|p| p.starts_with(path)).map(|p| p.to_string().into()).collect())
    }
}

const STAND_INS: &[(&str, &str)] = &[
    ("chevron.left", "chevron-left"),
    ("chevron.right", "chevron-right"),
    ("chevron.down", "chevron-down"),
    ("square.grid.2x2", "grid"),
    ("list.bullet", "list"),
    ("rectangle.split.3x1", "columns"),
    ("squares.below.rectangle", "gallery"),
    ("square.grid.3x1.below.line.grid.1x2", "group"),
    ("square.and.arrow.up", "share"),
    ("tag", "tag"),
    ("ellipsis", "ellipsis"),
    ("magnifyingglass", "search"),
];

fn sym(name: &str, pt: f32, color: Hsla) -> gpui::Svg {
    let sf = format!("icons/sf/{name}.svg");
    let path = if Assets::get(&sf).is_some() {
        sf
    } else {
        let base = name.split('@').next().unwrap_or(name);
        let file = STAND_INS.iter().find(|(n, _)| *n == base).map_or(base, |(_, f)| f);
        format!("icons/{file}.svg")
    };
    svg().path(path).size(px(pt * 1.25)).flex_none().text_color(color)
}

struct Probe {
    focus: FocusHandle,
    style: GlassStyle,
    lens: Point<Pixels>,
    drag: Option<Point<Pixels>>,
    scroll: ScrollHandle,
    /// A starting scroll offset, applied once the list has been laid out.
    pending_scroll: Option<f32>,
}

/// A glass surface filling its (relative) parent.
fn glass(radius: f32, style: GlassStyle) -> impl IntoElement {
    canvas(|_, _, _| (), move |bounds, _, window, _| window.paint_glass(bounds, px(radius), style))
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

/// A Liquid Glass capsule holding toolbar buttons, as in Finder.
fn capsule(style: GlassStyle) -> gpui::Div {
    div()
        .relative()
        .h(px(CAPSULE_H))
        .flex_none()
        .flex()
        .items_center()
        .px(px(4.))
        .gap(px(2.))
        .child(glass(CAPSULE_H / 2., style))
}

fn icon_color(style: &GlassStyle) -> Hsla {
    if style.dark { hsla(0., 0., 1., 0.85) } else { hsla(0., 0., 0.12, 1.) }
}

fn tool_button(name: &str, on: bool, style: &GlassStyle) -> gpui::Div {
    let pressed = if style.dark { hsla(0., 0., 1., 0.12) } else { hsla(0., 0., 0., 0.07) };
    div()
        .h(px(CAPSULE_H - 8.))
        .min_w(px(38.))
        .px(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .when(on, |d| d.bg(pressed))
        .child(sym(name, 15., icon_color(style)))
}

impl Probe {
    fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let s = &mut self.style;
        match event.keystroke.key.as_str() {
            "r" => s.refraction += 5.,
            "f" => s.refraction = (s.refraction - 5.).max(0.),
            "e" => s.bezel += 2.,
            "w" => s.bezel = (s.bezel - 2.).max(1.),
            "b" => s.blur += 1.,
            "v" => s.blur = (s.blur - 1.).max(0.),
            "d" => s.dispersion = if s.dispersion > 0. { 0. } else { 0.03 },
            "h" => s.highlight = if s.highlight > 0. { 0. } else { 1. },
            "t" => s.tint = if s.tint.a > 0. { Hsla::transparent_black() } else { hsla(0.58, 0.9, 0.5, 0.6) },
            "s" => s.shadow = if s.shadow > 0. { 0. } else { GlassStyle::default().shadow },
            "m" => s.dark = !s.dark,
            "c" => s.blur = if s.blur > 0. { 0. } else { 7.33 },
            "0" => *s = GlassStyle::default(),
            _ => return,
        }
        cx.notify();
    }

    fn toolbar(&self) -> impl IntoElement {
        let s = self.style;
        let separator = if s.dark { hsla(0., 0., 1., 0.1) } else { hsla(0., 0., 0., 0.09) };
        let secondary = if s.dark { hsla(0., 0., 1., 0.55) } else { hsla(0., 0., 0., 0.5) };
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(TOOLBAR_H))
            .flex()
            .items_center()
            .px(px(12.))
            .gap(px(10.))
            .child(
                capsule(s)
                    .child(tool_button("chevron.left@medium", false, &s))
                    .child(div().w(px(1.)).h(px(16.)).bg(separator))
                    .child(tool_button("chevron.right@medium", false, &s).opacity(0.3)),
            )
            .child(
                div()
                    .flex_1()
                    .px(px(4.))
                    .text_size(px(15.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if s.dark { hsla(0., 0., 1., 0.88) } else { hsla(0., 0., 0., 0.87) })
                    .child("Documents"),
            )
            .child(
                capsule(s)
                    .child(tool_button("square.grid.2x2", false, &s))
                    .child(tool_button("list.bullet", true, &s))
                    .child(tool_button("rectangle.split.3x1", false, &s))
                    .child(tool_button("squares.below.rectangle", false, &s)),
            )
            .child(
                capsule(s).child(
                    tool_button("square.grid.3x1.below.line.grid.1x2", false, &s)
                        .gap(px(1.))
                        .child(sym("chevron.down@semibold", 8., icon_color(&s))),
                ),
            )
            .child(
                capsule(s)
                    .child(tool_button("square.and.arrow.up", false, &s))
                    .child(tool_button("tag", false, &s))
                    .child(tool_button("ellipsis", false, &s)),
            )
            .child(
                capsule(s)
                    .w(px(220.))
                    .px(px(10.))
                    .gap(px(5.))
                    .child(sym("magnifyingglass", 13., secondary))
                    .child(div().text_size(px(13.)).text_color(secondary).child("Search")),
            )
    }
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.focus(&self.focus, cx);
        if let Some(offset) = self.pending_scroll {
            if self.scroll.max_offset().y > px(0.) {
                self.scroll.set_offset(point(px(0.), px(-offset)));
                self.pending_scroll = None;
            } else {
                cx.on_next_frame(window, |_, _, cx| cx.notify());
            }
        }
        let style = self.style;
        let (text, line) = if style.dark { (rgb(0xf2f2f7), rgb(0x2c2c2e)) } else { (rgb(0x1d1d1f), rgb(0xe5e5e5)) };
        let rows = (0..80).map(|i| {
            let hue = (i as f32 * 0.07) % 1.0;
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .h(px(36.))
                .border_b_1()
                .border_color(line)
                .child(div().size(px(18.)).rounded(px(4.)).bg(hsla(hue, 0.75, 0.55, 1.)))
                .child(div().text_size(px(14.)).text_color(text).child(SharedString::from(format!(
                    "Quarterly Report {i:02}.pdf — The quick brown fox jumps over the lazy dog"
                ))))
        });
        let hud = format!(
            "refraction {:.0}  bezel {:.0}  blur {:.2}  dispersion {:.2}  highlight {:.0}  shadow {:.2}  tint {}  {}   ·  R/F E/W B/V D H S T M C 0",
            style.refraction,
            style.bezel,
            style.blur,
            style.dispersion,
            style.highlight,
            style.shadow,
            if style.tint.a > 0. { "on" } else { "off" },
            if style.dark { "dark" } else { "light" },
        );
        let lens = self.lens;

        div()
            .id("root")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| {
                let p = e.position;
                let l = this.lens;
                if p.x >= l.x && p.x <= l.x + px(260.) && p.y >= l.y && p.y <= l.y + px(140.) {
                    this.drag = Some(p - l);
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if let Some(offset) = this.drag {
                    this.lens = e.position - offset;
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| this.drag = None))
            .size_full()
            .relative()
            .bg(if style.dark { rgb(0x1e1e1e) } else { rgb(0xffffff) })
            // Content runs under the toolbar: a wallpaper beside a list.
            .child(
                div()
                    .absolute()
                    .size_full()
                    .flex()
                    .child(img(std::path::PathBuf::from(WALLPAPER)).w(px(420.)).h_full().object_fit(gpui::ObjectFit::Cover))
                    .child(
                        div()
                            .id("list")
                            .flex_1()
                            .h_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .pt(px(TOOLBAR_H))
                            .children(rows),
                    ),
            )
            .child(self.toolbar())
            // A free lens, for judging the optics at size.
            .child(div().absolute().left(lens.x).top(lens.y).w(px(260.)).h(px(140.)).child(glass(36., style)))
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .bg(rgb(0x1d1d1f))
                    .text_color(rgb(0xffffff))
                    .text_size(px(12.))
                    .child(hud),
            )
    }
}

fn initial() -> (GlassStyle, Point<Pixels>, f32) {
    let mut style = GlassStyle::default();
    let mut lens = point(px(300.), px(300.));
    let mut scroll = 0.;
    for pair in std::env::var("GLASS").unwrap_or_default().split(',') {
        let Some((k, v)) = pair.split_once('=') else { continue };
        let Ok(v) = v.parse::<f32>() else { continue };
        match k {
            "refraction" => style.refraction = v,
            "bezel" => style.bezel = v,
            "blur" => style.blur = v,
            "dispersion" => style.dispersion = v,
            "highlight" => style.highlight = v,
            "shadow" => style.shadow = v,
            "dark" => style.dark = v > 0.,
            "tint" => style.tint = hsla(0.58, 0.9, 0.5, v),
            "x" => lens.x = px(v),
            "y" => lens.y = px(v),
            "scroll" => scroll = v,
            _ => {}
        }
    }
    (style, lens, scroll)
}

fn main() {
    gpui_platform::application().with_assets(Assets).run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1180.), px(760.)), cx);
        cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            |_, cx| {
                let (style, lens, offset) = initial();
                cx.new(|cx| Probe {
                    focus: cx.focus_handle(),
                    style,
                    lens,
                    drag: None,
                    scroll: ScrollHandle::new(),
                    pending_scroll: (offset > 0.).then_some(offset),
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
