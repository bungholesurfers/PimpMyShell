//! What a Twatch TV draws itself, in the manner of a flat-screen set from around 2010: the input's panel in the corner, a blue bar
//! when there's nothing to show, the same panel when the channel changes, and a very small light when it's playing below the
//! quality it could.

use crate::tv::{Canvas, text_width};

/// What the set calls its input.
const INPUT: &str = "TWATCH TV";
const NO_CLOCK: &str = "NO TIME INFO";
const NO_SIGNAL: &str = "NO SIGNAL ;(";
const STANDBY: &str = "STANDBY SOON...";

const BLACK: [u8; 3] = [0, 0, 0];
const PANEL: [u8; 3] = [10, 24, 78];
const EDGE: [u8; 3] = [186, 198, 226];
const NAME: [u8; 3] = [255, 150, 40];
const WORDS: [u8; 3] = [255, 255, 255];
const BAR: [u8; 3] = [14, 72, 178];
const LOWERED: [u8; 3] = [255, 176, 32];
const PURPLE: [u8; 3] = [130, 70, 235];
const PURPLE_LIGHT: [u8; 3] = [190, 150, 255];
/// How many pixels a dot of its letters is, at this height.
fn dot(c: &Canvas) -> usize {
    (c.h / 48).max(1)
}

fn rect(c: &mut Canvas, x: usize, y: usize, w: usize, h: usize, color: [u8; 3]) {
    for py in y..y + h {
        for px in x..x + w {
            c.set(px, py, color);
        }
    }
}

/// A box with a thin edge.
fn panel(c: &mut Canvas, x: usize, y: usize, w: usize, h: usize, fill: [u8; 3]) {
    rect(c, x, y, w, h, EDGE);
    rect(c, x + 1, y + 1, w.saturating_sub(2), h.saturating_sub(2), fill);
}

/// As much of `text` as fits in `room` pixels.
fn cut(text: &str, room: usize, s: usize) -> String {
    let fits = (room / s + 1) / 4;
    text.chars().take(fits).collect()
}

/// The screen with nothing on: the input's panel saying `word`, over black, and the blue bar if something's wrong.
fn plain(c: &mut Canvas, word: &str, trouble: bool) {
    c.px.fill(0);
    rect(c, 0, 0, c.w, c.h, BLACK);
    panels(c, word, trouble.then_some(NO_SIGNAL));
}

/// The input's panel saying `word`, and the blue bar saying `bar`, over what's there.
fn panels(c: &mut Canvas, word: &str, bar: Option<&str>) {
    let s = dot(c);
    let (at, pad) = (2 * s, 2 * s);
    let word = cut(word, c.w.saturating_sub(4 * pad), s);
    let inner = text_width(&word, s) + 4 * s;
    let width = text_width(INPUT, s).max(inner) + 2 * pad;
    let height = pad + 5 * s + 2 * s + 9 * s + pad;
    panel(c, at, at, width, height, PANEL);
    c.text(INPUT, at + pad, at + pad, s, NAME, false);
    let (box_x, box_y, box_w) = (at + pad, at + pad + 7 * s, width - 2 * pad);
    panel(c, box_x, box_y, box_w, 9 * s, PANEL);
    c.text(&word, box_x + box_w.saturating_sub(text_width(&word, s)) / 2, box_y + 2 * s, s, WORDS, false);
    if let Some(bar) = bar {
        let bar = cut(bar, c.w.saturating_sub(8 * s), s);
        let (w, h) = (text_width(&bar, s) + 8 * s, 9 * s);
        let (x, y) = (c.w.saturating_sub(w) / 2, (c.h * 5 / 8).min(c.h.saturating_sub(h + s)));
        rect(c, x, y, w, h, BAR);
        c.text(&bar, x + 4 * s, y + 2 * s, s, WORDS, false);
    }
}

/// There's a problem with playback.
pub fn no_signal(c: &mut Canvas) {
    plain(c, NO_CLOCK, true);
}

/// Somebody's live, but there's no Streamlink to play them: the bar says where to get it.
pub fn no_streamlink(c: &mut Canvas) {
    c.px.fill(0);
    rect(c, 0, 0, c.w, c.h, BLACK);
    panels(c, "NO STREAMLINK", Some("SEE SETTINGS"));
}

/// Somebody's live, and video is wanted, but the player that plays it, part of PimpMyShell, is missing.
pub fn no_player(c: &mut Canvas) {
    c.px.fill(0);
    rect(c, 0, 0, c.w, c.h, BLACK);
    panels(c, "NO PLAYER", Some("REINSTALL PMS"));
}

/// Nothing's wrong, there's only nothing to show, and `word` says why.
pub fn standby(c: &mut Canvas, word: &str) {
    plain(c, word, false);
}

/// Where the little logo of an idle set is, which way it's going, and the logo itself at the size it's drawn.
#[derive(Default)]
pub struct Bounce {
    at: Option<(f32, f32)>,
    heading: (f32, f32),
    logo: Option<Sprite>,
}

/// The logo's shape, wide for its height.
const LOGO_WIDE: f32 = 176.0 / 72.0;

/// A picture kept `FINE` times finer than the screen, each of its dots a color and how much of it there is, so that it can stand
/// between pixels and glide, as the crab on channel 3 does, and lie over anything with soft edges.
pub struct Sprite {
    width: usize,
    height: usize,
    /// Each fine dot's color, already scaled by its cover, and the cover, from 0 to 1.
    fine: Vec<[f32; 4]>,
}

impl Sprite {
    /// The Twatch TV logo, `width` by `height` screen pixels. It's drawn twice, over black and over white: where the two differ
    /// is how much of the logo there is, and over black is its color at that.
    fn logo(width: usize, height: usize) -> Sprite {
        let fine = crate::tv::FINE;
        let (fw, fh) = (width * fine, height * fine);
        let mut over_black = vec![[0u8; 3]; fw * fh];
        let mut over_white = vec![[0u8; 3]; fw * fh];
        pms_screen::logo(fw, fh, 1.0, BLACK, |x, y, color| over_black[y * fw + x] = color);
        pms_screen::logo(fw, fh, 1.0, WORDS, |x, y, color| over_white[y * fw + x] = color);
        let fine = over_black
            .iter()
            .zip(&over_white)
            .map(|(b, w)| {
                let shows = (0..3).map(|i| f32::from(w[i]) - f32::from(b[i])).sum::<f32>() / (3.0 * 255.0);
                let cover = (1.0 - shows).clamp(0.0, 1.0);
                [f32::from(b[0]), f32::from(b[1]), f32::from(b[2]), cover]
            })
            .collect();
        Sprite { width, height, fine }
    }

    /// Lays it over the canvas with its corner at (`x`, `y`), which needn't be on a pixel.
    fn draw(&self, c: &mut Canvas, x: f32, y: f32) {
        let fine = crate::tv::FINE as i32;
        let (fw, fh) = ((self.width as i32) * fine, (self.height as i32) * fine);
        let (left, top) = ((x * fine as f32).round() as i32, (y * fine as f32).round() as i32);
        let dots = (fine * fine) as f32;
        for py in top.div_euclid(fine)..=(top + fh).div_euclid(fine) {
            for px in left.div_euclid(fine)..=(left + fw).div_euclid(fine) {
                if px < 0 || py < 0 || px as usize >= c.w || py as usize >= c.h {
                    continue;
                }
                let mut sum = [0f32; 4];
                for dy in 0..fine {
                    for dx in 0..fine {
                        let (sx, sy) = (px * fine + dx - left, py * fine + dy - top);
                        if sx >= 0 && sy >= 0 && sx < fw && sy < fh {
                            let dot = self.fine[(sy * fw + sx) as usize];
                            for i in 0..4 {
                                sum[i] += dot[i];
                            }
                        }
                    }
                }
                let cover = sum[3] / dots;
                if cover <= 0.0 {
                    continue;
                }
                let at = (py as usize * c.w + px as usize) * 4;
                let pixel = &mut c.px[at..at + 3];
                // BGRA: the color over what's there, as much as the logo covers.
                for (i, v) in pixel.iter_mut().enumerate() {
                    let ours = sum[2 - i] / dots;
                    *v = (ours + f32::from(*v) * (1.0 - cover)).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

/// A set with nobody to show, and `word` saying why: a little logo going round the screen as a DVD player's did when left alone,
/// `dt` milliseconds on from the last frame, under the input's panel and the bar that says there's no signal, or, `warn`, that
/// it's about to go to standby.
pub fn idle(c: &mut Canvas, word: &str, b: &mut Bounce, dt: i64, warn: bool) {
    c.px.fill(0);
    rect(c, 0, 0, c.w, c.h, BLACK);
    let lh = (c.h / 5).max(4);
    let lw = ((lh as f32 * LOGO_WIDE) as usize).min(c.w);
    if b.logo.as_ref().is_none_or(|l| l.height != lh || l.width != lw) {
        b.logo = Some(Sprite::logo(lw, lh));
    }
    let room = (c.w.saturating_sub(lw) as f32, c.h.saturating_sub(lh) as f32);
    let (mut x, mut y) = b.at.unwrap_or_else(|| {
        b.heading = (1.0, 0.8);
        (room.0 * 0.6, room.1 * 0.7)
    });
    // Across the screen in about six seconds.
    let speed = c.w as f32 / 6000.0 * dt as f32;
    x += b.heading.0 * speed;
    y += b.heading.1 * speed;
    if x <= 0.0 || x >= room.0 {
        b.heading.0 = -b.heading.0;
        x = x.clamp(0.0, room.0);
    }
    if y <= 0.0 || y >= room.1 {
        b.heading.1 = -b.heading.1;
        y = y.clamp(0.0, room.1);
    }
    b.at = Some((x, y));
    if let Some(logo) = &b.logo {
        logo.draw(c, x, y);
    }
    panels(c, word, Some(if warn { STANDBY } else { NO_SIGNAL }));
}

/// The channel, over the picture for a moment.
pub fn channel(c: &mut Canvas, handle: &str) {
    let s = dot(c);
    let (at, pad) = (2 * s, 2 * s);
    let name = cut(&handle.to_ascii_uppercase(), c.w.saturating_sub(2 * at + 2 * pad), s);
    let width = text_width(INPUT, s).max(text_width(&name, s)) + 2 * pad;
    panel(c, at, at, width, pad + 5 * s + 2 * s + 5 * s + pad, PANEL);
    c.text(INPUT, at + pad, at + pad, s, NAME, false);
    c.text(&name, at + pad, at + pad + 7 * s, s, WORDS, false);
}

/// Starting up: the logo, fading in and out, `left` of `total` milliseconds to go.
pub fn boot(c: &mut Canvas, left: i64, total: i64) {
    const FADE: f32 = 160.0;
    let bright = ((left as f32).min((total - left) as f32) / FADE).clamp(0.0, 1.0);
    let (w, h) = (c.w, c.h);
    pms_screen::logo(w, h, bright, BLACK, |x, y, color| c.set(x, y, color));
}

/// A set that's kept to one channel, and it isn't live.
pub fn waiting(c: &mut Canvas, handle: &str) {
    plain(c, &handle.to_ascii_uppercase(), true);
}

/// A plus: its middle, how far each arm reaches from it, and how thick the arms are.
#[derive(Clone, Copy)]
struct Plus {
    cx: usize,
    cy: usize,
    arm: usize,
    thick: usize,
}

impl Plus {
    fn covers(self, x: isize, y: isize) -> bool {
        let (half, rest) = (self.thick / 2, self.thick - self.thick / 2);
        // The arms reach as far each way from the middle of the bar, which an odd thickness puts on a pixel, not between two.
        let across = |p: isize, c: usize, reach: usize| p >= c as isize - reach as isize && p < (c + reach + rest - half) as isize;
        let along = |p: isize, c: usize| p >= c as isize - half as isize && p < (c + rest) as isize;
        (across(x, self.cx, self.arm) && along(y, self.cy)) || (along(x, self.cx) && across(y, self.cy, self.arm))
    }

    /// Whether (x, y) is on its edge: in it, with something beside it that isn't.
    fn edge(self, x: isize, y: isize) -> bool {
        self.covers(x, y) && [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)].iter().any(|&(a, b)| !self.covers(a, b))
    }
}

#[cfg(test)]
mod plus_tests {
    use super::Plus;

    #[test]
    fn every_arm_of_the_plus_is_as_long_as_the_others() {
        for thick in 1..9 {
            for arm in thick..20 {
                let p = Plus { cx: 30, cy: 30, arm, thick };
                let row: Vec<isize> = (0..70).filter(|&x| p.covers(x, 30)).collect();
                let column: Vec<isize> = (0..70).filter(|&y| p.covers(30, y)).collect();
                let bar: Vec<isize> = (0..70).filter(|&x| p.covers(x, 0) || p.covers(x, 30 - arm as isize)).collect();
                let (left, right) = (bar[0] - row[0], row[row.len() - 1] - bar[bar.len() - 1]);
                let (top, bottom) = (bar[0] - column[0], column[column.len() - 1] - bar[bar.len() - 1]);
                assert_eq!((left, top), (right, bottom), "thick {thick}, arm {arm}: left/right and top/bottom");
                assert_eq!(left, top, "thick {thick}, arm {arm}");
            }
        }
    }
}

/// `color` at `alpha` over what's at (x, y) already, which may be see-through.
fn over(c: &mut Canvas, x: usize, y: usize, [r, g, b]: [u8; 3], alpha: f32) {
    if x >= c.w || y >= c.h || alpha <= 0.0 {
        return;
    }
    let at = (y * c.w + x) * 4;
    let p = &mut c.px[at..at + 4];
    let keep = 1.0 - alpha;
    for (v, add) in p[..3].iter_mut().zip([b, g, r]) {
        *v = (f32::from(add) * alpha + f32::from(*v) * keep).round() as u8;
    }
    p[3] = (255.0 * alpha + f32::from(p[3]) * keep).round() as u8;
}

/// The button beside the sets that adds one: with how many more favorites are live to say so, and a plus beneath, or, put away,
/// only the plus. `faded`, from 0 to 1, is how far the put-away plus has gone from white on purple to a purple outline on nothing.
pub fn more(c: &mut Canvas, how_many: Option<usize>, faded: f32) {
    c.px.fill(0);
    let s = dot(c);
    match how_many {
        Some(n) => {
            rect(c, 0, 0, c.w, c.h, PURPLE);
            let first = format!("+{n} MORE");
            let lines = [first.as_str(), if n == 1 { "FAV ONLINE" } else { "FAVS ONLINE" }];
            // The words, and the plus that's all of this button once it's put away, so that it's known for what it is.
            let (arm, thick) = (4 * s, 2 * s);
            let tall = 13 * s + 3 * s + 2 * arm;
            let top = c.h.saturating_sub(tall) / 2;
            for (i, line) in lines.iter().enumerate() {
                c.text(line, c.w.saturating_sub(text_width(line, s)) / 2, top + i * 8 * s, s, WORDS, false);
            }
            let plus = Plus { cx: c.w / 2, cy: top + 16 * s + arm, arm, thick };
            for y in 0..c.h {
                for x in 0..c.w {
                    if plus.covers(x as isize, y as isize) {
                        c.set(x, y, WORDS);
                    }
                }
            }
        }
        None => {
            let side = c.w.min(c.h);
            let plus = Plus { cx: c.w / 2, cy: c.h / 2, arm: (side * 3 / 8).max(2), thick: (side / 6).max(1) };
            let (solid, outline) = (1.0 - faded.clamp(0.0, 1.0), faded.clamp(0.0, 1.0));
            for y in 0..c.h {
                for x in 0..c.w {
                    let (px, py) = (x as isize, y as isize);
                    over(c, x, y, PURPLE, solid);
                    if plus.covers(px, py) {
                        over(c, x, y, WORDS, solid);
                    }
                    if plus.edge(px, py) {
                        over(c, x, y, PURPLE, outline);
                    }
                }
            }
        }
    }
}

/// The button that says sets come by themselves, and leads to the setting.
pub fn automatic(c: &mut Canvas) {
    c.px.fill(0);
    let (w, h) = (c.w, c.h);
    rect(c, 0, 0, w, h, PURPLE);
    rect(c, 1, 1, w.saturating_sub(2), h.saturating_sub(2), BLACK);
    let s = (w.min(h) / 7).max(1);
    c.text("A", w.saturating_sub(3 * s) / 2, h.saturating_sub(5 * s) / 2, s, PURPLE_LIGHT, false);
}

/// The light for playing below the set's best.
pub fn lowered(c: &mut Canvas) {
    let s = dot(c);
    let side = 2 * s;
    let (x, y) = (c.w.saturating_sub(side + s), c.h.saturating_sub(side + s));
    rect(c, x.saturating_sub(1), y.saturating_sub(1), side + 2, side + 2, BLACK);
    rect(c, x, y, side, side, LOWERED);
}

/// A still picture, RGB, `width` by `height`, averaged down to fill the screen.
pub fn still(c: &mut Canvas, rgb: &[u8], width: usize, height: usize) {
    if width == 0 || height == 0 || rgb.len() < width * height * 3 {
        return;
    }
    let (out_w, out_h) = (c.w, c.h);
    for y in 0..out_h {
        let (top, bottom) = (y * height / out_h, ((y + 1) * height / out_h).max(y * height / out_h + 1).min(height));
        for x in 0..out_w {
            let (left, right) = (x * width / out_w, ((x + 1) * width / out_w).max(x * width / out_w + 1).min(width));
            let mut sum = [0u32; 3];
            for sy in top..bottom {
                for dot in rgb[(sy * width + left) * 3..(sy * width + right) * 3].as_chunks::<3>().0 {
                    for (s, v) in sum.iter_mut().zip(dot) {
                        *s += u32::from(*v);
                    }
                }
            }
            let n = ((bottom - top) * (right - left)).max(1) as u32;
            c.set(x, y, sum.map(|v| (v / n) as u8));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tv::glyph;

    const W: usize = 85;
    const H: usize = 48;

    fn rgb(px: &[u8], x: usize, y: usize) -> [u8; 3] {
        let at = (y * W + x) * 4;
        [px[at + 2], px[at + 1], px[at]]
    }

    #[test]
    fn no_signal_is_the_panel_in_the_corner_and_the_bar_in_the_middle() {
        let mut px = vec![0; W * H * 4];
        no_signal(&mut Canvas { w: W, h: H, px: &mut px });
        assert_eq!(rgb(&px, 2, 2), EDGE, "the panel's thin edge");
        assert_eq!(rgb(&px, 3, 3), PANEL);
        assert!(px.chunks(4).any(|p| [p[2], p[1], p[0]] == NAME), "the input's name");
        assert_eq!(rgb(&px, W / 2 - 24, 31), BAR, "the bar");
        assert_eq!(rgb(&px, 0, 0), BLACK);
        assert!(px.chunks(4).all(|p| p[3] == 255), "opaque");
    }

    #[test]
    fn the_logo_is_the_name_across_the_oval_and_it_fades() {
        let mut px = vec![0; W * H * 4];
        boot(&mut Canvas { w: W, h: H, px: &mut px }, 500, 1000);
        let count = |color: [u8; 3]| px.chunks(4).filter(|p| [p[2], p[1], p[0]] == color).count();
        assert!(count(pms_screen::LOGO_BLUE) > 300, "the oval: {}", count(pms_screen::LOGO_BLUE));
        assert!(count(pms_screen::LOGO_NAME) > 40, "the name: {}", count(pms_screen::LOGO_NAME));
        assert_eq!(rgb(&px, 0, 0), BLACK);
        assert_eq!(rgb(&px, W - 1, H - 1), BLACK);
        assert!(px.chunks(4).all(|p| p[3] == 255), "opaque");
        boot(&mut Canvas { w: W, h: H, px: &mut px }, 1000, 1000);
        assert!(px.chunks(4).all(|p| p[..3] == [0, 0, 0]), "it starts from nothing");
    }

    #[test]
    fn standing_by_has_no_bar() {
        let mut px = vec![0; W * H * 4];
        standby(&mut Canvas { w: W, h: H, px: &mut px }, "NOBODY LIVE");
        assert!(!px.chunks(4).any(|p| [p[2], p[1], p[0]] == BAR));
    }

    #[test]
    fn every_word_has_its_letters_and_fits() {
        for word in [INPUT, NO_CLOCK, NO_SIGNAL, "NOBODY LIVE", "NO FAVORITES", "NO STREAMLINK", "TUNING"] {
            for ch in word.chars().filter(|c| *c != ' ') {
                assert_ne!(glyph(ch), [0; 5], "{ch}");
            }
            assert!(text_width(word, 1) + 12 <= W, "{word}");
        }
        for ch in "abcdefghijklmnopqrstuvwxyz0123456789_".to_ascii_uppercase().chars() {
            assert_ne!(glyph(ch), [0; 5], "a handle can have {ch}");
        }
    }

    #[test]
    fn it_draws_at_any_size() {
        for (w, h) in [(85, 48), (107, 60), (170, 96), (3, 2), (1, 1)] {
            let mut px = vec![0; w * h * 4];
            let mut c = Canvas { w, h, px: &mut px };
            no_signal(&mut c);
            standby(&mut c, "NOBODY LIVE");
            boot(&mut c, 500, 1000);
            channel(&mut c, "a_very_long_handle_for_a_little_set");
            lowered(&mut c);
        }
    }

    #[test]
    fn a_still_is_averaged_down() {
        let (mut px, rgb_in) = (vec![0; 4 * 4], [100u8, 150, 200].repeat(8 * 8));
        still(&mut Canvas { w: 2, h: 2, px: &mut px }, &rgb_in, 8, 8);
        assert_eq!(px[..4], [200, 150, 100, 255]);
        still(&mut Canvas { w: 2, h: 2, px: &mut px }, &[], 8, 8);
    }
}
