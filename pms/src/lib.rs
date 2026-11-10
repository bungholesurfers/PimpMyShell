//! Client side of PimpMyShell: talking to the TAP inside explorer, injecting it, and the pieces the
//! daemon is built from (config, perf-counter sampler, microphone watcher).

pub mod config;
mod layers;
pub mod daemon;
mod devices;
pub mod explorer;
pub mod games;
mod expr;
mod hotmic;
pub mod meters;
pub mod net;
mod nvml;
pub mod signing;
pub mod system;
mod load;
pub mod parts;
mod pimpcast;
pub mod pressure;
pub mod problem;
pub mod procs;
pub mod streamlink;
pub mod tv;
pub mod twatch;
pub mod wads;
pub mod notices;
mod toasts;
mod tray;
pub mod update;
pub mod version;
pub mod watchdog;

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub use pms_tap::ipc::quote;
/// The question software update asks once (examples/dialogs.rs shows it).
pub use tray::{ask_click, update_question};
use pms_tap::ipc::{PIPE_NAME, read_frame, write_frame};
use pms_tap::{CLSID_PMS_TAP, data_dir, xamlom::InitializeXamlDiagnosticsExFn};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowThreadProcessId};
use windows_core::{HSTRING, PCWSTR, s, w};

// ---------------------------------------------------------------------------------------------
// Logging

static LOG_TO_FILE: AtomicBool = AtomicBool::new(false);

/// Also append log lines to `%APPDATA%\PimpMyShell\logs\daemon.log` (the windowless daemon has no
/// console), which `pms_tap::logfile` keeps to three files of 1 MB.
pub fn log_to_file() {
    LOG_TO_FILE.store(true, Ordering::Release);
}

pub fn log(msg: impl AsRef<str>) {
    let line = pms_tap::logfile::redact(&format!("{} {}", pms_tap::logfile::stamp(), msg.as_ref()));
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

/// Whether `path` is the name of a TAP we've put somewhere for Explorer to load.
fn is_tap_copy(path: &str) -> bool {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path).to_ascii_lowercase();
    name.starts_with("pms_tap-") && name.ends_with(".dll")
}

fn file_info(path: &Path) -> Option<windows::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::GetFileInformationByHandle;
    let file = File::open(path).ok()?;
    let mut info = Default::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.ok()?;
    Some(info)
}

/// What a file is, whatever it's called: two hard links to it say the same.
fn file_id(path: &Path) -> Option<(u32, u32, u32)> {
    file_info(path).map(|info| (info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow))
}

/// Deletes the TAPs in `dir` that nothing has loaded. Deleting one that is loaded could work, with another name for it left, and
/// that name would then be the last, which an update couldn't replace: so `loaded` says which to leave.
fn sweep_taps(dir: &Path, loaded: &[(u32, u32, u32)]) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if is_tap_copy(&path.to_string_lossy()) && file_id(&path).is_some_and(|id| !loaded.contains(&id)) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Puts the TAP at `from` where Explorer can load it as `to`, a name of its own, so that the file `from` names can be replaced
/// meanwhile. A hard link does that with nothing written but the name. It's a copy if Explorer has that very file loaded, inert
/// from an earlier time: Windows would hand back that one, not start it afresh. And if the file has other names already, which
/// makes it something else's to look after, like a build's output, that would then find it locked. And on another drive.
fn place_tap(from: &Path, to: &Path, loaded: Option<&[(u32, u32, u32)]>) -> Result<(), String> {
    let fresh = loaded.is_some_and(|ids| file_id(from).is_some_and(|id| !ids.contains(&id)));
    let only_name = file_info(from).is_some_and(|info| info.nNumberOfLinks == 1);
    if fresh && only_name && std::fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map(drop).map_err(|e| format!("copy {}: {e}", from.display()))
}

/// Detach any live TAP and inject a fresh `pms_tap.dll` (next to this exe).
pub fn inject() -> Result<String, String> {
    let mut out = Vec::new();
    if let Ok(mut old) = connect(Duration::ZERO) {
        match request(&mut old, "detach") {
            Ok(r) => out.push(format!("old TAP: {r}")),
            Err(e) => out.push(format!("old TAP didn't answer detach: {e}")),
        }
    }
    let pid = taskbar_pid().ok_or("no taskbar yet (Shell_TrayWnd without its XAML island)")?;

    // Explorer holds on to each TAP it loads until it exits: each gets a name of its own, and the names nothing has loaded any
    // more are swept up, here and where they used to go.
    let dir = data_dir().join("tap");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let loaded: Option<Vec<_>> = system::loaded_taps(pid).map(|taps| taps.iter().filter_map(|t| file_id(t)).collect());
    if let Some(loaded) = &loaded {
        sweep_taps(&dir, loaded);
        sweep_taps(&data_dir(), loaded);
    }
    let src = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("pms_tap.dll");
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let dll = dir.join(format!("pms_tap-{stamp}.dll"));
    place_tap(&src, &dll, loaded.as_deref())?;

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

/// `path` as the settings and notices show it: under a folder Windows has a name for, by that name (`%LOCALAPPDATA%\PimpMyShell`),
/// so that what's on screen doesn't spell out the Windows user name. Only for reading: a path to open is the path itself.
pub fn shown(path: &std::path::Path) -> String {
    let vars: Vec<(&str, String)> = ["LOCALAPPDATA", "APPDATA", "USERPROFILE"].iter().filter_map(|v| std::env::var(v).ok().map(|p| (*v, p))).collect();
    shown_with(&path.display().to_string(), &vars)
}

/// `shown`, with the folders' names and where they are: the first that `path` is in, or is, goes by its name.
fn shown_with(path: &str, vars: &[(&str, String)]) -> String {
    for (name, place) in vars {
        let place = place.trim_end_matches('\\');
        if place.is_empty() || path.len() < place.len() || !path.is_char_boundary(place.len()) {
            continue;
        }
        let (head, rest) = path.split_at(place.len());
        if head.eq_ignore_ascii_case(place) && (rest.is_empty() || rest.starts_with('\\')) {
            return format!("%{name}%{rest}");
        }
    }
    path.to_string()
}

/// `PMS_TRACE=1`: log timing details (sampling, pushes) for diagnosis.
pub fn trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("PMS_TRACE").is_some_and(|v| v != "0"))
}

#[cfg(test)]
mod shown_tests {
    use super::shown_with;

    #[test]
    fn paths_go_by_their_folders_names() {
        let vars = [("LOCALAPPDATA", r"C:\Users\Bob\AppData\Local".to_string()), ("APPDATA", r"C:\Users\Bob\AppData\Roaming".to_string()), ("USERPROFILE", r"C:\Users\Bob".to_string())];
        assert_eq!(shown_with(r"C:\Users\Bob\AppData\Local\PimpMyShell\logs", &vars), r"%LOCALAPPDATA%\PimpMyShell\logs");
        assert_eq!(shown_with(r"c:\users\bob\appdata\roaming\PimpMyShell\pms.toml", &vars), r"%APPDATA%\PimpMyShell\pms.toml");
        assert_eq!(shown_with(r"C:\Users\Bob\Desktop\x.toml", &vars), r"%USERPROFILE%\Desktop\x.toml");
        // Not somebody else whose name starts the same, and not elsewhere.
        assert_eq!(shown_with(r"C:\Users\Bobby\x", &vars), r"C:\Users\Bobby\x");
        assert_eq!(shown_with(r"D:\Games", &vars), r"D:\Games");
    }
}

#[cfg(test)]
mod tap_tests {
    use super::*;
    use std::path::PathBuf;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pms-tap-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_tap_is_known_by_its_name() {
        assert!(is_tap_copy(r"C:\Users\x\AppData\Local\PimpMyShell\tap\pms_tap-1794.dll"));
        assert!(is_tap_copy(r"C:\A\PMS_TAP-1.DLL"));
        assert!(!is_tap_copy(r"C:\Program Files\PimpMyShell\pms_tap.dll"), "the installed one is never one to sweep");
        assert!(!is_tap_copy(r"C:\pms_tap-1.dll\other.dll"));
    }

    #[test]
    fn a_new_tap_is_a_link_and_one_explorer_has_already_is_a_copy() {
        let dir = folder("place");
        let from = dir.join("pms_tap.dll");
        std::fs::write(&from, "tap").unwrap();
        let id = file_id(&from).unwrap();

        let (linked, copied, unsure) = (dir.join("pms_tap-1.dll"), dir.join("pms_tap-2.dll"), dir.join("pms_tap-3.dll"));
        place_tap(&from, &linked, Some(&[])).unwrap();
        assert_eq!(file_id(&linked), Some(id), "nothing written but a name");
        place_tap(&from, &copied, Some(&[id])).unwrap();
        assert_ne!(file_id(&copied), Some(id), "Windows would hand back the one it has");
        place_tap(&from, &unsure, None).unwrap();
        assert_ne!(file_id(&unsure), Some(id), "when we can't tell, the safe one");
        assert_eq!(std::fs::read(&copied).unwrap(), b"tap");

        let (built, also, placed) = (dir.join("built.dll"), dir.join("built-too.dll"), dir.join("pms_tap-4.dll"));
        std::fs::write(&built, "tap").unwrap();
        std::fs::hard_link(&built, &also).unwrap();
        place_tap(&built, &placed, Some(&[])).unwrap();
        assert_ne!(file_id(&placed), file_id(&built), "a build's output has two names, and a build mustn't find it locked");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_sweep_leaves_what_is_loaded_and_everything_else_of_ours() {
        let dir = folder("sweep");
        let (kept, gone, other) = (dir.join("pms_tap-1.dll"), dir.join("pms_tap-2.dll"), dir.join("pms_tap.dll"));
        for file in [&kept, &gone, &other] {
            std::fs::write(file, "x").unwrap();
        }
        sweep_taps(&dir, &[file_id(&kept).unwrap()]);
        assert!(kept.exists() && other.exists());
        assert!(!gone.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
