//! The console coming on: its boot animation (pimpbox-boot, a program of its own that knows nothing of any game), shown on
//! the taskbar's screen while the game loads behind it. It's drawn by the graphics card on a thread of its own, read back,
//! and shrunk to the size the taskbar asks for, as the game's frames are; the picture as it was drawn is kept too, for the
//! window to show, and once there's a window it's drawn at the size of the console's video mode, as sharp as that is. It
//! waits for someone to be watching before it starts, since it only plays once, and then it plays to its end: nothing cuts
//! it short. There's a short one too, for when the console's only just been on; and that one can stop, once the mark's up,
//! for a notice that the game fell over (`sorry`), and carry on when the notice has gone.
//!
//! Its shader's kept compiled, in a folder the console's told of, so that it's only compiled the first time (or the first
//! time after that folder's been emptied). When it does have to be compiled, that's done before anything's shown, and every
//! part of the animation is drawn once to nowhere, so that nothing's left for the card to do while it's watched; and the
//! console shows its wheel meanwhile (`spinner`), to the end of whichever second that's done in. With no graphics card to draw it, there's
//! none.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pimpbox_boot::gpu::Gpu;
use pimpbox_boot::{FPS, H, LENGTH, SHORT, W};
use pms_screen::Screen;

/// A picture: which it is, how big, and its pixels.
type Kept = (u64, (usize, usize), Vec<u32>);

struct Shared {
    over: AtomicBool,
    /// The console's been switched off: it stops where it is.
    stop: AtomicBool,
    /// Its shader's being compiled: there's nothing of it to show yet, and the console shows its wheel.
    spinning: AtomicBool,
    /// Someone's opened the window: as good as watching.
    wanted: AtomicBool,
    /// The console's video mode: its width, and its height in the low half.
    mode: AtomicU64,
    /// The newest picture.
    picture: Mutex<Kept>,
}

/// The animation, playing or played.
pub struct Booting(Arc<Shared>);

impl Booting {
    /// Whether it's done.
    pub fn over(&self) -> bool {
        self.0.over.load(Ordering::Relaxed)
    }

    /// Whether it's still getting ready, with the console's wheel to be shown meanwhile.
    pub fn spinning(&self) -> bool {
        self.0.spinning.load(Ordering::Relaxed)
    }

    /// Stops it where it is: the console's been switched off, or has something else to do.
    pub fn stop(&self) {
        self.0.stop.store(true, Ordering::Relaxed);
    }

    /// Says someone wants to see it, whether or not the taskbar's showing it.
    pub fn want(&self) {
        self.0.wanted.store(true, Ordering::Relaxed);
    }

    /// Says how big the console's picture is, now.
    pub fn set_mode(&self, (w, h): (usize, usize)) {
        self.0.mode.store((w as u64) << 32 | h as u64, Ordering::Relaxed);
    }

    /// The newest picture into `out`, if there's one newer than `after`: which it is, and its size.
    pub fn latest(&self, after: u64, out: &mut Vec<u32>) -> Option<(u64, (usize, usize))> {
        let picture = self.0.picture.lock().ok()?;
        (picture.0 != after).then(|| {
            out.clone_from(&picture.2);
            (picture.0, picture.1)
        })
    }

}

/// How far into the short one the notice comes: the mark and the name are up, and nothing else has started.
const NOTICE_AT: f32 = 0.45;

/// It's been drawn once to nowhere since this process started.
static WARMED: AtomicBool = AtomicBool::new(false);

fn play(name: &str, short: bool, sorry: bool, kept: Option<&std::path::Path>, shared: &Shared) -> Result<(), String> {
    let screen = Screen::open(name)?;
    let getting_ready = Instant::now();
    let boot = pimpbox_boot::fonts::boot();
    // If its shader isn't kept compiled, it's compiled now, and the console shows its wheel while it is.
    let compiling = kept.is_some_and(|kept| !pimpbox_boot::gpu::is_kept(&boot, kept));
    shared.spinning.store(compiling, Ordering::Relaxed);
    let (mut gpu, took) = match kept {
        Some(kept) => Gpu::cached(boot, kept),
        None => Gpu::new(boot).map(|gpu| (gpu, None)),
    }
    .map_err(|e| e.to_string())?;
    if let (Some(took), Some(kept)) = (took, kept) {
        eprintln!("pms-pimpbox: compiled the boot animation's shader in {} ms, and kept it in {}", took.as_millis(), kept.display());
    }
    // Every part of it drawn once to nowhere, the first time, at the sizes it'll be shown at.
    if took.is_some() || !WARMED.swap(true, Ordering::Relaxed) {
        let began = Instant::now();
        let mode = shared.mode.load(Ordering::Relaxed);
        gpu.warm(&[(W, H), ((mode >> 32) as usize, (mode & 0xFFFF_FFFF) as usize)]).map_err(|e| e.to_string())?;
        eprintln!("pms-pimpbox: drew the boot animation through once, unseen, in {} ms", began.elapsed().as_millis());
    }
    // The wheel's shown to the end of the second it finished in: long enough to be seen for what it is.
    if compiling {
        let until = getting_ready.elapsed().as_secs_f32().ceil().max(1.0);
        while getting_ready.elapsed().as_secs_f32() < until && !shared.stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
        }
        shared.spinning.store(false, Ordering::Relaxed);
    }
    gpu.set_short(short);
    let length = if short { SHORT } else { LENGTH };
    // Made ready before anyone's kept waiting: the first frame is the slow one.
    let mut picture = vec![0u32; W * H];
    gpu.frame(0.0, &mut picture).map_err(|e| e.to_string())?;
    while !screen.watched_within(1500) && !shared.wanted.load(Ordering::Relaxed) {
        if shared.stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let began = Instant::now();
    let mut shrunk = Vec::new();
    let mut shown = 0u32;
    let mut notice = crate::sorry::Sorry::default();
    loop {
        // With a notice to give, it stops where the notice comes, for as long as that takes, and goes on from there.
        let gone = began.elapsed().as_secs_f32();
        let (t, noticed) = match gone - NOTICE_AT {
            since if sorry && since >= crate::sorry::TAKES => (gone - crate::sorry::TAKES, None),
            since if sorry && since >= 0.0 => (NOTICE_AT, Some(since)),
            _ => (gone, None),
        };
        if t >= length || shared.stop.load(Ordering::Relaxed) {
            break;
        }
        // As big as the console's picture is, once there's a window to show that in, and as wide as it is either way. The
        // taskbar gets as much of that as it's the shape for.
        let mode = shared.mode.load(Ordering::Relaxed);
        let mode = ((mode >> 32) as usize, (mode & 0xFFFF_FFFF) as usize);
        let size = if shared.wanted.load(Ordering::Relaxed) || mode.0 * 3 > mode.1 * 4 { mode } else { (W, H) };
        picture.resize(size.0 * size.1, 0);
        gpu.picture(t, size, &mut picture).map_err(|e| e.to_string())?;
        if let Some(since) = noticed {
            notice.over(&mut picture, size, since);
        }
        crate::show(&screen, &picture, size, size.0 as f32 / size.1 as f32, &mut shrunk);
        // At its own pace, which isn't the game's.
        shown += 1;
        if let Ok(mut kept) = shared.picture.lock() {
            kept.0 = u64::from(shown);
            kept.1 = size;
            kept.2.clone_from(&picture);
        }
        let next = Duration::from_secs_f64(f64::from(shown) / f64::from(FPS));
        if let Some(wait) = next.checked_sub(began.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    Ok(())
}

/// Starts it on the screen called `name`, which has to be there already: the short one, if `short`, and with the notice that
/// the game fell over in it, if `sorry`. `mode` is the size of the console's picture.
pub fn start(name: &str, mode: (usize, usize), short: bool, sorry: bool, kept: Option<std::path::PathBuf>) -> Booting {
    let shared = Arc::new(Shared { over: AtomicBool::new(false), stop: AtomicBool::new(false), spinning: AtomicBool::new(false), wanted: AtomicBool::new(false), mode: AtomicU64::new(0), picture: Mutex::new((0, (W, H), Vec::new())) });
    let booting = Booting(shared.clone());
    booting.set_mode(mode);
    let name = name.to_string();
    std::thread::spawn(move || {
        if let Err(e) = play(&name, short, short && sorry, kept.as_deref(), &shared) {
            shared.spinning.store(false, Ordering::Relaxed);
            eprintln!("pms-pimpbox: no boot animation: {e}");
        }
        shared.over.store(true, Ordering::Relaxed);
    });
    booting
}
