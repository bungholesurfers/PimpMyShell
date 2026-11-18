//! POOM: Doom's games, played by `room`, a Rust port of the Doom engine. The console starts this, a program of its own
//! (`pms-poom <iwad> <pictures>`), and it's all a game is to the console: it hands the engine the six things its platform
//! layer owes it, in place of its window, puts what the engine draws where the console can see it (`link`), and takes what
//! the console says was done at its window from its input. With no one at the keys the engine falls back on its own demo
//! loop. When the console goes, so does this.
//!
//! It has no window, and draws nothing itself: nothing here calls on windows or drawing (user32, gdi32), so that,
//! sandboxed, it's started without them at all.

#![windows_subsystem = "windows"]

mod unix;
/// What we changed in the engine, tested here, where it links with the C functions it needs (`unix`).
#[cfg(test)]
mod engine_fixes;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{CString, c_char};
use std::io::Read;
use std::time::{Duration, Instant};

use room::doom::d_event::{D_PostEvent, event_t};
use room::doom::doomgeneric::{DG_ScreenBuffer, doomgeneric_Create};
use room::doom::doomkeys::KEY_PAUSE;

use poom::link::{ACROSS, DOWN, Pictures, Said};

// What the console was told of the engine (link), as the engine has it.
const _: () = {
    use poom::link::keys as k;
    use room::doom::doomkeys::*;
    assert!(ACROSS == room::doom::doomgeneric::DOOMGENERIC_RESX && DOWN == room::doom::doomgeneric::DOOMGENERIC_RESY);
    let ours = [k::RIGHTARROW, k::LEFTARROW, k::UPARROW, k::DOWNARROW, k::STRAFE_L, k::STRAFE_R, k::USE, k::FIRE, k::ESCAPE, k::ENTER, k::TAB, k::BACKSPACE, k::PAUSE, k::EQUALS, k::MINUS, k::RSHIFT, k::LALT];
    let its = [KEY_RIGHTARROW, KEY_LEFTARROW, KEY_UPARROW, KEY_DOWNARROW, KEY_STRAFE_L, KEY_STRAFE_R, KEY_USE, KEY_FIRE, KEY_ESCAPE, KEY_ENTER, KEY_TAB, KEY_BACKSPACE, KEY_PAUSE, KEY_EQUALS, KEY_MINUS, KEY_RSHIFT, KEY_LALT];
    let f = [KEY_F1, KEY_F2, KEY_F3, KEY_F4, KEY_F5, KEY_F6, KEY_F7, KEY_F8, KEY_F9, KEY_F10, KEY_F11, KEY_F12];
    let mut i = 0;
    while i < ours.len() {
        assert!(ours[i] == its[i], "a key's number isn't the engine's");
        i += 1;
    }
    let mut i = 0;
    while i < f.len() {
        assert!(k::F[i] == f[i], "a function key's number isn't the engine's");
        i += 1;
    }
};

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
    /// Checking a demo (`--check`): nothing's waited for.
    static CHECKING: Cell<bool> = const { Cell::new(false) };
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
    if !CHECKING.get() {
        std::thread::sleep(Duration::from_millis(u64::from(ms)));
    }
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

/// Which of the `given` demos the engine's playing just now, if it's playing one of them: they're its DEMO1 and on (`-demos`),
/// in the order they were given. Its own (beyond those given) aren't any of them.
fn showing_demo(given: usize) -> Option<usize> {
    unsafe {
        let name = room::doom::g_game::defdemoname;
        if room::doom::g_game::demoplayback == 0 || name.is_null() {
            return None;
        }
        let name = std::ffi::CStr::from_ptr(name).to_string_lossy().to_ascii_lowercase();
        let n: usize = name.strip_prefix("demo")?.parse().ok()?;
        (1..=given).contains(&n).then(|| n - 1)
    }
}

/// Ends this process, as it ends when the console does.
fn leave() -> ! {
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

/// How far the mouse turns you, against what the engine would make of it: 4, twice what it was until 0.1.8, which
/// players found slow.
const TURNING: i32 = 4;

/// Checks a speed demo before it's shown (the daemon's demos.rs): plays it on `iwad` unseen, as fast as it goes, and says on
/// stdout when its first level was left, by the level's clock in tics, and how many times the player fired a weapon on the
/// way (`left 869` and `fired 42`, a line each), and ends there. A demo that's out of step on this game never leaves its level: it plays to its
/// end, and the engine ends this without saying anything of the kind. About a tenth of a second, whatever the demo.
fn check(iwad: &str, demo: &str) -> ! {
    CHECKING.set(true);
    unix::set_stdout();
    let engine: Vec<CString> = ["pms-poom", "-iwad", iwad, "-nosound", "-nogui", "-mb", "64", "-nodraw", "-timedemo", demo].iter().map(|a| CString::new(*a).expect("no NUL")).collect();
    let mut argv: Vec<*mut c_char> = engine.iter().map(|a| a.as_ptr().cast_mut()).collect();
    argv.push(std::ptr::null_mut());
    unsafe { doomgeneric_Create((argv.len() - 1) as i32, argv.as_mut_ptr()) };
    let mut in_level = false;
    loop {
        room::doom::d_main::doomgeneric_Tick();
        let (state, time) = unsafe { (room::doom::g_game::gamestate, room::doom::p_tick::leveltime) };
        if in_level && state != 0 {
            // A line each, so that a daemon from before there was a count still reads when.
            println!("left {time}\nfired {}", unsafe { room::doom::p_pspr::weapon_firings });
            let _ = std::io::Write::flush(&mut std::io::stdout());
            leave();
        }
        in_level = state == 0;
    }
}

/// Plays the game in `iwad`, putting its pictures up in the memory whose handle it was given. It stands still until it's
/// told someone's watching. Any demos it's given (checked ones) play in the title loop in place of the game's own.
///
///   pms-poom <iwad> <pictures' handle> [demo...]
///   pms-poom --check <iwad> <demo>
///   pms-poom --maps <iwad>        `maps E1M1 E1M2 ...`, by its directory; nothing's played
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--maps") {
        let maps = args.get(2).and_then(|iwad| std::fs::File::open(iwad).ok()).and_then(|f| poom::maps::in_wad(std::io::BufReader::new(f)));
        match maps {
            Some(maps) => {
                println!("maps {}", maps.join(" "));
                let _ = std::io::Write::flush(&mut std::io::stdout());
                std::process::exit(0)
            }
            None => std::process::exit(1),
        }
    }
    if args.get(1).is_some_and(|a| a == "--check") {
        match (args.get(2), args.get(3)) {
            (Some(iwad), Some(demo)) => check(iwad, demo),
            _ => std::process::exit(2),
        }
    }
    let (Some(iwad), Some(pictures)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: pms-poom <iwad> <pictures' handle> [demo...]");
        std::process::exit(2);
    };
    let demos = &args[3..];
    match pictures.parse().map_err(|_| "its pictures' handle is a number".to_string()).and_then(Pictures::from_handle) {
        Ok(p) => PICTURES.with_borrow_mut(|to| *to = Some(p)),
        Err(e) => {
            eprintln!("pms-pimpbox: POOM: {e}");
            std::process::exit(1);
        }
    }
    unix::set_stdout();
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
    // Silent, for now: nothing here opens a sound device, and the engine's built without one (its `sound` feature). When sound
    // is wanted, it can't be that device here: it needs COM, whose ole32 brings user32, and sandboxed this starts without
    // win32k, so it wouldn't start at all (0xC0000142). Have the console play it instead, which has a window anyway: the game
    // hands it what to play (the samples and the music, or which ones and where), down a pipe or in shared memory, as it
    // hands it its pictures. And more memory than Doom had: Freedoom is seven times its size.
    // -nogui: the engine's error box is a Linux program (zenity), which there's no starting here.
    let mut engine: Vec<&str> = vec!["pms-poom", "-iwad", iwad, "-nosound", "-nogui", "-mb", "64"];
    if !demos.is_empty() {
        engine.push("-demos");
        engine.extend(demos.iter().map(String::as_str));
    }
    let engine: Vec<CString> = engine.iter().map(|a| CString::new(*a).expect("no NUL")).collect();
    let mut argv: Vec<*mut c_char> = engine.iter().map(|a| a.as_ptr().cast_mut()).collect();
    argv.push(std::ptr::null_mut());
    unsafe { doomgeneric_Create((argv.len() - 1) as i32, argv.as_mut_ptr()) };
    // The game moves 35 times a second, and drawing more often than that shows nothing new.
    let mut next = Instant::now();
    let mut resting = true;
    // Which of the demos it was given it's showing, said to the console as that changes (for the PIMPBOX card to say whose run
    // it is): `pms-poom: showing demo <n>`, counting from 0, or `showing no demo`.
    let mut showing: Option<Option<usize>> = None;
    loop {
        let now_showing = showing_demo(demos.len());
        if showing != Some(now_showing) {
            showing = Some(now_showing);
            match now_showing {
                Some(n) => eprintln!("pms-poom: showing demo {n}"),
                None => eprintln!("pms-poom: showing no demo"),
            }
        }
        for said in said.try_iter() {
            match said {
                Said::Key { down, key } => KEYS.with_borrow_mut(|k| k.push_back((down, key))),
                // Turning only: pushing the mouse forward walking you into things isn't what anyone expects now.
                Said::Mouse { buttons, across } => D_PostEvent(&event_t { type_: EV_MOUSE, data1: buttons, data2: across * TURNING, data3: 0, data4: 0 }),
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
