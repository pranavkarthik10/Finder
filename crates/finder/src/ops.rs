//! File operations through the Windows shell (SHFileOperationW): Explorer's
//! own progress window, conflict prompts and Recycle Bin, so nothing here
//! reimplements copying. Calls block until done: run them off the UI thread.

use std::path::{Path, PathBuf};
use windows::Win32::UI::Shell::{
    FILEOPERATION_FLAGS, FO_COPY, FO_DELETE, FO_MOVE, FOF_ALLOWUNDO, FOF_MULTIDESTFILES, FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR,
    SHFILEOPSTRUCTW, SHFileOperationW,
};
use windows::core::PCWSTR;

pub enum Op {
    /// Copy into a folder.
    Copy,
    /// Move into a folder.
    Move,
    /// To the Recycle Bin (no prompt: Finder doesn't ask before Trash).
    Recycle,
}

/// Paths as the shell wants them: each NUL-terminated, then one more NUL.
fn multi(paths: &[PathBuf]) -> Vec<u16> {
    let mut v: Vec<u16> = Vec::new();
    for p in paths {
        v.extend(p.as_os_str().to_string_lossy().encode_utf16());
        v.push(0);
    }
    v.push(0);
    v
}

fn shell(func: u32, from: &[PathBuf], to: &[PathBuf], flags: FILEOPERATION_FLAGS) -> Result<(), String> {
    if from.is_empty() {
        return Ok(());
    }
    let (f, t) = (multi(from), multi(to));
    let mut op = SHFILEOPSTRUCTW {
        wFunc: func,
        pFrom: PCWSTR(f.as_ptr()),
        pTo: if to.is_empty() { PCWSTR::null() } else { PCWSTR(t.as_ptr()) },
        fFlags: flags.0 as u16,
        ..Default::default()
    };
    match unsafe { SHFileOperationW(&mut op) } {
        0 => Ok(()),
        // Cancelled by the user (in a conflict prompt or the progress window).
        _ if op.fAnyOperationsAborted.as_bool() => Ok(()),
        e => Err(format!("The operation couldn't be completed (error {e:#x}).")),
    }
}

pub fn run(op: Op, from: &[PathBuf], into: Option<&Path>) -> Result<(), String> {
    let into: Vec<PathBuf> = into.map(Path::to_path_buf).into_iter().collect();
    match op {
        Op::Copy => shell(FO_COPY, from, &into, FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR),
        Op::Move => shell(FO_MOVE, from, &into, FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR),
        Op::Recycle => shell(FO_DELETE, from, &[], FOF_ALLOWUNDO | FOF_NOCONFIRMATION),
    }
}

/// Copy each of `from` to the matching path in `to` (new names).
pub fn copy_as(from: &[PathBuf], to: &[PathBuf]) -> Result<(), String> {
    shell(FO_COPY, from, to, FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOF_MULTIDESTFILES)
}

/// Finder's names for copies: "name copy.ext", then "name copy 2.ext"...
pub fn copy_name(p: &Path) -> PathBuf {
    let dir = p.parent().unwrap_or(Path::new(""));
    let is_dir = p.is_dir();
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !is_dir && !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    (1..)
        .map(|n| dir.join(if n == 1 { format!("{stem} copy{ext}") } else { format!("{stem} copy {n}{ext}") }))
        .find(|c| !c.exists())
        .unwrap()
}

/// "untitled folder", "untitled folder 2", ... in `dir`, created.
pub fn new_folder(dir: &Path) -> Result<PathBuf, String> {
    let p = (1..)
        .map(|n| dir.join(if n == 1 { "untitled folder".to_string() } else { format!("untitled folder {n}") }))
        .find(|c| !c.exists())
        .unwrap();
    std::fs::create_dir(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

/// Rename in place; names with path separators or reserved characters are
/// refused like Explorer does.
pub fn rename(from: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().any(|c| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
        return Err(format!("The name \u{201c}{name}\u{201d} can't be used."));
    }
    let to = from.with_file_name(name);
    if to != from && to.exists() && !to.as_os_str().eq_ignore_ascii_case(from.as_os_str()) {
        return Err(format!("The name \u{201c}{name}\u{201d} is already taken. Please choose a different name."));
    }
    std::fs::rename(from, &to).map_err(|e| e.to_string())?;
    Ok(to)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join("finder-ops-tests").join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn copy_names_follow_finder() {
        let d = scratch("names");
        std::fs::write(d.join("a.txt"), "x").unwrap();
        assert_eq!(copy_name(&d.join("a.txt")), d.join("a copy.txt"));
        std::fs::write(d.join("a copy.txt"), "x").unwrap();
        assert_eq!(copy_name(&d.join("a.txt")), d.join("a copy 2.txt"));
        std::fs::create_dir(d.join("my.folder")).unwrap();
        assert_eq!(copy_name(&d.join("my.folder")), d.join("my.folder copy"));
    }

    #[test]
    fn new_folders_count_up() {
        let d = scratch("new");
        assert_eq!(new_folder(&d).unwrap(), d.join("untitled folder"));
        assert_eq!(new_folder(&d).unwrap(), d.join("untitled folder 2"));
    }

    #[test]
    fn rename_checks_names() {
        let d = scratch("rename");
        std::fs::write(d.join("a.txt"), "x").unwrap();
        std::fs::write(d.join("b.txt"), "x").unwrap();
        assert!(rename(&d.join("a.txt"), "b.txt").is_err());
        assert!(rename(&d.join("a.txt"), "x/y").is_err());
        assert!(rename(&d.join("a.txt"), "  ").is_err());
        assert_eq!(rename(&d.join("a.txt"), "c.txt").unwrap(), d.join("c.txt"));
        // Changing only the case is allowed.
        assert_eq!(rename(&d.join("c.txt"), "C.txt").unwrap(), d.join("C.txt"));
    }

    #[test]
    fn shell_copy_move_and_copy_as() {
        let d = scratch("shell");
        let (src, dst) = (d.join("src"), d.join("dst"));
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("f.txt"), "hello").unwrap();
        std::fs::write(src.join("sub").join("g.txt"), "deep").unwrap();
        std::fs::create_dir(&dst).unwrap();
        run(Op::Copy, &[src.join("f.txt"), src.join("sub")], Some(&dst)).unwrap();
        assert_eq!(std::fs::read_to_string(dst.join("f.txt")).unwrap(), "hello");
        assert_eq!(std::fs::read_to_string(dst.join("sub").join("g.txt")).unwrap(), "deep");
        run(Op::Move, &[dst.join("f.txt")], Some(&src.join("sub"))).unwrap();
        assert!(!dst.join("f.txt").exists() && src.join("sub").join("f.txt").exists());
        let c = copy_name(&src.join("f.txt"));
        copy_as(&[src.join("f.txt")], std::slice::from_ref(&c)).unwrap();
        assert_eq!(c, src.join("f copy.txt"));
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "hello");
    }
}
