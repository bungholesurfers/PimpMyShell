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

use crate::lang::notices as t;

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
    /// What a click opens, in place of an explanation: there's nothing to explain.
    pub opens: Option<String>,
    /// A file to show in File Explorer, with a button of its own.
    pub reveal: Option<String>,
    /// One line on what to do, for where there's only a Windows notification to say it.
    pub hint: String,
    /// While it lasts, it comes again this often; None: once.
    pub again: Option<Duration>,
    /// Up for the toasts' usual time (`[toasts] duration_ms`), not Windows' long one: news with nothing to do about it.
    pub brief: bool,
}

/// Favorites that have just gone live, best first. There's nothing to do about it, so a click goes to the first one's channel.
pub fn favorite_live(handles: &[String]) -> Notice {
    let title = match handles {
        [one] => t::LIVE_ONE.with(&[("one", one)]),
        [one, two] => t::LIVE_TWO.with(&[("one", one), ("two", two)]),
        [one, rest @ ..] => t::LIVE_MORE.with(&[("one", one), ("n", &rest.len())]),
        [] => String::new(),
    };
    Notice {
        key: "favorite live",
        title,
        body: t::LIVE_BODY.t().into(),
        heading: String::new(),
        detail: String::new(),
        fix: None,
        keep_native: false,
        opens: handles.first().map(|h| format!("https://www.twitch.tv/{h}")),
        reveal: None,
        hint: t::LIVE_HINT.t().into(),
        again: None,
        brief: true,
    }
}

/// Memory is low, and the console would have gone, but it has a game someone's been playing.
pub fn console_spared() -> Notice {
    Notice {
        key: "console spared",
        title: t::SPARED_TITLE.t().into(),
        body: t::SPARED_BODY.t().into(),
        heading: t::SPARED_HEADING.t().into(),
        detail: t::SPARED_DETAIL.t().into(),
        fix: None,
        keep_native: false,
        opens: None,
        reveal: None,
        hint: t::SPARED_BODY.t().into(),
        again: None,
        brief: false,
    }
}

/// The ISS has emptied its urine tank, down to `level` percent: Number One saw it (channel 37), and you asked to be told.
pub fn iss_flushed(level: f32) -> Notice {
    let level = level.round() as i64;
    Notice {
        key: "iss flushed",
        title: t::ISS_FLUSHED_TITLE.t().into(),
        body: t::ISS_FLUSHED_BODY.with(&[("level", &level)]),
        heading: t::ISS_FLUSHED_HEADING.t().into(),
        detail: t::ISS_FLUSHED_DETAIL.with(&[("level", &level)]),
        fix: None,
        keep_native: false,
        opens: None,
        reveal: None,
        hint: t::ISS_FLUSHED_HINT.t().into(),
        again: None,
        brief: false,
    }
}

/// Where a restart of Explorer is the fix.
const RESTART_EXPLORER: &str = "pms:restart-explorer";

/// Explorer is holding `taps` of ours and `megabytes` of memory. Restarting it is for you to choose: this only says so.
pub fn explorer_heavy(taps: usize, megabytes: u64) -> Notice {
    Notice {
        key: "explorer",
        title: t::EXPLORER_TITLE.t().into(),
        body: t::EXPLORER_BODY.t().into(),
        heading: t::EXPLORER_HEADING.t().into(),
        detail: t::EXPLORER_DETAIL.with(&[("taps", &taps), ("megabytes", &megabytes)]),
        fix: Some((t::EXPLORER_FIX.t(), RESTART_EXPLORER.into())),
        keep_native: false,
        opens: None,
        reveal: None,
        hint: t::EXPLORER_HINT.t().into(),
        again: None,
        brief: false,
    }
}

/// The name of the file at `path`, or `otherwise`.
fn file_name(path: &Path, otherwise: &str) -> String {
    path.file_name().map_or(otherwise.into(), |n| n.to_string_lossy().into_owned())
}

/// After the first few of a list, how many more there are, on a line of its own: or nothing.
fn and_more(more: usize, gap: &str) -> String {
    if more > 0 { format!("{gap}{}", t::AND_MORE.with(&[("n", &more)])) } else { String::new() }
}

/// Lines of the watchlist at `path` that can't be channels: skipped, and the rest are in use.
pub fn watchlist(path: &Path, bad: &[(usize, String)]) -> Notice {
    let name = file_name(path, t::WATCHLIST_NAME.t());
    let lines: Vec<String> = bad.iter().take(6).map(|(n, text)| t::LINE.with(&[("n", n), ("text", text)])).collect();
    let lines = format!("{}{}", lines.join("\n"), and_more(bad.len().saturating_sub(lines.len()), "\n"));
    Notice {
        key: "watchlist",
        title: t::WATCHLIST_TITLE.t().into(),
        body: t::WATCHLIST_BODY.with(&[("name", &name)]),
        heading: t::PROBLEM_IN.with(&[("name", &name)]),
        detail: t::WATCHLIST_DETAIL.with(&[("lines", &lines)]),
        fix: Some((t::WATCHLIST_FIX.t(), path.display().to_string())),
        keep_native: false,
        opens: None,
        reveal: Some(path.display().to_string()),
        hint: t::FIX_AND_SAVE.with(&[("name", &name)]),
        again: None,
        brief: false,
    }
}

/// Where the settings window opens on a page of its own: this, then the page's tag.
const SETTINGS_ON: &str = "pms:settings:";

/// Settings in the file at `path` that couldn't be used (`config.issues`): each back to its default, or left out, and the rest
/// loaded. Said once for each time the file changes and still has some, as an update can bring them about as easily as a slip.
pub fn settings_issues(path: &Path, issues: &[String]) -> Notice {
    let name = file_name(path, "pms.toml");
    let said: Vec<String> = issues.iter().take(6).map(|e| crate::problem::Problem::of(path, e).words()).collect();
    let said = format!("{}{}", said.join("\n\n"), and_more(issues.len().saturating_sub(said.len()), "\n\n"));
    let page = issues.first().map_or("general", |e| crate::config::settings_page(&crate::config::issue_setting(e)));
    Notice {
        key: "settings issues",
        title: t::ISSUES_TITLE.t().into(),
        body: t::ISSUES_BODY.with(&[("name", &name)]),
        heading: t::ISSUES_HEADING.with(&[("name", &name)]),
        detail: t::ISSUES_DETAIL.with(&[("issues", &said)]),
        fix: Some((t::OPEN_SETTINGS.t(), format!("{SETTINGS_ON}{page}"))),
        keep_native: false,
        opens: None,
        reveal: Some(path.display().to_string()),
        hint: t::ISSUES_HINT.t().into(),
        again: None,
        brief: false,
    }
}

/// The network settings' file at `path` won't load, so nothing goes online: there's no knowing whether a proxy was wanted, and
/// going without one might be what mustn't happen.
pub fn network_settings(path: &Path, error: &str) -> Notice {
    let name = file_name(path, "pms-network.toml");
    let problem = crate::problem::Problem::of(path, error).words();
    Notice {
        key: "network",
        title: t::NETWORK_TITLE.with(&[("name", &name)]),
        body: t::NETWORK_BODY.t().into(),
        heading: t::PROBLEM_IN.with(&[("name", &name)]),
        detail: t::NETWORK_DETAIL.with(&[("problem", &problem)]),
        fix: Some((t::OPEN_FILE.t(), path.display().to_string())),
        keep_native: false,
        opens: None,
        reveal: Some(path.display().to_string()),
        hint: t::NETWORK_HINT.with(&[("name", &name)]),
        again: None,
        brief: false,
    }
}

/// Toasts can't come to the taskbar: Windows doesn't let apps read notifications.
pub fn notification_access() -> Notice {
    Notice {
        key: "notification access",
        title: t::TOASTS_TITLE.t().into(),
        body: t::ACCESS_BODY.t().into(),
        heading: t::ACCESS_HEADING.t().into(),
        detail: t::ACCESS_DETAIL.t().into(),
        fix: Some((t::ACCESS_FIX.t(), "ms-settings:privacy-notifications".into())),
        keep_native: true,
        opens: None,
        reveal: None,
        hint: t::ACCESS_HINT.t().into(),
        again: Some(AGAIN),
        brief: false,
    }
}

/// Where a newer Streamlink is got: the fix that opens the settings.
const TWATCH_SETTINGS: &str = "pms:twatch";

/// Streamlink couldn't open a live channel, and there's a newer one. Nothing's been fetched: it's new code from elsewhere, so
/// it's for you to say, knowing what's been checked and what hasn't.
pub fn newer_streamlink(version: &str) -> Notice {
    let have = crate::streamlink::version().unwrap_or_default();
    Notice {
        key: "streamlink",
        title: t::NEWER_STREAMLINK_TITLE.with(&[("version", &version)]),
        body: t::NEWER_STREAMLINK_BODY.t().into(),
        heading: t::NEWER_STREAMLINK_HEADING.with(&[("version", &version)]),
        detail: t::NEWER_STREAMLINK_DETAIL.with(&[("have", &have), ("url", &crate::streamlink::URL)]),
        fix: Some((t::TWATCH_SETTINGS.t(), TWATCH_SETTINGS.into())),
        keep_native: false,
        opens: None,
        reveal: None,
        hint: t::NEWER_STREAMLINK_HINT.with(&[("version", &version)]),
        again: None,
        brief: false,
    }
}

/// A favorite is live and a set would play it, but Streamlink isn't here. Once, and again only after Streamlink has been here and
/// gone (twatch/mod.rs): the set itself says so for as long as it lasts.
pub fn no_streamlink() -> Notice {
    Notice {
        key: "streamlink",
        title: t::NO_STREAMLINK_TITLE.t().into(),
        body: t::NO_STREAMLINK_BODY.t().into(),
        heading: t::NO_STREAMLINK_HEADING.t().into(),
        detail: t::NO_STREAMLINK_DETAIL.with(&[("url", &crate::streamlink::URL)]),
        fix: Some((t::TWATCH_SETTINGS.t(), TWATCH_SETTINGS.into())),
        keep_native: false,
        opens: None,
        reveal: None,
        hint: t::NO_STREAMLINK_HINT.t().into(),
        again: None,
        brief: false,
    }
}

/// Part of PimpMyShell (`file`, from parts.rs) isn't beside the rest, so what needs it can't work.
pub fn missing_part(file: &'static str) -> Notice {
    let dir = crate::parts::path(file).and_then(|p| p.parent().map(crate::shown)).unwrap_or_default();
    let without = crate::parts::without(file);
    let with: &[(&str, &dyn std::fmt::Display)] = &[("file", &file), ("without", &without), ("dir", &dir)];
    Notice {
        key: file,
        title: t::MISSING_TITLE.t().into(),
        body: t::MISSING_BODY.with(with),
        heading: t::MISSING_HEADING.with(with),
        detail: t::MISSING_DETAIL.with(with),
        fix: Some((t::MISSING_FIX.t(), crate::update::RELEASES_PAGE.into())),
        keep_native: false,
        opens: None,
        reveal: crate::parts::path("pmsd.exe").map(|p| p.display().to_string()),
        hint: t::MISSING_HINT.with(with),
        again: None,
        brief: false,
    }
}

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
        body: t::PROBLEM_BODY.t().into(),
        heading: t::PROBLEM_HEADING.t().into(),
        detail: t::PROBLEM_DETAIL.with(&[("what", &what), ("error", &error)]),
        fix: Some((t::SAVE_REPORT.t(), REPORT.to_string())),
        keep_native: false,
        opens: None,
        reveal: None,
        hint: t::PROBLEM_HINT.t().into(),
        again: None,
        brief: false,
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

/// id Software's licence for the shareware episode, and whether it's agreed to. Nothing is fetched otherwise. The licence is as
/// id Software wrote it, in English, whatever language the rest is in.
pub fn accepts_shareware_license() -> bool {
    crate::tray::explain(t::SHAREWARE_HEADING.t(), &reflowed(crate::wads::SHAREWARE_LICENSE), &[t::SHAREWARE_AGREE.t()]).is_some()
}

/// `text` with each paragraph on one line, for a window to wrap: a text wrapped for a terminal, as a license is, breaks raggedly
/// in a window of any other width. Every word stays, and so do the paragraphs, as runs of non-blank lines.
fn reflowed(text: &str) -> String {
    let paragraphs: Vec<String> = text
        .replace("\r\n", "\n")
        .split("\n\n")
        .map(|p| p.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect();
    paragraphs.join("\n\n")
}

/// What posting a report means, and whether to go ahead.
pub fn wants_to_post() -> bool {
    crate::tray::explain(t::POST_HEADING.t(), t::POST_TEXT.t(), &[t::POST_OPEN.t()]).is_some()
}

/// What PimpMyShell works with, in the README.
const SUPPORTED: &str = "https://github.com/bungholesurfers/PimpMyShell#what-it-works-with";

/// Something else runs the taskbar (`name`, if we know it), so there's nothing for us to hook. Nothing's wrong, and
/// nothing we can do: this one's said kindly, and a little jive.
pub fn taskbar_replaced(name: Option<&str>) -> Notice {
    let (who, running, way_back) = match name {
        Some(n) => (n.to_string(), t::REPLACED_RUNNING.with(&[("name", &n)]), t::REPLACED_WAY_BACK.with(&[("name", &n)])),
        None => (t::REPLACED_SOMEONE.t().to_string(), t::REPLACED_RUNNING_UNKNOWN.t().to_string(), t::REPLACED_WAY_BACK_UNKNOWN.t().to_string()),
    };
    Notice {
        key: "taskbar replaced",
        title: t::REPLACED_TITLE.with(&[("who", &who)]),
        body: t::REPLACED_BODY.t().into(),
        heading: t::REPLACED_HEADING.t().into(),
        detail: t::REPLACED_DETAIL.with(&[("running", &running), ("way_back", &way_back)]),
        fix: Some((t::REPLACED_FIX.t(), SUPPORTED.to_string())),
        keep_native: false,
        opens: None,
        reveal: None,
        hint: t::REPLACED_HINT.with(&[("way_back", &way_back)]),
        again: None,
        brief: false,
    }
}

/// The taskbar wouldn't take a toast: notifications went back to Windows, so that none go missing. `around`: what else
/// changes the taskbar here, which may well be why.
pub fn toasts_refused(error: &str, around: &[&str]) -> Notice {
    let why = if around.is_empty() {
        String::new()
    } else {
        let others = around.join(&format!(" {} ", t::AND.t()));
        format!("{}\n\n", t::REFUSED_WHY.with(&[("others", &others)]))
    };
    Notice {
        key: "toasts",
        title: t::TOASTS_TITLE.t().into(),
        body: t::REFUSED_BODY.t().into(),
        heading: t::POPUPS_HEADING.t().into(),
        detail: t::REFUSED_DETAIL.with(&[("why", &why), ("error", &error)]),
        fix: Some((t::SAVE_REPORT.t(), REPORT.to_string())),
        keep_native: false,
        opens: None,
        reveal: None,
        hint: t::REFUSED_HINT.t().into(),
        again: None,
        brief: false,
    }
}

/// Toasts couldn't start, for a reason other than notification access.
pub fn toasts_failed(error: &str) -> Notice {
    Notice {
        key: "toasts",
        title: t::TOASTS_TITLE.t().into(),
        body: t::FAILED_BODY.t().into(),
        heading: t::POPUPS_HEADING.t().into(),
        detail: t::FAILED_DETAIL.with(&[("error", &error)]),
        fix: None,
        keep_native: false,
        opens: None,
        reveal: None,
        hint: t::FAILED_HINT.t().into(),
        again: None,
        brief: false,
    }
}

/// The config at `path` didn't load. `kept`: the taskbar keeps the one that last did; otherwise PimpMyShell started
/// without one (a stock taskbar).
pub fn config(path: &Path, error: &str, kept: bool) -> Notice {
    let name = file_name(path, "pms.toml");
    let problem = crate::problem::Problem::of(path, error);
    let error = problem.words();
    let now = if kept { t::CONFIG_KEPT.t() } else { t::CONFIG_STOCK.t() };
    Notice {
        key: "config",
        title: t::CONFIG_TITLE.t().into(),
        body: t::CONFIG_BODY.with(&[("now", &now)]),
        heading: t::PROBLEM_IN.with(&[("name", &name)]),
        detail: t::CONFIG_DETAIL.with(&[("now", &now), ("error", &error)]),
        fix: Some((t::CONFIG_FIX.t(), path.display().to_string())),
        keep_native: false,
        opens: None,
        reveal: Some(path.display().to_string()),
        hint: t::CONFIG_HINT.with(&[("now", &now), ("name", &name)]),
        again: None,
        brief: false,
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
        Choice::Reveal => {
            if let Some(file) = &n.reveal {
                show_file(Path::new(file));
            }
        }
        Choice::Fix => {
            let Some((_, target)) = &n.fix else { return };
            if target == REPORT {
                return crate::tray::queue(crate::tray::Command::SaveReport);
            }
            if target == TWATCH_SETTINGS {
                return crate::tray::open_settings_on("twatch");
            }
            if let Some(page) = target.strip_prefix(SETTINGS_ON) {
                return crate::tray::open_settings_on(page);
            }
            if target == RESTART_EXPLORER {
                // The heading says it wasn't, so the words only need say why.
                let asked = match crate::explorer::busy() {
                    Some(why) => Err(crate::explorer::why_not(why)),
                    None => crate::explorer::ask(),
                };
                if let Err(why) = asked {
                    crate::tray::explain(t::NOT_RESTARTED.t(), &why, &[]);
                }
                return;
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
    /// *Show in File Explorer* (`reveal`).
    Reveal,
    /// Close, its x, Esc.
    Close,
}

/// The notice's explanation, waiting for its answer (examples/dialogs.rs shows them so; `explain` acts on it).
pub fn choose(n: &Notice) -> Choice {
    // Told apart by which they are, not by what they say: the words are the language's.
    let mut choices: Vec<(&str, Choice)> = n.fix.iter().map(|(label, _)| (*label, Choice::Fix)).collect();
    if n.keep_native {
        choices.push((t::KEEP_NATIVE.t(), Choice::KeepNative));
    }
    if n.reveal.is_some() {
        choices.push((t::REVEAL.t(), Choice::Reveal));
    }
    let labels: Vec<&str> = choices.iter().map(|(label, _)| *label).collect();
    match crate::tray::explain(&n.heading, &n.detail, &labels) {
        Some(i) if i < choices.len() => choices.swap_remove(i).1,
        _ => Choice::Close,
    }
}

/// A file, picked out in a File Explorer window.
pub fn show_file(path: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg("/select,").arg(path).spawn();
}

/// A file in whatever opens it, or Notepad where nothing does (a .toml often has no app).
pub fn open_file(path: &str) -> Result<(), String> {
    crate::toasts::open(path).or_else(|_| {
        std::process::Command::new("notepad.exe").arg(path).spawn().map(drop).map_err(|e| format!("notepad {path}: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favorites_going_live_are_said_in_few_words() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(favorite_live(&names(&["crab"])).title, "crab is live");
        assert_eq!(favorite_live(&names(&["crab", "lobster"])).title, "crab and lobster are live");
        assert_eq!(favorite_live(&names(&["crab", "lobster", "shrimp"])).title, "crab and 2 more are live");
        assert_eq!(favorite_live(&names(&["crab", "lobster"])).opens.as_deref(), Some("https://www.twitch.tv/crab"));
    }

    #[test]
    fn asking_about_explorer_restarts_nothing_by_itself() {
        let n = explorer_heavy(44, 360);
        assert!(n.detail.contains("44") && n.detail.contains("360 MB"));
        assert_eq!(n.fix.as_ref().map(|f| f.1.as_str()), Some(RESTART_EXPLORER), "it's a button you press");
        assert_eq!(n.again, None, "and it's said once");
    }

    #[test]
    fn a_favorite_going_live_is_up_for_the_usual_time_and_a_problem_for_the_long_one() {
        assert!(favorite_live(&["one".into()]).brief);
        assert!(!console_spared().brief && !notification_access().brief);
    }

    #[test]
    fn a_long_explanation_has_no_gaps_in_it() {
        for n in [console_spared(), explorer_heavy(44, 360)] {
            assert!(!n.detail.contains("  "), "{}", n.detail);
        }
    }

    #[test]
    fn the_license_keeps_every_word_and_paragraph_when_reflowed() {
        let license = crate::wads::SHAREWARE_LICENSE;
        let shown = reflowed(license);
        let words = |t: &str| t.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(words(&shown), words(license), "not a word changed, added or lost");
        assert!(shown.contains("thatd"), "even the original's typo");
        assert!(shown.split("\n\n").count() > 5, "the paragraphs stay apart");
        assert!(!shown.split("\n\n").any(|p| p.contains('\n')), "each on one line, for the window to wrap");
    }

    #[test]
    fn raised_once_or_again() {
        let mut n = Notices::default();
        n.raise(notification_access());
        n.raise(config(Path::new(r"C:\x\pms.toml"), r"C:\x\pms.toml: TOML parse error at line 3", true));
        let now = Instant::now();
        let due = n.due(now);
        assert_eq!(due.len(), 2);
        assert!(due.iter().all(|(id, _)| Notices::is_ours(*id)));
        assert_eq!(due[1].1.detail.lines().nth(2), Some("Line 3"));
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

}
