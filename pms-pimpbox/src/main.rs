//! The PIMPBOX: a console on a screen in the taskbar. It comes on with its boot animation (`boot`), plays a game, and puts
//! what the game draws on its screen (pms-screen), shrunk to the size the taskbar asks for, and in its window (`play`), its
//! display on your desktop, at the size of its video mode (`video`). The game is POOM (`poom`): Doom's games on `room`, a
//! Rust port of the Doom engine, as a process of its own that this starts, so that the console and its window are still
//! there when a game ends: then it comes back on by itself, with a shorter animation, and plays the game again. Switched
//! off (its screen's power keys), it's dark, and its window stays. Before it comes on and after, and before a game's seen,
//! there's a moment of nothing, never the same length twice.
//!
//!   pms-pimpbox <iwad> [screen] [parent process] [video mode] [on|off] [folder for compiled shaders]
//!   pms-pimpbox --poom <iwad> <pictures>          the game: what the console starts
//!
//! With a parent (the daemon), it goes when the parent does (0 is none). The video mode is how big a picture the console
//! puts out, which is what its window shows: 480p unless it's told 720p or 1080i. The screen's video keys change it while
//! it's on. It starts switched on unless it's told off.

#![windows_subsystem = "windows"]

mod boot;
mod link;
mod play;
mod poom;
mod sorry;
mod spinner;
mod unix;
mod video;

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use link::{ACROSS, DOWN, Pictures, Said};
use pms_screen::{Button, Screen};

/// Part of a frame whose rows are `stride` pixels long (the part `across` by `down` pixels that starts `left` pixels in),
/// averaged down to `width` by `height`, opaque. Every source pixel counts, so thin lines don't flicker as they would if
/// pixels were only picked.
fn shrink(source: &[u32], stride: usize, (left, across, down): (usize, usize, usize), width: usize, height: usize, out: &mut Vec<u8>) {
    out.clear();
    out.reserve(width * height * 4);
    for y in 0..height {
        let (top, bottom) = (y * down / height, ((y + 1) * down / height).max(y * down / height + 1));
        for x in 0..width {
            let (from, to) = (left + x * across / width, left + ((x + 1) * across / width).max(x * across / width + 1));
            let (mut b, mut g, mut r) = (0u32, 0u32, 0u32);
            for row in top..bottom {
                for &p in &source[row * stride + from..row * stride + to] {
                    b += p & 0xFF;
                    g += (p >> 8) & 0xFF;
                    r += (p >> 16) & 0xFF;
                }
            }
            let n = ((bottom - top) * (to - from)) as u32;
            out.extend_from_slice(&[(b / n) as u8, (g / n) as u8, (r / n) as u8, 255]);
        }
    }
}

/// Puts a picture on the taskbar's screen, at the size that asks for, whatever shape that is: `source` is `across` by
/// `down` pixels, to be seen `shape` times as wide as it's tall. Where the screen's narrower than the picture it gets the
/// middle of it; where it's wider, all of it, with black to each side.
fn show(screen: &Screen, source: &[u32], (across, down): (usize, usize), shape: f32, shrunk: &mut Vec<u8>) {
    let (w, h) = screen.wanted().unwrap_or((64, 48));
    let (w, h) = (w as usize, h as usize);
    let asked = w as f32 / h as f32;
    if shape >= asked * 0.98 {
        let part = ((across as f32 * asked / shape).round() as usize).clamp(1, across);
        shrink(source, across, ((across - part) / 2, part, down), w, h, shrunk);
    } else {
        let wide = ((h as f32 * shape).round() as usize).clamp(1, w);
        let mut middle = Vec::new();
        shrink(source, across, (0, across, down), wide, h, &mut middle);
        shrunk.clear();
        shrunk.resize(w * h * 4, 0);
        shrunk.chunks_mut(4).for_each(|p| p[3] = 255);
        let left = (w - wide) / 2;
        for (to, from) in shrunk.chunks_mut(w * 4).zip(middle.chunks(wide * 4)) {
            to[left * 4..(left + wide) * 4].copy_from_slice(from);
        }
    }
    screen.publish(w as u32, h as u32, shrunk);
}

/// Ends this process when `parent` ends, however it does.
fn go_with(parent: u32) {
    use windows::Win32::System::Threading::{INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, parent) }) else { std::process::exit(0) };
    let handle = handle.0 as isize;
    std::thread::spawn(move || {
        unsafe { WaitForSingleObject(windows::Win32::Foundation::HANDLE(handle as *mut _), INFINITE) };
        poom::leave();
    });
}

/// What the console's doing.
enum Doing {
    /// Switched off: nothing on its screen, and no game.
    Off,
    /// A moment of nothing: before it comes on, and after, and before a game's seen. Until when, and what then.
    Dark(Instant, Then),
    /// Coming on: the animation plays, and the game, started with it, loads behind it and waits.
    Booting(boot::Booting),
    Playing,
}

/// What comes after the dark.
#[derive(Clone, Copy)]
enum Then {
    /// It comes on: with the short animation, if it's only just been on; and in that, if the game fell over, a notice of it.
    Boot { short: bool, sorry: bool },
    Play,
}

/// How long the dark lasts: from half a second to a whole one, and not the same twice.
fn a_moment() -> Instant {
    use std::hash::{BuildHasher, Hasher};
    let mut dice = std::collections::hash_map::RandomState::new().build_hasher();
    dice.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    Instant::now() + Duration::from_millis(500 + dice.finish() % 501)
}

/// The game, as the console has hold of it.
struct Game {
    child: Child,
    /// Whether it's been told to stand still.
    resting: bool,
    /// The last things it said on its way: why it fell over, if it does.
    said: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<String>>>,
}

impl Game {
    /// Starts the game in `iwad`, standing still, putting its pictures up as `pictures`.
    fn start(iwad: &str, pictures: &str) -> Option<Game> {
        let me = std::env::current_exe().ok()?;
        match Command::new(me).args(["--poom", iwad, pictures]).stdin(Stdio::piped()).stderr(Stdio::piped()).spawn() {
            Ok(mut child) => {
                // Said where whatever started the console keeps what it says: a game that's only just started hasn't been
                // played, whatever the one before it was.
                eprintln!("pms-pimpbox: POOM started");
                // What it says is kept, the last of it: it's only wanted if it falls over.
                let said = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
                if let Some(from) = child.stderr.take() {
                    let said = said.clone();
                    std::thread::spawn(move || {
                        for line in std::io::BufRead::lines(std::io::BufReader::new(from)).map_while(Result::ok) {
                            if let Ok(mut said) = said.lock() {
                                if said.len() == 40 {
                                    said.pop_front();
                                }
                                said.push_back(line);
                            }
                        }
                    });
                }
                Some(Game { child, resting: true, said })
            }
            Err(e) => {
                eprintln!("pms-pimpbox: POOM didn't start: {e}");
                None
            }
        }
    }

    /// Tells it something. False if it isn't there to be told.
    fn tell(&mut self, said: Said) -> bool {
        self.child.stdin.as_mut().is_some_and(|to| to.write_all(&said.pack()).and_then(|()| to.flush()).is_ok())
    }

    /// Whether it's ended; and if it has, whether it ended as a game does that's been quit, or fell over.
    fn over(&mut self) -> Option<bool> {
        match self.child.try_wait() {
            Ok(None) => None,
            Ok(Some(how)) => {
                if !how.success() {
                    // Said where whatever started the console keeps what it says: how it ended, and its last words.
                    std::thread::sleep(Duration::from_millis(60));
                    let words: Vec<String> = self.said.lock().map(|said| said.iter().filter(|l| !l.trim().is_empty()).cloned().collect()).unwrap_or_default();
                    let last: Vec<String> = words.iter().rev().take(12).rev().cloned().collect();
                    // Its last words once, however many times running it says them: one that falls over every time it's
                    // started says the same each time, and the record needn't.
                    static BEFORE: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
                    let mut before = BEFORE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if *before == last && !last.is_empty() {
                        eprintln!("pms-pimpbox: POOM fell over again ({how}), saying what it said before");
                    } else {
                        eprintln!("pms-pimpbox: POOM fell over ({how})");
                        for line in &last {
                            eprintln!("pms-pimpbox: POOM said: {line}");
                        }
                        *before = last;
                    }
                }
                Some(how.success())
            }
            Err(_) => Some(false),
        }
    }

    /// Ends it, there and then.
    fn end(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn main() {
    pms_screen::started();
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--poom") {
        match (args.get(2), args.get(3)) {
            (Some(iwad), Some(pictures)) => poom::main(iwad, pictures),
            _ => std::process::exit(2),
        }
    }
    let Some(mut iwad) = args.get(1).cloned() else {
        eprintln!("usage: pms-pimpbox <iwad> [screen] [parent process] [video mode] [on|off]");
        std::process::exit(2);
    };
    let name = args.get(2).map_or("pimpbox", String::as_str);
    if let Some(parent) = args.get(3).and_then(|p| p.parse().ok()).filter(|p| *p != 0) {
        go_with(parent);
    }
    video::set(args.get(4).map_or("", String::as_str));
    let screen = match Screen::create(name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("pms-pimpbox: {e}");
            std::process::exit(1);
        }
    };
    let pictures_name = format!("Local\\PimpMyShell.Pimpbox.{}", std::process::id());
    let pictures = match Pictures::open(&pictures_name) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("pms-pimpbox: {e}");
            std::process::exit(1);
        }
    };
    let kept = args.get(6).map(std::path::PathBuf::from);
    let started = Instant::now();
    let mut clicks = screen.clicks();
    let keys = [Button::Video480, Button::Video720, Button::Video1080, Button::ConsoleOn, Button::ConsoleOff, Button::ConsoleGame];
    let mut pressed = keys.map(|key| screen.presses(key));
    // Switched on, unless it's told it starts off: a moment of nothing, and then it comes on.
    let mut doing = if args.get(5).is_some_and(|a| a == "off") { Doing::Off } else { Doing::Dark(a_moment(), Then::Boot { short: false, sorry: false }) };
    let mut game: Option<Game> = None;
    // The newest picture of whatever's on; and room to work in.
    let (mut seen, mut picture) = (0u64, Vec::new());
    let (mut out, mut shrunk) = (Vec::new(), Vec::new());
    // When the game last fell over, or couldn't be started: it isn't tried again sooner than the notice of that takes,
    // whether or not there was a notice (with no graphics card there's none, and nothing else to slow it).
    let mut fell: Option<Instant> = None;
    let try_again = |fell: &mut Option<Instant>| {
        let soonest = fell.map_or_else(Instant::now, |at| at + Duration::from_secs(12));
        *fell = Some(Instant::now());
        a_moment().max(soonest)
    };
    // Somebody's pressed a key at the game that's going: said once for each game, for whoever decides what can be closed.
    let mut played = false;
    let mut shown_dark = Instant::now() - Duration::from_secs(1);
    loop {
        // A click on the screen opens the window, whatever's on, or nothing.
        let now_clicks = screen.clicks();
        if now_clicks != clicks {
            clicks = now_clicks;
            if let Doing::Booting(booting) = &doing {
                booting.want();
            }
            play::open();
            shown_dark = Instant::now() - Duration::from_secs(1);
        }
        let mut next = None;
        // Its keys: the picture's another size from now on, and so is the window; and it's switched on, or off.
        let now_pressed = keys.map(|key| screen.presses(key));
        for (i, key) in keys.iter().enumerate().filter(|(i, _)| now_pressed[*i] != pressed[*i]) {
            match key {
                Button::Video480 | Button::Video720 | Button::Video1080 if video::name() != video::MODES[i].0 => {
                    video::set(video::MODES[i].0);
                    if let Doing::Booting(booting) = &doing {
                        booting.set_mode(video::mode());
                    }
                    play::fit_mode();
                    shown_dark = Instant::now() - Duration::from_secs(1);
                }
                Button::ConsoleOn if matches!(doing, Doing::Off) => next = Some(Doing::Dark(a_moment(), Then::Boot { short: false, sorry: false })),
                Button::ConsoleOff if !matches!(doing, Doing::Off) => {
                    // Off: whatever it was doing ends there and then, the game or its own coming on, and whatever was on
                    // its screen goes with it. Its window stays.
                    if let Doing::Booting(booting) = &doing {
                        booting.stop();
                    }
                    if let Some(game) = game.take() {
                        game.end();
                    }
                    next = Some(Doing::Off);
                    shown_dark = Instant::now() - Duration::from_secs(1);
                }
                // Another game: which, its screen says. Switched on, the one that's going ends, and it comes back on with
                // the new one the short way, as it's been on all along. Switched off, it's only what it'll play.
                Button::ConsoleGame => {
                    let Some(wad) = screen.info().lines().find_map(|l| l.strip_prefix("wad\t").map(str::to_string)) else { continue };
                    if wad == iwad {
                        continue;
                    }
                    iwad = wad;
                    if !matches!(doing, Doing::Off) {
                        if let Doing::Booting(booting) = &doing {
                            booting.stop();
                        }
                        if let Some(game) = game.take() {
                            game.end();
                        }
                        next = Some(Doing::Dark(a_moment(), Then::Boot { short: true, sorry: false }));
                        shown_dark = Instant::now() - Duration::from_secs(1);
                    }
                }
                _ => {}
            }
        }
        pressed = now_pressed;
        // Switched on or off, it's that from here on: not what it was until the next time round.
        if let Some(next) = next.take() {
            doing = next;
        }
        // The mouse is the game's while there's a game, and yours otherwise.
        play::playing(matches!(doing, Doing::Playing));
        let said = play::messages();
        let watched = screen.watched_within(1500) || play::is_open() || started.elapsed() < Duration::from_secs(3);
        match &doing {
            Doing::Off | Doing::Dark(..) => {
                // Nothing, on the taskbar and in the window: said now and then, as either may have only just looked.
                if shown_dark.elapsed() >= Duration::from_millis(250) {
                    shown_dark = Instant::now();
                    let (w, h) = screen.wanted().unwrap_or((64, 48));
                    shrunk.clear();
                    shrunk.resize((w * h * 4) as usize, 0);
                    shrunk.chunks_mut(4).for_each(|p| p[3] = 255);
                    screen.publish(w, h, &shrunk);
                    if play::is_open() {
                        let (w, h) = video::mode();
                        out.clear();
                        out.resize(w * h, 0);
                        play::draw(&out, (w, h));
                    }
                }
                if let Doing::Dark(until, then) = &doing
                    && Instant::now() >= *until
                    && next.is_none()
                {
                    next = Some(match *then {
                        Then::Boot { short, sorry } => {
                            // The game's started with the animation, to load behind it; it stands still until it's seen.
                            seen = 0;
                            if let Some(old) = game.take() {
                                old.end();
                            }
                            game = Game::start(&iwad, &pictures_name);
                            let booting = boot::start(name, video::mode(), short, sorry, kept.clone());
                            if play::is_open() {
                                booting.want();
                            }
                            Doing::Booting(booting)
                        }
                        Then::Play if game.is_some() => Doing::Playing,
                        // There's no game to play: it couldn't be started. Said, the way one falling over is.
                        Then::Play => Doing::Dark(try_again(&mut fell), Then::Boot { short: true, sorry: true }),
                    });
                }
            }
            Doing::Booting(booting) if booting.spinning() => {
                // Its shader's being compiled: the wheel, on the taskbar and in the window, until it's ready.
                if shown_dark.elapsed() >= Duration::from_millis(16) {
                    shown_dark = Instant::now();
                    let size = video::mode();
                    spinner::draw(&mut out, size, started.elapsed().as_secs_f32());
                    show(&screen, &out, size, size.0 as f32 / size.1 as f32, &mut shrunk);
                    if play::is_open() {
                        play::draw(&out, size);
                    }
                }
            }
            Doing::Booting(booting) => {
                if play::is_open()
                    && let Some((newer, its_size)) = booting.latest(seen, &mut picture)
                {
                    seen = newer;
                    play::draw(&picture, its_size);
                }
                // Whatever was done at the window meanwhile was done at nothing: the game doesn't get it. And after it,
                // a moment of nothing again, before the game's seen.
                if booting.over() && next.is_none() {
                    seen = 0;
                    shown_dark = Instant::now() - Duration::from_secs(1);
                    next = Some(Doing::Dark(a_moment(), Then::Play));
                }
            }
            Doing::Playing => {
                let Some(playing) = game.as_mut() else { unreachable!() };
                if !played && said.iter().any(|s| matches!(s, Said::Key { down: true, .. })) {
                    played = true;
                    eprintln!("pms-pimpbox: POOM played");
                }
                let mut there = said.into_iter().all(|said| playing.tell(said));
                if watched == playing.resting {
                    playing.resting = !watched;
                    there &= playing.tell(if watched { Said::Wake } else { Said::Rest });
                }
                if let Some(newer) = pictures.newest(seen, &mut picture) {
                    seen = newer;
                    // Doom's picture was 4:3 on a screen of its day, whatever its pixels' shape.
                    show(&screen, &picture, (ACROSS, DOWN), 4.0 / 3.0, &mut shrunk);
                    if play::is_open() {
                        video::compose(&picture, (ACROSS, DOWN), 4.0 / 3.0, &mut out);
                        play::draw(&out, video::mode());
                    }
                }
                let ended = playing.over().or((!there).then_some(false));
                if let Some(quit) = ended
                    && next.is_none()
                {
                    // The game's over, and the console comes back on by itself, as one does: a moment of nothing, the short
                    // animation, and the game again. If it wasn't quit but fell over, that's said on the way: which also
                    // means one that falls over every time is a quarter of a minute and more between tries, not a blur.
                    game = None;
                    shown_dark = Instant::now() - Duration::from_secs(1);
                    played = false;
                    next = Some(Doing::Dark(if quit { a_moment() } else { try_again(&mut fell) }, Then::Boot { short: true, sorry: !quit }));
                }
            }
        }
        if let Some(next) = next {
            doing = next;
        }
        // With no one watching and nothing to show them, there's no hurry.
        std::thread::sleep(Duration::from_millis(if watched { 4 } else { 100 }));
    }
}
