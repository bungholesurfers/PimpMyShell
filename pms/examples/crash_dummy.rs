//! A stand-in for explorer in the watchdog's drills: waits a while, then crashes with an access violation, which
//! Windows records ("Application Error", event 1000) as it would explorer's, with this exe as the faulting module.
//!
//!   crash_dummy [seconds]       crash after `seconds` (default 2)
//!   crash_dummy exit [seconds]  just exit (code 0) after `seconds`, crashing nothing
//!
//! A copy named pms_tap_*.exe crashes "in PimpMyShell's code", as far as the event log can tell.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let quiet = args.first().is_some_and(|a| a == "exit");
    let secs: f64 = args.iter().find_map(|a| a.parse().ok()).unwrap_or(2.0);
    std::thread::sleep(std::time::Duration::from_secs_f64(secs));
    if !quiet {
        // An address no process maps: the access violation is the whole point.
        unsafe { (8usize as *mut u32).write_volatile(1) };
    }
}
