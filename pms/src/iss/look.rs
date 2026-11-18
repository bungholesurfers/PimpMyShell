//! Number One's picture: an astronaut on the moon, facing the flag he's planted, Earth over his shoulder. The flag is the urine
//! tank: pee-yellow from its hem up to the level, the number big across it. PTV's bug in the bottom left, as a channel's logo sits
//! over what it shows. Original pixel art, after the design's Python mock-up, which is 64 by 48.
//!
//! Drawn at the screen's own size, as the TVs ask: the scene's laid out in the mock's units and each of those is as many pixels
//! as fit, so a bigger taskbar gets more detail, not bigger pixels. The letters and the bug are in whole pixels: one a dot below
//! twice the mock's size, two from there.

use crate::tv::{Canvas, glyph};

/// The mock's size, which the scene is laid out in.
const W: f32 = 64.0;
const H: f32 = 48.0;

type Rgb = [u8; 3];
const SPACE: Rgb = [6, 8, 20];
const STAR: Rgb = [210, 210, 230];
const STAR_DIM: Rgb = [90, 92, 120];
const GROUND: Rgb = [112, 112, 122];
const GROUND_TOP: Rgb = [150, 150, 160];
const CRATER: Rgb = [78, 78, 88];
const SUIT: Rgb = [236, 236, 242];
const SUIT_SHADE: Rgb = [176, 178, 190];
const VISOR: Rgb = [214, 152, 36];
const GLINT: Rgb = [255, 238, 190];
const PACK: Rgb = [150, 152, 164];
const BOOT: Rgb = [100, 100, 112];
const POLE: Rgb = [176, 176, 186];
const NAVY: Rgb = [28, 34, 82];
pub(crate) const PEE: Rgb = [246, 210, 46];
const PEE_DARK: Rgb = [196, 150, 18];
const BUBBLE: Rgb = [255, 244, 160];
const WHITE: Rgb = [255, 255, 255];
const BLACK: Rgb = [0, 0, 0];
const GREY: Rgb = [64, 66, 76];
const GREY_LIGHT: Rgb = [150, 152, 162];
const EARTH: Rgb = [40, 96, 200];
const LAND: Rgb = [60, 150, 80];
const AMBER: Rgb = [255, 120, 40];
/// The tour's other tanks: the waste water, and the drinking water ("tomorrow's coffee").
const WASTE: Rgb = [138, 128, 104];
const WASTE_DARK: Rgb = [98, 90, 72];
const WATER: Rgb = [72, 150, 232];
const WATER_DARK: Rgb = [40, 100, 186];
const LOST_WORDS: Rgb = [200, 202, 210];

const STARS: [(f32, f32, bool); 11] = [(44., 1., false), (49., 7., true), (51., 2., false), (61., 13., true), (62., 24., false), (59., 33., true), (4., 35., false), (15., 37., true), (27., 34., false), (36., 38., true), (0., 1., true)];

/// The flag: its free end on the left, its hoist at the pole.
const FX: f32 = 2.0;
const FY: f32 = 6.0;
const FW: f32 = 39.0;
const FH: f32 = 26.0;
const POLE_X: f32 = 41.0;

/// PTV's bug: a P and a little tv.
const BUG: [&str; 6] = ["XXX.......", "X.X.......", "XXX.X.....", "X..XXX.X.X", "X...X..X.X", "X...XX..X."];

/// What the cloth is filled with.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fill {
    Urine,
    Waste,
    Water,
}

/// Everything the picture shows: what it's drawn from, and what tells one picture from another.
#[derive(Clone, PartialEq, Debug)]
pub struct Scene {
    /// How full, 0 to 100.
    pub level: f32,
    pub fill: Fill,
    /// Big across the flag, and small under that.
    pub words: String,
    pub small: Option<String>,
    /// The signal's lost: grey, and no glint on his visor.
    pub lost: bool,
    /// The urine processor's distilling it.
    pub bubbles: bool,
    /// The amber light: nearly full.
    pub full: bool,
    /// The whirlpool of a flush, turned this far.
    pub swirl: Option<f32>,
    /// Freshly planted: the folds' phase, and how much of them is left (1 to 0).
    pub ripple: Option<(f32, f32)>,
    /// A bar over the moon saying this: SEE SETTINGS.
    pub bar: Option<&'static str>,
}

fn shade(c: Rgb, k: f32) -> Rgb {
    c.map(|v| (f32::from(v) * k).clamp(0.0, 255.0) as u8)
}

fn blend(c: Rgb, to: Rgb, a: f32) -> Rgb {
    std::array::from_fn(|i| (f32::from(c[i]) * (1.0 - a) + f32::from(to[i]) * a) as u8)
}

/// The canvas, with the mock's units mapped onto it: `s` pixels a unit, from (`ox`, `oy`).
struct Paint<'c, 'a> {
    c: &'c mut Canvas<'a>,
    s: f32,
    ox: f32,
    oy: f32,
}

impl Paint<'_, '_> {
    fn px(&self, x: f32) -> i64 {
        (self.ox + x * self.s).round() as i64
    }

    fn py(&self, y: f32) -> i64 {
        (self.oy + y * self.s).round() as i64
    }

    fn set(&mut self, x: i64, y: i64, col: Rgb) {
        if x >= 0 && y >= 0 {
            self.c.set(x as usize, y as usize, col);
        }
    }

    fn get(&self, x: i64, y: i64) -> Rgb {
        if x < 0 || y < 0 || x as usize >= self.c.w || y as usize >= self.c.h {
            return SPACE;
        }
        let at = (y as usize * self.c.w + x as usize) * 4;
        [self.c.px[at + 2], self.c.px[at + 1], self.c.px[at]]
    }

    /// A rectangle in units: every pixel it covers, and at least one.
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, col: Rgb) {
        let (x0, y0) = (self.px(x), self.py(y));
        let (x1, y1) = (self.px(x + w).max(x0 + 1), self.py(y + h).max(y0 + 1));
        for py in y0..y1 {
            for px in x0..x1 {
                self.set(px, py, col);
            }
        }
    }

    fn dot(&mut self, x: f32, y: f32, col: Rgb) {
        self.rect(x, y, 1.0, 1.0, col);
    }

    /// The same, the other way round: the astronaut's drawn facing right, and stands facing left.
    fn rect_m(&mut self, x: f32, y: f32, w: f32, h: f32, col: Rgb) {
        self.rect(W - x - w, y, w, h, col);
    }

    fn dot_m(&mut self, x: f32, y: f32, col: Rgb) {
        self.rect_m(x, y, 1.0, 1.0, col);
    }

    /// Every pixel whose middle, in units, `inside` takes: for round things, smooth at any size.
    fn shape(&mut self, (x0, y0, x1, y1): (f32, f32, f32, f32), inside: impl Fn(f32, f32) -> Option<Rgb>) {
        for py in self.py(y0)..self.py(y1) + 1 {
            for px in self.px(x0)..self.px(x1) + 1 {
                let (u, v) = ((px as f32 + 0.5 - self.ox) / self.s, (py as f32 + 0.5 - self.oy) / self.s);
                if let Some(col) = inside(u, v) {
                    self.set(px, py, col);
                }
            }
        }
    }

    /// How many pixels a dot of the letters is: `big` dots a unit, in whole pixels.
    fn dots(&self, big: f32) -> i64 {
        ((big * self.s).floor() as i64).max(1)
    }

    /// `text` with its top left at pixel (`x`, `y`), `d` pixels a dot, with an edge of `edge` round it if there's one.
    fn text(&mut self, text: &str, x: i64, y: i64, d: i64, col: Rgb, edge: Option<Rgb>) {
        let lit: Vec<(i64, i64)> = text
            .chars()
            .enumerate()
            .flat_map(|(i, ch)| glyph(ch).into_iter().enumerate().flat_map(move |(ry, row)| (0..3).filter(move |rx| row & (4 >> rx) != 0).map(move |rx| ((i as i64 * 4 + rx) * d, ry as i64 * d))))
            .collect();
        let line = (d / 2).max(1);
        if let Some(edge) = edge {
            for (dx, dy) in &lit {
                for ey in -line..d + line {
                    for ex in -line..d + line {
                        self.set(x + dx + ex, y + dy + ey, edge);
                    }
                }
            }
        }
        for (dx, dy) in &lit {
            for ey in 0..d {
                for ex in 0..d {
                    self.set(x + dx + ex, y + dy + ey, col);
                }
            }
        }
    }
}

fn text_width(text: &str, d: i64) -> i64 {
    (text.chars().count() as i64 * 4 - 1).max(0) * d
}

/// The sky, Earth, the ground, its craters and his boot prints.
fn scene(p: &mut Paint) {
    for (x, y, dim) in STARS {
        p.dot(x, y, if dim { STAR_DIM } else { STAR });
    }
    // Earth, risen over the astronaut's shoulder.
    p.shape((54.0, 1.0, 63.0, 10.0), |u, v| {
        let (x, y) = (u - 54.0, v - 1.0);
        let round = (x - 4.5).powi(2) + (y - 4.5).powi(2) <= 16.5;
        let (cx, cy) = (x.floor() as i32, y.floor() as i32);
        round.then_some(if (cx + cy).rem_euclid(5) == 0 || (cx == 5 && (cy == 3 || cy == 4)) { LAND } else { EARTH })
    });
    // The ground, a low hill, out to the screen's edges.
    let (left, right) = (-p.ox / p.s, (p.c.w as f32 - p.ox) / p.s);
    let top = |u: f32| 40.0 + ((u - 36.5) / 34.0).powi(2) * 4.0;
    let edge = 1.0 / p.s;
    p.shape((left, 39.0, right, H + p.oy / p.s), |u, v| {
        let t = top(u);
        (v >= t).then_some(if v < t + edge.max(1.0) { GROUND_TOP } else { GROUND })
    });
    for (x, y) in [(24.0, 45.0), (57.0, 45.0)] {
        p.rect(x, y, 3.0, 1.0, CRATER);
        p.dot(x + 1.0, y - 1.0, CRATER);
    }
    for (x, y) in [(37.0, 42.0), (34.0, 43.0), (44.0, 43.0), (47.0, 42.0)] {
        p.rect(x, y, 2.0, 1.0, CRATER);
    }
}

fn astronaut(p: &mut Paint, lost: bool) {
    p.rect_m(6.0, 22.0, 3.0, 9.0, PACK);
    p.rect_m(9.0, 22.0, 8.0, 10.0, SUIT);
    p.rect_m(9.0, 22.0, 1.0, 10.0, SUIT_SHADE);
    p.rect_m(10.0, 32.0, 3.0, 6.0, SUIT);
    p.rect_m(14.0, 32.0, 3.0, 6.0, SUIT);
    p.rect_m(10.0, 32.0, 1.0, 6.0, SUIT_SHADE);
    p.rect_m(9.0, 38.0, 4.0, 2.0, BOOT);
    p.rect_m(14.0, 38.0, 4.0, 2.0, BOOT);
    // His helmet: round, at any size.
    p.shape((W - 19.0, 13.0, W - 8.0, 23.0), |u, v| ((W - u - 13.5).powi(2) + (v - 18.0).powi(2) <= 22.0).then_some(SUIT));
    p.rect_m(13.0, 15.0, 5.0, 4.0, VISOR);
    // No glint without a signal: nobody's looking.
    if !lost {
        p.dot_m(16.0, 15.0, GLINT);
        p.dot_m(17.0, 16.0, GLINT);
    }
    // His arm, up to the pole.
    for (x, y) in [(17.0, 24.0), (18.0, 23.0), (19.0, 22.0), (20.0, 21.0), (17.0, 25.0), (18.0, 24.0), (19.0, 23.0), (20.0, 22.0)] {
        p.dot_m(x, y, SUIT);
    }
    p.rect_m(20.0, 20.0, 2.0, 2.0, SUIT_SHADE);
}

/// The pole, into the ground, and Apollo's bar along the flag's top: there's no wind on the moon.
fn pole(p: &mut Paint) {
    p.rect(POLE_X, 4.0, 1.0, 38.0, POLE);
    bar(p);
}

fn bar(p: &mut Paint) {
    p.rect(FX - 1.0, 5.0, POLE_X - FX + 2.0, 1.0, POLE);
}

/// The flag's pixels: left, top, right and bottom, the bottom and right not in it.
fn flag_box(p: &Paint) -> (i64, i64, i64, i64) {
    (p.px(FX), p.py(FY), p.px(FX + FW), p.py(FY + FH))
}

fn flag(p: &mut Paint, s: &Scene) {
    let (l, t, r, b) = flag_box(p);
    let (fill, dark) = match s.fill {
        Fill::Urine => (PEE, PEE_DARK),
        Fill::Waste => (WASTE, WASTE_DARK),
        Fill::Water => (WATER, WATER_DARK),
    };
    // From the hem up to the level, and its surface a shade darker.
    let level = (b as f32 - (b - t) as f32 * s.level.clamp(0.0, 100.0) / 100.0).round() as i64;
    let line = (p.s.round() as i64).max(1);
    for y in t..b {
        for x in l..r {
            let inside = y >= level;
            let col = if s.lost {
                if inside { shade(GREY_LIGHT, 0.8) } else { GREY }
            } else if inside {
                if y < level + line && s.level > 0.0 { dark } else { fill }
            } else {
                NAVY
            };
            p.set(x, y, col);
        }
    }
    // Its free end and its hem, a shade darker.
    for y in t..b {
        for x in l..l + line {
            let c = p.get(x, y);
            p.set(x, y, shade(c, 0.75));
        }
    }
    for y in b - line..b {
        for x in l + line..r {
            let c = p.get(x, y);
            p.set(x, y, shade(c, 0.75));
        }
    }
    if s.bubbles && !s.lost {
        let filled = (FH * s.level / 100.0).round() as i32;
        for i in 0..6 {
            let bx = FX + 3.0 + ((i * 7 + 6) % (FW as i32 - 6)) as f32;
            let by = FY + FH - 3.0 - ((i * 5 + 4) % (filled - 4).max(1)) as f32;
            p.dot(bx, by, WHITE);
            p.dot(bx + 1.0, by, BUBBLE);
        }
    }
    if let Some(turn) = s.swirl {
        let (cx, cy) = (FX + (FW / 2.0).floor(), FY + (FH / 2.0).floor() + 2.0);
        let steps = (220.0 * p.s) as i32;
        for k in 0..steps {
            let k = k as f32 / p.s;
            let (a, r) = (k * 0.09 + turn, 11.0 - k * 0.05);
            if r <= 0.0 {
                break;
            }
            let (x, y) = (cx + a.cos() * r * 1.6, cy + a.sin() * r * 0.8);
            p.dot(x.round(), y.round(), PEE);
            p.dot(x.round(), y.round() + 1.0, PEE_DARK);
        }
    }
    let (big, small) = (p.dots(2.0), p.dots(1.0));
    let tall = 5 * big + s.small.as_ref().map_or(0, |_| 3 * small + 5 * small);
    let top = t + ((b - t) - tall) / 2;
    let words = if s.lost { LOST_WORDS } else { WHITE };
    p.text(&s.words, l + ((r - l) - text_width(&s.words, big)) / 2, top, big, words, Some(BLACK));
    if let Some(under) = &s.small {
        p.text(under, l + ((r - l) - text_width(under, small)) / 2, top + 5 * big + 3 * small, small, if s.lost { [190, 192, 200] } else { WHITE }, Some(BLACK));
    }
    if s.full && !s.lost {
        p.rect(FX + 2.0, FY + 1.0, 3.0, 3.0, AMBER);
    }
}

/// Freshly planted: folds of light and shade, more the further from the pole, and the hem rising and falling.
fn ripple(p: &mut Paint, phase: f32, amount: f32) {
    let (l, t, r, b) = flag_box(p);
    let line = (p.s.round() as i64).max(1);
    let span = (r - l - 1).max(1) as f32;
    for x in l..r {
        let from_pole = (r - 1 - x) as f32;
        let reach = from_pole / span;
        let fold = (from_pole / p.s * 0.5 + phase).sin() * reach * amount;
        for y in t..b {
            let c = p.get(x, y);
            p.set(x, y, shade(c, 1.0 + 0.16 * fold));
        }
        if fold > 0.5 {
            let hem = p.get(x, b - 1);
            for y in b..b + line {
                p.set(x, y, hem);
            }
        } else if fold < -0.5 {
            for y in b - line..b {
                p.set(x, y, SPACE);
            }
        }
    }
}

/// PTV's bug: white, seen through, with a shadow down and to the right.
fn bug(p: &mut Paint) {
    let d = p.dots(1.0);
    let (x0, y0) = (p.px(2.0), p.c.h as i64 - p.oy.round() as i64 - 8 * d);
    let lit: Vec<(i64, i64)> = BUG.iter().enumerate().flat_map(|(y, row)| row.bytes().enumerate().filter(|(_, b)| *b != b'.').map(move |(x, _)| (x as i64, y as i64))).collect();
    for (x, y) in &lit {
        if !lit.contains(&(x + 1, y + 1)) {
            for ey in 0..d {
                for ex in 0..d {
                    let (px, py) = (x0 + (x + 1) * d + ex, y0 + (y + 1) * d + ey);
                    let c = p.get(px, py);
                    p.set(px, py, blend(c, BLACK, 0.55));
                }
            }
        }
    }
    for (x, y) in &lit {
        for ey in 0..d {
            for ex in 0..d {
                let (px, py) = (x0 + x * d + ex, y0 + y * d + ey);
                let c = p.get(px, py);
                p.set(px, py, blend(c, WHITE, 0.78));
            }
        }
    }
}

/// A bar across the picture, over the moon, saying `words`: as a set says what's wrong.
fn notice(p: &mut Paint, words: &str) {
    let d = p.dots(1.0);
    let (w, h) = (text_width(words, d) + 6 * d, 9 * d);
    let (x, y) = ((p.c.w as i64 - w) / 2, p.py(FY + FH) + 2 * d);
    for py in y..y + h {
        for px in x..x + w {
            p.set(px, py, [14, 72, 178]);
        }
    }
    p.text(words, x + 3 * d, y + 2 * d, d, WHITE, None);
}

/// Draws `s` on the whole of `c`.
pub(crate) fn draw(c: &mut Canvas, s: &Scene) {
    c.fill(SPACE);
    let scale = (c.w as f32 / W).min(c.h as f32 / H).max(0.01);
    let (ox, oy) = (((c.w as f32 - W * scale) / 2.0).floor(), ((c.h as f32 - H * scale) / 2.0).floor());
    let mut p = Paint { c, s: scale, ox, oy };
    scene(&mut p);
    astronaut(&mut p, s.lost);
    pole(&mut p);
    flag(&mut p, s);
    if let Some((phase, amount)) = s.ripple {
        ripple(&mut p, phase, amount);
    }
    bar(&mut p);
    if let Some(words) = s.bar {
        notice(&mut p, words);
    }
    bug(&mut p);
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn live(level: f32) -> Scene {
        Scene { level, fill: Fill::Urine, words: format!("{level}%"), small: None, lost: false, bubbles: false, full: false, swirl: None, ripple: None, bar: None }
    }

    fn at(px: &[u8], w: usize, x: usize, y: usize) -> Rgb {
        let i = (y * w + x) * 4;
        [px[i + 2], px[i + 1], px[i]]
    }

    fn drawn(s: &Scene, w: usize, h: usize) -> Vec<u8> {
        let mut px = vec![0; w * h * 4];
        draw(&mut Canvas { w, h, px: &mut px }, s);
        px
    }

    #[test]
    fn the_flag_fills_from_its_hem_to_the_level() {
        let px = drawn(&live(29.0), 64, 48);
        // Near its hem it's pee; near its top, navy; the corner by the pole's bar is the bar.
        assert_eq!(at(&px, 64, 38, 29), PEE);
        assert_eq!(at(&px, 64, 38, 8), NAVY);
        assert_eq!(at(&px, 64, 41, 20), POLE);
        assert_eq!(at(&px, 64, 20, 5), POLE, "Apollo's bar");
        let full = drawn(&live(100.0), 64, 48);
        assert_eq!(at(&full, 64, 38, 8), PEE);
        let empty = drawn(&live(0.0), 64, 48);
        assert_eq!((at(&empty, 64, 38, 29), at(&empty, 64, 38, 31)), (NAVY, shade(NAVY, 0.75)), "empty: navy to the hem, a shade darker");
    }

    #[test]
    fn every_state_looks_different() {
        let base = live(61.0);
        let looks = [
            base.clone(),
            Scene { bubbles: true, ..base.clone() },
            Scene { full: true, level: 94.0, ..base.clone() },
            Scene { lost: true, words: "LOS".into(), small: Some("61%".into()), ..base.clone() },
            Scene { swirl: Some(0.0), small: Some("FLUSH!".into()), level: 4.0, ..base.clone() },
            Scene { ripple: Some((1.1, 1.0)), ..base.clone() },
            Scene { fill: Fill::Waste, ..base.clone() },
            Scene { fill: Fill::Water, ..base.clone() },
            Scene { bar: Some("SEE SETTINGS"), ..base.clone() },
        ];
        let pictures: Vec<Vec<u8>> = looks.iter().map(|s| drawn(s, 64, 48)).collect();
        for (i, a) in pictures.iter().enumerate() {
            for b in &pictures[i + 1..] {
                assert_ne!(a, b);
            }
        }
        // Lost, the flag's grey and his visor has no glint.
        let lost = &pictures[3];
        assert!(lost.chunks(4).all(|p| [p[2], p[1], p[0]] != GLINT));
        assert!(pictures[0].chunks(4).any(|p| [p[2], p[1], p[0]] == GLINT));
        assert!(pictures[2].chunks(4).any(|p| [p[2], p[1], p[0]] == AMBER), "nearly full: the amber light");
    }

    #[test]
    fn it_draws_at_any_size_with_more_detail_not_bigger_pixels() {
        for (w, h) in [(64, 48), (80, 60), (96, 72), (112, 84), (128, 96), (85, 48), (3, 2), (1, 1), (256, 256)] {
            let px = drawn(&Scene { ripple: Some((0.5, 1.0)), swirl: Some(1.0), bubbles: true, small: Some("FLUSH!".into()), bar: Some("SEE SETTINGS"), ..live(50.0) }, w, h);
            assert_eq!(px.len(), w * h * 4);
        }
        // At twice the size, the pee is where it was, at twice the place, and the letters are two pixels a dot.
        let big = drawn(&live(29.0), 128, 96);
        assert_eq!(at(&big, 128, 76, 58), PEE);
        assert_eq!(at(&big, 128, 76, 16), NAVY);
        let colors = |px: &[u8]| px.chunks(4).map(|p| [p[0], p[1], p[2]]).collect::<std::collections::HashSet<_>>().len();
        assert!(colors(&drawn(&live(29.0), 112, 84)) >= colors(&drawn(&live(29.0), 64, 48)) - 2);
    }

    #[test]
    fn every_word_it_says_has_letters() {
        for word in ["0123456789%", "LOS", "FLUSH!", "??", "...", "OFF", "SEE SETTINGS", "WASTE", "DRINK"] {
            for ch in word.chars().filter(|c| *c != ' ') {
                assert_ne!(glyph(ch), [0; 5], "{ch}");
            }
        }
        assert!(text_width("100%", 2) <= FW as i64);
        assert!(text_width("SEE SETTINGS", 1) + 6 <= W as i64);
    }
}
