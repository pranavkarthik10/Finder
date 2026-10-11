//! Platform glue. Everything else works on paths as bytes, '/'-separated,
//! under one root "/". On macOS that is the real path. On Windows the
//! drives are the root's children: `C:\Users\me` is "/C:/Users/me", and
//! names are UTF-8 (converted from UTF-16 when listed).

use std::path::{Path, PathBuf};

/// How urgently a thread's work is wanted.
#[derive(Clone, Copy)]
pub enum Qos {
    /// Someone is waiting on it right now (a search).
    Interactive,
    /// Someone asked and will wait for it (a first build).
    Initiated,
    /// Background upkeep.
    Utility,
}

#[cfg(target_os = "macos")]
pub fn set_qos(q: Qos) {
    let c = match q {
        Qos::Interactive => libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE,
        Qos::Initiated => libc::qos_class_t::QOS_CLASS_USER_INITIATED,
        Qos::Utility => libc::qos_class_t::QOS_CLASS_UTILITY,
    };
    unsafe { libc::pthread_set_qos_class_self_np(c, 0) };
}

#[cfg(windows)]
pub fn set_qos(q: Qos) {
    use windows_sys::Win32::System::Threading::*;
    let p = match q {
        Qos::Interactive => THREAD_PRIORITY_ABOVE_NORMAL,
        Qos::Initiated => THREAD_PRIORITY_NORMAL,
        // Background mode lowers IO priority too, like utility QoS.
        Qos::Utility => THREAD_MODE_BACKGROUND_BEGIN,
    };
    unsafe { SetThreadPriority(GetCurrentThread(), p) };
}

/// Never let indexing download cloud placeholders (iCloud): on the calling
/// thread, opening or listing a dataless file fails fast instead of
/// materializing it. (Windows: content reads skip placeholders instead,
/// see `content::open_sized`.)
pub fn no_materialize() {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn setiopolicy_np(iotype: i32, scope: i32, policy: i32) -> i32;
        }
        // IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES, IOPOL_SCOPE_THREAD, OFF
        unsafe { setiopolicy_np(3, 1, 1) };
    }
}

/// Hand freed allocator memory back to the OS after big transient work
/// (index builds, content batches) instead of letting malloc cache it.
pub fn release_memory() {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
    }
    // Windows' heap already returns big blocks (VirtualAlloc'd) on free.
}

/// One-writer locks on the index files; held until the process exits.
pub fn try_lock(f: &std::fs::File) -> bool {
    f.try_lock().is_ok()
}

/// An index path as an OS path.
#[cfg(unix)]
pub fn os_path(p: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(p))
}

/// An OS path as an index path.
#[cfg(unix)]
pub fn index_path(p: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    p.as_os_str().as_bytes().to_vec()
}

/// An index path as an OS path: "/C:/a/b" is `C:\a\b`. The root ("/") has
/// none; it comes back empty.
#[cfg(windows)]
pub fn os_path(p: &[u8]) -> PathBuf {
    let s = String::from_utf8_lossy(p);
    let s = s.strip_prefix('/').unwrap_or(&s);
    let mut out = s.replace('/', "\\");
    // A bare drive ("C:") is its current directory; its root is "C:\".
    if out.len() == 2 && out.ends_with(':') {
        out.push('\\');
    }
    PathBuf::from(out)
}

/// An OS path as an index path: `C:\a\b` (or `\\?\C:\a\b`) is "/C:/a/b".
/// The drive letter is upper-cased; other paths pass through with
/// separators turned around.
#[cfg(windows)]
pub fn index_path(p: &Path) -> Vec<u8> {
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    let mut out = Vec::with_capacity(s.len() + 1);
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        out.push(b'/');
        out.push(b[0].to_ascii_uppercase());
        out.push(b':');
        out.extend(b[2..].iter().map(|&c| if c == b'\\' { b'/' } else { c }));
    } else {
        out.extend(b.iter().map(|&c| if c == b'\\' { b'/' } else { c }));
    }
    while out.len() > 1 && out.last() == Some(&b'/') {
        out.pop();
    }
    if out.is_empty() {
        out.push(b'/');
    }
    out
}

/// Same as `index_path`, for strings typed by people (`in:C:\Users`).
pub fn index_path_str(s: &str) -> String {
    String::from_utf8_lossy(&index_path(Path::new(s))).into_owned()
}

/// The user's home folder, as an index path.
pub fn home() -> String {
    #[cfg(unix)]
    let h = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    #[cfg(windows)]
    let h = std::env::var("USERPROFILE").map_or_else(|_| "/".into(), |h| index_path_str(&h));
    h
}

/// Where the index lives (`FSEARCH_DIR` overrides).
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("FSEARCH_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(target_os = "macos")]
    let d = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into())).join("Library/Application Support/FSearch");
    #[cfg(windows)]
    let d = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into())).join("FSearch");
    d
}

/// Seconds since the epoch of a Windows FILETIME (100 ns since 1601).
#[cfg(windows)]
pub fn filetime_secs(ft: i64) -> u32 {
    (ft / 10_000_000 - 11_644_473_600).clamp(0, u32::MAX as i64) as u32
}
