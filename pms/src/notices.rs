//! What PimpMyShell tells you itself: the rare troubles that need you, as cards of its own in the taskbar's toast
//! deck. A click on one explains, briefly, with the fix a button away. Where there's no deck (toasts off in the
//! config, the stock taskbar), a Windows notification says it instead.
//!
//! - Notification access is off: toasts can't come to the taskbar (they stay Windows'). Again every 5 minutes
//!   until it's on, or until you choose Windows' notifications (which turns `[toasts]` off: `enabled = false`).
//! - pms.toml didn't load (at the start, or a save since): once for each such save.
//! - Toasts couldn't start for another reason: once.
//!
//! The rest has its own way: the watchdog giving up tells you with a Windows notification (the taskbar is stock by
//! then), as software update does when a version is ready or didn't install.

use std::path::Path;
use std::time::{Duration, Instant};

/// How often a notice that lasts (notification access) comes again.
pub const AGAIN: Duration = Duration::from_secs(5 * 60);
/// Cards of our own take ids from here up, far above the notification platform's (which count up from 1), and
/// below `u32::MAX` (the id `toast show` takes without one).
const FIRST_ID: u32 = 0xFFF0_0000;

#[derive(Clone)]
pub struct Notice {
    /// What it's about: a newer notice with the same key takes the place of the old one.
    pub key: &'static str,
    /// The card's lines.
    pub title: String,
    pub body: String,
    /// The explanation a click opens: a heading over a few short paragraphs.
    pub heading: String,
    pub detail: String,
    /// The fix: a button's label, and what it opens (a URI or a file).
    pub fix: Option<(&'static str, String)>,
    /// A second button, "Keep Windows' notifications": the taskbar's toasts off, so the notice is over too.
    pub keep_native: bool,
    /// One line on what to do, for where there's only a Windows notification to say it.
    pub hint: String,
    /// While it lasts, it comes again this often; None: once.
    pub again: Option<Duration>,
}

/// Toasts can't come to the taskbar: Windows doesn't let apps read notifications.
pub fn notification_access() -> Notice {
    Notice {
        key: "notification access",
        title: "Notifications can't come to the taskbar".into(),
        body: "Windows doesn't let PimpMyShell read them. Click for how to fix it.".into(),
        heading: "Let PimpMyShell read your notifications".into(),
        detail: "PimpMyShell shows notifications in the taskbar by reading them as they come. Windows lets apps do that \
                 only while Settings > Privacy & security > Notifications allows it, and now it doesn't: your \
                 notifications pop up as usual, from Windows.\n\n\
                 Turn on \"Notification access\" and \"Let apps access your notifications\" there. They come to the \
                 taskbar within seconds; nothing needs restarting.\n\n\
                 Or keep Windows' notifications: PimpMyShell's taskbar ones turn off (the settings' Toasts page turns \
                 them on again), and this reminder stops."
            .into(),
        fix: Some(("Open notification settings", "ms-settings:privacy-notifications".into())),
        keep_native: true,
        hint: "Turn on notification access under Settings > Privacy & security > Notifications.".into(),
        again: Some(AGAIN),
    }
}

/// Toasts couldn't start, for a reason other than notification access.
pub fn toasts_failed(error: &str) -> Notice {
    Notice {
        key: "toasts",
        title: "Notifications can't come to the taskbar".into(),
        body: "Windows wouldn't let PimpMyShell listen for them. Click for more.".into(),
        heading: "Notifications stay with Windows".into(),
        detail: format!(
            "PimpMyShell couldn't start listening for notifications, so they pop up as usual, from Windows.\n\n\
             What Windows said: {error}\n\nIt keeps trying; once it works, they come to the taskbar."
        ),
        fix: None,
        keep_native: false,
        hint: "They pop up as usual, from Windows; PimpMyShell keeps trying.".into(),
        again: None,
    }
}

/// The config at `path` didn't load. `kept`: the taskbar keeps the one that last did; otherwise PimpMyShell started
/// without one (a stock taskbar).
pub fn config(path: &Path, error: &str, kept: bool) -> Notice {
    let name = path.file_name().map_or("pms.toml".into(), |n| n.to_string_lossy().into_owned());
    // The error starts with the path, which the heading already names.
    let error = concise(error.strip_prefix(&format!("{}: ", path.display())).unwrap_or(error).trim_end());
    let now = if kept { "The taskbar keeps the config that last loaded." } else { "PimpMyShell started with a stock taskbar." };
    Notice {
        key: "config",
        title: format!("{name} didn't load"),
        body: format!("{now} Click for what's wrong."),
        heading: format!("{name} didn't load"),
        detail: format!("{now}\n\n{error}\n\nFix it and save: it takes effect as soon as it loads."),
        fix: Some(("Open the config", path.display().to_string())),
        keep_native: false,
        hint: format!("{now} Fix {name} and save: it takes effect at once."),
        again: None,
    }
}

/// A TOML parse error as a dialog's proportional font can show it: the line (its number and text) and what's wrong
/// with it, without the caret drawing under it. Any other error as it is.
fn concise(error: &str) -> String {
    let Some(at) = error.lines().next().and_then(|l| l.strip_prefix("TOML parse error at line ")) else { return error.into() };
    let number = at.split(',').next().unwrap_or(at).trim();
    let source = error.lines().find_map(|l| l.trim_start().strip_prefix(number)?.trim_start().strip_prefix('|')).map(str::trim);
    // What's wrong: the lines after the drawing (neither the heading nor one of the drawing's `|` lines).
    let what: Vec<&str> = error.lines().skip(1).filter(|l| !l.trim_start().trim_start_matches(|c: char| c.is_ascii_digit()).trim_start().starts_with('|')).collect();
    match source {
        Some(s) if !s.is_empty() => format!("Line {number}: {s}\n{}", what.join("\n")),
        _ => format!("Line {number}: {}", what.join("\n")),
    }
}

/// The notices raised and not cleared, and when each is due on screen.
#[derive(Default)]
pub struct Notices {
    list: Vec<Raised>,
    ids: u32,
}

struct Raised {
    notice: Notice,
    id: u32,
    due: Option<Instant>,
}

impl Notices {
    /// Raise `n`: on screen at once, unless the same notice is already up (then it keeps its turn).
    pub fn raise(&mut self, n: Notice) {
        self.raise_in(n, Duration::ZERO);
    }

    /// Raise `n` to come on screen after `delay`, unless it's cleared by then (a save that only half-wrote the
    /// config, say, then the rest).
    pub fn raise_in(&mut self, n: Notice, delay: Duration) {
        let due = Some(Instant::now() + delay);
        if let Some(r) = self.list.iter_mut().find(|r| r.notice.key == n.key) {
            if r.notice.detail != n.detail {
                r.notice = n;
                r.due = due;
            }
            return;
        }
        let id = FIRST_ID + self.ids;
        self.ids += 1;
        self.list.push(Raised { notice: n, id, due });
    }

    /// It's over (the config loaded, access was given).
    pub fn clear(&mut self, key: &str) {
        self.list.retain(|r| r.notice.key != key);
    }

    pub fn is_raised(&self, key: &str) -> bool {
        self.list.iter().any(|r| r.notice.key == key)
    }

    pub fn any_due(&self, now: Instant) -> bool {
        self.list.iter().any(|r| r.due.is_some_and(|d| now >= d))
    }

    /// The notices due on screen now, each with its card's id; each then waits its `again`, or doesn't come again.
    pub fn due(&mut self, now: Instant) -> Vec<(u32, Notice)> {
        let mut out = Vec::new();
        for r in &mut self.list {
            if r.due.is_some_and(|d| now >= d) {
                r.due = r.notice.again.map(|a| now + a);
                out.push((r.id, r.notice.clone()));
            }
        }
        out
    }

    /// The notice whose card is `id`, if it's one of ours.
    pub fn by_card(&self, id: u32) -> Option<&Notice> {
        self.list.iter().find(|r| r.id == id).map(|r| &r.notice)
    }

    /// Whether `id` is one of our cards' ids (also one cleared since).
    pub fn is_ours(id: u32) -> bool {
        id >= FIRST_ID && id != u32::MAX
    }
}

/// A click on the notice's card: its explanation, and the fix if it's asked for. Waits, so: on a thread of its own.
pub fn explain(n: &Notice) {
    match choose(n) {
        Choice::KeepNative => crate::tray::queue(crate::tray::Command::KeepNativeToasts),
        Choice::Fix => {
            let Some((_, target)) = &n.fix else { return };
            let opened = if target.contains(':') && !Path::new(target).is_absolute() {
                crate::toasts::open(target)
            } else {
                open_file(target)
            };
            if let Err(e) = opened {
                crate::log(format!("notice {}: {e}", n.key));
            }
        }
        Choice::Close => {}
    }
}

/// What the explanation was answered with.
#[derive(Debug, PartialEq)]
pub enum Choice {
    /// Its fix (`fix`).
    Fix,
    /// *Keep Windows' notifications* (`keep_native`).
    KeepNative,
    /// Close, its x, Esc.
    Close,
}

/// The notice's explanation, waiting for its answer (examples/dialogs.rs shows them so; `explain` acts on it).
pub fn choose(n: &Notice) -> Choice {
    const KEEP_NATIVE: &str = "Keep Windows' notifications";
    let mut choices: Vec<&str> = n.fix.iter().map(|(label, _)| *label).collect();
    if n.keep_native {
        choices.push(KEEP_NATIVE);
    }
    match crate::tray::explain(&n.heading, &n.detail, &choices).map(|i| choices[i]) {
        Some(KEEP_NATIVE) => Choice::KeepNative,
        Some(_) => Choice::Fix,
        None => Choice::Close,
    }
}

/// A file in whatever opens it, or Notepad where nothing does (a .toml often has no app).
fn open_file(path: &str) -> Result<(), String> {
    crate::toasts::open(path).or_else(|_| {
        std::process::Command::new("notepad.exe").arg(path).spawn().map(drop).map_err(|e| format!("notepad {path}: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raised_once_or_again() {
        let mut n = Notices::default();
        n.raise(notification_access());
        n.raise(config(Path::new(r"C:\x\pms.toml"), r"C:\x\pms.toml: TOML parse error at line 3", true));
        let now = Instant::now();
        let due = n.due(now);
        assert_eq!(due.len(), 2);
        assert!(due.iter().all(|(id, _)| Notices::is_ours(*id)));
        assert_eq!(due[1].1.detail.lines().nth(2), Some("Line 3: "));
        // Shown: the config's doesn't come again, the access one does, AGAIN later.
        assert!(n.due(now + Duration::from_secs(1)).is_empty());
        let again = n.due(now + AGAIN);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].1.key, "notification access");
        // The same again keeps its turn; a different error comes at once; cleared, it's gone.
        n.raise(notification_access());
        assert!(n.due(now + AGAIN + Duration::from_secs(1)).is_empty());
        n.raise(config(Path::new(r"C:\x\pms.toml"), "line 4", true));
        assert!(n.is_raised("config"));
        n.clear("config");
        assert!(!n.is_raised("config"));
        assert!(n.by_card(due[0].0).is_some());
    }

    #[test]
    fn toml_errors_condensed() {
        let error = "TOML parse error at line 182, column 1\n    |\n182 | interval_ms = \"soon\"\n    | ^^^^^^^^^^^\n\
                     unknown field `interval_ms`, expected one of `hover`, `press`\n";
        assert_eq!(concise(error.trim_end()), "Line 182: interval_ms = \"soon\"\nunknown field `interval_ms`, expected one of `hover`, `press`");
        assert_eq!(concise("counter `x` (\\Nope\\Nothing): no such counter"), "counter `x` (\\Nope\\Nothing): no such counter");
        // As the daemon gets it: the path first.
        let n = config(Path::new(r"C:\x\pms.toml"), &format!(r"C:\x\pms.toml: {error}"), true);
        assert!(n.detail.contains("\n\nLine 182: interval_ms = \"soon\"\nunknown field"));
    }
}
