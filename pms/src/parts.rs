//! The programs PimpMyShell is made of, beside pmsd.exe. One that isn't there (an antivirus took it, a cleanup did, an update didn't
//! finish) is said, once a run, in a notice and where it's missed, rather than failing quietly every time it's wanted.

use std::path::PathBuf;
use std::sync::Mutex;

/// Twatch TV's player.
pub const TWATCH: &str = "pms-twatch.exe";
/// The console on channel 6.
pub const PIMPBOX: &str = "pms-pimpbox.exe";

/// Where `file` should be: beside this exe.
pub fn path(file: &str) -> Option<PathBuf> {
    std::env::current_exe().ok().map(|e| e.with_file_name(file))
}

/// Whether `file` is where it should be.
pub fn here(file: &str) -> bool {
    path(file).is_some_and(|p| p.is_file())
}

/// What's been found missing, and whether it's been said.
static FOUND: Mutex<Vec<(&'static str, bool)>> = Mutex::new(Vec::new());

/// `file` was wanted and isn't there: for the daemon to say, once a run.
pub fn found_missing(file: &'static str) {
    if let Ok(mut found) = FOUND.lock()
        && !found.iter().any(|(f, _)| *f == file)
    {
        crate::log(format!("{file} is missing from {}", path(file).and_then(|p| p.parent().map(|d| d.display().to_string())).unwrap_or_default()));
        found.push((file, false));
    }
}

/// What's been found missing and not yet said.
pub fn newly_missing() -> Vec<&'static str> {
    let Ok(mut found) = FOUND.lock() else { return Vec::new() };
    found.iter_mut().filter(|(_, said)| !*said).map(|(file, said)| {
        *said = true;
        *file
    }).collect()
}

/// What goes wrong without `file`, for saying so.
pub fn without(file: &str) -> &'static str {
    match file {
        TWATCH => "your Twatch TVs can't play live video",
        PIMPBOX => "the console on channel 6 can't start",
        _ => "part of PimpMyShell can't work",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whats_missing_is_said_once() {
        found_missing(PIMPBOX);
        found_missing(PIMPBOX);
        assert_eq!(newly_missing(), [PIMPBOX]);
        assert!(newly_missing().is_empty());
        assert!(!here("no-such-part-of-pms.exe"));
    }
}
