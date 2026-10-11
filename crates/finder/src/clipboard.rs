//! Files on the Windows clipboard, as Explorer puts them there: CF_HDROP
//! plus "Preferred DropEffect" (copy or move), so copying in one and
//! pasting in the other works both ways.

use std::path::PathBuf;
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::{DROPFILES, DragQueryFileW, HDROP};
use windows::core::w;

const DROPEFFECT_COPY: u32 = 1;
const DROPEFFECT_MOVE: u32 = 2;

fn drop_effect_format() -> u32 {
    unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) }
}

/// Open the clipboard for the duration of `f`.
fn with_clipboard<T>(f: impl FnOnce() -> T) -> Option<T> {
    // Another app may hold it for a moment.
    for _ in 0..10 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            let r = f();
            let _ = unsafe { CloseClipboard() };
            return Some(r);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

/// A movable global block holding `bytes`.
fn global(bytes: &[u8]) -> Option<HGLOBAL> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len()).ok()?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(h);
        Some(h)
    }
}

/// Put files on the clipboard to be copied, or moved when `cut`.
pub fn put_files(paths: &[PathBuf], cut: bool) -> Result<(), String> {
    // DROPFILES, then the paths as wide strings, each NUL-ended, then NUL.
    let header = DROPFILES { pFiles: size_of::<DROPFILES>() as u32, fWide: true.into(), ..Default::default() };
    let mut bytes: Vec<u8> = unsafe { std::slice::from_raw_parts((&raw const header).cast::<u8>(), size_of::<DROPFILES>()) }.to_vec();
    for p in paths {
        for u in p.as_os_str().to_string_lossy().encode_utf16().chain([0]) {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&[0, 0]);
    let effect = if cut { DROPEFFECT_MOVE } else { DROPEFFECT_COPY };
    let (files, effect) = (global(&bytes).ok_or("out of memory")?, global(&effect.to_le_bytes()).ok_or("out of memory")?);
    with_clipboard(|| unsafe {
        EmptyClipboard().map_err(|e| e.to_string())?;
        // The clipboard owns the blocks once set.
        SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(files.0))).map_err(|e| e.to_string())?;
        SetClipboardData(drop_effect_format(), Some(HANDLE(effect.0))).map_err(|e| e.to_string())?;
        Ok(())
    })
    .unwrap_or_else(|| Err("The clipboard is in use by another app.".into()))
}

/// Files on the clipboard, and whether they were cut (to be moved).
pub fn get_files() -> Option<(Vec<PathBuf>, bool)> {
    with_clipboard(|| unsafe {
        let h = GetClipboardData(CF_HDROP.0 as u32).ok()?;
        let drop = HDROP(h.0);
        let n = DragQueryFileW(drop, u32::MAX, None);
        let mut paths = Vec::with_capacity(n as usize);
        for i in 0..n {
            let len = DragQueryFileW(drop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(drop, i, Some(&mut buf));
            paths.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
        }
        let cut = GetClipboardData(drop_effect_format()).ok().is_some_and(|e| {
            let p = GlobalLock(HGLOBAL(e.0)) as *const u32;
            let v = if p.is_null() { 0 } else { *p };
            let _ = GlobalUnlock(HGLOBAL(e.0));
            v & DROPEFFECT_MOVE != 0
        });
        Some((paths, cut))
    })
    .flatten()
}

/// After a cut is pasted, the files are gone from where they were.
pub fn clear() {
    with_clipboard(|| unsafe { EmptyClipboard() });
}
