//! Client side of PimpMyShell: talking to the TAP inside explorer, injecting it, and the pieces the
//! daemon is built from (config, perf-counter sampler, microphone watcher).

pub mod config;
pub mod daemon;
mod expr;
mod hotmic;
pub mod meters;
mod nvml;
pub mod signing;
mod toasts;
mod tray;
pub mod update;
pub mod version;
pub mod watchdog;

use std::fs::File;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub use pms_tap::ipc::quote;
use pms_tap::ipc::{PIPE_NAME, read_frame, write_frame};
use pms_tap::{CLSID_PMS_TAP, data_dir, xamlom::InitializeXamlDiagnosticsExFn};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowThreadProcessId};
use windows_core::{HSTRING, PCWSTR, s, w};

// ---------------------------------------------------------------------------------------------
// Logging

static LOG_TO_FILE: AtomicBool = AtomicBool::new(false);

/// Also append log lines to `%LOCALAPPDATA%\PimpMyShell\daemon.log` (the windowless daemon has no
/// console), which `pms_tap::logfile` keeps to three files of 1 MB.
pub fn log_to_file() {
    LOG_TO_FILE.store(true, Ordering::Release);
}

pub fn log(msg: impl AsRef<str>) {
    let t = unsafe { GetLocalTime() };
    let line = format!("{:02}:{:02}:{:02} {}", t.wHour, t.wMinute, t.wSecond, msg.as_ref());
    eprintln!("{line}");
    if LOG_TO_FILE.load(Ordering::Acquire) {
        pms_tap::logfile::append(pms_tap::logfile::Log::Daemon, &line);
    }
}

// ---------------------------------------------------------------------------------------------
// Pipe client

pub fn connect(timeout: Duration) -> std::io::Result<File> {
    let deadline = Instant::now() + timeout;
    loop {
        match File::options().read(true).write(true).open(PIPE_NAME) {
            Ok(f) => return Ok(f),
            Err(e)
                if Instant::now() < deadline
                    && [ERROR_PIPE_BUSY.0, ERROR_FILE_NOT_FOUND.0].contains(&(e.raw_os_error().unwrap_or(0) as u32)) =>
            {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(e) => return Err(e),
        }
    }
}

pub fn request(conn: &mut File, line: &str) -> Result<String, String> {
    write_frame(conn, line).map_err(|e| e.to_string())?;
    read_frame(conn).map_err(|e| e.to_string())?.ok_or_else(|| "TAP hung up".to_string())
}

/// One request on a fresh connection, so long-running clients never hog the single-instance pipe.
pub fn send(line: &str) -> Result<String, String> {
    let mut conn = connect(Duration::from_secs(1)).map_err(|e| e.to_string())?;
    request(&mut conn, line)
}

/// True when another client holds the (single-instance) pipe.
pub fn busy(err: &str) -> bool {
    err.contains(&format!("os error {}", ERROR_PIPE_BUSY.0))
}

/// True when the TAP's pipe doesn't exist at all (as opposed to busy or misbehaving).
pub fn no_tap(err: &str) -> bool {
    err.contains(&format!("os error {}", ERROR_FILE_NOT_FOUND.0))
}

// ---------------------------------------------------------------------------------------------
// Injection

/// Explorer's pid, once the taskbar exists and hosts its XAML island.
pub fn taskbar_pid() -> Option<u32> {
    let tray = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.ok()?;
    unsafe { FindWindowExW(Some(tray), None, w!("Windows.UI.Composition.DesktopWindowContentBridge"), PCWSTR::null()) }.ok()?;
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(tray, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

/// Detach any live TAP and inject a fresh copy of `pms_tap.dll` (next to this exe).
pub fn inject() -> Result<String, String> {
    let mut out = Vec::new();
    if let Ok(mut old) = connect(Duration::ZERO) {
        match request(&mut old, "detach") {
            Ok(r) => out.push(format!("old TAP: {r}")),
            Err(e) => out.push(format!("old TAP didn't answer detach: {e}")),
        }
    }
    let pid = taskbar_pid().ok_or("no taskbar yet (Shell_TrayWnd without its XAML island)")?;

    // Explorer keeps each loaded copy locked until it exits, so inject a fresh copy each time,
    // and sweep up the copies that are no longer loaded anywhere.
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("pms_tap-") && name.ends_with(".dll") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let src = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("pms_tap.dll");
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let dll = dir.join(format!("pms_tap-{stamp}.dll"));
    std::fs::copy(&src, &dll).map_err(|e| format!("copy {}: {e}", src.display()))?;

    let init: InitializeXamlDiagnosticsExFn = unsafe {
        let xaml = LoadLibraryW(w!("Windows.UI.Xaml.dll")).map_err(|e| e.to_string())?;
        let f = GetProcAddress(xaml, s!("InitializeXamlDiagnosticsEx")).ok_or("no InitializeXamlDiagnosticsEx")?;
        std::mem::transmute(f)
    };
    let dll_w = HSTRING::from(dll.as_os_str());
    let mut injected = false;
    for i in 1..=100 {
        let endpoint = HSTRING::from(format!("VisualDiagConnection{i}"));
        let hr = unsafe { init(PCWSTR(endpoint.as_ptr()), pid, PCWSTR::null(), PCWSTR(dll_w.as_ptr()), CLSID_PMS_TAP, PCWSTR::null()) };
        if hr == ERROR_ALREADY_EXISTS.to_hresult() {
            continue;
        }
        hr.ok().map_err(|e| format!("InitializeXamlDiagnosticsEx: {e}"))?;
        injected = true;
        break;
    }
    if !injected {
        return Err("ran out of diagnostics endpoints".into());
    }
    out.push(format!("injected {} into explorer pid {pid}", dll.display()));

    let mut conn = connect(Duration::from_secs(10)).map_err(|e| format!("new TAP never opened its pipe: {e}"))?;
    out.push(request(&mut conn, "ping")?);
    Ok(out.join("\n"))
}

/// `PMS_TRACE=1`: log timing details (sampling, pushes) for diagnosis.
pub fn trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PMS_TRACE").is_some_and(|v| v != "0"))
}
