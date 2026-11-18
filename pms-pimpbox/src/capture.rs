//! What the console's window says when it takes your keyboard and mouse: a bar under its picture, the window growing to make room
//! for it so that none of the picture's covered, saying how to get them back, typed out a letter at a time behind a blinking
//! green cursor. After about ten seconds, or as soon as they're given back, the letters fall away as rain and the window shrinks
//! back. (The PIMPBOX card beside the window goes on saying it for as long as the window has them.) Arithmetic only: nothing's
//! random, and nothing here's a picture or a font file of its own (the lettering is Windows' Consolas).

use std::cell::Cell;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows_core::PCWSTR;

/// How long the window takes to grow, or shrink back.
const SLIDE: Duration = Duration::from_millis(180);
/// How long the bar's there, from the keys and the mouse being taken, before it goes by itself.
const SHOW_FOR: Duration = Duration::from_secs(10);
/// A letter typed every this long; a little longer after a full stop or a comma. About 450 words a minute: quick, and still
/// seen to be typed.
const KEY: Duration = Duration::from_millis(26);
const PAUSE: Duration = Duration::from_millis(220);
/// The cursor's on for this long, and off for as long.
const BLINK: Duration = Duration::from_millis(530);
/// How long the rain takes, before the bar goes.
const RAIN: Duration = Duration::from_millis(1100);

const GREEN: u32 = 0x33FF66;
const DIM_GREEN: u32 = 0x1F8F3F;
const PALE_GREEN: u32 = 0xC8FFD8;
const BAR: u32 = 0x0A120C;
/// What the rain's made of: the digits and the letters a terminal has, and some of its signs.
const GLYPHS: &[u8] = b"0123456789ABCDEFZXKRTY:.=*+-<>|";

thread_local! {
    /// When the keys and the mouse were last taken, and given back.
    static TAKEN: Cell<Option<Instant>> = const { Cell::new(None) };
    static GIVEN: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The keys and the mouse have just been taken.
pub fn taken() {
    TAKEN.set(Some(Instant::now()));
    GIVEN.set(None);
}

/// They've just been given back.
pub fn given() {
    if TAKEN.get().is_some() && GIVEN.get().is_none() {
        GIVEN.set(Some(Instant::now()));
    }
}

/// How long the bar's been going away, if it is: since they were given back, or since it had been up its while.
fn leaving(now: Instant) -> Option<Duration> {
    let taken = TAKEN.get()?;
    match GIVEN.get() {
        Some(given) => Some(now - given),
        None => (now - taken).checked_sub(SHOW_FOR),
    }
}

/// How much of the bar there's room for in the window, from none to all of it.
pub fn up() -> f32 {
    let now = Instant::now();
    let Some(taken) = TAKEN.get() else { return 0.0 };
    let grown = ((now - taken).as_secs_f32() / SLIDE.as_secs_f32()).min(1.0);
    match leaving(now) {
        None => grown,
        Some(g) if g < RAIN => grown,
        Some(g) => (1.0 - ((g - RAIN).as_secs_f32() / SLIDE.as_secs_f32()).min(1.0)).min(grown),
    }
}

/// Whether anything's moving: the window's to be drawn again soon, and perhaps resized, whether or not there's a new picture.
pub fn moving() -> bool {
    let now = Instant::now();
    let Some(_) = TAKEN.get() else { return false };
    if leaving(now).is_some_and(|g| g >= RAIN + SLIDE) {
        // All done: nothing more to draw, until the next time.
        TAKEN.set(None);
        GIVEN.set(None);
        return false;
    }
    true
}

/// How tall the bar is, for a picture `picture` pixels tall.
pub fn tall(picture: i32) -> i32 {
    (picture / 18).clamp(26, 52)
}

/// `rgb` as GDI has a color.
fn color(rgb: u32) -> COLORREF {
    COLORREF(((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF))
}

fn fill(dc: HDC, area: RECT, rgb: u32) {
    unsafe {
        let brush = CreateSolidBrush(color(rgb));
        FillRect(dc, &area, brush);
        let _ = DeleteObject(brush.into());
    }
}

/// How many of `text`'s letters are typed out `since` the typing began, and whether that's all of them.
fn typed(text: &[u16], since: Duration) -> (usize, bool) {
    let mut at = Duration::ZERO;
    for i in 0..text.len() {
        at += if i > 0 && matches!(text[i - 1], 0x2E | 0x2C) { PAUSE } else { KEY };
        if at > since {
            return (i, false);
        }
    }
    (text.len(), true)
}

/// A number from `a` and `b` that looks like nothing in particular, for where and how fast a letter falls: arithmetic, the same
/// every time.
fn mix(a: usize, b: usize) -> usize {
    (a.wrapping_mul(2_654_435_761) ^ b.wrapping_mul(40_503)).wrapping_add(a ^ (b << 7)) >> 3
}

/// Draws the bar with `text` in it, in `dc`, `width` wide, in the room there is for it under the picture: from `top` down,
/// `room` pixels of a bar `tall` pixels tall (the window's still growing, or shrinking back).
pub fn draw(dc: HDC, width: i32, top: i32, room: i32, tall: i32, text: &str) {
    let Some(taken) = TAKEN.get() else { return };
    if room <= 0 {
        return;
    }
    let now = Instant::now();
    let since = now - taken;
    fill(dc, RECT { left: 0, top, right: width, bottom: top + room }, BAR);
    fill(dc, RECT { left: 0, top, right: width, bottom: top + 1 }, DIM_GREEN);
    unsafe {
        let words: Vec<u16> = text.encode_utf16().collect();
        let pad = tall / 2;
        // As big as fits: the bar's height, or less where the words are long for the window.
        let mut size = tall * 11 / 20;
        let font = loop {
            let face: Vec<u16> = "Consolas\0".encode_utf16().collect();
            let font = CreateFontW(-size, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, 0, PCWSTR(face.as_ptr()));
            let old = SelectObject(dc, font.into());
            let mut extent = windows::Win32::Foundation::SIZE::default();
            let _ = GetTextExtentPoint32W(dc, &words, &mut extent);
            SelectObject(dc, old);
            if extent.cx + pad * 3 <= width || size <= 9 {
                break font;
            }
            let _ = DeleteObject(font.into());
            size -= 1;
        };
        let old = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        let mut one = windows::Win32::Foundation::SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &[0x4D], &mut one);
        let (cell, line) = (one.cx.max(1), one.cy.max(1));
        let (x, y) = (pad, top + (tall - line) / 2);
        // Only within the bar: what falls, falls out of sight at its bottom.
        let clip = CreateRectRgn(0, top + 1, width, top + room);
        SelectClipRgn(dc, Some(clip));
        match leaving(now) {
            None => {
                let (shown, done) = typed(&words, since.saturating_sub(SLIDE));
                SetTextColor(dc, color(GREEN));
                let _ = TextOutW(dc, x, y, &words[..shown]);
                // The cursor: on while typing, and blinking once it's done.
                if !done || (since.as_millis() / BLINK.as_millis()).is_multiple_of(2) {
                    let at = x + cell * shown as i32;
                    fill(dc, RECT { left: at, top: y, right: at + cell, bottom: y + line }, GREEN);
                }
            }
            Some(g) => {
                // Each letter in turn, a moment apart, flickers through the rain's glyphs and falls, a fading trail behind it.
                // What wasn't typed yet (given back early) has nothing to fall.
                let (shown, _) = typed(&words, (since - g).saturating_sub(SLIDE));
                let t = g.as_secs_f32();
                for (i, &c) in words.iter().enumerate().take(shown) {
                    if c == 0x20 {
                        continue;
                    }
                    let start = (mix(i, 1) % 9) as f32 * 0.035;
                    let lx = x + cell * i as i32;
                    if t < start {
                        SetTextColor(dc, color(GREEN));
                        let _ = TextOutW(dc, lx, y, &[c]);
                        continue;
                    }
                    let fall = t - start;
                    let speed = (1.6 + (mix(i, 2) % 7) as f32 * 0.35) * tall as f32;
                    let head = y as f32 + fall * speed;
                    let frame = (fall * 22.0) as usize;
                    for k in 0..5usize {
                        let ly = head - (k as i32 * line) as f32;
                        if ly < y as f32 - 1.0 {
                            break;
                        }
                        let glyph = GLYPHS[mix(i + k * 31, frame + k) % GLYPHS.len()] as u16;
                        SetTextColor(dc, color(match k {
                            0 => PALE_GREEN,
                            1 => GREEN,
                            _ => DIM_GREEN,
                        }));
                        let _ = TextOutW(dc, lx, ly as i32, &[glyph]);
                    }
                }
            }
        }
        SelectClipRgn(dc, None);
        let _ = DeleteObject(clip.into());
        SelectObject(dc, old);
        let _ = DeleteObject(font.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_goes_a_letter_at_a_time_and_waits_after_a_stop() {
        let text: Vec<u16> = "Ab. Cd".encode_utf16().collect();
        assert_eq!(typed(&text, Duration::ZERO), (0, false));
        assert_eq!(typed(&text, KEY), (1, false));
        assert_eq!(typed(&text, KEY * 3), (3, false), "A, b and the stop");
        // After the stop, the space waits a while.
        assert_eq!(typed(&text, KEY * 3 + PAUSE - Duration::from_millis(1)), (3, false));
        assert_eq!(typed(&text, KEY * 3 + PAUSE), (4, false));
        assert_eq!(typed(&text, Duration::from_secs(5)), (text.len(), true));
    }

    #[test]
    fn colors_are_as_gdi_has_them() {
        assert_eq!(color(0x33FF66).0, 0x66FF33);
    }

    #[test]
    fn the_bar_comes_stays_its_while_and_goes() {
        assert_eq!(up(), 0.0, "nothing taken");
        TAKEN.set(Some(Instant::now() - SLIDE * 2));
        assert_eq!(up(), 1.0, "grown");
        assert!(moving());
        TAKEN.set(Some(Instant::now() - SHOW_FOR - RAIN - SLIDE / 2));
        assert!(up() > 0.0 && up() < 1.0, "shrinking back by itself after its while");
        TAKEN.set(Some(Instant::now() - SHOW_FOR - RAIN - SLIDE * 2));
        assert_eq!(up(), 0.0);
        assert!(!moving(), "and done");
        assert!(TAKEN.get().is_none());
    }
}
