//! Sends every kind of toast Windows supports, from a registered test app with a COM activator,
//! so in-taskbar toasts can be checked end to end (icon, images, buttons, click-to-activate).
//!
//!   toast_lab register          HKCU: app id "PimpMyShell.ToastLab" (name, icon, activator) + COM server
//!   toast_lab unregister
//!   toast_lab <kind>..          plain long attribution logo circle hero inline buttons input
//!                               legacy header progress reminder alarm call urgent priority
//!                               silent replace expiring scheduled | all
//!   toast_lab <shell kind>..    what Explorer itself turns into toasts: tray balloons
//!                               (Shell_NotifyIcon NIF_INFO) balloon balloon-app balloon-icon
//!                               balloon-realtime balloon-quiet, and usernotification
//!                               (IUserNotification) | shell
//!   toast_lab everything        all, then shell
//!   toast_lab --gap 4 all       a pause between kinds (seconds), to watch each one; without it they
//!                               all arrive at once, stacking up
//!
//! Activations (clicks, buttons) start this exe as the COM server, which appends them to
//! %TEMP%\pms-toast-lab\activations.log.

use std::path::PathBuf;
use std::time::Duration;

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::{DateTime, IReference, PropertyValue};
use windows::UI::Notifications::{ScheduledToastNotification, ToastNotification, ToastNotificationManager, ToastNotificationPriority};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Notifications::{INotificationActivationCallback, INotificationActivationCallback_Impl, NOTIFICATION_USER_INPUT_DATA};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::{GUID, HSTRING, IUnknown, Interface, PCWSTR, Ref, implement, w};

/// PMS_LAB_AUMID picks another app id (a "new app" whose first toast Windows has never seen).
fn aumid() -> String {
    std::env::var("PMS_LAB_AUMID").unwrap_or_else(|_| "PimpMyShell.ToastLab".into())
}
const CLSID: GUID = GUID::from_u128(0x5ea5_4e11_70a5_4ab0_9c1e_0d1a_b5ea_1ab5);
const KINDS: &[&str] = &[
    "plain", "long", "attribution", "logo", "circle", "hero", "inline", "buttons", "input", "legacy", "header", "progress", "reminder", "alarm",
    "call", "urgent", "priority", "silent", "replace", "expiring", "scheduled",
];
const SHELL_KINDS: &[&str] = &["balloon", "balloon-app", "balloon-icon", "balloon-realtime", "balloon-quiet", "usernotification"];

fn dir() -> PathBuf {
    let d = std::env::temp_dir().join("pms-toast-lab");
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A soft diagonal gradient, so images are recognisable at a glance.
fn png(name: &str, w: u32, h: u32, from: [u8; 3], to: [u8; 3]) -> String {
    let path = dir().join(name);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let t = (x as f32 / w as f32 + y as f32 / h as f32) / 2.0;
            for c in 0..3 {
                px.push((from[c] as f32 * (1.0 - t) + to[c] as f32 * t) as u8);
            }
            px.push(255);
        }
    }
    let mut out = Vec::new();
    let mut enc = ::png::Encoder::new(&mut out, w, h);
    enc.set_color(::png::ColorType::Rgba);
    enc.set_depth(::png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&px)).unwrap();
    std::fs::write(&path, out).unwrap();
    format!("file:///{}", path.display().to_string().replace('\\', "/"))
}

fn set(key: &str, name: Option<&str>, value: &str) {
    let data: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let name = name.map(HSTRING::from);
    let err = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
            REG_SZ.0,
            Some(data.as_ptr() as _),
            (data.len() * 2) as u32,
        )
    };
    assert_eq!(err, ERROR_SUCCESS, "writing {key}");
}

fn register() {
    let aumid = aumid();
    let icon = png("icon.png", 128, 128, [0x26, 0xC6, 0xDA], [0x7E, 0x57, 0xC2]);
    let icon = icon.trim_start_matches("file:///").replace('/', "\\");
    let app = format!(r"Software\Classes\AppUserModelId\{aumid}");
    set(&app, Some("DisplayName"), "PimpMyShell Toast Lab");
    set(&app, Some("IconUri"), &icon);
    set(&app, Some("CustomActivator"), &format!("{{{CLSID:?}}}"));
    let exe = std::env::current_exe().unwrap();
    set(&format!(r"Software\Classes\CLSID\{{{CLSID:?}}}\LocalServer32"), None, &format!("\"{}\" -ToastActivated", exe.display()));
    println!("registered {aumid} (activator {{{CLSID:?}}} -> {})", exe.display());
}

fn unregister() {
    let aumid = aumid();
    for key in [format!(r"Software\Classes\AppUserModelId\{aumid}"), format!(r"Software\Classes\CLSID\{{{CLSID:?}}}")] {
        let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(&key)) };
    }
    println!("unregistered {aumid}");
}

fn payload(kind: &str) -> String {
    let logo = || png("logo.png", 256, 256, [0xFF, 0x8A, 0x65], [0xBA, 0x68, 0xC8]);
    let hero = || png("hero.png", 728, 360, [0x1E, 0x88, 0xE5], [0x43, 0xA0, 0x47]);
    let inline = || png("inline.png", 360, 180, [0xFF, 0xD5, 0x4F], [0xF0, 0x62, 0x92]);
    let generic = |body: &str| format!(r#"<toast launch="kind={kind}"><visual><binding template="ToastGeneric">{body}</binding></visual></toast>"#);
    match kind {
        "plain" => generic("<text>Plain toast</text><text>Title and one line of body text.</text>"),
        "long" => generic(
            "<text>A rather long title that will certainly not fit in the taskbar as it is</text>\
             <text>First body line, also long enough to need trimming somewhere along the way.</text>\
             <text>Second body line.</text>",
        ),
        "attribution" => generic(r#"<text>With attribution</text><text>Body text.</text><text placement="attribution">via toast_lab</text>"#),
        "logo" => generic(&format!(r#"<text>App logo override</text><text>Square image instead of the app icon.</text><image placement="appLogoOverride" src="{}"/>"#, logo())),
        "circle" => generic(&format!(
            r#"<text>Circle-cropped logo</text><text>Like a contact picture.</text><image placement="appLogoOverride" hint-crop="circle" src="{}"/>"#,
            logo()
        )),
        "hero" => generic(&format!(r#"<text>Hero image</text><text>Big banner above the text.</text><image placement="hero" src="{}"/>"#, hero())),
        "inline" => generic(&format!(r#"<text>Inline image</text><text>Image below the text.</text><image src="{}"/>"#, inline())),
        "buttons" => String::from(
            r#"<toast launch="kind=buttons"><visual><binding template="ToastGeneric"><text>Buttons</text><text>Two app buttons, one link, one dismiss.</text></binding></visual>
               <actions>
                 <action content="Accept" arguments="action=accept"/>
                 <action content="Later" arguments="action=later" activationType="background"/>
                 <action content="Open docs" arguments="https://learn.microsoft.com/windows/apps/design/shell/tiles-and-notifications/adaptive-interactive-toasts" activationType="protocol"/>
                 <action content="Dismiss" arguments="dismiss" activationType="system"/>
                 <action content="Context menu item" arguments="action=menu" placement="contextmenu"/>
               </actions></toast>"#
        ),
        "input" => r#"<toast launch="kind=input"><visual><binding template="ToastGeneric"><text>Quick reply</text><text>A text box and a reply button.</text></binding></visual>
               <actions><input id="reply" type="text" placeHolderContent="Type a reply"/><action content="Send" arguments="action=send" hint-inputId="reply"/><action content="Like" arguments="action=like"/></actions></toast>"#
            .into(),
        "legacy" => format!(
            r#"<toast launch="kind=legacy"><visual><binding template="ToastImageAndText04"><image id="1" src="{}"/><text id="1">Legacy template</text><text id="2">ToastImageAndText04, like Firefox web push.</text><text id="3">Third line</text></binding></visual></toast>"#,
            logo()
        ),
        "header" => r#"<toast launch="kind=header"><header id="lab" title="Lab conversation" arguments="header=lab"/><visual><binding template="ToastGeneric"><text>Grouped under a header</text><text>Headers group toasts in the center.</text></binding></visual></toast>"#.into(),
        "progress" => generic(r#"<text>Downloading</text><progress title="toast_lab.zip" value="0.6" valueStringOverride="60 of 100 MB" status="Downloading..."/>"#),
        "reminder" => r#"<toast scenario="reminder" launch="kind=reminder"><visual><binding template="ToastGeneric"><text>Reminder scenario</text><text>Stays until dismissed.</text></binding></visual><actions><action content="Dismiss" arguments="dismiss" activationType="system"/></actions></toast>"#.into(),
        "alarm" => r#"<toast scenario="alarm" launch="kind=alarm"><visual><binding template="ToastGeneric"><text>Alarm scenario</text><text>Loops its sound, stays until dismissed.</text></binding></visual><actions><action content="Dismiss" arguments="dismiss" activationType="system"/></actions></toast>"#.into(),
        "call" => r#"<toast scenario="incomingCall" launch="kind=call"><visual><binding template="ToastGeneric"><text>Incoming call</text><text>toast_lab is calling.</text></binding></visual><actions><action content="Answer" arguments="action=answer"/><action content="Decline" arguments="action=decline"/></actions></toast>"#.into(),
        "urgent" => r#"<toast scenario="urgent" launch="kind=urgent"><visual><binding template="ToastGeneric"><text>Urgent scenario</text><text>Breaks through do not disturb if the app may.</text></binding></visual></toast>"#.into(),
        "priority" => generic("<text>High priority</text><text>ToastNotification.Priority = High.</text>"),
        "silent" => generic("<text>Silent toast</text><text>SuppressPopup: straight to the notification center, no banner.</text>"),
        "replace" => generic("<text>Replace me</text><text>Tag + group; a second toast replaces this one in 3 s.</text>"),
        "replaced" => generic("<text>Replaced</text><text>Same tag and group: this took the first one's place.</text>"),
        "expiring" => generic("<text>Expiring toast</text><text>ExpirationTime 4 s from now: gone from the center by then.</text>"),
        "scheduled" => generic("<text>Scheduled toast</text><text>ScheduledToastNotification, delivered 5 s after it was scheduled.</text>"),
        other => panic!("unknown kind `{other}`; one of {KINDS:?}, {SHELL_KINDS:?}, all, shell or everything"),
    }
}

fn xml(kind: &str) -> windows_core::Result<XmlDocument> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(payload(kind)))?;
    Ok(xml)
}

/// Now + `secs`, as a WinRT DateTime (100 ns ticks since 1601).
fn in_secs(secs: i64) -> DateTime {
    let ft = unsafe { windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime() };
    DateTime { UniversalTime: (((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64) + secs * 10_000_000 }
}

fn send(kind: &str) -> windows_core::Result<()> {
    if SHELL_KINDS.contains(&kind) {
        return shell(kind);
    }
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(aumid()))?;
    if kind == "scheduled" {
        notifier.AddToSchedule(&ScheduledToastNotification::CreateScheduledToastNotification(&xml(kind)?, in_secs(5))?)?;
        println!("scheduled {kind} for 5 s from now");
        return Ok(());
    }
    let toast = ToastNotification::CreateToastNotification(&xml(kind)?)?;
    let tagged = |t: &ToastNotification| t.SetTag(&HSTRING::from("lab")).and_then(|()| t.SetGroup(&HSTRING::from("lab")));
    match kind {
        "silent" => toast.SetSuppressPopup(true)?,
        "priority" => toast.SetPriority(ToastNotificationPriority::High)?,
        "expiring" => toast.SetExpirationTime(&PropertyValue::CreateDateTime(in_secs(4))?.cast::<IReference<DateTime>>()?)?,
        "replace" => tagged(&toast)?,
        _ => {}
    }
    notifier.Show(&toast)?;
    println!("sent {kind}");
    if kind == "replace" {
        std::thread::sleep(Duration::from_secs(3));
        let second = ToastNotification::CreateToastNotification(&xml("replaced")?)?;
        tagged(&second)?;
        notifier.Show(&second)?;
        println!("sent its replacement");
    }
    Ok(())
}

fn wide<const N: usize>(s: &str) -> [u16; N] {
    let mut out = [0u16; N];
    for (o, c) in out.iter_mut().zip(s.encode_utf16().take(N - 1)) {
        *o = c;
    }
    out
}

const TRAY_CALLBACK: u32 = WM_APP + 1;

/// Reports what Explorer tells the tray icon about its balloon.
extern "system" fn balloon_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == TRAY_CALLBACK {
        let what = match lp.0 as u32 & 0xFFFF {
            NIN_BALLOONSHOW => Some("shown"),
            NIN_BALLOONHIDE => Some("hidden"),
            NIN_BALLOONTIMEOUT => Some("timed out / dismissed"),
            NIN_BALLOONUSERCLICK => Some("CLICKED"),
            _ => None,
        };
        if let Some(what) = what {
            println!("{} balloon {what}", now());
        }
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// The notifications Explorer itself turns into toasts on Windows 10/11.
fn shell(kind: &str) -> windows_core::Result<()> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if kind == "usernotification" {
            // The shell's balloon object: a tray icon of its own plus a balloon. Show blocks until the
            // balloon is clicked or times out.
            let n: IUserNotification = CoCreateInstance(&UserNotification, None, CLSCTX_INPROC_SERVER)?;
            n.SetBalloonInfo(w!("IUserNotification"), w!("The shell's balloon object: a tray icon with a balloon tip."), NIIF_INFO.0)?;
            n.SetBalloonRetry(10_000, 0, 0)?;
            n.SetIconInfo(LoadIconW(None, IDI_INFORMATION)?, w!("toast_lab"))?;
            println!("{} showing {kind} (blocks until clicked or timed out)", now());
            let r = n.Show(None, 0);
            println!("{} IUserNotification::Show -> {}", now(), if r.is_ok() { "clicked".to_string() } else { format!("{r:?}") });
            return Ok(());
        }
        if kind == "balloon-app" {
            // With an app id, Explorer files the balloon's toast under that app instead of under the exe.
            SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(aumid()))?;
        }
        let instance = GetModuleHandleW(None)?.into();
        let class = w!("PmsToastLabBalloon");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(balloon_proc), hInstance: instance, lpszClassName: class, ..Default::default() });
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("toast_lab"), WINDOW_STYLE(0), 0, 0, 0, 0, None, None, Some(instance), None)?;
        let mut d = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: TRAY_CALLBACK,
            hIcon: LoadIconW(None, IDI_INFORMATION)?,
            szTip: wide("toast_lab"),
            ..Default::default()
        };
        Shell_NotifyIconW(NIM_ADD, &d).ok()?;
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &d);
        d.uFlags = NIF_INFO;
        d.szInfoTitle = wide(&format!("Tray balloon ({kind})"));
        d.szInfo = wide("A tray icon's balloon tip: Shell_NotifyIcon with NIF_INFO.");
        d.dwInfoFlags = NIIF_INFO;
        match kind {
            "balloon-icon" => {
                d.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
                d.hBalloonIcon = LoadIconW(None, IDI_WARNING)?;
            }
            "balloon-realtime" => d.uFlags |= NIF_REALTIME,
            "balloon-quiet" => d.dwInfoFlags |= NIIF_RESPECT_QUIET_TIME,
            _ => {}
        }
        Shell_NotifyIconW(NIM_MODIFY, &d).ok()?;
        println!("{} sent {kind}; keeping its tray icon for 15 s", now());
        let until = std::time::Instant::now() + Duration::from_secs(15);
        let mut msg = MSG::default();
        while std::time::Instant::now() < until {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = Shell_NotifyIconW(NIM_DELETE, &d);
        let _ = DestroyWindow(hwnd);
    }
    Ok(())
}

#[implement(INotificationActivationCallback)]
struct Activator;

impl INotificationActivationCallback_Impl for Activator_Impl {
    fn Activate(&self, aumid: &PCWSTR, args: &PCWSTR, data: *const NOTIFICATION_USER_INPUT_DATA, count: u32) -> windows_core::Result<()> {
        let inputs: Vec<String> = (0..count as usize)
            .map(|i| unsafe {
                let d = &*data.add(i);
                format!("{}={}", d.Key.display(), d.Value.display())
            })
            .collect();
        let line = unsafe { format!("{} activated: aumid={} args={:?} inputs={inputs:?}\n", now(), aumid.display(), args.display().to_string()) };
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir().join("activations.log")).unwrap();
        let _ = f.write_all(line.as_bytes());
        Ok(())
    }
}

fn now() -> String {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, _outer: Ref<IUnknown>, riid: *const GUID, ppv: *mut *mut std::ffi::c_void) -> windows_core::Result<()> {
        let a: IUnknown = Activator.into();
        unsafe { a.query(riid, ppv).ok() }
    }
    fn LockServer(&self, _: windows_core::BOOL) -> windows_core::Result<()> {
        Ok(())
    }
}

/// Started by COM with -ToastActivated: serve the activator for a few seconds, then exit.
fn serve() -> windows_core::Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let factory: IClassFactory = Factory.into();
        let cookie = CoRegisterClassObject(&CLSID, &factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE)?;
        std::thread::sleep(Duration::from_secs(5));
        CoRevokeClassObject(cookie)?;
    }
    Ok(())
}

fn main() -> windows_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("-ToastActivated") | Some("-Embedding") => serve(),
        Some("register") => {
            register();
            Ok(())
        }
        Some("unregister") => {
            unregister();
            Ok(())
        }
        Some(_) => {
            // --gap <seconds>: a pause between kinds, to watch each one; without it they arrive together (a stack).
            let mut gap = None;
            let mut kinds = Vec::new();
            let mut it = args.iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--gap" => gap = it.next().and_then(|s| s.parse::<f64>().ok()).map(Duration::from_secs_f64),
                    "all" => kinds.extend(KINDS),
                    "shell" => kinds.extend(SHELL_KINDS),
                    "everything" => kinds.extend(KINDS.iter().chain(SHELL_KINDS)),
                    k => kinds.push(k),
                }
            }
            for (i, k) in kinds.iter().enumerate() {
                if i > 0 && let Some(g) = gap {
                    std::thread::sleep(g);
                }
                send(k)?;
            }
            Ok(())
        }
        None => {
            println!(
                "usage: toast_lab register|unregister | [--gap <seconds>] all|shell|everything|<kind>..\n  kinds: {KINDS:?}\n  shell kinds: {SHELL_KINDS:?}"
            );
            Ok(())
        }
    }
}
