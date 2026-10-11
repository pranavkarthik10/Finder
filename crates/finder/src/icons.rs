//! Real Windows icons and thumbnails, via the shell's IShellItemImageFactory
//! (what Explorer uses): app icons for .exe/.lnk, thumbnails for images,
//! videos and PDFs, the registered icon for everything else.

use gpui::RenderImage;
use std::path::Path;
use std::sync::Arc;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, GetObjectW, HGDIOBJ,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT};
use windows::core::HSTRING;

/// What an icon is cached by: per file where files differ (apps, links,
/// anything with a thumbnail), else per extension.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Path(std::path::PathBuf, u32),
    Ext(String, u32),
}

const OWN_ICON: &[&str] = &["exe", "lnk", "url", "ico", "msi", "appref-ms", "cpl", "scr"];
const THUMBNAIL: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "heic", "tif", "tiff", "mp4", "mov", "mkv", "avi", "webm", "m4v", "pdf", "psd", "svg",
];

fn ext(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Whether this file shows its own picture rather than its kind's icon.
pub fn has_thumbnail(path: &Path) -> bool {
    THUMBNAIL.contains(&ext(path).as_str())
}

pub fn key(path: &Path, px: u32) -> Key {
    let e = ext(path);
    if OWN_ICON.contains(&e.as_str()) || THUMBNAIL.contains(&e.as_str()) {
        Key::Path(path.to_path_buf(), px)
    } else {
        Key::Ext(e, px)
    }
}

/// The shell's picture of `path`, `px` pixels square at most. Blocking:
/// call it off the UI thread.
pub fn load(path: &Path, px: u32) -> Option<Arc<RenderImage>> {
    unsafe {
        // Per thread; repeat calls are cheap no-ops.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let f: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        let flags: SIIGBF = if has_thumbnail(path) { SIIGBF_RESIZETOFIT } else { SIIGBF_ICONONLY };
        let hbmp = f.GetImage(SIZE { cx: px as i32, cy: px as i32 }, flags).ok()?;
        let obj = HGDIOBJ(hbmp.0);
        let mut bm = BITMAP::default();
        GetObjectW(obj, size_of::<BITMAP>() as i32, Some((&raw mut bm).cast()));
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        if w <= 0 || h <= 0 {
            let _ = DeleteObject(obj);
            return None;
        }
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let rows = GetDIBits(dc, hbmp, 0, h as u32, Some(buf.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        let _ = DeleteObject(obj);
        if rows == 0 {
            return None;
        }
        // BGRA, premultiplied; GPUI wants it straight. A bitmap with no
        // alpha at all (some thumbnails) is opaque.
        if buf.chunks_exact(4).all(|p| p[3] == 0) {
            buf.chunks_exact_mut(4).for_each(|p| p[3] = 255);
        } else {
            for p in buf.chunks_exact_mut(4) {
                let a = p[3] as u32;
                if a > 0 && a < 255 {
                    for c in &mut p[..3] {
                        *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
        }
        let img = image::RgbaImage::from_raw(w as u32, h as u32, buf)?;
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(img)])))
    }
}
