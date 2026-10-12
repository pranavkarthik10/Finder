//! Liquid Glass probe: a Finder window in icon view, on an Applications
//! folder or the Windows wallpapers, with the toolbar's glass capsules
//! floating over content that scrolls beneath them.
//!
//! Keys: R/F refraction, E/W bezel, B/V blur, D dispersion, H highlight,
//! S shadow, T tint, M dark, C clear/regular, 0 reset; L shows a draggable
//! lens and I the current settings.
//!
//! GLASS="scroll=200,blur=0,lens=1,info=1,walls=1,search=1" sets the starting state, for
//! screenshots without driving the keyboard.

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use gpui::{
    AnimationExt, App, AssetSource, Bounds, Context, FocusHandle, FontWeight, GlassStyle, Hsla, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Point, Rgba,
    ScrollHandle, SharedString, TitlebarOptions, Window, WindowBounds, WindowControlArea,
    WindowOptions, canvas, div, hsla, img, point, prelude::*, px, rgb, rgba, size, svg,
};
use rust_embed::Embed;

const TOOLBAR_H: f32 = 52.;
const CAPSULE_H: f32 = 36.;
const SIDEBAR_W: f32 = 206.;
const HERO_H: f32 = 300.;
const APP_ICON: f32 = 64.;

/// The Applications folder: a name, a glyph, a hue, and whether the icon is
/// a light tile with a colored glyph rather than a colored tile.
const APPS: &[(&str, &str, f32, bool)] = &[
    ("App Store", "bag.fill", 0.58, false),
    ("Books", "book.fill", 0.08, false),
    ("Calculator", "textformat", 0.0, true),
    ("Calendar", "calendar", 0.0, true),
    ("Camera", "camera.fill", 0.0, true),
    ("Chess", "puzzlepiece.fill", 0.75, false),
    ("Clock", "clock.fill", 0.0, true),
    ("Compiler", "swift", 0.04, false),
    ("Contacts", "person.2.fill", 0.1, true),
    ("Fitness", "dumbbell.fill", 0.33, false),
    ("Freeform", "paintbrush.pointed.fill", 0.52, false),
    ("Games", "gamecontroller.fill", 0.97, false),
    ("Home", "house.fill", 0.09, true),
    ("Ideas", "lightbulb.fill", 0.13, false),
    ("Keychain", "key.fill", 0.12, false),
    ("Learn", "graduationcap.fill", 0.62, false),
    ("Mail", "envelope.fill", 0.58, false),
    ("Maps", "map.fill", 0.38, false),
    ("Messages", "message.fill", 0.36, false),
    ("Movies", "film.fill", 0.8, false),
    ("Music", "music.note", 0.98, false),
    ("News", "newspaper.fill", 0.99, true),
    ("Packages", "shippingbox.fill", 0.07, false),
    ("Phone", "phone.fill", 0.36, false),
    ("Photos", "photo.fill", 0.9, true),
    ("Podcasts", "headphones", 0.78, false),
    ("Quotes", "quote.bubble.fill", 0.68, false),
    ("Reminders", "checklist", 0.6, true),
    ("Safari", "safari.fill", 0.56, true),
    ("Security", "lock.fill", 0.0, false),
    ("Settings", "gearshape.fill", 0.6, true),
    ("Shortcuts", "sparkles", 0.72, false),
    ("Stocks", "chart.bar.fill", 0.6, false),
    ("Stopwatch", "stopwatch.fill", 0.03, false),
    ("System", "cpu.fill", 0.55, false),
    ("Timer", "timer", 0.08, true),
    ("Tools", "hammer.fill", 0.6, false),
    ("Travel", "airplane", 0.55, false),
    ("TV", "tv.fill", 0.0, false),
    ("Video", "video.fill", 0.35, false),
    ("Weather", "cloud.sun.fill", 0.57, false),
    ("World", "globe.americas.fill", 0.5, false),
    ("Wellness", "heart.fill", 0.97, true),
    ("Energy", "bolt.fill", 0.13, false),
    ("Garden", "leaf.fill", 0.3, false),
    ("Favorites", "star.fill", 0.12, false),
    ("Water", "drop.fill", 0.55, true),
    ("Hot", "flame.fill", 0.05, false),
];

#[derive(Clone, Copy, PartialEq)]
enum Folder {
    Applications,
    Wallpapers,
}

/// An app-style icon: a glossy rounded tile with a symbol on it.
fn app_icon(glyph: &str, hue: f32, light: bool) -> gpui::Div {
    let (top, bottom, ink) = if light {
        (hsla(0., 0., 1., 1.), hsla(0., 0., 0.93, 1.), hsla(hue, 0.85, 0.5, 1.))
    } else {
        (hsla(hue, 0.8, 0.62, 1.), hsla(hue, 0.85, 0.45, 1.), hsla(0., 0., 1., 1.))
    };
    div()
        .size(px(APP_ICON))
        .rounded(px(15.))
        .bg(gpui::linear_gradient(180., gpui::linear_color_stop(top, 0.), gpui::linear_color_stop(bottom, 1.)))
        .border(px(0.5))
        .border_color(hsla(0., 0., 0., 0.08))
        .shadow(vec![gpui::BoxShadow {
            color: hsla(0., 0., 0., 0.18),
            offset: point(px(0.), px(1.)),
            blur_radius: px(3.),
            spread_radius: px(0.),
            inset: false,
        }])
        .flex()
        .items_center()
        .justify_center()
        .child(sym(glyph, 26., ink))
}
const THUMB_W: f32 = 196.;
const THUMB_H: f32 = 124.;

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
    ("clock", "clock"),
    ("document", "document"),
    ("arrow.down.circle", "download"),
    ("menubar.dock.rectangle", "desktop"),
    ("internaldrive", "drive"),
    ("icloud", "cloud"),
    ("circle.fill", "circle"),
    ("xmark", "tl-close"),
    ("minus", "tl-min"),
    ("plus", "tl-max"),
];

fn sym(name: &str, pt: f32, color: impl Into<Hsla>) -> gpui::Svg {
    let sf = format!("icons/sf/{name}.svg");
    let path = if Assets::get(&sf).is_some() {
        sf
    } else {
        let base = name.split('@').next().unwrap_or(name);
        let file = STAND_INS.iter().find(|(n, _)| *n == base).map_or(base, |(_, f)| f);
        format!("icons/{file}.svg")
    };
    svg().path(path).size(px(pt * 1.25)).flex_none().text_color(color.into())
}

/// Finder's palette, light or dark.
struct Theme {
    dark: bool,
}

impl Theme {
    fn pick(&self, light: u32, dark: u32) -> Rgba {
        rgba(if self.dark { dark } else { light })
    }
    fn bg(&self) -> Rgba { self.pick(0xFFFFFFFF, 0x1E1E1EFF) }
    fn text(&self) -> Rgba { self.pick(0x000000DE, 0xFFFFFFE0) }
    fn secondary(&self) -> Rgba { self.pick(0x00000080, 0xFFFFFF8C) }
    fn separator(&self) -> Rgba { self.pick(0x00000017, 0xFFFFFF1A) }
    fn sidebar(&self) -> Rgba { self.pick(0xF4F4F6FF, 0x262628FF) }
    fn sidebar_header(&self) -> Rgba { self.pick(0x00000066, 0xFFFFFF66) }
    fn sidebar_icon(&self) -> Rgba { self.pick(0x000000B3, 0xFFFFFFB3) }
    fn selected(&self) -> Rgba { self.pick(0x0000000F, 0xFFFFFF1A) }
    fn accent(&self) -> Rgba { self.pick(0x0A7AFFFF, 0x3D9BFFFF) }
    fn glass_icon(&self) -> Hsla {
        if self.dark { hsla(0., 0., 1., 0.85) } else { hsla(0., 0., 0.2, 1.) }
    }
    fn pressed(&self) -> Hsla {
        if self.dark { hsla(0., 0., 1., 0.12) } else { hsla(0., 0., 0., 0.07) }
    }
}

/// A copy of `src` at most `edge` pixels on a side, cached in the temp folder.
fn cached(src: &Path, edge: u32) -> Option<PathBuf> {
    let cache = std::env::temp_dir().join("glass_probe_thumbs");
    let _ = std::fs::create_dir_all(&cache);
    let out = cache.join(format!("{edge}-{}", src.file_name()?.to_string_lossy()));
    if !out.exists() {
        image::open(src).ok()?.thumbnail(edge, edge).to_rgb8().save(&out).ok()?;
    }
    Some(out)
}

/// The banner: color and detail all the way to its top edge.
fn hero() -> Option<PathBuf> {
    cached(Path::new("C:/Windows/Web/Wallpaper/ThemeB/img27.jpg"), 2400)
}

/// Small copies of the wallpapers Windows ships.
fn photos() -> Vec<(SharedString, PathBuf)> {
    let mut sources: Vec<PathBuf> = ["Wallpaper/Windows", "Wallpaper/ThemeA", "Wallpaper/ThemeB", "Wallpaper/ThemeC", "Wallpaper/ThemeD", "Screen"]
        .iter()
        .flat_map(|dir| std::fs::read_dir(Path::new("C:/Windows/Web").join(dir)).into_iter().flatten())
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("jpg")))
        // Too small, and a flat color.
        .filter(|p| !p.ends_with("img50.jpg") && !p.ends_with("img105.jpg"))
        .collect();
    sources.sort();
    sources
        .into_iter()
        .filter_map(|src| Some((src.file_name()?.to_string_lossy().to_string().into(), cached(&src, 640)?)))
        .collect()
}

struct Probe {
    focus: FocusHandle,
    style: GlassStyle,
    folder: Folder,
    photos: Vec<(SharedString, PathBuf)>,
    hero: Option<PathBuf>,
    scroll: ScrollHandle,
    /// A starting scroll offset, applied once the content has been laid out.
    pending_scroll: Option<f32>,
    lens: Option<Point<Pixels>>,
    drag: Option<Point<Pixels>>,
    info: bool,
    /// Search starts as a round button and opens into a field.
    search_open: bool,
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

fn tool_button(name: &str, on: bool, theme: &Theme) -> gpui::Div {
    div()
        .h(px(CAPSULE_H - 8.))
        .min_w(px(38.))
        .px(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .when(on, |d| d.bg(theme.pressed()))
        .child(sym(name, 17., theme.glass_icon()))
}

impl Probe {
    fn open(&mut self, folder: Folder, cx: &mut Context<Self>) {
        self.folder = folder;
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

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
            "s" => s.shadow = if s.shadow > 0. { 0. } else { GlassStyle::default().shadow },
            "t" => s.tint = if s.tint.a > 0. { Hsla::transparent_black() } else { hsla(0.58, 0.9, 0.5, 0.6) },
            "m" => s.dark = !s.dark,
            "c" => s.blur = if s.blur > 0. { 0. } else { 7.33 },
            "0" => *s = GlassStyle::default(),
            "l" => self.lens = if self.lens.is_some() { None } else { Some(point(px(420.), px(260.))) },
            "i" => self.info = !self.info,
            "escape" => self.search_open = false,
            _ => return,
        }
        cx.notify();
    }

    fn sidebar(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = |title: &'static str| {
            div()
                .h(px(28.))
                .mt(px(8.))
                .px(px(10.))
                .flex()
                .items_end()
                .pb(px(6.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.sidebar_header())
                .child(title)
        };
        let row = |icon: &str, name: &'static str, on: bool| {
            let tint = if on { theme.accent() } else { theme.sidebar_icon() };
            div()
                .h(px(30.))
                .flex()
                .items_center()
                .gap(px(7.))
                .px(px(8.))
                .rounded(px(10.))
                .text_size(px(13.))
                .text_color(if on { theme.accent() } else { theme.text() })
                .when(on, |d| d.bg(theme.selected()))
                .child(div().w(px(22.)).flex().justify_center().child(sym(icon, 15., tint)))
                .child(name)
        };
        let tag = |hue: Option<f32>, name: &'static str| {
            let color = hue.map_or(hsla(0., 0., 0.6, 1.), |h| hsla(h, 0.85, 0.55, 1.));
            div()
                .h(px(30.))
                .flex()
                .items_center()
                .gap(px(7.))
                .px(px(8.))
                .text_size(px(13.))
                .text_color(theme.text())
                .child(div().w(px(22.)).flex().justify_center().child(div().size(px(10.)).rounded_full().bg(color)))
                .child(name)
        };
        let active = window.is_window_active();
        let light = |color: u32, glyph: &'static str, area: WindowControlArea| {
            div()
                .size(px(14.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(if active { rgb(color) } else { theme.pick(0xDDDDDDFF, 0x4A4A4CFF) })
                .border(px(0.5))
                .border_color(theme.pick(0x00000014, 0x00000040))
                .window_control_area(area)
                .child(
                    svg()
                        .path(format!("icons/{glyph}.svg"))
                        .size(px(11.))
                        .text_color(gpui::transparent_black())
                        .group_hover("lights", |s| s.text_color(rgba(0x00000099))),
                )
        };
        div()
            .relative()
            .w(px(SIDEBAR_W))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.sidebar())
            .border_r_1()
            .border_color(theme.separator())
            .child(div().h(px(TOOLBAR_H)).flex_none().window_control_area(WindowControlArea::Drag))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px(px(8.))
                    .child(header("Favorites").mt_0())
                    .child(row("airdrop", "AirDrop", false))
                    .child(row("clock", "Recents", false))
                    .child(row("applications", "Applications", self.folder == Folder::Applications).id("go-apps").on_click(
                        cx.listener(|this, _, _, cx| this.open(Folder::Applications, cx)),
                    ))
                    .child(row("menubar.dock.rectangle", "Desktop", false))
                    .child(row("document", "Documents", false))
                    .child(row("arrow.down.circle", "Downloads", false))
                    .child(row("folder", "Wallpapers", self.folder == Folder::Wallpapers).id("go-walls").on_click(
                        cx.listener(|this, _, _, cx| this.open(Folder::Wallpapers, cx)),
                    ))
                    .child(header("Locations"))
                    .child(row("icloud", "iCloud Drive", false))
                    .child(row("internaldrive", "Windows (C:)", false))
                    .child(header("Tags"))
                    .child(tag(Some(0.0), "Red"))
                    .child(tag(Some(0.08), "Orange"))
                    .child(tag(Some(0.33), "Green"))
                    .child(tag(Some(0.6), "Blue"))
                    .child(tag(None, "All Tags…")),
            )
            .child(
                div()
                    .group("lights")
                    .absolute()
                    .top(px((TOOLBAR_H - 14.) / 2.))
                    .left(px(19.))
                    .flex()
                    .gap(px(8.))
                    .child(light(0xFF5F57, "tl-close", WindowControlArea::Close))
                    .child(light(0xFEBC2E, "tl-min", WindowControlArea::Min))
                    .child(light(0x28C840, "tl-max", WindowControlArea::Max)),
            )
    }

    fn toolbar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.style;
        div()
            .absolute()
            .top_0()
            .left(px(SIDEBAR_W))
            .right_0()
            .h(px(TOOLBAR_H))
            .flex()
            .items_center()
            .px(px(12.))
            .gap(px(10.))
            .window_control_area(WindowControlArea::Drag)
            // Content scrolling under the toolbar sits behind a flat, milky
            // veil, so the title and capsules stay legible over it.
            .child({
                let color: Hsla = theme.bg().into();
                canvas(|_, _, _| (), move |bounds, _, window, _| {
                    window.paint_scroll_edge(bounds, color.opacity(0.45), 9.)
                })
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(TOOLBAR_H))
            })
            .child(div().absolute().bottom_0().left_0().right_0().h(px(1.)).bg(theme.separator()))
            .child(
                capsule(s)
                    .child(tool_button("chevron.left@medium", false, theme))
                    .child(div().w(px(1.)).h(px(16.)).bg(theme.separator()))
                    .child(tool_button("chevron.right@medium", false, theme).opacity(0.3)),
            )
            .child(
                div()
                    .flex_1()
                    .px(px(4.))
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text())
                    .child(match self.folder {
                        Folder::Applications => "Applications",
                        Folder::Wallpapers => "Wallpapers",
                    }),
            )
            .child(
                capsule(s)
                    .child(tool_button("square.grid.2x2", true, theme))
                    .child(tool_button("list.bullet", false, theme))
                    .child(tool_button("rectangle.split.3x1", false, theme))
                    .child(tool_button("squares.below.rectangle", false, theme)),
            )
            .child(
                capsule(s).child(
                    tool_button("square.grid.3x1.below.line.grid.1x2", false, theme)
                        .gap(px(1.))
                        .child(sym("chevron.down@semibold", 8., theme.glass_icon())),
                ),
            )
            .child(
                capsule(s)
                    .child(tool_button("square.and.arrow.up", false, theme))
                    .child(tool_button("tag", false, theme))
                    .child(tool_button("ellipsis", false, theme)),
            )
            .child(self.search(theme, cx))
    }

    /// A round glass button that springs open into the search field, as
    /// Finder's does when its window is narrow.
    fn search(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        const OPEN_W: f32 = 220.;
        let open = self.search_open;
        let (from, to) = if open { (CAPSULE_H, OPEN_W) } else { (OPEN_W, CAPSULE_H) };
        capsule(self.style)
            .id("search")
            .px(px(if open { 10. } else { 0. }))
            .gap(px(5.))
            .when(!open, |d| d.justify_center())
            .child(sym("magnifyingglass", if open { 13. } else { 15. }, if open { theme.secondary().into() } else { theme.glass_icon() }))
            // Only the label clips while the field grows: clipping the capsule
            // would cut off its glass rim and shadow too.
            .when(open, |d| {
                d.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(13.))
                        .text_color(theme.secondary())
                        .child("Search"),
                )
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.search_open = !this.search_open;
                cx.notify();
            }))
            .with_animation(
                ("search-anim", open as usize),
                gpui::Animation::new(std::time::Duration::from_millis(260)).with_easing(gpui::ease_out_quint()),
                move |el, t| el.w(px(from + (to - from) * t)),
            )
    }

    fn content(&self, theme: &Theme) -> gpui::AnyElement {
        match self.folder {
            Folder::Applications => self.applications(theme).into_any_element(),
            Folder::Wallpapers => self.wallpapers(theme).into_any_element(),
        }
    }

    fn applications(&self, theme: &Theme) -> impl IntoElement {
        let tiles = APPS.iter().map(|&(name, glyph, hue, light)| {
            div()
                .w(px(112.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(7.))
                .child(app_icon(glyph, hue, light))
                .child(div().text_size(px(12.)).text_color(theme.text()).child(name))
        });
        div()
            .id("content")
            .absolute()
            .top_0()
            .left(px(SIDEBAR_W))
            .right_0()
            .bottom_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pt(px(TOOLBAR_H + 22.))
            .child(div().flex().flex_wrap().gap_y(px(26.)).px(px(16.)).pb(px(32.)).children(tiles))
    }

    fn wallpapers(&self, theme: &Theme) -> impl IntoElement {
        let hero = self.hero.clone();
        let tiles = self.photos.iter().enumerate().map(|(i, (name, path))| {
            let on = i == 4;
            div()
                .w(px(THUMB_W + 16.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .p(px(4.))
                        .rounded(px(10.))
                        .when(on, |d| d.bg(theme.selected()))
                        .child(img(path.clone()).w(px(THUMB_W)).h(px(THUMB_H)).rounded(px(6.)).object_fit(ObjectFit::Cover)),
                )
                .child(
                    div()
                        .px(px(6.))
                        .rounded(px(4.))
                        .text_size(px(12.))
                        .when(on, |d| d.bg(theme.accent()).text_color(rgb(0xFFFFFF)))
                        .when(!on, |d| d.text_color(theme.text()))
                        .child(name.clone()),
                )
        });
        div()
            .id("content")
            .absolute()
            .top_0()
            .left(px(SIDEBAR_W))
            .right_0()
            .bottom_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pt(px(TOOLBAR_H))
            .children(hero.map(|p| {
                // An image sizes itself by its aspect ratio; pin it to the banner.
                div().mx(px(16.)).mt(px(8.)).h(px(HERO_H)).relative().child(
                    img(p).absolute().inset_0().size_full().rounded(px(14.)).object_fit(ObjectFit::Cover),
                )
            }))
            .child(
                div()
                    .px(px(20.))
                    .pt(px(18.))
                    .pb(px(6.))
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(theme.text()).child("All Photos"))
                    .child(div().text_size(px(12.)).text_color(theme.secondary()).child(format!("{} items", self.photos.len()))),
            )
            .child(div().flex().flex_wrap().justify_around().gap_y(px(18.)).px(px(8.)).pb(px(24.)).children(tiles))
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
        let theme = Theme { dark: style.dark };
        let info = format!(
            "refraction {:.0}  bezel {:.0}  blur {:.2}  dispersion {:.2}  highlight {:.0}  shadow {:.2}  tint {}  {}",
            style.refraction,
            style.bezel,
            style.blur,
            style.dispersion,
            style.highlight,
            style.shadow,
            if style.tint.a > 0. { "on" } else { "off" },
            if style.dark { "dark" } else { "light" },
        );

        div()
            .id("root")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, e: &MouseDownEvent, _, cx| {
                let (p, Some(l)) = (e.position, this.lens) else { return };
                if p.x >= l.x && p.x <= l.x + px(260.) && p.y >= l.y && p.y <= l.y + px(140.) {
                    this.drag = Some(p - l);
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if let Some(offset) = this.drag {
                    this.lens = Some(e.position - offset);
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, _| this.drag = None))
            .size_full()
            .relative()
            .bg(theme.bg())
            .font_family("SF Pro Text")
            .text_color(theme.text())
            .child(self.content(&theme))
            .child(self.toolbar(&theme, cx))
            .child(self.sidebar(&theme, window, cx))
            .children(self.lens.map(|l| div().absolute().left(l.x).top(l.y).w(px(260.)).h(px(140.)).child(glass(36., style))))
            .when(self.info, |d| {
                d.child(
                    div()
                        .absolute()
                        .bottom(px(10.))
                        .right(px(10.))
                        .px(px(10.))
                        .py(px(5.))
                        .rounded(px(8.))
                        .bg(rgba(0x000000B0))
                        .text_color(rgb(0xFFFFFF))
                        .text_size(px(11.))
                        .child(info),
                )
            })
    }
}

/// SF Pro from `%LOCALAPPDATA%\Finder\fonts`, as Finder loads it for local
/// testing (it may not be bundled).
fn load_local_fonts(cx: &mut App) {
    let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Finder").join("fonts")) else { return };
    let fonts: Vec<Cow<'static, [u8]>> = std::fs::read_dir(dir)
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

fn main() {
    let mut style = GlassStyle::default();
    let (mut scroll, mut lens, mut info, mut walls, mut search) = (0., None, false, false, false);
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
            "scroll" => scroll = v,
            "lens" => lens = (v > 0.).then(|| point(px(420.), px(260.))),
            "info" => info = v > 0.,
            "walls" => walls = v > 0.,
            "search" => search = v > 0.,
            _ => {}
        }
    }
    let (photos, hero) = (photos(), hero());

    gpui_platform::application().with_assets(Assets).run(move |cx: &mut App| {
        load_local_fonts(cx);
        let bounds = Bounds::centered(None, size(px(1000.), px(620.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions { title: Some("Wallpapers".into()), appears_transparent: true, traffic_light_position: None }),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| Probe {
                    focus: cx.focus_handle(),
                    folder: if walls { Folder::Wallpapers } else { Folder::Applications },
                    style,
                    photos,
                    hero,
                    scroll: ScrollHandle::new(),
                    pending_scroll: (scroll > 0.).then_some(scroll),
                    lens,
                    drag: None,
                    info,
                    search_open: search,
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
