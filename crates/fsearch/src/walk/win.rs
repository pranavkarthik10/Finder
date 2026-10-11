//! Parallel whole-disk enumeration on Windows.
//!
//! Each folder is opened once and read with GetFileInformationByHandleEx
//! (FileFullDirectoryInfo): one call returns hundreds of entries with size,
//! mtime and attributes already attached, so there is no per-file stat (the
//! getattrlistbulk of NTFS). Directories fan out over a rayon pool. The root
//! "/" lists the local drives ("C:", ...). Reparse points that stand for
//! another path (junctions, symlinks, mounted folders) are links, never
//! followed; cloud placeholders (OneDrive) are ordinary folders.

use super::*;
use rayon::Scope;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FULL_DIR_INFO,
    FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileFullDirectoryInfo, GetDriveTypeW, GetFileInformationByHandleEx,
    GetLogicalDrives, OPEN_EXISTING,
};

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
/// Reparse tag bit: the reparse point names another file (junction,
/// symlink, mount point) rather than holding data (cloud, dedup, WCI).
const TAG_NAME_SURROGATE: u32 = 0x2000_0000;

struct Ctx {
    next_id: AtomicU32,
    out: Vec<Mutex<Vec<Listing>>>,
}

thread_local! {
    // u64s: FILE_FULL_DIR_INFO needs 8-byte alignment.
    static BUF: RefCell<Vec<u64>> = RefCell::new(vec![0u64; 256 * 1024 / 8]);
}

/// Letters of the local drives the index covers (fixed and removable).
pub fn drives() -> Vec<u8> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| b'A' + i)
        .filter(|&l| {
            let root = [l as u16, b':' as u16, b'\\' as u16, 0];
            matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, DRIVE_FIXED | DRIVE_REMOVABLE)
        })
        .collect()
}

/// The root's listing: one folder per drive.
fn drive_listing(id: u32) -> Listing {
    let mut l = Listing { id, names: Vec::new(), ents: Vec::new() };
    for d in drives() {
        l.ents.push(RawEnt { name_off: l.names.len() as u32, name_len: 2, kind: KIND_DIR, size: 0, mtime: 0, child: NONE });
        l.names.extend_from_slice(&[d, b':']);
    }
    l
}

/// `\\?\` + the OS path: no MAX_PATH limit, no path parsing surprises.
fn wide(p: &[u8]) -> Vec<u16> {
    let os = crate::sys::os_path(p);
    r"\\?\".encode_utf16().chain(os.to_string_lossy().encode_utf16()).collect()
}

/// Scan `root` recursively. Listing id 0 is `root` itself.
pub fn scan(root: &[u8], threads: usize) -> Vec<Listing> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    let ctx = Ctx { next_id: AtomicU32::new(1), out: (0..threads + 1).map(|_| Mutex::new(Vec::new())).collect() };
    pool.scope(|s| {
        if root != b"/" {
            return finish_dir(s, wide(root), 0, &ctx);
        }
        let mut l = drive_listing(0);
        let mut kids = Vec::new();
        for e in l.ents.iter_mut() {
            e.child = ctx.next_id.fetch_add(1, Ordering::Relaxed);
            let name = &l.names[e.name_off as usize..][..2];
            kids.push((wide(&[b"/", name].concat()), e.child));
        }
        push(l, &ctx);
        for (path, id) in kids {
            let ctx = &ctx;
            s.spawn(move |s| finish_dir(s, path, id, ctx));
        }
    });
    let parts: Vec<Vec<Listing>> = ctx.out.into_iter().map(|m| m.into_inner().unwrap()).collect();
    let mut out = Vec::with_capacity(parts.iter().map(Vec::len).sum());
    for p in parts {
        out.extend(p);
    }
    out
}

/// List a single directory (no recursion). Subdirectories come back with
/// `child == NONE`. Used by the live updater.
pub fn list_one(path: &[u8]) -> Option<Listing> {
    if path == b"/" {
        return Some(drive_listing(0));
    }
    if blocked(path) {
        return None;
    }
    let h = open_dir(&wide(path))?;
    let mut l = Listing { id: 0, names: Vec::new(), ents: Vec::new() };
    list_handle(h, &mut l, |_, _| {});
    unsafe { CloseHandle(h) };
    Some(l)
}

fn finish_dir<'s>(s: &Scope<'s>, path: Vec<u16>, id: u32, ctx: &'s Ctx) {
    let mut l = Listing { id, names: Vec::new(), ents: Vec::new() };
    let mut kids = Vec::new();
    if let Some(h) = open_dir(&path) {
        list_handle(h, &mut l, |e, name| {
            e.child = ctx.next_id.fetch_add(1, Ordering::Relaxed);
            let mut p = Vec::with_capacity(path.len() + 1 + name.len());
            p.extend_from_slice(&path);
            if p.last() != Some(&(b'\\' as u16)) {
                p.push(b'\\' as u16);
            }
            p.extend_from_slice(name);
            kids.push((p, e.child));
        });
        unsafe { CloseHandle(h) };
    }
    push(l, ctx);
    for (p, cid) in kids {
        s.spawn(move |s| finish_dir(s, p, cid, ctx));
    }
}

fn push(l: Listing, ctx: &Ctx) {
    LISTED.fetch_add(l.ents.len(), Ordering::Relaxed);
    let slot = rayon::current_thread_index().unwrap_or(ctx.out.len() - 1);
    ctx.out[slot].lock().unwrap().push(l);
}

fn open_dir(path: &[u16]) -> Option<HANDLE> {
    let mut z = Vec::with_capacity(path.len() + 1);
    z.extend_from_slice(path);
    z.push(0);
    let h = unsafe {
        CreateFileW(
            z.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    (h != INVALID_HANDLE_VALUE).then_some(h)
}

/// Read every entry of the open directory `h` into `l`. `dir` sees each
/// subdirectory to descend into, with its UTF-16 name.
fn list_handle(h: HANDLE, l: &mut Listing, mut dir: impl FnMut(&mut RawEnt, &[u16])) {
    BUF.with_borrow_mut(|buf| {
        loop {
            let ok = unsafe { GetFileInformationByHandleEx(h, FileFullDirectoryInfo, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32) };
            if ok == 0 {
                break; // ERROR_NO_MORE_FILES, or unreadable
            }
            let base = buf.as_ptr() as *const u8;
            let mut off = 0usize;
            loop {
                // Safety: the call filled `buf` with a chain of entries, each
                // 8-aligned, linked by NextEntryOffset.
                let info = unsafe { &*(base.add(off) as *const FILE_FULL_DIR_INFO) };
                let name = unsafe { std::slice::from_raw_parts(std::ptr::addr_of!(info.FileName) as *const u16, info.FileNameLength as usize / 2) };
                if name != [b'.' as u16] && name != [b'.' as u16, b'.' as u16] {
                    parse_entry(info, name, l, &mut dir);
                }
                if info.NextEntryOffset == 0 {
                    break;
                }
                off += info.NextEntryOffset as usize;
            }
        }
    });
}

fn parse_entry(info: &FILE_FULL_DIR_INFO, name: &[u16], l: &mut Listing, dir: &mut impl FnMut(&mut RawEnt, &[u16])) {
    let attr = info.FileAttributes;
    let mut kind = if attr & FILE_ATTRIBUTE_DIRECTORY != 0 { KIND_DIR } else { KIND_FILE };
    // For a reparse point, EaSize holds its tag.
    if attr & FILE_ATTRIBUTE_REPARSE_POINT != 0 && info.EaSize & TAG_NAME_SURROGATE != 0 {
        kind = KIND_LINK;
    }
    if attr & FILE_ATTRIBUTE_HIDDEN != 0 {
        kind |= FLAG_HIDDEN;
    }
    let size = if kind & 3 == KIND_FILE { info.EndOfFile.max(0) as u64 } else { 0 };
    let off = l.names.len();
    push_utf8(&mut l.names, name);
    let len = l.names.len() - off;
    if len == 0 || len > u16::MAX as usize {
        l.names.truncate(off);
        return;
    }
    let mut e = RawEnt { name_off: off as u32, name_len: len as u16, kind, size, mtime: crate::sys::filetime_secs(info.LastWriteTime), child: NONE };
    if kind & 3 == KIND_DIR {
        dir(&mut e, name);
    }
    l.ents.push(e);
}

/// UTF-16 to UTF-8 (unpaired surrogates become U+FFFD), ASCII fast.
pub(crate) fn push_utf8(out: &mut Vec<u8>, w: &[u16]) {
    if w.iter().all(|&c| c < 0x80) {
        out.extend(w.iter().map(|&c| c as u8));
        return;
    }
    for c in char::decode_utf16(w.iter().copied()) {
        let mut b = [0u8; 4];
        out.extend_from_slice(c.unwrap_or('\u{FFFD}').encode_utf8(&mut b).as_bytes());
    }
}
