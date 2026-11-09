//! What PimpMyShell is running on and among, for the log and for telling people why it can't help: the Windows
//! version, the versions of what we hook (explorer.exe and the taskbar's code), and the software that replaces or
//! changes the taskbar. A report from someone's PC starts with these.
//!
//! Only names and versions go in the log: file names, not paths (a path can carry a user name).

use std::mem::size_of;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId, IsWindowVisible};
use windows_core::{HSTRING, PCWSTR, w};

/// What a piece of taskbar software does to us.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    /// Can swap the Windows 11 taskbar for a classic one, inside explorer: then there's no XAML to hook. It may be
    /// installed with that off (just a Start menu, say), so this alone proves nothing.
    Replaces,
    /// Draws a bar of its own, and hides Windows' while it runs.
    Hides,
    /// Hooks the Windows 11 taskbar, as we do: it's still there, but maybe not as we know it.
    Changes,
}

/// Software we know of, and how to spot it: a substring of a module's path in explorer (lowercase), or a process.
pub struct Known {
    pub name: &'static str,
    pub role: Role,
    modules: &'static [&'static str],
    processes: &'static [&'static str],
}

pub const KNOWN: &[Known] = &[
    // Its `dxgi.dll` sits in the Windows folder, ahead of System32's.
    Known { name: "ExplorerPatcher", role: Role::Replaces, modules: &["\\windows\\dxgi.dll", "\\ep_taskbar.", "\\explorerpatcher\\"], processes: &[] },
    Known { name: "StartAllBack", role: Role::Replaces, modules: &["\\startallback"], processes: &[] },
    Known { name: "Start11", role: Role::Replaces, modules: &["\\stardock\\start11\\"], processes: &[] },
    Known { name: "RetroBar", role: Role::Hides, modules: &[], processes: &["retrobar.exe"] },
    Known { name: "Cairo", role: Role::Hides, modules: &[], processes: &["cairodesktop.exe"] },
    Known { name: "Seelen UI", role: Role::Hides, modules: &[], processes: &["seelen-ui.exe"] },
    Known { name: "Windhawk", role: Role::Changes, modules: &["\\windhawk\\"], processes: &["windhawk.exe"] },
    Known { name: "TranslucentTB", role: Role::Changes, modules: &["\\translucenttb"], processes: &["translucenttb.exe"] },
];

/// The software in `KNOWN` that these modules (paths) and processes (names) give away, whatever their case.
fn spot(modules: &[String], processes: &[String]) -> Vec<&'static Known> {
    let modules: Vec<String> = modules.iter().map(|m| m.to_lowercase()).collect();
    let processes: Vec<String> = processes.iter().map(|p| p.to_lowercase()).collect();
    KNOWN
        .iter()
        .filter(|k| k.modules.iter().any(|n| modules.iter().any(|m| m.contains(n))) || k.processes.iter().any(|n| processes.iter().any(|p| p == n)))
        .collect()
}

/// Whether a module in explorer is someone else's: not Windows', Microsoft's packaged runtimes or ours. Windows' own
/// live in folders of the Windows folder; a DLL right in it, as ExplorerPatcher's `dxgi.dll` is, doesn't belong there.
fn third_party(path: &str, windows: &str) -> bool {
    let (path, windows) = (path.to_lowercase(), windows.to_lowercase());
    let of_windows = path.strip_prefix(&format!("{windows}\\")).is_some_and(|rest| rest.contains('\\') || rest == "explorer.exe");
    let file = path.rsplit('\\').next().unwrap_or("");
    // Windows' own folders in Program Files too: Windows Photo Viewer, Windows Defender, ...
    !(of_windows || path.contains("\\windowsapps\\microsoft") || path.contains("\\program files\\windows ") || file.starts_with("pms_tap"))
}

fn text(wide: &[u16]) -> String {
    String::from_utf16_lossy(&wide[..wide.iter().position(|&c| c == 0).unwrap_or(wide.len())])
}

/// The paths of the modules loaded in process `pid`.
fn modules(pid: u32) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) }) else { return out };
    let mut e = MODULEENTRY32W { dwSize: size_of::<MODULEENTRY32W>() as u32, ..Default::default() };
    let mut more = unsafe { Module32FirstW(snap, &mut e) }.is_ok();
    while more {
        out.push(text(&e.szExePath));
        more = unsafe { Module32NextW(snap, &mut e) }.is_ok();
    }
    close(snap);
    out
}

/// The TAPs loaded in process `pid`, which may be several, one of them working: None if that can't be found out.
pub(crate) fn loaded_taps(pid: u32) -> Option<Vec<std::path::PathBuf>> {
    let all = modules(pid);
    (!all.is_empty()).then(|| all.iter().filter(|m| crate::is_tap_copy(m)).map(std::path::PathBuf::from).collect())
}

/// The names of the running processes.
fn processes() -> Vec<String> {
    let mut out = Vec::new();
    let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else { return out };
    let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut more = unsafe { Process32FirstW(snap, &mut e) }.is_ok();
    while more {
        out.push(text(&e.szExeFile));
        more = unsafe { Process32NextW(snap, &mut e) }.is_ok();
    }
    close(snap);
    out
}

fn close(h: HANDLE) {
    let _ = unsafe { CloseHandle(h) };
}

/// A file's version, from its version resource: `10.0.26100.1234`.
pub fn file_version(path: &Path) -> Option<String> {
    let name = HSTRING::from(path);
    let size = unsafe { GetFileVersionInfoSizeW(&name, None) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    unsafe { GetFileVersionInfoW(&name, None, size, data.as_mut_ptr().cast()) }.ok()?;
    let (mut info, mut len) = (std::ptr::null_mut(), 0u32);
    if !unsafe { VerQueryValueW(data.as_ptr().cast(), w!("\\"), &mut info, &mut len) }.as_bool() || (len as usize) < size_of::<VS_FIXEDFILEINFO>() {
        return None;
    }
    let f = unsafe { &*(info as *const VS_FIXEDFILEINFO) };
    Some(format!("{}.{}.{}.{}", f.dwFileVersionMS >> 16, f.dwFileVersionMS & 0xFFFF, f.dwFileVersionLS >> 16, f.dwFileVersionLS & 0xFFFF))
}

const CURRENT_VERSION: PCWSTR = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");

fn registry_text(value: PCWSTR) -> Option<String> {
    let mut buffer = [0u16; 128];
    let mut size = size_of::<[u16; 128]>() as u32;
    unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, CURRENT_VERSION, value, RRF_RT_REG_SZ, None, Some(buffer.as_mut_ptr().cast()), Some(&mut size)) }.ok().ok()?;
    Some(text(&buffer))
}

fn registry_number(value: PCWSTR) -> Option<u32> {
    let mut number = 0u32;
    let mut size = size_of::<u32>() as u32;
    unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, CURRENT_VERSION, value, RRF_RT_REG_DWORD, None, Some((&mut number as *mut u32).cast()), Some(&mut size)) }.ok().ok()?;
    Some(number)
}

/// `Windows 11 Enterprise 25H2, build 26300.1234`. (`ProductName` won't do: it still says Windows 10 on 11.)
pub fn windows_version() -> String {
    let build = registry_text(w!("CurrentBuild")).unwrap_or_default();
    let number: u32 = build.parse().unwrap_or(0);
    let edition = registry_text(w!("EditionID")).unwrap_or_default();
    let release = registry_text(w!("DisplayVersion")).unwrap_or_default();
    let patch = registry_number(w!("UBR")).unwrap_or(0);
    let name = format!("Windows {} {edition} {release}", if number >= 22000 { 11 } else { 10 });
    format!("{}, build {build}.{patch}", name.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The process that has the taskbar's window, whatever draws in it.
pub fn shell_pid() -> Option<u32> {
    let tray = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.ok()?;
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(tray, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

/// Whether Windows' taskbar exists but is hidden (an auto-hiding one stays "visible", off the screen's edge).
pub fn taskbar_hidden() -> bool {
    unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.is_ok_and(|tray| !unsafe { IsWindowVisible(tray) }.as_bool())
}

/// The software in `KNOWN` around the shell `pid`.
pub fn taskbar_software(pid: u32) -> Vec<&'static Known> {
    spot(&modules(pid), &processes())
}

/// What the log says about the shell `pid`: the versions of what we hook, the taskbar software around it, and every
/// third-party module in it, which also covers what `KNOWN` doesn't.
pub fn shell_report(pid: u32) -> Vec<String> {
    let windows = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let version = |path: String| {
        let file = path.rsplit('\\').next().unwrap_or("").to_string();
        file_version(Path::new(&path)).map(|v| format!("{file} {v}"))
    };
    let hooked: Vec<String> = [
        format!(r"{windows}\explorer.exe"),
        format!(r"{windows}\System32\Taskbar.dll"),
        format!(r"{windows}\SystemApps\MicrosoftWindows.Client.Core_cw5n1h2txyewy\Taskbar.View.dll"),
        format!(r"{windows}\System32\Windows.UI.Xaml.dll"),
    ]
    .into_iter()
    .filter_map(version)
    .collect();
    let loaded = modules(pid);
    let known = spot(&loaded, &processes());
    let others: Vec<String> = known.iter().map(|k| k.name.to_string()).collect();
    let mut outside: Vec<String> = loaded.iter().filter(|m| third_party(m, &windows)).filter_map(|m| version(m.clone()).or_else(|| m.rsplit('\\').next().map(str::to_string))).collect();
    outside.sort();
    outside.dedup();
    vec![
        format!("shell (pid {pid}): {}", hooked.join(", ")),
        format!("taskbar software: {}", if others.is_empty() { "none we know of".to_string() } else { others.join(", ") }),
        format!("third-party modules in the shell: {}", if outside.is_empty() { "none".to_string() } else { outside.join(", ") }),
    ]
}

/// A log line without what a notification said: lines from before 0.1.4 had its title, and the link it opened.
fn without_notification(line: &str) -> String {
    if let Some(at) = line.find("toast from ").filter(|_| !line.contains("showing toast from"))
        && let Some(colon) = line[at..].find(": ")
    {
        return line[..at + colon].to_string();
    }
    match line.find("toast click: opened ") {
        Some(at) => format!("{}toast click: opened a link", &line[..at]),
        None => line.to_string(),
    }
}

/// Saves what a developer would need to see into the logs folder, and returns the file. Every line goes through the
/// logs' redaction, and loses what any notification said.
pub fn write_report(config: &Path) -> Result<std::path::PathBuf, String> {
    use pms_tap::logfile::{self, Log};
    let tail = |path: std::path::PathBuf, lines: usize| {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let all: Vec<&str> = text.lines().collect();
        all[all.len().saturating_sub(lines)..].join("\n")
    };
    let mut report = vec![format!("PimpMyShell {} on {}", crate::version::CURRENT, windows_version())];
    report.extend(shell_pid().into_iter().flat_map(shell_report));
    for (title, body) in [
        ("Settings (pms.toml)", std::fs::read_to_string(config).unwrap_or_default()),
        ("daemon.log", tail(Log::Daemon.path(), 400)),
        ("tap.log", tail(Log::Tap.path(), 200)),
        ("The taskbar's layout (tree.txt)", std::fs::read_to_string(pms_tap::data_dir().join("tree.txt")).unwrap_or_default()),
    ] {
        report.push(format!("\n===== {title} =====\n{body}"));
    }
    let text: Vec<String> = report.join("\n").lines().map(|l| logfile::redact(&without_notification(l))).collect();
    let dir = logfile::dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join("pms-report.txt");
    std::fs::write(&file, text.join("\n")).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(modules: &[&str], processes: &[&str]) -> Vec<&'static str> {
        let owned = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        spot(&owned(modules), &owned(processes)).iter().map(|k| k.name).collect()
    }

    #[test]
    fn spots_taskbar_software() {
        assert_eq!(names(&[r"C:\Program Files\StartAllBack\StartAllBackX64.dll"], &[]), ["StartAllBack"]);
        assert_eq!(names(&[r"C:\Program Files (x86)\Stardock\Start11\Start10_64.dll"], &[]), ["Start11"]);
        assert_eq!(names(&[], &["RetroBar.exe", "windhawk.exe"]), ["RetroBar", "Windhawk"]);
    }

    #[test]
    fn explorerpatcher_is_the_dxgi_outside_system32() {
        assert_eq!(names(&[r"C:\WINDOWS\dxgi.dll"], &[]), ["ExplorerPatcher"]);
        assert!(names(&[r"C:\Windows\System32\dxgi.dll"], &[]).is_empty());
    }

    #[test]
    fn third_party_leaves_out_windows_and_us() {
        let is = |p: &str| third_party(p, r"C:\Windows");
        assert!(is(r"C:\Program Files\Foo\shell.dll") && is(r"C:\Windows\dxgi.dll"));
        assert!(!is(r"C:\WINDOWS\System32\ntdll.dll") && !is(r"C:\Windows\explorer.exe"));
        assert!(!is(r"C:\Windows\ShellExperiences\TileControl.dll") && !is(r"C:\Windows\ShellComponents\WindowsInternal.ShellUI.dll"));
        assert!(!is(r"C:\Users\x\AppData\Local\PimpMyShell\pms_tap-1.dll"));
        assert!(!is(r"C:\Program Files\Windows Photo Viewer\PhotoViewer.dll"));
        assert!(!is(r"C:\Program Files\WindowsApps\Microsoft.VCLibs.140\vcruntime140.dll"));
    }

    #[test]
    fn a_report_leaves_out_what_notifications_said() {
        assert_eq!(without_notification("17:27:58 toast from Telegram: Alice sent you a photo"), "17:27:58 toast from Telegram");
        assert_eq!(without_notification("12:00:00 toast click: opened tg://resolve?domain=alice"), "12:00:00 toast click: opened a link");
        assert_eq!(without_notification("12:00:00 toast from Telegram"), "12:00:00 toast from Telegram");
        // An error is what a report is for.
        let error = "12:00:00 showing toast from Telegram: error: no Grid";
        assert_eq!(without_notification(error), error);
        assert_eq!(without_notification("12:00:00 applied config to tap=1"), "12:00:00 applied config to tap=1");
    }

    #[test]
    fn reads_this_machine() {
        assert!(windows_version().contains("build"));
        let windows = std::env::var("WINDIR").unwrap();
        assert!(file_version(&Path::new(&windows).join("explorer.exe")).is_some_and(|v| v.split('.').count() == 4));
    }
}
