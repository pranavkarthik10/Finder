//! Folder contents and how Finder words them: kinds, sizes, dates.

use chrono::{DateTime, Local};
use gpui::SharedString;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: SharedString,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub kind: SharedString,
}

impl Entry {
    pub fn from_path(path: PathBuf) -> Option<Entry> {
        let m = std::fs::metadata(&path).ok()?;
        let name = display_name(&path);
        Some(Entry::new(name, path, &m))
    }

    fn new(name: String, path: PathBuf, m: &std::fs::Metadata) -> Entry {
        let mut e = Entry::from_parts(name, path, m.is_dir(), m.len(), m.modified().ok());
        e.created = m.created().ok();
        e.accessed = m.accessed().ok();
        e
    }

    /// An entry from what the index knows (search results): creation and
    /// access times are left for `from_path`.
    pub fn from_parts(name: String, path: PathBuf, is_dir: bool, size: u64, modified: Option<SystemTime>) -> Entry {
        let kind = kind_of(&name, is_dir).into();
        // Windows never shows a shortcut's extension.
        let name = match name.rsplit_once('.') {
            Some((stem, e)) if !is_dir && !stem.is_empty() && (e.eq_ignore_ascii_case("lnk") || e.eq_ignore_ascii_case("url")) => stem.to_string(),
            _ => name,
        };
        Entry { name: name.into(), path, is_dir, size: if is_dir { 0 } else { size }, modified, created: None, accessed: None, kind }
    }

    /// The icon asset for this entry.
    pub fn icon(&self) -> &'static str {
        if self.is_dir {
            return "icons/folder.svg";
        }
        match ext(&self.path.to_string_lossy()).as_str() {
            "exe" | "msi" | "app" | "lnk" => "icons/app.svg",
            _ => "icons/file.svg",
        }
    }
}

/// What a folder or drive is called in the UI: "C:\" is "Local Disk (C:)".
pub fn display_name(path: &Path) -> String {
    match path.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => {
            let s = path.to_string_lossy();
            format!("Local Disk ({})", s.trim_end_matches('\\'))
        }
    }
}

/// The folder's visible entries (no hidden or system files, like Finder).
pub fn list(dir: &Path) -> std::io::Result<Vec<Entry>> {
    use std::os::windows::fs::MetadataExt;
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let Ok(e) = e else { continue };
        let Ok(m) = e.metadata() else { continue };
        // FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM
        if m.file_attributes() & (0x2 | 0x4) != 0 {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name.eq_ignore_ascii_case("desktop.ini") {
            continue;
        }
        out.push(Entry::new(name, e.path(), &m));
    }
    Ok(out)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Modified,
    Size,
    Kind,
}

/// Finder's order: the key, then name (natural, case-insensitive).
pub fn sort(v: &mut [Entry], key: SortKey, ascending: bool) {
    v.sort_by(|a, b| {
        let o = match key {
            SortKey::Name => std::cmp::Ordering::Equal,
            SortKey::Modified => a.modified.cmp(&b.modified),
            SortKey::Size => a.size.cmp(&b.size).then(b.is_dir.cmp(&a.is_dir)),
            SortKey::Kind => kind_rank(a).cmp(&kind_rank(b)).then_with(|| a.kind.cmp(&b.kind)),
        };
        let o = o.then_with(|| natural_cmp(&a.name, &b.name));
        if ascending { o } else { o.reverse() }
    });
}

fn kind_rank(e: &Entry) -> u8 {
    if e.is_dir { 0 } else { 1 }
}

/// "file2" before "file10", ignoring case.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek(), y.peek()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let n: String = std::iter::from_fn(|| x.next_if(|c| c.is_ascii_digit())).collect();
                let m: String = std::iter::from_fn(|| y.next_if(|c| c.is_ascii_digit())).collect();
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if o.is_ne() {
                    return o;
                }
            }
            (Some(_), Some(_)) => {
                let (c, d) = (x.next().unwrap(), y.next().unwrap());
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o.is_ne() {
                    return o;
                }
            }
        }
    }
}

fn ext(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() => e.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Finder's "Kind" column wording.
pub fn kind_of(name: &str, is_dir: bool) -> String {
    if is_dir {
        return "Folder".into();
    }
    let e = ext(name);
    let k = match e.as_str() {
        "png" => "PNG image",
        "jpg" | "jpeg" => "JPEG image",
        "gif" => "GIF Image",
        "heic" => "HEIC image",
        "webp" => "WebP image",
        "svg" => "SVG document",
        "bmp" => "BMP image",
        "ico" => "Windows icon",
        "psd" => "Adobe Photoshop file",
        "mp4" | "m4v" => "MPEG-4 movie",
        "mov" => "QuickTime movie",
        "mkv" => "Matroska video",
        "avi" => "AVI movie",
        "webm" => "WebM video",
        "mp3" => "MP3 audio",
        "m4a" => "Apple MPEG-4 audio",
        "wav" => "Waveform audio",
        "flac" => "FLAC audio",
        "pdf" => "PDF document",
        "txt" => "Plain Text",
        "md" => "Markdown document",
        "rtf" => "Rich Text Document",
        "doc" | "docx" => "Microsoft Word document",
        "xls" | "xlsx" => "Microsoft Excel spreadsheet",
        "ppt" | "pptx" => "Microsoft PowerPoint presentation",
        "csv" => "CSV document",
        "json" => "JSON",
        "html" | "htm" => "HTML text",
        "css" => "CSS",
        "js" | "mjs" => "JavaScript script",
        "ts" | "tsx" => "TypeScript",
        "rs" => "Rust source",
        "py" => "Python script",
        "c" => "C source",
        "h" => "C header",
        "cpp" | "cc" => "C++ source",
        "go" => "Go source",
        "java" => "Java source",
        "swift" => "Swift source",
        "sh" => "Shell script",
        "ps1" => "PowerShell script",
        "bat" | "cmd" => "Batch file",
        "toml" => "TOML document",
        "yaml" | "yml" => "YAML document",
        "xml" => "XML text",
        "zip" => "ZIP archive",
        "7z" => "7-Zip archive",
        "rar" => "RAR archive",
        "gz" | "tgz" => "gzip compressed archive",
        "tar" => "tar archive",
        "iso" => "Disk image",
        "dmg" => "Apple disk image",
        "exe" => "Application",
        "msi" => "Windows Installer package",
        "lnk" => "Alias",
        "dll" => "Dynamic library",
        "ttf" | "otf" => "Font",
        "" => "Document",
        _ => return format!("{} file", e.to_ascii_uppercase()),
    };
    k.into()
}

/// Free bytes on the volume holding `path`.
pub fn free_space(path: &Path) -> Option<u64> {
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(&windows::core::HSTRING::from(path.as_os_str()), Some(&mut free), None, None) }.ok()?;
    Some(free)
}

/// Finder's sizes: decimal units, whole KB, one decimal from MB up.
pub fn format_size(e: &Entry) -> String {
    if e.is_dir {
        return "--".into();
    }
    format_bytes(e.size)
}

pub fn format_bytes(b: u64) -> String {
    if b == 0 {
        return "Zero bytes".into();
    }
    if b < 1000 {
        return format!("{b} bytes");
    }
    if b < 1_000_000 {
        return format!("{} KB", (b + 500) / 1000);
    }
    let units = ["MB", "GB", "TB"];
    let mut v = b as f64 / 1e6;
    let mut u = 0;
    while v >= 1000. && u < units.len() - 1 {
        v /= 1000.;
        u += 1;
    }
    format!("{v:.1} {}", units[u])
}

/// "Today at 5:15 PM", "Yesterday at 9:02 AM", "Sep 20, 2026 at 8:17 PM".
pub fn format_date(t: Option<SystemTime>) -> String {
    let Some(t) = t else { return "--".into() };
    let d: DateTime<Local> = t.into();
    let today = Local::now().date_naive();
    let time = d.format("%-I:%M %p");
    if d.date_naive() == today {
        format!("Today at {time}")
    } else if Some(d.date_naive()) == today.pred_opt() {
        format!("Yesterday at {time}")
    } else {
        format!("{} at {time}", d.format("%b %-d, %Y"))
    }
}

/// "Saturday, September 6, 2025 at 2:49 PM": the inspector's long form.
pub fn format_date_long(t: Option<SystemTime>) -> String {
    let Some(t) = t else { return "--".into() };
    let d: DateTime<Local> = t.into();
    d.format("%A, %B %-d, %Y at %-I:%M %p").to_string()
}
