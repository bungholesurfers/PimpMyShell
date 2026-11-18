//! PTV's bumps: what Number One shows between its pictures of the tank, as a channel's idents ran between its programmes. Seven
//! idents (the P as a thing in the world, after MTV's early IDs, a few seconds each) and four homages to a 1985 music video, 10 s
//! each, with the same astronaut. Scenes as code, frame by frame, as the design's Python prototype has them, drawn
//! by solid.rs at the screen's real size: a bigger display scale gets more detail, not bigger pixels. The letters and PTV's bug
//! are in whole pixels, one a dot below 200%, two from there. 15 frames a second.
//!
//! Nothing's random: what looks it (the flames, the smoke, the static, the blocks flying in) is `noise` of where and when.

use std::f32::consts::PI;
use std::sync::LazyLock;

use super::solid::{Cam, Ctx, Frame, Mat, Mesh, Rgb, Side, V3, add, all, at, clamp01, color, cube, dot, ease, hue, lerp, mix, mul, noise, norm, one, only, place, shade, sub, textured, turn};

pub(crate) const FPS: usize = 15;

const PEE: Rgb = [246, 210, 46];
const PEE_DARK: Rgb = [196, 150, 18];
const EMPTY: Rgb = [232, 232, 238];
const SIDE: Rgb = [30, 143, 214];
const RED: Rgb = [200, 32, 42];
const INK: Rgb = [0, 0, 0];
const NAVY: Rgb = [28, 34, 82];
const WHITE: Rgb = [255, 255, 255];
const SPACE_BG: Rgb = [4, 6, 16];
const SUIT: Rgb = [232, 232, 238];
const BOOT: Rgb = [110, 110, 122];
const PACK: Rgb = [170, 172, 186];
const GOLD: Rgb = [214, 152, 36];
const GLINT: Rgb = [255, 240, 200];
const SHADES: Rgb = [14, 14, 18];
const CABINET: Rgb = [86, 94, 130];
const TEAL: Rgb = [38, 128, 124];
const DROP: Rgb = [236, 200, 60];
const CEILING: Rgb = [22, 66, 74];
const CHECK_DARK: Rgb = [34, 34, 40];
const CHECK_LIGHT: Rgb = [214, 214, 222];
const PORCELAIN: Rgb = [240, 242, 246];
const EARTH_SEA: Rgb = [34, 84, 180];
const EARTH_LAND: Rgb = [70, 140, 70];
const CLOUD: Rgb = [236, 240, 246];
const AIR: Rgb = [120, 170, 240];
/// Powder blue, as an 80s bathroom's: a white toilet would vanish behind his suit. And a black seat.
const SEAT: Rgb = [26, 26, 30];
const BLUE_PORCELAIN: Rgb = [140, 196, 236];

const TAG: &str = "I WANT MY PTV";
const BUG: [&str; 6] = ["XXX.......", "X.X.......", "XXX.X.....", "X..XXX.X.X", "X...X..X.X", "X...XX..X."];

/// Every bump.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Tumble,
    Liftoff,
    Flush,
    Pieces,
    Skins,
    Squash,
    Stencil,
    Showroom,
    Spacewalk,
    Throne,
    Orbit,
}

impl Kind {
    pub const ALL: [Kind; 11] = [Kind::Tumble, Kind::Liftoff, Kind::Flush, Kind::Pieces, Kind::Skins, Kind::Squash, Kind::Stencil, Kind::Showroom, Kind::Spacewalk, Kind::Throne, Kind::Orbit];
    pub const IDENTS: [Kind; 7] = [Kind::Tumble, Kind::Liftoff, Kind::Flush, Kind::Pieces, Kind::Skins, Kind::Squash, Kind::Stencil];
    pub const HOMAGES: [Kind; 4] = [Kind::Showroom, Kind::Spacewalk, Kind::Throne, Kind::Orbit];
    /// The homages on a toilet, for a flush.
    pub const TOILETS: [Kind; 2] = [Kind::Throne, Kind::Orbit];

    /// As the prototype named it: `ident-tumble`, `homage-orbit`.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Tumble => "ident-tumble",
            Kind::Liftoff => "ident-liftoff",
            Kind::Flush => "ident-flush",
            Kind::Pieces => "ident-pieces",
            Kind::Skins => "ident-skins",
            Kind::Squash => "ident-squash",
            Kind::Stencil => "ident-stencil",
            Kind::Showroom => "homage-showroom",
            Kind::Spacewalk => "homage-spacewalk",
            Kind::Throne => "homage-throne",
            Kind::Orbit => "homage-orbit",
        }
    }

    /// By its name, or the end of it: `tumble`, `ident-tumble`.
    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name || k.name().split_once('-').is_some_and(|(_, short)| short == name))
    }

    /// How many frames it has, at 15 a second.
    pub fn frames(self) -> usize {
        match self {
            Kind::Tumble | Kind::Flush | Kind::Pieces => 69,
            Kind::Liftoff => 78,
            Kind::Skins => 72,
            Kind::Squash | Kind::Stencil => 75,
            Kind::Showroom | Kind::Spacewalk | Kind::Throne | Kind::Orbit => 150,
        }
    }

    pub fn is_homage(self) -> bool {
        Kind::HOMAGES.contains(&self)
    }

    /// Which it is, counting from 0: for asking for it by number.
    pub fn index(self) -> usize {
        Kind::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }
}

/// The screen: how many times the mock's size it is, its size, the size drawn (twice), and how many pixels a dot of the letters is.
struct Sc {
    s: f32,
    w: usize,
    h: usize,
    rw: usize,
    rh: usize,
    dot: i64,
}

impl Sc {
    fn new(w: usize, h: usize) -> Sc {
        let s = (w as f32 / 64.0).min(h as f32 / 48.0).max(0.05);
        Sc { s, w, h, rw: w * 2, rh: h * 2, dot: ((s + 1e-4).floor() as i64).max(1) }
    }

    fn cam(&self, eye: V3, target: V3, fov: f32) -> Cam {
        Cam::new(eye, target, fov, self.rw, self.rh)
    }

    fn frame(&self, bg: impl Fn(usize, usize) -> Rgb) -> Frame {
        Frame::new(self.rw, self.rh, bg)
    }
}

// --- The P ------------------------------------------------------------------------------------------------------------------------
// In the logo's units (its SVG's pixels), as four boxes: its stem, its top, the right of its bowl, the bottom of its bowl. In
// object units with the whole logo (P and tv) centred.
const U: f32 = 580.0;

fn lx(x: f32) -> f32 {
    (x - 120.0) / U - 0.675
}

fn ly(y: f32) -> f32 {
    (640.0 - y) / U - 0.5
}

const P_BOXES: [[f32; 4]; 4] = [[120.0, 60.0, 330.0, 640.0], [330.0, 60.0, 640.0, 166.0], [496.0, 166.0, 640.0, 410.0], [330.0, 304.0, 496.0, 410.0]];

fn p_front(c: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    let surface = c.fill + 0.035 * (o[0] * 15.0 + c.t * 11.0).sin();
    if o[1] < surface - 0.03 {
        PEE
    } else if o[1] < surface {
        PEE_DARK
    } else {
        EMPTY
    }
}

/// The P: each of its boxes' fronts, backs and sides, outlined.
fn p_mesh(front: Mat, back: Mat, side: Mat) -> Mesh {
    let mut out = Vec::new();
    for [x0, y0, x1, y1] in P_BOXES {
        out.extend(cube([lx(x0), ly(y1), -0.11], [lx(x1), ly(y0), 0.11], move |s| Some(match s {
            Side::Front => front,
            Side::Back => back,
            _ => side,
        }), true));
    }
    out
}

/// Filling up as it's told (`Ctx::fill`).
static P_MESH: LazyLock<Mesh> = LazyLock::new(|| p_mesh(textured(p_front).unlit(), color(PEE), color(SIDE)));
/// Full.
static P_FULL: LazyLock<Mesh> = LazyLock::new(|| p_mesh(color(PEE).unlit(), color(PEE), color(SIDE)));

/// The tv's three strokes, as points along them.
static TV_POINTS: LazyLock<Vec<Vec<V3>>> = LazyLock::new(|| {
    type Curve = ([f32; 2], [f32; 2], [f32; 2]);
    let paths: [(&[f32; 2], &[Curve]); 3] = [
        (&[562.0, 318.0], &[([556.0, 420.0], [548.0, 560.0], [566.0, 636.0]), ([574.0, 668.0], [604.0, 670.0], [634.0, 648.0])]),
        (&[424.0, 474.0], &[([494.0, 452.0], [576.0, 432.0], [664.0, 418.0])]),
        (&[664.0, 498.0], &[([690.0, 556.0], [704.0, 612.0], [722.0, 652.0]), ([764.0, 556.0], [824.0, 430.0], [902.0, 318.0])]),
    ];
    paths
        .iter()
        .map(|(start, curves)| {
            let (mut p, mut pts) = (**start, vec![**start]);
            for (c1, c2, p3) in curves.iter() {
                for i in 1..=20 {
                    let t = i as f32 / 20.0;
                    let s = 1.0 - t;
                    pts.push(std::array::from_fn(|k| s * s * s * p[k] + 3.0 * s * s * t * c1[k] + 3.0 * s * t * t * c2[k] + t * t * t * p3[k]));
                }
                p = *p3;
            }
            pts.iter().map(|q| [lx(q[0]), ly(q[1]), -0.12]).collect()
        })
        .collect()
});

/// How the tv's written: its color, its outline (if it has one), how fat, and how much it wobbles.
#[derive(Clone, Copy)]
struct Pen {
    col: Rgb,
    ink: Option<Rgb>,
    fat: f32,
    wobble: f32,
}

const SIGNED: Pen = Pen { col: RED, ink: Some(INK), fat: 1.0, wobble: 0.0 };

/// The tv, written on as far as `frac` of the way: each stroke in turn, its outline under it.
#[allow(clippy::too_many_arguments)]
fn scrawl(fr: &mut Frame, cam: &Cam, sc: &Sc, c: &Ctx, frac: f32, scale: V3, to: V3, pen: Pen) {
    let total: usize = TV_POINTS.iter().map(Vec::len).sum();
    let mut left = (total as f32 * frac) as i64;
    let mut shown: Vec<&[V3]> = Vec::new();
    for p in TV_POINTS.iter() {
        shown.push(&p[..left.clamp(0, p.len() as i64) as usize]);
        left -= p.len() as i64;
    }
    let layers: Vec<(Rgb, f32)> = match pen.ink {
        Some(ink) => vec![(ink, 1.6 * sc.s), (pen.col, 0.0)],
        None => vec![(pen.col, 0.0)],
    };
    for (col, extra) in layers {
        for stroke in &shown {
            for (n, p) in stroke.iter().enumerate() {
                let [x, y, z] = cam.screen(add([p[0] * scale[0], p[1] * scale[1], p[2]], to));
                if z <= 0.05 {
                    continue;
                }
                let r = (27.0 / U) / z * cam.big_f * pen.fat * (1.0 + pen.wobble * (n as f32 * 1.7 + c.t * 9.0).sin());
                fr.disc(x, y, r + extra, col);
            }
        }
    }
}

/// The P as it comes to rest, seen straight on.
fn rest_cam(sc: &Sc) -> Cam {
    sc.cam([0.0, 0.0, -2.05], [0.0, -0.02, 0.0], 45.0)
}

/// A drop of pee from the P's foot, `d` seconds after it left.
fn drop_from_foot(fr: &mut Frame, cam: &Cam, sc: &Sc, d: f32, scale: V3, to: V3) {
    let [x, y, _] = cam.screen(add([lx(225.0) * scale[0], ly(640.0) * scale[1], -0.11], to));
    let (s, y) = (sc.s, y + 6.0 * sc.s + 140.0 * sc.s * d * d);
    fr.disc(x, y - 3.0 * s, 2.2 * s, INK);
    fr.disc(x, y, 3.6 * s, INK);
    fr.disc(x, y - 3.0 * s, 1.2 * s, PEE);
    fr.disc(x, y, 2.6 * s, PEE);
}

// --- Backgrounds ------------------------------------------------------------------------------------------------------------------
fn sunburst(sc: &Sc, c1: Rgb, c2: Rgb, spin: f32, rays: f32) -> impl Fn(usize, usize) -> Rgb {
    let (cx, cy) = (sc.rw as f32 / 2.0, sc.rh as f32 / 2.0);
    move |x, y| {
        let a = (y as f32 - cy).atan2(x as f32 - cx) + spin;
        if ((a / (2.0 * PI)) * rays + 100.0) as i64 % 2 != 0 { c1 } else { c2 }
    }
}

fn solid(c: Rgb) -> impl Fn(usize, usize) -> Rgb {
    move |_, _| c
}

fn gradient(sc: &Sc, top: Rgb, bottom: Rgb) -> impl Fn(usize, usize) -> Rgb {
    let rows: Vec<Rgb> = (0..sc.rh).map(|y| mix(top, bottom, y as f32 / (sc.rh.max(2) - 1) as f32)).collect();
    move |_, y| rows[y]
}

/// A striped sunset over a neon grid rolling toward us.
fn synthwave(sc: &Sc, t: f32) -> impl Fn(usize, usize) -> Rgb {
    let (rw, rh) = (sc.rw as f32, sc.rh as f32);
    let (hz, sun_r) = (rh * 0.58, rh * 0.3);
    move |x, y| {
        let (fx, fy) = (x as f32, y as f32);
        if fy < hz {
            let (dx, dy) = (fx + 0.5 - rw / 2.0, fy + 0.5 - hz);
            if dx * dx + dy * dy < sun_r * sun_r {
                let band = (hz - fy) / sun_r;
                if band < 0.55 && (fy / (rh * 0.035)) as i64 % 2 == 0 {
                    return [40, 10, 70];
                }
                return mix([255, 70, 150], [255, 220, 60], band);
            }
            return mix([70, 10, 110], [20, 4, 40], 1.0 - fy / hz);
        }
        let depth = (rh * 0.6) / (fy - hz + 1.0);
        let gz = (depth + t * 3.0).rem_euclid(1.0);
        let gx = ((fx + 0.5 - rw / 2.0) / rw * depth * 1.8).rem_euclid(1.0);
        let line = 0.06 * depth.min(3.0);
        if gz < 0.08 * depth.powf(0.3) || gx < line || gx > 1.0 - line { [255, 60, 200] } else { [24, 6, 44] }
    }
}

/// Space: stars fixed in their directions, and Earth (which way, and how big) lit from one side, with its air at the rim.
fn sky<'a>(cam: &'a Cam, earth: Option<V3>, erad: f32) -> impl Fn(usize, usize) -> Rgb + 'a {
    let cosr = erad.cos();
    let sun = norm([0.7, 0.5, -0.5]);
    move |x, y| {
        let d = cam.ray(x, y);
        if let Some(earth) = earth {
            let c = dot(d, earth);
            if c > cosr {
                if (c - cosr) / (1.0 - cosr) < 0.05 {
                    return AIR;
                }
                let n = norm(sub(d, mul(earth, c)));
                let k = 0.45 + 0.55 * (dot(n, sun) * 0.8 + 0.2).max(0.0);
                let at = |m: f32, o: f32| [(d[0] * m + o) as i64, (d[1] * m + o) as i64, (d[2] * m + o) as i64];
                let (g, f) = (noise(&at(9.0, 40.0)), noise(&at(23.0, 90.0)));
                return shade(if f > 225 { CLOUD } else if g < 85 { EARTH_LAND } else { EARTH_SEA }, k);
            }
        }
        let n = noise(&[(d[0] * 70.0 + 300.0) as i64, (d[1] * 70.0 + 300.0) as i64, (d[2] * 70.0 + 300.0) as i64]);
        if n > 252 {
            [230, 230, 245]
        } else if n > 249 {
            [110, 112, 140]
        } else {
            SPACE_BG
        }
    }
}

/// Looking down into a toilet bowl, its water swirling (`calm`, 0 to 1, as it settles).
fn bowl(sc: &Sc, spin: f32, calm: f32) -> impl Fn(usize, usize) -> Rgb {
    let (rw, rh, tile) = (sc.rw as f32, sc.rh as f32, 12.0 * sc.s);
    move |x, y| {
        let (fx, fy) = (x as f32, y as f32);
        let (dx, dy) = ((fx + 0.5 - rw / 2.0) / (rw * 0.47), (fy + 0.5 - rh / 2.0) / (rh * 0.47));
        let r = dx.hypot(dy);
        if r > 1.0 {
            return if ((fx / tile) as i64 + (fy / tile) as i64) % 2 != 0 { [196, 218, 230] } else { [168, 192, 208] };
        }
        if r > 0.9 {
            return PORCELAIN;
        }
        if r > 0.84 {
            return [196, 198, 208];
        }
        let swirl = (dy.atan2(dx) * 3.0 + r * 10.0 * (1.0 - calm) - spin).sin();
        if r < 0.1 * (1.0 - calm) {
            return [150, 110, 10];
        }
        if swirl > 0.6 { PEE_DARK } else { PEE }
    }
}

// --- Letters, the bug -------------------------------------------------------------------------------------------------------------
/// What's shown: the picture, halved, at the screen's size.
struct Rows {
    px: Vec<Rgb>,
    w: i64,
    h: i64,
}

impl Rows {
    fn set(&mut self, x: i64, y: i64, c: Rgb) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.px[(y * self.w + x) as usize] = c;
        }
    }

    fn get(&self, x: i64, y: i64) -> Rgb {
        self.px[(y.clamp(0, self.h - 1) * self.w + x.clamp(0, self.w - 1)) as usize]
    }

    /// `text`, `d` pixels a dot, with an edge round it, across the middle unless `x` says where.
    fn text(&mut self, text: &str, y: i64, col: Rgb, d: i64, x: Option<i64>, ink: Option<Rgb>) {
        let x = x.unwrap_or((self.w - (text.chars().count() as i64 * 4 - 1) * d) / 2);
        let put = |rows: &mut Rows, ox: i64, oy: i64, c: Rgb| {
            for (i, ch) in text.chars().enumerate() {
                for (ry, bits) in crate::tv::glyph(ch).iter().enumerate() {
                    for rx in 0..3 {
                        if bits & (4 >> rx) == 0 {
                            continue;
                        }
                        for sy in 0..d {
                            for sx in 0..d {
                                rows.set(x + ox + (i as i64 * 4 + rx) * d + sx, y + oy + ry as i64 * d + sy, c);
                            }
                        }
                    }
                }
            }
        };
        if let Some(ink) = ink {
            for (ox, oy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                put(self, ox * d, oy * d, ink);
            }
        }
        put(self, 0, 0, col);
    }

    /// PTV's bug, in the bottom left: white, seen through, with a shadow.
    fn bug(&mut self, d: i64) {
        let (ox, oy) = (2 * d, self.h - 8 * d);
        let cells: Vec<(i64, i64)> = BUG.iter().enumerate().flat_map(|(y, r)| r.bytes().enumerate().filter(|(_, b)| *b == b'X').map(move |(x, _)| (x as i64, y as i64))).collect();
        for (x, y) in &cells {
            if !cells.contains(&(x + 1, y + 1)) {
                for sy in 0..d {
                    for sx in 0..d {
                        let (px, py) = (ox + (x + 1) * d + sx, oy + (y + 1) * d + sy);
                        if px < self.w && py < self.h {
                            let c = self.get(px, py);
                            self.set(px, py, shade(c, 0.45));
                        }
                    }
                }
            }
        }
        for (x, y) in &cells {
            for sy in 0..d {
                for sx in 0..d {
                    let (px, py) = (ox + x * d + sx, oy + y * d + sy);
                    let c = self.get(px, py);
                    self.set(px, py, c.map(|v| (f32::from(v) * 0.22 + 255.0 * 0.78) as u8));
                }
            }
        }
    }
}

fn halved(fr: &Frame, sc: &Sc) -> Rows {
    Rows { px: fr.halved(sc.w, sc.h), w: sc.w as i64, h: sc.h as i64 }
}

/// A homage's last touches: the tagline, once it's time, and the bug.
fn finish(fr: &Frame, sc: &Sc, t: f32, tag_from: f32) -> Rows {
    let mut rows = halved(fr, sc);
    if t >= tag_from {
        rows.text(TAG, 3 * sc.dot, WHITE, sc.dot, None, Some(INK));
    }
    rows.bug(sc.dot);
    rows
}

// --- The idents -------------------------------------------------------------------------------------------------------------------
fn tumble(sc: &Sc, f: usize) -> Rows {
    let t = f as f32 / FPS as f32;
    let cam = rest_cam(sc);
    let mut fr = if (1.2..1.27).contains(&t) {
        sc.frame(solid(WHITE))
    } else if t < 2.6 {
        sc.frame(sunburst(sc, [255, 70, 170], [140, 40, 210], t * 1.6, 14.0))
    } else {
        sc.frame(sunburst(sc, [40, 200, 220], [30, 110, 210], -t * 1.6, 14.0))
    };
    let (to, rot, scale) = if t < 1.2 {
        let s = 1.0 - (1.0 - t / 1.2).powi(3);
        ([0.0, 0.0, 7.0 * (1.0 - s)], [(1.0 - s) * 1.3, (1.0 - s) * 4.0 * PI, (1.0 - s) * 0.6], [1.0; 3])
    } else {
        let d = t - 1.2;
        let sy = 1.0 - 0.32 * (-d * 6.0).exp() * (d * 22.0).cos();
        ([0.0, -(1.0 - sy) * 0.5, 0.0], [0.0; 3], [1.0 / sy.sqrt(), sy, 1.0])
    };
    let c = Ctx { t, f: f as i64, fill: if t < 2.6 { -0.7 } else { (-0.55 + (t - 2.6) * 1.1).min(0.6) }, ..Ctx::default() };
    fr.draw(&place(&P_MESH, scale, rot, to), &cam, 0.7, &c);
    fr.outline(sc.s);
    if t >= 1.7 {
        scrawl(&mut fr, &cam, sc, &c, ((t - 1.7) / 1.0).min(1.0), scale, to, SIGNED);
    }
    if t >= 3.7 {
        drop_from_foot(&mut fr, &cam, sc, t - 3.7, scale, to);
    }
    halved(&fr, sc)
}

fn pad_top(_: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    if o[0].abs() > 0.78 || o[2].abs() > 0.33 {
        return if ((o[0] + o[2]) * 9.0).floor() as i64 % 2 != 0 { [235, 190, 30] } else { [30, 30, 34] };
    }
    [140, 140, 150]
}

static PAD: LazyLock<Mesh> = LazyLock::new(|| cube([-0.95, -0.66, -0.45], [0.95, -0.5, 0.45], one(Side::Top, textured(pad_top), color([120, 120, 130])), false));

/// 3, 2, 1: the P lifts off its pad, gone; it skywrites its tv; it falls back on its pad, and the smoke turns red.
fn liftoff(sc: &Sc, f: usize) -> Rows {
    let t = f as f32 / FPS as f32;
    let c = Ctx { t, f: f as i64, ..Ctx::default() };
    let landed = 3.8 + 0.32f32.sqrt();
    let (mut y, mut scale) = (Some(0.0), [1.0; 3]);
    if (1.5..2.7).contains(&t) {
        y = Some(3.0 * (t - 1.5).powi(2));
    } else if (2.7..3.8).contains(&t) {
        y = None;
    } else if t >= 3.8 {
        let d = t - 3.8;
        let fall = (3.2 - 10.0 * d * d).max(0.0);
        if fall == 0.0 {
            let e = d - 0.32f32.sqrt();
            let sy = 1.0 - 0.3 * (-e * 6.0).exp() * (e * 22.0).cos();
            scale = [1.0 / sy.sqrt(), sy, 1.0];
        }
        y = Some(fall);
    }
    let follow = if t < 2.7 { y.unwrap_or(0.0).min(1.4) } else { 0.0 };
    let cam = sc.cam([0.0, follow * 0.6, -2.3], [0.0, -0.05 + follow * 0.8, 0.0], 45.0);
    let mut fr = if (landed..landed + 0.07).contains(&t) { Frame::new(sc.rw, sc.rh, solid(WHITE)) } else { Frame::new(sc.rw, sc.rh, gradient(sc, [40, 80, 190], [170, 210, 245])) };
    fr.draw(&PAD, &cam, 0.6, &c);
    if let Some(y) = y {
        let to = [0.0, y - (1.0 - scale[1]) * 0.5, 0.0];
        fr.draw(&place(&P_FULL, scale, [0.0; 3], to), &cam, 0.7, &c);
        fr.outline(sc.s);
        if (1.3..2.7).contains(&t) {
            for i in 0..7 {
                let fl = noise(&[i, f as i64]) as f32 / 255.0;
                let p = [lx(380.0) + (i as f32 - 3.0) * 0.09, y - 0.55 - fl * 0.25 - (i % 3) as f32 * 0.06, -0.05];
                fr.blob(&cam, p, 0.06 + fl * 0.05, if fl > 0.5 { [255, 230, 90] } else { [255, 120, 30] });
            }
        }
    }
    // The smoke thins before it comes back down.
    let clear = 1.0 - ease((t - 3.4) / 0.9);
    for i in 0..12 {
        let born = 0.9 + i as f32 * 0.13;
        if born <= t && clear > 0.0 {
            let age = t - born;
            let p = [(noise(&[i, 1]) as f32 / 255.0 - 0.5) * 2.4 * age.min(1.0), -0.55 + age * 0.18, (noise(&[i, 2]) as f32 / 255.0 - 0.5) * 0.6];
            fr.blob(&cam, p, (0.07 + 0.11 * age.min(1.0)) * clear, shade([210, 210, 218], 1.0 - 0.1 * (i % 3) as f32));
        }
    }
    if t >= 2.7 {
        let red = t >= 4.6;
        let pen = if red { SIGNED } else { Pen { col: [236, 236, 242], ink: None, fat: 1.5, wobble: 0.25 } };
        scrawl(&mut fr, &cam, sc, &c, ((t - 2.7) / 0.9).min(1.0), [1.0; 3], [0.0; 3], pen);
    }
    let mut rows = halved(&fr, sc);
    if t < 1.5 {
        let n = ["3", "2", "1"][((t / 0.5) as usize).min(2)];
        rows.text(n, 3 * sc.dot, WHITE, sc.dot * 3, Some(sc.w as i64 - 14 * sc.dot), Some(INK));
    }
    rows
}

/// Round the bowl and down the drain it goes; up it comes again in a splash.
fn flush(sc: &Sc, f: usize) -> Rows {
    let t = f as f32 / FPS as f32;
    let c = Ctx { t, f: f as i64, ..Ctx::default() };
    let cam = rest_cam(sc);
    let spin = if t < 2.2 { 6.0 * t * t } else { 6.0 * 2.2 * 2.2 + (t - 2.2) * 2.0 };
    let mut fr = sc.frame(bowl(sc, spin, if t < 2.4 { 0.0 } else { ((t - 2.4) / 1.2).min(1.0) }));
    let s = sc.s;
    if t < 1.7 {
        let k = t / 1.7;
        let (r, a) = (0.7 * (1.0 - k), t * 9.0);
        let size = (1.0 - k.powf(1.4)).max(0.0);
        if size > 0.03 {
            fr.draw(&place(&P_FULL, [size; 3], [0.0, 0.0, t * t * 7.0], [r * a.cos(), r * a.sin() * 0.7, 0.0]), &cam, 0.7, &c);
            fr.outline(s);
        }
    } else if t >= 2.2 {
        let k = (t - 2.2) / 0.5;
        let size = if k > 1.0 {
            1.0 + 0.15 * (-(k - 1.0) * 3.0).exp() * ((k - 1.0) * 9.0).cos()
        } else if k >= 1.0 {
            1.0
        } else {
            1.18 * ease(k)
        };
        fr.draw(&place(&P_FULL, [size; 3], [0.0; 3], [0.0; 3]), &cam, 0.7, &c);
        fr.outline(s);
        if t < 3.4 {
            let d = t - 2.2;
            for i in 0..18 {
                let a = noise(&[i, 3]) as f32 / 255.0 * 2.0 * PI;
                let v = 60.0 + noise(&[i, 4]) as f32 / 2.0;
                let (x, y) = (sc.rw as f32 / 2.0 + a.cos() * v * d * s, sc.rh as f32 / 2.0 + (a.sin() * v * d - 30.0 * d + 90.0 * d * d) * s);
                fr.disc(x, y, ((3.0 - d * 2.2) * s).max(0.5), if i % 3 == 0 { PEE_DARK } else { PEE });
            }
        }
        if t >= 2.8 {
            scrawl(&mut fr, &cam, sc, &c, ((t - 2.8) / 0.9).min(1.0), [1.0; 3], [0.0; 3], SIGNED);
        }
    } else {
        // A gulp of bubbles.
        for i in 0..5 {
            let d = t - 1.7 - i as f32 * 0.09;
            if d > 0.0 {
                fr.disc(sc.rw as f32 / 2.0 + (i as f32 - 2.0) * 4.0 * s, sc.rh as f32 / 2.0 - d * 30.0 * s, (1.2 + (i % 2) as f32) * s, WHITE);
            }
        }
    }
    halved(&fr, sc)
}

/// The P, as blocks: twelve across, fifteen down.
const VOX: [&str; 15] =
    ["XXXXXXXXXXXX", "XXXXXXXXXXXX", "XXXXXXXXXXXX", "XXXXX...XXXX", "XXXXX...XXXX", "XXXXX...XXXX", "XXXXXXXXXXXX", "XXXXXXXXXXXX", "XXXXXXXXXXX.", "XXXXX.......", "XXXXX.......", "XXXXX.......", "XXXXX.......", "XXXXX.......", "XXXXX......."];

static CELLS: LazyLock<Vec<(usize, usize)>> = LazyLock::new(|| VOX.iter().enumerate().flat_map(|(y, r)| r.bytes().enumerate().filter(|(_, b)| *b == b'X').map(move |(x, _)| (x, y))).collect());

fn block_w() -> f32 {
    (lx(640.0) - lx(120.0)) / 12.0
}

const BLOCK_H: f32 = 1.0 / 15.0;

static BLOCK: LazyLock<Mesh> = LazyLock::new(|| cube([-block_w() / 2.0, -BLOCK_H / 2.0, -0.1], [block_w() / 2.0, BLOCK_H / 2.0, 0.1], one(Side::Front, color(PEE).unlit(), color(SIDE)), false));

/// In from everywhere, a block at a time, over a neon grid: the P builds itself, flashes solid, and is signed.
fn pieces(sc: &Sc, f: usize) -> Rows {
    let t = f as f32 / FPS as f32;
    let c = Ctx { t, f: f as i64, ..Ctx::default() };
    let cam = rest_cam(sc);
    let mut fr = if (2.3..2.37).contains(&t) { sc.frame(solid(WHITE)) } else { sc.frame(synthwave(sc, t)) };
    if t < 2.3 {
        let yaw = (1.0 - ease(t / 2.3)) * 1.4;
        let mut mesh = Vec::with_capacity(CELLS.len() * 6);
        let n = |i: usize, k: i64| noise(&[i as i64, k]) as f32;
        for (i, (cx, cy)) in CELLS.iter().enumerate() {
            let target = [lx(120.0) + (*cx as f32 + 0.5) * block_w(), 0.5 - (*cy as f32 + 0.5) * BLOCK_H, 0.0];
            let d = norm([(n(i, 1) - 128.0) / 128.0, (n(i, 2) - 128.0) / 128.0, (n(i, 3) - 160.0) / 128.0]);
            let start = add(target, mul(d, 3.5));
            let p = ease((t - 0.1 - n(i, 4) / 255.0 * 1.1) / 0.9);
            let spin = [(n(i, 5) - 128.0) / 20.0, (n(i, 6) - 128.0) / 20.0, (n(i, 7) - 128.0) / 20.0];
            mesh.extend(place(&BLOCK, [1.0; 3], mul(spin, 1.0 - p), lerp(start, target, p)));
        }
        fr.draw(&place(&mesh, [1.0; 3], [0.0, yaw, 0.0], [0.0; 3]), &cam, 0.7, &c);
    } else {
        let d = t - 2.3;
        let s = 1.0 + 0.12 * (-d * 5.0).exp() * (d * 20.0).cos();
        fr.draw(&place(&P_FULL, [s; 3], [0.0; 3], [0.0; 3]), &cam, 0.7, &c);
        fr.outline(sc.s);
        if t >= 2.6 {
            scrawl(&mut fr, &cam, sc, &c, ((t - 2.6) / 0.9).min(1.0), [s; 3], [0.0; 3], SIGNED);
        }
    }
    halved(&fr, sc)
}

fn zebra(_: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    if (o[0] * 30.0 + (o[1] * 9.0).sin() * 2.4).sin() > 0.15 { [18, 18, 18] } else { [245, 245, 245] }
}

fn checks(_: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    if ((o[0] * 11.0).floor() as i64 + (o[1] * 11.0).floor() as i64) % 2 != 0 { [255, 40, 180] } else { [40, 230, 240] }
}

fn rainbow(c: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    hue(o[1] * 1.2 + o[0] * 0.4 + c.t * 0.9)
}

fn tv_static(c: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    let g = noise(&[(o[0] * 40.0 + 99.0) as i64, (o[1] * 40.0 + 99.0) as i64, c.f]) as u8;
    [g, g, g]
}

fn leopard(_: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    let (gx, gy) = ((o[0] / 0.13).floor(), (o[1] / 0.13).floor());
    let cx = (gx + 0.3 + noise(&[gx as i64, gy as i64, 1]) as f32 / 255.0 * 0.4) * 0.13;
    let cy = (gy + 0.3 + noise(&[gx as i64, gy as i64, 2]) as f32 / 255.0 * 0.4) * 0.13;
    let d = (o[0] - cx).hypot(o[1] - cy);
    if d > 0.026 && d < 0.042 {
        [70, 36, 12]
    } else if d <= 0.026 {
        [170, 100, 30]
    } else {
        [236, 176, 76]
    }
}

fn polka(_: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    let gy = (o[1] / 0.11).floor() as i64;
    let off = if gy % 2 != 0 { 0.055 } else { 0.0 };
    let d = ((o[0] - off).rem_euclid(0.11) - 0.055).hypot(o[1].rem_euclid(0.11) - 0.055);
    if d < 0.03 { WHITE } else { [220, 30, 50] }
}

/// Drops on teal, as the showroom's wallpaper: across `a`, up `b`, each `size` apart.
fn droplets(a: f32, b: f32, size: f32) -> Rgb {
    let (tz, ty) = (a / size, b / size);
    let row = ty.floor() as i64;
    let (fu, fv) = ((tz + if row % 2 != 0 { 0.5 } else { 0.0 }).rem_euclid(1.0), ty.rem_euclid(1.0));
    let (dx, dy) = (fu - 0.5, fv - 0.36);
    if dx * dx + dy * dy < 0.016 || ((0.36..0.64).contains(&fv) && dx.abs() < 0.127 * (0.64 - fv) / 0.28) { DROP } else { TEAL }
}

fn drops(_: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    droplets(o[0], o[1], 0.16)
}

/// A skin, the background that clashes with it, and its sides' color.
const SKINS: [(super::solid::Tex, Rgb, Rgb); 7] = [
    (zebra, [255, 60, 160], [60, 60, 60]),
    (leopard, [40, 170, 90], [150, 80, 20]),
    (checks, [255, 230, 40], [120, 30, 170]),
    (tv_static, [220, 40, 40], [90, 90, 90]),
    (rainbow, [20, 20, 30], [240, 240, 240]),
    (polka, [40, 200, 230], [150, 20, 30]),
    (drops, [255, 140, 30], [20, 90, 90]),
];

/// A new skin every four frames, as MTV's logo always had one: zebra, leopard, checks, static, rainbow, polka dots, the
/// wallpaper; then ours, and the tv.
fn skins(sc: &Sc, f: usize) -> Rows {
    const EVERY: usize = 4;
    let t = f as f32 / FPS as f32;
    let c = Ctx { t, f: f as i64, ..Ctx::default() };
    let cam = rest_cam(sc);
    let (mesh, pulse, rot, mut fr) = if t < 3.2 {
        let (tex, bgc, side) = SKINS[(f / EVERY) % SKINS.len()];
        let mesh = p_mesh(textured(tex).unlit(), textured(tex).unlit(), color(side));
        let pulse = 1.0 + 0.14 * (-((f % EVERY) as f32) * 1.2).exp();
        let rot = [0.15 * (t * 3.1).sin(), 0.6 * (t * 2.4).sin(), 0.08 * (t * 4.7).sin()];
        let fr = if !(f / EVERY).is_multiple_of(2) { sc.frame(solid(bgc)) } else { sc.frame(sunburst(sc, bgc, shade(bgc, 0.7), t * 2.0, 10.0)) };
        (mesh, pulse, rot, fr)
    } else {
        let k = ease((t - 3.2) / 0.4);
        (P_FULL.clone(), 1.0, [0.15 * (t * 3.1).sin() * (1.0 - k), 0.6 * (t * 2.4).sin() * (1.0 - k), 0.0], sc.frame(sunburst(sc, [40, 200, 220], [30, 110, 210], -t * 1.6, 14.0)))
    };
    fr.draw(&place(&mesh, [pulse; 3], rot, [0.0; 3]), &cam, 0.75, &c);
    fr.outline(sc.s);
    if t >= 3.4 {
        scrawl(&mut fr, &cam, sc, &c, ((t - 3.4) / 0.8).min(1.0), [1.0; 3], [0.0; 3], SIGNED);
    }
    halved(&fr, sc)
}

fn moon_top(c: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    if let Some((cx, r)) = c.shadow
        && ((o[0] - cx) / (r * 1.6)).powi(2) + (o[2] / r).powi(2) < 1.0
    {
        return [60, 60, 68];
    }
    let (gx, gz) = ((o[0] / 0.5).floor(), (o[2] / 0.5).floor());
    let n = noise(&[gx as i64, gz as i64, 9]);
    let d = (o[0] - (gx + 0.5) * 0.5).hypot(o[2] - (gz + 0.5) * 0.5);
    if n > 200 && d < 0.16 {
        return if d < 0.12 { [96, 96, 106] } else { [160, 160, 170] };
    }
    if n & 1 != 0 { [128, 128, 138] } else { [136, 136, 146] }
}

static MOON: LazyLock<Mesh> = LazyLock::new(|| cube([-9.0, -1.2, -5.0], [9.0, -1.0, 14.0], only(Side::Top, textured(moon_top)), false));

/// After MTV's 1982 "Construction Worker", where the logo flattens a man: the P falls out of the sky on the astronaut, who ends up
/// squashed under its bowl, flailing, as it's signed.
fn squash(sc: &Sc, f: usize) -> Rows {
    const LAND: f32 = 1.4;
    let t = f as f32 / FPS as f32;
    let earth = norm([-0.38, 0.22, 0.9]);
    let shake = if t >= LAND { 0.05 * (-(t - LAND) * 7.0).exp() * (t * 80.0).sin() } else { 0.0 };
    let cam = sc.cam([0.5, shake, -4.4], [0.05, -0.15 + shake, 0.0], 45.0);
    let mut fr = if (LAND..LAND + 0.07).contains(&t) { sc.frame(solid(WHITE)) } else { sc.frame(sky(&cam, Some(earth), 0.2)) };
    let y = if (0.5..LAND).contains(&t) {
        Some(8.0 * (1.0 - ((t - 0.5) / (LAND - 0.5)).powi(2)))
    } else if t >= LAND {
        Some(0.0)
    } else {
        None
    };
    let shadow = y.filter(|y| *y > 0.0).map(|y| (lx(380.0) * 2.0, 0.35 + 0.5 * (1.0 - y / 8.0)));
    let c = Ctx { t, f: f as i64, shadow, ..Ctx::default() };
    let guy = if t < LAND {
        let look = if t < 0.6 { 0.0 } else { 0.45 };
        let arms = if t < 0.9 { [(0.1, 0.1), (0.1, 0.1)] } else { [(2.9, 0.5 + 0.3 * (t * 30.0).sin()), (2.9, 0.5 - 0.3 * (t * 30.0).sin())] };
        astronaut([0.0, -1.0, 0.0], &Pose { arms, prop: Prop::None, nod: look, ..Pose::default() })
    } else {
        let d = t - LAND;
        let sy = 0.42 + 0.06 * (-d * 5.0).exp() * (d * 30.0).cos();
        let flail = (t * 22.0).sin();
        let pose = Pose { arms: [(1.4, 1.3 + 0.3 * flail), (1.4, 1.3 - 0.3 * flail)], prop: Prop::None, legs: Some([(0.3 * flail, 0.0), (-0.3 * flail, 0.0)]), ..Pose::default() };
        place(&astronaut([0.0; 3], &pose), [1.15, sy, 1.15], [0.0; 3], [0.0, -1.0, 0.0])
    };
    let mut scene = MOON.clone();
    scene.extend(guy);
    fr.draw(&scene, &cam, 0.6, &c);
    if let Some(y) = y {
        fr.draw(&place(&P_FULL, [2.0; 3], [0.0; 3], [0.0, y, 0.0]), &cam, 0.7, &c);
        fr.outline(sc.s);
    }
    if t >= LAND {
        let d = t - LAND;
        for i in 0..14 {
            let a = noise(&[i, 11]) as f32 / 255.0 * PI * 2.0;
            let r = (0.6 + noise(&[i, 12]) as f32 / 255.0) * (d * 2.5).min(1.0) * 1.6;
            fr.blob(&cam, [lx(380.0) * 2.0 + a.cos() * r * 1.4, -0.95 + d * 0.2, a.sin() * r * 0.6], 0.16 * (1.0 - d / 1.6).max(0.0), [190, 190, 198]);
        }
    }
    if t >= 2.2 {
        scrawl(&mut fr, &cam, sc, &c, ((t - 2.2) / 0.9).min(1.0), [2.0; 3], [0.0; 3], SIGNED);
    }
    halved(&fr, sc)
}

fn in_p(x: f32, y: f32) -> bool {
    P_BOXES.iter().any(|[x0, y0, x1, y1]| lx(*x0) <= x && x <= lx(*x1) && ly(*y1) <= y && y <= ly(*y0))
}

fn tank_wall(c: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    let (x, y) = (o[0], o[1]);
    if in_p(x, y) && (noise(&[(x * 80.0) as i64 + 500, (y * 80.0) as i64 + 500]) as f32) < c.spray * 256.0 {
        return PEE;
    }
    if c.drips > 0.0 && y < ly(640.0) && lx(120.0) <= x && x <= lx(330.0) {
        let col = ((x - lx(120.0)) * 30.0) as i64;
        if noise(&[col, 7]) > 140 && ((x - lx(120.0)) * 30.0).rem_euclid(1.0) < 0.55 && y > ly(640.0) - (noise(&[col, 8]) as f32 / 255.0 * 0.35 + 0.05) * c.drips {
            return PEE;
        }
    }
    if y.abs() > 0.82 {
        return [196, 198, 208];
    }
    // Rivets, near enough.
    if (x.abs() * 10.0).rem_euclid(1.0) < 0.06 && y.abs() > 0.7 {
        return [150, 152, 160];
    }
    [230, 232, 238]
}

static TANK_WALL: LazyLock<Mesh> = LazyLock::new(|| cube([-1.5, -1.05, -0.2], [1.5, 1.05, 0.4], one(Side::Front, textured(tank_wall).unlit(), color([200, 202, 210])), false));
static CAN: LazyLock<Mesh> = LazyLock::new(|| {
    let mut can = cube([-0.07, -0.17, -0.07], [0.07, 0.17, 0.07], all(color([176, 178, 188])), false);
    can.extend(cube([-0.045, 0.17, -0.045], [0.045, 0.25, 0.045], all(color(RED)), false));
    can
});

/// After MTV's 1983 "Suburbia", its logo sprayed through a stencil onto houses (and a cow): PTV sprayed onto the station's urine
/// tank, back and forth, then the tv in red; and the paint runs.
fn stencil(sc: &Sc, f: usize) -> Rows {
    let t = f as f32 / FPS as f32;
    let c = Ctx { t, f: f as i64, spray: clamp01((t - 0.3) / 1.7), drips: ease((t - 3.4) / 1.0), ..Ctx::default() };
    let cam = sc.cam([0.3 - 0.06 * t, 0.05, -2.35], [0.0; 3], 45.0);
    let mut fr = sc.frame(gradient(sc, [24, 28, 44], [60, 66, 90]));
    fr.draw(&TANK_WALL, &cam, 0.6, &c);
    let pts: Vec<V3> = TV_POINTS.iter().flatten().copied().collect();
    let (mut can, mut spot, mut mist) = (None, None, PEE);
    if (0.3..2.0).contains(&t) {
        let k = (t - 0.3) / 1.7;
        let row = k * 5.0;
        let along = if (row as i64) % 2 == 0 { row.fract() } else { 1.0 - row.fract() };
        let s = [lx(120.0) + (lx(640.0) - lx(120.0)) * along, 0.42 - row / 5.0 * 0.9, -0.21];
        (can, spot) = (Some(add(s, [0.12, 0.05, -0.45])), Some(s));
    } else if (2.2..3.2).contains(&t) {
        let p = pts[(((t - 2.2) / 0.9 * pts.len() as f32) as usize).min(pts.len() - 1)];
        let s = [p[0], p[1], -0.21];
        (can, spot, mist) = (Some(add(s, [0.12, 0.05, -0.45])), Some(s), RED);
    } else if (2.0..2.2).contains(&t) || (3.2..3.8).contains(&t) {
        let d = t - if t < 2.2 { 2.0 } else { 3.2 };
        can = Some([0.6 + d * 3.0, 0.1, -0.66]);
    }
    if let Some(can) = can {
        fr.draw(&place(&CAN, [1.0; 3], [-0.5, 0.3, 0.0], can), &cam, 0.6, &c);
        if let Some(spot) = spot
            && f % 3 != 2
        {
            for i in 0..5 {
                fr.blob(&cam, lerp(add(can, [0.0, -0.1, 0.05]), spot, (i as f32 + 1.0) / 6.0), 0.02 + 0.012 * i as f32, mist);
            }
        }
    }
    if t >= 2.2 {
        scrawl(&mut fr, &cam, sc, &c, ((t - 2.2) / 0.9).min(1.0), [1.0; 3], [0.0; 3], Pen { col: RED, ink: None, fat: 1.0, wobble: 0.18 });
    }
    halved(&fr, sc)
}

// --- The homages: the cast, the sets ----------------------------------------------------------------------------------------------
/// A little P, for the TVs once they've turned to PTV.
const P_SMALL: [&str; 7] = ["XXXX.", "XX.XX", "XX.XX", "XXXX.", "XX...", "XX...", "XX..."];

/// A TV's screen, at `param` (its column and row in the wall): static, until the wave of PTV gets to its column.
fn tv_screen(c: &Ctx, param: [i32; 2], u: f32, v: f32, _o: V3) -> Rgb {
    let [col, row] = param;
    if !(0.1..=0.9).contains(&u) || !(0.12..=0.88).contains(&v) {
        return CABINET;
    }
    let (su, sv) = ((u - 0.1) / 0.8, (v - 0.12) / 0.76);
    if c.t >= c.wave + col as f32 * 0.07 {
        let (gx, gy) = ((su * 9.0) as i64 - 2, ((1.0 - sv) * 9.0) as i64 - 1);
        if (0..7).contains(&gy) && (0..5).contains(&gx) && P_SMALL[gy as usize].as_bytes()[gx as usize] == b'X' {
            return PEE;
        }
        return NAVY;
    }
    let g = 40 + noise(&[(su * 10.0) as i64, (sv * 8.0) as i64, c.f, i64::from(col * 7 + row * 13)]) * 3 / 4;
    let g = g as u8;
    [g, g, g.saturating_add(12)]
}

fn screen_mat(col: i32, row: i32) -> Mat {
    textured(tv_screen).unlit().with([col, row])
}

fn wallpaper(_: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    droplets(o[2], o[1], 0.42)
}

fn checker(_: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    if ((o[0] / 0.45).floor() as i64 + (o[2] / 0.45).floor() as i64) % 2 != 0 { CHECK_DARK } else { CHECK_LIGHT }
}

/// The showroom: a checked floor, wallpaper, and a wall of TVs down the aisle.
static ROOM: LazyLock<Mesh> = LazyLock::new(|| {
    let mut room = cube([-1.75, -0.1, -4.0], [1.45, 0.0, 10.0], only(Side::Top, textured(checker)), false);
    room.extend(cube([-1.75, 2.7, -4.0], [1.45, 2.8, 10.0], only(Side::Bottom, color(CEILING)), false));
    room.extend(cube([1.35, 0.0, -4.0], [1.45, 2.7, 10.0], only(Side::Left, textured(wallpaper)), false));
    room.extend(cube([-1.75, 0.0, -4.0], [-1.65, 2.7, 10.0], only(Side::Right, color(TEAL)), false));
    for ci in 0..16 {
        for ri in 0..5 {
            let (z0, y0) = (-2.4 + ci as f32 * 0.6, 0.05 + ri as f32 * 0.47);
            room.extend(cube([-1.65, y0, z0], [-1.15, y0 + 0.44, z0 + 0.57], one(Side::Right, screen_mat(ci, ri), color(CABINET)), false));
        }
    }
    room
});

/// TVs adrift in orbit, facing us.
static SPACE_TVS: LazyLock<Mesh> = LazyLock::new(|| {
    let mut tvs = Vec::new();
    for ci in 0..4 {
        for ri in 0..3 {
            let (x0, y0) = (0.6 + ci as f32 * 0.62, -0.7 + ri as f32 * 0.48);
            tvs.extend(cube([x0, y0, 3.2], [x0 + 0.57, y0 + 0.44, 3.7], one(Side::Front, screen_mat(ci, ri), color(CABINET)), false));
        }
    }
    tvs
});

fn solar_panel(_: &Ctx, _: [i32; 2], u: f32, v: f32, _o: V3) -> Rgb {
    if (u * 6.0).rem_euclid(1.0) < 0.07 || (v * 16.0).rem_euclid(1.0) < 0.08 { [190, 160, 70] } else { [30, 44, 116] }
}

/// The station: its truss, eight arrays, its modules and radiators.
static STATION: LazyLock<Mesh> = LazyLock::new(|| {
    let mut iss = cube([-3.3, -0.09, -0.09], [3.3, 0.09, 0.09], all(color([180, 182, 190])), false);
    for x in [-3.0f32, -2.4, 2.4, 3.0] {
        for (y0, y1) in [(0.15, 2.2), (-2.2, -0.15)] {
            iss.extend(cube([x - 0.24, y0, -0.015], [x + 0.24, y1, 0.015], move |s| Some(if matches!(s, Side::Front | Side::Back) { textured(solar_panel) } else { color([90, 90, 100]) }), false));
        }
    }
    iss.extend(cube([-0.34, -0.9, -1.9], [0.34, -0.22, 1.5], all(color([226, 226, 230])), false));
    iss.extend(cube([-1.5, -0.86, -0.3], [1.5, -0.26, 0.3], all(color([214, 214, 220])), false));
    for x in [-1.1f32, 1.1] {
        iss.extend(cube([x - 0.4, -0.2, -0.01], [x + 0.4, -0.18, 1.6], all(color([236, 236, 240])), false));
    }
    iss
});

fn visor(_: &Ctx, _: [i32; 2], u: f32, v: f32, _o: V3) -> Rgb {
    if 0.14 < u && u < 0.86 && 0.2 < v && v < 0.8 {
        return if 0.6 < u && u < 0.74 && 0.58 < v && v < 0.7 { GLINT } else { GOLD };
    }
    SUIT
}

fn chest(_: &Ctx, _: [i32; 2], u: f32, v: f32, _o: V3) -> Rgb {
    if 0.24 < u && u < 0.76 && 0.52 < v && v < 0.8 {
        return if u < 0.5 { [60, 90, 170] } else { [200, 50, 50] };
    }
    // PTV's patch.
    if 0.6 < u && u < 0.82 && 0.2 < v && v < 0.4 {
        return PEE;
    }
    SUIT
}

fn tank_face(c: &Ctx, _: [i32; 2], u: f32, v: f32, _o: V3) -> Rgb {
    if 0.34 < u && u < 0.66 && 0.14 < v && v < 0.86 {
        return if v < 0.14 + 0.72 * c.tank { PEE } else { [206, 210, 220] };
    }
    [242, 242, 246]
}

fn paper_tex(_: &Ctx, _: [i32; 2], u: f32, v: f32, _o: V3) -> Rgb {
    if v > 0.8 && 0.08 < u && u < 0.92 {
        return if (u * 9.0).rem_euclid(1.0) < 0.8 { [30, 30, 30] } else { [236, 232, 220] };
    }
    // A photo of the tank, on the front page.
    if 0.08 < u && u < 0.42 && 0.36 < v && v < 0.74 {
        return PEE;
    }
    if 0.08 < u && u < 0.92 && 0.1 < v && v < 0.76 && (v * 16.0).rem_euclid(1.0) < 0.45 && !(u < 0.46 && v > 0.34) {
        return [130, 130, 130];
    }
    [236, 232, 220]
}

static TANK: LazyLock<Mesh> = LazyLock::new(|| cube([-0.22, -0.17, -0.12], [0.22, 0.17, 0.12], one(Side::Front, textured(tank_face), color([236, 236, 240])), false));
static PAPER: LazyLock<Mesh> = LazyLock::new(|| cube([-0.3, -0.24, -0.012], [0.3, 0.24, 0.012], all(textured(paper_tex)), false));

fn seat_top(_: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
    if (o[0] / 0.16).powi(2) + ((o[2] + 0.1) / 0.21).powi(2) < 1.0 { PEE } else { SEAT }
}

/// A throne, not a toilet: wider than he is, and its cistern standing up behind him, so it shows round him from the front.
static TOILET: LazyLock<Mesh> = LazyLock::new(|| {
    let blue = all(color(BLUE_PORCELAIN));
    let mut t = cube([-0.16, 0.0, -0.1], [0.16, 0.14, 0.18], &blue, false);
    t.extend(cube([-0.3, 0.14, -0.4], [0.3, 0.4, 0.2], &blue, false));
    t.extend(cube([-0.33, 0.4, -0.43], [0.33, 0.45, 0.2], one(Side::Top, textured(seat_top), color(SEAT)), false));
    t.extend(cube([-0.4, 0.42, 0.2], [0.4, 1.3, 0.38], &blue, false));
    t.extend(cube([-0.43, 1.3, 0.17], [0.43, 1.36, 0.41], &blue, false));
    t.extend(cube([0.28, 1.16, 0.12], [0.4, 1.21, 0.2], all(color([210, 212, 220])), false));
    t
});

/// What's in his hands.
#[derive(Clone, Copy, PartialEq)]
enum Prop {
    None,
    Tank,
    Paper,
}

/// How the astronaut is: which way he faces, his stride, his head, his arms (each forward and out), what he holds, sitting or
/// not, his sway and lean, his shades (how far above his eyes, or none), turned head over heels or sideways, and his legs (hip
/// and knee), if they're not what walking or standing makes them.
#[derive(Clone, Copy)]
struct Pose {
    yaw: f32,
    walk: Option<f32>,
    look: f32,
    nod: f32,
    arms: [(f32, f32); 2],
    prop: Prop,
    sit: bool,
    sway: f32,
    lean: f32,
    shades: Option<f32>,
    spin: (f32, f32),
    legs: Option<[(f32, f32); 2]>,
}

impl Default for Pose {
    fn default() -> Pose {
        Pose { yaw: 0.0, walk: None, look: 0.0, nod: 0.0, arms: [(1.2, 0.0), (1.2, 0.0)], prop: Prop::Tank, sit: false, sway: 0.0, lean: 0.0, shades: None, spin: (0.0, 0.0), legs: None }
    }
}

fn helmet(look: f32, nod: f32, tilt: f32, shades: Option<f32>) -> Mesh {
    let mut h = cube([-0.21, 0.0, -0.21], [0.21, 0.44, 0.21], one(Side::Front, textured(visor), color(SUIT)), false);
    if let Some(s) = shades {
        let black = all(color(SHADES).unlit());
        for (x0, x1) in [(-0.18, -0.02), (0.02, 0.18)] {
            h.extend(cube([x0, 0.22 + s, -0.25], [x1, 0.31 + s, -0.21], &black, false));
        }
        h.extend(cube([-0.02, 0.27 + s, -0.24], [0.02, 0.3 + s, -0.21], &black, false));
    }
    place(&h, [1.0; 3], [nod, look, tilt], [0.0, 1.34, 0.0])
}

fn arm(rot: (f32, f32), side: f32) -> Mesh {
    let mut a = cube([-0.07, -0.44, -0.07], [0.07, 0.0, 0.07], all(color(SUIT)), false);
    a.extend(cube([-0.08, -0.54, -0.08], [0.08, -0.44, 0.08], all(color(BOOT)), false));
    // A stripe on each arm, as a spacewalker's.
    a.extend(cube([-0.075, -0.12, -0.075], [0.075, -0.06, 0.075], all(color(RED)), false));
    place(&a, [1.0; 3], [rot.0, 0.0, rot.1 * side], [0.32 * side, 1.26, 0.0])
}

fn leg(hip: f32, knee: f32, side: f32) -> Mesh {
    let mut shin = cube([-0.085, -0.3, -0.085], [0.085, 0.0, 0.085], all(color(SUIT)), false);
    shin.extend(cube([-0.1, -0.43, -0.17], [0.1, -0.3, 0.1], all(color(BOOT)), false));
    let mut thigh = cube([-0.09, -0.32, -0.09], [0.09, 0.0, 0.09], all(color(SUIT)), false);
    thigh.extend(place(&shin, [1.0; 3], [knee, 0.0, 0.0], [0.0, -0.3, 0.0]));
    place(&thigh, [1.0; 3], [hip, 0.0, 0.0], [0.12 * side, 0.73, 0.0])
}

/// Where what he holds is, his arms `a` forward.
fn held(a: f32) -> V3 {
    let d = 0.56;
    [0.0, 1.26 - a.cos() * d, -a.sin() * d - 0.04]
}

/// Low-poly, as everybody was in 1985: boxes, a gold visor, knees. Standing at `pos`, facing -z at yaw 0.
fn astronaut(pos: V3, p: &Pose) -> Mesh {
    let mut upper = cube([-0.25, 0.72, -0.17], [0.25, 1.32, 0.17], one(Side::Front, textured(chest), color(SUIT)), false);
    upper.extend(cube([-0.22, 0.8, 0.17], [0.22, 1.28, 0.38], all(color(PACK)), false));
    upper.extend(cube([-0.17, 1.3, -0.17], [0.17, 1.36, 0.17], all(color(BOOT)), false));
    upper.extend(cube([-0.26, 0.7, -0.18], [0.26, 0.78, 0.18], all(color(BOOT)), false));
    upper.extend(helmet(p.look, p.nod, -p.sway * 0.7, p.shades));
    upper.extend(arm(p.arms[0], -1.0));
    upper.extend(arm(p.arms[1], 1.0));
    let a = (p.arms[0].0 + p.arms[1].0) / 2.0;
    match p.prop {
        Prop::Tank => upper.extend(at(&TANK, held(a))),
        Prop::Paper => upper.extend(at(&PAPER, add(held(a), [0.0, 0.1, 0.0]))),
        Prop::None => {}
    }
    let mut body = place(&at(&upper, [0.0, -0.72, 0.0]), [1.0; 3], [p.lean, 0.0, p.sway], [0.0, 0.72, 0.0]);
    for (i, side) in [-1.0f32, 1.0].into_iter().enumerate() {
        let (hip, knee) = if p.sit {
            (PI / 2.0, -PI / 2.0)
        } else if let Some(legs) = p.legs {
            legs[i]
        } else if let Some(w) = p.walk {
            (0.5 * w.sin() * side, -0.8 * (w.cos() * side).max(0.0))
        } else {
            (0.0, 0.0)
        };
        body.extend(leg(hip, knee, side));
    }
    let bob = p.walk.map_or(0.0, |w| w.sin().abs() * 0.03);
    place(&at(&body, [0.0, -0.9, 0.0]), [1.0; 3], [p.spin.0, p.yaw, p.spin.1], [pos[0], pos[1] + 0.9 + bob, pos[2]])
}

/// The astronaut on the toilet: the toilet's its origin.
fn throne(rot: V3, to: V3, pose: &Pose) -> Mesh {
    let mut mesh = TOILET.clone();
    mesh.extend(astronaut([0.0, -0.29, -0.18], &Pose { sit: true, ..*pose }));
    place(&mesh, [1.0; 3], rot, to)
}

/// The scene's `Ctx`, for a homage: the TVs turn at `wave`, and the tank he carries is as full as the real one.
fn homage_ctx(f: usize, wave: f32, tank: f32) -> Ctx {
    Ctx { t: f as f32 / FPS as f32, f: f as i64, wave, tank, ..Ctx::default() }
}

fn with(mut a: Mesh, b: Mesh) -> Mesh {
    a.extend(b);
    a
}

/// The aisle of TVs; in he struts with the tank, sets it down, the shades come down, he points at us; and every TV's PTV.
fn showroom(sc: &Sc, f: usize, tank: f32) -> Rows {
    let c = homage_ctx(f, 7.3, tank);
    let t = c.t;
    let tank_floor = [0.5, 0.17, -0.1];
    let mut extra = Vec::new();
    let (cam, guy) = if t < 3.0 {
        let z = -2.6 + 1.1 * t;
        (sc.cam([0.55, 1.5, z], [-1.15, 1.15, z + 2.4], 55.0), Vec::new())
    } else if t < 7.2 {
        let cam = sc.cam([0.15 - 0.04 * (t - 3.0), 1.42, 0.15], [0.0, 1.25, 2.4], 50.0);
        let guy = if t < 5.0 {
            let (s, w) = ((t - 3.0) / 2.0, (t - 3.0) * 8.0);
            astronaut([1.15 - 1.1 * s, 0.0, 2.4], &Pose { yaw: PI / 2.0, walk: Some(w), sway: 0.16 * w.sin(), nod: 0.08 * (2.0 * w).sin(), ..Pose::default() })
        } else {
            let yaw = PI / 2.0 * (1.0 - ease((t - 5.0) / 0.4));
            let down = ease((t - 5.4) / 0.4);
            let shades = (t >= 5.8).then(|| 0.7 * (1.0 - ease((t - 5.8) / 0.5)));
            let (arms, nod) = if t < 6.5 {
                let a = (1.2 - 1.1 * down, 0.1 * down);
                ([a, a], 0.0)
            } else {
                let k = ease((t - 6.5) / 0.3);
                ([(0.3 * k + 0.1, 0.65 * k), (0.1 + 1.35 * k, -0.15 * k)], if t >= 6.8 { -0.18 * (((t - 6.8) / 0.4).min(1.0) * PI).sin() } else { 0.0 })
            };
            let lean = if shades.is_some() { -0.08 } else { 0.0 };
            let prop = if down == 0.0 { Prop::Tank } else { Prop::None };
            if down > 0.0 {
                let start = add(turn(held(1.2), [0.0, yaw, 0.0]), [0.05, 0.0, 2.4]);
                extra = at(&TANK, lerp(start, add(tank_floor, [0.05, 0.0, 2.4]), down));
            }
            astronaut([0.05, 0.0, 2.4], &Pose { yaw, arms, prop, shades, nod, lean, ..Pose::default() })
        };
        (cam, guy)
    } else {
        let s = ease((t - 7.2) / 2.6);
        let cam = sc.cam([0.3, 1.55 + 0.35 * s, 0.6 - 2.6 * s], [-0.2, 1.3 + 0.2 * s, 3.5], 50.0);
        let pump = (t * 10.0).sin();
        let raise = (2.6 + 0.3 * pump, 0.4);
        extra = at(&TANK, add(tank_floor, [0.05, 0.0, 2.4]));
        (cam, astronaut([0.05, 0.0, 2.4], &Pose { arms: [raise, raise], prop: Prop::None, shades: Some(0.0), sway: 0.18 * (t * 10.0).sin(), nod: 0.1 * pump, ..Pose::default() }))
    };
    let mut fr = sc.frame(solid(CEILING));
    fr.draw(&with(with(ROOM.clone(), guy), extra), &cam, 0.6, &c);
    finish(&fr, sc, t, 8.0)
}

/// Past the station's arrays to TVs adrift in orbit: he moonwalks by in zero g, shades on, lets the tank go, points at us, and
/// backflips as the TVs turn to PTV.
fn spacewalk(sc: &Sc, f: usize, tank: f32) -> Rows {
    let c = homage_ctx(f, 7.1, tank);
    let t = c.t;
    let earth = norm([0.15, -0.8, 0.58]);
    let tank_v = [0.22, 0.14, 0.06];
    let mut extra = Vec::new();
    let (cam, guy) = if t < 3.2 {
        let k = t / 3.2;
        (sc.cam(lerp([-5.2, 1.6, -4.2], [-2.2, 0.9, -4.6], k), [0.4, -0.2, 0.2], 55.0), Vec::new())
    } else {
        let pos = [1.35 - 0.12 * (t - 3.2), -0.6 + 0.05 * (t * 2.0).sin(), 1.4];
        let made = if t < 7.0 {
            let cam = sc.cam([0.9, 0.3, -1.1], [1.15, 0.25, 1.4], 50.0);
            let shades = (t >= 5.0).then(|| 0.7 * (1.0 - ease((t - 5.0) / 0.5)));
            let (arms, prop) = if t < 5.6 {
                ([(1.2, 0.0), (1.2, 0.0)], Prop::Tank)
            } else if t < 5.9 {
                ([(0.6, 0.4), (0.6, 0.4)], Prop::None)
            } else {
                let k = ease((t - 5.9) / 0.3);
                ([(0.3 + 0.1 * k, 0.6), (0.6 + 0.85 * k, -0.15 * k)], Prop::None)
            };
            let walk = (t < 5.6).then(|| -(t - 3.2) * 6.0);
            (cam, astronaut(pos, &Pose { walk, arms, prop, shades, spin: (0.0, 0.2 * (t * 1.3).sin()), sway: 0.1 * (t * 6.0).sin(), ..Pose::default() }))
        } else {
            let s = ease((t - 7.0) / 2.8);
            let cam = sc.cam(lerp([0.9, 0.3, -1.1], [0.4, 1.3, -5.0], s), [1.5, 0.1, 2.0], 50.0);
            let flip = 2.0 * PI * ease((t - 7.2) / 1.6);
            let legs = (7.2 < t && t < 8.8).then_some([(0.4, -0.8), (0.4, -0.8)]);
            (cam, astronaut(pos, &Pose { arms: [(2.7, 0.3), (2.7, 0.3)], prop: Prop::None, shades: Some(0.0), spin: (-flip, 0.0), legs, ..Pose::default() }))
        };
        if t >= 5.6 {
            let d = t - 5.6;
            let start = add(held(1.2), [1.35 - 0.12 * 2.4, -0.6, 1.4]);
            extra = place(&TANK, [1.0; 3], [d * 0.8, d * 1.3, d * 0.5], add(start, mul(tank_v, d)));
        }
        made
    };
    let mut fr = sc.frame(sky(&cam, Some(earth), 0.8));
    // After the cut, the station's behind them, not between us and them.
    let station = if t < 3.2 { STATION.clone() } else { place(&STATION, [1.0; 3], [0.0, 0.5, 0.0], [-2.0, -1.6, 7.0]) };
    fr.draw(&with(with(with(station, SPACE_TVS.clone()), guy), extra), &cam, 0.6, &c);
    finish(&fr, sc, t, 7.9)
}

/// Down the aisle to a toilet, and him on it, reading the paper. Down it comes; off goes the paper; on go the shades; he spreads
/// out, flushes, and every TV's PTV.
fn throne_room(sc: &Sc, f: usize, tank: f32) -> Rows {
    let c = homage_ctx(f, 5.9, tank);
    let t = c.t;
    let to = [0.05, 0.0, 2.7];
    let mut extra = Vec::new();
    let shades = (t >= 3.3).then(|| 0.7 * (1.0 - ease((t - 3.3) / 0.5)));
    let (cam, arms, prop, nod) = if t < 2.4 {
        (sc.cam([0.35, 1.4, -2.0 + 1.0 * t], [0.05, 0.95, 2.7], 50.0), [(1.9, 0.05), (1.9, 0.05)], Prop::Paper, 0.0)
    } else if t < 3.2 {
        let k = ease((t - 2.4) / 0.6);
        let a = (1.9 - 1.1 * k, 0.05);
        (sc.cam([0.95, 1.2, 1.1], [0.05, 0.85, 2.7], 50.0), [a, a], Prop::Paper, -0.15 * k)
    } else if t < 5.9 {
        let cam = sc.cam([0.95 - 0.05 * (t - 3.2), 1.2, 1.1], [0.05, 0.85, 2.7], 50.0);
        let d = t - 3.2;
        if d < 1.2 {
            extra = place(&PAPER, [1.0; 3], [d * 3.0, d * 5.0, d * 2.0], [0.05 + 1.2 * d, 0.9 + 1.6 * d - 2.5 * d * d, 2.2 - 0.2 * d]);
        }
        let k = ease((t - 3.9) / 0.4);
        let (arms, nod) = if t < 5.2 {
            let a = (0.2 + 0.1 * (1.0 - k), 1.25 * k);
            ([a, a], 0.12 * (t * 9.0).sin() * k)
        } else {
            let r = ease((t - 5.2) / 0.4);
            ([(0.2, 1.25), (0.2 - 0.6 * r, 1.25 - 0.8 * r)], 0.0)
        };
        (cam, arms, Prop::None, nod)
    } else {
        let s = ease((t - 5.9) / 3.0);
        let cam = sc.cam(lerp([0.8, 1.2, 1.1], [0.6, 2.0, -1.6], s), [0.0, 1.0 + 0.2 * s, 3.4], 50.0);
        let p = ease((t - 6.2) / 0.3) * (1.0 - ease((t - 7.4) / 0.4));
        (cam, [(0.2, 1.25), (0.2 + 1.25 * p, 1.25 - 1.4 * p)], Prop::None, 0.1 * (t * 9.0).sin())
    };
    let guy = throne([0.0; 3], to, &Pose { arms, prop, nod, shades, lean: if t > 3.9 { -0.12 } else { 0.0 }, ..Pose::default() });
    let mut fr = sc.frame(solid(CEILING));
    fr.draw(&with(with(ROOM.clone(), guy), extra), &cam, 0.6, &c);
    finish(&fr, sc, t, 7.0)
}

/// A toilet in orbit, him on it, shades on, drops of pee adrift; he waves, points, flushes, and it's a rocket.
fn orbit(sc: &Sc, f: usize, tank: f32) -> Rows {
    let c = homage_ctx(f, 5.6, tank);
    let t = c.t;
    let earth = norm([0.1, -0.9, 0.45]);
    let wide = sc.cam([0.0, 0.9, -3.4], [0.0, 0.55, 0.0], 50.0);
    let (pos, rot, cam, arms) = if t < 3.5 {
        let k = ease(t / 3.5);
        ([3.2 * (1.0 - k), 0.0, 0.0], [0.0, 0.5 * (1.0 - k), 0.6 * (1.0 - k) + 0.05 * t.sin()], wide, [(0.25, 1.2), (0.25, 1.2)])
    } else if t < 5.6 {
        let cam = if t < 5.0 { sc.cam([0.25, 1.15, -1.55], [0.0, 0.85, 0.0], 50.0) } else { wide };
        let arms = if t < 4.3 {
            [(0.25, 1.2), (2.6, 0.35 + 0.35 * (t * 12.0).sin())]
        } else if t < 5.0 {
            [(0.25, 1.2), (1.45, -0.15)]
        } else {
            [(0.25, 1.2), (-0.5, 0.5)]
        };
        ([0.0; 3], [0.0, 0.0, 0.05 * t.sin()], cam, arms)
    } else {
        let d = t - 5.6;
        ([0.0, 4.0 * d * d, 0.0], [-0.3 * d.min(1.0), 0.0, 0.0], sc.cam([0.0, 0.9, -3.4], [0.0, 0.55 + d.min(1.2), 0.0], 50.0), [(0.25, 1.2), (0.25, 1.2)])
    };
    // Turned a little, so the toilet shows.
    let rot = [rot[0], rot[1] + 0.7, rot[2]];
    let mesh = throne(rot, pos, &Pose { arms, shades: Some(0.0), lean: -0.12, prop: Prop::None, ..Pose::default() });
    let mut fr = sc.frame(sky(&cam, Some(earth), 0.95));
    fr.draw(&with(SPACE_TVS.clone(), mesh), &cam, 0.6, &c);
    for i in 0..14i64 {
        let g = [(noise(&[i, 1]) - 128) as f32 / 70.0, (noise(&[i, 2]) - 100) as f32 / 90.0, (noise(&[i, 3]) - 128) as f32 / 90.0];
        let p = add(g, [0.05 * (t * 0.7 + i as f32).sin(), 0.04 * t, 0.05 * (t * 0.9 + i as f32).cos()]);
        fr.blob(&cam, p, 0.035 + (i % 3) as f32 * 0.01, PEE);
    }
    if t >= 5.3 {
        let d = (t - 5.6).max(0.0);
        for i in 0..9i64 {
            let fl = noise(&[i, f as i64]) as f32 / 255.0;
            let p = add(turn([((i % 3) - 1) as f32 * 0.07, -0.05 - fl * 0.35 - (i / 3) as f32 * 0.08, 0.05], rot), pos);
            fr.blob(&cam, p, 0.07 + fl * 0.06, if fl > 0.6 { [255, 235, 120] } else { [255, 140, 30] });
        }
        for i in 0..12 {
            let age = i as f32 * 0.12;
            if d - age > 0.0 {
                let yy = 4.0 * (d - age).powi(2);
                fr.blob(&cam, [0.0, yy - 0.2, 0.05], 0.12 + age * 0.15, shade([220, 220, 228], 1.0 - age * 0.3));
            }
        }
    }
    finish(&fr, sc, t, 7.6)
}

/// Frame `f` of `kind`, `w` by `h`, with the tank he carries `tank` full (0 to 1): the picture, top row first.
pub(crate) fn picture(kind: Kind, f: usize, w: usize, h: usize, tank: f32) -> Vec<Rgb> {
    let sc = Sc::new(w.max(1), h.max(1));
    let f = f.min(kind.frames().saturating_sub(1));
    let rows = match kind {
        Kind::Tumble => tumble(&sc, f),
        Kind::Liftoff => liftoff(&sc, f),
        Kind::Flush => flush(&sc, f),
        Kind::Pieces => pieces(&sc, f),
        Kind::Skins => skins(&sc, f),
        Kind::Squash => squash(&sc, f),
        Kind::Stencil => stencil(&sc, f),
        Kind::Showroom => showroom(&sc, f, tank),
        Kind::Spacewalk => spacewalk(&sc, f, tank),
        Kind::Throne => throne_room(&sc, f, tank),
        Kind::Orbit => orbit(&sc, f, tank),
    };
    rows.px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bump_draws_every_frame_at_every_scale() {
        for kind in Kind::ALL {
            for (w, h) in [(64, 48), (128, 96), (3, 2)] {
                for f in (0..kind.frames()).step_by(7) {
                    assert_eq!(picture(kind, f, w, h, 0.3).len(), w * h, "{kind:?} {f}");
                }
            }
        }
    }

    #[test]
    fn names_and_lengths() {
        assert_eq!(Kind::from_name("ident-tumble"), Some(Kind::Tumble));
        assert_eq!(Kind::from_name("orbit"), Some(Kind::Orbit));
        assert_eq!(Kind::from_name("nonsense"), None);
        assert!(Kind::IDENTS.iter().all(|k| (4.5..5.3).contains(&(k.frames() as f32 / FPS as f32)) && !k.is_homage()));
        assert!(Kind::HOMAGES.iter().all(|k| k.frames() == 10 * FPS && k.is_homage()));
        for (i, k) in Kind::ALL.iter().enumerate() {
            assert_eq!(k.index(), i);
        }
    }

    fn colors(px: &[Rgb]) -> usize {
        px.iter().collect::<std::collections::HashSet<_>>().len()
    }

    #[test]
    fn they_move_and_end_signed() {
        // Each frame isn't the last: something moves.
        for kind in Kind::ALL {
            let (a, b) = (picture(kind, 20, 64, 48, 0.3), picture(kind, 21, 64, 48, 0.3));
            assert_ne!(a, b, "{kind:?}");
            assert!(colors(&a) > 4, "{kind:?}");
        }
        // The idents end with the P signed: pee-yellow and the tv's red both there.
        for kind in Kind::IDENTS {
            let last = picture(kind, kind.frames() - 1, 64, 48, 0.3);
            assert!(last.iter().any(|c| c[0] > 180 && c[1] < 80 && c[2] < 80), "{kind:?}: the tv's red");
        }
        // The homages have the bug, and end with the tagline: white letters at the top.
        for kind in Kind::HOMAGES {
            let last = picture(kind, kind.frames() - 1, 64, 48, 0.3);
            assert!(last[3 * 64..8 * 64].iter().filter(|c| **c == WHITE).count() > 20, "{kind:?}");
        }
    }

    /// For looking at them: a sheet of every other half second of each, 3x, at 100% and 200%, into the folder `ISS_SHEETS` names.
    /// `cargo test --release -p pms --lib -- --ignored bump_sheets`.
    #[test]
    #[ignore]
    fn bump_sheets() {
        let Some(out) = std::env::var_os("ISS_SHEETS").map(std::path::PathBuf::from) else { return };
        for kind in Kind::ALL {
            for (w, h, scale) in [(64usize, 48usize, 3usize), (128, 96, 2)] {
                let picks: Vec<usize> = (0..kind.frames()).step_by(FPS / 2).collect();
                let per_row = 8;
                let rows_n = picks.len().div_ceil(per_row);
                let gap = 6;
                let (sw, sh) = (per_row * (w * scale + gap) + gap, rows_n * (h * scale + gap) + gap);
                let mut px = vec![24u8; sw * sh * 3];
                for (n, f) in picks.iter().enumerate() {
                    let pic = picture(kind, *f, w, h, 0.29);
                    let (ox, oy) = (gap + (n % per_row) * (w * scale + gap), gap + (n / per_row) * (h * scale + gap));
                    for y in 0..h * scale {
                        for x in 0..w * scale {
                            let at = ((oy + y) * sw + ox + x) * 3;
                            px[at..at + 3].copy_from_slice(&pic[(y / scale) * w + x / scale]);
                        }
                    }
                }
                let file = std::fs::File::create(out.join(format!("rust-{}-{w}x{h}.png", kind.name()))).unwrap();
                let mut e = png::Encoder::new(file, sw as u32, sh as u32);
                e.set_color(png::ColorType::Rgb);
                e.write_header().unwrap().write_image_data(&px).unwrap();
            }
        }
    }

    /// What each costs to draw, a frame, on this thread: its time and its cycles, at 100% and 200%.
    /// `cargo test --release -p pms --lib -- --ignored bump_costs --nocapture`.
    #[test]
    #[ignore]
    fn bump_costs() {
        use windows::Win32::System::{Threading::GetCurrentThread, WindowsProgramming::QueryThreadCycleTime};
        let cycles = || {
            let mut c = 0u64;
            let _ = unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut c) };
            c
        };
        for (w, h) in [(64usize, 48usize), (128, 96)] {
            for kind in Kind::ALL {
                // Once through first, for the scenes that are made the first time they're wanted.
                let _ = picture(kind, 0, w, h, 0.29);
                let (began, c0) = (std::time::Instant::now(), cycles());
                let mut worst = 0u64;
                for f in 0..kind.frames() {
                    let c = cycles();
                    let _ = picture(kind, f, w, h, 0.29);
                    worst = worst.max(cycles() - c);
                }
                let n = kind.frames() as u64;
                println!("{w}x{h} {:18} {:6.2} ms a frame, {:6.2} M cycles a frame (worst {:6.2} M)", kind.name(), began.elapsed().as_secs_f64() * 1000.0 / n as f64, (cycles() - c0) as f64 / n as f64 / 1e6, worst as f64 / 1e6);
            }
        }
    }
}

