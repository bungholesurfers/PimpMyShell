//! The daemon's notification-area icon, in the manner of Nagare's: the logo (the crab and its dollar), and a menu.
//!
//!   PimpMyShell X.Y.Z | Paused | Stopped: (why)
//!   Settings              (also a left click)
//!   Pause                 the standard taskbar until resumed
//!   Try again             in Pause's place, only after the watchdog gave up (watchdog.rs)
//!   Start when I sign in
//!   Check for updates     or, with one found, Install PimpMyShell X… (which asks first), or its page
//!   Open log
//!   Exit                  stops the daemon, which puts the standard taskbar back
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
    /// Turn the taskbar's toasts off, notifications staying Windows' (asked from a notice: notices.rs).
    KeepNativeToasts,
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

    /// Asks whether software update may check GitHub by itself (the first run's question: until it's answered,
    /// nothing is checked); the answer comes back as `Command::UpdateConsent`. As Nagare's card has it: OK, or
    /// closing the question, says yes; *No, don't check* says no. `latest`: the channel is the latest release's (the
    /// question shows the URL it reads first). On a thread of its own, so the daemon goes on while the question waits.
    pub fn ask_update_consent(&self, latest: bool) {
        let _ =
            std::thread::Builder::new().name("update question".into()).spawn(move || queue(Command::UpdateConsent(update_question(latest))));
    }
}

/// The question whether software update may check by itself, waiting for the answer: true unless it was no. Where
/// there's no task dialog, a Yes/No message box (which can't be closed without an answer).
pub fn update_question(latest: bool) -> bool {
    let url = crate::update::first_url(latest);
    // Closing it (its x, Esc) answers as OK does: so does a window closed without an answer (0).
    if let Some(pressed) = modern("Check for updates automatically?", &consent_text("the address below"), Some(url), &["Check automatically", "Don't check"]) {
        return pressed != 2;
    }
    let content = consent_text(&format!("<a href=\"{url}\">{url}</a>"));
    match task_dialog("Check for updates automatically?", &content, &[(NO_CHECKS, "Don't check")], true, IDOK.0, true) {
        Ok(pressed) => pressed != NO_CHECKS,
        Err(e) => {
            log(format!("tray: no task dialog ({e}); a message box instead"));
            message_box(&format!("{}\n\nCheck for updates by itself?", consent_text(url)), true)
        }
    }
}

/// A question as a WinUI 3 window, the settings' look (`pms-gui --ask`, beside this exe), where the PC has the
/// Windows App Runtime: the button pressed, 1 for the first, or 0 for the window closed without one. None where
/// there's no such window (no pms-gui.exe here, no runtime): ask another way.
fn modern(heading: &str, text: &str, link: Option<&str>, buttons: &[&str]) -> Option<u32> {
    let exe = std::env::current_exe().ok()?.with_file_name("pms-gui.exe");
    if !exe.is_file() {
        return None;
    }
    let mut ask = std::process::Command::new(exe);
    ask.args(["--ask", "--heading", heading, "--text", text]);
    if let Some(link) = link {
        ask.args(["--link", link]);
    }
    for b in buttons {
        ask.args(["--button", b]);
    }
    // Its window comes up in front: the daemon may give it the foreground (it can't take it itself).
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    let mut child = match ask.spawn() {
        Ok(child) => child,
        Err(e) => {
            log(format!("tray: pms-gui --ask: {e}"));
            return None;
        }
    };
    // Waiting, this thread goes on handling its windows' messages, as a message box's would: the tray's thread asks
    // too (Install), and its icon must stay alive meanwhile.
    use std::os::windows::io::AsRawHandle;
    let process = HANDLE(child.as_raw_handle());
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                log(format!("tray: pms-gui --ask: {e}"));
                return None;
            }
        }
        unsafe {
            MsgWaitForMultipleObjects(Some(&[process]), false, u32::MAX, QS_ALLINPUT);
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    };
    status.code().and_then(|c| u32::try_from(c).ok()).filter(|&c| c < 255)
}

/// What the question about software update says; `url` is the first request's (a link, in a task dialog).
fn consent_text(url: &str) -> String {
    format!(
        "PimpMyShell can check GitHub for new versions shortly after it starts, then every day or two. New versions \
         download in the background, and you choose when to install them. We only install versions signed with \
         PimpMyShell's release key.\n\n\
         Each check starts by reading {url}. We don't send any information about you or your PC.\n\n\
         We won't check until you choose. You can change this anytime in Settings > General."
    )
}

/// The question's button that says no.
const NO_CHECKS: i32 = 100;

/// A notice's explanation (notices.rs), with what can be done as buttons (the first is the default), waiting until
/// it's put away: the button pressed, by its place in `choices`, or None for Close. Where there's no task dialog, a
/// message box, which offers the first.
pub fn explain(instruction: &str, content: &str, choices: &[&str]) -> Option<usize> {
    let mut buttons = choices.to_vec();
    buttons.push("Close");
    if let Some(pressed) = modern(instruction, content, None, &buttons) {
        return (pressed as usize).checked_sub(1).filter(|&i| i < choices.len());
    }
    const FIRST: i32 = 101;
    let buttons: Vec<(i32, &str)> = choices.iter().enumerate().map(|(i, c)| (FIRST + i as i32, *c)).collect();
    let default = if choices.is_empty() { IDCLOSE.0 } else { FIRST };
    match task_dialog(instruction, content, &buttons, false, default, false) {
        Ok(pressed) => usize::try_from(pressed - FIRST).ok().filter(|&i| i < choices.len()),
        Err(e) => {
            log(format!("tray: no task dialog ({e}); a message box instead"));
            match choices.first() {
                Some(c) => message_box(&format!("{instruction}\n\n{content}\n\n{c}?"), true).then_some(0),
                None => {
                    message_box(&format!("{instruction}\n\n{content}"), false);
                    None
                }
            }
        }
    }
}

/// A message box in front of everything: Yes/No (true for yes), or just OK.
fn message_box(text: &str, ask: bool) -> bool {
    let style = if ask { MB_YESNO | MB_ICONQUESTION } else { MB_OK | MB_ICONINFORMATION };
    unsafe { MessageBoxW(None, &HSTRING::from(text), w!("PimpMyShell"), style | MB_SETFOREGROUND | MB_TOPMOST) == IDYES }
}

/// A task dialog under PimpMyShell's name and logo: `instruction` over `content`, `buttons` (id, label) beside the
/// common OK (`ok`; otherwise Close), `default` the one Enter presses; `links`: the content's `<a href="...">`s are
/// links, opened in the browser when clicked. It waits, and gives the id pressed (closing it with its x or Esc
/// presses the common button). Err where there's no task dialog (comctl32 before version 6, which the daemon's
/// manifest asks for; it's looked up rather than imported, so that an exe without that manifest, a test's, still
/// starts).
fn task_dialog(instruction: &str, content: &str, buttons: &[(i32, &str)], ok: bool, default: i32, links: bool) -> Result<i32, String> {
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
    use windows::Win32::UI::Controls::*;
    use windows_core::{BOOL, HRESULT};
    type TaskDialogIndirect = unsafe extern "system" fn(*const TASKDIALOGCONFIG, *mut i32, *mut i32, *mut BOOL) -> HRESULT;

    let comctl = unsafe { LoadLibraryW(w!("comctl32.dll")) }.map_err(|e| e.to_string())?;
    let found = unsafe { GetProcAddress(comctl, windows_core::s!("TaskDialogIndirect")) }.ok_or("comctl32 has no TaskDialogIndirect")?;
    let show: TaskDialogIndirect = unsafe { std::mem::transmute(found) };
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|e| e.to_string())?;
    // The logo, from the exe's own icon (resource number 1: the icon group pms-res writes); an exe without one (an
    // example's) goes without.
    let side = unsafe { GetSystemMetrics(SM_CXICON) }.max(32);
    let logo = unsafe { LoadImageW(Some(module.into()), PCWSTR(std::ptr::without_provenance(1)), IMAGE_ICON, side, side, LR_DEFAULTCOLOR) };
    let logo = logo.map_or(HICON::default(), |h| HICON(h.0));
    let (instruction, content) = (HSTRING::from(instruction), HSTRING::from(content));
    let labels: Vec<HSTRING> = buttons.iter().map(|(_, l)| HSTRING::from(*l)).collect();
    let buttons: Vec<TASKDIALOG_BUTTON> =
        buttons.iter().zip(&labels).map(|((id, _), l)| TASKDIALOG_BUTTON { nButtonID: *id, pszButtonText: PCWSTR(l.as_ptr()) }).collect();
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hInstance: module.into(),
        // Links only where asked for: a config error's line could look like one.
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION
            | if logo.is_invalid() { TASKDIALOG_FLAGS(0) } else { TDF_USE_HICON_MAIN }
            | if links { TDF_ENABLE_HYPERLINKS } else { TASKDIALOG_FLAGS(0) },
        dwCommonButtons: if ok { TDCBF_OK_BUTTON } else { TDCBF_CLOSE_BUTTON },
        pszWindowTitle: w!("PimpMyShell"),
        Anonymous1: TASKDIALOGCONFIG_0 { hMainIcon: logo },
        pszMainInstruction: PCWSTR(instruction.as_ptr()),
        pszContent: PCWSTR(content.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: if buttons.is_empty() { std::ptr::null() } else { buttons.as_ptr() },
        nDefaultButton: default,
        pfCallback: Some(on_top),
        ..Default::default()
    };
    let mut pressed = 0;
    let shown = unsafe { show(&config, &mut pressed, std::ptr::null_mut(), std::ptr::null_mut()) }.ok();
    if !logo.is_invalid() {
        let _ = unsafe { DestroyIcon(logo) };
    }
    shown.map_err(|e| e.to_string())?;
    Ok(pressed)
}

/// A task dialog comes up in front, as a message box with MB_TOPMOST and MB_SETFOREGROUND does; its links open in
/// the browser (only https ones: ours).
unsafe extern "system" fn on_top(
    hwnd: HWND,
    msg: windows::Win32::UI::Controls::TASKDIALOG_NOTIFICATIONS,
    _: WPARAM,
    lparam: LPARAM,
    _: isize,
) -> windows_core::HRESULT {
    use windows::Win32::UI::Controls::{TDN_CREATED, TDN_HYPERLINK_CLICKED};
    if msg == TDN_CREATED {
        unsafe {
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            let _ = SetForegroundWindow(hwnd);
        }
    } else if msg == TDN_HYPERLINK_CLICKED && lparam.0 != 0 {
        let href = unsafe { PCWSTR(lparam.0 as *const u16).to_string() }.unwrap_or_default();
        if href.starts_with("https://")
            && let Err(e) = crate::toasts::open(&href)
        {
            log(format!("tray: {e}"));
        }
    }
    windows_core::HRESULT(0)
}

/// Asks whether to install the downloaded `version` now: the menu's Install item.
fn confirm_install(hwnd: HWND, version: &str) -> bool {
    let text = format!(
        "You have PimpMyShell {}. Version {version} is downloaded and verified.\n\n\
         While it installs, your taskbar returns to normal for a moment. Then PimpMyShell starts again.",
        crate::version::CURRENT
    );
    if let Some(pressed) = modern(&format!("Install PimpMyShell {version}?"), &text, None, &["Install", "Not now"]) {
        return pressed == 1;
    }
    let text = format!("Install PimpMyShell {version}?\n\n{text}");
    let answer = unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), w!("PimpMyShell"), MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND) };
    answer == IDYES
}

/// The logo as the notifications show it (pms-art renders it).
const LOGO_128: &[u8] = include_bytes!("../../assets/icon/pms-128.png");

/// The app id PimpMyShell's notifications go out under, registered with its name and logo (the uninstaller removes it).
const AUMID: &str = "PimpMyShell";

/// The logo as a file (`pms-128.png` in %LOCALAPPDATA%\PimpMyShell), for notifications: Windows' and the taskbar's.
pub(crate) fn logo_file() -> Result<PathBuf, String> {
    let dir = pms_tap::data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let logo = dir.join("pms-128.png");
    if std::fs::read(&logo).ok().as_deref() != Some(LOGO_128) {
        std::fs::write(&logo, LOGO_128).map_err(|e| format!("{}: {e}", logo.display()))?;
    }
    Ok(logo)
}

fn toast(title: &str, text: &str) -> Result<(), String> {
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    let logo = logo_file()?.display().to_string();
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

pub(crate) fn queue(c: Command) {
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
        (Some(_), _) => "PimpMyShell (stopped)".into(),
        (None, true) => "PimpMyShell (paused)".into(),
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
        (Some(why), _) => format!("Stopped: {why}"),
        (None, true) => "Paused".into(),
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
        item(MF_STRING, ID_RETRY, "Try again");
    } else {
        item(MF_STRING | check(state.paused), ID_PAUSE, "Pause");
    }
    item(MF_STRING | check(autostart), ID_AUTOSTART, "Start when I sign in");
    match &state.update {
        Some(Offer { version, ready: true }) => item(MF_STRING, ID_UPDATE, &format!("Install PimpMyShell {version}…")),
        Some(Offer { version, ready: false }) => item(MF_STRING, ID_UPDATE, &format!("PimpMyShell {version} is available…")),
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
