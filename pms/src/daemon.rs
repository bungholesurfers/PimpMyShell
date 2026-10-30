//! `pms daemon`: keep the taskbar looking the way `pms.toml` says, for as long as it
//! runs. Once per meter interval (or when the microphone state changes) it:
//!
//!   1. asks the TAP for `status`. No TAP (explorer restarted, first run) -> inject one, once the
//!      taskbar's XAML island exists. A TAP copy or taskbar tree it hasn't configured yet, or one
//!      that drifted (someone ran `reset`) -> `reset` and apply the whole config again.
//!   2. shows or hides the hot-mic alert,
//!   3. samples the perf counters and pushes the meters.
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
/// Per-toast time while more are waiting.
const QUEUED_TOAST_MS: u32 = 4000;
/// Windows' own "long" toast duration.
const LONG_TOAST_MS: u32 = 25_000;
/// How often to ask the TAP for clicks while a toast is up.
const EVENT_POLL: Duration = Duration::from_millis(120);

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
    /// Apps holding the mic (minus ignored ones), and whether the TAP shows the alert.
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
    /// The toast feed was stopped because the taskbar wasn't ours to show toasts on: native banners are back until
    /// it is.
    toasts_parked: bool,
}

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
    /// The question whether to check by itself waits a while into the first run, and is asked once.
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
    listener: Listener,
    banners: Banners,
    queue: VecDeque<Incoming>,
    showing_until: Option<Instant>,
    /// The toast on screen (or just taken down), whose clicks we still owe an activation.
    current: Option<Incoming>,
    next_events: Instant,
    next_poll: Instant,
}

/// Start the feed if the config asks for one; otherwise undo banner changes a previous run
/// (maybe one that crashed) left behind.
fn toast_feed(config: &Config) -> Option<ToastFeed> {
    let Some(t) = &config.toasts else {
        Banners::load().restore();
        return None;
    };
    let listener = match Listener::new() {
        Ok(l) => l,
        Err(e) => {
            log(format!("toasts disabled: {e}"));
            return None;
        }
    };
    // Start from the user's own settings (a changed ignore list must take effect), then re-suppress.
    let mut banners = Banners::load();
    banners.restore();
    banners.suppress_known(|aumid| t.ignores("", aumid));
    Some(ToastFeed { listener, banners, queue: VecDeque::new(), showing_until: None, current: None, next_events: Instant::now(), next_poll: Instant::now() })
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
                let old = std::mem::replace(&mut self.config, config);
                self.sampler = sampler;
                let new = &self.config;
                if old.toasts != new.toasts {
                    if let Some(mut feed) = self.toasts.take() {
                        feed.banners.restore();
                    }
                    // Started now only while the taskbar has our config; otherwise `apply` starts it.
                    self.toasts = if self.active() && self.applied.is_some() { toast_feed(&self.config) } else { None };
                    if self.toasts.is_none() && self.config.toasts.is_some() {
                        self.toasts_parked = self.active();
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
                cmds.extend(self.sampler.define(&self.config.meter_height));
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
            Err(e) => log(format!("keeping the previous config: {e}")),
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
        if self.last_inject.is_some_and(|t| t.elapsed() < INJECT_BACKOFF) {
            return;
        }
        if taskbar_pid().is_none() {
            self.note("waiting for the taskbar".into());
            return;
        }
        self.last_inject = Some(Instant::now());
        match inject() {
            Ok(msg) => {
                log(msg);
                self.failures = 0;
                self.applied = None;
                self.retry_apply = Instant::now();
                if let Some(why) = taskbar_pid().and_then(|pid| self.watchdog.injected(pid)) {
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
        self.failures = 0;
        self.sync_tray();
    }

    /// The watchdog gave up on explorer: leave it be, and tell the user why.
    fn bench(&mut self, why: String) {
        self.quiet();
        // A TAP that keeps failing goes inert for good (until a retry injects a fresh one).
        let _ = send("detach");
        if let Some(t) = &self.tray {
            t.notify("PimpMyShell stopped hooking explorer", &format!("{why}. The taskbar is back to stock; the PimpMyShell icon's menu can try again."));
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
        if self.config.update_check.is_none()
            && !self.updates.asked
            && self.updates.since.elapsed() >= UPDATE_QUESTION_AFTER
            && let Some(t) = &self.tray
        {
            self.updates.asked = true;
            t.ask_update_consent();
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
                        "Downloaded from GitHub, and its signature checks out. Install it from the PimpMyShell icon's menu: it asks first.".to_string(),
                    )),
                    update::Status::Available { version } => Some((
                        version.clone(),
                        format!("PimpMyShell {version} is out"),
                        "This copy isn't the installed one, so it can't update itself: the PimpMyShell icon's menu opens the new version's page.".to_string(),
                    )),
                    update::Status::Stuck { version } if manual => Some((
                        version.clone(),
                        format!("PimpMyShell {version} didn't install"),
                        "Its installer ran before, but this is still the older version. Check again from the settings, or get it from the releases page.".to_string(),
                    )),
                    update::Status::UpToDate { stable } if manual => Some((
                        String::new(),
                        "PimpMyShell is up to date".to_string(),
                        format!("This is PimpMyShell {}; the {} release is {stable}.", crate::version::CURRENT, if self.config.latest_channel() { "latest" } else { "stable" }),
                    )),
                    _ => None,
                };
                self.updates.found = Some(status);
                told
            }
            Err(e) => {
                self.updates.next = now + update::random_secs(update::RETRY);
                log(format!("update: the check failed: {e}"));
                manual.then(|| (String::new(), "PimpMyShell couldn't check for updates".to_string(), e))
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

    /// The tray menu's Install, which the user said yes to.
    fn install_update(&mut self) {
        let Some(update::Status::Ready { version, path, sha256 }) = self.updates.found.clone() else { return };
        log(format!("update: installing {version} from {}, as asked", path.display()));
        if let Err(e) = update::install(&path, &version, &sha256) {
            log(format!("update: {e}"));
            self.updates.found = None;
            self.sync_tray();
            if let Some(t) = &self.tray {
                t.notify(&format!("PimpMyShell {version} didn't install"), &e);
            }
        }
        // Otherwise the installer stops this daemon, installs, and starts the new one.
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
            }
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
        // Clicked away (or gone with a reset): move on to the next one.
        if let Some(f) = self.toasts.as_mut().filter(|_| field(&status, "toast") == Some("hidden")) {
            f.showing_until = None;
        }
        let key = format!("tap={} taskbar={}", field(&status, "tap").unwrap_or("?"), field(&status, "taskbar").unwrap_or("none"));
        if field(&status, "taskbar") == Some("none") {
            self.applied = None;
            return self.note("TAP is up, taskbar tree not there yet".into());
        }
        let want = |b: bool| if b { "on" } else { "off" };
        let drifted = field(&status, "backdrop") != Some(want(self.config.backdrop.is_some()))
            || field(&status, "alert") != Some(want(self.alert_shown))
            || field(&status, "meters") != Some(&self.config.meters.len().to_string());
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
            cmds.extend(self.sampler.define(&self.config.meter_height));
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
                return self.note(format!("applying config: `{cmd}` -> {e}"));
            }
        }
        self.mic_state = None;
        self.sent_muted = None;
        self.sent_level = -1.0;
        self.sent_tide = -1.0;
        self.alert_shown = false;
        if let Some(f) = self.toasts.as_mut() {
            f.showing_until = None;
        }
        log(format!("applied config to {key}"));
        self.last_note.clear();
        self.applied = Some(key);
        self.unhooked_since = None;
        if std::mem::take(&mut self.toasts_parked) {
            self.toasts = toast_feed(&self.config);
            if self.toasts.is_some() {
                log("the taskbar is ours again: notification banners go to it again");
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
        let (Some(cfg), Some(f)) = (self.config.toasts.as_ref(), self.toasts.as_mut()) else { return };
        let now = Instant::now();
        if now >= f.next_poll {
            f.next_poll = now + TOAST_POLL;
            match f.listener.poll() {
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
                            log(format!("toast from {}: {}", n.app, n.title));
                            f.queue.push_back(n);
                        }
                    }
                }
                Err(e) => log(format!("reading notifications: {e}")),
            }
        }
        // Clicks on the card: activate like Windows would, off the loop (an app may take a while to start).
        if f.current.is_some() && now >= f.next_events {
            f.next_events = now + EVENT_POLL;
            let events = send("toast events").unwrap_or_default();
            for ev in events.lines() {
                let Some(t) = f.current.take() else { break };
                let picked = match ev.split_once(' ') {
                    _ if ev == "open" => Some((t.launch.0.clone(), t.launch.1)),
                    Some(("action", i)) => i.parse::<usize>().ok().and_then(|i| t.actions.get(i)).map(|a| (a.args.clone(), a.kind)),
                    _ => None,
                };
                f.showing_until = None;
                let Some((args, kind)) = picked else {
                    log(format!("toast from {} closed; it stays in the notification center", t.app));
                    continue;
                };
                f.listener.remove(t.id);
                std::thread::spawn(move || {
                    let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
                    match toasts::activate(&t, &args, kind) {
                        Ok(how) => log(format!("toast click: {how}")),
                        Err(e) => log(format!("toast click on {}: {e}", t.app)),
                    }
                });
            }
            if f.showing_until.is_none() {
                // Gone (expired, reset) and its last clicks collected.
                f.current = None;
            }
        }
        if f.showing_until.is_some_and(|t| now >= t) {
            f.showing_until = None;
            if f.queue.is_empty()
                && let Err(e) = send("toast hide") {
                    log(format!("toast hide: {e}"));
                }
        }
        if f.showing_until.is_some() || self.applied.is_none() {
            return;
        }
        let Some(n) = f.queue.pop_front() else { return };
        let ms = match (n.long, f.queue.is_empty()) {
            (true, _) => cfg.duration_ms.max(LONG_TOAST_MS),
            (false, true) => cfg.duration_ms,
            (false, false) => cfg.duration_ms.min(QUEUED_TOAST_MS),
        };
        match send(&n.command(ms)) {
            Ok(r) if !r.starts_with("error") => {
                f.showing_until = Some(now + Duration::from_millis(ms as u64));
                f.current = Some(n);
            }
            Ok(e) | Err(e) => log(format!("showing toast from {}: {e}", n.app)),
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
                self.applied = None;
            }
            Ok(_) => {}
            Err(e) => self.note(format!("meters push: {e}")),
        }
    }
}

pub fn run(path: &Path) -> Result<(), String> {
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    let config = config::load(&path)?;
    let sampler = Sampler::new(&config)?;

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
    };
    // The feed (which turns native banners off) starts once the taskbar has our config (`apply`): toasts that came
    // before then would have nowhere to show. A previous run's leftovers go back meanwhile.
    Banners::load().restore();
    d.toasts_parked = true;
    d.tray = Tray::start(&d.path).map_err(|e| log(format!("no tray icon: {e}"))).ok();
    d.sync_tray();
    log(format!(
        "daemon up (pid {}): {}; backdrop {}; hot-mic {}; toasts {}; meters: {}",
        std::process::id(),
        d.path.display(),
        if d.config.backdrop.is_some() { "custom" } else { "stock" },
        if d.config.hotmic.is_some() { "on" } else { "off" },
        if d.config.toasts.is_some() { "in the taskbar (once it's ours)" } else { "native" },
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
            if f.current.is_some() {
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
    let (old, new) = (base.join("SeaShell"), pms_tap::data_dir());
    if old.join("seashell.toml").is_file() && !new.join("pms.toml").exists() {
        let _ = std::fs::create_dir_all(&new);
        for (from, to) in [("seashell.toml", "pms.toml"), ("banners.txt", "banners.txt")] {
            if old.join(from).is_file() && std::fs::copy(old.join(from), new.join(to)).is_ok() {
                log(format!("carried over {} from SeaShell", to));
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
}

pub fn autostart(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        Some("on") => {
            let config = args.get(1).map(PathBuf::from).unwrap_or_else(config::default_path);
            let config = std::path::absolute(&config).map_err(|e| e.to_string())?;
            config::load(&config)?;
            let exe = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("pmsd.exe");
            if !exe.is_file() {
                return Err(format!("{} not found; `cargo build --release` builds it", exe.display()));
            }
            let cmd = HSTRING::from(format!("\"{}\" \"{}\"", exe.display(), config.display()));
            let bytes = unsafe { std::slice::from_raw_parts(cmd.as_ptr() as *const u8, (cmd.len() + 1) * 2) };
            let err = unsafe {
                RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, REG_SZ.0, Some(bytes.as_ptr() as _), bytes.len() as u32)
            };
            if err != ERROR_SUCCESS {
                return Err(format!("writing Run key: {}", err.to_hresult().message()));
            }
            Ok(format!("at logon: {cmd}"))
        }
        Some("off") => {
            let err = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE) };
            if err != ERROR_SUCCESS {
                return Err(format!("removing Run value: {}", err.to_hresult().message()));
            }
            Ok("autostart off".into())
        }
        Some("status") | None => {
            let mut buf = [0u16; 2048];
            let mut size = (buf.len() * 2) as u32;
            let err = unsafe {
                RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut size))
            };
            Ok(if err == ERROR_SUCCESS {
                format!("at logon: {}", String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
            } else {
                "autostart off".into()
            })
        }
        Some(other) => Err(format!("unknown autostart subcommand `{other}` (on [config]|off|status)")),
    }
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