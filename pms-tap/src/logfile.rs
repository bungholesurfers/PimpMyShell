//! The log files in `%LOCALAPPDATA%\PimpMyShell\logs`, kept small: once
//! one passes `MAX` bytes it starts over, the full
//! one kept beside it as `NAME.1.log` and the one before that as `NAME.2.log`, so each log holds at most three
//! files, about 3 MB.
//!
//! Nothing here deletes a file outright. The oldest generation goes only by the next one being renamed over it, and
//! the only paths ever touched are the fixed names below, in `dir()`, and only while each is a plain file (not
//! a folder, nor a link to somewhere else).
//!
//! People send these files to others, so no line keeps the Windows user name: see `redact`.
//!
//! Each line starts with when, `stamp`: UTC, to the millisecond, as RFC 3339 (ISO 8601's profile for the internet) writes it:
//! `2026-11-09T18:21:33.512Z`. It sorts as text, and means the same hour whatever the clocks did: and unlike local time with
//! its offset, it doesn't say where in the world the PC is, in a log someone may post.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

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
        dir().join(self.names()[0])
    }
}

/// Where the logs live: with this PC's own files, not in the roaming profile. A log is about this PC, it's written all the time,
/// and tap.log from inside Explorer: where the roaming folder is a network share (folder redirection), every line would be a
/// network write from the taskbar's thread, and a roaming profile would carry them all to the server and back at every sign-in.
pub fn dir() -> PathBuf {
    data_dir().join("logs")
}

/// The logs' earlier homes, newest first: the roaming folder (0.1.4), then `data_dir()` itself (before that).
fn old_dirs() -> Vec<PathBuf> {
    let roaming = std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("PimpMyShell").join("logs"));
    roaming.into_iter().chain([data_dir()]).collect()
}

/// Brings the logs from where they were before: each moves, unless one is in its place already (from a newer home, or begun).
/// A folder they leave empty goes too.
pub fn move_old() {
    let new = dir();
    let _ = std::fs::create_dir_all(&new);
    for old in old_dirs() {
        for name in [Log::Daemon, Log::Tap].iter().flat_map(|l| l.names()) {
            let (from, to) = (old.join(name), new.join(name));
            if std::fs::symlink_metadata(&from).is_ok_and(|m| m.file_type().is_file()) && !to.exists() {
                let _ = std::fs::rename(from, to);
            }
        }
    }
    if let Some(roaming) = old_dirs().first().filter(|d| **d != new) {
        // Only an empty folder can be removed this way: anything left in it stays.
        let _ = std::fs::remove_dir(roaming);
    }
}

/// One process's appends and rotations, one at a time.
static LOCK: Mutex<()> = Mutex::new(());

/// `line` without the user's name: their profile folder as `%USERPROFILE%`, and the name as `<user>` wherever it's a
/// whole part of a path (or `DOMAIN\\name`). Only there: a user called User shouldn't cost us `UserNotification`.
pub fn redact(line: &str) -> String {
    static ME: OnceLock<(String, String)> = OnceLock::new();
    let (profile, user) = ME.get_or_init(|| (std::env::var("USERPROFILE").unwrap_or_default(), std::env::var("USERNAME").unwrap_or_default()));
    redact_for(line, profile, user)
}

fn separator(c: char) -> bool {
    c == '\\' || c == '/'
}

/// How much of `text` (in bytes) is `what`: in any case, either kind of separator, and a doubled one (a path printed
/// with `{:?}`). None unless it ends where a name would, so that `bob` isn't found in `bobby`.
fn starts_with_name(text: &str, what: &str) -> Option<usize> {
    let mut rest = text;
    for w in what.chars() {
        let t = rest.chars().next()?;
        if separator(w) {
            if !separator(t) {
                return None;
            }
            rest = rest.trim_start_matches(separator);
        } else if t.to_lowercase().eq(w.to_lowercase()) {
            rest = &rest[t.len_utf8()..];
        } else {
            return None;
        }
    }
    // A full stop ends a sentence, unless more of the name follows (`bob.old`).
    let mut after = rest.chars();
    let ends = match after.next() {
        Some('.') => !after.next().is_some_and(char::is_alphanumeric),
        Some(c) => !(c.is_alphanumeric() || matches!(c, '_' | '-' | '$')),
        None => true,
    };
    ends.then_some(text.len() - rest.len())
}

fn redact_for(line: &str, profile: &str, user: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        if let Some(n) = Some(profile).filter(|p| !p.is_empty()).and_then(|p| starts_with_name(rest, p)) {
            out.push_str("%USERPROFILE%");
            rest = &rest[n..];
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
        if separator(c)
            && let Some(n) = Some(user).filter(|u| !u.is_empty()).and_then(|u| starts_with_name(rest, u))
        {
            out.push_str("<user>");
            rest = &rest[n..];
        }
    }
    out
}

/// Appends `line` to `log` (redacted), then starts the log over if that took it past `MAX`.
pub fn append(log: Log, line: &str) {
    // One entry, one line: a multi-line error would leave lines with no time, and the settings window's "last entry" a fragment.
    let line = redact(&line.trim_end().replace("\r\n", " | ").replace('\n', " | "));
    let _held = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = std::fs::create_dir_all(dir());
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
    let folder = dir();
    let [current, older, oldest] = log.names().map(|n| folder.join(n));
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

/// Now, for the start of a log line: see the top of this file.
pub fn stamp() -> String {
    rfc3339(unsafe { windows::Win32::System::SystemInformation::GetSystemTime() })
}

/// `utc`, as RFC 3339 writes it.
fn rfc3339(t: windows::Win32::Foundation::SYSTEMTIME) -> String {
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_are_utc() {
        let t = windows::Win32::Foundation::SYSTEMTIME { wYear: 2026, wMonth: 1, wDay: 9, wHour: 7, wMinute: 5, wSecond: 3, wMilliseconds: 7, ..Default::default() };
        assert_eq!(rfc3339(t), "2026-01-09T07:05:03.007Z");
        assert!(stamp().ends_with('Z') && stamp().len() == 24, "{}", stamp());
    }

    fn redacted(line: &str) -> String {
        redact_for(line, r"C:\Users\Bob", "Bob")
    }

    #[test]
    fn the_profile_folder_goes() {
        assert_eq!(redacted(r"daemon up: C:\Users\Bob\AppData\Roaming\PimpMyShell\pms.toml; meters"), r"daemon up: %USERPROFILE%\AppData\Roaming\PimpMyShell\pms.toml; meters");
        assert_eq!(redacted(r"c:\users\BOB\x and C:/Users/bob/y"), r"%USERPROFILE%\x and %USERPROFILE%/y");
        assert_eq!(redacted(r#"path: "C:\\Users\\Bob\\x""#), r#"path: "%USERPROFILE%\\x""#);
        assert_eq!(redacted(r"at C:\Users\Bob"), r"at %USERPROFILE%");
    }

    #[test]
    fn the_name_goes_from_paths_elsewhere() {
        assert_eq!(redacted(r"D:\Users\Bob\x"), r"D:\Users\<user>\x");
        assert_eq!(redacted(r"\\server\homes\bob, then"), r"\\server\homes\<user>, then");
        assert_eq!(redacted(r"signed in as CORP\Bob."), r"signed in as CORP\<user>.");
    }

    #[test]
    fn everything_else_stays() {
        // A name that's an everyday word: it only goes as a whole part of a path.
        let plain = r"UserNotification from Bob: C:\Users\Bobby\x, D:\Bob_old\y, C:\Windows\explorer.exe";
        assert_eq!(redact_for(plain, r"C:\Users\Bob", "Bob"), plain);
        assert_eq!(redact_for(r"User said C:\Users\User\x", r"C:\Users\User", "User"), r"User said %USERPROFILE%\x");
        assert_eq!(redact_for("nothing to go on", "", ""), "nothing to go on");
    }

    #[test]
    fn redacting_twice_changes_nothing_more() {
        let once = redacted(r"C:\Users\Bob\x D:\Bob\y");
        assert_eq!(redacted(&once), once);
    }

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
