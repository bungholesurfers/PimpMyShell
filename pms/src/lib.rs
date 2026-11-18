//! Client side of PimpMyShell: talking to the TAP inside explorer, injecting it, and the pieces the
//! daemon is built from (config, perf-counter sampler, microphone watcher).

pub mod config;
mod layers;
pub mod daemon;
pub mod demos;
mod devices;
pub mod efficiency;
pub mod explorer;
pub mod fetch;
pub mod games;
mod expr;
mod hotmic;
pub mod iss;
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
pub mod relay;
pub mod sandbox;
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

/// What PimpMyShell says, in every language it speaks: the texts, compiled in (pms-lang, from `lang/`).
pub use pms_lang as lang;
pub use pms_tap::ipc::quote;
/// The question software update asks once (examples/dialogs.rs shows it).
pub use tray::{ask_click, update_question};
use pms_tap::ipc::{OLD_PIPE_NAME, pipe_name, read_frame, write_frame};
use pms_tap::{CLSID_PMS_TAP, data_dir, xamlom::InitializeXamlDiagnosticsExFn};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, HANDLE};
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

/// What `connect` says of a pipe with the TAP's name that isn't Explorer's (`not_ours` knows it by this).
pub const NOT_OURS: &str = "something that isn't PimpMyShell's part in Explorer is answering on its pipe";

/// A connection to the TAP in this session's Explorer, and to nothing else that has its pipe's name.
pub fn connect(timeout: Duration) -> std::io::Result<File> {
    connect_to(&pipe_name(), timeout)
}

fn connect_to(name: &str, timeout: Duration) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;
    let deadline = Instant::now() + timeout;
    loop {
        // SECURITY_IDENTIFICATION: whatever serves the pipe may learn who we are, but not act as us. (Left unsaid, it may.)
        match File::options().read(true).write(true).security_qos_flags(SECURITY_IDENTIFICATION.0).open(name) {
            // Before a word is sent or believed. A pipe's name is anyone's to take who gets to it first, and what the daemon
            // sends (your notifications' text) and does on the answers (presses their buttons) is for Explorer alone.
            Ok(f) if explorers(&f) => return Ok(f),
            Ok(_) => return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, NOT_OURS)),
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

/// Whether a pipe we've opened is the one the TAP serves from Explorer: made by us, and served by the taskbar's process.
fn explorers(pipe: &File) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    let pipe = HANDLE(pipe.as_raw_handle());
    let mut server = 0;
    made_by_us(pipe) && unsafe { GetNamedPipeServerProcessId(pipe, &mut server) }.is_ok() && Some(server) == tray_pid()
}

/// Whether a pipe was made by a process that's as much us as we are: it has our user for its owner, and it isn't marked
/// as made at low integrity. Those two are the pipe's own, set by Windows when it's made, and can't be made up. Another
/// account's pipe has that account for its owner, and can only say otherwise with a privilege that would let it do as it
/// liked anyway. A low-integrity process of ours (something sandboxed) does make pipes that we own, but Windows labels each
/// one low, and won't let it claim a higher label. A pipe that won't let us read these isn't ours either.
///
/// (The server's process number alone wouldn't do: it's the number of whatever made the pipe, which may be gone, with the
/// number given to something else since.)
fn made_by_us(pipe: HANDLE) -> bool {
    use windows::Win32::Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT};
    use windows::Win32::Security::{ACL, GetLengthSid, IsValidSid, LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID};
    let (mut owner, mut sacl, mut descriptor) = (PSID::default(), std::ptr::null_mut::<ACL>(), PSECURITY_DESCRIPTOR::default());
    let what = OWNER_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION;
    if unsafe { GetSecurityInfo(pipe, SE_KERNEL_OBJECT, what, Some(&mut owner), None, None, Some(&mut sacl), Some(&mut descriptor)) } != ERROR_SUCCESS {
        return false;
    }
    let ours = !owner.is_invalid() && unsafe { IsValidSid(owner) }.as_bool() && {
        let owner = unsafe { std::slice::from_raw_parts(owner.0 as *const u8, GetLengthSid(owner) as usize) };
        us().iter().any(|sid| sid == owner)
    };
    let low = label(sacl).is_some_and(|level| level < MEDIUM_INTEGRITY);
    unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    ours && !low
}

/// SECURITY_MANDATORY_MEDIUM_RID: the integrity level of an ordinary process, and of what has no label.
const MEDIUM_INTEGRITY: u32 = 0x2000;

/// The integrity level an object's label gives it, if it has a label.
fn label(sacl: *const windows::Win32::Security::ACL) -> Option<u32> {
    use windows::Win32::Security::{GetAce, GetSidSubAuthority, GetSidSubAuthorityCount, PSID, SYSTEM_MANDATORY_LABEL_ACE};
    /// SYSTEM_MANDATORY_LABEL_ACE_TYPE.
    const LABEL: u8 = 0x11;
    if sacl.is_null() {
        return None;
    }
    (0..u32::from(unsafe { (*sacl).AceCount })).find_map(|i| {
        let mut ace = std::ptr::null_mut();
        unsafe { GetAce(sacl, i, &mut ace) }.ok()?;
        let ace = ace as *const SYSTEM_MANDATORY_LABEL_ACE;
        if unsafe { (*ace).Header.AceType } != LABEL {
            return None;
        }
        // The level is the last number of the label's SID (S-1-16-4096 is low).
        let sid = PSID(unsafe { &raw const (*ace).SidStart } as *mut std::ffi::c_void);
        let last = u32::from(unsafe { *GetSidSubAuthorityCount(sid) }).checked_sub(1)?;
        Some(unsafe { *GetSidSubAuthority(sid, last) })
    })
}

/// Who we are to Windows, as SIDs' bytes: our user, and the owner Windows gives to what we make (the same, except in an
/// administrator's elevated process, where that can be the Administrators group).
fn us() -> &'static [Vec<u8>] {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Security::{GetLengthSid, GetTokenInformation, PSID, TOKEN_QUERY, TokenOwner, TokenUser};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    static US: std::sync::OnceLock<Vec<Vec<u8>>> = std::sync::OnceLock::new();
    US.get_or_init(|| {
        let mut token = HANDLE::default();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
            return Vec::new();
        }
        let sids = [TokenUser, TokenOwner]
            .into_iter()
            .filter_map(|class| {
                // Either answer starts with a pointer to the SID, which follows it in the same memory.
                let mut answer = [0u64; 16];
                let mut length = 0;
                unsafe { GetTokenInformation(token, class, Some(answer.as_mut_ptr().cast()), size_of_val(&answer) as u32, &mut length) }.ok()?;
                let sid = PSID(answer[0] as *mut std::ffi::c_void);
                Some(unsafe { std::slice::from_raw_parts(sid.0 as *const u8, GetLengthSid(sid) as usize) }.to_vec())
            })
            .collect();
        unsafe { CloseHandle(token) }.ok();
        sids
    })
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

/// True when the TAP's pipe is there, but it's something other than Explorer serving it (`NOT_OURS`).
pub fn not_ours(err: &str) -> bool {
    err.contains(NOT_OURS)
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

/// The pid of the process the taskbar's window belongs to, whether or not its XAML is up: Explorer, where a TAP lives.
fn tray_pid() -> Option<u32> {
    let tray = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.ok()?;
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
    // Under the name of now, and under the one a TAP from 0.1.7 or before answers to: after an update, that's what Explorer
    // still has, and it wouldn't otherwise hear that it's to go.
    for name in [pipe_name(), OLD_PIPE_NAME.to_string()] {
        if let Ok(mut old) = connect_to(&name, Duration::ZERO) {
            match request(&mut old, "detach") {
                Ok(r) => out.push(format!("old TAP: {r}")),
                Err(e) => out.push(format!("old TAP didn't answer detach: {e}")),
            }
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

/// A question's arguments for `pms-gui --ask`, as they go down its stdin: one to a line, a backslash and a line's end
/// written `\\` and `\n`. Not on its command line, where what a notification said, and the paths and arguments it would
/// open, would be there for any program to read.
pub fn question_lines(args: &[String]) -> String {
    args.iter().map(|a| a.replace('\\', "\\\\").replace('\r', "").replace('\n', "\\n") + "\n").collect()
}

/// The arguments `question_lines` wrote.
pub fn question_args(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| {
            let mut out = String::with_capacity(line.len());
            let mut chars = line.chars();
            while let Some(c) = chars.next() {
                match (c, c == '\\') {
                    (_, true) => match chars.next() {
                        Some('n') => out.push('\n'),
                        Some(other) => out.push(other),
                        None => out.push('\\'),
                    },
                    (c, false) => out.push(c),
                }
            }
            out
        })
        .collect()
}

/// From now on this process speaks what the settings say: `language` as they have it, a code or "auto", which is Windows' own
/// language where PimpMyShell speaks it, and English where it doesn't.
pub fn speak(language: &str) {
    lang::set(lang::choose(language, windows_languages()));
}

/// What the settings keep for language `chosen` (its place in `lang::LANGUAGES`): its code, or "auto" where that's the one
/// Windows' own language gives anyway, so that somebody who never chose differently keeps following Windows.
pub fn language_setting(chosen: usize) -> String {
    let code = lang::LANGUAGES.get(chosen).map_or(lang::AUTO, |l| l.code);
    if lang::choose(lang::AUTO, windows_languages()) == chosen { lang::AUTO.to_string() } else { code.to_string() }
}

/// The languages Windows shows itself in for whoever's signed in, as it names them, the preferred one first: "en-US".
/// Asked once: a change takes signing out to show anywhere else, too.
fn windows_languages() -> &'static [String] {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserPreferredUILanguages(flags: u32, count: *mut u32, names: *mut u16, room: *mut u32) -> i32;
    }
    /// MUI_LANGUAGE_NAME: as names, not numbers.
    const BY_NAME: u32 = 8;
    static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        let (mut count, mut room) = (0u32, 0u32);
        // First how much room the names take, then the names: each ends with a zero, and the list with another.
        unsafe { GetUserPreferredUILanguages(BY_NAME, &mut count, std::ptr::null_mut(), &mut room) };
        let mut names = vec![0u16; room as usize];
        if room == 0 || unsafe { GetUserPreferredUILanguages(BY_NAME, &mut count, names.as_mut_ptr(), &mut room) } == 0 {
            return Vec::new();
        }
        names.split(|c| *c == 0).filter(|name| !name.is_empty()).map(String::from_utf16_lossy).collect()
    })
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

#[cfg(test)]
mod pipe_tests {
    use super::*;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_TYPE_BYTE};

    /// A pipe of this test's own, with `sddl` for its security, served by this process: its server's end, and its name.
    fn pipe(called: &str, sddl: &str) -> (File, String) {
        let name = format!(r"\\.\pipe\pms-test-{called}-{}", std::process::id());
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&HSTRING::from(sddl), SDDL_REVISION_1, &mut descriptor, None) }.unwrap();
        let security = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor.0, bInheritHandle: false.into() };
        let pipe = unsafe { CreateNamedPipeW(&HSTRING::from(name.as_str()), PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_TYPE_BYTE, 1, 4096, 4096, 0, Some(&security)) };
        assert!(!pipe.is_invalid(), "{name}");
        (unsafe { File::from_raw_handle(pipe.0) }, name)
    }

    /// The security the TAP gives its pipe (pms-tap's server.rs).
    const THE_TAPS: &str = "D:P(A;;GA;;;SY)(A;;GA;;;OW)";

    #[test]
    fn a_pipe_we_made_is_ours_and_one_made_at_low_integrity_is_not() {
        let (_server, name) = pipe("ours", THE_TAPS);
        let client = File::options().read(true).write(true).open(&name).unwrap();
        assert!(made_by_us(HANDLE(client.as_raw_handle())));

        // Labelled low, as Windows labels what a low-integrity process makes (and such a process can't ask for better): not
        // ours, though it's our user that owns it.
        let (_server, name) = pipe("low", "D:(A;;GA;;;WD)S:(ML;;NW;;;LW)");
        let client = File::options().read(true).write(true).open(&name).unwrap();
        assert!(!made_by_us(HANDLE(client.as_raw_handle())));
    }

    #[test]
    fn only_explorers_pipe_is_connected_to() {
        // Ours, and with the TAP's own security: but this process serves it, not the taskbar's.
        let (_server, name) = pipe("here", THE_TAPS);
        let e = connect_to(&name, Duration::ZERO).unwrap_err().to_string();
        assert!(not_ours(&e) && !no_tap(&e) && !busy(&e), "{e}");
        // One that isn't there is still "no TAP", which is what has the daemon put one in.
        let e = connect_to(r"\\.\pipe\pms-test-there-is-none", Duration::ZERO).unwrap_err().to_string();
        assert!(no_tap(&e) && !not_ours(&e), "{e}");
    }

    #[test]
    fn each_session_has_a_pipe_of_its_own() {
        let name = pipe_name();
        assert!(name.starts_with(r"\\.\pipe\pms-tap-") && name != OLD_PIPE_NAME, "{name}");
        assert!(name[r"\\.\pipe\pms-tap-".len()..].parse::<u32>().is_ok(), "{name}");
    }
}

#[cfg(test)]
mod question_tests {
    use super::*;

    #[test]
    fn a_questions_arguments_come_back_as_they_went() {
        let args: Vec<String> = ["--heading", "Open this?", "--row", "Path", "C:\\Users\\me\\a \"b\".exe", "--text", "two\nlines\\n"].map(String::from).to_vec();
        assert_eq!(question_args(&question_lines(&args)), args);
        assert_eq!(question_args(""), Vec::<String>::new());
    }
}

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn windows_says_what_it_speaks_and_the_settings_have_the_last_word() {
        let windows = windows_languages();
        assert!(windows.iter().all(|name| !name.is_empty() && name.is_ascii()), "{windows:?}");
        // Whatever Windows speaks, following it is "auto", and anything else is that language's own code.
        let auto = lang::choose(lang::AUTO, windows);
        assert_eq!(language_setting(auto), lang::AUTO);
        for (chosen, language) in lang::LANGUAGES.iter().enumerate().filter(|(chosen, _)| *chosen != auto) {
            assert_eq!(language_setting(chosen), language.code);
            assert_eq!(lang::choose(&language_setting(chosen), windows), chosen);
        }
    }
}
