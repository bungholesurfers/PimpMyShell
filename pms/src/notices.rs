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
    /// What a click opens, in place of an explanation: there's nothing to explain.
    pub opens: Option<String>,
    /// A file to show in File Explorer, with a button of its own.
    pub reveal: Option<String>,
    /// One line on what to do, for where there's only a Windows notification to say it.
    pub hint: String,
    /// While it lasts, it comes again this often; None: once.
    pub again: Option<Duration>,
}

/// Favorites that have just gone live, best first. There's nothing to do about it, so a click goes to the first one's channel.
pub fn favorite_live(handles: &[String]) -> Notice {
    let title = match handles {
        [one] => format!("{one} is live"),
        [one, two] => format!("{one} and {two} are live"),
        [one, rest @ ..] => format!("{one} and {} more are live", rest.len()),
        [] => String::new(),
    };
    Notice {
        key: "favorite live",
        title,
        body: "On Twitch now. Select to watch.".into(),
        heading: String::new(),
        detail: String::new(),
        fix: None,
        keep_native: false,
        opens: handles.first().map(|h| format!("https://www.twitch.tv/{h}")),
        reveal: None,
        hint: "On Twitch now.".into(),
        again: None,
    }
}

/// Memory is low, and the console would have gone, but it has a game someone's been playing.
pub fn console_spared() -> Notice {
    Notice {
        key: "console spared",
        title: "The PIMPBOX is still running, though memory is low".into(),
        body: "You've been playing, so we didn't close your game.".into(),
        heading: "We left your game running".into(),
        detail: "Your PC is low on memory, and the PIMPBOX would usually close to free some. You've played this game, and closing it \
                 would lose anything you haven't saved, so it keeps running until it's restarted.\n\n\
                 To free that memory yourself, save your game and switch the console off on the TV's remote."
            .into(),
        fix: None,
        keep_native: false,
        opens: None,
        reveal: None,
        hint: "You've been playing, so we didn't close your game.".into(),
        again: None,
    }
}

/// Where a restart of Explorer is the fix.
const RESTART_EXPLORER: &str = "pms:restart-explorer";

/// Explorer is holding `taps` of ours and `megabytes` of memory. Restarting it is for you to choose: this only says so.
pub fn explorer_heavy(taps: usize, megabytes: u64) -> Notice {
    Notice {
        key: "explorer",
        title: "Explorer could use a restart".into(),
        body: "Old copies of PimpMyShell are taking up its memory. Select for details.".into(),
        heading: "Restart Explorer to free up memory?".into(),
        detail: format!(
            "Windows can't unload PimpMyShell from Explorer, so every update leaves an old copy behind until Explorer restarts. It has {taps} \
             copies now, and it's using {megabytes} MB.\n\n\
             Your taskbar goes away for a few seconds. We won't restart Explorer while a File Explorer window, a file copy or an Explorer \
             dialog is open in it.\n\n\
             Nothing restarts unless you select Restart Explorer."
        ),
        fix: Some(("Restart Explorer", RESTART_EXPLORER.into())),
        keep_native: false,
        opens: None,
        reveal: None,
        hint: "You can restart it in PimpMyShell's Settings, under General.".into(),
        again: None,
    }
}

/// Lines of the watchlist at `path` that can't be channels: skipped, and the rest are in use.
pub fn watchlist(path: &Path, bad: &[(usize, String)]) -> Notice {
    let name = path.file_name().map_or("the watchlist".into(), |n| n.to_string_lossy().into_owned());
    let lines: Vec<String> = bad.iter().take(6).map(|(n, text)| format!("Line {n}: {text}")).collect();
    let more = bad.len().saturating_sub(lines.len());
    let rest = if more > 0 { format!("\n...and {more} more") } else { String::new() };
    Notice {
        key: "watchlist",
        title: "Some of your favorites were skipped".into(),
        body: format!("{name} has lines that can't be channels. Select to see them."),
        heading: format!("There's a problem in {name}"),
        detail: format!("These aren't channel names, so they were skipped. The rest of your favorites are working.\n\n{}{rest}\n\nFix them and save the file. Your changes take effect right away.", lines.join("\n")),
        fix: Some(("Open watchlist", path.display().to_string())),
        keep_native: false,
        opens: None,
        reveal: Some(path.display().to_string()),
        hint: format!("Fix {name} and save it."),
        again: None,
    }
}

/// Where the settings window opens on a page of its own: this, then the page's tag.
const SETTINGS_ON: &str = "pms:settings:";

/// Settings in the file at `path` that couldn't be used (`config.issues`): each back to its default, or left out, and the rest
/// loaded. Said once for each time the file changes and still has some, as an update can bring them about as easily as a slip.
pub fn settings_issues(path: &Path, issues: &[String]) -> Notice {
    let name = path.file_name().map_or("pms.toml".into(), |n| n.to_string_lossy().into_owned());
    let said: Vec<String> = issues.iter().take(6).map(|e| crate::problem::Problem::of(path, e).words()).collect();
    let more = issues.len().saturating_sub(said.len());
    let rest = if more > 0 { format!("\n\n...and {more} more") } else { String::new() };
    let page = issues.first().map_or("general", |e| crate::config::settings_page(&crate::config::issue_setting(e)));
    Notice {
        key: "settings issues",
        title: "Some of your settings couldn't be used".into(),
        body: format!("Everything else in {name} loaded. Select to see which."),
        heading: format!("Some settings in {name} couldn't be used"),
        detail: format!(
            "Each of these is back to its default for now, or left out, and everything else in your settings loaded. That can happen \
             after an update, as well as after a slip.\n\n{}{rest}\n\nSettings marks each in red where it belongs. Choose a value there, \
             or fix the file and save it.",
            said.join("\n\n")
        ),
        fix: Some(("Open Settings", format!("{SETTINGS_ON}{page}"))),
        keep_native: false,
        opens: None,
        reveal: Some(path.display().to_string()),
        hint: "Settings marks them in red where they belong.".into(),
        again: None,
    }
}

/// The network settings' file at `path` won't load, so nothing goes online: there's no knowing whether a proxy was wanted, and
/// going without one might be what mustn't happen.
pub fn network_settings(path: &Path, error: &str) -> Notice {
    let name = path.file_name().map_or("pms-network.toml".into(), |n| n.to_string_lossy().into_owned());
    let problem = crate::problem::Problem::of(path, error).words();
    Notice {
        key: "network",
        title: format!("Offline until {name} is fixed"),
        body: "Your network settings didn't load, so PimpMyShell isn't going online. Select for details.".into(),
        heading: format!("There's a problem in {name}"),
        detail: format!(
            "Your network settings didn't load, so PimpMyShell can't tell whether you wanted a proxy. Rather than risk going \
             online without one, it doesn't go online at all: no checking who's live, no Twatch TV video, no update checks and no \
             downloads.\n\n{problem}\n\nFix the file and save it, and everything goes back to normal by itself. To go without a proxy, \
             delete the file."
        ),
        fix: Some(("Open file", path.display().to_string())),
        keep_native: false,
        opens: None,
        reveal: Some(path.display().to_string()),
        hint: format!("Nothing goes online until {name} is fixed."),
        again: None,
    }
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
        opens: None,
        reveal: None,
        hint: "To fix this, turn on notification access in Settings > Privacy & security > Notifications.".into(),
        again: Some(AGAIN),
    }
}

/// Toasts couldn't start, for a reason other than notification access.
/// Where a newer Streamlink is got: the fix that opens the settings.
const TWATCH_SETTINGS: &str = "pms:twatch";

/// Streamlink couldn't open a live channel, and there's a newer one. Nothing's been fetched: it's new code from elsewhere, so
/// it's for you to say, knowing what's been checked and what hasn't.
pub fn newer_streamlink(version: &str) -> Notice {
    let have = crate::streamlink::version().unwrap_or_default();
    Notice {
        key: "streamlink",
        title: format!("Streamlink {version} may fix Twatch TV"),
        body: "It couldn't open a channel that's live. Select to see your options.".into(),
        heading: format!("Update Streamlink to {version}?"),
        detail: format!(
            "Streamlink {have} couldn't open a channel that Twitch says is live. That usually means Twitch changed something, and \
             a newer Streamlink is usually how that gets fixed.\n\n\
             Nothing has been downloaded. If you update, we'll only accept a build that GitHub's runners uploaded and that matches \
             GitHub's digest. That proves it's the file the Streamlink project published. It doesn't prove the file is safe.\n\n\
             You can read about this release at {}. To update, open the settings and select Get the newest.",
            crate::streamlink::URL
        ),
        fix: Some(("Open Twatch TV settings", TWATCH_SETTINGS.into())),
        keep_native: false,
        opens: None,
        reveal: None,
        hint: format!("Streamlink {version} may fix Twatch TV. You can get it in Settings, under Twatch TV."),
        again: None,
    }
}

/// A favorite is live and a set would play it, but Streamlink isn't here. Once, and again only after Streamlink has been here and
/// gone (twatch/mod.rs): the set itself says so for as long as it lasts.
pub fn no_streamlink() -> Notice {
    Notice {
        key: "streamlink",
        title: "Twatch TV needs Streamlink".into(),
        body: "A favorite is live, but there's nothing to play it with. Select to see your options.".into(),
        heading: "Get Streamlink for live video?".into(),
        detail: format!(
            "One of your favorites is live, but your TwaTVs need Streamlink to play live video, and it isn't installed. It's a separate, \
             free program that turns a channel into a stream. It isn't in PimpMyShell's installer, and nothing is downloaded until you ask.\n\n\
             To get it, open the settings and select Download, under Streamlink. It's about 85 MB, the newest stable build from {}, and we \
             only accept a build that GitHub's runners uploaded and that matches GitHub's digest.\n\n\
             If you'd rather not, choose Always under Pictures instead of video. Your TwaTVs then show a new picture every minute, which \
             doesn't need Streamlink.",
            crate::streamlink::URL
        ),
        fix: Some(("Open Twatch TV settings", TWATCH_SETTINGS.into())),
        keep_native: false,
        opens: None,
        reveal: None,
        hint: "A favorite is live, but live video needs Streamlink. You can get it in Settings, under Twatch TV.".into(),
        again: None,
    }
}

/// Part of PimpMyShell (`file`, from parts.rs) isn't beside the rest, so what needs it can't work.
pub fn missing_part(file: &'static str) -> Notice {
    let dir = crate::parts::path(file).and_then(|p| p.parent().map(crate::shown)).unwrap_or_default();
    let without = crate::parts::without(file);
    Notice {
        key: file,
        title: "Part of PimpMyShell is missing".into(),
        body: format!("{file} isn't there, so {without}. Select to see how to fix it."),
        heading: format!("{file} is missing"),
        detail: format!(
            "PimpMyShell's {file} should be in {dir}, beside pmsd.exe, but it isn't, so {without}.\n\n\
             Something may have taken it away: an antivirus program, a cleanup tool, or an update that didn't finish. If your \
             antivirus did, it may say why, and let you put it back.\n\n\
             Otherwise, installing PimpMyShell again puts it back, and keeps your settings. The installer is on the releases page."
        ),
        fix: Some(("Open the releases page", crate::update::RELEASES_PAGE.into())),
        keep_native: false,
        opens: None,
        reveal: crate::parts::path("pmsd.exe").map(|p| p.display().to_string()),
        hint: format!("{file} is missing, so {without}. Installing PimpMyShell again puts it back."),
        again: None,
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
        opens: None,
        reveal: None,
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

/// id Software's licence for the shareware episode, and whether it's agreed to. Nothing is fetched otherwise.
pub fn accepts_shareware_license() -> bool {
    crate::tray::explain("Doom shareware: id Software's license", &reflowed(crate::wads::SHAREWARE_LICENSE), &["I agree, download it"]).is_some()
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
        opens: None,
        reveal: None,
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
        opens: None,
        reveal: None,
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
        opens: None,
        reveal: None,
        hint: "They appear as standard pop-ups for now. We'll keep trying.".into(),
        again: None,
    }
}

/// The config at `path` didn't load. `kept`: the taskbar keeps the one that last did; otherwise PimpMyShell started
/// without one (a stock taskbar).
pub fn config(path: &Path, error: &str, kept: bool) -> Notice {
    let name = path.file_name().map_or("pms.toml".into(), |n| n.to_string_lossy().into_owned());
    let problem = crate::problem::Problem::of(path, error);
    let error = problem.words();
    let now = if kept { "PimpMyShell is still using your last working settings." } else { "PimpMyShell started with the standard taskbar." };
    Notice {
        key: "config",
        title: "We couldn't load your settings".into(),
        body: format!("{now} Select to see what's wrong."),
        heading: format!("There's a problem in {name}"),
        detail: format!("{now}\n\n{error}\n\nFix the problem and save the file. Your changes take effect right away. If you'd rather go back, the Settings window can restore your last working settings."),
        fix: Some(("Open settings file", path.display().to_string())),
        keep_native: false,
        opens: None,
        reveal: Some(path.display().to_string()),
        hint: format!("{now} Fix {name} and save it. Your changes take effect right away."),
        again: None,
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
                if let Err(why) = crate::explorer::ask() {
                    // The heading says it wasn't, so the words only need say why.
                    let because = why.strip_prefix("Explorer wasn't restarted, because ").map_or(why.clone(), |rest| {
                        let mut letters = rest.chars();
                        letters.next().map_or(String::new(), |first| first.to_uppercase().chain(letters).collect())
                    });
                    crate::tray::explain("Explorer wasn't restarted", &because, &[]);
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
    const KEEP_NATIVE: &str = "Keep Windows notifications";
    const REVEAL: &str = "Show in File Explorer";
    let mut choices: Vec<&str> = n.fix.iter().map(|(label, _)| *label).collect();
    if n.keep_native {
        choices.push(KEEP_NATIVE);
    }
    if n.reveal.is_some() {
        choices.push(REVEAL);
    }
    match crate::tray::explain(&n.heading, &n.detail, &choices).map(|i| choices[i]) {
        Some(KEEP_NATIVE) => Choice::KeepNative,
        Some(REVEAL) => Choice::Reveal,
        Some(_) => Choice::Fix,
        None => Choice::Close,
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
