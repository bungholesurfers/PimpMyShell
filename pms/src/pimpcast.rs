//! Channel 5, the PimpCast: a local-weather channel of the sort that ran on cable boxes, in a space this small only by
//! impression. It boots first, a logo tumbling on a gray screen over a band of the box's details, as the real thing did (at
//! the same rates, on the same three axes), and then shows the forecast, which is always the same.
//!
//! Drawn at 64 by 48, whatever size the screen is, and scaled up: its letters are three pixels wide.

use crate::tv::{Canvas, glyph, text_width};

const W: usize = 64;
const H: usize = 48;
const BOOT: i64 = 3000;
/// The forecast only has rain and a crawl to move: this often is plenty.
const STEP: i64 = 66;
/// How fast the logo turns about each axis, in radians a millisecond.
const TURNS: [f32; 3] = [-0.9 / 1900.0, 0.8 / 1900.0, -0.7 / 1900.0];
const CRAWL: &str = "IT GON RAIN!   BRING YO UMBRELLA, PLAYA   69 AND DRIPPIN   ";

const GRAY: [u8; 3] = [216, 216, 216];
const BAND: [u8; 3] = [16, 16, 16];
const AMBER: [u8; 3] = [255, 204, 0];
const NAVY: [u8; 3] = [8, 16, 96];
const STRIP: [u8; 3] = [48, 84, 168];
const YELLOW: [u8; 3] = [232, 216, 112];
const INK: [u8; 3] = [16, 16, 24];
const WHITE: [u8; 3] = [255, 255, 255];
const LOGO_BLUE: [u8; 3] = [16, 80, 160];
const GOLD: [u8; 3] = [136, 116, 48];

/// The logo, flat: a star and the name, black and blue, with the rest clear.
const LOGO_W: usize = 44;
const LOGO_H: usize = 11;

fn logo() -> Vec<Option<[u8; 3]>> {
    let mut px = vec![None; LOGO_W * LOGO_H];
    // A four-pointed star.
    for y in 0..LOGO_H as i32 {
        for x in 0..LOGO_H as i32 {
            let (dx, dy) = ((x - 5).abs(), (y - 5).abs());
            if dx * dy <= 1 && dx + dy <= 5 {
                px[y as usize * LOGO_W + x as usize] = Some(LOGO_BLUE);
            }
        }
    }
    for (i, c) in "PIMPSTAR".chars().enumerate() {
        for (ry, row) in glyph(c).iter().enumerate() {
            for rx in 0..3 {
                if row & (4 >> rx) != 0 {
                    let (x, y) = (12 + i * 4 + rx, 3 + ry);
                    px[y * LOGO_W + x] = Some(if i < 4 { [0, 0, 0] } else { LOGO_BLUE });
                }
            }
        }
    }
    px
}

fn mix(a: [u8; 3], b: [u8; 3], k: f32) -> [u8; 3] {
    std::array::from_fn(|i| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * k) as u8)
}

fn rect(c: &mut Canvas, x: usize, y: usize, w: usize, h: usize, color: [u8; 3]) {
    for py in y..y + h {
        for px in x..x + w {
            c.set(px, py, color);
        }
    }
}

/// The three turns, as what they do to the logo's plane on the screen: Z, then Y, then X, seen from straight ahead.
fn turned(ms: f32) -> [f32; 4] {
    let (x, y, z) = (TURNS[0] * ms, TURNS[1] * ms, TURNS[2] * ms);
    let (sx, cx, sy, cy, sz, cz) = (x.sin(), x.cos(), y.sin(), y.cos(), z.sin(), z.cos());
    [cz * cy, cz * sy * sx - sz * cx, sz * cy, sz * sy * sx + cz * cx]
}

fn boot(c: &mut Canvas, shape: &[Option<[u8; 3]>], ms: i64) {
    rect(c, 0, 0, W, 36, GRAY);
    let [a, b, d, e] = turned(ms as f32);
    let det = a * e - b * d;
    if det.abs() > 0.02 {
        let (cx, cy) = (32.0, 18.0);
        for y in 0..36 {
            for x in 0..W {
                // Four looks a pixel, for its edges.
                let mut seen = [0f32; 3];
                let mut hits = 0;
                for (ox, oy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                    let (px, py) = (x as f32 + ox - cx, y as f32 + oy - cy);
                    let (u, v) = ((e * px - b * py) / det + LOGO_W as f32 / 2.0, (a * py - d * px) / det + LOGO_H as f32 / 2.0);
                    if u >= 0.0 && v >= 0.0 && (u as usize) < LOGO_W && (v as usize) < LOGO_H
                        && let Some(color) = shape[v as usize * LOGO_W + u as usize]
                    {
                        hits += 1;
                        for i in 0..3 {
                            seen[i] += f32::from(color[i]);
                        }
                    }
                }
                if hits > 0 {
                    let color = seen.map(|v| (v / hits as f32) as u8);
                    c.set(x, y, mix(GRAY, color, hits as f32 / 4.0));
                }
            }
        }
    }
    rect(c, 0, 36, W, 12, BAND);
    c.text("HEADEND 069", 2, 37, 1, AMBER, false);
    c.text("UPTOWN CABLE", 2, 43, 1, AMBER, false);
    rect(c, 53, 37, 9, 9, [0, 40, 150]);
    c.text("P", 56, 39, 1, WHITE, false);
}

fn sky(c: &mut Canvas) {
    for y in 0..H {
        let color = mix([60, 104, 180], [132, 170, 226], y as f32 / H as f32);
        rect(c, 0, y, W, 1, color);
    }
}

fn forecast(c: &mut Canvas, ms: i64) {
    sky(c);
    // Left: where, now, and the map.
    rect(c, 0, 5, 18, 7, YELLOW);
    c.text("HOOD", 1, 6, 1, INK, false);
    for y in 12..33 {
        rect(c, 0, y, 18, 1, mix([226, 234, 248], [132, 170, 226], (y - 12) as f32 / 21.0));
    }
    c.text("NOW", 1, 13, 1, INK, false);
    c.text("69", 0, 21, 2, INK, false);
    // The cloud, and its rain.
    for (x, y, w) in [(14usize, 13usize, 2usize), (13, 14, 4), (13, 15, 5), (13, 16, 5)] {
        rect(c, x, y, w, 1, WHITE);
    }
    let fall = (ms / 130) as usize;
    for (i, x) in [15usize, 17, 16].into_iter().enumerate() {
        let y = 18 + (fall + i * 5) % 13;
        rect(c, x, y, 1, 2, [60, 110, 200]);
    }
    rect(c, 0, 33, 18, 2, [40, 40, 44]);
    for y in 35..42 {
        for x in 0..18 {
            let land = x + (y - 35) * 2 < 15;
            c.set(x, y, if land { if (x + y) % 5 == 0 { [120, 124, 120] } else { [64, 84, 56] } } else { [20, 52, 136] });
        }
    }
    // Right: the name, what's coming, and the day.
    rect(c, 18, 0, 46, 6, STRIP);
    c.text("PIMPCAST", 19, 1, 1, WHITE, false);
    rect(c, 18, 6, 46, 21, NAVY);
    for y in 0..4 {
        rect(c, 64 - (4 - y) * 2, 26 - y, (4 - y) * 2, 1, AMBER);
    }
    c.text("IT GON", 18, 6, 2, WHITE, false);
    c.text("RAIN!", 22, 16, 2, WHITE, false);
    rect(c, 18, 27, 46, 6, YELLOW);
    c.text("TODAY: WET", 19, 28, 1, INK, false);
    for (i, tall) in [6usize, 5, 3].into_iter().enumerate() {
        let x = 19 + i * 15;
        rect(c, x, 33, 14, 9, [88, 132, 206]);
        rect(c, x, 39, 14, 3, GOLD);
        rect(c, x + 1, 35, 5, 2, WHITE);
        rect(c, x + 9, 41 - tall, 4, tall, [176, 176, 184]);
    }
    // The crawl.
    rect(c, 0, 42, W, 6, [24, 24, 28]);
    let width = text_width(CRAWL, 1) as i64 + 1;
    let off = (ms / 45) % width;
    for pass in 0..2 {
        for (i, ch) in CRAWL.chars().enumerate() {
            let x = i as i64 * 4 - off + pass * width;
            if (0..W as i64 - 3).contains(&x) {
                c.text(&ch.to_string(), x as usize, 43, 1, WHITE, false);
            }
        }
    }
}

/// The channel: how long it's been on.
pub struct PimpCast {
    on: i64,
    drawn: i64,
    shape: Vec<Option<[u8; 3]>>,
    small: Vec<u8>,
}

impl PimpCast {
    pub fn new() -> PimpCast {
        PimpCast { on: 0, drawn: -STEP, shape: logo(), small: vec![255; W * H * 4] }
    }

    /// Tuned in: it boots again.
    pub fn restart(&mut self) {
        self.on = 0;
        self.drawn = -STEP;
    }

    /// Draws the next frame, if it's time for one.
    pub fn frame(&mut self, dt: i64, out: &mut Canvas) -> bool {
        self.on += dt;
        let booting = self.on < BOOT;
        if !booting && self.on - self.drawn < STEP {
            return false;
        }
        self.drawn = self.on;
        let mut c = Canvas { w: W, h: H, px: &mut self.small };
        if booting {
            boot(&mut c, &self.shape, self.on);
        } else {
            forecast(&mut c, self.on - BOOT);
        }
        for y in 0..out.h {
            for x in 0..out.w {
                let from = ((y * H / out.h.max(1)) * W + x * W / out.w.max(1)) * 4;
                out.px[(y * out.w + x) * 4..][..4].copy_from_slice(&self.small[from..from + 4]);
            }
        }
        true
    }
}

impl Default for PimpCast {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(px: &[u8], x: usize, y: usize) -> [u8; 3] {
        let i = (y * W + x) * 4;
        [px[i + 2], px[i + 1], px[i]]
    }

    fn run(ms: i64) -> Vec<u8> {
        let (mut cast, mut px) = (PimpCast::new(), vec![0; W * H * 4]);
        let mut gone = 0;
        while gone < ms {
            cast.frame(33, &mut Canvas { w: W, h: H, px: &mut px });
            gone += 33;
        }
        px
    }

    #[test]
    fn it_boots_on_gray_over_a_band() {
        let px = run(500);
        assert_eq!(at(&px, 1, 1), GRAY);
        assert_eq!(at(&px, 1, 47), BAND);
        assert!(px.chunks(4).any(|p| p[0] > 120 && p[2] < 60), "the logo's blue is there");
    }

    #[test]
    fn then_it_gon_rain() {
        let px = run(3200);
        assert_eq!(at(&px, 19, 26), NAVY);
        assert!(px.chunks(4).filter(|p| p[..3] == [255, 255, 255]).count() > 60, "the words");
    }

    #[test]
    fn the_logo_turns() {
        assert_ne!(turned(0.0), turned(1000.0));
        assert_eq!(turned(0.0), [1.0, 0.0, 0.0, 1.0], "it starts square on");
    }

    #[test]
    fn every_word_has_its_letters_and_fits() {
        for line in ["HEADEND 069", "UPTOWN CABLE", "PIMPCAST", "TODAY: WET", "HOOD", "NOW", "69", CRAWL] {
            for ch in line.chars().filter(|c| *c != ' ') {
                assert_ne!(glyph(ch), [0; 5], "{ch}");
            }
        }
        assert!(text_width("IT GON", 2) <= 46);
    }

    #[test]
    fn it_scales_to_any_screen() {
        let (mut cast, mut px) = (PimpCast::new(), vec![0; 112 * 84 * 4]);
        assert!(cast.frame(33, &mut Canvas { w: 112, h: 84, px: &mut px }));
    }
}

