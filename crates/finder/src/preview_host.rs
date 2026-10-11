//! Windows preview handlers (what Explorer's preview pane shows) hosted for
//! Quick Look: PDFs page by page, videos and audio with playback controls,
//! Office documents, fonts. A handler draws into a window; GPUI draws its
//! DirectComposition layer above child windows, so the host is a
//! borderless popup owned by our window, placed over Quick Look's body.
//! It never takes focus, so Space and Escape still reach Quick Look.

use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, CLSIDFromString, CoCreateInstance, STGM_READ, STGM_SHARE_DENY_NONE};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream};
use windows::Win32::UI::Shell::{
    ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_SHELLEXTENSION, AssocQueryStringW, IInitializeWithItem, IPreviewHandler, IShellItem, SHCreateItemFromParsingName,
    SHCreateStreamOnFileEx,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, HWND_TOP, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos, WS_CLIPCHILDREN, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_POPUP,
};
use windows::core::{HSTRING, Interface, PWSTR, w};

/// The shell extension point for preview handlers.
const PREVIEW_HANDLER: &str = "{8895b1c6-b41f-4c1c-a562-0d564250836f}";

/// The CLSID of the preview handler registered for `path`'s type.
fn handler_for(path: &Path) -> Option<windows::core::GUID> {
    let ext = format!(".{}", path.extension()?.to_string_lossy());
    let mut buf = [0u16; 64];
    let mut len = buf.len() as u32;
    unsafe {
        AssocQueryStringW(ASSOCF_INIT_DEFAULTTOSTAR, ASSOCSTR_SHELLEXTENSION, &HSTRING::from(ext), &HSTRING::from(PREVIEW_HANDLER), Some(PWSTR(buf.as_mut_ptr())), &mut len)
            .ok()
            .ok()?;
        CLSIDFromString(PWSTR(buf.as_mut_ptr())).ok()
    }
}

/// Whether Windows can preview `path` with a handler.
pub fn can_preview(path: &Path) -> bool {
    handler_for(path).is_some()
}

pub struct PreviewHost {
    pub path: PathBuf,
    popup: HWND,
    handler: IPreviewHandler,
    rect: RECT,
}

/// `r` (client pixels of `owner`) in screen pixels.
fn to_screen(owner: HWND, r: RECT) -> RECT {
    let mut p = POINT { x: 0, y: 0 };
    unsafe {
        let _ = ClientToScreen(owner, &mut p);
    }
    RECT { left: r.left + p.x, top: r.top + p.y, right: r.right + p.x, bottom: r.bottom + p.y }
}

impl PreviewHost {
    /// Preview `path` over `rect` (client pixels of `owner`). None if no
    /// handler takes it.
    pub fn open(owner: HWND, path: &Path, rect: RECT) -> Option<PreviewHost> {
        let clsid = handler_for(path)?;
        unsafe {
            // Most handlers run out of process (prevhost.exe), as Explorer runs them.
            let handler: IPreviewHandler =
                CoCreateInstance(&clsid, None, CLSCTX_LOCAL_SERVER).or_else(|_| CoCreateInstance(&clsid, None, CLSCTX_INPROC_SERVER)).ok()?;
            let hpath = HSTRING::from(path.as_os_str());
            let ok = if let Ok(i) = handler.cast::<IInitializeWithFile>() {
                i.Initialize(&hpath, STGM_READ.0).is_ok()
            } else if let Ok(i) = handler.cast::<IInitializeWithItem>() {
                let item: IShellItem = SHCreateItemFromParsingName(&hpath, None).ok()?;
                i.Initialize(&item, STGM_READ.0).is_ok()
            } else if let Ok(i) = handler.cast::<IInitializeWithStream>() {
                let stream = SHCreateStreamOnFileEx(&hpath, (STGM_READ | STGM_SHARE_DENY_NONE).0, 0, false, None).ok()?;
                i.Initialize(&stream, STGM_READ.0).is_ok()
            } else {
                false
            };
            if !ok {
                return None;
            }
            let s = to_screen(owner, rect);
            let popup = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                None,
                WS_POPUP | WS_CLIPCHILDREN,
                s.left,
                s.top,
                s.right - s.left,
                s.bottom - s.top,
                Some(owner),
                None,
                None,
                None,
            )
            .ok()?;
            let _ = SetWindowPos(popup, Some(HWND_TOP), s.left, s.top, s.right - s.left, s.bottom - s.top, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            let inner = RECT { left: 0, top: 0, right: s.right - s.left, bottom: s.bottom - s.top };
            if handler.SetWindow(popup, &inner).is_err() || handler.DoPreview().is_err() {
                let _ = handler.Unload();
                let _ = DestroyWindow(popup);
                return None;
            }
            Some(PreviewHost { path: path.to_path_buf(), popup, handler, rect })
        }
    }

    /// Keep over `rect` (client pixels of `owner`) as the window moves.
    pub fn place(&mut self, owner: HWND, rect: RECT) {
        let s = to_screen(owner, rect);
        unsafe {
            let _ = SetWindowPos(self.popup, Some(HWND_TOP), s.left, s.top, s.right - s.left, s.bottom - s.top, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            if rect != self.rect {
                let _ = self.handler.SetRect(&RECT { left: 0, top: 0, right: s.right - s.left, bottom: s.bottom - s.top });
            }
        }
        self.rect = rect;
    }
}

impl Drop for PreviewHost {
    fn drop(&mut self) {
        unsafe {
            let _ = self.handler.Unload();
            let _ = DestroyWindow(self.popup);
        }
    }
}
