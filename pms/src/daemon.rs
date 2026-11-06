//! `pms daemon`: keep the taskbar looking the way `pms.toml` says, for as long as it
//! runs. Its loop wakes for meters, toasts and microphone changes, and:
//!
//!   1. once a second, asks the TAP for `status`. No TAP (explorer restarted, first run) -> inject
//!      one, once the taskbar's XAML island exists. A TAP of another build (from before an update)
//!      -> swap ours in (up to MAX_SWAPS times). A TAP copy or taskbar tree it hasn't configured
//!      yet, or one that drifted (someone ran `reset`) -> `reset` and apply the whole config again.
//!   2. shows or hides the hot-mic alert,
//!   3. samples the perf counters that are due and pushes the meters.
//!
//! The config file is reloaded when it changes. `pms daemon stop` (or Ctrl+C, or the tray
//! icon's Exit) restores the stock taskbar and exits; the TAP stays loaded, inert until the next
//! daemon. Paused (from the tray), or benched by the watchdog (watchdog.rs: explorer crashing,
//! or hooking it failing), the daemon keeps to itself: stock taskbar, native notifications.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::*;
use windows_core::{BOOL, HSTRING, PCWSTR, w};

use crate::config::{self, Config};
use crate::hotmic::{self, MicWatch};
use crate::meters::Sampler;
use crate::notices::{self, Notices};
use crate::system;
use crate::toasts::{self, Banners, Incoming, Listener};
use crate::tray::{self, Tray};
use crate::watchdog::Watchdog;
use crate::{inject, log, send, taskbar_pid, update};

const STOP_EVENT: PCWSTR = w!(r"Local\PimpMyShellDaemonStop");
const INSTANCE_MUTEX: PCWSTR = w!(r"Local\PimpMyShellDaemon");
/// Consecutive failed exchanges with a TAP whose pipe exists before we replace it.
const MAX_FAILURES: u32 = 3;
const INJECT_BACKOFF: Duration = Duration::from_secs(5);
const APPLY_BACKOFF: Duration = Duration::from_secs(5);
const STATUS_EVERY: Duration = Duration::from_secs(1);
/// How often the mic loudness is read while the mic is hot.
const LEVEL_EVERY: Duration = Duration::from_millis(66);
/// How long an app that let go of the mic stays in the label.
const CLOSED_FOR: Duration = Duration::from_secs(10);
const TOAST_POLL: Duration = Duration::from_millis(500);
/// A card's time once it comes forward again with more behind it.
const QUEUED_TOAST_MS: u32 = 4000;
/// The most cards on screen at once (newest in front); older ones are let go (they stay in the notification center).
const DECK_MAX: usize = 8;
/// How long clicks on a card that has left are still taken (one may be in the TAP's queue as it goes).
const LATE_CLICKS: Duration = Duration::from_secs(2);
/// Windows' own "long" toast duration.
const LONG_TOAST_MS: u32 = 25_000;
/// How often to ask the TAP for clicks while a toast is up.
const EVENT_POLL: Duration = Duration::from_millis(120);
/// How often to look for apps installed since (their banners go off ahead of their first toast).
const NOTIFIER_SCAN: Duration = Duration::from_secs(600);
/// How often to try listening for notifications again, while Windows won't let us (notification access off): soon
/// after it's turned on, toasts come to the taskbar.
const LISTEN_RETRY: Duration = Duration::from_secs(15);
/// How long a config that didn't load waits before its notice shows: a save that writes the file in two goes
/// (truncated, then filled) loads at the second, and the notice never comes.
const CONFIG_NOTICE_AFTER: Duration = Duration::from_secs(3);
/// How long a shell can go without its XAML taskbar before we say why we can't help: with software that can replace
/// it, and without (where it might just be a slow start, so longer).
const REPLACED_AFTER: Duration = Duration::from_secs(30);
const NO_TASKBAR_AFTER: Duration = Duration::from_secs(120);
/// How often to look for a bar of someone else's over a hidden taskbar.
const SHELL_CHECK: Duration = Duration::from_secs(60);
/// How many toasts in a row the taskbar may refuse before they all go back to Windows: one is a glitch, or one app's odd
/// toast, and not reason enough to give up on the taskbar.
const REFUSALS_ALLOWED: u32 = 2;
/// How long they stay with Windows before we try the taskbar again, doubling while it goes on refusing.
const REFUSED_RETRY: Duration = Duration::from_secs(5 * 60);
const REFUSED_RETRY_MAX: Duration = Duration::from_secs(60 * 60);
/// That the welcome has been offered, here. Nobody should sit through it twice.
const WELCOMED: &str = "welcomed.txt";
/// How many times in a row the meters may fail before we ask for a report.
const METER_FAILURES: u32 = 3;
/// Whose replaced taskbar we've explained: once is plenty, not at every sign-in.
const TOLD: &str = "taskbar-told.txt";

static STOP: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

extern "system" fn on_ctrl(_: u32) -> BOOL {
    unsafe { SetEvent(HANDLE(STOP.load(Ordering::Acquire))) }.is_ok().into()
}

/// Whether a daemon is running (it holds the single-instance mutex).
pub fn running() -> bool {
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, INSTANCE_MUTEX) } {
        Ok(h) => {
            unsafe { CloseHandle(h) }.ok();
            true
        }
        Err(_) => false,
    }
}

/// Signal a running daemon to restore the taskbar and exit.
pub fn stop() -> Result<String, String> {
    let ev = unsafe { OpenEventW(EVENT_MODIFY_STATE, false, STOP_EVENT) }.map_err(|_| "no daemon running".to_string())?;
    let r = unsafe { SetEvent(ev) }.map_err(|e| e.to_string());
    unsafe { CloseHandle(ev) }.ok();
    r.map(|()| "asked the daemon to stop".into())
}

struct Daemon {
    path: PathBuf,
    modified: Option<SystemTime>,
    config: Config,
    sampler: Sampler,
    /// `tap=.. taskbar=..` of the TAP state our config was last applied to.
    applied: Option<String>,
    retry_apply: Instant,
    /// Apps holding the mic (minus ignored ones).
    hot: Vec<String>,
    hot_since: Option<Instant>,
    /// Apps that let go of the mic, and when (shown "stopped N sec. ago" for `CLOSED_FOR`).
    mic_closed: Vec<(String, Instant)>,
    /// An app let go while the whole taskbar warned: settle into the label now rather than at `broad_ms`.
    settle_early: bool,
    app_names: HashMap<String, String>,
    /// What the TAP shows for the mic: (label, settled into local mode).
    mic_state: Option<(String, bool)>,
    /// Loudness and mute, read while the mic is hot; and what the TAP last got of each.
    mic_level: Option<hotmic::MicLevel>,
    next_level: Instant,
    sent_level: f32,
    sent_tide: f32,
    sent_muted: Option<bool>,
    alert_shown: bool,
    failures: u32,
    last_inject: Option<Instant>,
    toasts: Option<ToastFeed>,
    /// The look in force: Windows' light mode (with `theme = "auto"`), or the one configured.
    light: bool,
    /// Last problem logged, so a persistent one is logged once rather than every tick.
    last_note: String,
    watchdog: Watchdog,
    /// Paused from the tray: the stock taskbar until resumed.
    paused: bool,
    tray: Option<Tray>,
    /// The watchdog drill's stand-in was started (PMS_WATCHDOG_DRILL).
    drilled: bool,
    /// The frame-rate cap the TAP has (0 = none).
    fps: u32,
    updates: Updates,
    /// Since when the taskbar hasn't had our config (no TAP, or a TAP that can't see the taskbar's tree).
    unhooked_since: Option<Instant>,
    /// The toast feed waits for the next `apply` to (re)start it: at startup, after a resume or a TAP swap, or once it
    /// was stopped because the taskbar wasn't ours to show toasts on. Native banners are back meanwhile.
    toasts_parked: bool,
    /// Our TAP's build (`pms_tap::build_of` the pms_tap.dll beside us), which the TAP in explorer must match.
    tap_build: Option<String>,
    /// TAPs of another build we've tried to swap ours in for, in a row.
    swaps: u32,
    /// The TAP in explorer is of another build and wouldn't be swapped: it gets the rest of the config, but not the
    /// toasts (its protocol may differ); native banners stay until explorer restarts.
    stale_tap: bool,
    /// The explorer we've put in the log (`survey_shell`), and the taskbar software around it.
    surveyed: Option<u32>,
    taskbar_software: Vec<&'static system::Known>,
    /// Since when the shell has been up without the XAML taskbar we hook, and whether we've said why.
    no_taskbar_since: Option<Instant>,
    told_replaced: bool,
    next_shell_check: Instant,
    /// The taskbar's failures in a row (`failed`).
    meter_failures: u32,
    /// The welcome window is up.
    welcoming: Arc<std::sync::atomic::AtomicBool>,
    /// What PimpMyShell has to tell the user itself (notices.rs).
    notices: Notices,
    /// The config didn't load at the start: this one is empty (a stock taskbar), until a save that loads.
    stand_in: bool,
}

/// How many times in a row a TAP of another build gets swapped for ours before we stop trying.
const MAX_SWAPS: u32 = 2;

/// How long the taskbar may go without our config (explorer restarting, say) before notification banners go back to
/// Windows: toasts the taskbar can't show must not vanish. Short, but past a re-apply's blink.
const TOAST_GRACE: Duration = Duration::from_secs(3);

/// Software update (update.rs), the daemon's part: checks by itself (with `update_check` on) or from the tray, on a
/// thread of their own; tells of a new version once, with a notification and the tray menu; installs it only once
/// the user said yes to the menu's question.
struct Updates {
    /// The result of the check under way, once it's done.
    result: Arc<Mutex<Option<Result<update::Status, String>>>>,
    busy: bool,
    /// The check under way was asked for from the tray: its answer is told whatever it is.
    manual: bool,
    /// When the next check by itself is due (unix seconds).
    next: u64,
    /// What the last check found.
    found: Option<update::Status>,
    /// The version last told of (once a run each).
    told: Option<String>,
    /// The question whether to check by itself waits a while into a run, and is asked once a run until answered.
    since: Instant,
    asked: bool,
    /// A finished update's download was cleared away (a minute in: the installer that started this daemon still
    /// holds its files at first).
    tidied: bool,
}

/// How long into a run the question about software update waits.
const UPDATE_QUESTION_AFTER: Duration = Duration::from_secs(60);
/// How long into a run a finished update's download is cleared away: by then the installer that started this
/// daemon has exited and let go of its files.
const UPDATE_TIDY_AFTER: Duration = Duration::from_secs(60);

/// In-taskbar toasts: where they come from, which native banners we turned off, and what is
/// on screen.
struct ToastFeed {
    /// None until Windows lets us read notifications (`listen`); native banners stay meanwhile, and only our own
    /// cards (notices.rs) come to the deck.
    listener: Option<Listener>,
    next_listen: Instant,
    banners: Banners,
    /// Toasts waiting for the taskbar (it's being set up).
    queue: VecDeque<Incoming>,
    /// The cards on screen, oldest first; the last is in front, counting down to `showing_until`.
    deck: Vec<Incoming>,
    showing_until: Option<Instant>,
    /// Cards that left lately, with when, whose clicks we may still owe an activation.
    gone: VecDeque<(Incoming, Instant)>,
    /// The TAP said it shows no toast: once its last clicks are in, the deck is empty.
    tap_empty: bool,
    next_events: Instant,
    next_poll: Instant,
    /// Apps that could notify, being listed on a worker thread (`toasts::notifiers`).
    scan: Option<std::sync::mpsc::Receiver<Vec<String>>>,
    next_scan: Instant,
    /// The toasts the taskbar has refused in a row.
    refusals: u32,
    /// Toasts are back with Windows (`give_back`) until then, and notices can't come through here meanwhile.
    given_back_until: Option<Instant>,
    /// How long the next `give_back` lasts.
    give_back_for: Duration,
}

impl ToastFeed {
    /// Start listening for notifications, if Windows lets us: then native banners go (to the taskbar from now on),
    /// and so do the notices that said it couldn't. Otherwise a notice says why, and it tries again in a while.
    fn listen(&mut self, cfg: &crate::config::Toasts, notices: &mut Notices, now: Instant) {
        match Listener::new() {
            Ok(l) => {
                self.listener = Some(l);
                self.banners.suppress_known(|aumid| cfg.ignores("", aumid));
                self.scan = Some(scan_notifiers());
                self.next_scan = now + NOTIFIER_SCAN;
                if notices.is_raised("notification access") || notices.is_raised("toasts") {
                    log("listening for notifications at last: they come to the taskbar");
                }
                notices.clear("notification access");
                // A taskbar that was refusing toasts has yet to show one: `shown` says when it's over.
                if self.refusals == 0 {
                    notices.clear("toasts");
                }
            }
            Err(e) => {
                self.next_listen = now + LISTEN_RETRY;
                let notice = if toasts::access_allowed() { notices::toasts_failed(&e) } else { notices::notification_access() };
                if !notices.is_raised(notice.key) {
                    log(format!("notifications stay with Windows: {e} (trying again every {}s)", LISTEN_RETRY.as_secs()));
                }
                notices.raise(notice);
            }
        }
    }

    /// Windows stopped letting us read notifications: their banners go back to it, until `listen` works again.
    fn deafen(&mut self, notices: &mut Notices, now: Instant) {
        log("notification access was turned off: notification banners are back to Windows");
        self.listener = None;
        self.scan = None;
        self.banners.restore();
        self.next_listen = now + LISTEN_RETRY;
        notices.raise(notices::notification_access());
    }

    /// The taskbar took a toast: whatever it was refusing before, that's over.
    fn shown(&mut self, notices: &mut Notices) {
        if self.refusals > 0 {
            self.refusals = 0;
            self.give_back_for = REFUSED_RETRY;
            notices.clear("toasts");
        }
    }

    /// The taskbar refused a toast. Should that go on, whether it's time to `give_back`: once it has, the first
    /// refusal after a retry is enough.
    fn refused(&mut self) -> bool {
        self.refusals += 1;
        self.refusals >= REFUSALS_ALLOWED
    }

    /// The taskbar won't take toasts: with native banners off they'd go missing, so they go back to Windows for a while.
    fn give_back(&mut self, error: &str, around: &[&str], notices: &mut Notices, now: Instant) {
        log(format!("the taskbar won't show toasts, so they're back to Windows for {} min: {error}", self.give_back_for.as_secs() / 60));
        self.given_back_until = Some(now + self.give_back_for);
        self.give_back_for = (self.give_back_for * 2).min(REFUSED_RETRY_MAX);
        self.listener = None;
        self.scan = None;
        self.queue.clear();
        self.deck.clear();
        self.showing_until = None;
        self.banners.restore();
        let _ = send("toast hide");
        if !notices.is_raised("toasts") {
            notices.raise(notices::toasts_refused(error, around));
        }
    }

    /// Clicks may still come: a card shows, or one left only just now.
    fn wants_events(&self, now: Instant) -> bool {
        !self.deck.is_empty() || self.gone.iter().any(|(_, at)| now.duration_since(*at) < LATE_CLICKS)
    }

    /// The card `id` the TAP took away (it was clicked or swiped): out of the deck, or from those just gone.
    fn take(&mut self, id: u32) -> Option<Incoming> {
        if let Some(i) = self.deck.iter().position(|n| n.id == id) {
            return Some(self.deck.remove(i));
        }
        let i = self.gone.iter().position(|(n, _)| n.id == id)?;
        self.gone.remove(i).map(|(n, _)| n)
    }
}

/// How long a card that comes forward stays: shorter while more wait behind it, Windows' long time for those that
/// ask for it, and 0 (no limit) for the kinds it keeps up until they're answered (alarms, calls, reminders).
fn front_ms(cfg: &crate::config::Toasts, n: &Incoming, deck: usize) -> u32 {
    match (n.sticky, n.long, deck > 1) {
        (true, ..) => 0,
        (_, true, _) => cfg.duration_ms.max(LONG_TOAST_MS),
        (_, false, true) => cfg.duration_ms.min(QUEUED_TOAST_MS),
        (_, false, false) => cfg.duration_ms,
    }
}

/// When a card shown now for `ms` leaves: never, for 0.
fn time_up(now: Instant, ms: u32) -> Option<Instant> {
    (ms > 0).then(|| now + Duration::from_millis(ms as u64))
}

/// List the apps that could notify, off the main loop.
fn scan_notifiers() -> std::sync::mpsc::Receiver<Vec<String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
        let _ = tx.send(toasts::notifiers());
    });
    rx
}

/// Start the feed if the config asks for one; otherwise undo banner changes a previous run
/// (maybe one that crashed) left behind. It listens for notifications from its first pump (`listen`).
fn toast_feed(config: &Config) -> Option<ToastFeed> {
    // Start from the user's own settings (a changed ignore list must take effect); `listen` re-suppresses.
    let mut banners = Banners::load();
    banners.restore();
    config.toasts()?;
    let now = Instant::now();
    Some(ToastFeed {
        listener: None,
        next_listen: now,
        banners,
        queue: VecDeque::new(),
        deck: Vec::new(),
        showing_until: None,
        gone: VecDeque::new(),
        tap_empty: false,
        next_events: now,
        next_poll: now,
        scan: None,
        next_scan: now,
        refusals: 0,
        given_back_until: None,
        give_back_for: REFUSED_RETRY,
    })
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// `key=value` field of a status line.
fn field<'a>(status: &'a str, key: &str) -> Option<&'a str> {
    status.split_whitespace().find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

impl Daemon {
    fn note(&mut self, msg: String) {
        if msg != self.last_note {
            log(&msg);
            self.last_note = msg;
        }
    }

    fn reload_if_changed(&mut self) {
        let m = modified(&self.path);
        if m == self.modified {
            return;
        }
        self.modified = m;
        match config::load(&self.path).and_then(|c| Sampler::new(&c).map(|s| (c, s))) {
            Ok((config, sampler)) => {
                log(format!("reloaded {}", self.path.display()));
                self.notices.clear("config");
                self.stand_in = false;
                let old = std::mem::replace(&mut self.config, config);
                self.sampler = sampler;
                let new = &self.config;
                if old.toasts != new.toasts {
                    if let Some(mut feed) = self.toasts.take() {
                        feed.banners.restore();
                        // Cards on screen go with the feed: the TAP keeps a card up until we drop it.
                        if !feed.deck.is_empty()
                            && let Err(e) = send("toast hide")
                        {
                            log(format!("toast hide: {e}"));
                        }
                    }
                    // Started now only while the taskbar has our config; otherwise `apply` starts it.
                    self.toasts = if self.active() && self.applied.is_some() && !self.stale_tap { toast_feed(&self.config) } else { None };
                    if self.toasts.is_none() && self.config.toasts().is_some() {
                        self.toasts_parked = self.active() && !self.stale_tap;
                    }
                }
                // Apply only what changed, so editing (say, from the GUI) doesn't flash the whole
                // taskbar back to stock and rebuild everything on every save. The meters always
                // start over: the new sampler knows nothing of the old strip.
                if self.applied.is_none() {
                    return;
                }
                let mut cmds = Vec::new();
                if old.backdrop != new.backdrop {
                    cmds.push("backdrop off".to_string());
                    cmds.extend(new.backdrop.iter().flat_map(|b| b.commands(self.light)));
                }
                if old.start_glow != new.start_glow || old.app_glow != new.app_glow {
                    cmds.push("buttonglow off".to_string());
                    cmds.extend(new.start_glow.iter().map(|g| g.command()));
                    cmds.extend(new.app_glow.iter().map(|g| g.command()));
                }
                // (A changed frame-rate cap is follow_power()'s, on the next tick.)
                if old.hotmic != new.hotmic && self.mic_state.is_some() {
                    cmds.extend(["alert off".to_string(), "meters mic off".to_string()]);
                    self.mic_state = None;
                    self.sent_muted = None;
                    self.sent_level = -1.0;
                    self.sent_tide = -1.0;
                }
                cmds.extend(self.sampler.define(&self.config.meter_height, &self.config.bar_height));
                let batch = std::iter::once("batch".to_string()).chain(cmds.iter().map(|c| crate::quote(c))).collect::<Vec<_>>().join(" ");
                match send(&batch) {
                    Ok(r) if !r.lines().any(|l| l.starts_with("error")) => self.sync_mic(),
                    Ok(e) | Err(e) => {
                        log(format!("applying the reloaded config: {e}; re-applying everything"));
                        self.applied = None;
                        self.retry_apply = Instant::now();
                    }
                }
            }
            Err(e) => {
                log(format!("keeping the previous config: {e}"));
                self.notices.raise_in(notices::config(&self.path, &e, !self.stand_in), CONFIG_NOTICE_AFTER);
            }
        }
    }

    fn fail(&mut self, e: String) {
        self.failures += 1;
        self.note(format!("TAP trouble ({}/{MAX_FAILURES}): {e}", self.failures));
        if self.failures >= MAX_FAILURES {
            self.inject();
        }
    }

    fn inject(&mut self) {
        self.inject_tap(false);
    }

    /// Inject our TAP (detaching any other). `swap`: in place of a working one of another build; that, done, doesn't
    /// count toward the watchdog's re-injection limit (a swap that fails still counts as a failed injection).
    fn inject_tap(&mut self, swap: bool) {
        if self.last_inject.is_some_and(|t| t.elapsed() < INJECT_BACKOFF) {
            return;
        }
        if taskbar_pid().is_none() {
            self.note("waiting for the taskbar".into());
            self.no_taskbar();
            return;
        }
        self.no_taskbar_since = None;
        self.last_inject = Some(Instant::now());
        match inject() {
            Ok(msg) => {
                log(msg);
                self.failures = 0;
                self.applied = None;
                self.retry_apply = Instant::now();
                if let Some(why) = taskbar_pid().and_then(|pid| self.watchdog.injected(pid, !swap)) {
                    self.bench(why);
                }
            }
            Err(e) => {
                self.note(format!("inject failed: {e}"));
                if let Some(why) = self.watchdog.inject_failed(&e) {
                    self.bench(why);
                }
            }
        }
    }

    /// Puts the shell's versions and what's around it in the log, once per explorer: a report starts with these.
    fn survey_shell(&mut self) {
        let Some(pid) = system::shell_pid().filter(|&pid| self.surveyed != Some(pid)) else { return };
        self.surveyed = Some(pid);
        system::shell_report(pid).iter().for_each(log);
        self.taskbar_software = system::taskbar_software(pid);
    }

    /// The shell is up, but without the XAML taskbar we hook. We go on waiting (switched back, it just works), and
    /// once it's clear this isn't a slow start, say why nothing's happening.
    fn no_taskbar(&mut self) {
        let Some(pid) = system::shell_pid() else {
            self.no_taskbar_since = None;
            return;
        };
        let waited = self.no_taskbar_since.get_or_insert_with(Instant::now).elapsed();
        if self.told_replaced || waited < REPLACED_AFTER {
            return;
        }
        let replacer = system::taskbar_software(pid).into_iter().find(|k| k.role == system::Role::Replaces);
        if replacer.is_some() || waited >= NO_TASKBAR_AFTER {
            self.tell_replaced(replacer.map(|k| k.name));
        }
    }

    /// A bar of someone else's over a hidden taskbar: cards and meters would go on a taskbar nobody sees, so we stop.
    fn check_taskbar_hidden(&mut self, now: Instant) {
        if now < self.next_shell_check {
            return;
        }
        self.next_shell_check = now + SHELL_CHECK;
        if !self.active() || !system::taskbar_hidden() {
            return;
        }
        let Some(pid) = system::shell_pid() else { return };
        if let Some(k) = system::taskbar_software(pid).into_iter().find(|k| k.role == system::Role::Hides) {
            self.watchdog.bench(format!("{} has the keys to your taskbar", k.name));
            self.quiet();
            let _ = send("detach");
            self.tell_replaced(Some(k.name));
        }
    }

    /// Explains that `name` runs the taskbar: in the log each run, on screen the first time (for that name).
    fn tell_replaced(&mut self, name: Option<&str>) {
        self.told_replaced = true;
        let who = name.unwrap_or("unknown");
        log(format!("no Windows 11 taskbar to hook (running it: {who})"));
        let told = pms_tap::data_dir().join(TOLD);
        if std::fs::read_to_string(&told).is_ok_and(|t| t.trim() == who) {
            return;
        }
        let _ = std::fs::write(&told, who);
        let notice = notices::taskbar_replaced(name);
        std::thread::spawn(move || {
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            notices::explain(&notice);
        });
    }

    /// Keep the watchdog on the explorer we're talking to, whether or not we injected it: a daemon started while a TAP
    /// was there already (after a restart of the daemon, say) watches it too.
    ///
    /// PMS_WATCHDOG_DRILL=<exe> watches a stand-in instead, started here, which crashes in a few seconds: the
    /// crash_dummy example, or a copy named pms_tap_*.exe to crash "in PimpMyShell's code". All that follows is
    /// real: the stock taskbar, the tray's warning, its Retry. (See examples/watchdog_drill.rs for the rest.)
    fn watch_explorer(&mut self) {
        if let Some(exe) = std::env::var_os("PMS_WATCHDOG_DRILL").map(PathBuf::from) {
            if !self.drilled {
                self.drilled = true;
                use std::os::windows::process::CommandExt as _;
                match std::process::Command::new(&exe).arg("8").creation_flags(CREATE_DEFAULT_ERROR_MODE.0).spawn() {
                    Ok(c) => {
                        let app = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        log(format!("watchdog drill: watching {app} (pid {}) in place of explorer; it crashes in 8 s", c.id()));
                        self.watchdog.watch(c.id(), &app);
                    }
                    Err(e) => log(format!("watchdog drill: {}: {e}", exe.display())),
                }
            }
            return;
        }
        if let Some(pid) = taskbar_pid().filter(|&pid| self.watchdog.watching() != Some(pid)) {
            self.watchdog.watch(pid, "explorer.exe");
        }
    }

    /// Whether we're dressing the taskbar: not paused, and the watchdog hasn't given up.
    fn active(&self) -> bool {
        !self.paused && self.watchdog.benched().is_none()
    }

    /// Back to the stock taskbar and native notifications, keeping the TAP (if any) inert.
    fn quiet(&mut self) {
        match send("reset") {
            Ok(r) => log(format!("stock taskbar: {r}")),
            Err(e) if crate::no_tap(&e) => {}
            Err(e) => log(format!("stock taskbar: TAP unreachable ({e})")),
        }
        if let Some(mut f) = self.toasts.take() {
            f.banners.restore();
        }
        self.applied = None;
        self.mic_state = None;
        self.mic_level = None;
        self.alert_shown = false;
        self.sync_tray();
    }

    fn resume(&mut self) {
        // The toast feed starts again once the config is applied again.
        self.toasts = None;
        self.toasts_parked = true;
        self.unhooked_since = None;
        self.applied = None;
        self.retry_apply = Instant::now();
        self.last_inject = None;
        self.swaps = 0;
        self.failures = 0;
        self.sync_tray();
    }

    /// The watchdog gave up on explorer: leave it be, and tell the user why.
    fn bench(&mut self, why: String) {
        self.quiet();
        // A TAP that keeps failing goes inert for good (until a retry injects a fresh one).
        let _ = send("detach");
        if let Some(t) = &self.tray {
            t.notify("PimpMyShell stopped", &format!("{why}. Your taskbar is back to normal. To try again, select Try again in the PimpMyShell menu in the system tray. Select Save a report there to help us fix this."));
        }
    }

    fn sync_tray(&self) {
        if let Some(t) = &self.tray {
            let update = match &self.updates.found {
                Some(update::Status::Ready { version, .. }) => Some(tray::Offer { version: version.clone(), ready: true }),
                Some(update::Status::Available { version }) => Some(tray::Offer { version: version.clone(), ready: false }),
                _ => None,
            };
            t.set_state(tray::State { paused: self.paused, benched: self.watchdog.benched().map(str::to_string), update });
        }
    }

    /// Starts a check, unless one is under way. `manual`: asked for from the tray, rather than one by itself.
    fn check_update(&mut self, manual: bool) {
        if self.updates.busy {
            return;
        }
        self.updates.busy = true;
        self.updates.manual = manual;
        let ask = update::Ask { install: update::installed(), automatic: !manual, force: false, latest: self.config.latest_channel() };
        log(format!("update: checking the {} channel{}", if ask.latest { "latest" } else { "stable" }, if manual { ", as asked" } else { "" }));
        let result = self.updates.result.clone();
        let spawned = std::thread::Builder::new().name("update".into()).spawn(move || {
            let found = update::check(ask, &|_, _| {});
            *result.lock().unwrap() = Some(found);
        });
        if let Err(e) = spawned {
            self.updates.busy = false;
            log(format!("update: no thread for the check: {e}"));
        }
    }

    /// Software update's part of each tick: the question (once), the checks by themselves when due, and telling of
    /// what a finished check found.
    fn pump_updates(&mut self) {
        let now = update::unix_now();
        if !self.updates.tidied && self.updates.since.elapsed() >= UPDATE_TIDY_AFTER && !self.updates.busy {
            self.updates.tidied = true;
            update::tidy();
        }
        // (Not while the config is a stand-in: the real one may well have the answer.)
        if self.config.update_check.is_none()
            && !self.stand_in
            && !self.updates.asked
            && self.updates.since.elapsed() >= UPDATE_QUESTION_AFTER
            && !self.welcoming.load(Ordering::Acquire)
            && let Some(t) = &self.tray
        {
            self.updates.asked = true;
            t.ask_update_consent(self.config.latest_channel());
        }
        let waiting = matches!(self.updates.found, Some(update::Status::Ready { .. }));
        if self.config.update_check == Some(true) && !self.updates.busy && !waiting && now >= self.updates.next {
            self.check_update(false);
        }
        let Some(result) = self.updates.result.lock().unwrap().take() else { return };
        self.updates.busy = false;
        let manual = std::mem::take(&mut self.updates.manual);
        let told = match result {
            Ok(status) => {
                update::checked();
                self.updates.next = now + update::random_secs(update::EVERY);
                log(format!("update: {status:?}"));
                let told = match &status {
                    update::Status::Ready { version, .. } => Some((
                        version.clone(),
                        format!("PimpMyShell {version} is ready to install"),
                        "It's downloaded and verified. To install it, select Install in the PimpMyShell menu in the system tray.".to_string(),
                    )),
                    update::Status::Available { version } => Some((
                        version.clone(),
                        format!("PimpMyShell {version} is available"),
                        "This copy can't install updates. To get it, select it in the PimpMyShell menu in the system tray.".to_string(),
                    )),
                    // Only a check by itself finds this (one asked for downloads it again).
                    update::Status::Stuck { version } => Some((
                        version.clone(),
                        format!("PimpMyShell {version} didn't install"),
                        "You still have the earlier version. To try again, check for updates in settings, or download it from the releases page.".to_string(),
                    )),
                    update::Status::UpToDate { stable } if manual => Some((
                        String::new(),
                        "You're up to date".to_string(),
                        format!(
                            "You have PimpMyShell {}. The latest {} is {stable}.",
                            crate::version::CURRENT,
                            if self.config.latest_channel() { "release" } else { "stable version" }
                        ),
                    )),
                    _ => None,
                };
                self.updates.found = Some(status);
                told
            }
            Err(e) => {
                self.updates.next = now + update::random_secs(update::RETRY);
                log(format!("update: the check failed: {e}"));
                manual.then(|| (String::new(), "We couldn't check for updates".to_string(), e))
            }
        };
        self.sync_tray();
        // A version is told of once a run, unless the user asked.
        if let (Some((version, title, text)), Some(t)) = (told, &self.tray)
            && (manual || version.is_empty() || self.updates.told.as_deref() != Some(&version))
        {
            if !version.is_empty() {
                self.updates.told = Some(version);
            }
            t.notify(&title, &text);
        }
    }

    /// Notices that are due while there's no deck to show them in, and none coming (toasts off in the config, the
    /// stock taskbar: paused, given up on, a TAP of another build, or one that can't see the taskbar a while now): a
    /// Windows notification says it instead. Once a second; with a deck, `pump_toasts` shows them.
    fn pump_notices(&mut self) {
        let unhooked = self.unhooked_since.is_some_and(|s| s.elapsed() >= TOAST_GRACE);
        let deck_coming = self.config.toasts().is_some() && self.active() && !self.stale_tap && !unhooked;
        let refused = self.toasts.as_ref().is_some_and(|f| f.given_back_until.is_some());
        let no_taskbar = self.no_taskbar_since.is_some_and(|s| s.elapsed() >= TOAST_GRACE);
        if !refused && !no_taskbar && (self.toasts.is_some() && !unhooked || deck_coming) {
            return;
        }
        let now = Instant::now();
        if !self.notices.any_due(now) {
            return;
        }
        for (_, n) in self.notices.due(now) {
            log(format!("notice, as a Windows notification: {}", n.title));
            if let Some(t) = &self.tray {
                t.notify(&n.title, &n.hint);
            }
        }
    }

    /// The tray menu's Install, which the user said yes to.
    fn install_update(&mut self) {
        let Some(update::Status::Ready { version, path, sha256 }) = self.updates.found.clone() else { return };
        log(format!("update: installing {version} from {}, as asked", path.display()));
        if let Err(e) = update::install(&path, &version, &sha256, false) {
            log(format!("update: {e}"));
            self.updates.found = None;
            self.sync_tray();
            if let Some(t) = &self.tray {
                t.notify(&format!("PimpMyShell {version} didn't install"), &e);
            }
        }
        // Otherwise the installer stops this daemon, installs, and starts the new one, and settings it found open.
    }

    /// The answer to the question whether software update may check by itself: kept in the config.
    fn set_update_consent(&mut self, yes: bool) {
        log(format!("update: checking by itself {}", if yes { "allowed" } else { "declined" }));
        match config::set_top_level(&self.path, "update_check", if yes { "true" } else { "false" }) {
            // The reload picks it up; until then, as answered.
            Ok(()) => self.config.update_check = Some(yes),
            Err(e) => log(format!("update: couldn't keep the answer in the config: {e}")),
        }
        if yes {
            self.updates.next = update::next_check();
        }
    }

    /// A first run: all defaults, in the usual place, and no welcome yet. Its window asks the first questions, among them
    /// software update's, which waits for it. A config that isn't new has been through all this.
    fn welcome(&mut self) {
        let mark = pms_tap::data_dir().join(WELCOMED);
        if mark.exists() || !config::is_default(&self.path) {
            return;
        }
        let Some(exe) = std::env::current_exe().ok().map(|e| e.with_file_name("pms-gui.exe")).filter(|_| config::untouched(&self.config)) else {
            let _ = std::fs::write(&mark, "");
            return;
        };
        let (waiting, config) = (self.welcoming.clone(), self.path.clone());
        waiting.store(true, Ordering::Release);
        std::thread::spawn(move || {
            if let Err(e) = std::process::Command::new(exe).arg("--welcome").arg(config).status() {
                log(format!("no welcome: {e}"));
            }
            let _ = std::fs::write(&mark, "");
            waiting.store(false, Ordering::Release);
        });
    }

    /// Something on the taskbar failed. Once it's clearly not a blip, a notice asks for a report.
    fn failed(&mut self, what: &str, error: &str) {
        self.meter_failures += 1;
        if self.meter_failures == METER_FAILURES {
            log(format!("{what}, {METER_FAILURES} times in a row: asking for a report"));
            self.notices.raise(notices::problem(what, error));
        }
    }

    /// *Shut up, computer*: clicks on notifications go ahead without asking, from now on.
    fn stop_confirming_clicks(&mut self) {
        match config::set_in_table(&self.path, "toasts", "confirm_clicks", "\"off\"") {
            Ok(()) => log("notification clicks won't ask any more"),
            Err(e) => log(format!("couldn't stop asking about notification clicks: {e}")),
        }
    }

    /// The notice about notification access was answered *Keep Windows notifications*: `[toasts]` off in the config
    /// (`enabled = false`, its settings kept). The reload that follows takes the feed down; the notice is over.
    fn keep_native_toasts(&mut self) {
        match config::set_in_table(&self.path, "toasts", "enabled", "false") {
            Ok(()) => {
                log("toasts off, as asked: notifications stay Windows' (the settings' Toasts page turns them on again)");
                self.notices.clear("notification access");
                self.notices.clear("toasts");
            }
            Err(e) => log(format!("couldn't turn toasts off in the config: {e}")),
        }
    }

    /// What the tray's menu asked for. True to exit.
    fn handle_tray(&mut self) -> bool {
        let Some(commands) = self.tray.as_ref().map(Tray::take) else { return false };
        for c in commands {
            match c {
                tray::Command::Exit => return true,
                tray::Command::TogglePause => {
                    self.paused = !self.paused;
                    log(if self.paused { "paused from the tray" } else { "resumed from the tray" });
                    if self.paused { self.quiet() } else { self.resume() }
                }
                tray::Command::Retry => {
                    log("hooking explorer again, from the tray");
                    self.watchdog.retry();
                    if self.active() {
                        self.resume();
                    }
                }
                tray::Command::CheckUpdate => self.check_update(true),
                tray::Command::InstallUpdate => self.install_update(),
                tray::Command::UpdateConsent(yes) => self.set_update_consent(yes),
                tray::Command::KeepNativeToasts => self.keep_native_toasts(),
                tray::Command::StopConfirmingClicks => self.stop_confirming_clicks(),
                tray::Command::SaveReport => {
                    let config = self.path.clone();
                    std::thread::spawn(move || notices::save_report(&config));
                }
                tray::Command::RemoveToast(id) => {
                    if let Some(l) = self.toasts.as_ref().and_then(|f| f.listener.as_ref()) {
                        l.remove(id);
                    }
                }
            }
        }
        false
    }

    /// The TAP in explorer is build `theirs`, not ours: swap ours in (true: wait for it). After MAX_SWAPS tries,
    /// make do with theirs (false: go on applying), minus the toasts.
    fn swap_tap(&mut self, theirs: &str, ours: &str) -> bool {
        if self.swaps < MAX_SWAPS {
            if self.last_inject.is_some_and(|t| t.elapsed() < INJECT_BACKOFF) {
                return true;
            }
            self.swaps += 1;
            log(format!("explorer has a TAP of another build ({theirs}; ours is {ours}), likely from before an update: swapping ours in"));
            self.inject_tap(true);
            return true;
        }
        if !self.stale_tap {
            self.stale_tap = true;
            self.toasts_parked = false;
            if let Some(mut f) = self.toasts.take() {
                f.banners.restore();
            }
            log(format!(
                "the TAP in explorer ({theirs}) wouldn't be swapped for ours ({ours}): it keeps the rest of the config, but notification \
                 banners stay with Windows until explorer restarts"
            ));
        }
        false
    }

    /// Make sure a TAP is there and shows our config.
    fn ensure(&mut self) {
        let status = match send("status") {
            Ok(s) if s.starts_with("error") => return self.fail(s),
            Ok(s) => s,
            Err(e) if crate::no_tap(&e) => return self.inject(),
            // Someone else (REPL, layout watch) holds the pipe; not the TAP's fault.
            Err(e) if crate::busy(&e) => return self.note("TAP pipe busy with another client".into()),
            Err(e) => return self.fail(e),
        };
        self.failures = 0;
        self.watch_explorer();
        // After an update explorer still has the TAP it loaded before, which may not speak our protocol (toasts
        // would fail while native banners are off): swap ours in.
        if let Some(ours) = self.tap_build.clone() {
            let theirs = field(&status, "build").unwrap_or("none (older)").to_string();
            if theirs != ours {
                if self.swap_tap(&theirs, &ours) {
                    return;
                }
            } else {
                self.swaps = 0;
                if std::mem::take(&mut self.stale_tap) {
                    // Ours at last (explorer restarted): toasts again, from the next apply.
                    self.toasts_parked = true;
                }
            }
        }
        // Every card clicked or swiped away (or gone with a reset): the deck empties once their clicks are in.
        if let Some(f) = self.toasts.as_mut().filter(|f| !f.deck.is_empty() && field(&status, "toast") == Some("hidden")) {
            f.tap_empty = true;
        }
        let key = format!("tap={} taskbar={}", field(&status, "tap").unwrap_or("?"), field(&status, "taskbar").unwrap_or("none"));
        if field(&status, "taskbar") == Some("none") {
            self.applied = None;
            return self.note("TAP is up, taskbar tree not there yet".into());
        }
        let want = |b: bool| if b { "on" } else { "off" };
        let drifted = field(&status, "backdrop") != Some(want(self.config.backdrop.is_some()))
            || field(&status, "alert") != Some(want(self.alert_shown))
            // The sampler's count, not the config's: `per_gpu` makes a meter per GPU with dedicated memory, or none.
            || field(&status, "meters") != Some(&self.sampler.len().to_string());
        if self.applied.as_deref() == Some(key.as_str()) && !drifted {
            return;
        }
        if Instant::now() < self.retry_apply {
            return;
        }
        if self.applied.as_deref() == Some(key.as_str()) {
            log(format!("taskbar drifted from the config ({status}); re-applying"));
        }
        self.apply(key);
    }

    fn apply(&mut self, key: String) {
        let mut cmds = vec!["reset".to_string()];
        self.light = self.wants_light();
        cmds.push(format!("theme {}", if self.light { "light" } else { "dark" }));
        self.fps = self.wanted_fps();
        cmds.push(format!("pace {}", self.fps));
        cmds.extend(self.config.backdrop.iter().flat_map(|b| b.commands(self.light)));
        cmds.extend(self.config.start_glow.iter().map(|g| g.command()));
        cmds.extend(self.config.app_glow.iter().map(|g| g.command()));
        if !self.sampler.is_empty() {
            cmds.extend(self.sampler.define(&self.config.meter_height, &self.config.bar_height));
        }
        for cmd in &cmds {
            let r = send(cmd);
            if let Some(e) = match &r {
                Ok(s) if s.starts_with("error") => Some(s.clone()),
                Ok(_) => None,
                Err(e) => Some(e.clone()),
            } {
                self.applied = None;
                self.retry_apply = Instant::now() + APPLY_BACKOFF;
                let what = if cmd.starts_with("meters") { "The meters couldn't start" } else { "PimpMyShell couldn't set up your taskbar" };
                self.failed(what, &e);
                return self.note(format!("applying config: `{cmd}` -> {e}"));
            }
        }
        self.mic_state = None;
        self.sent_muted = None;
        self.sent_level = -1.0;
        self.sent_tide = -1.0;
        self.alert_shown = false;
        if let Some(f) = self.toasts.as_mut() {
            // The reset took the cards down.
            f.showing_until = None;
            f.deck.clear();
            f.tap_empty = false;
        }
        log(format!("applied config to {key}"));
        self.last_note.clear();
        self.applied = Some(key);
        self.meter_failures = 0;
        self.unhooked_since = None;
        if !self.stale_tap && std::mem::take(&mut self.toasts_parked) {
            self.toasts = toast_feed(&self.config);
            if self.toasts.is_some() {
                log("the taskbar is ours again: toasts go to it again");
            }
        }
        self.sync_mic();
    }

    /// While the taskbar goes without our config for longer than TOAST_GRACE, its toasts couldn't be shown: give
    /// the banners back to Windows until it has it again (`apply` takes them back).
    fn park_toasts_if_unhooked(&mut self) {
        if self.applied.is_some() {
            self.unhooked_since = None;
            return;
        }
        let since = *self.unhooked_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= TOAST_GRACE
            && let Some(mut f) = self.toasts.take()
        {
            f.banners.restore();
            self.toasts_parked = true;
            log("the taskbar isn't ours (yet): notification banners are back to Windows until it is");
        }
    }

    /// Whether the light look is wanted now.
    fn wants_light(&self) -> bool {
        match self.config.theme.as_str() {
            "light" => true,
            "dark" => false,
            _ => shell_light(),
        }
    }

    /// The animations' frame-rate cap now: `max_fps` (none unless set), or on battery power `battery_fps` (30 unless
    /// set) if that's lower. 0 = none.
    fn wanted_fps(&self) -> u32 {
        let always = self.config.max_fps.unwrap_or(0);
        if !on_battery() {
            return always;
        }
        match (always, self.config.battery_fps.unwrap_or(BATTERY_FPS)) {
            (0, b) | (b, 0) => b,
            (a, b) => a.min(b),
        }
    }

    /// Follow the power going from the wall to the battery and back (checked once a second), and a changed cap in
    /// the config. The cap applies to animations started after it, so a showing alert starts over with it.
    fn follow_power(&mut self) {
        let fps = self.wanted_fps();
        if self.applied.is_none() || fps == self.fps {
            return;
        }
        self.fps = fps;
        let mut cmds = vec![format!("pace {fps}")];
        if self.mic_state.is_some() {
            cmds.extend(["alert off".to_string(), "meters mic off".to_string()]);
            self.mic_state = None;
            self.sent_muted = None;
            self.sent_level = -1.0;
            self.sent_tide = -1.0;
        }
        let batch = std::iter::once("batch".to_string()).chain(cmds.iter().map(|c| crate::quote(c))).collect::<Vec<_>>().join(" ");
        match send(&batch) {
            Ok(_) => {
                log(format!("animations at {}", if fps == 0 { "the display's full rate".to_string() } else { format!("{fps} fps") }));
                self.sync_mic();
            }
            Err(e) => self.note(format!("frame rate: {e}")),
        }
    }

    /// Follow Windows switching between light and dark (checked once a second).
    fn follow_theme(&mut self) {
        let light = self.wants_light();
        if self.applied.is_none() || light == self.light {
            return;
        }
        self.light = light;
        let mut cmds = vec![format!("theme {}", if light { "light" } else { "dark" })];
        cmds.extend(self.config.backdrop.iter().flat_map(|b| b.look(light)));
        let batch = std::iter::once("batch".to_string()).chain(cmds.iter().map(|c| crate::quote(c))).collect::<Vec<_>>().join(" ");
        match send(&batch) {
            Ok(_) => log(format!("switched to the {} look", if light { "light" } else { "dark" })),
            Err(e) => self.note(format!("theme switch: {e}")),
        }
    }

    fn update_mic(&mut self, mic: &MicWatch) {
        let ignore: Vec<String> = self.config.hotmic.iter().flat_map(|h| &h.ignore).map(|s| s.to_lowercase()).collect();
        let mut hot = mic.poll();
        hot.retain(|app| !ignore.iter().any(|i| app.to_lowercase().contains(i)));
        if hot != self.hot {
            for key in &hot {
                if !self.app_names.contains_key(key) {
                    self.app_names.insert(key.clone(), hotmic::display_name(key));
                }
            }
            let now = Instant::now();
            // A new app listening gets the whole taskbar's warning again; one coming back within `CLOSED_FOR` (OBS
            // restarting, say) just rejoins the label.
            let fresh = hot.iter().any(|k| !self.hot.contains(k) && !self.mic_closed.iter().any(|(c, _)| c == k));
            // Apps that let go stay in the label a while, "(stopped N sec. ago)"; one letting go while the whole
            // taskbar warns settles it into the label at once.
            let broad = self.mic_state.as_ref().is_some_and(|(_, local)| !local);
            for key in self.hot.iter().filter(|k| !hot.contains(k)) {
                self.mic_closed.retain(|(k, _)| k != key);
                self.mic_closed.push((key.clone(), now));
                self.settle_early |= broad;
            }
            self.mic_closed.retain(|(k, _)| !hot.contains(k));
            if fresh {
                self.hot_since = Some(now);
                self.settle_early = false;
            }
            self.hot = hot;
            log(match self.mic_names() {
                None => "mic released".to_string(),
                Some(names) => format!("mic in use by {names}"),
            });
        }
        self.sync_mic();
    }

    /// The label: the apps holding the mic by friendly name (deduplicated), then the ones that let
    /// go within `CLOSED_FOR`, "OBS Studio (stopped 3 sec. ago)".
    fn mic_names(&self) -> Option<String> {
        let name = |key: &String| self.app_names.get(key).map_or(hotmic::friendly(key), String::as_str).to_string();
        let mut names: Vec<String> = Vec::new();
        for key in &self.hot {
            let n = name(key);
            if !names.contains(&n) {
                names.push(n);
            }
        }
        for (key, at) in self.mic_closed.iter().rev().filter(|(_, at)| at.elapsed() < CLOSED_FOR) {
            let n = name(key);
            if !names.iter().any(|m| m == &n || m.starts_with(&format!("{n} ("))) {
                names.push(format!("{n} (stopped {} sec. ago)", at.elapsed().as_secs()));
            }
        }
        (!names.is_empty()).then(|| names.join(", "))
    }

    /// When the mic display next needs updating: the broad warning settling into the local one,
    /// or the next tick of a "stopped N sec. ago" count.
    fn mic_settles_at(&self) -> Option<Instant> {
        let h = self.config.hotmic.as_ref()?;
        let settle = self.hot_since.map(|t| t + Duration::from_millis(h.broad_ms as u64)).filter(|_| self.mic_state.as_ref().is_some_and(|(_, local)| !local));
        let tick = self.mic_closed.iter().map(|(_, at)| {
            let next = at.elapsed().as_secs() + 1;
            *at + Duration::from_secs(next)
        });
        settle.into_iter().chain(tick).min()
    }

    /// Bring the TAP's mic label and alert in line with who holds the mic: broad warning first,
    /// the local one after `broad_ms`, both gone when released.
    fn sync_mic(&mut self) {
        let Some(h) = self.config.hotmic.clone() else { return };
        if self.applied.is_none() {
            return;
        }
        // "stopped" entries expire; with none left and nothing holding the mic, it's over.
        self.mic_closed.retain(|(_, at)| at.elapsed() < CLOSED_FOR);
        if self.hot.is_empty() && self.mic_closed.is_empty() {
            self.hot_since = None;
            self.settle_early = false;
        }
        let settled = |since: Instant| since.elapsed() >= Duration::from_millis(h.broad_ms as u64) || self.settle_early;
        let want = self.mic_names().zip(self.hot_since).map(|(names, since)| (names, settled(since)));
        if want == self.mic_state {
            return;
        }
        let mut cmds = Vec::new();
        match (&want, &self.mic_state) {
            (None, _) => cmds.extend(["alert off".to_string(), "meters mic off".to_string()]),
            (Some((names, local)), have) => {
                if have.as_ref().map(|h| &h.0) != Some(names) {
                    cmds.push(format!("meters mic {}", crate::quote(names)));
                }
                // Showing nothing yet, or settled in the label when a new app wants the whole taskbar again.
                if have.as_ref().is_none_or(|h| h.1 && !*local) {
                    cmds.push(h.command());
                }
                if *local && !have.as_ref().is_some_and(|h| h.1) {
                    cmds.push("alert local".into());
                }
            }
        }
        for cmd in &cmds {
            match send(cmd) {
                Ok(r) if !r.starts_with("error") => {}
                Ok(e) | Err(e) => return self.note(format!("`{cmd}` -> {e}")),
            }
        }
        self.alert_shown = want.is_some();
        self.mic_state = want;
    }

    /// While the mic is hot: feed its loudness to the alert flicker (about 15 times a second) and
    /// its mute state to the label.
    fn pump_mic_level(&mut self) {
        if self.mic_state.is_none() {
            self.mic_level = None;
            return;
        }
        let now = Instant::now();
        if now < self.next_level {
            return;
        }
        self.next_level = now + LEVEL_EVERY;
        if self.mic_level.is_none() {
            match hotmic::MicLevel::open() {
                Ok(m) => self.mic_level = Some(m),
                Err(e) => return self.note(format!("mic level: {e}")),
            }
        }
        let Some(m) = &self.mic_level else { return };
        let muted = m.muted();
        // Peak amplitude to something closer to loudness, quantized so near-silence stays still.
        let peak = if muted { 0.0 } else { m.level() };
        let level = ((peak.sqrt() * 1.3).min(1.0) * 50.0).round() / 50.0;
        // The tide: decibels, -60 dB at the bottom to 0 dB full, so near-silence still shows.
        let tide = if peak <= 0.0 { 0.0 } else { (((20.0 * peak.log10() + 60.0) / 60.0).clamp(0.0, 1.0) * 50.0).round() / 50.0 };
        let mut cmds = Vec::new();
        if self.sent_muted != Some(muted) {
            cmds.push(format!("alert muted {}", muted as u8));
            cmds.push(format!("meters micmute {}", muted as u8));
        }
        if (level - self.sent_level).abs() > 0.001 || (tide - self.sent_tide).abs() > 0.001 {
            cmds.push(format!("alert level {level} {tide}"));
        }
        if cmds.is_empty() {
            return;
        }
        let batch = std::iter::once("batch".to_string()).chain(cmds.iter().map(|c| crate::quote(c))).collect::<Vec<_>>().join(" ");
        if send(&batch).is_ok() {
            (self.sent_muted, self.sent_level, self.sent_tide) = (Some(muted), level, tide);
        }
    }

    fn pump_toasts(&mut self) {
        let (Some(cfg), Some(f)) = (self.config.toasts(), self.toasts.as_mut()) else { return };
        let now = Instant::now();
        match f.given_back_until {
            Some(until) if now < until => return,
            Some(_) => {
                log("trying the taskbar for toasts again");
                f.given_back_until = None;
                f.next_listen = now;
            }
            None => {}
        }
        if f.listener.is_none() && now >= f.next_listen {
            f.listen(cfg, &mut self.notices, now);
        }
        // Apps that haven't notified yet: banners off ahead of their first toast.
        if f.listener.is_none() {
        } else if let Some(rx) = &f.scan {
            match rx.try_recv() {
                Ok(list) => {
                    f.banners.suppress_ahead(&list, |aumid| cfg.ignores("", aumid));
                    f.scan = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => f.scan = None,
            }
        } else if now >= f.next_scan {
            f.next_scan = now + NOTIFIER_SCAN;
            f.scan = Some(scan_notifiers());
        }
        if let Some(listener) = f.listener.as_mut().filter(|_| now >= f.next_poll) {
            f.next_poll = now + TOAST_POLL;
            match listener.poll() {
                Ok(new) => {
                    for n in new {
                        if cfg.ignores(&n.app, &n.aumid) || f.banners.user_silenced(&n.aumid) {
                            continue;
                        }
                        let native = !f.banners.is_managed(&n.aumid);
                        f.banners.suppress(&n.aumid);
                        if native {
                            log(format!("first toast from {}: shown natively; its banners go to the taskbar from now on", n.app));
                        } else if !toasts::windows_accepts_toasts() {
                            log(format!("toast from {} held back by Windows (do not disturb, full screen, ...)", n.app));
                        } else {
                            // The app alone: a toast's words are often someone's name, and logs get sent to people.
                            log(format!("toast from {}", n.app));
                            f.queue.push_back(n);
                        }
                    }
                    // A card with no time limit whose notification is gone (a call that ended) goes too.
                    let taken_back: Vec<u32> = f.deck.iter().filter(|n| n.sticky && !f.listener.as_ref().is_some_and(|l| l.holds(n.id))).map(|n| n.id).collect();
                    for id in taken_back {
                        let was_front = f.deck.last().is_some_and(|n| n.id == id);
                        let Some(n) = f.take(id) else { continue };
                        log(format!("toast from {} taken back by the app", n.app));
                        let next = f.deck.last().filter(|_| was_front).map(|m| front_ms(cfg, m, f.deck.len()));
                        if let Err(e) = send(&format!("toast drop id={id}{}", next.map(|ms| format!(" ms={ms}")).unwrap_or_default())) {
                            log(format!("toast drop: {e}"));
                        }
                        if was_front {
                            f.showing_until = next.and_then(|ms| time_up(now, ms));
                        }
                        f.gone.push_back((n, now));
                    }
                }
                Err(e) if !toasts::access_allowed() => {
                    log(format!("reading notifications: {e}"));
                    f.deafen(&mut self.notices, now);
                }
                Err(e) => log(format!("reading notifications: {e}")),
            }
        }
        // Clicks and swipes on the cards (`<id> open|action <i>|close|swipe`): activate like Windows would, off the
        // loop (an app may take a while to start). The TAP has already taken the card away.
        if f.tap_empty || (f.wants_events(now) && now >= f.next_events) {
            f.next_events = now + EVENT_POLL;
            let front = f.deck.last().map(|n| n.id);
            let events = send("toast events").unwrap_or_default();
            for ev in events.lines() {
                let Some((id, what)) = ev.split_once(' ') else { continue };
                match what {
                    // A card asking stays until it's answered, as long as that takes.
                    "asking" => {
                        let asking = id.parse().ok();
                        if let Some(n) = f.deck.iter_mut().find(|n| Some(n.id) == asking) {
                            n.sticky = true;
                        }
                        if front == asking {
                            f.showing_until = None;
                        }
                        continue;
                    }
                    "quiet" => {
                        tray::queue(tray::Command::StopConfirmingClicks);
                        continue;
                    }
                    _ => {}
                }
                let Some(t) = id.parse().ok().and_then(|id| f.take(id)) else { continue };
                // One of our own: a click explains (the explanation waits for its answer, so off the loop).
                if Notices::is_ours(t.id) {
                    match self.notices.by_card(t.id).filter(|_| what == "open") {
                        Some(n) => {
                            let n = n.clone();
                            log(format!("notice \"{}\" clicked: explaining", n.title));
                            std::thread::spawn(move || {
                                let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
                                notices::explain(&n);
                            });
                        }
                        None => log(format!("notice \"{}\" put away", t.title)),
                    }
                    continue;
                }
                let picked = match what.split_once(' ') {
                    _ if what == "open" => Some((t.launch.0.clone(), t.launch.1)),
                    Some(("action", i)) => i.parse::<usize>().ok().and_then(|i| t.actions.get(i)).map(|a| (a.args.clone(), a.kind)),
                    _ => None,
                };
                let Some((args, kind)) = picked else {
                    let how = if what == "swipe" { "swiped away" } else { "closed" };
                    log(format!("toast from {} {how}; it stays in the notification center", t.app));
                    continue;
                };
                // On a card, that's been asked and answered by now.
                let confirm = cfg.confirm_clicks == "window";
                if let Some(l) = f.listener.as_ref().filter(|_| !confirm) {
                    l.remove(t.id);
                }
                std::thread::spawn(move || {
                    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
                    if confirm {
                        if let Some(plan) = toasts::plan(&t, &args, kind) {
                            let answer = tray::ask_click(&plan.about, &plan.question, &plan.rows);
                            if answer.quiet {
                                tray::queue(tray::Command::StopConfirmingClicks);
                            }
                            if !answer.yes {
                                log(format!("toast from {}: you said no; it stays in the notification center", t.app));
                                return;
                            }
                        }
                        tray::queue(tray::Command::RemoveToast(t.id));
                    }
                    match toasts::activate(&t, &args, kind) {
                        Ok(how) => log(format!("toast click: {how}")),
                        Err(e) => log(format!("toast click on {}: {e}", t.app)),
                    }
                });
            }
            if std::mem::take(&mut f.tap_empty) {
                // The TAP shows none (all clicked away, or a reset): neither do we.
                f.showing_until = None;
                for n in std::mem::take(&mut f.deck) {
                    f.gone.push_back((n, now));
                }
            } else if f.deck.last().map(|n| n.id) != front {
                // The front card was clicked or swiped away: the next one has come forward, and counts down.
                f.showing_until = None;
                if let Some(n) = f.deck.last() {
                    let ms = front_ms(cfg, n, f.deck.len());
                    match send(&format!("toast front id={} ms={ms}", n.id)) {
                        Ok(r) if !r.starts_with("error") => f.showing_until = time_up(now, ms),
                        Ok(e) | Err(e) => log(format!("toast front: {e}")),
                    }
                }
            }
        }
        f.gone.retain(|(_, at)| now.duration_since(*at) < LATE_CLICKS);
        // The front card's time is up: it goes, and the one behind it comes forward.
        if f.showing_until.is_some_and(|t| now >= t) {
            f.showing_until = None;
            if let Some(n) = f.deck.pop() {
                let next = f.deck.last().map(|m| front_ms(cfg, m, f.deck.len()));
                let cmd = format!("toast drop id={}{}", n.id, next.map(|ms| format!(" ms={ms}")).unwrap_or_default());
                if let Err(e) = send(&cmd) {
                    log(format!("toast drop: {e}"));
                }
                f.showing_until = next.and_then(|ms| time_up(now, ms));
                f.gone.push_back((n, now));
            }
        }
        if self.applied.is_none() {
            return;
        }
        // Our own notices that are due, unless Windows holds toasts back (do not disturb, full screen, ...): then
        // they wait.
        if self.notices.any_due(now) && toasts::windows_accepts_toasts() {
            let logo = tray::logo_file().ok();
            for (id, n) in self.notices.due(now) {
                log(format!("notice: {}", n.title));
                f.queue.push_back(Incoming::own(id, &n.title, &n.body, logo.clone()));
            }
        }
        // New toasts go on top, in front of the ones still showing.
        let mut refused = None;
        while let Some(n) = f.queue.pop_front() {
            let ms = front_ms(cfg, &n, 1);
            let asks = if cfg.confirm_clicks == "card" && !Notices::is_ours(n.id) { n.asks() } else { Vec::new() };
            match send(&n.command(ms, cfg.pimped(), &asks)) {
                Ok(r) if !r.starts_with("error") => {
                    f.shown(&mut self.notices);
                    f.showing_until = time_up(now, ms);
                    // A card with the same id (a notice come again) is replaced: in front now.
                    f.deck.retain(|d| d.id != n.id);
                    f.deck.push(n);
                    if f.deck.len() > DECK_MAX {
                        // The oldest, sparing what waits for an answer.
                        let old = f.deck.remove(f.deck.iter().position(|d| !d.sticky).unwrap_or(0));
                        if let Err(e) = send(&format!("toast drop id={}", old.id)) {
                            log(format!("toast drop: {e}"));
                        }
                        f.gone.push_back((old, now));
                    }
                }
                Ok(e) => {
                    log(format!("showing toast from {}: {e}", n.app));
                    if f.refused() {
                        refused = Some(e);
                        break;
                    }
                }
                Err(e) => log(format!("showing toast from {}: {e}", n.app)),
            }
        }
        if let Some(e) = refused {
            let around: Vec<&str> = self.taskbar_software.iter().map(|k| k.name).collect();
            f.give_back(&e, &around, &mut self.notices, now);
        }
    }

    /// Sample whichever meter groups are due and push them.
    fn push_meters(&mut self) {
        let lines = self.sampler.sample_due(Instant::now());
        if self.applied.is_none() {
            return;
        }
        if lines.is_empty() {
            return;
        }
        // All of this tick's updates in one request: one UI-thread job, one layout pass.
        let batch = std::iter::once("batch".to_string()).chain(lines.iter().map(|l| crate::quote(l))).collect::<Vec<_>>().join(" ");
        let t0 = Instant::now();
        let r = send(&batch);
        if crate::trace() {
            crate::log(format!("trace: {:.1}ms for {} update(s)", t0.elapsed().as_secs_f64() * 1000.0, lines.len()));
        }
        match r {
            Ok(r) if r.lines().any(|l| l.starts_with("error")) => {
                self.note(format!("meters push -> {r}"));
                self.failed("The meters keep failing", &r);
                self.applied = None;
            }
            Ok(_) => self.meter_failures = 0,
            Err(e) => self.note(format!("meters push: {e}")),
        }
    }
}

pub fn run(path: &Path) -> Result<(), String> {
    pms_tap::logfile::move_old();
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    match config::adopt_layers(&path) {
        Ok(true) => log(format!("{} now holds only what's been changed; the whole file is kept as .toml.full.bak", path.display())),
        Ok(false) => {}
        Err(e) => log(format!("keeping the config as it is: {e}")),
    }
    // A config that doesn't load doesn't stop us: a stock taskbar (an empty config) meanwhile, a notice saying what's
    // wrong, and the next save that loads takes over.
    let (config, sampler, broken) = match config::load(&path).and_then(|c| Sampler::new(&c).map(|s| (c, s))) {
        Ok((c, s)) => (c, s, None),
        Err(e) => {
            log(format!("{e}: starting with a stock taskbar until it loads"));
            let empty: Config = toml::from_str("").map_err(|e| format!("an empty config: {e}"))?;
            let sampler = Sampler::new(&empty)?;
            (empty, sampler, Some(e))
        }
    };

    let instance = unsafe { CreateMutexW(None, false, INSTANCE_MUTEX) }.map_err(|e| e.to_string())?;
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Err("a PimpMyShell daemon is already running (`pms daemon stop` stops it)".into());
    }
    let stop = unsafe { CreateEventW(None, true, false, STOP_EVENT) }.map_err(|e| e.to_string())?;
    STOP.store(stop.0, Ordering::Release);
    let _ = unsafe { SetConsoleCtrlHandler(Some(on_ctrl), true) };
    // Shell item APIs (toast icons) need COM on this thread.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let mic = MicWatch::new()?;

    let mut d = Daemon {
        modified: modified(&path),
        path,
        config,
        sampler,
        applied: None,
        retry_apply: Instant::now(),
        hot: Vec::new(),
        hot_since: None,
        mic_closed: Vec::new(),
        settle_early: false,
        app_names: HashMap::new(),
        mic_state: None,
        mic_level: None,
        next_level: Instant::now(),
        sent_level: -1.0,
        sent_tide: -1.0,
        sent_muted: None,
        alert_shown: false,
        failures: 0,
        last_inject: None,
        toasts: None,
        light: false,
        last_note: String::new(),
        watchdog: Watchdog::default(),
        paused: false,
        tray: None,
        drilled: false,
        fps: 0,
        updates: Updates {
            result: Arc::default(),
            busy: false,
            manual: false,
            next: update::next_check(),
            found: None,
            told: None,
            since: Instant::now(),
            asked: false,
            tidied: false,
        },
        unhooked_since: None,
        toasts_parked: false,
        tap_build: std::env::current_exe().ok().and_then(|e| pms_tap::build_of(&e.with_file_name("pms_tap.dll"))),
        swaps: 0,
        stale_tap: false,
        surveyed: None,
        taskbar_software: Vec::new(),
        no_taskbar_since: None,
        told_replaced: false,
        meter_failures: 0,
        welcoming: Arc::default(),
        next_shell_check: Instant::now() + SHELL_CHECK,
        notices: Notices::default(),
        stand_in: broken.is_some(),
    };
    if let Some(e) = &broken {
        let notice = notices::config(&d.path, e, false);
        d.notices.raise(notice);
    }
    // The feed (which turns native banners off) starts once the taskbar has our config (`apply`): toasts that came
    // before then would have nowhere to show. A previous run's leftovers go back meanwhile.
    Banners::load().restore();
    d.toasts_parked = true;
    d.tray = Tray::start(&d.path).map_err(|e| log(format!("no tray icon: {e}"))).ok();
    d.sync_tray();
    d.welcome();
    log(format!("PimpMyShell {} on {}", crate::version::CURRENT, system::windows_version()));
    log(format!(
        "daemon up (pid {}): {}; backdrop {}; hot-mic {}; toasts {}; meters: {}",
        std::process::id(),
        d.path.display(),
        if d.config.backdrop.is_some() { "custom" } else { "stock" },
        if d.config.hotmic.is_some() { "on" } else { "off" },
        if d.config.toasts().is_some() { "in the taskbar (once it's ours)" } else { "native" },
        d.sampler.describe(),
    ));

    let mut next_status = Instant::now();
    loop {
        if d.handle_tray() {
            log("exit from the tray");
            break;
        }
        let tick = Instant::now() >= next_status;
        d.reload_if_changed();
        d.update_mic(&mic);
        if tick && let Some(why) = d.watchdog.poll() {
            d.bench(why);
        }
        let active = d.active();
        // Meters and toast polls wake us often; checking on the TAP once a second is plenty.
        if active && (tick || d.applied.is_none()) {
            d.ensure();
            d.follow_theme();
            d.follow_power();
        }
        if active && tick {
            d.park_toasts_if_unhooked();
        }
        if tick {
            next_status = Instant::now() + STATUS_EVERY;
            d.pump_updates();
            d.pump_notices();
            d.survey_shell();
            d.check_taskbar_hidden(Instant::now());
        }
        if active {
            d.pump_toasts();
            d.pump_mic_level();
            d.push_meters();
        }
        let mut deadline = next_status;
        if let Some(due) = d.sampler.next_due().filter(|_| active) {
            deadline = deadline.min(due);
        }
        if let Some(at) = d.mic_settles_at() {
            deadline = deadline.min(at);
        }
        if d.mic_state.is_some() {
            deadline = deadline.min(d.next_level);
        }
        if let Some(f) = &d.toasts {
            deadline = deadline.min(f.next_poll);
            deadline = f.showing_until.map_or(deadline, |t| deadline.min(t));
            if f.wants_events(Instant::now()) {
                deadline = deadline.min(f.next_events);
            }
        }
        let wait = deadline.saturating_duration_since(Instant::now()).as_millis() as u32;
        let mut events = vec![stop, mic.event()];
        events.extend(d.tray.as_ref().map(Tray::event));
        if unsafe { WaitForMultipleObjects(&events, false, wait) } == WAIT_OBJECT_0 {
            break;
        }
    }

    log(match send("reset") {
        Ok(r) => format!("stopping; {r}"),
        Err(e) => format!("stopping; TAP unreachable ({e})"),
    });
    if let Some(mut f) = d.toasts.take() {
        f.banners.restore();
    }
    unsafe {
        CloseHandle(stop).ok();
        CloseHandle(instance).ok();
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Autostart: HKCU Run key -> pmsd.exe (the windowless build of the daemon).

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const RUN_VALUE: PCWSTR = w!("PimpMyShell");

/// PimpMyShell was called SeaShell: carry over what that name kept, once - its config (to the new place, if there's
/// none there yet), the record of the notification banners it switched off (so they can be switched back on), the
/// start-at-sign-in entry (pointed at this copy), and the name its notifications went out under (dropped). The old
/// folder stays, for the user to remove.
pub fn migrate() {
    let Some(base) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else { return };
    let old = base.join("SeaShell");
    // Only where PimpMyShell has no config of its own yet, in either place it has kept one (the old folder stays
    // after the first carry-over, so this is asked every start). Not `default_path()`: that would move a config.
    let config = config::config_dir().join("pms.toml");
    if old.join("seashell.toml").is_file() && !config.exists() && !config::legacy_path().exists() {
        let banners = pms_tap::data_dir().join("banners.txt");
        for (from, to) in [(old.join("seashell.toml"), &config), (old.join("banners.txt"), &banners)] {
            if let Some(dir) = to.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if from.is_file() && std::fs::copy(&from, to).is_ok() {
                log(format!("carried over {} from SeaShell", to.display()));
            }
        }
    }
    let had_autostart = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, w!("SeaShell"), RRF_RT_REG_SZ, None, None, None) } == ERROR_SUCCESS;
    if had_autostart {
        let _ = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("SeaShell")) };
        match autostart(&["on".to_string(), config::default_path().display().to_string()]) {
            Ok(m) => log(format!("start at sign-in moved over from SeaShell: {m}")),
            Err(e) => log(format!("start at sign-in (was SeaShell's): {e}")),
        }
    }
    let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, w!(r"Software\Classes\AppUserModelId\SeaShell")) };
    // Start at sign-in as the settings wrote it before 0.1.3: with the config's path, its old place. The daemon takes
    // that as the default anyway (config::given), but the path goes, so the entry names the exe alone, as the
    // installer's does. Its exe stays as it is (another copy's entry isn't pointed at this one); only that exact form
    // is touched.
    if let Some(cmd) = read_run()
        && let Some(plain) = without_legacy_config(&cmd, &config::legacy_path())
    {
        match write_run(&HSTRING::from(&plain)) {
            Ok(()) => log(format!("start at sign-in: {plain} (it named the config's old place, {})", config::legacy_path().display())),
            Err(e) => log(format!("start at sign-in: {e}")),
        }
    }
}

pub fn autostart(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        Some("on") => {
            let config = config::given(args.get(1).map(Into::into));
            let config = std::path::absolute(&config).map_err(|e| e.to_string())?;
            config::load(&config)?;
            let exe = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("pmsd.exe");
            if !exe.is_file() {
                return Err(format!("{} not found; `cargo build --release` builds it", exe.display()));
            }
            // The default config goes unnamed (pmsd finds it), as in the installer's entry: one that names it would
            // outlive a move of it.
            let cmd = if config::is_default(&config) {
                HSTRING::from(format!("\"{}\"", exe.display()))
            } else {
                HSTRING::from(format!("\"{}\" \"{}\"", exe.display(), config.display()))
            };
            write_run(&cmd)?;
            Ok(format!("at logon: {cmd}"))
        }
        Some("off") => {
            let err = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE) };
            if err != ERROR_SUCCESS {
                return Err(format!("removing Run value: {}", err.to_hresult().message()));
            }
            Ok("autostart off".into())
        }
        Some("status") | None => Ok(match read_run() {
            Some(cmd) => format!("at logon: {cmd}"),
            None => "autostart off".into(),
        }),
        Some(other) => Err(format!("unknown autostart subcommand `{other}` (on [config]|off|status)")),
    }
}

/// The start-at-sign-in entry's command line (HKCU Run, value PimpMyShell), if there's one.
fn read_run() -> Option<String> {
    let mut buf = [0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    let err = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut size)) };
    (err == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
}

/// Writes the start-at-sign-in entry's command line.
fn write_run(cmd: &HSTRING) -> Result<(), String> {
    let bytes = unsafe { std::slice::from_raw_parts(cmd.as_ptr() as *const u8, (cmd.len() + 1) * 2) };
    let err = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, REG_SZ.0, Some(bytes.as_ptr() as _), bytes.len() as u32) };
    if err != ERROR_SUCCESS {
        return Err(format!("writing Run key: {}", err.to_hresult().message()));
    }
    Ok(())
}

/// A start-at-sign-in command line as the settings wrote it before 0.1.3, `"<exe>" "<config>"` with `<config>` the old
/// default (`legacy`): the same without it, `"<exe>"`. Anything else: None (left as it is).
fn without_legacy_config(cmd: &str, legacy: &Path) -> Option<String> {
    let (exe, rest) = cmd.strip_prefix('"')?.split_once('"')?;
    let config = rest.trim().strip_prefix('"')?.strip_suffix('"')?;
    Path::new(config).as_os_str().eq_ignore_ascii_case(legacy.as_os_str()).then(|| format!("\"{exe}\""))
}

/// Windows' light mode for the shell (taskbar, Start): what the taskbar's own look follows.
fn shell_light() -> bool {
    let mut v = 0u32;
    let mut size = 4u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as _),
            Some(&mut size),
        )
    };
    err == ERROR_SUCCESS && v == 1
}

/// The cap while on battery power, unless `battery_fps` says otherwise.
const BATTERY_FPS: u32 = 30;

/// Whether the PC runs on its battery (unplugged, discharging): not a desktop, nor a laptop on its charger.
fn on_battery() -> bool {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut status = SYSTEM_POWER_STATUS::default();
    // ACLineStatus 0 = offline; BatteryFlag 128 = no battery, 255 = unknown.
    unsafe { GetSystemPowerStatus(&mut status) }.is_ok() && status.ACLineStatus == 0 && status.BatteryFlag != 128 && status.BatteryFlag != 255
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_entry_loses_the_old_config_path() {
        let legacy = Path::new(r"C:\Users\U\AppData\Local\PimpMyShell\pms.toml");
        let exe = r#""C:\Users\U\AppData\Local\Programs\PimpMyShell\pmsd.exe""#;
        // As the settings wrote it before 0.1.3 (any case): the exe alone.
        assert_eq!(without_legacy_config(&format!(r#"{exe} "C:\Users\U\AppData\Local\PimpMyShell\pms.toml""#), legacy).as_deref(), Some(exe));
        assert_eq!(without_legacy_config(&format!(r#"{exe} "c:\users\u\appdata\local\pimpmyshell\PMS.TOML""#), legacy).as_deref(), Some(exe));
        // Already plain (the installer's), or another config: left alone.
        assert_eq!(without_legacy_config(exe, legacy), None);
        assert_eq!(without_legacy_config(&format!(r#"{exe} "D:\mine.toml""#), legacy), None);
        assert_eq!(without_legacy_config("pmsd.exe", legacy), None);
    }
}
