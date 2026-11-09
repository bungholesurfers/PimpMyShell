//! What a `screen` meter shows: a little TV. Each `screen` meter is a set of its own, drawn here in the daemon (pictures this
//! small don't need a process of their own) and handed over like any screen (pms-screen). A channel that holds still sends no
//! frames at all, so it costs nothing.
//!
//!   2      the test card
//!   3      the crab bouncing around, changing color, as a DVD player's logo does when left alone
//!   4      static
//!   5      the PimpCast (pimpcast.rs): IT GON RAIN!
//!   6      the console: Space Pimpin', or POOM to its friends (pms-poom), plugged in like a console of the day. It's a process
//!          of its own with a screen of its own, and any number of TVs can tune in. Changing the channel doesn't touch it: it
//!          has a power button of its own. With nobody watching it rests, and carries on when someone tunes back in. The channel
//!          is always there: asked for with no game to play, it opens the settings' TV page, where the games are. With a game and
//!          the console switched off it shows no signal, unless `console_autostart` says to switch the console on.
//!   1295   Captain Midnight: the message John MacDougall put over HBO's satellite feed in April 1986, from his teleport's
//!          uplink, about the price of scrambled TV. On the same bars, as it went out, tracking errors and all.
//!
//! Each set has a remote, in the tooltip (the TAP's remote.rs): channel up and down, shuffle, and power for the TV and the
//! console. Clicking the picture itself stops shuffling; after that, it picks up the controller and plays.
//!
//! Shuffling goes by a beat all the sets share, so that they can move together. With jitter, each is late by its own random amount,
//! up to a quarter of the beat. How long the beat is and whether there's jitter are for every set, whichever remote changes them.
//!
//! On battery a TV is asleep, holding its last picture and sending nothing, until it's touched.
//!
//! Turned off, a set stays where it is for a few seconds, then is put away: only a small button is left of it, drawn here like
//! everything else on it, which is what a click turns it back on with.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pms_screen::{BUTTONS, Button, Screen, Status};

/// Frames a second: on mains, on mains with the machine busy, and on battery while it's awake.
pub(crate) const FPS: u32 = 60;
pub(crate) const BUSY_FPS: u32 = 30;
const BATTERY_FPS: u32 = 10;
/// How long a TV stays awake on battery after it's touched.
const AWAKE: Duration = Duration::from_secs(30);
const MIDNIGHT_CHANNEL: u32 = 1295;
/// The console's channel.
pub const GAME: u32 = pms_screen::GAME_CHANNEL;
/// The screen the console draws on.
pub const CONSOLE: &str = "poom";
/// Every channel, in the order up goes through them.
pub const CHANNELS: [u32; 6] = [2, 3, 4, 5, GAME, MIDNIGHT_CHANNEL];
/// Where a set starting up begins its run through the channels, and where it ends and stays, unless the settings say otherwise.
pub const INTRO_FROM: u32 = MIDNIGHT_CHANNEL;
pub const INTRO_TO: u32 = GAME;
/// How long the run stays on each channel on its way: Captain Midnight gets long enough to be read.
const INTRO_MIDNIGHT_MS: i64 = 6000;
const INTRO_MS: i64 = 2000;
/// How long the channel-change snow lasts, how long the number stays up, and how long the picture takes to collapse when the
/// set's turned off, in milliseconds.
const SNOW: i64 = 170;
const NUMBER: i64 = 2500;
const COLLAPSE: i64 = 350;
/// How long a shuffling set stays on a channel, unless you've said otherwise, and what the remote steps through, in seconds.
pub const USUAL_SHUFFLE: u32 = 30;
const SHUFFLE_STEPS: [u32; 14] = [5, 10, 15, 20, 30, 45, 60, 90, 120, 300, 600, 900, 1800, 3600];
/// The most a set can be late, as a share of the beat.
const JITTER_SHARE: u64 = 4;
/// How long a set that's off stays full size before it's put away.
pub(crate) const STAYS_OFF: i64 = 7000;
/// The channels a set starting up runs through, and for how long, going up from `from` until it comes to `to`, which isn't in it:
/// that's where it stays. A channel there isn't is the usual one.
fn intro(from: u32, to: u32) -> Vec<(u32, i64)> {
    let at = |c: u32, usual: u32| CHANNELS.iter().position(|x| *x == c).or_else(|| CHANNELS.iter().position(|x| *x == usual)).unwrap_or(0);
    let (start, end) = (at(from, INTRO_FROM), at(to, INTRO_TO));
    let mut out = Vec::new();
    let mut i = start;
    while i != end {
        let channel = CHANNELS[i];
        out.push((channel, if channel == MIDNIGHT_CHANNEL { INTRO_MIDNIGHT_MS } else { INTRO_MS }));
        i = (i + 1) % CHANNELS.len();
    }
    out
}

/// What went out, wrapped to what fits across the screen.
const MIDNIGHT: [&str; 7] = ["GOODEVENING HBO", "FROM CAPTAIN", "MIDNIGHT", "$12.95/MONTH ?", "NO WAY !", "[SHOWTIME/MOVIE", "CHANNEL BEWARE!]"];

const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];
/// The number in the corner, as a set of the 1980s had it.
const OSD_GREEN: [u8; 3] = [64, 255, 96];

/// The bars, top to bottom, as fractions of the height: seven colors, their reverse, and the dark row.
const BARS: [[u8; 3]; 7] = [[192, 192, 192], [192, 192, 0], [0, 192, 192], [0, 192, 0], [192, 0, 192], [192, 0, 0], [0, 0, 192]];
const REVERSE: [[u8; 3]; 7] = [[0, 0, 192], [19, 19, 19], [192, 0, 192], [19, 19, 19], [0, 192, 192], [19, 19, 19], [192, 192, 192]];
/// The bottom row: each color, and where across the screen it ends.
const DARK: [([u8; 3], f32); 7] = [
    ([0, 33, 76], 0.179),
    ([255, 255, 255], 0.357),
    ([50, 0, 106], 0.536),
    ([19, 19, 19], 0.714),
    ([9, 9, 9], 0.762),
    ([19, 19, 19], 0.81),
    ([29, 29, 29], 0.857),
];

/// The colors a bouncing logo goes through.
const LOGO_COLORS: [[u8; 3]; 7] = [[255, 64, 129], [64, 160, 255], [255, 214, 0], [0, 230, 118], [255, 112, 0], [170, 90, 255], [255, 255, 255]];

const LOGO: &[u8] = include_bytes!("../../assets/icon/crab-512.png");
/// How many times finer than the screen the crab is kept: it's drawn from that, so its edges are smooth and it can stand between
/// pixels, gliding instead of stepping.
pub(crate) const FINE: usize = 4;

/// A pixel font, three wide and five tall: each row's bits, left one highest.
pub(crate) fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [7, 1, 7, 4, 7],
        '3' => [7, 1, 3, 1, 7],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 7, 1, 7],
        '6' => [7, 4, 7, 5, 7],
        '7' => [7, 1, 1, 2, 2],
        '8' => [7, 5, 7, 5, 7],
        '9' => [7, 5, 7, 1, 7],
        'A' => [2, 5, 7, 5, 5],
        'B' => [6, 5, 6, 5, 6],
        'C' => [3, 4, 4, 4, 3],
        'D' => [6, 5, 5, 5, 6],
        'E' => [7, 4, 6, 4, 7],
        'F' => [7, 4, 6, 4, 4],
        'G' => [3, 4, 5, 5, 3],
        'H' => [5, 5, 7, 5, 5],
        'I' => [7, 2, 2, 2, 7],
        'J' => [1, 1, 1, 5, 2],
        'K' => [5, 5, 6, 5, 5],
        'L' => [4, 4, 4, 4, 7],
        'M' => [5, 7, 7, 5, 5],
        'N' => [6, 5, 5, 5, 5],
        'O' => [2, 5, 5, 5, 2],
        'P' => [6, 5, 6, 4, 4],
        'Q' => [2, 5, 5, 6, 3],
        'R' => [6, 5, 6, 5, 5],
        'S' => [3, 4, 2, 1, 6],
        'T' => [7, 2, 2, 2, 2],
        'U' => [5, 5, 5, 5, 7],
        'V' => [5, 5, 5, 5, 2],
        'W' => [5, 5, 7, 7, 5],
        'X' => [5, 5, 2, 5, 5],
        'Y' => [5, 5, 2, 2, 2],
        'Z' => [7, 1, 2, 4, 7],
        '$' => [3, 6, 2, 3, 6],
        '.' => [0, 0, 0, 0, 2],
        ',' => [0, 0, 0, 2, 4],
        ':' => [0, 2, 0, 2, 0],
        '/' => [1, 1, 2, 4, 4],
        '?' => [6, 1, 2, 0, 2],
        '!' => [2, 2, 2, 0, 2],
        ';' => [0, 2, 0, 2, 4],
        '(' => [1, 2, 2, 2, 1],
        ')' => [4, 2, 2, 2, 4],
        '_' => [0, 0, 0, 0, 7],
        '-' => [0, 0, 7, 0, 0],
        '[' => [3, 2, 2, 2, 3],
        ']' => [6, 2, 2, 2, 6],
        _ => [0; 5],
    }
}

/// The logo's shape, at a height in screen pixels: how much it covers of each of the `FINE` by `FINE` dots of each pixel.
struct Shape {
    width: usize,
    height: usize,
    cover: Vec<u8>,
}

impl Shape {
    /// Draws it with its corner at (`x`, `y`), which needn't be on a pixel, over black.
    fn draw(&self, c: &mut Canvas, x: f32, y: f32, color: [u8; 3]) {
        let (fine_w, fine_h) = ((self.width * FINE) as i32, (self.height * FINE) as i32);
        let (fine, dots) = (FINE as i32, (FINE * FINE) as u32);
        let (left, top) = ((x * FINE as f32).round() as i32, (y * FINE as f32).round() as i32);
        for py in top / fine..=(top + fine_h) / fine {
            for px in left / fine..=(left + fine_w) / fine {
                let mut sum = 0u32;
                for dy in 0..fine {
                    for dx in 0..fine {
                        let (sx, sy) = (px * fine + dx - left, py * fine + dy - top);
                        if sx >= 0 && sy >= 0 && sx < fine_w && sy < fine_h {
                            sum += u32::from(self.cover[(sy * fine_w + sx) as usize]);
                        }
                    }
                }
                let k = sum / dots;
                if k > 0 && px >= 0 && py >= 0 {
                    c.set(px as usize, py as usize, color.map(|v| (u32::from(v) * k / 255) as u8));
                }
            }
        }
    }
}

fn logo_shape(height: usize) -> Option<Shape> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(LOGO)).read_info().ok()?;
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut pixels).ok()?;
    let (side, step) = (info.width as usize, info.line_size);
    let alpha = |x: usize, y: usize| pixels[y * step + x * 4 + 3] as u32;
    // Its dark lines stay out: the eyes and the smile are what make it a crab, in one color.
    let solid = |x: usize, y: usize| {
        let p = &pixels[y * step + x * 4..][..4];
        if (p[0] as u32 * 3 + p[1] as u32 * 6 + p[2] as u32) / 10 < 70 { 0 } else { p[3] as u32 }
    };
    // Only as wide as the logo really is.
    let (mut left, mut right, mut top, mut bottom) = (side, 0, side, 0);
    for y in 0..side {
        for x in 0..side {
            if alpha(x, y) > 32 {
                (left, right, top, bottom) = (left.min(x), right.max(x + 1), top.min(y), bottom.max(y + 1));
            }
        }
    }
    let (tall, wide) = (bottom.checked_sub(top)?, right.checked_sub(left)?);
    let width = (wide * height / tall.max(1)).max(1);
    let (fine_w, fine_h) = (width * FINE, height * FINE);
    let mut cover = Vec::with_capacity(fine_w * fine_h);
    for y in 0..fine_h {
        for x in 0..fine_w {
            let (y0, y1) = (top + y * tall / fine_h, top + ((y + 1) * tall / fine_h).max(y * tall / fine_h + 1));
            let (x0, x1) = (left + x * wide / fine_w, left + ((x + 1) * wide / fine_w).max(x * wide / fine_w + 1));
            let mut sum = 0;
            for sy in y0..y1.min(side) {
                for sx in x0..x1.min(side) {
                    sum += solid(sx, sy);
                }
            }
            cover.push((sum / ((y1 - y0) * (x1 - x0)).max(1) as u32) as u8);
        }
    }
    Some(Shape { width, height, cover })
}

/// A picture, BGRA, opaque.
pub(crate) struct Canvas<'a> {
    pub w: usize,
    pub h: usize,
    pub px: &'a mut [u8],
}

impl Canvas<'_> {
    pub fn set(&mut self, x: usize, y: usize, [r, g, b]: [u8; 3]) {
        if x < self.w && y < self.h {
            self.px[(y * self.w + x) * 4..][..4].copy_from_slice(&[b, g, r, 255]);
        }
    }

    pub fn fill(&mut self, c: [u8; 3]) {
        for y in 0..self.h {
            for x in 0..self.w {
                self.set(x, y, c);
            }
        }
    }

    /// `text` at `scale` pixels a dot, with a dark edge round it if `edged`, so that it reads on anything.
    pub fn text(&mut self, text: &str, x: usize, y: usize, scale: usize, color: [u8; 3], edged: bool) {
        for pass in usize::from(!edged)..2 {
            for (i, c) in text.chars().enumerate() {
                let rows = glyph(c);
                for (ry, row) in rows.iter().enumerate() {
                    for rx in 0..3 {
                        if row & (4 >> rx) == 0 {
                            continue;
                        }
                        let (dx, dy) = (x + (i * 4 + rx) * scale, y + ry * scale);
                        for sy in 0..scale {
                            for sx in 0..scale {
                                if pass == 0 {
                                    self.set(dx + sx + 1, dy + sy + 1, BLACK);
                                } else {
                                    self.set(dx + sx, dy + sy, color);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

pub(crate) fn text_width(text: &str, scale: usize) -> usize {
    (text.chars().count() * 4).saturating_sub(1) * scale
}

fn bars(c: &mut Canvas) {
    let (w, h) = (c.w, c.h);
    let (first, second) = (h * 2 / 3, h * 3 / 4);
    for y in 0..h {
        for x in 0..w {
            let across = x as f32 / w as f32;
            let bar = (x * 7 / w).min(6);
            let color = if y < first {
                BARS[bar]
            } else if y < second {
                REVERSE[bar]
            } else {
                DARK.iter().find(|(_, end)| across < *end).map_or([19, 19, 19], |(color, _)| *color)
            };
            c.set(x, y, color);
        }
    }
}

/// Whether the put-away button of a set covers a point, in the 0 to 1 square it's drawn in: an old set with its aerial, or a flat
/// one on its stand.
fn button_covers(u: f32, v: f32, modern: bool) -> bool {
    const LINE: f32 = 0.1;
    let inside = |l: f32, t: f32, r: f32, b: f32| u >= l && u < r && v >= t && v < b;
    let frame = |l: f32, t: f32, r: f32, b: f32| inside(l, t, r, b) && !inside(l + LINE, t + LINE, r - LINE, b - LINE);
    if modern {
        frame(0.0, 0.16, 1.0, 0.72) || inside(0.44, 0.72, 0.56, 0.84) || inside(0.24, 0.84, 0.76, 0.84 + LINE)
    } else {
        let aerial = v < 0.36 && ((u - 0.5).abs() - (0.36 - v) * 0.75).abs() < LINE * 0.55;
        frame(0.08, 0.36, 0.92, 0.96) || aerial
    }
}

/// Why the console has to go, given why it might: a game that's been played stays until it's restarted, as closing it could lose it.
fn console_evicted(short: Option<&'static str>, running: bool, played: bool) -> Option<&'static str> {
    short.filter(|_| !(running && played))
}

/// A very sad little set, over whatever's up, dimmed, with why it's been sent away beneath it. Original art: dots, one row a string.
pub(crate) fn evicted(c: &mut Canvas, why: &str) {
    const SAD: [&str; 12] = [
        "....#....#....",
        ".....#..#.....",
        "##############",
        "#............#",
        "#..##....##..#",
        "#............#",
        "#............#",
        "#....####....#",
        "#...#....#...#",
        "#............#",
        "##############",
        "...##....##...",
    ];
    const GRAY: [u8; 3] = [214, 216, 224];
    const WORDS: [u8; 3] = [170, 174, 186];
    for dot in c.px.as_chunks_mut::<4>().0 {
        for v in &mut dot[..3] {
            *v /= 3;
        }
    }
    let s = (c.h / 36).max(1);
    let tall = (SAD.len() + 1 + 5 + 1 + 5) * s;
    let top = c.h.saturating_sub(tall) / 2;
    let left = c.w.saturating_sub(SAD[0].len() * s) / 2;
    for (y, row) in SAD.iter().enumerate() {
        for (x, _) in row.bytes().enumerate().filter(|(_, b)| *b == b'#') {
            for dy in 0..s {
                for dx in 0..s {
                    c.set(left + x * s + dx, top + y * s + dy, GRAY);
                }
            }
        }
    }
    let lines = ["EVICTED:", why];
    for (i, line) in lines.iter().enumerate() {
        c.text(line, c.w.saturating_sub(text_width(line, s)) / 2, top + (SAD.len() + 1 + i * 6) * s, s, WORDS, false);
    }
}

/// What's left of a set put away: a small gray outline with nothing behind it, for any taskbar. A set that's only on standby, and
/// comes back on by itself, has it in a quiet yellow, as a set's standby light is.
pub(crate) fn put_away(c: &mut Canvas, modern: bool, standby: bool) {
    const LOOKS: usize = 4;
    const STRENGTH: f32 = 0.65;
    let color: [f32; 3] = if standby { [214.0, 182.0, 92.0] } else { [176.0, 176.0, 176.0] };
    c.px.fill(0);
    let side = (c.w.min(c.h) as f32 * 0.72).max(1.0);
    let (left, top) = ((c.w as f32 - side) / 2.0, (c.h as f32 - side) / 2.0);
    for y in 0..c.h {
        for x in 0..c.w {
            let mut hits = 0;
            for dy in 0..LOOKS {
                for dx in 0..LOOKS {
                    let at = |p: usize, d: usize, start: f32| (p as f32 + (d as f32 + 0.5) / LOOKS as f32 - start) / side;
                    hits += usize::from(button_covers(at(x, dx, left), at(y, dy, top), modern));
                }
            }
            if hits > 0 {
                let alpha = hits as f32 / (LOOKS * LOOKS) as f32 * STRENGTH;
                let [r, g, b] = color.map(|v| (v * alpha) as u8);
                c.px[(y * c.w + x) * 4..][..4].copy_from_slice(&[b, g, r, (alpha * 255.0) as u8]);
            }
        }
    }
}

/// Adds one more TV, after the last one there is.
pub fn add_tv(config: &mut crate::config::Config) {
    let is_tv = |m: &crate::config::MeterSpec| m.style == crate::config::Style::Screen && m.set.as_deref() != Some("twatch");
    let have: Vec<&String> = config.meters.iter().filter(|m| is_tv(m)).map(|m| &m.label).collect();
    let label = (2..).map(|n| format!("TV {n}")).find(|l| !have.contains(&l)).unwrap_or_default();
    let Some(set) = crate::config::parse(&format!("[[meter]]\nlabel = \"{label}\"\nstyle = \"screen\"")).ok().and_then(|c| c.meters.into_iter().next()) else { return };
    let at = config.meters.iter().rposition(is_tv).map_or(config.meters.len(), |i| i + 1);
    config.meters.insert(at, set);
}

/// Whether the console is running: taking the last set away would end it.
pub fn console_running() -> bool {
    Screen::open(CONSOLE).is_ok()
}

/// Takes the TV called `name` off the taskbar.
pub fn nuke_tv(config: &mut crate::config::Config, name: &str) {
    config.meters.retain(|m| !(m.style == crate::config::Style::Screen && m.set.as_deref() != Some("twatch") && m.label == name));
}

/// The next step up or down from `seconds`, wherever that is between them.
pub fn shuffle_step(seconds: u32, later: bool) -> u32 {
    let next = if later { SHUFFLE_STEPS.iter().find(|s| **s > seconds) } else { SHUFFLE_STEPS.iter().rev().find(|s| **s < seconds) };
    next.copied().unwrap_or(if later { SHUFFLE_STEPS[SHUFFLE_STEPS.len() - 1] } else { SHUFFLE_STEPS[0] })
}

/// What the remote's little display says for `seconds`.
fn shuffle_words(seconds: u32) -> String {
    match (seconds / 60, seconds % 60) {
        (0, s) => format!("{s} S"),
        (60, 0) => "1 HR".to_string(),
        (m, 0) => format!("{m} MIN"),
        (m, s) => format!("{m}:{s:02}"),
    }
}

/// What a click on the picture came to.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Clicked {
    /// The set dealt with it.
    Done,
    /// Pick up the controller.
    Play,
    /// It's on the console's channel, and there's no game.
    NoGame,
}

/// What a set keeps between runs.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Kept {
    pub channel: u32,
    pub shuffle: bool,
    pub on: bool,
}

impl Default for Kept {
    fn default() -> Self {
        Kept { channel: 2, shuffle: true, on: true }
    }
}

/// The TV: which channel it's on, and for how long.
pub struct Tv {
    channel: usize,
    on: bool,
    shuffle: bool,
    /// The channels still to run through on switching on, and how long each has.
    intro: Vec<(u32, i64)>,
    in_intro: bool,
    /// Where the run ends.
    intro_to: u32,
    has_game: bool,
    /// Shuffling may land on the console: there's something on it, or would be.
    console_ready: bool,
    left: i64,
    snow: i64,
    number: i64,
    collapse: i64,
    /// How much longer it stays full size, being off, and whether it's been put away.
    stays: i64,
    small: bool,
    /// A word in place of the number, for a while.
    word: Option<&'static str>,
    seed: u32,
    tape: u32,
    logo: (f32, f32),
    heading: (f32, f32),
    color: usize,
    shape: Option<Shape>,
    drawn: Option<(u32, usize, usize)>,
    card: Vec<u8>,
    card_for: Option<(usize, usize, bool)>,
    /// The console's newest frame shown.
    shown: u64,
    cast: crate::pimpcast::PimpCast,
    /// The time on the clock the sets share, and how long a beat is by it, in milliseconds.
    now: u64,
    every: u64,
    jitter: bool,
    /// The beat it's in, if it's been shuffling since the last one, and when in this beat it moves, if it hasn't yet.
    beat: Option<u64>,
    moves_at: Option<u64>,
    /// Why the console has been sent away, if it has.
    pub evicted: Option<&'static str>,
}

impl Tv {
    pub fn new(seed: u32, kept: Kept, has_game: bool) -> Tv {
        let mut tv = Tv {
            channel: CHANNELS.iter().position(|c| *c == kept.channel).unwrap_or(0),
            on: kept.on,
            shuffle: kept.shuffle,
            intro: Vec::new(),
            in_intro: false,
            intro_to: INTRO_TO,
            has_game,
            console_ready: has_game,
            left: 0,
            snow: 0,
            number: 0,
            collapse: 0,
            stays: 0,
            small: !kept.on,
            word: None,
            seed: seed | 1,
            tape: 0,
            logo: (3.0, 2.0),
            heading: (1.0, 1.0),
            color: 0,
            shape: None,
            drawn: None,
            evicted: None,
            card: Vec::new(),
            card_for: None,
            shown: 0,
            cast: Default::default(),
            now: 0,
            every: u64::from(USUAL_SHUFFLE) * 1000,
            jitter: true,
            beat: None,
            moves_at: None,
        };
        tv.tuned();
        tv
    }

    /// Starting up: up through the channels from `from` to `to`, where it stays, whatever's in the console.
    pub fn start_intro(&mut self, from: u32, to: u32) {
        if !self.on {
            return;
        }
        self.shuffle = false;
        self.intro_to = if CHANNELS.contains(&to) { to } else { INTRO_TO };
        self.intro = intro(from, to).into_iter().rev().collect();
        self.in_intro = true;
        self.next_of_intro();
    }

    fn next_of_intro(&mut self) {
        match self.intro.pop() {
            Some((channel, ms)) => {
                self.tune(channel);
                self.left = ms;
            }
            None => {
                self.in_intro = false;
                self.tune(self.intro_to);
            }
        }
    }

    pub fn kept(&self) -> Kept {
        Kept { channel: CHANNELS[self.channel], shuffle: self.shuffle, on: self.on }
    }

    pub fn status(&self, game_on: bool) -> Status {
        Status { channel: CHANNELS[self.channel], tv_on: self.on, shuffle: self.shuffle, has_game: self.has_game, game_on, collapsed: self.small, ..Status::default() }
    }

    /// A game came or went, or the console did: `ready` says whether shuffling may go there. A set already there stays, with no
    /// signal, until something moves it.
    pub fn set_game(&mut self, has_game: bool, ready: bool) {
        (self.has_game, self.console_ready) = (has_game, ready);
    }

    /// Whether it's on, and on the console's channel.
    pub fn on_console(&self) -> bool {
        self.on && CHANNELS[self.channel] == GAME
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    fn random(&mut self, below: u32) -> u32 {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x >> 8) % below.max(1)
    }

    /// What time it is, how long a beat is, and whether each set goes a little late: for the next frame.
    pub fn set_clock(&mut self, now: u64, every: u64, jitter: bool) {
        self.now = now;
        if every != self.every {
            self.every = every.max(1);
            self.not_on_the_beat();
        }
        self.jitter = jitter;
    }

    /// Whatever it was waiting for, it starts with the next beat that comes.
    fn not_on_the_beat(&mut self) {
        self.beat = None;
        self.moves_at = None;
    }

    /// Whether it's time to move on. A new beat says when; with no jitter, that's right away, and at once on every set.
    fn beat_says_go(&mut self) -> bool {
        let beat = self.now / self.every;
        if self.beat != Some(beat) {
            let late = if self.jitter { u64::from(self.random((self.every / JITTER_SHARE + 1) as u32)) } else { 0 };
            self.moves_at = self.beat.is_some().then_some(beat * self.every + late);
            self.beat = Some(beat);
        }
        let go = self.moves_at.is_some_and(|at| self.now >= at);
        if go {
            self.moves_at = None;
        }
        go
    }

    /// The channel just changed.
    fn tuned(&mut self) {
        self.snow = SNOW;
        self.number = NUMBER;
        self.word = None;
        self.drawn = None;
        self.shown = 0;
        self.cast.restart();
    }

    fn tune(&mut self, channel: u32) {
        self.channel = CHANNELS.iter().position(|c| *c == channel).unwrap_or(0);
        self.tuned();
    }

    /// A channel picked from the list: it stays there.
    pub fn pick(&mut self, channel: u32) {
        if self.on && CHANNELS.contains(&channel) {
            self.intro.clear();
            self.in_intro = false;
            self.shuffle = false;
            self.tune(channel);
        }
    }

    fn step(&mut self, by: isize) {
        self.channel = (self.channel as isize + by).rem_euclid(CHANNELS.len() as isize) as usize;
        self.tuned();
    }

    /// A press on the remote.
    pub fn press(&mut self, button: Button) {
        self.intro.clear();
        self.in_intro = false;
        match button {
            Button::TvPower => {
                self.on = !self.on;
                self.collapse = if self.on { 0 } else { COLLAPSE };
                self.stays = STAYS_OFF;
                self.small = false;
                self.drawn = None;
                if self.on {
                    self.tuned();
                }
            }
            _ if !self.on => {}
            Button::ChannelUp => self.step(1),
            Button::ChannelDown => self.step(-1),
            Button::Shuffle => {
                self.shuffle = !self.shuffle;
                self.word = Some(if self.shuffle { "SHUFFLE" } else { "HOLD" });
                self.number = NUMBER;
                self.drawn = None;
            }
            Button::GamePower
            | Button::Mute
            | Button::AddTv
            | Button::Follow
            | Button::Stay
            | Button::Only
            | Button::QualityUp
            | Button::QualityDown
            | Button::ShuffleSooner
            | Button::ShuffleLater
            | Button::Jitter
            | Button::NukeTv
            | Button::VolumeUp
            | Button::VolumeDown
            | Button::AddFavorite => {}
            Button::GameChannel => {
                self.shuffle = false;
                self.tune(GAME);
            }
        }
    }

    /// A click on the picture. Off, it turns it on. Shuffling, it stops that. After that, it's the console, if there's a game
    /// running. Without one it's the next channel, or, on the console's own with no game at all, the news that there's none.
    pub fn click(&mut self, game_on: bool) -> Clicked {
        if !self.on {
            self.press(Button::TvPower);
            return Clicked::Done;
        }
        if self.in_intro {
            self.intro.clear();
            self.in_intro = false;
            self.tune(self.intro_to);
            return Clicked::Done;
        }
        if self.shuffle {
            self.press(Button::Shuffle);
            return Clicked::Done;
        }
        if self.has_game && game_on {
            if CHANNELS[self.channel] != GAME {
                self.tune(GAME);
            }
            return Clicked::Play;
        }
        if CHANNELS[self.channel] == GAME && !self.has_game {
            return Clicked::NoGame;
        }
        self.step(1);
        Clicked::Done
    }

    /// Somewhere else. Captain Midnight is an event: it's rarer than the others.
    fn wander(&mut self) {
        let last = CHANNELS.len() - 1;
        loop {
            let pick = if self.random(6) == 0 { last } else { self.random(last as u32) as usize };
            if pick != self.channel && (CHANNELS[pick] != GAME || self.console_ready) {
                self.channel = pick;
                break;
            }
        }
        self.tuned();
    }

    fn snow(&mut self, c: &mut Canvas) {
        for y in 0..c.h {
            for x in 0..c.w {
                let v = (self.random(256)) as u8;
                c.set(x, y, [v, v, v]);
            }
        }
    }

    fn bounce(&mut self, c: &mut Canvas, dt: i64) {
        c.fill(BLACK);
        let tall = (c.h * 2 / 5).max(4);
        if self.shape.as_ref().is_none_or(|s| s.height != tall) {
            self.shape = logo_shape(tall);
        }
        let Some(shape) = &self.shape else { return };
        // Pixels a millisecond: across the screen in about five seconds.
        let speed = c.h as f32 / 48.0 * 0.012 * dt as f32;
        let (room_x, room_y) = ((c.w.saturating_sub(shape.width)) as f32, (c.h.saturating_sub(shape.height)) as f32);
        let (mut x, mut y) = (self.logo.0 + self.heading.0 * speed, self.logo.1 + self.heading.1 * speed * 0.7);
        let mut hit = false;
        if x <= 0.0 || x >= room_x {
            self.heading.0 = -self.heading.0;
            x = x.clamp(0.0, room_x);
            hit = true;
        }
        if y <= 0.0 || y >= room_y {
            self.heading.1 = -self.heading.1;
            y = y.clamp(0.0, room_y);
            hit = true;
        }
        self.logo = (x, y);
        if hit {
            self.color = (self.color + 1) % LOGO_COLORS.len();
        }
        shape.draw(c, x, y, LOGO_COLORS[self.color]);
    }

    /// The card with the message on it, kept: only the tape's faults change from frame to frame.
    fn midnight_card(&mut self, w: usize, h: usize, message: bool) {
        if self.card_for == Some((w, h, message)) {
            return;
        }
        self.card_for = Some((w, h, message));
        self.card.resize(w * h * 4, 0);
        let mut c = Canvas { w, h, px: &mut self.card };
        bars(&mut c);
        if !message {
            return;
        }
        let scale = (h / 48).max(1);
        let pitch = h / MIDNIGHT.len();
        for (i, line) in MIDNIGHT.iter().enumerate() {
            let top = i * pitch + (pitch.saturating_sub(5 * scale)) / 2;
            // White on a dimmed band: at this size an outline would fill the gaps between the letters.
            for y in top.saturating_sub(1)..(top + 5 * scale + 1).min(h) {
                for at in (y * w * 4..(y + 1) * w * 4).step_by(4) {
                    for v in &mut c.px[at..at + 3] {
                        *v /= 2;
                    }
                }
            }
            let x = w.saturating_sub(text_width(line, scale)) / 2;
            c.text(line, x, top, scale, WHITE, false);
        }
    }

    /// On a tape: a row slips sideways now and then, and a band of noise rolls up the picture. The message comes up once
    /// the channel's number has gone.
    fn midnight(&mut self, c: &mut Canvas, message: bool, dt: i64) {
        let (w, h) = (c.w, c.h);
        self.midnight_card(w, h, message);
        self.tape = self.tape.wrapping_add(dt as u32);
        let band = h - 1 - (self.tape as usize / 130) % h;
        c.px.copy_from_slice(&self.card);
        for y in 0..h {
            // Rarely: letters here are five rows tall, and a row out of place is a letter lost.
            if self.random(300) == 0 {
                c.px[y * w * 4..(y + 1) * w * 4].rotate_left(4);
            }
            if y == band {
                for x in 0..w {
                    if self.random(4) == 0 {
                        let v = self.random(256) as u8;
                        c.set(x, y, [v, v, v]);
                    }
                }
            }
        }
    }

    fn no_signal(c: &mut Canvas) {
        c.fill([0, 0, 170]);
        let scale = (c.h / 48).max(1);
        for (i, word) in ["NO", "SIGNAL"].iter().enumerate() {
            let x = c.w.saturating_sub(text_width(word, scale)) / 2;
            c.text(word, x, (c.h / 2).saturating_sub(7 * scale) + i * 8 * scale, scale, WHITE, false);
        }
    }

    /// The set going off: the picture collapses to a line, then a dot, as a tube's did.
    fn collapsing(&self, c: &mut Canvas) {
        c.fill(BLACK);
        let k = self.collapse as f32 / COLLAPSE as f32;
        let (cx, cy) = (c.w / 2, c.h / 2);
        let half = ((c.w as f32 / 2.0) * k.min(0.6) / 0.6) as usize;
        for x in cx.saturating_sub(half)..(cx + half + 1).min(c.w) {
            c.set(x, cy, WHITE);
        }
    }

    /// Draws the next frame into `px`, if there's anything new, `dt` milliseconds after the last. `game` is the console's
    /// screen, if it's on.
    pub fn frame(&mut self, dt: i64, w: usize, h: usize, px: &mut [u8], game: Option<&Screen>) -> bool {
        let mut c = Canvas { w, h, px };
        if !self.on {
            self.not_on_the_beat();
            if self.collapse > 0 {
                self.collapse -= dt;
                self.collapsing(&mut c);
                return true;
            }
            self.stays -= dt;
            self.small |= self.stays <= 0;
            let state = (u32::from(self.small), w, h);
            if self.drawn == Some(state) {
                return false;
            }
            self.drawn = Some(state);
            if self.small {
                put_away(&mut c, false, false);
            } else {
                c.fill(BLACK);
            }
            return true;
        }
        self.left -= dt;
        if self.in_intro {
            if self.left <= 0 {
                self.next_of_intro();
            }
        } else if !self.shuffle {
            self.not_on_the_beat();
        } else if self.beat_says_go() {
            self.wander();
        }
        let number_up = self.number > 0;
        self.number -= dt;
        let channel = CHANNELS[self.channel];
        if self.snow > 0 {
            self.snow -= dt;
            self.snow(&mut c);
            self.drawn = None;
        } else {
            match channel {
                2 => {
                    // It holds still: only the number going away, or a new size, changes it.
                    let state = (u32::from(number_up), w, h);
                    if self.drawn == Some(state) {
                        return false;
                    }
                    self.drawn = Some(state);
                    bars(&mut c);
                }
                3 => self.bounce(&mut c, dt),
                4 => self.snow(&mut c),
                5 => {
                    if !self.cast.frame(dt, &mut c) && !number_up {
                        return false;
                    }
                }
                GAME => match game {
                    Some(screen) => {
                        screen.watch();
                        screen.want(w as u32, h as u32);
                        match screen.latest(self.shown, w as u32, h as u32, c.px) {
                            Some(n) => self.shown = n,
                            None if self.shown != 0 && !number_up => return false,
                            None if self.shown == 0 => Self::no_signal(&mut c),
                            None => {}
                        }
                    }
                    None => {
                        let state = (u32::from(number_up) + if self.evicted.is_some() { 4 } else { 2 }, w, h);
                        if self.drawn == Some(state) {
                            return false;
                        }
                        self.drawn = Some(state);
                        match self.evicted {
                            Some(why) => {
                                c.fill(BLACK);
                                evicted(&mut c, why);
                            }
                            None => Self::no_signal(&mut c),
                        }
                    }
                },
                _ => self.midnight(&mut c, !number_up, dt),
            }
        }
        if number_up {
            let scale = if self.word.is_some() { (h / 48).max(1) } else { (h / 24).max(1) };
            let text = self.word.map_or_else(|| channel.to_string(), str::to_string);
            let x = w.saturating_sub(text_width(&text, scale) + 2 * scale);
            c.text(&text, x, scale, scale, OSD_GREEN, true);
        }
        true
    }
}

/// What the sets share with what looks after them.
#[derive(Default)]
struct Shared {
    console: Mutex<Option<Arc<Screen>>>,
    game_pid: std::sync::atomic::AtomicU32,
    has_game: AtomicBool,
    game_on: AtomicBool,
    battery: AtomicBool,
    /// The machine's busy: frames can wait.
    busy: AtomicBool,
    /// Nothing's to be shown: the taskbar isn't ours just now.
    resting: AtomicBool,
    /// Why the console has been sent away, if it has.
    console_evicted: Mutex<Option<&'static str>>,
    /// Somebody's picked up the controller since the game last started.
    played: AtomicBool,
    /// A set landing on the console's channel switches it on.
    autostart: AtomicBool,
    /// A set starting up runs through its channels, and from where to where.
    intro: AtomicBool,
    intro_from: std::sync::atomic::AtomicU32,
    intro_to: std::sync::atomic::AtomicU32,
    /// A remote has asked for one more set.
    add_tv: AtomicBool,
    /// The sets remotes have asked to see the back of.
    nuked: Mutex<Vec<String>>,
    /// How many sets there are: the console only runs while there's one.
    sets: AtomicUsize,
    /// What the game that's in the console is called.
    game_title: Mutex<String>,
    /// How long a beat is, in seconds, whether there's jitter, and whether a remote has changed either and that's yet to be saved.
    shuffle_seconds: AtomicU32,
    jitter: AtomicBool,
    shuffle_changed: AtomicBool,
    /// The settings file, for opening the settings on.
    config: Mutex<PathBuf>,
    state: Mutex<HashMap<String, Kept>>,
}

impl Shared {
    fn console(&self) -> Option<Arc<Screen>> {
        self.console.lock().ok()?.clone()
    }

    fn save(&self) {
        let Ok(state) = self.state.lock() else { return };
        let _ = std::fs::write(state_file(), state_text(&state, self.game_on.load(Ordering::Relaxed)));
    }
}

fn state_file() -> PathBuf {
    crate::config::beside_config("tv.txt")
}

/// Opens the settings on the TV page, in front: a click got us here, and passes that on.
fn open_settings(config: &Path) {
    use windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;
    let Ok(me) = std::env::current_exe() else { return };
    match std::process::Command::new(me.with_file_name("pms-gui.exe")).arg("--page=poom").arg(config).spawn() {
        Ok(gui) => {
            let _ = unsafe { AllowSetForegroundWindow(gui.id()) };
        }
        Err(e) => crate::log(format!("TV: couldn't open the settings: {e}")),
    }
}

fn state_text(sets: &HashMap<String, Kept>, game_on: bool) -> String {
    let mut lines: Vec<String> = sets.iter().map(|(name, k)| format!("{}\t{}\t{}\t{name}", k.channel, u8::from(k.shuffle), u8::from(k.on))).collect();
    lines.sort();
    lines.insert(0, format!("console\t{}", u8::from(game_on)));
    lines.join("\n") + "\n"
}

fn parse_state(text: &str) -> (HashMap<String, Kept>, bool) {
    let (mut sets, mut game_on) = (HashMap::new(), true);
    for line in text.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        match parts.as_slice() {
            ["console", on] => game_on = *on != "0",
            [channel, shuffle, on, name] => {
                if let Ok(channel) = channel.parse() {
                    sets.insert(name.to_string(), Kept { channel, shuffle: *shuffle != "0", on: *on != "0" });
                }
            }
            _ => {}
        }
    }
    (sets, game_on)
}

fn run(name: &str, stop: &AtomicBool, shared: &Shared) {
    let screen = match Screen::create(name) {
        Ok(s) => s,
        Err(e) => {
            crate::log(format!("TV `{name}`: {e}"));
            return;
        }
    };
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos());
    let kept = shared.state.lock().ok().and_then(|s| s.get(name).copied()).unwrap_or_default();
    let mut tv = Tv::new(seed, kept, shared.has_game.load(Ordering::Relaxed));
    if shared.intro.load(Ordering::Relaxed) {
        tv.start_intro(shared.intro_from.load(Ordering::Relaxed), shared.intro_to.load(Ordering::Relaxed));
    }
    let mut clicks = screen.clicks();
    let mut pressed = BUTTONS.map(|b| screen.presses(b));
    let mut px = Vec::new();
    let (mut last, mut touched) = (Instant::now(), Instant::now());
    // Left there is not arriving: it mustn't undo switching the console off, then or at the next start.
    let mut on_console = tv.on_console();
    // How long a beat its remote's been told it is, and whether nuking this set would end the game.
    let mut said = String::new();
    while !stop.load(Ordering::Relaxed) {
        let began = Instant::now();
        let (has_game, game_on, battery) = (shared.has_game.load(Ordering::Relaxed), shared.game_on.load(Ordering::Relaxed), shared.battery.load(Ordering::Relaxed));
        let autostart = shared.autostart.load(Ordering::Relaxed);
        tv.set_game(has_game, has_game && (game_on || autostart));
        tv.evicted = shared.console_evicted.lock().ok().and_then(|e| *e);
        let mut changed = false;
        // The console's been asked for by name, as opposed to come across: with no game, that's the way to the games.
        let mut asked = false;
        for (i, button) in BUTTONS.iter().enumerate() {
            let now = screen.presses(*button);
            if now != pressed[i] {
                pressed[i] = now;
                changed = true;
                match button {
                    Button::GamePower if has_game => shared.game_on.store(!game_on, Ordering::Relaxed),
                    Button::GamePower | Button::GameChannel => asked = true,
                    Button::AddTv => shared.add_tv.store(true, Ordering::Relaxed),
                    Button::NukeTv => {
                        if let Ok(mut nuked) = shared.nuked.lock() {
                            nuked.push(name.to_string());
                        }
                    }
                    Button::ShuffleSooner | Button::ShuffleLater => {
                        let seconds = shared.shuffle_seconds.load(Ordering::Relaxed);
                        shared.shuffle_seconds.store(shuffle_step(seconds, *button == Button::ShuffleLater), Ordering::Relaxed);
                        shared.shuffle_changed.store(true, Ordering::Relaxed);
                    }
                    Button::Jitter => {
                        shared.jitter.fetch_not(Ordering::Relaxed);
                        shared.shuffle_changed.store(true, Ordering::Relaxed);
                    }
                    _ => {}
                }
                tv.press(*button);
            }
        }
        if let Some(channel) = screen.take_tune() {
            tv.pick(channel);
            asked |= channel == GAME;
            changed = true;
        }
        let console = shared.console().filter(|_| game_on);
        if screen.clicks() != clicks {
            clicks = screen.clicks();
            changed = true;
            match tv.click(game_on) {
                Clicked::Play => {
                    if let Some(console) = &console {
                        console.click();
                        shared.played.store(true, Ordering::Relaxed);
                    }
                }
                Clicked::NoGame => asked = true,
                Clicked::Done => {}
            }
        }
        // However it got to the console's channel, arriving switches the console on, if that's what's wanted.
        let arrived = tv.on_console() && !on_console;
        on_console = tv.on_console();
        let running = has_game && shared.game_on.load(Ordering::Relaxed);
        if !running && has_game && autostart && arrived {
            shared.game_on.store(true, Ordering::Relaxed);
            changed = true;
        } else if !has_game
            && asked
            && tv.is_on()
            && let Ok(config) = shared.config.lock()
        {
            open_settings(&config);
        }
        if changed {
            touched = Instant::now();
            if let Ok(mut state) = shared.state.lock() {
                state.insert(name.to_string(), tv.kept());
            }
            shared.save();
        }
        let (seconds, jitter) = (shared.shuffle_seconds.load(Ordering::Relaxed), shared.jitter.load(Ordering::Relaxed));
        tv.set_clock(pms_screen::now_ms(), u64::from(seconds.max(1)) * 1000, jitter);
        let alone = shared.sets.load(Ordering::Relaxed) == 1 && shared.game_pid.load(Ordering::Relaxed) != 0;
        // And, for the console's own tooltip, what's in it and why it's gone, if it has.
        let title = shared.game_title.lock().map(|t| t.clone()).unwrap_or_default();
        let info = format!("every\t{}\nalone\t{}\ngame\t{title}\nevicted\t{}\n", shuffle_words(seconds), u8::from(alone), tv.evicted.unwrap_or_default());
        if said != info {
            screen.set_info(&info);
            said = info;
        }
        screen.set_status(Status { jitter, ..tv.status(shared.game_on.load(Ordering::Relaxed)) });
        screen.set_also(shared.game_pid.load(Ordering::Relaxed));
        // Nothing to draw for until something's showing it; and on battery, a set left alone sleeps on its last picture.
        let asleep = shared.resting.load(Ordering::Relaxed) || (battery && touched.elapsed() > AWAKE);
        let Some((w, h)) = screen.wanted().filter(|_| !asleep) else {
            last = Instant::now();
            std::thread::sleep(Duration::from_millis(250));
            continue;
        };
        let dt = last.elapsed().as_millis().min(250) as i64;
        last = Instant::now();
        px.resize((w * h * 4) as usize, 0);
        if tv.frame(dt, w as usize, h as usize, &mut px, console.as_deref()) {
            screen.publish(w, h, &px);
        }
        let rate = if battery {
            BATTERY_FPS
        } else if shared.busy.load(Ordering::Relaxed) {
            BUSY_FPS
        } else {
            FPS
        };
        std::thread::sleep(Duration::from_secs(1).div_f64(f64::from(rate)).saturating_sub(began.elapsed()));
    }
}

/// The sets, one for each `screen` meter, and the console behind them.
#[derive(Default)]
pub struct Screens {
    sets: Vec<(String, JoinHandle<()>, Arc<AtomicBool>)>,
    shared: Arc<Shared>,
    game: Option<std::process::Child>,
    /// What's in the console.
    playing: Option<std::path::PathBuf>,
    loaded: bool,
    load: crate::load::Load,
    /// When the game ended by itself lately: one that keeps falling over stays off.
    falls: Vec<Instant>,
    twatch: crate::twatch::Group,
    /// The console is staying though memory is short, and whether that's yet to be said.
    spared: bool,
    spared_news: bool,
    /// The last lines the game wrote to its stderr: a panic says why there, and nowhere else.
    game_said: Arc<Mutex<std::collections::VecDeque<String>>>,
    /// The shuffling the settings last said.
    shuffling: Option<(u32, bool)>,
}

/// What the remotes have asked for that goes in the settings.
#[derive(Default, PartialEq, Debug)]
pub struct Asked {
    pub add_tv: bool,
    /// The TVs to take off the taskbar.
    pub nuked: Vec<String>,
    /// How long a beat is now, and whether there's jitter.
    pub shuffling: Option<(u32, bool)>,
}

impl Screens {
    fn stop_game(&mut self) {
        if let Ok(mut console) = self.shared.console.lock() {
            *console = None;
        }
        self.shared.game_pid.store(0, Ordering::Relaxed);
        if let Some(mut game) = self.game.take() {
            // An instrumented build (PGO training) lets it go instead: it goes with the daemon, ending as a program does, which
            // is when it writes what it recorded.
            if cfg!(pms_pgo) {
                return drop(game);
            }
            let _ = game.kill();
            let _ = game.wait();
        }
    }

    pub fn stop(&mut self) {
        for (_, thread, stop) in self.sets.drain(..) {
            stop.store(true, Ordering::Relaxed);
            let _ = thread.join();
        }
        self.twatch.stop();
        self.stop_game();
        self.shared.save();
    }

    fn start_game(&mut self, wad: &Path) {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let Ok(me) = std::env::current_exe() else { return };
        let exe = me.with_file_name(crate::parts::POOM);
        let saves = crate::wads::saves();
        if !exe.is_file() {
            return crate::parts::found_missing(crate::parts::POOM);
        }
        if std::fs::create_dir_all(&saves).is_err() {
            return;
        }
        let spawned = std::process::Command::new(&exe)
            .arg(wad)
            .arg(CONSOLE)
            .arg(std::process::id().to_string())
            .current_dir(&saves)
            .stderr(std::process::Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        match spawned {
            Ok(mut child) => {
                // The last lines are kept, and only said if it falls over: enough to reach back past a backtrace to the panic.
                const KEPT: usize = 200;
                if let Some(err) = child.stderr.take() {
                    let said = self.game_said.clone();
                    if let Ok(mut s) = said.lock() {
                        s.clear();
                    }
                    std::thread::spawn(move || {
                        for line in std::io::BufRead::lines(std::io::BufReader::new(err)).map_while(Result::ok) {
                            if let Ok(mut s) = said.lock() {
                                if s.len() == KEPT {
                                    s.pop_front();
                                }
                                s.push_back(line);
                            }
                        }
                    });
                }
                crate::log(format!("console: playing {}", wad.file_name().unwrap_or_default().to_string_lossy()));
                self.shared.game_pid.store(child.id(), Ordering::Relaxed);
                self.shared.played.store(false, Ordering::Relaxed);
                self.game = Some(child);
                self.playing = Some(wad.to_path_buf());
            }
            Err(e) => crate::log(format!("console: {e}")),
        }
    }

    /// What the remotes have asked for, since this was last asked. A nuked set leaves nothing behind to come back to.
    pub fn take_asked(&self) -> Asked {
        let nuked = self.shared.nuked.lock().map(|mut n| std::mem::take(&mut *n)).unwrap_or_default();
        if !nuked.is_empty()
            && let Ok(mut state) = self.shared.state.lock()
        {
            state.retain(|name, _| !nuked.contains(name));
            drop(state);
            self.shared.save();
        }
        let changed = self.shared.shuffle_changed.swap(false, Ordering::Relaxed);
        let shuffling = changed.then(|| (self.shared.shuffle_seconds.load(Ordering::Relaxed), self.shared.jitter.load(Ordering::Relaxed)));
        Asked { add_tv: self.shared.add_tv.swap(false, Ordering::Relaxed), nuked, shuffling }
    }

    /// Whether the console has just been spared, since this was last asked.
    pub fn take_spared(&mut self) -> bool {
        std::mem::take(&mut self.spared_news)
    }

    /// Keeps the sets and the console as they should be. Every second or so. `path` is where `config` came from. `dark` is only for
    /// the Twatch sets: the console carries on as it was, whatever the display's doing. Short of memory, it isn't so lucky.
    pub fn update(&mut self, config: &crate::config::Config, path: &Path, active: bool, battery: bool, dark: bool, pressure: crate::pressure::Pressure) {
        if let Ok(mut known) = self.shared.config.lock()
            && *known != path
        {
            *known = path.to_path_buf();
        }
        let tv = |m: &&crate::config::MeterSpec| m.style == crate::config::Style::Screen && m.set.as_deref() != Some("twatch");
        let names: Vec<&String> = config.meters.iter().filter(tv).map(|m| &m.label).collect();
        self.shared.sets.store(names.len(), Ordering::Relaxed);
        if !self.loaded {
            self.loaded = true;
            let (sets, game_on) = parse_state(&std::fs::read_to_string(state_file()).unwrap_or_default());
            if let Ok(mut state) = self.shared.state.lock() {
                *state = sets;
            }
            self.shared.game_on.store(game_on, Ordering::Relaxed);
        }
        self.shared.battery.store(battery, Ordering::Relaxed);
        let was = self.shared.busy.swap(self.load.busy(Instant::now()), Ordering::Relaxed);
        if self.shared.busy.load(Ordering::Relaxed) != was {
            crate::log(format!("TV: {} fps, as the PC is {}", if was { FPS } else { BUSY_FPS }, if was { "quiet again" } else { "busy" }));
        }
        self.shared.resting.store(!active, Ordering::Relaxed);
        self.shared.autostart.store(config.console_autostart, Ordering::Relaxed);
        self.shared.intro.store(config.tv_intro, Ordering::Relaxed);
        self.shared.intro_from.store(config.tv_intro_from, Ordering::Relaxed);
        self.shared.intro_to.store(config.tv_intro_to, Ordering::Relaxed);
        // Only what the settings have changed: a remote may be ahead of them, until what it asked for is saved.
        let shuffling = (config.shuffle_seconds.clamp(SHUFFLE_STEPS[0], SHUFFLE_STEPS[SHUFFLE_STEPS.len() - 1]), config.shuffle_jitter);
        if self.shuffling != Some(shuffling) && !self.shared.shuffle_changed.load(Ordering::Relaxed) {
            self.shuffling = Some(shuffling);
            self.shared.shuffle_seconds.store(shuffling.0, Ordering::Relaxed);
            self.shared.jitter.store(shuffling.1, Ordering::Relaxed);
        }
        self.twatch.update(config, active, battery, self.shared.busy.load(Ordering::Relaxed), dark, pressure);
        let short = pressure.console();
        let evicted = console_evicted(short, self.game.is_some(), self.shared.played.load(Ordering::Relaxed));
        // Once for each time memory runs short.
        let spared = short.is_some() && evicted.is_none();
        if spared && !self.spared {
            crate::log("console: memory is low, but it keeps running: someone's been playing it");
            self.spared_news = true;
        }
        self.spared = spared;
        if let Ok(mut said) = self.shared.console_evicted.lock() {
            *said = evicted;
        }
        if let Some(game) = &mut self.game
            && let Ok(Some(status)) = game.try_wait()
        {
            // Quit from its menu ends it too, and a fresh start is what's wanted then.
            crate::log(format!("console: the game stopped ({status})"));
            if !status.success()
                && let Ok(said) = self.game_said.lock()
            {
                // What it said, without a backtrace's frames: the panic, where it was, and the last word.
                let frame = |l: &str| {
                    let l = l.trim_start();
                    l.split_once(':').is_some_and(|(n, rest)| n.chars().all(|c| c.is_ascii_digit()) && rest.trim_start().starts_with("0x")) || l.starts_with("at ")
                };
                let words: Vec<&String> = said.iter().filter(|l| !l.trim().is_empty() && !frame(l)).collect();
                for line in words.iter().rev().take(10).rev() {
                    crate::log(format!("console said: {line}"));
                }
            }
            self.stop_game();
            let now = Instant::now();
            self.falls.retain(|t| now.duration_since(*t) < Duration::from_secs(120));
            self.falls.push(now);
        }
        let wad = crate::wads::chosen().filter(|_| self.falls.len() < 3 && !names.is_empty());
        self.shared.has_game.store(wad.is_some(), Ordering::Relaxed);
        let title = wad.as_ref().and_then(|w| w.file_name()).map(|n| crate::wads::title(&n.to_string_lossy())).unwrap_or_default();
        if let Ok(mut known) = self.shared.game_title.lock()
            && *known != title
        {
            *known = title;
        }
        let wanted = self.shared.game_on.load(Ordering::Relaxed) && evicted.is_none();
        match wad {
            // Another game chosen: out with the old cartridge.
            Some(wad) if wanted && self.game.is_some() && self.playing.as_ref() != Some(&wad) => {
                self.stop_game();
                self.start_game(&wad);
            }
            Some(wad) if wanted && self.game.is_none() => self.start_game(&wad),
            Some(_) if wanted => {}
            _ if self.game.is_some() => self.stop_game(),
            _ => {}
        }
        if self.game.is_some()
            && let Ok(mut console) = self.shared.console.lock()
            && console.is_none()
        {
            *console = Screen::open(CONSOLE).ok().map(Arc::new);
        }
        // A set whose meter went stops; a new one starts.
        let (keep, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.sets).into_iter().partition(|(name, ..)| names.contains(&name));
        self.sets = keep;
        for (_, thread, stop) in gone {
            stop.store(true, Ordering::Relaxed);
            let _ = thread.join();
        }
        for name in names {
            if !self.sets.iter().any(|(n, ..)| n == name) {
                let stop = Arc::new(AtomicBool::new(false));
                let (flag, shared, label) = (stop.clone(), self.shared.clone(), name.clone());
                self.sets.push((name.clone(), std::thread::spawn(move || run(&label, &flag, &shared)), stop));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_that_has_been_played_stays_until_it_is_restarted() {
        let short = Some("RAM PRESSURE");
        assert_eq!(console_evicted(short, true, false), short, "nobody's touched it: it goes");
        assert_eq!(console_evicted(short, true, true), None, "somebody has: it stays");
        assert_eq!(console_evicted(short, false, true), short, "with no game running, there's nothing to lose");
        assert_eq!(console_evicted(None, true, false), None, "and with plenty of memory, nothing goes");
    }

    const W: usize = 64;
    const H: usize = 48;
    const DT: i64 = 33;

    fn on(channel: u32, has_game: bool) -> Tv {
        let mut tv = Tv::new(7, Kept { channel, shuffle: false, on: true }, has_game);
        tv.snow = 0;
        tv.number = 0;
        tv
    }

    fn rgb(px: &[u8], x: usize, y: usize) -> [u8; 3] {
        let at = (y * W + x) * 4;
        [px[at + 2], px[at + 1], px[at]]
    }

    fn channel(tv: &Tv) -> u32 {
        CHANNELS[tv.channel]
    }

    #[test]
    fn the_test_card_has_its_bars_and_then_holds_still() {
        let (mut tv, mut px) = (on(2, false), vec![0; W * H * 4]);
        assert!(tv.frame(DT, W, H, &mut px, None));
        assert_eq!(rgb(&px, 1, 2), [192, 192, 192]);
        assert_eq!(rgb(&px, W - 2, 2), [0, 0, 192]);
        assert_eq!(rgb(&px, W / 2, 2), [0, 192, 0]);
        assert_eq!(rgb(&px, 1, H / 2 + 10), [0, 0, 192], "the reversed row");
        assert_eq!(rgb(&px, 1, H - 1), [0, 33, 76], "the dark row");
        assert!(!tv.frame(DT, W, H, &mut px, None), "nothing new to send");
    }

    #[test]
    fn static_is_different_every_frame() {
        let (mut tv, mut a, mut b) = (on(4, false), vec![0; W * H * 4], vec![0; W * H * 4]);
        tv.frame(DT, W, H, &mut a, None);
        tv.frame(DT, W, H, &mut b, None);
        assert_ne!(a, b);
    }

    #[test]
    fn the_logo_stays_on_screen_and_changes_color_when_it_bounces() {
        let (mut tv, mut px) = (on(3, false), vec![0; W * H * 4]);
        let start = tv.color;
        let (mut lit, mut changed) = (0, false);
        for _ in 0..1200 {
            tv.left = 10_000;
            tv.frame(DT, W, H, &mut px, None);
            let shape = tv.shape.as_ref().expect("the logo decodes");
            assert!(tv.logo.0 + shape.width as f32 <= W as f32 + 0.5 && tv.logo.1 + shape.height as f32 <= H as f32 + 0.5);
            lit = px.chunks(4).filter(|p| p[..3] != [0, 0, 0]).count();
            changed |= tv.color != start;
        }
        assert!(lit > 50, "{lit}");
        assert!(changed);
    }

    #[test]
    fn captain_midnight_fits_and_every_letter_has_a_shape() {
        let height = MIDNIGHT.len() * (H / MIDNIGHT.len());
        assert!(height <= H);
        for line in MIDNIGHT {
            assert!(text_width(line, 1) <= W, "{line}");
            for c in line.chars().filter(|c| *c != ' ') {
                assert_ne!(glyph(c), [0; 5], "{c}");
            }
        }
        assert!(MIDNIGHT.concat().contains("$12.95/MONTH"));
    }

    #[test]
    fn every_channel_draws_at_any_size() {
        let mut tv = Tv::new(99, Kept::default(), true);
        for (w, h) in [(64, 48), (80, 60), (128, 96), (3, 2), (1, 1)] {
            let mut px = vec![0; w * h * 4];
            for _ in CHANNELS {
                tv.press(Button::ChannelUp);
                for _ in 0..12 {
                    tv.frame(DT, w, h, &mut px, None);
                }
            }
        }
    }

    #[test]
    fn up_and_down_go_round_every_channel_with_or_without_a_game() {
        let mut tv = on(2, false);
        let mut seen = vec![];
        for _ in 0..5 {
            tv.press(Button::ChannelUp);
            seen.push(channel(&tv));
        }
        assert_eq!(seen, [3, 4, 5, GAME, 1295]);
        tv.press(Button::ChannelUp);
        assert_eq!(channel(&tv), 2);
        tv.press(Button::ChannelDown);
        assert_eq!(channel(&tv), 1295);
        tv.press(Button::ChannelDown);
        assert_eq!(channel(&tv), GAME);
        let mut tv = on(5, true);
        tv.press(Button::ChannelUp);
        assert_eq!(channel(&tv), GAME);
    }

    #[test]
    fn shuffling_wanders_and_holding_stays() {
        let mut tv = Tv::new(5, Kept { channel: 2, shuffle: true, on: true }, false);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let before = tv.channel;
            tv.wander();
            assert_ne!(tv.channel, before);
            seen.insert(channel(&tv));
        }
        assert_eq!(seen.len(), CHANNELS.len() - 1, "all but the console");
        let (mut tv, mut px) = (on(3, false), vec![0; W * H * 4]);
        tv.left = 10;
        tv.frame(DT, W, H, &mut px, None);
        assert_eq!(channel(&tv), 3);
    }

    #[test]
    fn starting_up_runs_through_to_the_console() {
        let (mut tv, mut px) = (Tv::new(3, Kept::default(), true), vec![0; W * H * 4]);
        tv.start_intro(INTRO_FROM, INTRO_TO);
        assert!(!tv.shuffle);
        let (mut seen, mut ms) = (vec![channel(&tv)], 0);
        while channel(&tv) != GAME && ms < 30_000 {
            tv.frame(DT, W, H, &mut px, None);
            ms += DT;
            if *seen.last().unwrap() != channel(&tv) {
                seen.push(channel(&tv));
            }
        }
        assert_eq!(seen, [1295, 2, 3, 4, 5, GAME]);
        assert!((13_500..15_000).contains(&ms), "Captain Midnight for six seconds, then two for each channel: {ms}");
    }

    #[test]
    fn the_list_and_the_set_have_the_same_channels() {
        assert_eq!(pms_screen::CHANNELS.map(|c| c.0), CHANNELS);
    }

    #[test]
    fn a_channel_from_the_list_holds() {
        let mut tv = Tv::new(3, Kept::default(), false);
        tv.pick(5);
        assert_eq!(channel(&tv), 5);
        assert!(!tv.shuffle);
        tv.pick(77);
        assert_eq!(channel(&tv), 5);
        tv.pick(GAME);
        assert_eq!(channel(&tv), GAME, "the console's channel is always there");
    }

    #[test]
    fn the_console_channel_is_always_there_but_shuffling_keeps_off_it_unless_something_is_on() {
        let mut tv = on(5, false);
        tv.press(Button::ChannelUp);
        assert_eq!(channel(&tv), GAME);
        assert!(tv.on_console());
        assert_eq!(tv.click(false), Clicked::NoGame, "on its own channel, a click says so");
        assert_eq!(channel(&tv), GAME, "and stays there");
        let mut off = on(GAME, true);
        assert_eq!(off.click(false), Clicked::Done, "with a game and the console off, it's only a channel");
        assert_ne!(channel(&off), GAME);
        tv.press(Button::ChannelDown);
        tv.press(Button::GameChannel);
        assert_eq!(channel(&tv), GAME);
        let mut shuffled = on(2, false);
        shuffled.shuffle = true;
        for _ in 0..500 {
            shuffled.wander();
            assert_ne!(channel(&shuffled), GAME);
        }
        shuffled.set_game(true, true);
        assert!((0..500).any(|_| {
            shuffled.wander();
            channel(&shuffled) == GAME
        }));
    }

    #[test]
    fn another_tv_goes_after_the_last_one() {
        let mut c = crate::config::parse("[[meter]]\nlabel = \"RAM\"\nexpr = \"1\"\nmax = 1\n[[meter]]\nlabel = \"TV\"\nstyle = \"screen\"\n[[meter]]\nlabel = \"Twatch TV\"\nstyle = \"screen\"\nset = \"twatch\"\n").unwrap();
        add_tv(&mut c);
        add_tv(&mut c);
        let labels: Vec<&str> = c.meters.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, ["RAM", "TV", "TV 2", "TV 3", "Twatch TV"]);
    }

    #[test]
    fn a_nuked_tv_goes_and_only_that_one() {
        let mut c = crate::config::parse("[[meter]]\nlabel = \"RAM\"\nexpr = \"1\"\nmax = 1\n[[meter]]\nlabel = \"TV\"\nstyle = \"screen\"\n[[meter]]\nlabel = \"TV 2\"\nstyle = \"screen\"\n[[meter]]\nlabel = \"Twatch TV\"\nstyle = \"screen\"\nset = \"twatch\"\n").unwrap();
        nuke_tv(&mut c, "TV");
        nuke_tv(&mut c, "RAM");
        nuke_tv(&mut c, "Twatch TV");
        let labels: Vec<&str> = c.meters.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, ["RAM", "TV 2", "Twatch TV"]);
    }

    const SECOND: u64 = 1000;

    /// The times a set moves, from a moment after `start`, looking every 16 ms for `ms`.
    fn moves(tv: &mut Tv, start: u64, ms: u64, every: u64, jitter: bool) -> Vec<u64> {
        let (mut px, mut seen, mut at) = (vec![0; W * H * 4], vec![], start);
        while at < start + ms {
            let before = tv.channel;
            tv.set_clock(at, every, jitter);
            tv.frame(16, W, H, &mut px, None);
            if tv.channel != before {
                seen.push(at);
            }
            at += 16;
        }
        seen
    }

    fn shuffling(seed: u32) -> Tv {
        Tv::new(seed, Kept { channel: 2, shuffle: true, on: true }, false)
    }

    #[test]
    fn without_jitter_every_set_moves_on_the_same_beat() {
        let every = 10 * SECOND;
        let start = 1_000_000;
        let (mut a, mut b) = (shuffling(5), shuffling(991));
        let (first, second) = (moves(&mut a, start, 61 * SECOND, every, false), moves(&mut b, start, 61 * SECOND, every, false));
        assert_eq!(first, second, "together");
        assert_eq!(first.len(), 6);
        for (i, at) in first.iter().enumerate() {
            let beat = start + (i as u64 + 1) * every;
            assert!((beat..beat + 16).contains(at), "move {i} at {at}, on the beat at {beat}");
        }
    }

    #[test]
    fn with_jitter_each_set_is_late_by_up_to_a_quarter_of_the_beat() {
        let every = 8 * SECOND;
        let start = 800_000;
        let (mut lates, mut count) = (vec![], 0);
        for seed in 1..20 {
            for at in moves(&mut shuffling(seed), start, 40 * SECOND, every, true) {
                count += 1;
                lates.push(at % every);
            }
        }
        assert!(count > 50);
        assert!(lates.iter().all(|late| *late <= every / 4 + 16), "{lates:?}");
        assert!(lates.iter().any(|late| *late > every / 8), "they use the room");
        assert!(lates.iter().any(|late| *late < every / 8));
    }

    #[test]
    fn a_new_beat_or_a_set_coming_on_does_not_move_it_there_and_then() {
        let every = 10 * SECOND;
        let mut tv = shuffling(5);
        assert_eq!(moves(&mut tv, 1_005_000, 4 * SECOND, every, false), [] as [u64; 0]);
        assert_eq!(moves(&mut tv, 1_009_000, 500, 2 * every, false), [] as [u64; 0], "a longer beat starts from the next one");
        tv.press(Button::TvPower);
        moves(&mut tv, 1_009_500, 30 * SECOND, every, false);
        tv.press(Button::TvPower);
        assert_eq!(moves(&mut tv, 1_041_000, 4 * SECOND, every, false), [] as [u64; 0], "off for beats, it's not late for them");
        tv.press(Button::Shuffle);
        moves(&mut tv, 1_045_000, 30 * SECOND, every, false);
        tv.press(Button::Shuffle);
        assert_eq!(moves(&mut tv, 1_081_000, 4 * SECOND, every, false), [] as [u64; 0], "and the same goes for holding");
    }

    #[test]
    fn the_remote_steps_through_the_beats() {
        assert_eq!(shuffle_step(30, true), 45);
        assert_eq!(shuffle_step(30, false), 20);
        assert_eq!(shuffle_step(37, true), 45, "from between two steps");
        assert_eq!(shuffle_step(37, false), 30);
        assert_eq!(shuffle_step(5, false), 5);
        assert_eq!(shuffle_step(3600, true), 3600);
        assert_eq!(shuffle_words(30), "30 S");
        assert_eq!(shuffle_words(90), "1:30");
        assert_eq!(shuffle_words(300), "5 MIN");
        assert_eq!(shuffle_words(3600), "1 HR");
    }

    #[test]
    fn the_run_goes_on_without_a_game_and_from_and_to_where_you_say() {
        let mut tv = Tv::new(3, Kept::default(), false);
        tv.start_intro(INTRO_FROM, INTRO_TO);
        assert!(!tv.intro.is_empty() && !tv.shuffle, "no game needed");
        assert_eq!(intro(3, 5), [(3, INTRO_MS), (4, INTRO_MS)]);
        assert_eq!(intro(4, 2), [(4, INTRO_MS), (5, INTRO_MS), (GAME, INTRO_MS), (MIDNIGHT_CHANNEL, INTRO_MIDNIGHT_MS)], "round past the top");
        assert!(intro(5, 5).is_empty(), "starting where it ends: it just goes there");
        assert_eq!(intro(77, 99), intro(INTRO_FROM, INTRO_TO), "channels there aren't are the usual ones");
        let mut tv = Tv::new(3, Kept::default(), false);
        tv.start_intro(5, 5);
        assert_eq!(channel(&tv), 5);
        let mut tv = Tv::new(3, Kept::default(), false);
        tv.start_intro(2, 4);
        assert_eq!(tv.click(false), Clicked::Done);
        assert_eq!(channel(&tv), 4, "a click skips to the end of the run");
    }

    #[test]
    fn a_click_stops_shuffling_and_the_next_one_plays() {
        let mut tv = Tv::new(3, Kept { channel: 3, shuffle: true, on: true }, true);
        assert_eq!(tv.click(true), Clicked::Done, "the first only stops the shuffling");
        assert!(!tv.shuffle);
        assert_eq!(channel(&tv), 3, "and stays on the channel");
        assert_eq!(tv.click(true), Clicked::Play, "the next plays");
        assert_eq!(channel(&tv), GAME);
    }

    #[test]
    fn a_click_without_a_console_changes_the_channel() {
        let mut tv = on(2, false);
        assert_eq!(tv.click(false), Clicked::Done);
        assert_eq!(channel(&tv), 3);
        let mut tv = on(2, true);
        assert_eq!(tv.click(false), Clicked::Done, "the console's off");
    }

    #[test]
    fn off_is_off() {
        let (mut tv, mut px) = (on(4, false), vec![0; W * H * 4]);
        tv.press(Button::TvPower);
        assert!(tv.frame(DT, W, H, &mut px, None), "the picture collapses");
        for _ in 0..30 {
            tv.frame(DT, W, H, &mut px, None);
        }
        assert!(px.chunks(4).all(|p| p[..3] == [0, 0, 0]));
        assert!(!tv.frame(DT, W, H, &mut px, None), "and then nothing is sent");
        tv.press(Button::ChannelUp);
        assert_eq!(channel(&tv), 4, "its buttons do nothing but power");
        tv.press(Button::TvPower);
        assert!(tv.frame(DT, W, H, &mut px, None));
    }

    #[test]
    fn off_for_a_while_it_is_put_away_and_a_click_brings_it_back() {
        let (mut tv, mut px) = (on(4, false), vec![0; W * H * 4]);
        tv.press(Button::TvPower);
        let mut gone = 0;
        while !tv.status(false).collapsed {
            tv.frame(DT, W, H, &mut px, None);
            gone += DT;
            assert!(gone < 8000, "it's put away");
        }
        assert!(gone > 6500, "but not at once: {gone}");
        assert!(px.chunks(4).any(|p| p[3] > 0) && px.chunks(4).any(|p| p[3] == 0), "a button with nothing behind it");
        assert!(!tv.frame(DT, W, H, &mut px, None), "which holds still");
        assert!(tv.frame(DT, 20, H, &mut vec![0; 20 * H * 4], None), "until it's asked for another size");
        assert_eq!(tv.click(true), Clicked::Done, "a click is only the power");
        assert!(tv.on && !tv.status(false).collapsed);
        let kept = Tv::new(1, Kept { channel: 2, shuffle: false, on: false }, false);
        assert!(kept.status(false).collapsed, "left off, it starts put away");
    }

    #[test]
    fn the_two_kinds_of_set_have_different_buttons() {
        let (mut a, mut b) = (vec![0; 20 * H * 4], vec![0; 20 * H * 4]);
        put_away(&mut Canvas { w: 20, h: H, px: &mut a }, false, false);
        put_away(&mut Canvas { w: 20, h: H, px: &mut b }, true, false);
        let mut c = vec![0; 20 * H * 4];
        put_away(&mut Canvas { w: 20, h: H, px: &mut c }, true, true);
        assert_ne!(b, c, "on standby, it's another color");
        assert_ne!(a, b);
    }

    #[test]
    fn the_console_goes_with_its_game() {
        let mut tv = on(GAME, true);
        tv.set_game(false, false);
        assert_eq!(channel(&tv), GAME, "it stays, with no signal");
        assert_eq!(channel(&on(GAME, false)), GAME, "and comes back there");
    }

    #[test]
    fn with_the_console_off_channel_six_says_so() {
        let (mut tv, mut px) = (on(GAME, true), vec![0; W * H * 4]);
        assert!(tv.frame(DT, W, H, &mut px, None));
        assert_eq!(rgb(&px, 1, 1), [0, 0, 170]);
        assert!(!tv.frame(DT, W, H, &mut px, None));
    }

    #[test]
    fn what_a_set_keeps_comes_back() {
        let sets = HashMap::from([("TV".to_string(), Kept { channel: 6, shuffle: false, on: true }), ("TV 2".to_string(), Kept { channel: 4, shuffle: true, on: false })]);
        let (back, game_on) = parse_state(&state_text(&sets, false));
        assert_eq!(back, sets);
        assert!(!game_on);
        assert!(parse_state("").1, "on, to begin with");
        assert!(parse_state("rubbish\nmore\trubbish").0.is_empty());
    }
}
