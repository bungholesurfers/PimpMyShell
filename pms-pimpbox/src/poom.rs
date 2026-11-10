//! POOM: Doom's games, played by `room`, a Rust port of the Doom engine. The console starts this as a process of its own
//! (`pms-pimpbox --poom <iwad> <pictures>`), and it's all a game is to the console: it hands the engine the six things its
//! platform layer owes it, in place of its window, puts what the engine draws where the console can see it (`link`), and
//! takes what the console says was done at its window from its input. With no one at the keys the engine falls back on its
//! own demo loop. When the console goes, so does this.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{CString, c_char};
use std::io::Read;
use std::time::{Duration, Instant};

use room::doom::d_event::{D_PostEvent, event_t};
use room::doom::doomgeneric::{DG_ScreenBuffer, doomgeneric_Create};
use room::doom::doomkeys::KEY_PAUSE;

use crate::link::{ACROSS, DOWN, Pictures, Said};

const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 35);
const EV_MOUSE: i32 = 2;

thread_local! {
    static PICTURES: RefCell<Option<Pictures>> = const { RefCell::new(None) };
    static STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// How long it's rested in all.
    static RESTED: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    static KEYS: RefCell<VecDeque<(bool, u8)>> = const { RefCell::new(VecDeque::new()) };
    /// A game was paused by closing the window: opening it again carries on.
    static WE_PAUSED: Cell<bool> = const { Cell::new(false) };
    static FRAMES: Cell<(u32, Option<Instant>)> = const { Cell::new((0, None)) };
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
        std::slice::from_raw_parts(at, ACROSS * DOWN)
    };
    PICTURES.with_borrow(|p| {
        if let Some(pictures) = p {
            pictures.put(source);
        }
    });
    let (n, since) = FRAMES.get();
    let since = since.unwrap_or_else(Instant::now);
    if since.elapsed() >= Duration::from_secs(5) {
        println!("pms-pimpbox: POOM: {:.1} frames a second", (n + 1) as f32 / since.elapsed().as_secs_f32());
        FRAMES.set((0, Some(Instant::now())));
    } else {
        FRAMES.set((n + 1, Some(since)));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn DG_SleepMs(ms: u32) {
    std::thread::sleep(Duration::from_millis(u64::from(ms)));
}

/// The game's clock: it stops while the game rests.
#[unsafe(no_mangle)]
pub extern "C" fn DG_GetTicksMs() -> u32 {
    STARTED.get().map_or(0, |t| t.elapsed().saturating_sub(RESTED.get()).as_millis() as u32)
}

/// The next key, for the engine.
///
/// # Safety
/// `pressed` and `key` are the engine's own, and good to write to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn DG_GetKey(pressed: *mut i32, key: *mut u8) -> i32 {
    match KEYS.with_borrow_mut(VecDeque::pop_front) {
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

fn in_game() -> bool {
    unsafe { room::doom::g_game::usergame != 0 }
}

fn paused() -> bool {
    unsafe { room::doom::g_game::paused != 0 }
}

fn tap(key: u8) {
    KEYS.with_borrow_mut(|k| {
        k.push_back((true, key));
        k.push_back((false, key));
    });
}

/// Ends this process, as it ends when the console does.
pub fn leave() -> ! {
    // An instrumented build (PGO training) writes what it recorded first: exit, on Windows, ends the process before the
    // runtime would have.
    #[cfg(pms_pgo)]
    unsafe {
        unsafe extern "C" {
            fn __llvm_profile_write_file() -> i32;
        }
        __llvm_profile_write_file();
    }
    std::process::exit(0)
}

/// Plays the game in `iwad`, putting its pictures up as `pictures`. It stands still until it's told someone's watching.
pub fn main(iwad: &str, pictures: &str) -> ! {
    match Pictures::open(pictures) {
        Ok(p) => PICTURES.with_borrow_mut(|to| *to = Some(p)),
        Err(e) => {
            eprintln!("pms-pimpbox: POOM: {e}");
            std::process::exit(1);
        }
    }
    crate::unix::set_stdout();
    STARTED.set(Some(Instant::now()));
    // What the console says, as it says it; and when it stops saying anything, because it's gone, so does this.
    let (to, said) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut from = std::io::stdin().lock();
        let mut bytes = [0u8; 8];
        while from.read_exact(&mut bytes).is_ok() {
            if let Some(said) = Said::unpack(bytes)
                && to.send(said).is_err()
            {
                break;
            }
        }
        leave();
    });
    // Silent, for now: nothing here opens a sound device. And more memory than Doom had: Freedoom is seven times its size.
    let engine: Vec<CString> = ["pms-pimpbox", "-iwad", iwad, "-nosound", "-mb", "64"].iter().map(|a| CString::new(*a).expect("no NUL")).collect();
    let mut argv: Vec<*mut c_char> = engine.iter().map(|a| a.as_ptr().cast_mut()).collect();
    argv.push(std::ptr::null_mut());
    unsafe { doomgeneric_Create((argv.len() - 1) as i32, argv.as_mut_ptr()) };
    // The game moves 35 times a second, and drawing more often than that shows nothing new.
    let mut next = Instant::now();
    let mut resting = true;
    loop {
        for said in said.try_iter() {
            match said {
                Said::Key { down, key } => KEYS.with_borrow_mut(|k| k.push_back((down, key))),
                // Turning only: pushing the mouse forward walking you into things isn't what anyone expects now.
                Said::Mouse { buttons, across } => D_PostEvent(&event_t { type_: EV_MOUSE, data1: buttons, data2: across * 2, data3: 0, data4: 0 }),
                Said::Opened => {
                    if WE_PAUSED.replace(false) && paused() {
                        tap(KEY_PAUSE);
                    }
                }
                Said::Closed => {
                    if in_game() && !paused() {
                        tap(KEY_PAUSE);
                        WE_PAUSED.set(true);
                    }
                }
                Said::Rest => resting = true,
                Said::Wake => resting = false,
            }
        }
        // With no one watching and no one playing it rests, and carries on from there when someone tunes back in.
        if resting {
            let rest = Duration::from_millis(20);
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
