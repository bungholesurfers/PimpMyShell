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
    /// A second button, "Keep Windows notifications": the taskbar's toasts off, so the notice is over too.
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
        title: "Notifications can't appear in the taskbar".into(),
        body: "Windows isn't letting PimpMyShell read notifications. Select to fix it.".into(),
        heading: "Let PimpMyShell read your notifications".into(),
        detail: "To show notifications in the taskbar, PimpMyShell needs to read them as they arrive. Windows doesn't \
                 allow that right now, so your notifications appear as standard pop-ups.\n\n\
                 To fix this, go to Settings > Privacy & security > Notifications, and turn on Notification access and \
                 Let apps access your notifications. Your notifications move to the taskbar within a few seconds.\n\n\
                 If you'd rather keep standard pop-ups, select Keep Windows notifications. We'll turn off taskbar \
                 notifications and stop reminding you."
            .into(),
        fix: Some(("Open notification settings", "ms-settings:privacy-notifications".into())),
        keep_native: true,
        hint: "To fix this, turn on notification access in Settings > Privacy & security > Notifications.".into(),
        again: Some(AGAIN),
    }
}

/// Toasts couldn't start, for a reason other than notification access.
/// Where a report goes: the fix that isn't a file or a page, but `save_report`.
const REPORT: &str = "pms:report";
/// A new issue, for a report to be attached to.
const NEW_ISSUE: &str = "https://github.com/bungholesurfers/PimpMyShell/issues/new";

/// Something that should work didn't (`what`, a sentence), more than once: we can't fix it from here, but whoever made
/// this can, with a report. Said plainly, and with what's in the report, so that sending it is an informed choice.
pub fn problem(what: &str, error: &str) -> Notice {
    Notice {
        key: "problem",
        title: what.to_string(),
        body: "A report would help us fix this. Select for details.".into(),
        heading: "Help us fix this".into(),
        detail: format!(
            "{what}. It's nothing you did, and your taskbar is fine.\n\n\
             A report usually tells us why. It has your Windows version, PimpMyShell's logs and settings, and the layout of \
             your taskbar. It leaves out your Windows user name and what your notifications say.\n\n\
             Select Save report to see it. Nothing is sent: you decide whether to post it.\n\nDetails: {error}"
        ),
        fix: Some(("Save report", REPORT.to_string())),
        keep_native: false,
        hint: "To help us fix it, select Save a report in PimpMyShell's menu in the system tray.".into(),
        again: None,
    }
}

/// Saves a report on the config at `config` and shows it in File Explorer. Every way to a report ends here, so this is
/// where its owner hears what posting it means, and GitHub opens only if they choose it.
pub fn save_report(config: &Path) {
    match crate::system::write_report(config) {
        Ok(file) => {
            crate::log(format!("report saved: {}", file.display()));
            let _ = std::process::Command::new("explorer.exe").arg("/select,").arg(&file).spawn();
            if wants_to_post() {
                let _ = crate::toasts::open(NEW_ISSUE);
            }
        }
        Err(e) => crate::log(format!("couldn't save a report: {e}")),
    }
}

/// What posting a report means, and whether to go ahead.
pub fn wants_to_post() -> bool {
    let text = "It's in the folder that just opened, and it hasn't been sent anywhere.\n\n\
                GitHub issues are public: anyone can read and download what you attach, and search engines can find it. The \
                report leaves out your Windows user name and what your notifications say, but it does have your settings, \
                the names of apps that sent notifications, and your Windows version. Read it first, and remove anything \
                you'd rather not share.\n\nYou need a GitHub account to post it.";
    crate::tray::explain("Your report is saved", text, &["Open GitHub"]).is_some()
}

/// What PimpMyShell works with, in the README.
const SUPPORTED: &str = "https://github.com/bungholesurfers/PimpMyShell#what-it-works-with";

/// Something else runs the taskbar (`name`, if we know it), so there's nothing for us to hook. Nothing's wrong, and
/// nothing we can do: this one's said kindly, and a little jive.
pub fn taskbar_replaced(name: Option<&str>) -> Notice {
    let (who, running) = match name {
        Some(n) => (n.to_string(), format!("We dig {n}, but it's running your taskbar right now")),
        None => ("Something else".to_string(), "Something other than Windows seems to be running your taskbar right now".to_string()),
    };
    let way_back = match name {
        Some(n) => format!("Switch back to the Windows 11 taskbar in {n}'s settings"),
        None => "Switch back to the Windows 11 taskbar".to_string(),
    };
    Notice {
        key: "taskbar replaced",
        title: format!("{who} has the keys to your taskbar"),
        body: "PimpMyShell only rolls with the Windows 11 taskbar. Select for details.".into(),
        heading: "PimpMyShell only rolls with the Windows 11 taskbar".into(),
        detail: format!(
            "{running}, and PimpMyShell can only work with the one that comes with Windows 11. Meters, taskbar \
             notifications and the rest have nowhere to live, so PimpMyShell is laying low. Your PC is cool, and \
             nothing's been changed.\n\nWant to get pimped? {way_back}, and PimpMyShell will be right there with \
             you. Happy with your ride? That's cool too: you can exit PimpMyShell from its menu in the system tray. \
             We won't bring this up again."
        ),
        fix: Some(("See what's supported", SUPPORTED.to_string())),
        keep_native: false,
        hint: format!("{way_back} to use PimpMyShell. Until then, it's laying low."),
        again: None,
    }
}

/// The taskbar wouldn't take a toast: notifications went back to Windows, so that none go missing. `around`: what else
/// changes the taskbar here, which may well be why.
pub fn toasts_refused(error: &str, around: &[&str]) -> Notice {
    let why = if around.is_empty() {
        String::new()
    } else {
        format!("{} also changes your taskbar, which may be why. Turning that off and restarting Windows Explorer may help.\n\n", around.join(" and "))
    };
    Notice {
        key: "toasts",
        title: "Notifications can't appear in the taskbar".into(),
        body: "They're back to standard pop-ups. Select for details.".into(),
        heading: "Notifications appear as standard pop-ups for now".into(),
        detail: format!(
            "PimpMyShell couldn't show a notification in the taskbar, so notifications went back to standard pop-ups. \
             The ones that failed are in notification center. We'll try the taskbar again in a few minutes.\n\n\
             {why}A report would help us fix this. Select Save report to see it. Nothing is sent: you decide whether to \
             post it.\n\nDetails: {error}"
        ),
        fix: Some(("Save report", REPORT.to_string())),
        keep_native: false,
        hint: "They appear as standard pop-ups for now. The ones that failed are in notification center.".into(),
        again: None,
    }
}

pub fn toasts_failed(error: &str) -> Notice {
    Notice {
        key: "toasts",
        title: "Notifications can't appear in the taskbar".into(),
        body: "Windows didn't let PimpMyShell listen for them. Select for details.".into(),
        heading: "Notifications appear as standard pop-ups for now".into(),
        detail: format!(
            "PimpMyShell couldn't start listening for notifications, so they appear as standard pop-ups. We'll keep \
             trying, and they'll move to the taskbar once it works.\n\nDetails: {error}"
        ),
        fix: None,
        keep_native: false,
        hint: "They appear as standard pop-ups for now. We'll keep trying.".into(),
        again: None,
    }
}

/// The config at `path` didn't load. `kept`: the taskbar keeps the one that last did; otherwise PimpMyShell started
/// without one (a stock taskbar).
pub fn config(path: &Path, error: &str, kept: bool) -> Notice {
    let name = path.file_name().map_or("pms.toml".into(), |n| n.to_string_lossy().into_owned());
    // The error starts with the path, which the heading already names.
    let error = concise(error.strip_prefix(&format!("{}: ", path.display())).unwrap_or(error).trim_end());
    let now = if kept { "PimpMyShell is still using your last working settings." } else { "PimpMyShell started with the standard taskbar." };
    Notice {
        key: "config",
        title: "We couldn't load your settings".into(),
        body: format!("{now} Select to see what's wrong."),
        heading: format!("There's a problem in {name}"),
        detail: format!("{now}\n\n{error}\n\nFix the problem and save the file. Your changes take effect right away."),
        fix: Some(("Open settings file", path.display().to_string())),
        keep_native: false,
        hint: format!("{now} Fix {name} and save it. Your changes take effect right away."),
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
            if target == REPORT {
                return crate::tray::queue(crate::tray::Command::SaveReport);
            }
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
    /// *Keep Windows notifications* (`keep_native`).
    KeepNative,
    /// Close, its x, Esc.
    Close,
}

/// The notice's explanation, waiting for its answer (examples/dialogs.rs shows them so; `explain` acts on it).
pub fn choose(n: &Notice) -> Choice {
    const KEEP_NATIVE: &str = "Keep Windows notifications";
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
