//! Whole-disk file search for macOS and Windows: a fuzzy name index over
//! every entry on disk, kept live from FSEvents (ReadDirectoryChangesW on
//! Windows), plus a trigram content index for text files. `Engine` runs it all in-process; the `fsearch` binary wraps one in
//! a daemon with a JSON-lines socket.

pub mod content;
mod engine;
pub mod sys;
#[cfg(target_os = "macos")]
#[path = "fsevents.rs"]
mod watch;
#[cfg(windows)]
#[path = "watch_win.rs"]
mod watch;
pub mod index;
pub mod live;
pub mod query;
pub mod walk;

pub use content::{FileMatches, Grep, GrepResult};
pub use engine::{Engine, Found, Options, Status, default_dir, gated, has_full_disk_access, no_materialize};
pub use query::{GrepMode, Query};
