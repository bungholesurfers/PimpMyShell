//! `screen` meters' pictures: live ones, drawn by another process and handed over in shared memory (pms-screen). For each, a
//! thread of ours waits for frames and asks the UI thread to take the newest, once at a time, so a busy taskbar skips frames
//! rather than queueing them. All the UI thread does is copy a few KB into a bitmap the size of the cell's pixels.
//!
//! Live video can come another way, under that picture: a Twatch TV's player presents each frame to a DirectComposition
//! surface of its own, straight from its GPU, and the screen offers it here (`offer_surface`). Shown on a sprite, its frames
//! reach the screen with nothing done here for any of them: no copy, and no frame of XAML's for the whole taskbar, which each
//! new bitmap costs (with 7 sets playing on a 240 Hz display, ~300 M cycles a second of this thread). The picture over it is
//! then see-through but for what goes over the video, and changes only when that does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pms_screen::{Button, Screen, Status};
use pms_xaml::{Border, Canvas, ElementCompositionPreview, Image, ImageSource, SolidColorBrush, UIElement, WriteableBitmap};
use windows::UI::Composition::{CompositionStretch, CompositionSurfaceBrush, ICompositionSurface, SpriteVisual, Visual};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::WinRT::Composition::ICompositorInterop;
use windows::Win32::System::WinRT::IBufferByteAccess;
use windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;
use windows_core::Interface;
use windows_numerics::Vector2;

use crate::commands::R;

/// A screen being shown: what its thread and the UI thread share.
struct Feed {
    source: String,
    stop: AtomicBool,
    /// The picture's size in pixels, for the thread to ask for.
    size: [AtomicU32; 2],
    /// The UI thread has been asked for a frame and hasn't taken it yet.
    asked: AtomicBool,
    open: Mutex<Option<Arc<Screen>>>,
}

static FEEDS: Mutex<Vec<Arc<Feed>>> = Mutex::new(Vec::new());

struct Target {
    bitmap: WriteableBitmap,
    width: u32,
    height: u32,
    shown: u64,
    /// Under the picture: where live video straight from its player's GPU is hung, and the sprite it's on, while it is.
    host: Border,
    dips: (f64, f64),
    video: Option<SpriteVisual>,
}

thread_local! {
    static TARGETS: RefCell<HashMap<String, Target>> = RefCell::new(HashMap::new());
    /// Each screen's live video, straight from its player (`take_surface`): kept apart from its target, which a rebuilt strip
    /// makes again.
    static SURFACES: RefCell<HashMap<String, CompositionSurfaceBrush>> = RefCell::new(HashMap::new());
}

fn w(e: windows_core::Error) -> String {
    e.to_string()
}

fn feed(source: &str) -> Option<Arc<Feed>> {
    FEEDS.lock().ok()?.iter().find(|f| f.source == source).cloned()
}

fn screen(source: &str) -> Option<Arc<Screen>> {
    feed(source)?.open.lock().ok()?.clone()
}

/// A picture of `pixels` (width, height) filling `dips`, showing the screen called `source`: the screen's own picture, over a
/// place for live video to go straight from its player (`take_surface`).
pub(crate) fn picture(source: &str, pixels: (u32, u32), dips: (f64, f64)) -> R<Canvas> {
    let (width, height) = (pixels.0.clamp(1, pms_screen::MAX_SIDE), pixels.1.clamp(1, pms_screen::MAX_SIDE));
    let bitmap = WriteableBitmap::CreateInstanceWithDimensions(width as i32, height as i32).map_err(w)?;
    let image = Image::new().map_err(w)?;
    image.SetSource(&bitmap.cast::<ImageSource>().map_err(w)?).map_err(w)?;
    image.SetWidth(dips.0).map_err(w)?;
    image.SetHeight(dips.1).map_err(w)?;
    let host = Border::new().map_err(w)?;
    host.SetWidth(dips.0).map_err(w)?;
    host.SetHeight(dips.1).map_err(w)?;
    let both = crate::stage::canvas()?;
    both.SetWidth(dips.0).map_err(w)?;
    both.SetHeight(dips.1).map_err(w)?;
    let children = both.Children().map_err(w)?;
    children.Append(&host.cast::<UIElement>().map_err(w)?).map_err(w)?;
    children.Append(&image.cast::<UIElement>().map_err(w)?).map_err(w)?;
    TARGETS.with_borrow_mut(|t| t.insert(source.to_string(), Target { bitmap, width, height, shown: 0, host, dips, video: None }));
    // The video it was showing goes on showing, in the new one.
    if let Err(e) = hang(source) {
        crate::log(format!("screen `{source}`: its video: {e}"));
    }
    let f = feed(source).unwrap_or_else(|| start(source));
    f.size[0].store(width, Ordering::Relaxed);
    f.size[1].store(height, Ordering::Relaxed);
    // A new picture starts empty, and a channel that's holding still won't send it anything. If the feed isn't open yet, its
    // thread does this once it is.
    paint(&f);
    Ok(both)
}

/// The screen called `source` offers live video straight from its player (`offer_surface`, number `n`): the surface `handle`
/// in process `from` (0, none now). Shown, or let go, and said which. On the UI thread.
fn take_surface(source: &str, screen: &Screen, n: u64, from: u32, handle: u64) {
    if handle == 0 {
        let_go(source);
        return;
    }
    // A screen with nowhere to show it is one whose strip is going: the next one's thread looks again.
    let Some(host) = TARGETS.with_borrow(|t| t.get(source).map(|t| t.host.clone())) else { return };
    let shown = surface_of(&host, from, handle).and_then(|surface| {
        let compositor = crate::meters::compositor(&host)?;
        let brush = compositor.CreateSurfaceBrushWithSurface(&surface).map_err(w)?;
        brush.SetStretch(CompositionStretch::Fill).map_err(w)?;
        SURFACES.with_borrow_mut(|s| s.insert(source.to_string(), brush));
        hang(source)
    });
    screen.surface_taken(n, shown.is_ok());
    match shown {
        Ok(()) => crate::log(format!("screen `{source}`: its video straight from its player")),
        Err(e) => {
            let_go(source);
            crate::log(format!("screen `{source}`: its player's video couldn't be shown straight: {e}"));
        }
    }
}

/// The surface `handle` of process `from`, as a composition surface here: only ever a player's (pms-twatch.exe), and only
/// ever a DirectComposition surface.
fn surface_of(host: &Border, from: u32, handle: u64) -> R<ICompositionSurface> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut core::ffi::c_void;
        fn GetCurrentProcess() -> *mut core::ffi::c_void;
        fn DuplicateHandle(from: *mut core::ffi::c_void, handle: *mut core::ffi::c_void, to: *mut core::ffi::c_void, copy: *mut *mut core::ffi::c_void, access: u32, inherit: i32, options: u32) -> i32;
        fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
    }
    const DUP_HANDLE: u32 = 0x0040;
    const QUERY_LIMITED: u32 = 0x1000;
    const SAME_ACCESS: u32 = 0x2;
    unsafe {
        let process = OpenProcess(DUP_HANDLE | QUERY_LIMITED, 0, from);
        if process.is_null() {
            return Err(format!("its player (pid {from}) couldn't be opened: {}", std::io::Error::last_os_error()));
        }
        let program = image_name(process);
        if !program.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("pms-twatch.exe")) {
            CloseHandle(process);
            return Err(format!("pid {from} isn't a player but {}", program.unwrap_or_else(|| "who knows what".into())));
        }
        let mut mine = std::ptr::null_mut();
        let copied = DuplicateHandle(process, handle as *mut _, GetCurrentProcess(), &mut mine, 0, 0, SAME_ACCESS);
        CloseHandle(process);
        if copied == 0 {
            return Err(format!("its handle couldn't be had: {}", std::io::Error::last_os_error()));
        }
        let kind = kind_of(mine);
        let surface = if kind == SURFACE_KIND {
            crate::meters::compositor(host)?.cast::<ICompositorInterop>().map_err(w)?.CreateCompositionSurfaceForHandle(HANDLE(mine)).map_err(w)
        } else {
            Err(format!("its handle isn't a surface but a {kind}"))
        };
        CloseHandle(mine);
        surface
    }
}

/// What kind of kernel object a DirectComposition surface's handle is (seen 2026-11-17, Windows 11 build 26300).
const SURFACE_KIND: &str = "DxgkCompositionObject";

/// What kind of kernel object `handle` is, as Windows names it.
unsafe fn kind_of(handle: *mut core::ffi::c_void) -> String {
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQueryObject(handle: *mut core::ffi::c_void, class: u32, info: *mut core::ffi::c_void, length: u32, returned: *mut u32) -> i32;
    }
    const OBJECT_TYPE_INFORMATION: u32 = 2;
    // PUBLIC_OBJECT_TYPE_INFORMATION: its name (a UNICODE_STRING: its length in bytes, then a pointer to the name, which is
    // in the same buffer) and more after.
    let mut info = [0u64; 128];
    let mut returned = 0;
    if unsafe { NtQueryObject(handle, OBJECT_TYPE_INFORMATION, info.as_mut_ptr().cast(), size_of_val(&info) as u32, &mut returned) } < 0 {
        return "?".into();
    }
    let length = (info[0] & 0xFFFF) as usize / 2;
    let name = info[1] as *const u16;
    if name.is_null() || length > 64 {
        return "?".into();
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(name, length) })
}

/// The file name of the program `process` runs.
unsafe fn image_name(process: *mut core::ffi::c_void) -> Option<String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryFullProcessImageNameW(process: *mut core::ffi::c_void, flags: u32, name: *mut u16, size: *mut u32) -> i32;
    }
    let (mut name, mut size) = ([0u16; 1024], 1024u32);
    if unsafe { QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut size) } == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&name[..size as usize]);
    path.rsplit(['\\', '/']).next().map(str::to_string)
}

/// Hangs `source`'s live video, if it has some, under its picture: on the sprite already there, or one from the pool.
fn hang(source: &str) -> R<()> {
    let Some(brush) = SURFACES.with_borrow(|s| s.get(source).cloned()) else { return Ok(()) };
    TARGETS.with_borrow_mut(|t| {
        let Some(t) = t.get_mut(source) else { return Ok(()) };
        let sprite = match &t.video {
            Some(s) => s.clone(),
            None => {
                let s = crate::stage::sprite(&crate::meters::compositor(&t.host)?)?;
                ElementCompositionPreview::SetElementChildVisual(&t.host, &s.cast::<Visual>().map_err(w)?).map_err(w)?;
                t.video = Some(s.clone());
                s
            }
        };
        sprite.SetSize(Vector2 { X: t.dips.0 as f32, Y: t.dips.1 as f32 }).map_err(w)?;
        sprite.SetBrush(&brush).map_err(w)?;
        // Black under it, while it's there: a player that's gone leaves its surface empty, and what's over it is see-through,
        // until the set says otherwise a moment later. Better a black screen than a hole in the taskbar meanwhile.
        backed(&t.host, true)
    })
}

/// Black behind `host`'s video (`on`), or nothing.
fn backed(host: &Border, on: bool) -> R<()> {
    let black = windows::UI::Color { A: if on { 0xFF } else { 0 }, R: 0, G: 0, B: 0 };
    host.SetBackground(&SolidColorBrush::CreateInstanceWithColor(black).map_err(w)?).map_err(w)
}

/// No more live video for `source`: its sprite goes back to the pool, and its picture is all there is.
fn let_go(source: &str) {
    SURFACES.with_borrow_mut(|s| s.remove(source));
    TARGETS.with_borrow_mut(|t| {
        if let Some(t) = t.get_mut(source)
            && t.video.take().is_some()
            && let Ok(host) = t.host.cast::<UIElement>()
        {
            crate::stage::unbuild(&host);
            let _ = backed(&t.host, false);
        }
    });
}

/// A picture that never changes: `bgra`, `pixels` (width, height), filling `dips`.
pub(crate) fn still_image(bgra: &[u8], pixels: (u32, u32), dips: (f64, f64)) -> R<Image> {
    let bitmap = WriteableBitmap::CreateInstanceWithDimensions(pixels.0 as i32, pixels.1 as i32).map_err(w)?;
    let buffer = bitmap.PixelBuffer().map_err(w)?;
    let bytes = buffer.cast::<IBufferByteAccess>().map_err(w)?;
    let at = unsafe { bytes.Buffer() }.map_err(w)?;
    let room = (pixels.0 * pixels.1 * 4) as usize;
    unsafe { std::slice::from_raw_parts_mut(at, room) }.copy_from_slice(&bgra[..room]);
    bitmap.Invalidate().map_err(w)?;
    let image = Image::new().map_err(w)?;
    image.SetSource(&bitmap.cast::<ImageSource>().map_err(w)?).map_err(w)?;
    image.SetWidth(dips.0).map_err(w)?;
    image.SetHeight(dips.1).map_err(w)?;
    Ok(image)
}

fn start(source: &str) -> Arc<Feed> {
    let f = Arc::new(Feed { source: source.to_string(), stop: AtomicBool::new(false), size: [AtomicU32::new(0), AtomicU32::new(0)], asked: AtomicBool::new(false), open: Mutex::new(None) });
    if let Ok(mut feeds) = FEEDS.lock() {
        feeds.push(f.clone());
    }
    let thread = f.clone();
    std::thread::spawn(move || pump(&thread));
    f
}

/// What draws a screen decides what a click or a press means, and may come to the foreground for it, which only the process
/// the click went to can allow. Which process that is, the screen's memory says: so it's only allowed if it's one of
/// PimpMyShell's own programs, and not whatever wrote a number there.
fn may_come_forward(screen: &Screen, writer: u32) {
    for pid in [writer, screen.also()] {
        if pid != 0 && is_ours(pid) {
            let _ = unsafe { AllowSetForegroundWindow(pid) };
        }
    }
}

/// The programs a screen's writer can be, that may come forward when it's clicked.
/// `pms.exe` is the daemon too, run as `pms daemon`.
const OURS: [&str; 4] = ["pmsd.exe", "pms.exe", "pms-gui.exe", "pms-pimpbox.exe"];

/// Whether process `pid` is running one of PimpMyShell's programs, by its file name.
fn is_ours(pid: u32) -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut core::ffi::c_void;
        fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
    }
    const QUERY_LIMITED: u32 = 0x1000;
    unsafe {
        let process = OpenProcess(QUERY_LIMITED, 0, pid);
        if process.is_null() {
            return false;
        }
        let file = image_name(process);
        CloseHandle(process);
        file.is_some_and(|file| OURS.iter().any(|o| o.eq_ignore_ascii_case(&file)))
    }
}

/// A click on the picture.
pub(crate) fn click(source: &str) {
    let Some(screen) = screen(source) else { return };
    let writer = screen.click();
    may_come_forward(&screen, writer);
}

pub(crate) fn press(source: &str, button: Button) {
    if let Some(screen) = screen(source) {
        may_come_forward(&screen, screen.writer());
        screen.press(button);
    }
}

pub(crate) fn tune(source: &str, channel: u32) {
    if let Some(screen) = screen(source) {
        may_come_forward(&screen, screen.writer());
        screen.tune(channel);
    }
}

pub(crate) fn status(source: &str) -> Option<Status> {
    Some(screen(source)?.status())
}

pub(crate) fn info(source: &str) -> Option<String> {
    Some(screen(source)?.info())
}

/// No more frames: the strip is going. (Its cells, the video's hosts among them, are taken apart with it, the sprites going
/// back to the pool.)
pub(crate) fn stop() {
    if let Ok(mut feeds) = FEEDS.lock() {
        for f in feeds.drain(..) {
            f.stop.store(true, Ordering::Relaxed);
        }
    }
    TARGETS.with_borrow_mut(HashMap::clear);
    SURFACES.with_borrow_mut(HashMap::clear);
}

/// One set is looking after what goes beside the console's display.
static DOCKING: AtomicBool = AtomicBool::new(false);

fn pump(f: &Arc<Feed>) {
    let running = || !f.stop.load(Ordering::Relaxed);
    let mut looks_after = false;
    while running() {
        let Ok(screen) = Screen::open(&f.source) else {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };
        let screen = Arc::new(screen);
        if let Ok(mut open) = f.open.lock() {
            *open = Some(screen.clone());
        }
        crate::log(format!("screen `{}`: showing", f.source));
        let (mut put_away, mut widened) = (None, None);
        // The console's display, out on the desktop, has a remote and the console's tooltip beside it: one set looks after
        // that, the first there is of the kind with such a remote.
        let mut beside: Option<(Option<[i32; 9]>, u32, String)> = None;
        // A picture made before this opened is still empty, and a channel that's holding still sends nothing new: what's there
        // goes up as soon as it can, whether or not anything new comes.
        let mut first = true;
        // The last offer of live video taken up (`take_surface`): none yet, for a screen only just opened, whatever it's had.
        let mut offered = 0;
        while running() {
            screen.want(f.size[0].load(Ordering::Relaxed), f.size[1].load(Ordering::Relaxed));
            let (n, from, handle) = screen.surface();
            if n != offered {
                offered = n;
                let (source, screen) = (f.source.clone(), screen.clone());
                crate::meters::on_ui(move || take_surface(&source, &screen, n, from, handle));
            }
            let fresh = screen.wait(250) | std::mem::replace(&mut first, false);
            let small = screen.status().collapsed;
            if put_away != Some(small) {
                put_away = Some(small);
                let source = f.source.clone();
                crate::meters::on_ui(move || crate::meters::screen_small(&source, small));
            }
            let wide = screen.status().wide;
            if widened != Some(wide) {
                widened = Some(wide);
                let source = f.source.clone();
                crate::meters::on_ui(move || crate::meters::screen_wide(&source, wide));
            }
            let status = screen.status();
            if !status.modern && !status.note && (looks_after || !DOCKING.swap(true, Ordering::AcqRel)) {
                looks_after = true;
                let now = (crate::remote::display_place(), screen.presses(pms_screen::Button::GamePower).wrapping_add(u32::from(status.game_on) << 31 | status.channel << 8), screen.info());
                if beside.as_ref() != Some(&now) && (now.0.is_some() || beside.as_ref().is_some_and(|b| b.0.is_some())) {
                    let (source, place) = (f.source.clone(), now.0);
                    crate::meters::on_ui(move || crate::remote::dock(&source, place));
                }
                // Once more a moment after it's first there, when the tooltip can be seen: that's when it takes its colors.
                let settled = beside.as_ref().is_some_and(|b| b.0.is_some()) || now.0.is_none();
                beside = Some(if settled { now } else { (now.0, u32::MAX, String::new()) });
            }
            if fresh && !f.asked.swap(true, Ordering::AcqRel) {
                let feed = f.clone();
                crate::meters::on_ui(move || paint(&feed));
            }
        }
    }
    if looks_after {
        DOCKING.store(false, Ordering::Release);
        crate::meters::on_ui(|| crate::remote::dock("", None));
    }
    if let Ok(mut open) = f.open.lock() {
        *open = None;
    }
}

/// Takes the newest frame. On the UI thread.
fn paint(f: &Feed) {
    f.asked.store(false, Ordering::Release);
    let Some(screen) = f.open.lock().ok().and_then(|o| o.clone()) else { return };
    TARGETS.with_borrow_mut(|t| {
        if let Some(t) = t.get_mut(&f.source) {
            let _ = copy(&screen, t);
        }
    });
}

fn copy(screen: &Screen, t: &mut Target) -> R<()> {
    let buffer = t.bitmap.PixelBuffer().map_err(w)?;
    let bytes = buffer.cast::<IBufferByteAccess>().map_err(w)?;
    let at = unsafe { bytes.Buffer() }.map_err(w)?;
    let pixels = unsafe { std::slice::from_raw_parts_mut(at, (t.width * t.height * 4) as usize) };
    if let Some(n) = screen.latest(t.shown, t.width, t.height, pixels) {
        t.shown = n;
        t.bitmap.Invalidate().map_err(w)?;
    }
    Ok(())
}
