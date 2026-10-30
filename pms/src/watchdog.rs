//! Looking out for explorer, while the daemon has it hooked.
//!
//! PimpMyShell never brings back an explorer that was simply ended: people end it on purpose, and Task Manager or
//! taskkill leave it with exit code 1. An explorer we hooked that *crashed* (its exit code an exception's, 0xC...)
//! is another matter. Windows' event log says which module faulted ("Application Error", event 1000), and then:
//!
//! - our TAP faulted, or XAML / composition did (which the TAP drives; a XAML fail-fast names XAML, not the caller):
//!   stop hooking explorer for the rest of this run, at once;
//! - anything else faulted: once is explorer's own business; twice within `CRASH_WINDOW` and we stop all the same.
//!
//! Hooking that keeps failing gives up too: `MAX_INJECT_FAILURES` failed injections in a row, or a TAP that has to be
//! replaced `MAX_REINJECTS` times in the same explorer. Benched is for the rest of the run (the tray's Retry starts
//! over); the daemon then leaves the stock taskbar alone.
//!
//! And whether or not it was ours: Windows starts a crashed shell again by itself. If it hasn't within
//! `RESTART_GRACE`, we start explorer.exe - only after a crash, never after it was ended.

use std::os::windows::process::CommandExt as _;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE, STILL_ACTIVE, WAIT_OBJECT_0};
use windows::Win32::System::EventLog::{EVT_HANDLE, EvtClose, EvtNext, EvtQuery, EvtQueryChannelPath, EvtQueryReverseDirection, EvtRender, EvtRenderEventXml};
use windows::Win32::System::Threading::{CREATE_DEFAULT_ERROR_MODE, GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
use windows_core::{HSTRING, PCWSTR, w};

use crate::log;

const CRASH_WINDOW: Duration = Duration::from_secs(10 * 60);
const MAX_CRASHES: usize = 2;
const MAX_INJECT_FAILURES: u32 = 3;
const MAX_REINJECTS: u32 = 3;
/// How long Windows gets to restart a crashed shell before we do.
const RESTART_GRACE: Duration = Duration::from_secs(12);
/// How long to wait for the crash's event log record.
const RECORD_WAIT: Duration = Duration::from_secs(8);
/// How far back to look for it.
const RECORD_AGE_MS: u64 = 5 * 60 * 1000;
/// Faulting modules that count against us: the TAP itself, and what it drives.
const OURS: &str = "pms_tap";
const SUSPECT: [&str; 6] = ["windows.ui.xaml.dll", "windows.ui.dll", "dcomp.dll", "coremessaging.dll", "twinapi.appcore.dll", "windows.ui.composition.dll"];

struct Hooked {
    pid: u32,
    process: HANDLE,
    /// Its exe's name, as the event log records it: explorer.exe, or a drill's stand-in.
    app: String,
}

/// A crash whose record we're waiting for.
struct Crash {
    app: String,
    pid: u32,
    code: u32,
    at: Instant,
}

/// What the faulting module says about the crash.
#[derive(PartialEq)]
enum Blame {
    Ours(String),
    Suspect(String),
    Elsewhere(String),
    Unknown,
}

pub struct Watchdog {
    hooked: Option<Hooked>,
    pending: Option<Crash>,
    /// Crashes of explorers we had hooked, lately.
    crashes: Vec<Instant>,
    inject_failures: u32,
    /// Injections into the same explorer: (pid, how many).
    reinjects: (u32, u32),
    /// A crashed shell Windows hasn't brought back yet: when we'll start it.
    restart_at: Option<Instant>,
    /// Why we stopped hooking explorer, for the rest of the run.
    benched: Option<String>,
    /// Whether a shell is up, and what to start when none came back: explorer, unless a drill stands in for it.
    pub shell_up: fn() -> bool,
    pub shell: std::path::PathBuf,
}

impl Default for Watchdog {
    fn default() -> Watchdog {
        Watchdog {
            hooked: None,
            pending: None,
            crashes: Vec::new(),
            inject_failures: 0,
            reinjects: (0, 0),
            restart_at: None,
            benched: None,
            shell_up,
            shell: r"C:\Windows\explorer.exe".into(),
        }
    }
}

impl Watchdog {
    pub fn benched(&self) -> Option<&str> {
        self.benched.as_deref()
    }

    /// The pid of the process being watched.
    pub fn watching(&self) -> Option<u32> {
        self.hooked.as_ref().map(|h| h.pid)
    }

    /// Nothing watched or pending: no crash awaiting its record, no shell awaiting a restart.
    pub fn settled(&self) -> bool {
        self.hooked.is_none() && self.pending.is_none() && self.restart_at.is_none()
    }

    /// Start over (the tray's Retry): forget the strikes.
    pub fn retry(&mut self) {
        self.benched = None;
        self.crashes.clear();
        self.inject_failures = 0;
        self.reinjects = (0, 0);
    }

    /// We injected a TAP into explorer `pid`. Returns why we give up, if this is one replacement too many.
    pub fn injected(&mut self, pid: u32) -> Option<String> {
        self.inject_failures = 0;
        self.reinjects = if self.reinjects.0 == pid { (pid, self.reinjects.1 + 1) } else { (pid, 1) };
        if self.hooked.as_ref().is_none_or(|h| h.pid != pid) {
            self.watch(pid, "explorer.exe");
        }
        (self.reinjects.1 > MAX_REINJECTS).then(|| self.bench(format!("PimpMyShell's hook kept failing in explorer ({} tries)", self.reinjects.1)))
    }

    /// Watch process `pid` (whose exe is `app`) as the hooked explorer. The daemon watches explorer; a drill
    /// (examples/watchdog_drill.rs, or PMS_WATCHDOG_DRILL) watches a stand-in that crashes on cue.
    pub fn watch(&mut self, pid: u32, app: &str) {
        self.release();
        match unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
            Ok(process) => self.hooked = Some(Hooked { pid, process, app: app.into() }),
            Err(e) => log(format!("watchdog: can't watch {app} (pid {pid}): {e}")),
        }
    }

    /// An injection failed. Returns why we give up, after a few in a row.
    pub fn inject_failed(&mut self, error: &str) -> Option<String> {
        self.inject_failures += 1;
        (self.inject_failures >= MAX_INJECT_FAILURES).then(|| self.bench(format!("PimpMyShell couldn't hook explorer ({} tries; last: {error})", self.inject_failures)))
    }

    fn bench(&mut self, why: String) -> String {
        log(format!("watchdog: {why}; leaving explorer alone until PimpMyShell restarts"));
        self.benched = Some(why.clone());
        why
    }

    fn release(&mut self) {
        if let Some(h) = self.hooked.take() {
            unsafe { CloseHandle(h.process) }.ok();
        }
    }

    /// Check on explorer; call about once a second. Returns why we give up, the moment we do.
    pub fn poll(&mut self) -> Option<String> {
        let mut gave_up = None;
        if let Some(h) = &self.hooked
            && unsafe { WaitForSingleObject(h.process, 0) } == WAIT_OBJECT_0
        {
            let mut code = STILL_ACTIVE.0 as u32;
            let _ = unsafe { GetExitCodeProcess(h.process, &mut code) };
            let (pid, app) = (h.pid, h.app.clone());
            self.release();
            if code >= 0x8000_0000 {
                log(format!("watchdog: {app} (pid {pid}) crashed, exception 0x{code:08X}"));
                self.pending = Some(Crash { app, pid, code, at: Instant::now() });
                self.restart_at = Some(Instant::now() + RESTART_GRACE);
            } else {
                log(format!("watchdog: {app} (pid {pid}) was ended (exit code {code}); not starting it again"));
            }
        }
        if let Some(c) = &self.pending {
            let blame = blame(&c.app, c.pid, RECORD_AGE_MS);
            if blame != Blame::Unknown || c.at.elapsed() > RECORD_WAIT {
                let code = c.code;
                self.pending = None;
                gave_up = self.judge(blame, code);
            }
        }
        if let Some(at) = self.restart_at {
            if (self.shell_up)() {
                self.restart_at = None;
            } else if Instant::now() >= at {
                self.restart_at = None;
                log(format!("watchdog: no shell came back after the crash; starting {}", self.shell.display()));
                // Windows' own error mode, not ours: a daemon started from a shell that turns crash reporting off (Git
                // Bash does) would pass that on, and a crash of the explorer we started would go unrecorded.
                if let Err(e) = std::process::Command::new(&self.shell).creation_flags(CREATE_DEFAULT_ERROR_MODE.0).spawn() {
                    log(format!("watchdog: couldn't start explorer: {e}"));
                }
            }
        }
        gave_up
    }

    fn judge(&mut self, blame: Blame, code: u32) -> Option<String> {
        let now = Instant::now();
        self.crashes.retain(|t| now.duration_since(*t) < CRASH_WINDOW);
        self.crashes.push(now);
        match blame {
            Blame::Ours(m) => Some(self.bench(format!("explorer crashed in PimpMyShell's code ({m}, 0x{code:08X})"))),
            Blame::Suspect(m) => Some(self.bench(format!("explorer crashed in {m} (0x{code:08X}) while PimpMyShell was hooked into it"))),
            Blame::Elsewhere(m) if self.crashes.len() < MAX_CRASHES => {
                log(format!("watchdog: the crash was in {m}, not PimpMyShell; carrying on"));
                None
            }
            Blame::Unknown if self.crashes.len() < MAX_CRASHES => {
                log("watchdog: no record of what crashed; carrying on");
                None
            }
            _ => Some(self.bench(format!("explorer crashed {} times in {} minutes while PimpMyShell was hooked into it", self.crashes.len(), CRASH_WINDOW.as_secs() / 60))),
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.release();
    }
}

fn shell_up() -> bool {
    unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.is_ok()
}

/// What the event log says faulted in `app` (process `pid`), from its "Application Error" records of the last
/// `within_ms`.
fn blame(app: &str, pid: u32, within_ms: u64) -> Blame {
    let query = HSTRING::from(format!("*[System[Provider[@Name='Application Error'] and EventID=1000 and TimeCreated[timediff(@SystemTime) <= {within_ms}]]]"));
    let Ok(results) = (unsafe { EvtQuery(None, w!("Application"), &query, EvtQueryChannelPath.0 | EvtQueryReverseDirection.0) }) else { return Blame::Unknown };
    let pid_hex = format!("0x{pid:x}");
    let mut found = Blame::Unknown;
    'records: loop {
        let mut events = [0isize; 8];
        let mut n = 0u32;
        if unsafe { EvtNext(results, &mut events, 0, 0, &mut n) }.is_err() {
            break;
        }
        for &event in &events[..n as usize] {
            let event = EVT_HANDLE(event);
            let xml = render(event);
            unsafe { EvtClose(event) }.ok();
            let Some(xml) = xml else { continue };
            let field = |name: &str| {
                let tag = format!("<Data Name='{name}'>");
                xml.find(&tag).map(|i| &xml[i + tag.len()..]).and_then(|rest| rest.split('<').next()).unwrap_or("").to_string()
            };
            if !field("AppName").eq_ignore_ascii_case(app) || !field("ProcessId").eq_ignore_ascii_case(&pid_hex) {
                continue;
            }
            let module = field("ModuleName");
            let lower = module.to_ascii_lowercase();
            found = if lower.starts_with(OURS) {
                Blame::Ours(module)
            } else if SUSPECT.contains(&lower.as_str()) {
                Blame::Suspect(module)
            } else {
                Blame::Elsewhere(module)
            };
            break 'records;
        }
    }
    unsafe { EvtClose(results) }.ok();
    found
}

fn render(event: EVT_HANDLE) -> Option<String> {
    let (mut used, mut count) = (0u32, 0u32);
    let _ = unsafe { EvtRender(None, event, EvtRenderEventXml.0, 0, None, &mut used, &mut count) };
    if used == 0 {
        return None;
    }
    let mut buf = vec![0u16; used as usize / 2 + 1];
    unsafe { EvtRender(None, event, EvtRenderEventXml.0, (buf.len() * 2) as u32, Some(buf.as_mut_ptr().cast()), &mut used, &mut count) }.ok()?;
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the Application log without error (whatever is in it), and finds nothing for a pid that isn't one.
    #[test]
    fn blame_reads_the_log() {
        assert!(blame("explorer.exe", 3, RECORD_AGE_MS) == Blame::Unknown);
    }

    /// Against a record a machine happens to have; set PMS_TEST_CRASH=app,pid,module (pid as in the record's
    /// ProcessId, hex) to run it, e.g. LockApp.exe,0x2408,Windows.UI.Xaml.dll.
    #[test]
    #[ignore]
    fn blame_finds_a_real_crash() {
        let spec = std::env::var("PMS_TEST_CRASH").expect("PMS_TEST_CRASH=app,pid,module");
        let [app, pid, module] = spec.split(',').collect::<Vec<_>>()[..] else { panic!("app,pid,module") };
        let pid = u32::from_str_radix(pid.trim_start_matches("0x"), 16).unwrap();
        let found = blame(app, pid, 365 * 24 * 3600 * 1000);
        let expected = if module.to_ascii_lowercase().starts_with(OURS) {
            Blame::Ours(module.into())
        } else if SUSPECT.contains(&module.to_ascii_lowercase().as_str()) {
            Blame::Suspect(module.into())
        } else {
            Blame::Elsewhere(module.into())
        };
        assert!(found == expected);
    }
}
