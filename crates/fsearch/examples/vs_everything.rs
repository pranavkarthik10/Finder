//! Head to head: the same queries against the fsearch daemon (its socket)
//! and Everything (its SDK DLL, IPC to the running Everything), each the
//! way an app would use it: one connection, top 50 results with paths.
//!
//!   cargo run --release --example vs_everything -- <Everything64.dll> [query...]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    use std::io::{BufRead, BufReader, Write};
    use std::time::{Duration, Instant};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let dll = args.first().expect("path to Everything64.dll");
    let mut queries: Vec<String> = args[1..].to_vec();
    if queries.is_empty() {
        queries = ["main.rs", "chrome.exe", "readme", "fse", "node_modules", "invoice", "cargo.toml", "screenshot png", "a", "fndrsrch"]
            .map(String::from)
            .to_vec();
    }
    let ev = everything::Api::load(dll);
    if !ev.ready() {
        eprintln!("Everything isn't running (or its database isn't loaded yet)");
        std::process::exit(1);
    }
    let sock = fsearch::sys::data_dir().join("fsearch.sock");
    let conn = uds_windows::UnixStream::connect(&sock).unwrap_or_else(|e| panic!("fsearch daemon at {}: {e}", sock.display()));
    let mut w = conn.try_clone().unwrap();
    let mut r = BufReader::new(conn);
    let mut fs = |q: &str| -> (Duration, usize, String) {
        let t = Instant::now();
        writeln!(w, "{}", serde_json::json!({ "q": q, "limit": 50 })).unwrap();
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        let took = t.elapsed();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        let hits = v["hits"].as_array().map_or(&[][..], |a| a.as_slice());
        (took, hits.len(), hits.first().and_then(|h| h["path"].as_str()).unwrap_or("").to_string())
    };
    let median = |mut v: Vec<Duration>| {
        v.sort();
        v[v.len() / 2]
    };
    println!("{:<16} {:>10} {:>10} {:>8}   top hit (fsearch | Everything)", "query", "fsearch", "Everything", "ratio");
    for q in &queries {
        // Everything has no fuzzy matching: spaces are AND of substrings,
        // the closest to fsearch's words.
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let (mut fa, mut eb) = ((0, String::new()), (0, String::new()));
        for i in 0..23 {
            let (t, n, top) = fs(q);
            let (t2, n2, top2) = ev.query(q);
            if i >= 3 {
                a.push(t);
                b.push(t2);
            }
            (fa, eb) = ((n, top), (n2, top2));
        }
        let (ma, mb) = (median(a), median(b));
        println!(
            "{:<16} {:>8.2}ms {:>8.2}ms {:>7.1}x   {} [{}] | {} [{}]",
            q,
            ma.as_secs_f64() * 1e3,
            mb.as_secs_f64() * 1e3,
            mb.as_secs_f64() / ma.as_secs_f64(),
            short(&fa.1),
            fa.0,
            short(&eb.1),
            eb.0
        );
    }
}

#[cfg(windows)]
fn short(p: &str) -> String {
    let n = p.chars().count();
    if n <= 48 { p.to_string() } else { format!("…{}", p.chars().skip(n - 47).collect::<String>()) }
}

#[cfg(windows)]
mod everything {
    use std::time::{Duration, Instant};
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    type SetSearch = unsafe extern "system" fn(*const u16);
    type SetU32 = unsafe extern "system" fn(u32);
    type Query = unsafe extern "system" fn(i32) -> i32;
    type GetU32 = unsafe extern "system" fn() -> u32;
    type FullPath = unsafe extern "system" fn(u32, *mut u16, u32) -> u32;

    pub struct Api {
        set_search: SetSearch,
        set_max: SetU32,
        set_flags: SetU32,
        set_sort: SetU32,
        query: Query,
        num: GetU32,
        db_loaded: GetU32,
        full_path: FullPath,
    }

    impl Api {
        pub fn load(dll: &str) -> Api {
            let w: Vec<u16> = dll.encode_utf16().chain([0]).collect();
            let h = unsafe { LoadLibraryW(w.as_ptr()) };
            assert!(!h.is_null(), "cannot load {dll}");
            macro_rules! f {
                ($n:literal) => {
                    unsafe { std::mem::transmute(GetProcAddress(h, concat!($n, "\0").as_ptr()).expect($n)) }
                };
            }
            Api {
                set_search: f!("Everything_SetSearchW"),
                set_max: f!("Everything_SetMax"),
                set_flags: f!("Everything_SetRequestFlags"),
                set_sort: f!("Everything_SetSort"),
                query: f!("Everything_QueryW"),
                num: f!("Everything_GetNumResults"),
                db_loaded: f!("Everything_IsDBLoaded"),
                full_path: f!("Everything_GetResultFullPathNameW"),
            }
        }

        pub fn ready(&self) -> bool {
            unsafe { (self.db_loaded)() != 0 }
        }

        /// Top 50 by name with full paths (what a results list needs).
        pub fn query(&self, q: &str) -> (Duration, usize, String) {
            let w: Vec<u16> = q.encode_utf16().chain([0]).collect();
            let t = Instant::now();
            unsafe {
                (self.set_search)(w.as_ptr());
                (self.set_max)(50);
                // EVERYTHING_REQUEST_FILE_NAME | EVERYTHING_REQUEST_PATH
                (self.set_flags)(1 | 2);
                // EVERYTHING_SORT_NAME_ASCENDING (its default)
                (self.set_sort)(1);
                (self.query)(1);
            }
            let n = unsafe { (self.num)() } as usize;
            let mut top = String::new();
            for i in 0..n {
                let mut buf = [0u16; 1024];
                let len = unsafe { (self.full_path)(i as u32, buf.as_mut_ptr(), buf.len() as u32) } as usize;
                if i == 0 {
                    top = String::from_utf16_lossy(&buf[..len]);
                }
            }
            (t.elapsed(), n, top)
        }
    }
}
