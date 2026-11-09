//! Restarting Explorer, gently, and only when you ask: someone watching a film or giving a talk touches nothing for an hour, so a PC
//! that looks idle is no reason. Left to itself, all PimpMyShell does is say so, once Explorer has both plenty of TAPs and plenty of
//! memory. Windows has no way to take a DLL back out of a running process, so what the TAP has cost Explorer stays until Explorer
//! starts again, and the only cost of that is what's open in it. So nothing here happens with a folder window, a file copy or one of
//! Explorer's dialogs open, and it's Explorer that's asked to leave, the way Ctrl+Shift+right-click on the taskbar
//! does. An Explorer that says no is left where it is. One can also take its taskbar down and then never finish leaving, and nobody
//! should be left without a taskbar, so that one is ended: with nothing open in it, it has nothing left to lose. However it goes,
//! this doesn't finish until there's a taskbar again.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, FindWindowW, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_USER};
use windows_core::{BOOL, PCWSTR, w};

/// What the taskbar's own menu sends, for Exit Explorer.
const EXIT_EXPLORER: u32 = WM_USER + 436;
/// How long a copy's window can stay hidden as the copy starts: about a second here, measured. One that stays hidden longer is the
/// last copy's, which Explorer keeps after it's done.
const COPY_SHOWS_WITHIN: Duration = Duration::from_secs(3);
/// How long Explorer gets to leave, in milliseconds.
const LEAVES_WITHIN: u32 = 6_000;
/// How long it gets once it's been ended.
const ENDED_WITHIN: u32 = 3_000;
/// How long Windows gets to bring it back by itself, before that's up to us.
const WINDOWS_MAY_RESTART_IT: Duration = Duration::from_millis(1500);
/// How long a new one gets to put its taskbar up.
const COMES_BACK_WITHIN: Duration = Duration::from_secs(15);
/// How many TAPs Explorer has to have loaded, and how much memory it has to hold, for a restart to be worth mentioning. Every new
/// build it meets is one more TAP. A fresh Explorer here holds about 130 MB, and one with 44 TAPs held 360 MB.
const TAPS_WORTH_IT: usize = 10;
const MEMORY_WORTH_IT: u64 = 300 << 20;

const FOLDER_OPEN: &str = "a File Explorer window is open";
/// A way round that: folder windows in a process of their own aren't in the taskbar's, so they don't stop a restart, and they
/// survive it.
const SEPARATE_PROCESS: &str = " To keep folder windows open through a restart, turn on \"Launch folder windows in a separate process\" in File Explorer's options, on the View tab. It takes effect the next time Explorer starts.";

/// What it says if it's asked to while something's open: a reason that reads after "because".
pub fn blocker(windows: &[(String, bool)]) -> Option<&'static str> {
    let any = |class: &str| windows.iter().any(|(c, _)| c == class);
    // Explorer keeps the last copy's window after it's done, hidden: only one that shows is a copy going on. Minimized, it still shows.
    if windows.iter().any(|(c, visible)| c == "OperationStatusWindow" && *visible) {
        Some("a file copy is running")
    } else if any("CabinetWClass") || any("ExploreWClass") {
        Some(FOLDER_OPEN)
    } else if windows.iter().any(|(c, visible)| c == "#32770" && *visible) {
        Some("one of Explorer's dialogs is open")
    } else {
        None
    }
}

/// Whether Explorer holds enough of ours to be worth a restart: it takes both, as either alone can be something else.
pub fn worth_it(taps: usize, memory: u64) -> bool {
    taps >= TAPS_WORTH_IT && memory >= MEMORY_WORTH_IT
}

unsafe extern "system" fn add_window(hwnd: HWND, list: LPARAM) -> BOOL {
    let (found, pid) = unsafe { &mut *(list.0 as *mut (Vec<(String, bool)>, u32)) };
    let mut owner = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
    if owner == *pid {
        let mut class = [0u16; 64];
        let n = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
        found.push((String::from_utf16_lossy(&class[..n]), unsafe { IsWindowVisible(hwnd) }.as_bool()));
    }
    BOOL(1)
}

/// The classes of the top-level windows of process `pid`, and whether each is visible.
fn windows_of(pid: u32) -> Vec<(String, bool)> {
    let mut list = (Vec::new(), pid);
    let _ = unsafe { EnumWindows(Some(add_window), LPARAM(&raw mut list as isize)) };
    list.0
}

/// Why Explorer shouldn't be restarted just now.
pub fn busy() -> Option<&'static str> {
    blocker(&windows_of(crate::taskbar_pid()?))
}

/// What process `pid` has committed.
pub fn memory(pid: u32) -> u64 {
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { return 0 };
    let mut counters = PROCESS_MEMORY_COUNTERS_EX { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
    let read = unsafe { GetProcessMemoryInfo(process, (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(), counters.cb) };
    let _ = unsafe { CloseHandle(process) };
    if read.is_ok() { counters.PrivateUsage as u64 } else { 0 }
}

/// Asks Explorer to leave and starts it again, if nothing's open in it. Takes seconds: not on a thread that's doing anything else.
pub fn restart() -> Result<(), String> {
    let began = Instant::now();
    let tray = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.map_err(|_| "there's no taskbar".to_string())?;
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(tray, Some(&mut pid)) };
    let open = windows_of(pid);
    if let Some(why) = blocker(&open) {
        return Err(format!("it wasn't restarted, because {why}"));
    }
    // A hidden copy window is a leftover, or a copy that's only just started: one of those shows itself within a second or so.
    if open.iter().any(|(class, visible)| class == "OperationStatusWindow" && !visible) {
        std::thread::sleep(COPY_SHOWS_WITHIN);
        if let Some(why) = blocker(&windows_of(pid)) {
            return Err(format!("it wasn't restarted, because {why}"));
        }
    }
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, false, pid) }.map_err(|e| e.to_string())?;
    unsafe { PostMessageW(Some(tray), EXIT_EXPLORER, WPARAM(0), LPARAM(0)) }.map_err(|e| e.to_string())?;
    let mut left = unsafe { WaitForSingleObject(process, LEAVES_WITHIN) } == WAIT_OBJECT_0;
    if !left {
        if has_taskbar() {
            let _ = unsafe { CloseHandle(process) };
            return Err("it didn't close, so it's been left running".into());
        }
        // Its taskbar has gone, and it hasn't. Something opened since would still be worth keeping, but not at the cost of a taskbar:
        // a new Explorer comes up beside it.
        let open = blocker(&windows_of(pid));
        crate::log(format!("Explorer closed its taskbar but didn't exit: {}", open.map_or("ending it".to_string(), |why| format!("leaving it running, because {why}"))));
        if open.is_none() {
            let _ = unsafe { TerminateProcess(process, 1) };
            left = unsafe { WaitForSingleObject(process, ENDED_WITHIN) } == WAIT_OBJECT_0;
        }
    }
    let _ = unsafe { CloseHandle(process) };
    std::thread::sleep(WINDOWS_MAY_RESTART_IT);
    // Started with a shell already there, it would only open a folder window.
    if !has_taskbar() {
        let windows = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        std::process::Command::new(windows.join("explorer.exe")).spawn().map_err(|e| format!("it closed, and couldn't be started again: {e}"))?;
    }
    let waited = Instant::now();
    while !has_taskbar() {
        if waited.elapsed() >= COMES_BACK_WITHIN {
            return Err("it closed, and the taskbar hasn't come back".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    crate::log(format!("restarted Explorer in {} ms{}", began.elapsed().as_millis(), if left { "" } else { ", with the old one still running" }));
    Ok(())
}

fn has_taskbar() -> bool {
    unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.is_ok()
}

/// The file that asks a running daemon for a restart.
pub fn request_file() -> PathBuf {
    pms_tap::data_dir().join("restart-explorer.txt")
}

/// You've asked for a restart. The daemon does it if there is one, as it has things to look after while that happens.
pub fn ask() -> Result<(), String> {
    if let Some(why) = busy() {
        return Err(format!("Explorer wasn't restarted, because {why}.{}", if why == FOLDER_OPEN { SEPARATE_PROCESS } else { "" }));
    }
    if crate::daemon::running() { std::fs::write(request_file(), "").map_err(|e| e.to_string()) } else { restart() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows(list: &[(&str, bool)]) -> Vec<(String, bool)> {
        list.iter().map(|(c, v)| (c.to_string(), *v)).collect()
    }

    #[test]
    fn it_never_goes_with_something_open() {
        assert_eq!(blocker(&windows(&[("Shell_TrayWnd", true), ("Progman", true)])), None);
        assert_eq!(blocker(&windows(&[("CabinetWClass", true)])), Some("a File Explorer window is open"));
        assert_eq!(blocker(&windows(&[("CabinetWClass", false)])), Some("a File Explorer window is open"), "even out of sight");
        assert_eq!(blocker(&windows(&[("CabinetWClass", true), ("OperationStatusWindow", true)])), Some("a file copy is running"), "that's the one that matters");
        assert_eq!(blocker(&windows(&[("OperationStatusWindow", false)])), None, "the last copy's window, kept hidden after it's done");
        assert_eq!(blocker(&windows(&[("#32770", true)])), Some("one of Explorer's dialogs is open"));
        assert_eq!(blocker(&windows(&[("#32770", false)])), None, "Explorer keeps hidden ones of its own");
    }

    #[test]
    fn it_takes_many_taps_and_plenty_of_memory() {
        assert!(!worth_it(1, 130 << 20));
        assert!(!worth_it(44, 200 << 20), "taps alone aren't enough");
        assert!(!worth_it(3, 2 << 30), "and nor is memory alone");
        assert!(worth_it(10, 300 << 20));
        assert!(worth_it(44, 360 << 20), "what this PC had");
    }

    #[test]
    fn this_pc_answers() {
        assert!(memory(std::process::id()) > 0);
    }
}
