//! Whole-volume listing straight from the NTFS master file table: read the
//! $MFT's extents off the raw volume in big sequential chunks and parse
//! every file record in parallel, instead of opening every folder. Every
//! record carries its names (with parent), size, mtime and attributes, so
//! one pass yields the whole tree. Needs a volume handle, i.e. admin rights;
//! `scan_volume` returns None without them (or on anything but NTFS) and the
//! caller falls back to listing folders.

use super::*;
use rayon::prelude::*;
use std::os::windows::fs::FileExt;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{FSCTL_GET_NTFS_VOLUME_DATA, NTFS_VOLUME_DATA_BUFFER};

const ROOT: u32 = 5;
/// Records below this are NTFS metadata ($MFT, $LogFile, ..., $Extend);
/// only the root folder among them is shown.
const FIRST_USER: u32 = 16;
const CHUNK: usize = 8 << 20;

const ATTR_STANDARD_INFORMATION: u32 = 0x10;
const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;
const ATTR_REPARSE_POINT: u32 = 0xC0;
const ATTR_END: u32 = 0xFFFF_FFFF;
const NAMESPACE_DOS: u8 = 2;
const TAG_NAME_SURROGATE: u32 = 0x2000_0000;

/// What one record says about its file (an extension record's findings
/// are folded into its base record's).
#[derive(Clone, Copy, Default)]
struct Info {
    used: bool,
    dir: bool,
    seq: u16,
    attrs: u32,
    mtime: u32,
    size: u64,
    tag: u32,
}

/// One name of a file: (record, parent record, parent's sequence number,
/// name offset and length in the chunk's name bytes).
#[derive(Clone, Copy)]
struct Name {
    rec: u32,
    parent: u32,
    pseq: u16,
    off: u32,
    len: u16,
}

#[derive(Default)]
struct Part {
    names: Vec<Name>,
    bytes: Vec<u8>,
    /// Findings of extension records, for their base records.
    ext: Vec<(u32, Info)>,
}

fn rd16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn rd32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn rd64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// Every entry on the NTFS volume `letter`, as listings: id 0 is the
/// volume's root folder. None without admin rights or on another file
/// system.
pub fn scan_volume(letter: u8) -> Option<Vec<Listing>> {
    let path = format!(r"\\.\{}:", letter as char);
    let vol = std::fs::File::open(&path).ok()?;
    let mut vd: NTFS_VOLUME_DATA_BUFFER = unsafe { std::mem::zeroed() };
    let mut got = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            vol.as_raw_handle(),
            FSCTL_GET_NTFS_VOLUME_DATA,
            std::ptr::null(),
            0,
            (&raw mut vd).cast(),
            size_of::<NTFS_VOLUME_DATA_BUFFER>() as u32,
            &mut got,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return None;
    }
    let rec_size = vd.BytesPerFileRecordSegment as usize;
    let cluster = vd.BytesPerCluster as u64;
    let total = (vd.MftValidDataLength as u64 / rec_size as u64) as usize;
    // Record 0 is the $MFT itself: its $DATA runs say where the rest is.
    let mut r0 = vec![0u8; rec_size];
    vol.seek_read(&mut r0, vd.MftStartLcn as u64 * cluster).ok()?;
    let extents = mft_extents(&mut r0, rec_size)?;

    // Byte extents of the $MFT on the volume, cut into chunks, each with
    // the number of its first record.
    let mut chunks: Vec<(u64, usize, u32)> = Vec::new();
    let mut rec = 0usize;
    for (lcn, clusters) in extents {
        let (mut at, mut left) = (lcn * cluster, (clusters * cluster) as usize);
        while left > 0 && rec < total {
            let n = left.min(CHUNK).min((total - rec) * rec_size);
            chunks.push((at, n, rec as u32));
            rec += n / rec_size;
            at += n as u64;
            left -= n;
        }
    }

    let mut info = vec![Info::default(); total];
    // Each chunk writes only its own records' slots.
    let mut slots: Vec<&mut [Info]> = Vec::with_capacity(chunks.len());
    let mut rest = info.as_mut_slice();
    for &(_, n, _) in &chunks {
        let (a, b) = rest.split_at_mut(n / rec_size);
        slots.push(a);
        rest = b;
    }
    let parts: Vec<Part> = chunks
        .par_iter()
        .zip(slots.into_par_iter())
        .map(|(&(at, n, first), slots)| {
            let mut buf = vec![0u8; n];
            let mut part = Part::default();
            if vol.seek_read(&mut buf, at).is_ok() {
                for (k, r) in buf.chunks_exact_mut(rec_size).enumerate() {
                    parse_record(r, first + k as u32, &mut slots[k], &mut part);
                }
            }
            part
        })
        .collect();
    for p in &parts {
        for &(base, e) in &p.ext {
            let i = &mut info[base as usize];
            if e.size != 0 {
                i.size = e.size;
            }
            if e.tag != 0 {
                i.tag = e.tag;
            }
        }
    }
    LISTED.fetch_add(parts.iter().map(|p| p.names.len()).sum(), std::sync::atomic::Ordering::Relaxed);
    Some(listings(&info, &parts))
}

/// The $MFT's own extents, (first cluster, clusters), from record 0.
fn mft_extents(r: &mut [u8], rec_size: usize) -> Option<Vec<(u64, u64)>> {
    if !fixup(r, rec_size) {
        return None;
    }
    let mut a = rd16(r, 0x14) as usize;
    while a + 8 <= r.len() {
        let (ty, len) = (rd32(r, a), rd32(r, a + 4) as usize);
        if ty == ATTR_END || len == 0 {
            break;
        }
        if ty == ATTR_DATA && r[a + 8] != 0 && r[a + 9] == 0 {
            return Some(runs(&r[a + rd16(r, a + 0x20) as usize..a + len]));
        }
        a += len;
    }
    None
}

/// Decode a non-resident attribute's run list into (first cluster,
/// clusters); sparse runs are skipped.
fn runs(b: &[u8]) -> Vec<(u64, u64)> {
    let (mut out, mut i, mut lcn) = (Vec::new(), 0usize, 0i64);
    while i < b.len() && b[i] != 0 {
        let (nl, no) = ((b[i] & 0xF) as usize, (b[i] >> 4) as usize);
        i += 1;
        if i + nl + no > b.len() {
            break;
        }
        let mut len = 0u64;
        for k in 0..nl {
            len |= (b[i + k] as u64) << (8 * k);
        }
        i += nl;
        if no > 0 {
            let mut off = 0i64;
            for k in 0..no {
                off |= (b[i + k] as i64) << (8 * k);
            }
            // Sign-extend.
            off = off << (64 - 8 * no) >> (64 - 8 * no);
            lcn += off;
            out.push((lcn as u64, len));
        }
        i += no;
    }
    out
}

/// Apply the update sequence array: the last two bytes of each 512-byte
/// sector were swapped out for a check value when the record was written.
fn fixup(r: &mut [u8], rec_size: usize) -> bool {
    if &r[0..4] != b"FILE" {
        return false;
    }
    let (off, n) = (rd16(r, 4) as usize, rd16(r, 6) as usize);
    if n == 0 || off + 2 * n > r.len() || (n - 1) * 512 > rec_size {
        return false;
    }
    let check = [r[off], r[off + 1]];
    for s in 1..n {
        let end = s * 512 - 2;
        if r[end..end + 2] != check {
            return false;
        }
        r[end] = r[off + 2 * s];
        r[end + 1] = r[off + 2 * s + 1];
    }
    true
}

fn parse_record(r: &mut [u8], idx: u32, slot: &mut Info, part: &mut Part) {
    let rec_size = r.len();
    if !fixup(r, rec_size) {
        return;
    }
    let flags = rd16(r, 0x16);
    if flags & 1 == 0 {
        return; // not in use
    }
    let base = (rd64(r, 0x20) & 0xFFFF_FFFF_FFFF) as u32;
    let mut i = Info { used: true, dir: flags & 2 != 0, seq: rd16(r, 0x10), ..Default::default() };
    let owner = if base != 0 { base } else { idx };
    let mut a = rd16(r, 0x14) as usize;
    while a + 16 <= rec_size {
        let (ty, len) = (rd32(r, a), rd32(r, a + 4) as usize);
        if ty == ATTR_END || len < 16 || a + len > rec_size {
            break;
        }
        let nonres = r[a + 8] != 0;
        let named = r[a + 9] != 0;
        let val = |r: &[u8]| -> Option<(usize, usize)> {
            if nonres {
                return None;
            }
            let (vl, vo) = (rd32(r, a + 0x10) as usize, rd16(r, a + 0x14) as usize);
            (a + vo + vl <= a + len).then_some((a + vo, vl))
        };
        match ty {
            ATTR_STANDARD_INFORMATION => {
                if let Some((v, vl)) = val(r)
                    && vl >= 0x24
                {
                    i.mtime = crate::sys::filetime_secs(rd64(r, v + 8) as i64);
                    i.attrs = rd32(r, v + 0x20);
                }
            }
            ATTR_FILE_NAME => {
                if let Some((v, vl)) = val(r)
                    && vl >= 0x42
                {
                    let (nlen, ns) = (r[v + 0x40] as usize, r[v + 0x41]);
                    if ns != NAMESPACE_DOS && v + 0x42 + 2 * nlen <= v + vl {
                        let pref = rd64(r, v);
                        let w: Vec<u16> = (0..nlen).map(|k| rd16(r, v + 0x42 + 2 * k)).collect();
                        let off = part.bytes.len();
                        win::push_utf8(&mut part.bytes, &w);
                        let l = part.bytes.len() - off;
                        if l > 0 && l <= u16::MAX as usize {
                            part.names.push(Name {
                                rec: owner,
                                parent: (pref & 0xFFFF_FFFF_FFFF) as u32,
                                pseq: (pref >> 48) as u16,
                                off: off as u32,
                                len: l as u16,
                            });
                        } else {
                            part.bytes.truncate(off);
                        }
                    }
                }
            }
            ATTR_DATA if !named => {
                if nonres {
                    // Sizes are only in the extent that starts at VCN 0.
                    if rd64(r, a + 0x10) == 0 {
                        i.size = rd64(r, a + 0x30);
                    }
                } else if let Some((_, vl)) = val(r) {
                    i.size = vl as u64;
                }
            }
            ATTR_REPARSE_POINT => {
                if let Some((v, vl)) = val(r)
                    && vl >= 4
                {
                    i.tag = rd32(r, v);
                }
            }
            _ => {}
        }
        a += len;
    }
    if base != 0 {
        if i.size != 0 || i.tag != 0 {
            part.ext.push((base, i));
        }
    } else {
        *slot = i;
    }
}

/// Group names by parent folder into listings, root first (id 0).
fn listings(info: &[Info], parts: &[Part]) -> Vec<Listing> {
    let n = info.len();
    let shown = |rec: u32| (rec as usize) < n && info[rec as usize].used && (rec >= FIRST_USER || rec == ROOT);
    // A folder's listing id, for every folder record.
    let mut id = vec![NONE; n];
    let mut next = 1u32;
    id[ROOT as usize] = 0;
    for (r, i) in info.iter().enumerate() {
        if i.dir && i.used && r as u32 >= FIRST_USER {
            id[r] = next;
            next += 1;
        }
    }
    let mut out: Vec<Listing> = (0..next).map(|k| Listing { id: k, names: Vec::new(), ents: Vec::new() }).collect();
    for p in parts {
        for nm in &p.names {
            if !shown(nm.rec) || nm.rec == ROOT || !shown(nm.parent) {
                continue;
            }
            let (pi, i) = (&info[nm.parent as usize], &info[nm.rec as usize]);
            // A name whose parent record was since reused is stale.
            if !pi.dir || pi.seq != nm.pseq {
                continue;
            }
            let l = &mut out[id[nm.parent as usize] as usize];
            let link = i.attrs & 0x400 != 0 && i.tag & TAG_NAME_SURROGATE != 0;
            let mut kind = if link {
                KIND_LINK
            } else if i.dir {
                KIND_DIR
            } else {
                KIND_FILE
            };
            if i.attrs & 0x2 != 0 {
                kind |= FLAG_HIDDEN;
            }
            let child = if kind & 3 == KIND_DIR { id[nm.rec as usize] } else { NONE };
            let size = if kind & 3 == KIND_FILE { i.size } else { 0 };
            l.ents.push(RawEnt { name_off: l.names.len() as u32, name_len: nm.len, kind, size, mtime: i.mtime, child });
            l.names.extend_from_slice(&p.bytes[nm.off as usize..][..nm.len as usize]);
        }
    }
    out
}
