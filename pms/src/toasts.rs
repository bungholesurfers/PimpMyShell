//! Where in-taskbar toasts come from, how the native banners get out of the way, and how a
//! click on our card reaches the app.
//!
//! Source: `UserNotificationListener` (works unpackaged once "let apps access your
//! notifications" allows it) tells us a toast arrived; its change event needs package identity,
//! so we poll. It only exposes text, so the full toast XML (images, buttons, launch arguments)
//! comes from the notification platform's own database, `wpndatabase.db`, by the same id.
//!
//! Banners: each app's `HKCU\...\Notifications\Settings\<AUMID>` takes `ShowBanner=0` live,
//! which keeps the notification (center, listener) but drops the popup. We set it per app as we
//! meet them, write down what we changed in `banners.txt` *before* changing it, and put it all
//! back on `restore` (daemon stop, `[toasts]` removed, or the next start after a crash). An
//! app's very first notification predates its registry key, so that one still pops natively.
//!
//! Activation mirrors what Windows does for a click: the app's COM activator
//! (`INotificationActivationCallback`, registered as `CustomActivator` or in a packaged app's
//! manifest) gets the launch/button arguments; protocol actions open their URI; apps without an
//! activator are launched, or failing that the notification center opens.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use windows::Data::Xml::Dom::{XmlDocument, XmlElement};
use windows::Foundation::Size;
use windows::Storage::Streams::DataReader;
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};
use windows::Win32::Foundation::{ERROR_SUCCESS, SIZE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{CLSCTX_LOCAL_SERVER, CLSIDFromString, CoCreateInstance};
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Notifications::INotificationActivationCallback;
use windows::Win32::UI::Shell::{
    AO_NONE, ApplicationActivationManager, IApplicationActivationManager, IShellItemImageFactory, QUNS_ACCEPTS_NOTIFICATIONS,
    SHCreateItemFromParsingName, SHLoadIndirectString, SHQueryUserNotificationState, SIIGBF_ICONONLY, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows_core::{GUID, HSTRING, PCWSTR, w};

use pms_tap::data_dir;

const SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings";

fn e(err: windows_core::Error) -> String {
    err.to_string()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Activation {
    /// Foreground or background: the app's activator gets the arguments.
    App,
    /// Arguments are a URI to open.
    Protocol,
    /// The system `dismiss` action: just remove the notification.
    Dismiss,
}

impl Activation {
    fn parse(activation_type: &str, args: &str) -> Option<Activation> {
        match activation_type {
            "protocol" => Some(Activation::Protocol),
            "system" if args == "dismiss" => Some(Activation::Dismiss),
            // snooze needs its selection box; we can't offer that.
            "system" => None,
            _ => Some(Activation::App),
        }
    }
}

pub struct Action {
    pub label: String,
    pub args: String,
    pub kind: Activation,
}

pub struct Incoming {
    pub id: u32,
    pub aumid: String,
    pub app: String,
    pub attribution: String,
    pub title: String,
    pub body: String,
    pub icon: Option<PathBuf>,
    pub logo: Option<PathBuf>,
    pub circle: bool,
    pub image: Option<PathBuf>,
    /// What clicking the card does.
    pub launch: (String, Activation),
    pub actions: Vec<Action>,
    /// 0..1, or negative for indeterminate.
    pub progress: Option<f64>,
    /// `duration="long"`, or a scenario Windows keeps on screen (reminder, alarm, call).
    pub long: bool,
    /// Packaged apps resolve ms-appx/ms-appdata URIs and activators through their package.
    package: Option<Package>,
}

#[derive(Clone)]
struct Package {
    family: String,
    installed: PathBuf,
}

impl Incoming {
    /// `toast show` arguments for the TAP.
    pub fn command(&self, ms: u32) -> String {
        let q = crate::quote;
        let path = |p: &Option<PathBuf>| p.as_ref().map_or(String::new(), |p| p.display().to_string());
        let mut fields = vec![
            format!("ms={ms}"),
            format!("app={}", self.app),
            format!("attribution={}", self.attribution),
            format!("title={}", self.title),
            format!("body={}", self.body),
            format!("icon={}", path(&self.icon)),
            format!("logo={}", path(&self.logo)),
            format!("image={}", path(&self.image)),
            format!("circle={}", self.circle as u8),
        ];
        fields.extend(self.progress.map(|p| format!("progress={p}")));
        fields.extend(self.actions.iter().map(|a| format!("b={}", a.label)));
        std::iter::once("toast show".to_string()).chain(fields.iter().map(|f| q(f))).collect::<Vec<_>>().join(" ")
    }
}

pub struct Listener {
    inner: UserNotificationListener,
    seen: HashSet<u32>,
    /// Per AUMID: display name and icon, looked up once.
    apps: HashMap<String, (String, Option<PathBuf>)>,
}

impl Listener {
    pub fn new() -> Result<Listener, String> {
        let inner = UserNotificationListener::Current().map_err(e)?;
        let mut access = inner.GetAccessStatus().map_err(e)?;
        if access == UserNotificationListenerAccessStatus::Unspecified {
            access = inner.RequestAccessAsync().and_then(|op| op.join()).map_err(e)?;
        }
        if access != UserNotificationListenerAccessStatus::Allowed {
            return Err("notification access denied (Settings > Privacy & security > Notifications)".into());
        }
        let mut l = Listener { inner, seen: HashSet::new(), apps: HashMap::new() };
        // Whatever is already in the notification center is old news.
        l.seen = l.current()?.iter().filter_map(|n| n.Id().ok()).collect();
        Ok(l)
    }

    fn current(&self) -> Result<Vec<UserNotification>, String> {
        let list = self.inner.GetNotificationsAsync(NotificationKinds::Toast).and_then(|op| op.join()).map_err(e)?;
        Ok(list.into_iter().collect())
    }

    /// Notifications that arrived since the last poll, oldest first.
    pub fn poll(&mut self) -> Result<Vec<Incoming>, String> {
        let mut now = self.current()?;
        now.sort_by_key(|n| n.Id().unwrap_or(0));
        let mut out = Vec::new();
        let mut ids = HashSet::new();
        for n in now {
            let Ok(id) = n.Id() else { continue };
            ids.insert(id);
            if !self.seen.contains(&id) {
                out.extend(self.describe(&n, id));
            }
        }
        self.seen = ids;
        Ok(out)
    }

    /// Take it out of the notification center, as Windows does after an activation.
    pub fn remove(&self, id: u32) {
        if let Err(err) = self.inner.RemoveNotification(id) {
            crate::log(format!("removing notification {id}: {err}"));
        }
    }

    fn describe(&mut self, n: &UserNotification, id: u32) -> Option<Incoming> {
        let info = n.AppInfo().ok()?;
        let aumid = info.AppUserModelId().ok()?.to_string();
        let package = info.Package().ok().and_then(|p| {
            Some(Package { family: p.Id().ok()?.FamilyName().ok()?.to_string(), installed: PathBuf::from(p.InstalledPath().ok()?.to_string()) })
        });
        let (app, icon) = match self.apps.get(&aumid) {
            Some(known) => known.clone(),
            None => {
                let known = identity(&aumid, info.DisplayInfo().ok());
                self.apps.insert(aumid.clone(), known.clone());
                known
            }
        };
        let mut t = Incoming {
            id,
            aumid,
            app,
            attribution: String::new(),
            title: String::new(),
            body: String::new(),
            icon,
            logo: None,
            circle: false,
            image: None,
            launch: (String::new(), Activation::App),
            actions: Vec::new(),
            progress: None,
            long: false,
            package,
        };
        if let Err(err) = payload(id).and_then(|(xml, data)| t.read_payload(&xml, &data)) {
            // Text-only fallback straight from the listener.
            crate::log(format!("toast {id} payload: {err}; showing its text only"));
            let texts: Vec<String> = n
                .Notification()
                .and_then(|t| t.Visual())
                .and_then(|v| v.GetBinding(&KnownNotificationBindings::ToastGeneric()?))
                .and_then(|b| b.GetTextElements())
                .map(|els| els.into_iter().filter_map(|t| t.Text().ok()).map(|t| t.to_string()).collect())
                .unwrap_or_default();
            t.set_texts(texts);
        }
        if t.title.is_empty() {
            t.title = t.app.clone();
        }
        Some(t)
    }
}

/// The toast's XML as the app sent it, and its data-binding values (progress bars bind theirs).
fn payload(id: u32) -> Result<(String, HashMap<String, String>), String> {
    let path = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join(r"Microsoft\Windows\Notifications\wpndatabase.db");
    let db = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = db.query_row("select Payload from Notification where Id = ?1", [id], |r| r.get(0)).map_err(|e| e.to_string())?;
    let mut data = HashMap::new();
    if let Ok(mut q) = db.prepare("select Key, Value from NotificationData where NotificationId = ?1")
        && let Ok(rows) = q.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
            data.extend(rows.flatten());
        }
    Ok((String::from_utf8_lossy(&bytes).into_owned(), data))
}

fn attr(el: &XmlElement, name: &str) -> String {
    el.GetAttribute(&HSTRING::from(name)).map(|s| s.to_string()).unwrap_or_default()
}

fn elements(root: &XmlElement, xpath: &str) -> Vec<XmlElement> {
    root.SelectNodes(&HSTRING::from(xpath))
        .map(|l| l.into_iter().filter_map(|n| windows_core::Interface::cast(&n).ok()).collect())
        .unwrap_or_default()
}

/// `%41` -> `A`, and forward slashes to backslashes: a file URI's path as a Windows path.
fn uri_path(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).replace('/', "\\")
}

impl Incoming {
    fn set_texts(&mut self, texts: Vec<String>) {
        let mut texts = texts.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        self.title = texts.next().unwrap_or_default();
        self.body = texts.collect::<Vec<_>>().join(" · ");
    }

    fn read_payload(&mut self, xml: &str, data: &HashMap<String, String>) -> Result<(), String> {
        let doc = XmlDocument::new().map_err(e)?;
        doc.LoadXml(&HSTRING::from(xml)).map_err(e)?;
        let toast = doc.DocumentElement().map_err(e)?;
        let launch = attr(&toast, "launch");
        let kind = Activation::parse(&attr(&toast, "activationType"), &launch).unwrap_or(Activation::App);
        self.launch = (launch, kind);
        self.long = attr(&toast, "duration") == "long" || ["reminder", "alarm", "incomingCall", "urgent"].contains(&attr(&toast, "scenario").as_str());
        if let Some(h) = elements(&toast, "header").first() {
            self.attribution = attr(h, "title");
        }

        let bindings = elements(&toast, "visual/binding");
        let binding = bindings.iter().find(|b| attr(b, "template") == "ToastGeneric").or(bindings.first()).ok_or("no <binding>")?;
        let legacy = attr(binding, "template") != "ToastGeneric";

        let mut texts = Vec::new();
        for t in elements(binding, ".//text") {
            let s = t.InnerText().map(|s| s.to_string()).unwrap_or_default();
            if attr(&t, "placement") == "attribution" {
                self.attribution = if self.attribution.is_empty() { s } else { format!("{} · {s}", self.attribution) };
            } else {
                texts.push(s);
            }
        }
        self.set_texts(texts);

        // <progress title value valueStringOverride status>, any of them possibly "{bound}".
        if let Some(p) = elements(binding, "progress").first() {
            let get = |name: &str| {
                let v = attr(p, name);
                match v.strip_prefix('{').and_then(|k| k.strip_suffix('}')) {
                    Some(key) => data.get(key).cloned().unwrap_or_default(),
                    None => v,
                }
            };
            let value = get("value");
            self.progress = Some(if value == "indeterminate" { -1.0 } else { value.parse::<f64>().unwrap_or(0.0).clamp(0.0, 1.0) });
            let amount = Some(get("valueStringOverride")).filter(|s| !s.is_empty()).or_else(|| {
                self.progress.filter(|v| *v >= 0.0).map(|v| format!("{:.0}%", v * 100.0))
            });
            let line: Vec<String> = [get("title"), amount.unwrap_or_default(), get("status")].into_iter().filter(|s| !s.is_empty()).collect();
            let line = line.join(" · ");
            self.body = if self.body.is_empty() { line } else { format!("{} · {line}", self.body) };
        }

        for img in elements(binding, ".//image") {
            let Some(path) = self.resolve(&attr(&img, "src")) else { continue };
            match attr(&img, "placement").as_str() {
                "appLogoOverride" => {
                    self.circle = attr(&img, "hint-crop") == "circle";
                    self.logo = Some(path);
                }
                // Legacy templates show their image where the logo goes.
                _ if legacy => self.logo = self.logo.take().or(Some(path)),
                "hero" => self.image = Some(path),
                _ => self.image = self.image.take().or(Some(path)),
            }
        }

        for a in elements(&toast, "actions/action") {
            // Context-menu entries aren't buttons; reply buttons need their (unsupported) text box.
            if attr(&a, "placement") == "contextmenu" || !attr(&a, "hint-inputId").is_empty() {
                continue;
            }
            let args = attr(&a, "arguments");
            let Some(kind) = Activation::parse(&attr(&a, "activationType"), &args) else { continue };
            let mut label = attr(&a, "content");
            if label.is_empty() && kind == Activation::Dismiss {
                label = "Dismiss".into();
            }
            self.actions.push(Action { label, args, kind });
        }
        Ok(())
    }

    /// An image src as a local file: file:///, plain paths, and a packaged app's ms-appx:/// and
    /// ms-appdata:///local/. Remote images are skipped.
    fn resolve(&self, src: &str) -> Option<PathBuf> {
        let lower = src.to_ascii_lowercase();
        let path = if lower.starts_with("file:///") {
            PathBuf::from(uri_path(&src[8..]))
        } else if lower.starts_with("ms-appx:///") {
            scaled(&self.package.as_ref()?.installed.join(uri_path(&src[11..])))?
        } else if lower.starts_with("ms-appdata:///local/") {
            let packages = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Packages");
            packages.join(&self.package.as_ref()?.family).join("LocalState").join(uri_path(&src[20..]))
        } else if lower.contains("://") {
            return None;
        } else {
            PathBuf::from(src)
        };
        path.is_file().then_some(path)
    }
}

/// Packaged assets are often stored per scale (`logo.scale-200.png` for `logo.png`); take the
/// exact file or the largest scale.
fn scaled(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    let (stem, ext) = (path.file_stem()?.to_string_lossy().into_owned(), path.extension()?.to_string_lossy().into_owned());
    let (prefix, suffix) = (format!("{stem}.scale-"), format!(".{ext}"));
    let mut best: Option<(u32, PathBuf)> = None;
    for entry in std::fs::read_dir(path.parent()?).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(scale) = name.strip_prefix(&prefix).and_then(|r| r.strip_suffix(&suffix)) else { continue };
        let scale: u32 = scale.parse().unwrap_or(0);
        if best.as_ref().is_none_or(|(s, _)| scale > *s) {
            best = Some((scale, entry.path()));
        }
    }
    best.map(|(_, p)| p)
}

/// `HKCU`, then `HKLM`, `Software\Classes\AppUserModelId\<AUMID>` value: where apps that aren't
/// in a package register their toast name, icon and activator.
fn aumid_value(aumid: &str, name: &str) -> Option<String> {
    let key = HSTRING::from(format!(r"Software\Classes\AppUserModelId\{aumid}"));
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut buf = [0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let err = unsafe { RegGetValueW(root, &key, &HSTRING::from(name), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut size)) };
        if err == ERROR_SUCCESS {
            return Some(String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]));
        }
    }
    None
}

/// Display name and icon, the way Windows picks them for a toast header.
fn identity(aumid: &str, display: Option<windows::ApplicationModel::AppDisplayInfo>) -> (String, Option<PathBuf>) {
    let registered = aumid_value(aumid, "DisplayName").map(|n| {
        if !n.starts_with('@') {
            return n;
        }
        // "@C:\...\x.dll,-5": a resource string.
        let mut buf = [0u16; 512];
        match unsafe { SHLoadIndirectString(&HSTRING::from(&n), &mut buf, None) } {
            Ok(()) => String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)]),
            Err(_) => n,
        }
    });
    let name = registered
        .filter(|n| !n.is_empty())
        .or_else(|| display.as_ref().and_then(|d| d.DisplayName().ok()).map(|s| s.to_string()))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| aumid.to_string());
    let icon = aumid_value(aumid, "IconUri")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(|| save_logo(display.as_ref(), aumid).inspect_err(|err| crate::log(format!("icon for {name}: {err}"))).ok());
    (name, icon)
}

/// The app's icon as an image file the TAP can load (the TAP reads files, not WinRT streams).
/// The shell's AppsFolder knows every app with a Start entry, packaged or not; the package
/// logo is the fallback.
fn save_logo(display: Option<&windows::ApplicationModel::AppDisplayInfo>, aumid: &str) -> Result<PathBuf, String> {
    let bytes = match shell_icon(aumid) {
        Ok(png) => png,
        Err(shell_err) => {
            let display = display.ok_or(shell_err)?;
            let stream = display.GetLogo(Size { Width: 64.0, Height: 64.0 }).and_then(|r| r.OpenReadAsync()).and_then(|op| op.join()).map_err(e)?;
            let size = stream.Size().map_err(e)? as u32;
            let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).map_err(e)?).map_err(e)?;
            reader.LoadAsync(size).and_then(|op| op.join()).map_err(e)?;
            let mut bytes = vec![0u8; size as usize];
            reader.ReadBytes(&mut bytes).map_err(e)?;
            bytes
        }
    };
    let dir = data_dir().join("icons");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name: String = aumid.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    let path = dir.join(format!("{name}.img"));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

// ---------------------------------------------------------------------------------------------
// Activation

/// The app's `INotificationActivationCallback` CLSID, if it has one.
fn activator(t: &Incoming) -> Option<GUID> {
    let text = aumid_value(&t.aumid, "CustomActivator").or_else(|| {
        // Packaged desktop apps declare it in their manifest instead.
        let manifest = std::fs::read_to_string(t.package.as_ref()?.installed.join("AppxManifest.xml")).ok()?;
        let key = "ToastActivatorCLSID=\"";
        let at = manifest.find(key)? + key.len();
        Some(format!("{{{}}}", manifest[at..].split('"').next()?.trim_matches(['{', '}'])))
    })?;
    unsafe { CLSIDFromString(&HSTRING::from(text)) }.ok()
}

fn open(uri: &str) -> Result<(), String> {
    let r = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(uri), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if r.0 as usize > 32 { Ok(()) } else { Err(format!("ShellExecute({uri}) failed ({})", r.0 as usize)) }
}

/// Do what clicking `args` on the real toast would. Returns how it went, for the log.
pub fn activate(t: &Incoming, args: &str, kind: Activation) -> Result<String, String> {
    match kind {
        Activation::Dismiss => Ok("dismissed".into()),
        Activation::Protocol => open(args).map(|()| format!("opened {args}")),
        Activation::App => {
            if let Some(clsid) = activator(t) {
                let cb: INotificationActivationCallback = unsafe { CoCreateInstance(&clsid, None, CLSCTX_LOCAL_SERVER) }.map_err(e)?;
                unsafe { cb.Activate(&HSTRING::from(&t.aumid), &HSTRING::from(args), &[]) }.map_err(e)?;
                return Ok(format!("activated {} via its activator", t.app));
            }
            if t.package.is_some() {
                let mgr: IApplicationActivationManager =
                    unsafe { CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER) }.map_err(e)?;
                unsafe { mgr.ActivateApplication(&HSTRING::from(&t.aumid), &HSTRING::from(args), AO_NONE) }.map_err(e)?;
                return Ok(format!("launched {} with the toast's arguments", t.app));
            }
            // No activator: the app only hears about clicks while it runs. Show the real one.
            open("ms-actioncenter:").map(|()| format!("{} has no activator; opened the notification center", t.app))
        }
    }
}

/// `shell:AppsFolder\<AUMID>` rendered at 64px (crisp at 32 DIPs on high-DPI), as PNG.
fn shell_icon(aumid: &str) -> Result<Vec<u8>, String> {
    const PX: i32 = 64;
    let name = HSTRING::from(format!(r"shell:AppsFolder\{aumid}"));
    let factory: IShellItemImageFactory = unsafe { SHCreateItemFromParsingName(&name, None) }.map_err(e)?;
    let bitmap = unsafe { factory.GetImage(SIZE { cx: PX, cy: PX }, SIIGBF_ICONONLY) }.map_err(e)?;
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: PX,
            biHeight: -PX, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut px = vec![0u8; (PX * PX * 4) as usize];
    let rows = unsafe {
        let dc = GetDC(None);
        let rows = GetDIBits(dc, bitmap, 0, PX as u32, Some(px.as_mut_ptr() as _), &mut info, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        let _ = DeleteObject(bitmap.into());
        rows
    };
    if rows != PX {
        return Err("GetDIBits failed".into());
    }
    // Premultiplied BGRA -> straight RGBA.
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        let un = |c: u8| (c as u32 * 255 + a / 2).checked_div(a).map_or(0, |v| v.min(255) as u8);
        let (b, g, r) = (un(p[0]), un(p[1]), un(p[2]));
        p[0] = r;
        p[1] = g;
        p[2] = b;
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, PX as u32, PX as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(&px)).map_err(|e| e.to_string())?;
    Ok(out)
}

/// False while Windows itself holds notifications back (do not disturb, full-screen app,
/// presentation mode, ...).
pub fn windows_accepts_toasts() -> bool {
    unsafe { SHQueryUserNotificationState() }.is_ok_and(|s| s == QUNS_ACCEPTS_NOTIFICATIONS)
}

// ---------------------------------------------------------------------------------------------
// Native banner suppression

fn state_file() -> PathBuf {
    data_dir().join("banners.txt")
}

fn subkey(aumid: &str) -> HSTRING {
    HSTRING::from(format!(r"{SETTINGS}\{aumid}"))
}

fn show_banner(aumid: &str) -> Option<u32> {
    let mut v = 0u32;
    let mut size = 4u32;
    let err = unsafe { RegGetValueW(HKEY_CURRENT_USER, &subkey(aumid), w!("ShowBanner"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as _), Some(&mut size)) };
    (err == ERROR_SUCCESS).then_some(v)
}

/// Apps whose banners we turned off, with each one's original `ShowBanner` (None = unset).
pub struct Banners {
    managed: Vec<(String, Option<u32>)>,
}

impl Banners {
    /// Picks up what a previous run (possibly crashed) left suppressed.
    pub fn load() -> Banners {
        let text = std::fs::read_to_string(state_file()).unwrap_or_default();
        let managed = text
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .map(|(aumid, orig)| (aumid.to_string(), orig.parse().ok()))
            .collect();
        Banners { managed }
    }

    fn save(&self) {
        let text: String = self.managed.iter().map(|(a, o)| format!("{a}\t{}\n", o.map_or("-".into(), |v| v.to_string()))).collect();
        if let Err(err) = std::fs::write(state_file(), text) {
            crate::log(format!("writing {}: {err}", state_file().display()));
        }
    }

    pub fn is_managed(&self, aumid: &str) -> bool {
        self.managed.iter().any(|(a, _)| a == aumid)
    }

    /// The user turned this app's banners off themselves: no toast in the taskbar either.
    pub fn user_silenced(&self, aumid: &str) -> bool {
        !self.is_managed(aumid) && show_banner(aumid) == Some(0)
    }

    /// Turn this app's native banner off (no-op if we already did, or the user did).
    pub fn suppress(&mut self, aumid: &str) {
        if self.is_managed(aumid) {
            return;
        }
        let orig = show_banner(aumid);
        if orig == Some(0) {
            return;
        }
        self.managed.push((aumid.to_string(), orig));
        self.save();
        let zero = 0u32.to_le_bytes();
        let err = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &subkey(aumid), w!("ShowBanner"), REG_DWORD.0, Some(zero.as_ptr() as _), 4) };
        if err != ERROR_SUCCESS {
            crate::log(format!("suppressing banners for {aumid}: {}", err.to_hresult().message()));
        }
    }

    /// Every app that has notified before, i.e. has a settings key with notification history.
    pub fn suppress_known(&mut self, keep: impl Fn(&str) -> bool) {
        let mut apps = Vec::new();
        let mut root = HKEY::default();
        if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(SETTINGS), None, KEY_READ, &mut root) } == ERROR_SUCCESS {
            collect_apps(root, "", &mut apps);
            unsafe {
                let _ = RegCloseKey(root);
            }
        }
        for aumid in apps.iter().filter(|a| !keep(a)) {
            self.suppress(aumid);
        }
    }

    /// Put every banner setting we touched back the way we found it.
    pub fn restore(&mut self) {
        for (aumid, orig) in self.managed.drain(..) {
            let err = match orig {
                None => unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &subkey(&aumid), w!("ShowBanner")) },
                Some(v) => {
                    let b = v.to_le_bytes();
                    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &subkey(&aumid), w!("ShowBanner"), REG_DWORD.0, Some(b.as_ptr() as _), 4) }
                }
            };
            if err != ERROR_SUCCESS {
                crate::log(format!("restoring banners for {aumid}: {}", err.to_hresult().message()));
            }
        }
        let _ = std::fs::remove_file(state_file());
    }
}

/// AUMIDs may contain backslashes, so app keys nest; an app key is one with a notification history.
fn collect_apps(key: HKEY, prefix: &str, out: &mut Vec<String>) {
    let mut buf = [0u16; 512];
    for i in 0.. {
        let mut len = buf.len() as u32;
        if unsafe { RegEnumKeyExW(key, i, Some(windows_core::PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) } != ERROR_SUCCESS {
            break;
        }
        let name = String::from_utf16_lossy(&buf[..len as usize]);
        let path = if prefix.is_empty() { name.clone() } else { format!(r"{prefix}\{name}") };
        let mut sub = HKEY::default();
        if unsafe { RegOpenKeyExW(key, &HSTRING::from(&name), None, KEY_READ, &mut sub) } != ERROR_SUCCESS {
            continue;
        }
        let has_history = unsafe { RegGetValueW(sub, None, w!("LastNotificationAddedTime"), RRF_RT_ANY, None, None, None) } == ERROR_SUCCESS;
        if has_history {
            out.push(path.clone());
        }
        collect_apps(sub, &path, out);
        unsafe {
            let _ = RegCloseKey(sub);
        }
    }
}
