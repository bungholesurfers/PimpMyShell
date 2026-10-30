//! The daemon's notification-area icon, in the manner of Nagare's: the logo (the crab and its dollar), and a menu.
//!
//!   PimpMyShell - running | paused | not hooked (why)
//!   Settings             (also a left click)
//!   Pause                the stock taskbar until resumed
//!   Hook explorer again  only after the watchdog gave up (watchdog.rs)
//!   Start at sign-in
//!   Check for updates    or, with one found, Install PimpMyShell X… (which asks first), or its page
//!   Open log
//!   Exit                 stops the daemon, which puts the stock taskbar back
//!
//! It runs on a thread of its own with a hidden window (the shell talks to icons through one), and comes back when
//! explorer does: explorer broadcasts "TaskbarCreated" to every top-level window when it starts. Settings, the log
//! and sign-in are handled here; what the daemon must do (pause, retry, exit) is queued for it with an event set.
//!
//! The first time an exe shows its icon, it's put on the taskbar itself rather than in the overflow, as Settings >
//! Personalization > Taskbar > Other system tray icons would; an icon the user puts away later stays away.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{CreateEventW, SetEvent};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::{HSTRING, PCWSTR, w};

use crate::log;

/// What the daemon is asked to do.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Command {
    /// Pause (stock taskbar) or resume.
    TogglePause,
    /// Hook explorer again after the watchdog gave up.
    Retry,
    /// Check for a newer version now.
    CheckUpdate,
    /// Install the downloaded update: the user said yes to it.
    InstallUpdate,
    /// The answer to whether software update may check by itself.
    UpdateConsent(bool),
    Exit,
}

/// What the menu shows of the daemon.
#[derive(Clone, Default, PartialEq)]
pub struct State {
    pub paused: bool,
    /// Why the watchdog stopped hooking explorer.
    pub benched: Option<String>,
    /// A newer version: downloaded and checked (`ready`, so the menu offers to install it), or only out (this copy
    /// can't install it, so the menu opens its page).
    pub update: Option<Offer>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Offer {
    pub version: String,
    pub ready: bool,
}

struct Shared {
    hwnd: Mutex<isize>,
    /// Set when something is queued for the daemon.
    wake: isize,
    queued: Mutex<Vec<Command>>,
    state: Mutex<State>,
    balloon: Mutex<Option<(String, String)>>,
    config: PathBuf,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

const ICON_ID: u32 = 1;
const WM_TRAY: u32 = WM_APP + 1;
/// The daemon changed `state`: refresh the tooltip.
const WM_STATE: u32 = WM_APP + 2;
/// Show the queued balloon.
const WM_BALLOON: u32 = WM_APP + 3;
/// The keyboard's "select" on the icon (NIN_SELECT | NINF_KEY), which the bindings lack.
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const ID_SETTINGS: usize = 1;
const ID_PAUSE: usize = 2;
const ID_RETRY: usize = 3;
const ID_AUTOSTART: usize = 4;
const ID_LOG: usize = 5;
const ID_EXIT: usize = 6;
const ID_UPDATE: usize = 7;
const ID_CHECK: usize = 8;
/// Lists the exes whose first start has put the icon on the taskbar, one path a line.
const PINNED: &str = "tray-pinned.txt";

pub struct Tray;

impl Tray {
    /// Puts up the icon. `config` is what Settings and sign-in are given.
    pub fn start(config: &Path) -> Result<Tray, String> {
        let wake = unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.map_err(|e| e.to_string())?;
        let shared = Shared {
            hwnd: Mutex::new(0),
            wake: wake.0 as isize,
            queued: Mutex::default(),
            state: Mutex::default(),
            balloon: Mutex::default(),
            config: config.to_path_buf(),
        };
        SHARED.set(shared).map_err(|_| "the tray is up already")?;
        std::thread::Builder::new().name("tray".into()).spawn(run).map_err(|e| e.to_string())?;
        std::thread::Builder::new().name("tray pin".into()).spawn(pin).map_err(|e| e.to_string())?;
        Ok(Tray)
    }

    /// Set when the menu queued something: wait on it with the daemon's other events.
    pub fn event(&self) -> HANDLE {
        HANDLE(shared().wake as *mut _)
    }

    pub fn take(&self) -> Vec<Command> {
        std::mem::take(&mut *shared().queued.lock().unwrap())
    }

    pub fn set_state(&self, state: State) {
        let mut s = shared().state.lock().unwrap();
        if *s != state {
            *s = state;
            drop(s);
            post(WM_STATE);
        }
    }

    /// A notification in PimpMyShell's name, with its logo; or, where Windows won't take that, a balloon from the icon
    /// (which Windows names after the exe, "pmsd.exe"). From a thread with COM, like the daemon's.
    pub fn notify(&self, title: &str, text: &str) {
        if let Err(e) = toast(title, text) {
            log(format!("tray: no toast ({e}); a balloon instead"));
            *shared().balloon.lock().unwrap() = Some((title.into(), text.into()));
            post(WM_BALLOON);
        }
    }

    /// Asks, once, whether software update may check GitHub by itself; the answer comes back as
    /// `Command::UpdateConsent`. On a thread of its own, so the daemon goes on while the question waits.
    pub fn ask_update_consent(&self) {
        let _ = std::thread::Builder::new().name("update question".into()).spawn(|| {
            let text = "PimpMyShell can check GitHub for a newer version every day or two and download it. It always asks \
                        you before installing anything, and only installs what PimpMyShell's release key signed.\n\n\
                        Nothing about you or your PC is sent, only the requests themselves.\n\n\
                        Check for updates by itself? (You can change this in the settings, under software update.)";
            let answer = unsafe {
                MessageBoxW(None, &HSTRING::from(text), w!("PimpMyShell: software update"), MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND | MB_TOPMOST)
            };
            queue(Command::UpdateConsent(answer == IDYES));
        });
    }
}

/// Asks whether to install the downloaded `version` now: the menu's Install item.
fn confirm_install(hwnd: HWND, version: &str) -> bool {
    let text = format!(
        "Install PimpMyShell {version} now? (This is PimpMyShell {}.)\n\n\
         It was downloaded from GitHub, and its signature checks out. While it installs, the taskbar goes back to \
         stock for a moment; then PimpMyShell starts again by itself.",
        crate::version::CURRENT
    );
    let answer = unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("PimpMyShell: software update"), MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND) };
    answer == IDYES
}

/// The logo as the notifications show it (pms-art renders it).
const LOGO_128: &[u8] = include_bytes!("../../assets/icon/pms-128.png");

/// The app id PimpMyShell's notifications go out under, registered with its name and logo (the uninstaller removes it).
const AUMID: &str = "PimpMyShell";

fn toast(title: &str, text: &str) -> Result<(), String> {
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    let dir = pms_tap::data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let logo = dir.join("pms-128.png");
    std::fs::write(&logo, LOGO_128).map_err(|e| format!("{}: {e}", logo.display()))?;
    let logo = logo.display().to_string();
    let key = HSTRING::from(format!(r"Software\Classes\AppUserModelId\{AUMID}"));
    for (name, value) in [("DisplayName", "PimpMyShell"), ("IconUri", logo.as_str())] {
        let value: Vec<u16> = value.encode_utf16().chain([0]).collect();
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &key, &HSTRING::from(name), REG_SZ.0, Some(value.as_ptr().cast()), (value.len() * 2) as u32) }
            .ok()
            .map_err(|e| format!("registering the notifications' {name}: {e}"))?;
    }
    let escape = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let xml = format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text><image placement="appLogoOverride" src="{}"/></binding></visual></toast>"#,
        escape(title),
        escape(text),
        escape(&logo),
    );
    let show = || -> windows_core::Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml.as_str()))?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?.Show(&ToastNotification::CreateToastNotification(&doc)?)
    };
    show().map_err(|e| e.message())
}

fn shared() -> &'static Shared {
    SHARED.get().expect("tray started")
}

fn post(msg: u32) {
    let hwnd = *shared().hwnd.lock().unwrap();
    if hwnd != 0 {
        let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), msg, WPARAM(0), LPARAM(0)) };
    }
}

fn queue(c: Command) {
    let s = shared();
    s.queued.lock().unwrap().push(c);
    let _ = unsafe { SetEvent(HANDLE(s.wake as *mut _)) };
}

fn wide<const N: usize>(s: &str) -> [u16; N] {
    let mut out = [0u16; N];
    for (o, c) in out.iter_mut().zip(s.encode_utf16().take(N - 1)) {
        *o = c;
    }
    out
}

fn tooltip() -> String {
    let s = shared().state.lock().unwrap();
    match (&s.benched, s.paused) {
        (Some(_), _) => "PimpMyShell: not hooked into explorer".into(),
        (None, true) => "PimpMyShell: paused".into(),
        (None, false) => "PimpMyShell".into(),
    }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: ICON_ID, ..Default::default() }
}

fn add_icon(hwnd: HWND, icon: HICON) {
    let mut data = icon_data(hwnd);
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = icon;
    data.szTip = wide(&tooltip());
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
            log("tray: the shell didn't take the icon (no taskbar yet?)");
            return;
        }
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
    }
}

fn run() {
    let class = w!("PimpMyShellTray");
    let wc = WNDCLASSW { lpfnWndProc: Some(proc), lpszClassName: class, ..Default::default() };
    unsafe { RegisterClassW(&wc) };
    // A top-level window (not message-only), hidden: message-only windows don't get the TaskbarCreated broadcast.
    let hwnd = match unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("PimpMyShell"), WINDOW_STYLE(0), 0, 0, 0, 0, None, None, None, None) } {
        Ok(h) => h,
        Err(e) => return log(format!("tray: no window: {e}")),
    };
    *shared().hwnd.lock().unwrap() = hwnd.0 as isize;
    add_icon(hwnd, icon());
    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// The logo, at the small-icon size, from the exe's own icon (the build puts assets/icon/pms.ico in it).
fn icon() -> HICON {
    static ICON: OnceLock<isize> = OnceLock::new();
    HICON(*ICON.get_or_init(|| {
        let side = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
        let module = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(PCWSTR::null()) }.unwrap_or_default();
        // Resource number 1: the icon group pms-res writes.
        let handle = unsafe { LoadImageW(Some(module.into()), PCWSTR(std::ptr::without_provenance(1)), IMAGE_ICON, side, side, LR_DEFAULTCOLOR) };
        handle.map_or(0, |h| h.0 as isize)
    }) as *mut _)
}

extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
    let created = *TASKBAR_CREATED.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) });
    match msg {
        _ if msg == created && created != 0 => {
            // Explorer (re)started: its notification area knows nothing of us.
            add_icon(hwnd, icon());
            LRESULT(0)
        }
        WM_TRAY => {
            match (lparam.0 & 0xFFFF) as u32 {
                NIN_SELECT | NIN_KEYSELECT => open_settings(),
                WM_CONTEXTMENU => menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_STATE => {
            let mut data = icon_data(hwnd);
            data.uFlags = NIF_TIP | NIF_SHOWTIP;
            data.szTip = wide(&tooltip());
            let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
            LRESULT(0)
        }
        WM_BALLOON => {
            if let Some((title, text)) = shared().balloon.lock().unwrap().take() {
                let mut data = icon_data(hwnd);
                data.uFlags = NIF_INFO;
                data.szInfoTitle = wide(&title);
                data.szInfo = wide(&text);
                data.dwInfoFlags = NIIF_WARNING;
                let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn menu(hwnd: HWND) {
    let state = shared().state.lock().unwrap().clone();
    let autostart = crate::daemon::autostart(&["status".into()]).is_ok_and(|s| s.starts_with("at logon"));
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else { return };
    let status = match (&state.benched, state.paused) {
        (Some(why), _) => format!("Not hooked: {why}"),
        (None, true) => "Paused: the stock taskbar".into(),
        (None, false) => format!("PimpMyShell {}", crate::version::CURRENT),
    };
    let item = |flags: MENU_ITEM_FLAGS, id: usize, text: &str| {
        let text = HSTRING::from(text);
        let _ = unsafe { AppendMenuW(menu, flags, id, &text) };
    };
    let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
    item(MF_STRING | MF_GRAYED, 0, &status);
    item(MF_SEPARATOR, 0, "");
    item(MF_STRING, ID_SETTINGS, "Settings");
    if state.benched.is_some() {
        item(MF_STRING, ID_RETRY, "Hook explorer again");
    } else {
        item(MF_STRING | check(state.paused), ID_PAUSE, "Pause (stock taskbar)");
    }
    item(MF_STRING | check(autostart), ID_AUTOSTART, "Start at sign-in");
    match &state.update {
        Some(Offer { version, ready: true }) => item(MF_STRING, ID_UPDATE, &format!("Install PimpMyShell {version}…")),
        Some(Offer { version, ready: false }) => item(MF_STRING, ID_UPDATE, &format!("PimpMyShell {version} is out: open its page")),
        None => item(MF_STRING, ID_CHECK, "Check for updates"),
    }
    item(MF_STRING, ID_LOG, "Open log");
    item(MF_SEPARATOR, 0, "");
    item(MF_STRING, ID_EXIT, "Exit");
    let _ = unsafe { SetMenuDefaultItem(menu, ID_SETTINGS as u32, 0) };
    let mut at = POINT::default();
    let _ = unsafe { GetCursorPos(&mut at) };
    // The menu closes on a click elsewhere only if its window is in front.
    let _ = unsafe { SetForegroundWindow(hwnd) };
    let picked = unsafe { TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, at.x, at.y, Some(0), hwnd, None) }.0 as usize;
    let _ = unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) };
    let _ = unsafe { DestroyMenu(menu) };
    match picked {
        ID_SETTINGS => open_settings(),
        ID_PAUSE => queue(Command::TogglePause),
        ID_RETRY => queue(Command::Retry),
        ID_AUTOSTART => {
            let args = if autostart { vec!["off".to_string()] } else { vec!["on".to_string(), shared().config.display().to_string()] };
            match crate::daemon::autostart(&args) {
                Ok(m) => log(format!("tray: {m}")),
                Err(e) => log(format!("tray: start at sign-in: {e}")),
            }
        }
        ID_CHECK => queue(Command::CheckUpdate),
        ID_UPDATE => match state.update {
            Some(Offer { version, ready: true }) => {
                if confirm_install(hwnd, &version) {
                    queue(Command::InstallUpdate);
                }
            }
            Some(Offer { version, ready: false }) => spawn("explorer.exe", &[crate::update::release_page(&version).into()]),
            None => {}
        },
        ID_LOG => spawn("notepad.exe", &[pms_tap::logfile::Log::Daemon.path().into_os_string()]),
        ID_EXIT => queue(Command::Exit),
        _ => {}
    }
}

fn open_settings() {
    match std::env::current_exe().map(|e| e.with_file_name("pms-gui.exe")) {
        Ok(gui) => spawn(&gui.display().to_string(), &[shared().config.clone().into_os_string()]),
        Err(e) => log(format!("tray: {e}")),
    }
}

fn spawn(exe: &str, args: &[std::ffi::OsString]) {
    if let Err(e) = std::process::Command::new(exe).args(args).spawn() {
        log(format!("tray: couldn't start {exe}: {e}"));
    }
}

// ---------------------------------------------------------------------------------------------
// Pinning the icon on its first start (Nagare does the same): explorer keeps a key per icon under
// NotifyIconSettings, holding the exe's path, and shows those whose IsPromoted is 1. The key appears
// once explorer has seen the icon, so look for it for a while.

fn pin() {
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.display().to_string().to_lowercase();
    let list = pms_tap::data_dir().join(PINNED);
    let pinned = std::fs::read_to_string(&list).unwrap_or_default();
    if pinned.lines().any(|l| l == exe) {
        return;
    }
    for _ in 0..40 {
        std::thread::sleep(Duration::from_secs(3));
        match promote(&exe) {
            Some(Ok(())) => {
                let _ = std::fs::create_dir_all(pms_tap::data_dir());
                let _ = std::fs::write(&list, format!("{pinned}{exe}\n"));
                return log("tray: put the icon on the taskbar (first start of this copy)");
            }
            Some(Err(e)) => return log(format!("tray: couldn't put the icon on the taskbar: {e}")),
            None => {}
        }
    }
}

/// Promotes the icon whose key names `exe`, unless the user already chose. None while explorer has no key for it.
fn promote(exe: &str) -> Option<Result<(), String>> {
    let mut root = HKEY::default();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, w!(r"Control Panel\NotifyIconSettings"), Some(0), KEY_READ, &mut root) }.is_err() {
        return Some(Err("no NotifyIconSettings".into()));
    }
    let mut result = None;
    for i in 0.. {
        let mut name = [0u16; 64];
        let mut len = name.len() as u32;
        if unsafe { RegEnumKeyExW(root, i, Some(windows_core::PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) }.is_err() {
            break;
        }
        let sub = HSTRING::from_wide(&name[..len as usize]);
        let mut path = [0u16; 512];
        let mut size = (path.len() * 2) as u32;
        let read = unsafe { RegGetValueW(root, &sub, w!("ExecutablePath"), RRF_RT_REG_SZ, None, Some(path.as_mut_ptr().cast()), Some(&mut size)) };
        if read.is_err() {
            continue;
        }
        let path = String::from_utf16_lossy(&path[..(size as usize / 2).saturating_sub(1)]).to_lowercase();
        if path != exe {
            continue;
        }
        let mut promoted = 0u32;
        let mut dsize = 4u32;
        let chose = unsafe { RegGetValueW(root, &sub, w!("IsPromoted"), RRF_RT_REG_DWORD, None, Some((&mut promoted as *mut u32).cast()), Some(&mut dsize)) }.is_ok();
        result = Some(if chose {
            Ok(())
        } else {
            let one = 1u32.to_le_bytes();
            unsafe { RegSetKeyValueW(root, &sub, w!("IsPromoted"), REG_DWORD.0, Some(one.as_ptr().cast()), 4) }.ok().map_err(|e| e.to_string())
        });
        break;
    }
    let _ = unsafe { RegCloseKey(root) };
    result
}
