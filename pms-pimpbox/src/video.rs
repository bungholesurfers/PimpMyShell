//! The console's video modes: how big a picture it puts out. Three of the modes the consoles of its day had, as square
//! pixels: 480p, which nearly everything used; and 720p and 1080i, which a few things did. (Not the wide 480p some games had:
//! its pixels weren't square, which a window's are.) Whatever's playing draws at its own size and shape, and is fitted to the
//! mode: a 4:3 picture fills a 4:3 mode, and sits in the middle of a wide one with black to each side. The mode can change
//! while the console's on.

use std::cell::Cell;

/// A mode: what it's called, and its picture's size in pixels.
pub const MODES: [(&str, (usize, usize)); 3] = [("480p", (640, 480)), ("720p", (1280, 720)), ("1080i", (1920, 1080))];

thread_local! {
    static MODE: Cell<usize> = const { Cell::new(0) };
}

/// Sets the mode by name. One there isn't is the first, which is the usual.
pub fn set(name: &str) {
    MODE.set(MODES.iter().position(|(called, _)| *called == name).unwrap_or(0));
}

/// The size of the picture the console puts out.
pub fn mode() -> (usize, usize) {
    MODES[MODE.get()].1
}

/// What the mode's called.
pub fn name() -> &'static str {
    MODES[MODE.get()].0
}

/// A picture, `across` by `down` pixels, that's to be seen `shape` times as wide as it's tall whatever shape its pixels
/// are, as the mode shows it: stretched smoothly to be as big as fits in `out`, which is made the mode's size, in the middle
/// of it, with black round it.
pub fn compose(source: &[u32], (across, down): (usize, usize), shape: f32, out: &mut Vec<u32>) {
    let (w, h) = mode();
    let wide = ((h as f32 * shape).round() as usize).clamp(1, w);
    let tall = ((wide as f32 / shape).round() as usize).clamp(1, h);
    let (left, top) = ((w - wide) / 2, (h - tall) / 2);
    out.clear();
    out.resize(w * h, 0);
    // Where each column comes from, worked out once: the column to its left, and how much of the next one, out of 256.
    let from = |i: usize, of: usize, to: usize| {
        let at = ((i * 2 + 1) * of * 128 / to).saturating_sub(128);
        ((at >> 8).min(of - 1), ((at >> 8) + 1).min(of - 1), (at & 255) as u32)
    };
    let columns: Vec<(usize, usize, u32)> = (0..wide).map(|x| from(x, across, wide)).collect();
    let blend = |a: u32, b: u32, t: u32| {
        let (rb, g) = ((a & 0x00FF_00FF) * (256 - t) + (b & 0x00FF_00FF) * t, (a & 0x0000_FF00) * (256 - t) + (b & 0x0000_FF00) * t);
        ((rb >> 8) & 0x00FF_00FF) | ((g >> 8) & 0x0000_FF00)
    };
    for (y, row) in out.chunks_mut(w).skip(top).take(tall).enumerate() {
        let (upper, lower, ty) = from(y, down, tall);
        let (upper, lower) = (&source[upper * across..(upper + 1) * across], &source[lower * across..(lower + 1) * across]);
        for (to, (x0, x1, tx)) in row[left..left + wide].iter_mut().zip(&columns) {
            *to = blend(blend(upper[*x0], upper[*x1], *tx), blend(lower[*x0], lower[*x1], *tx), ty);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_fitted_to_the_mode() {
        // One color all over stays that color, all over the middle; and in a wide mode there's black to each side of it.
        let source = vec![0x0012_3456; 640 * 400];
        let mut out = Vec::new();
        for (name, (w, h)) in MODES {
            set(name);
            compose(&source, (640, 400), 4.0 / 3.0, &mut out);
            assert_eq!(out.len(), w * h, "{name}");
            let wide = (h * 4 / 3).min(w);
            let left = (w - wide) / 2;
            assert_eq!((wide * 3, left * 2 + wide), (h * 4, w), "{name}");
            assert!(out.chunks(w).all(|row| row[left..left + wide].iter().all(|p| *p == 0x0012_3456) && row[..left].iter().all(|p| *p == 0)), "{name}");
        }
        // A wide picture in a mode that isn't has black over and under it.
        set("480p");
        compose(&vec![0x00FF_FFFF; 1280 * 720], (1280, 720), 16.0 / 9.0, &mut out);
        assert!(out[..640 * 60].iter().all(|p| *p == 0) && out[640 * 240..640 * 241].iter().all(|p| *p == 0x00FF_FFFF));
        set("no such mode");
        assert_eq!((mode(), name()), ((640, 480), "480p"));
    }
}
