//! The log files in `%LOCALAPPDATA%\PimpMyShell`, kept small: once one passes `MAX` bytes it starts over, the full
//! one kept beside it as `NAME.1.log` and the one before that as `NAME.2.log`, so each log holds at most three
//! files, about 3 MB.
//!
//! Nothing here deletes a file outright. The oldest generation goes only by the next one being renamed over it, and
//! the only paths ever touched are the fixed names below, in `data_dir()`, and only while each is a plain file (not
//! a folder, nor a link to somewhere else).

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::data_dir;

/// Each log's size before it starts over.
const MAX: u64 = 1 << 20;

/// A log, and the names of its files: the current one, then the older generations, newest first.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Log {
    /// The daemon's (pmsd.exe).
    Daemon,
    /// The TAP's, inside explorer.
    Tap,
}

impl Log {
    pub fn names(self) -> [&'static str; 3] {
        match self {
            Log::Daemon => ["daemon.log", "daemon.1.log", "daemon.2.log"],
            Log::Tap => ["tap.log", "tap.1.log", "tap.2.log"],
        }
    }

    /// The current file.
    pub fn path(self) -> PathBuf {
        data_dir().join(self.names()[0])
    }
}

/// One process's appends and rotations, one at a time.
static LOCK: Mutex<()> = Mutex::new(());

/// Appends `line` to `log`, then starts the log over if that took it past `MAX`.
pub fn append(log: Log, line: &str) {
    let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log.path()) else { return };
    let _ = writeln!(f, "{line}");
    let full = f.metadata().is_ok_and(|m| m.len() > MAX);
    drop(f);
    if full {
        rotate(log);
    }
}

/// `NAME.1.log` becomes `NAME.2.log` (over the old one), `NAME.log` becomes `NAME.1.log`. A step that can't be
/// taken - a file held open by another process, say - leaves the files as they are, to be tried again on the next
/// line; a name that isn't a plain file stops it altogether.
fn rotate(log: Log) {
    let dir = data_dir();
    let [current, older, oldest] = log.names().map(|n| dir.join(n));
    let plain_or_absent = |p: &PathBuf| match std::fs::symlink_metadata(p) {
        Ok(m) => m.file_type().is_file(),
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    };
    if ![&current, &older, &oldest].into_iter().all(plain_or_absent) {
        return;
    }
    if older.exists() && std::fs::rename(&older, &oldest).is_err() {
        return;
    }
    let _ = std::fs::rename(&current, &older);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names are fixed, distinct, plain file names: nothing that could reach outside the data folder.
    #[test]
    fn names() {
        let all: Vec<&str> = [Log::Daemon, Log::Tap].iter().flat_map(|l| l.names()).collect();
        for (i, n) in all.iter().enumerate() {
            assert!(n.ends_with(".log") && !n.contains(['/', '\\', ':']) && !n.starts_with('.'), "{n}");
            assert!(!all[i + 1..].contains(n), "{n} twice");
        }
    }
}
