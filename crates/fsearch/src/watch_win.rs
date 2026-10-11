//! Whole-disk change stream on Windows, at directory granularity like
//! FSEvents: one recursive ReadDirectoryChangesW per indexed drive, each
//! change reported as the folder whose listing it touched.
//!
//! There is no history to replay without admin rights (the USN journal
//! needs a volume handle), so a stream started from an older id reports
//! "/" as must-scan: the engine then relists the folders changed since its
//! last sync, by mtime. Event ids are microseconds since the epoch.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_ATTRIBUTES,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_INFORMATION,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadDirectoryChangesW,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects};

pub const MUST_SCAN_SUBDIRS: u32 = 0x1;
pub const USER_DROPPED: u32 = 0x2;
pub const KERNEL_DROPPED: u32 = 0x4;
pub const HISTORY_DONE: u32 = 0x10;

pub struct Event {
    pub path: Vec<u8>,
    pub flags: u32,
    pub id: u64,
    /// Which `watch` delivered it (its `stream` number).
    pub stream: u32,
}

fn now_us() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64)
}

/// When this process started watching: ids from before it have no history.
fn epoch() -> u64 {
    static E: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *E.get_or_init(now_us)
}

/// The newest event id on the system.
pub fn current_id() -> u64 {
    epoch();
    now_us()
}

/// A running stream; dropping it stops it.
pub struct Stream {
    stop: Arc<AtomicBool>,
    /// Manual-reset events that wake each drive's thread to stop.
    wake: Vec<usize>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for &e in &self.wake {
            unsafe { SetEvent(e as HANDLE) };
        }
    }
}

/// Watch every drive from `since` (an event id). Batches of directory-level
/// events, stamped with `stream`, arrive on `tx` until the returned stream
/// is dropped. `latency` is unused: each read already batches.
pub fn watch(since: u64, _latency: f64, tx: Sender<Vec<Event>>, stream: u32) -> Stream {
    let stop = Arc::new(AtomicBool::new(false));
    let mut wake = Vec::new();
    let mut first = Vec::new();
    if since < epoch() {
        // What changed while nobody watched is unknown.
        first.push(Event { path: b"/".to_vec(), flags: MUST_SCAN_SUBDIRS, id: now_us(), stream });
    }
    first.push(Event { path: b"/".to_vec(), flags: HISTORY_DONE, id: now_us(), stream });
    for d in crate::walk::drives() {
        let ev = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        wake.push(ev as usize);
        let (stop, tx) = (stop.clone(), tx.clone());
        let ev = ev as usize;
        let _ = std::thread::Builder::new().name(format!("fsearch-watch-{}", d as char)).spawn(move || {
            crate::sys::set_qos(crate::sys::Qos::Initiated);
            follow(d, ev as HANDLE, &stop, &tx, stream);
            unsafe { CloseHandle(ev as HANDLE) };
        });
    }
    // After the threads are reading: nothing between now and then is lost.
    let _ = tx.send(first);
    Stream { stop, wake }
}

const FILTER: u32 = FILE_NOTIFY_CHANGE_FILE_NAME
    | FILE_NOTIFY_CHANGE_DIR_NAME
    | FILE_NOTIFY_CHANGE_SIZE
    | FILE_NOTIFY_CHANGE_LAST_WRITE
    | FILE_NOTIFY_CHANGE_ATTRIBUTES;

/// Follow drive `d` until `stop`; `wake` is signalled to stop.
fn follow(d: u8, wake: HANDLE, stop: &AtomicBool, tx: &Sender<Vec<Event>>, stream: u32) {
    let root: Vec<u16> = format!(r"\\?\{}:\", d as char).encode_utf16().chain([0]).collect();
    let h = unsafe {
        CreateFileW(
            root.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return;
    }
    let done = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
    // u64s for the entries' 4-byte (DWORD) alignment and then some.
    let mut buf = vec![0u64; 1 << 17];
    let prefix = [b'/', d, b':'];
    while !stop.load(Ordering::SeqCst) {
        let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
        ov.hEvent = done;
        let ok = unsafe { ReadDirectoryChangesW(h, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32, 1, FILTER, std::ptr::null_mut(), &mut ov, None) };
        if ok == 0 {
            break;
        }
        let which = unsafe { WaitForMultipleObjects(2, [done, wake].as_ptr(), 0, INFINITE) };
        let mut n = 0u32;
        if which != WAIT_OBJECT_0 {
            unsafe {
                CancelIoEx(h, &ov);
                GetOverlappedResult(h, &ov, &mut n, 1);
            }
            break;
        }
        if unsafe { GetOverlappedResult(h, &ov, &mut n, 0) } == 0 {
            break;
        }
        let id = now_us();
        if n == 0 {
            // The buffer overflowed: changes were lost.
            let _ = tx.send(vec![Event { path: b"/".to_vec(), flags: MUST_SCAN_SUBDIRS | KERNEL_DROPPED, id, stream }]);
            continue;
        }
        let mut dirs: Vec<Vec<u8>> = Vec::new();
        let base = buf.as_ptr() as *const u8;
        let mut off = 0usize;
        loop {
            // Safety: the read filled `buf` with a chain of entries.
            let info = unsafe { &*(base.add(off) as *const FILE_NOTIFY_INFORMATION) };
            let name = unsafe { std::slice::from_raw_parts(std::ptr::addr_of!(info.FileName) as *const u16, info.FileNameLength as usize / 2) };
            let mut p = prefix.to_vec();
            if !name.is_empty() {
                p.push(b'/');
                crate::walk::push_utf8(&mut p, name);
                for c in &mut p[3..] {
                    if *c == b'\\' {
                        *c = b'/';
                    }
                }
            }
            // The folder whose listing changed: the entry's parent.
            let cut = p.iter().rposition(|&c| c == b'/').unwrap_or(0).max(3);
            p.truncate(cut);
            if dirs.last() != Some(&p) {
                dirs.push(p);
            }
            if info.NextEntryOffset == 0 {
                break;
            }
            off += info.NextEntryOffset as usize;
        }
        dirs.sort();
        dirs.dedup();
        let _ = tx.send(dirs.into_iter().map(|path| Event { path, flags: 0, id, stream }).collect());
    }
    unsafe {
        CloseHandle(done);
        CloseHandle(h);
    }
}
