//! Liquid Glass probe: glass surfaces over a wallpaper and scrolling text,
//! with keys to tune the optics live.
//!
//! Drag the lens. Keys: R/F refraction, E/W bezel, B/V blur, D dispersion,
//! H highlight, T tint, M dark, C clear/regular, 0 reset.
//!
//! GLASS="blur=0,refraction=40,x=300,y=200" sets the starting state, for
//! screenshots without driving the keyboard.

use gpui::{
    App, Bounds, Context, FocusHandle, GlassStyle, Hsla, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, Window, WindowBounds,
    WindowOptions, canvas, div, hsla, img, point, prelude::*, px, rgb, size,
};

const WALLPAPER: &str = "C:/Windows/Web/Wallpaper/Windows/img0.jpg";

struct Probe {
    focus: FocusHandle,
    style: GlassStyle,
    lens: Point<Pixels>,
    drag: Option<Point<Pixels>>,
}

fn glass(w: f32, h: f32, radius: f32, style: GlassStyle) -> impl IntoElement {
    canvas(|_, _, _| (), move |bounds, _, window, _| window.paint_glass(bounds, px(radius), style))
        .absolute()
        .size_full()
        .w(px(w))
        .h(px(h))
}

impl Probe {
    fn key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let s = &mut self.style;
        match event.keystroke.key.as_str() {
            "r" => s.refraction += 5.,
            "f" => s.refraction -= 5.,
            "e" => s.bezel += 2.,
            "w" => s.bezel = (s.bezel - 2.).max(1.),
            "b" => s.blur += 1.,
            "v" => s.blur = (s.blur - 1.).max(0.),
            "d" => s.dispersion = if s.dispersion > 0. { 0. } else { 0.1 },
            "h" => s.highlight = if s.highlight > 0. { 0. } else { 1. },
            "t" => s.tint = if s.tint.a > 0. { Hsla::transparent_black() } else { hsla(0.58, 0.9, 0.5, 0.6) },
            "m" => s.dark = !s.dark,
            "c" => s.blur = if s.blur > 0. { 0. } else { 7.33 },
            "0" => *s = GlassStyle::default(),
            _ => return,
        }
        cx.notify();
    }
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.focus(&self.focus, cx);
        let style = self.style;
        let rows = (0..80).map(|i| {
            let hue = (i as f32 * 0.07) % 1.0;
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .h(px(36.))
                .border_b_1()
                .border_color(rgb(0xe5e5e5))
                .child(div().size(px(18.)).rounded(px(4.)).bg(hsla(hue, 0.75, 0.55, 1.)))
                .child(div().text_size(px(14.)).text_color(rgb(0x1d1d1f)).child(SharedString::from(format!(
                    "Quarterly Report {i:02}.pdf — The quick brown fox jumps over the lazy dog"
                ))))
        });
        let hud = format!(
            "refraction {:.0}  bezel {:.0}  blur {:.2}  dispersion {:.2}  highlight {:.0}  tint {}  {}   ·  R/F E/W B/V D H T M C 0",
            style.refraction,
            style.bezel,
            style.blur,
            style.dispersion,
            style.highlight,
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
            .bg(rgb(0xffffff))
            .flex()
            // Backdrop: a wallpaper beside scrolling text.
            .child(img(std::path::PathBuf::from(WALLPAPER)).w(px(560.)).h_full().object_fit(gpui::ObjectFit::Cover))
            .child(div().id("list").flex_1().h_full().overflow_y_scroll().children(rows))
            // A toolbar capsule floating over the content.
            .child(
                div()
                    .absolute()
                    .top(px(14.))
                    .left(px(420.))
                    .w(px(220.))
                    .h(px(38.))
                    .child(glass(220., 38., 19., style))
                    .child(
                        div()
                            .absolute()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_around()
                            .text_size(px(15.))
                            .text_color(if style.dark { rgb(0xffffff) } else { rgb(0x1d1d1f) })
                            .child("◀")
                            .child("▶")
                            .child("☰")
                            .child("⌕"),
                    ),
            )
            // A round button.
            .child(div().absolute().top(px(14.)).left(px(660.)).size(px(38.)).child(glass(38., 38., 19., style)))
            // The draggable lens.
            .child(div().absolute().left(lens.x).top(lens.y).w(px(260.)).h(px(140.)).child(glass(260., 140., 36., style)))
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

fn initial() -> (GlassStyle, Point<Pixels>) {
    let mut style = GlassStyle::default();
    let mut lens = point(px(380.), px(220.));
    for pair in std::env::var("GLASS").unwrap_or_default().split(',') {
        let Some((k, v)) = pair.split_once('=') else { continue };
        let Ok(v) = v.parse::<f32>() else { continue };
        match k {
            "refraction" => style.refraction = v,
            "bezel" => style.bezel = v,
            "blur" => style.blur = v,
            "dispersion" => style.dispersion = v,
            "highlight" => style.highlight = v,
            "dark" => style.dark = v > 0.,
            "tint" => style.tint = hsla(0.58, 0.9, 0.5, v),
            "x" => lens.x = px(v),
            "y" => lens.y = px(v),
            _ => {}
        }
    }
    (style, lens)
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1180.), px(760.)), cx);
        cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            |_, cx| {
                let (style, lens) = initial();
                cx.new(|cx| Probe { focus: cx.focus_handle(), style, lens, drag: None })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
