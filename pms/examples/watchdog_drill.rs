//! Drills the watchdog (pms/src/watchdog.rs) without touching explorer: crash_dummy stands in for it, and the
//! watchdog's own code watches it, reads the event log about it, and decides.
//!
//!   cargo build --release --example crash_dummy --example watchdog_drill
//!   target\release\examples\watchdog_drill.exe
//!
//! 1. a crash in someone else's code: noted, carry on
//! 2. an "End task" (exit code 1): left alone, nothing restarted
//! 3. a second crash within ten minutes: give up
//! 4. (after Retry) a crash in a module named like the TAP: give up at once
//! 5. (after Retry) a crash, and no shell comes back: after the grace, the watchdog starts one (here, the dummy)

use std::path::Path;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use pms::watchdog::Watchdog;

fn yes() -> bool {
    true
}

fn no() -> bool {
    false
}

fn spawn(exe: &Path, args: &[&str]) -> Child {
    // Windows' own error mode: some shells (Git Bash) turn crash reporting off for what they start, and then there's
    // no record of the crash to read.
    Command::new(exe).args(args).creation_flags(0x0400_0000).spawn().unwrap_or_else(|e| panic!("{}: {e}", exe.display()))
}

/// Polls the watchdog until it has dealt with the stand-in's end (or `limit` passes); returns why it gave up, if it did.
fn settle(w: &mut Watchdog, limit: Duration) -> Option<String> {
    let start = Instant::now();
    let mut gave_up = None;
    while start.elapsed() < limit {
        if let Some(why) = w.poll() {
            gave_up = Some(why);
        }
        if w.settled() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    gave_up
}

fn check(step: &str, ok: bool, failures: &mut u32) {
    println!("{} {step}\n", if ok { "PASS" } else { "FAIL" });
    *failures += !ok as u32;
}

fn main() {
    let dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let dummy = dir.join("crash_dummy.exe");
    assert!(dummy.exists(), "build the crash_dummy example first");
    let ours = dir.join("pms_tap_drill.exe");
    std::fs::copy(&dummy, &ours).expect("copy the dummy under the TAP's name");
    let mut failures = 0;
    let mut w = Watchdog::default();
    w.shell_up = yes;

    println!("== 1. a crash in someone else's code");
    let c = spawn(&dummy, &["1"]);
    w.watch(c.id(), "crash_dummy.exe");
    let gave_up = settle(&mut w, Duration::from_secs(20));
    check("carries on after one crash", gave_up.is_none() && w.benched().is_none(), &mut failures);

    println!("== 2. ended from Task Manager (exit code 1)");
    let mut c = spawn(&dummy, &["exit", "60"]);
    w.watch(c.id(), "crash_dummy.exe");
    std::thread::sleep(Duration::from_millis(500));
    c.kill().unwrap();
    let gave_up = settle(&mut w, Duration::from_secs(5));
    check("leaves an ended process alone", gave_up.is_none() && w.benched().is_none(), &mut failures);

    println!("== 3. a second crash within ten minutes");
    let c = spawn(&dummy, &["1"]);
    w.watch(c.id(), "crash_dummy.exe");
    let gave_up = settle(&mut w, Duration::from_secs(20));
    check(&format!("gives up: {gave_up:?}"), gave_up.is_some() && w.benched().is_some(), &mut failures);

    println!("== 4. Retry, then a crash in a module named like the TAP");
    w.retry();
    let c = spawn(&ours, &["1"]);
    w.watch(c.id(), "pms_tap_drill.exe");
    let gave_up = settle(&mut w, Duration::from_secs(20));
    check(&format!("gives up at once: {gave_up:?}"), gave_up.as_deref().is_some_and(|g| g.contains("PimpMyShell's code")), &mut failures);

    println!("== 5. Retry, then a crash, and no shell comes back");
    w.retry();
    w.shell_up = no;
    w.shell = dummy.clone();
    let c = spawn(&dummy, &["1"]);
    w.watch(c.id(), "crash_dummy.exe");
    let t0 = Instant::now();
    // The restart: the watchdog starts `shell` (the dummy, which crashes again in 2 s, unwatched) after its grace.
    let gave_up = settle(&mut w, Duration::from_secs(30));
    check(&format!("starts a shell after {:.0} s, carrying on", t0.elapsed().as_secs_f64()), gave_up.is_none() && w.settled(), &mut failures);

    let _ = std::fs::remove_file(&ours);
    println!("{}", if failures == 0 { "all passed".to_string() } else { format!("{failures} failed") });
    std::process::exit(failures as i32);
}
