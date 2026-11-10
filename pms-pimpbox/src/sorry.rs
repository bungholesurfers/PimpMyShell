//! What the console says when a game has fallen over: a notice across the picture, over the console coming back on, in the
//! manner of a desktop of a later day than the console's own. The picture dims; a band of one flat color comes across the
//! whole width of it; and its words come in from the right, one after another, and settle: a title, the same again
//! smaller, and under them a thin bar that fills as a quarter of a minute goes by. Then the words leave to the left, the band goes,
//! and the console carries on coming on. It says what such notices said.
//!
//! Drawn here, onto the picture the graphics card's handed back, with letters from Windows' own fonts.

use pimpbox_boot::Lettering;

/// How long it takes to come, how long it stays (which is what the bar counts), and how long it takes to go: seconds.
const COMES: f32 = 0.55;
const STAYS: f32 = 15.0;
const GOES: f32 = 0.45;
/// How long it's there in all.
pub const TAKES: f32 = COMES + STAYS + GOES;

const TITLE: &str = "Something happened";
const BODY: &str = "Something happened";
/// The band's color, and its words'.
const BAND: [f32; 3] = [70.0, 23.0, 180.0];
const WORDS: [f32; 3] = [255.0, 255.0, 255.0];

/// The notice, with its words lettered for the size of picture it was last put on.
#[derive(Default)]
pub struct Sorry {
    lettered: Option<(usize, Option<Lettering>, Option<Lettering>)>,
}

fn blend(px: &mut u32, color: [f32; 3], alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    let mix = |shift: u32, c: f32| ((((*px >> shift) & 0xFF) as f32 * (1.0 - a) + c * a) as u32).min(255) << shift;
    *px = mix(16, color[0]) | mix(8, color[1]) | mix(0, color[2]);
}

/// 0 to 1, quick at first and settling: how things arrived.
fn settling(x: f32) -> f32 {
    1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(3)
}

impl Sorry {
    /// Puts the notice on `picture`, which is `w` by `h` pixels, as it is `since` seconds after it began to come.
    pub fn over(&mut self, picture: &mut [u32], (w, h): (usize, usize), since: f32) {
        if !(0.0..TAKES).contains(&since) {
            return;
        }
        if self.lettered.as_ref().is_none_or(|(tall, ..)| *tall != h) {
            let (big, small) = ((h as f32 * 0.085) as i32, (h as f32 * 0.042) as i32);
            self.lettered = Some((h, pimpbox_boot::fonts::lettering(TITLE, "Segoe UI Light", 300, false, big), pimpbox_boot::fonts::lettering(BODY, "Segoe UI", 400, false, small)));
        }
        let leaving = ((since - COMES - STAYS) / GOES).clamp(0.0, 1.0);
        // The band: there quickly, and the last thing to go.
        let band = (since / 0.2).clamp(0.0, 1.0) * (1.0 - ((leaving - 0.45) / 0.55).clamp(0.0, 1.0));
        let (top, bottom) = ((h as f32 * 0.3) as usize, (h as f32 * 0.7) as usize);
        for (y, row) in picture.chunks_mut(w).enumerate() {
            if (top..bottom).contains(&y) {
                row.iter_mut().for_each(|px| blend(px, BAND, band));
            } else {
                // What's behind it dims.
                row.iter_mut().for_each(|px| blend(px, [0.0; 3], 0.6 * band));
            }
        }
        // Its words and its bar: each comes in from the right a little after the one above it, and they all leave to the left.
        let left = w as f32 * 0.24;
        let arrives = |nth: f32| {
            let come = settling((since - 0.08 - 0.07 * nth) / 0.42);
            (left + (1.0 - come) * w as f32 * 0.1 - leaving * leaving * w as f32 * 0.08, come * (1.0 - leaving))
        };
        let Some((_, title, body)) = &self.lettered else { return };
        let mut write = |words: &Option<Lettering>, nth: f32, down: f32| {
            let Some(l) = words else { return };
            let (x0, alpha) = arrives(nth);
            // Lettered with half its height of room to its left, which isn't part of where it starts.
            let (x0, y0) = (x0 as i32 - l.h as i32 / 2, (h as f32 * down) as i32);
            for (ly, covers) in l.cover.chunks(l.w).enumerate() {
                let y = y0 + ly as i32;
                if y < 0 || y >= h as i32 {
                    continue;
                }
                for (lx, cover) in covers.iter().enumerate().filter(|(_, c)| **c > 0) {
                    let x = x0 + lx as i32;
                    if x >= 0 && x < w as i32 {
                        blend(&mut picture[y as usize * w + x as usize], WORDS, alpha * f32::from(*cover) / 255.0);
                    }
                }
            }
        };
        write(title, 0.0, 0.36);
        write(body, 1.0, 0.475);
        // The bar: thin and flat, a paler track with white filling it from the left as the time goes.
        let (x0, alpha) = arrives(2.0);
        let (wide, thick) = (w as f32 * 0.5, (h as f32 / 110.0).max(3.0));
        let done = ((since - COMES) / STAYS).clamp(0.0, 1.0);
        let y0 = (h as f32 * 0.6) as usize;
        for y in y0..(y0 + thick as usize).min(h) {
            for x in (x0.max(0.0) as usize)..((x0 + wide).max(0.0) as usize).min(w) {
                let filled = (x as f32 - x0) < wide * done;
                blend(&mut picture[y * w + x], WORDS, alpha * if filled { 1.0 } else { 0.28 });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_comes_stays_and_goes() {
        let (w, h) = (320, 240);
        let lit = |since: f32| {
            let mut picture = vec![0x0020_2020; w * h];
            Sorry::default().over(&mut picture, (w, h), since);
            picture
        };
        // Before and after, the picture's left alone.
        assert!(lit(-0.1).iter().all(|p| *p == 0x0020_2020) && lit(TAKES).iter().all(|p| *p == 0x0020_2020));
        // While it stays: the band's its color right across, and what's above it is dimmed.
        let there = lit(COMES + 1.0);
        assert_eq!(there[h / 2 * w + 2], 0x0046_17B4, "the band, at the picture's edge");
        assert!(there[2] & 0xFF < 0x20, "dimmed above it");
        // The bar fills: more of its row is white late than early.
        let white = |picture: &[u32]| picture[(h as f32 * 0.6) as usize * w..][..w].iter().filter(|p| **p == 0x00FF_FFFF).count();
        assert!(white(&lit(COMES + 12.0)) > white(&lit(COMES + 3.0)) + 50);
    }
}
