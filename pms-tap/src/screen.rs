//! `screen` meters' pictures: live ones, drawn by another process and handed over in shared memory (pms-screen). For each, a
//! thread of ours waits for frames and asks the UI thread to take the newest, once at a time, so a busy taskbar skips frames
//! rather than queueing them. All the UI thread does is copy a few KB into a bitmap the size of the cell's pixels.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pms_screen::{Button, Screen, Status};
use pms_xaml::{Image, ImageSource, WriteableBitmap};
use windows::Win32::System::WinRT::IBufferByteAccess;
use windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;
use windows_core::Interface;

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
}

thread_local! {
    static TARGETS: RefCell<HashMap<String, Target>> = RefCell::new(HashMap::new());
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

/// A picture of `pixels` (width, height) filling `dips`, showing the screen called `source`.
pub(crate) fn picture(source: &str, pixels: (u32, u32), dips: (f64, f64)) -> R<Image> {
    let (width, height) = (pixels.0.clamp(1, pms_screen::MAX_SIDE), pixels.1.clamp(1, pms_screen::MAX_SIDE));
    let bitmap = WriteableBitmap::CreateInstanceWithDimensions(width as i32, height as i32).map_err(w)?;
    let image = Image::new().map_err(w)?;
    image.SetSource(&bitmap.cast::<ImageSource>().map_err(w)?).map_err(w)?;
    image.SetWidth(dips.0).map_err(w)?;
    image.SetHeight(dips.1).map_err(w)?;
    TARGETS.with_borrow_mut(|t| t.insert(source.to_string(), Target { bitmap, width, height, shown: 0 }));
    let f = feed(source).unwrap_or_else(|| start(source));
    f.size[0].store(width, Ordering::Relaxed);
    f.size[1].store(height, Ordering::Relaxed);
    // A new picture starts empty, and a channel that's holding still won't send it anything. If the feed isn't open yet, its
    // thread does this once it is.
    paint(&f);
    Ok(image)
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
/// the click went to can allow.
fn may_come_forward(screen: &Screen, writer: u32) {
    let _ = unsafe { AllowSetForegroundWindow(writer) };
    let also = screen.also();
    if also != 0 {
        let _ = unsafe { AllowSetForegroundWindow(also) };
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

/// No more frames: the strip is going.
pub(crate) fn stop() {
    if let Ok(mut feeds) = FEEDS.lock() {
        for f in feeds.drain(..) {
            f.stop.store(true, Ordering::Relaxed);
        }
    }
    TARGETS.with_borrow_mut(HashMap::clear);
}

fn pump(f: &Arc<Feed>) {
    let running = || !f.stop.load(Ordering::Relaxed);
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
        let mut put_away = None;
        // A picture made before this opened is still empty, and a channel that's holding still sends nothing new: what's there
        // goes up as soon as it can, whether or not anything new comes.
        let mut first = true;
        while running() {
            screen.want(f.size[0].load(Ordering::Relaxed), f.size[1].load(Ordering::Relaxed));
            let fresh = screen.wait(250) | std::mem::replace(&mut first, false);
            let small = screen.status().collapsed;
            if put_away != Some(small) {
                put_away = Some(small);
                let source = f.source.clone();
                crate::meters::on_ui(move || crate::meters::screen_small(&source, small));
            }
            if fresh && !f.asked.swap(true, Ordering::AcqRel) {
                let feed = f.clone();
                crate::meters::on_ui(move || paint(&feed));
            }
        }
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
