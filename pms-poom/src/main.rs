//! Doom on a screen in the taskbar. `room`, a Rust port of the Doom engine, draws; we hand it the six things its platform
//! layer owes it, in place of its window, and put what it draws on a screen (pms-screen), shrunk to the size the taskbar
//! asks for. With no one at the keys the engine falls back on its own demo loop: that's what the taskbar shows.
//!
//!   pms-poom <iwad> [screen] [parent process]
//!
//! With a parent (the daemon), it goes when the parent does.

#![windows_subsystem = "windows"]

mod play;
mod unix;

use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char};
use std::time::{Duration, Instant};

use pms_screen::Screen;
use room::doom::doomgeneric::{DG_ScreenBuffer, DOOMGENERIC_RESX, DOOMGENERIC_RESY, doomgeneric_Create};

const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 35);

thread_local! {
    static SCREEN: RefCell<Option<Screen>> = const { RefCell::new(None) };
    static STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Each frame's pixels, kept between frames.
    static PIXELS: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static FRAMES: Cell<(u32, Option<Instant>)> = const { Cell::new((0, None)) };
    /// How long it's rested in all.
    static RESTED: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

/// The engine's frame, averaged down to `width` by `height`, opaque. Every source pixel counts, so thin lines don't flicker
/// as they would if pixels were only picked.
fn shrink(source: &[u32], width: usize, height: usize, out: &mut Vec<u8>) {
    out.clear();
    out.reserve(width * height * 4);
    for y in 0..height {
        let (top, bottom) = (y * DOOMGENERIC_RESY / height, ((y + 1) * DOOMGENERIC_RESY / height).max(y * DOOMGENERIC_RESY / height + 1));
        for x in 0..width {
            let (left, right) = (x * DOOMGENERIC_RESX / width, ((x + 1) * DOOMGENERIC_RESX / width).max(x * DOOMGENERIC_RESX / width + 1));
            let (mut b, mut g, mut r) = (0u32, 0u32, 0u32);
            for row in top..bottom {
                for &p in &source[row * DOOMGENERIC_RESX + left..row * DOOMGENERIC_RESX + right] {
                    b += p & 0xFF;
                    g += (p >> 8) & 0xFF;
                    r += (p >> 16) & 0xFF;
                }
            }
            let n = ((bottom - top) * (right - left)) as u32;
            out.extend_from_slice(&[(b / n) as u8, (g / n) as u8, (r / n) as u8, 255]);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_Init() {}

#[unsafe(no_mangle)]
pub extern "C" fn DG_DrawFrame() {
    let source = unsafe {
        let at = DG_ScreenBuffer as *const u32;
        if at.is_null() {
            return;
        }
        std::slice::from_raw_parts(at, DOOMGENERIC_RESX * DOOMGENERIC_RESY)
    };
    if play::is_open() {
        play::draw(source);
    }
    SCREEN.with_borrow(|s| {
        let Some(screen) = s else { return };
        let (w, h) = screen.wanted().unwrap_or((64, 48));
        PIXELS.with_borrow_mut(|px| {
            shrink(source, w as usize, h as usize, px);
            screen.publish(w, h, px);
        });
    });
    let (n, since) = FRAMES.get();
    let since = since.unwrap_or_else(Instant::now);
    if since.elapsed() >= Duration::from_secs(5) {
        println!("pms-poom: {:.1} frames a second", (n + 1) as f32 / since.elapsed().as_secs_f32());
        FRAMES.set((0, Some(Instant::now())));
    } else {
        FRAMES.set((n + 1, Some(since)));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_SleepMs(ms: u32) {
    std::thread::sleep(Duration::from_millis(u64::from(ms)));
}

/// The game's clock: it stops while the console rests.
#[unsafe(no_mangle)]
pub extern "C" fn DG_GetTicksMs() -> u32 {
    STARTED.get().map_or(0, |t| t.elapsed().saturating_sub(RESTED.get()).as_millis() as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_GetKey(pressed: *mut i32, key: *mut u8) -> i32 {
    match play::next_key() {
        Some((down, doom_key)) => {
            unsafe {
                *pressed = i32::from(down);
                *key = doom_key;
            }
            1
        }
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_SetWindowTitle(_title: *const c_char) {}

/// Ends this process when `parent` ends, however it does.
fn go_with(parent: u32) {
    use windows::Win32::System::Threading::{INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, parent) }) else { std::process::exit(0) };
    let handle = handle.0 as isize;
    std::thread::spawn(move || {
        unsafe { WaitForSingleObject(windows::Win32::Foundation::HANDLE(handle as *mut _), INFINITE) };
        // An instrumented build (PGO training) writes what it recorded first: exit, on Windows, ends the process before the
        // runtime would have.
        #[cfg(pms_pgo)]
        unsafe {
            unsafe extern "C" {
                fn __llvm_profile_write_file() -> i32;
            }
            __llvm_profile_write_file();
        }
        std::process::exit(0);
    });
}

fn main() {
    pms_screen::started();
    let args: Vec<String> = std::env::args().collect();
    let Some(iwad) = args.get(1) else {
        eprintln!("usage: pms-poom <iwad> [screen]");
        std::process::exit(2);
    };
    let name = args.get(2).map_or("poom", String::as_str);
    if let Some(parent) = args.get(3).and_then(|p| p.parse().ok()) {
        go_with(parent);
    }
    let screen = match Screen::create(name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("pms-poom: {e}");
            std::process::exit(1);
        }
    };
    unix::set_stdout();
    SCREEN.with_borrow_mut(|s| *s = Some(screen));
    STARTED.set(Some(Instant::now()));
    // Silent, for now: nothing here opens a sound device. And more memory than Doom had: Freedoom is seven times its size.
    let engine: Vec<CString> = ["pms-poom", "-iwad", iwad, "-nosound", "-mb", "64"].iter().map(|a| CString::new(*a).expect("no NUL")).collect();
    let mut argv: Vec<*mut c_char> = engine.iter().map(|a| a.as_ptr().cast_mut()).collect();
    argv.push(std::ptr::null_mut());
    unsafe { doomgeneric_Create((argv.len() - 1) as i32, argv.as_mut_ptr()) };
    // The game moves 35 times a second, and drawing more often than that shows nothing new.
    let mut next = Instant::now();
    let started = Instant::now();
    let mut clicks = SCREEN.with_borrow(|s| s.as_ref().map_or(0, Screen::clicks));
    loop {
        let now_clicks = SCREEN.with_borrow(|s| s.as_ref().map_or(0, Screen::clicks));
        if now_clicks != clicks {
            clicks = now_clicks;
            play::open();
        }
        play::pump();
        // With no one watching and no one playing it rests, and carries on from there when someone tunes back in.
        let watched = SCREEN.with_borrow(|s| s.as_ref().is_some_and(|s| s.watched_within(1500)));
        if !watched && !play::is_open() && started.elapsed() > Duration::from_secs(3) {
            let rest = Duration::from_millis(100);
            std::thread::sleep(rest);
            RESTED.set(RESTED.get() + rest);
            next = Instant::now();
            continue;
        }
        room::doom::d_main::doomgeneric_Tick();
        next += FRAME;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
    }
}
