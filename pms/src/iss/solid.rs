//! A tiny software 3D renderer, of the kind a 1985 music video was made with: boxes, flat shading with one light, a z-buffer,
//! clipping at the near plane, outlines where faces of an outlined thing meet, and balls (a drop, a puff, a flame) hidden by
//! what's nearer at their middle. Drawn at twice the screen's size and halved, so edges don't crawl. For Number One's bumps
//! (bumps.rs); after the design's Python prototype. No dependencies: nothing here but arithmetic.
//!
//! The scene's units are its own; the camera looks down +z with +y up. A face's texture, when it has one, is a function asked
//! for each pixel with its u and v (0 to 1 across the face) and its point in the thing's own space, before it was placed.

pub(crate) type Rgb = [u8; 3];
pub(crate) type V3 = [f32; 3];

pub(crate) fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn mul(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

pub(crate) fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub(crate) fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    mul(a, if l > 0.0 { 1.0 / l } else { 1.0 })
}

pub(crate) fn lerp(a: V3, b: V3, t: f32) -> V3 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// `p` turned about x, then y, then z.
pub(crate) fn turn(p: V3, rot: V3) -> V3 {
    let (c, s) = (rot[0].cos(), rot[0].sin());
    let p = [p[0], c * p[1] - s * p[2], s * p[1] + c * p[2]];
    let (c, s) = (rot[1].cos(), rot[1].sin());
    let p = [c * p[0] + s * p[2], p[1], -s * p[0] + c * p[2]];
    let (c, s) = (rot[2].cos(), rot[2].sin());
    [c * p[0] - s * p[1], s * p[0] + c * p[1], p[2]]
}

/// A color, `k` as bright (and no brighter than white).
pub(crate) fn shade(c: Rgb, k: f32) -> Rgb {
    c.map(|v| (f32::from(v) * k).clamp(0.0, 255.0) as u8)
}

/// From `a` to `b`, `t` of the way (held to 0..1).
pub(crate) fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = clamp01(t);
    std::array::from_fn(|i| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t) as u8)
}

pub(crate) fn clamp01(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}

/// Slow in, slow out, from 0 to 1.
pub(crate) fn ease(t: f32) -> f32 {
    let t = clamp01(t);
    t * t * (3.0 - 2.0 * t)
}

/// A number from 0 to 255 for `k`: the same every time, and nothing like its neighbours'. In place of a random number generator.
pub(crate) fn noise(k: &[i64]) -> i64 {
    let mut h: u32 = 2_166_136_261;
    for v in k {
        h = (h ^ (*v as u32)).wrapping_mul(16_777_619);
    }
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    i64::from((h ^ (h >> 15)) & 255)
}

/// A hue, 0 to 1 round the wheel.
pub(crate) fn hue(h: f32) -> Rgb {
    let h = h.rem_euclid(1.0) * 6.0;
    let (i, f) = (h as usize, h.fract());
    let (up, down) = ((255.0 * f) as u8, (255.0 * (1.0 - f)) as u8);
    [[255, up, 0], [down, 255, 0], [0, 255, up], [0, down, 255], [up, 0, 255], [255, 0, down]][i.min(5)]
}

/// What a texture is told besides where on its face it is: the time in the bump, and what each bump's scenes need.
#[derive(Clone, Copy, Default)]
pub(crate) struct Ctx {
    /// Seconds into the bump, and its frame.
    pub t: f32,
    pub f: i64,
    /// How full the P is: the pee's level in its own units.
    pub fill: f32,
    /// When the TVs turn to PTV.
    pub wave: f32,
    /// How much of the stencil's been sprayed, and how far its paint has run.
    pub spray: f32,
    pub drips: f32,
    /// The falling P's shadow on the moon: where, and how big.
    pub shadow: Option<(f32, f32)>,
    /// How full the tank he carries is, 0 to 1.
    pub tank: f32,
}

/// A texture: asked with the bump's `Ctx`, its material's own two numbers, u, v, and the point in the thing's own space.
pub(crate) type Tex = fn(&Ctx, [i32; 2], f32, f32, V3) -> Rgb;

/// What a face is made of: a color, or a texture, lit by the light, or not (as bright as it is).
#[derive(Clone, Copy)]
pub(crate) struct Mat {
    pub color: Rgb,
    pub tex: Option<Tex>,
    pub lit: bool,
    pub param: [i32; 2],
}

pub(crate) const fn color(c: Rgb) -> Mat {
    Mat { color: c, tex: None, lit: true, param: [0, 0] }
}

pub(crate) const fn textured(t: Tex) -> Mat {
    Mat { color: [0, 0, 0], tex: Some(t), lit: true, param: [0, 0] }
}

impl Mat {
    pub(crate) const fn unlit(self) -> Mat {
        Mat { lit: false, ..self }
    }

    pub(crate) const fn with(self, param: [i32; 2]) -> Mat {
        Mat { param, ..self }
    }
}

/// A box's sides.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Side {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

/// Each side: which way it faces, its corners (bit 0 is x's high end, 1 y's, 2 z's), and which side it is.
const SIDES: [(V3, [usize; 4], Side); 6] = [
    ([0.0, 0.0, -1.0], [0, 1, 3, 2], Side::Front),
    ([0.0, 0.0, 1.0], [5, 4, 6, 7], Side::Back),
    ([-1.0, 0.0, 0.0], [4, 0, 2, 6], Side::Left),
    ([1.0, 0.0, 0.0], [1, 5, 7, 3], Side::Right),
    ([0.0, 1.0, 0.0], [2, 3, 7, 6], Side::Top),
    ([0.0, -1.0, 0.0], [4, 5, 1, 0], Side::Bottom),
];
const UV: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

/// A face: its corners, which way it faces, what it's made of, its corners in its thing's own space, and which outlined face it
/// is (0, none).
#[derive(Clone, Copy)]
pub(crate) struct Face {
    pub pts: [V3; 4],
    pub n: V3,
    pub mat: Mat,
    pub obj: [V3; 4],
    pub id: u16,
}

pub(crate) type Mesh = Vec<Face>;

/// A box from `lo` to `hi`, each side of `mats` (None: no side there); outlined if `ids`.
pub(crate) fn cube(lo: V3, hi: V3, mats: impl Fn(Side) -> Option<Mat>, ids: bool) -> Mesh {
    let mut out = Vec::with_capacity(6);
    for (i, (n, idx, side)) in SIDES.iter().enumerate() {
        let Some(mat) = mats(*side) else { continue };
        let pts = idx.map(|c| [if c & 1 != 0 { hi[0] } else { lo[0] }, if c & 2 != 0 { hi[1] } else { lo[1] }, if c & 4 != 0 { hi[2] } else { lo[2] }]);
        out.push(Face { pts, n: *n, mat, obj: pts, id: if ids { i as u16 + 1 } else { 0 } });
    }
    out
}

/// Every side the same.
pub(crate) fn all(m: Mat) -> impl Fn(Side) -> Option<Mat> {
    move |_| Some(m)
}

/// One side one way, and the rest another.
pub(crate) fn one(side: Side, m: Mat, rest: Mat) -> impl Fn(Side) -> Option<Mat> {
    move |s| Some(if s == side { m } else { rest })
}

/// Only one side.
pub(crate) fn only(side: Side, m: Mat) -> impl Fn(Side) -> Option<Mat> {
    move |s| (s == side).then_some(m)
}

/// `mesh` scaled, turned and moved.
pub(crate) fn place(mesh: &[Face], scale: V3, rot: V3, at: V3) -> Mesh {
    mesh.iter().map(|f| Face { pts: f.pts.map(|p| add(turn([p[0] * scale[0], p[1] * scale[1], p[2] * scale[2]], rot), at)), n: turn(f.n, rot), ..*f }).collect()
}

/// `mesh` moved.
pub(crate) fn at(mesh: &[Face], to: V3) -> Mesh {
    place(mesh, [1.0; 3], [0.0; 3], to)
}

/// A camera at `eye`, looking at a target, with a field of view, for a picture `rw` by `rh`.
pub(crate) struct Cam {
    eye: V3,
    f: V3,
    r: V3,
    u: V3,
    /// How far the picture is from the eye, in its pixels.
    pub big_f: f32,
    rw: f32,
    rh: f32,
}

impl Cam {
    pub(crate) fn new(eye: V3, target: V3, fov: f32, rw: usize, rh: usize) -> Cam {
        let f = norm(sub(target, eye));
        let r = norm(cross([0.0, 1.0, 0.0], f));
        let u = cross(f, r);
        Cam { eye, f, r, u, big_f: (rh as f32 / 2.0) / (fov.to_radians() / 2.0).tan(), rw: rw as f32, rh: rh as f32 }
    }

    /// A point, as the camera has it: across, up, and how far ahead.
    pub(crate) fn to(&self, p: V3) -> V3 {
        let d = sub(p, self.eye);
        [dot(d, self.r), dot(d, self.u), dot(d, self.f)]
    }

    fn project(&self, q: V3) -> V3 {
        [self.rw / 2.0 + q[0] / q[2] * self.big_f, self.rh / 2.0 - q[1] / q[2] * self.big_f, q[2]]
    }

    /// Where a point is in the picture, and how far.
    pub(crate) fn screen(&self, p: V3) -> V3 {
        self.project(self.to(p))
    }

    /// Which way the pixel (`x`, `y`) looks.
    pub(crate) fn ray(&self, x: usize, y: usize) -> V3 {
        let across = (x as f32 + 0.5 - self.rw / 2.0) / self.big_f;
        let up = (self.rh / 2.0 - y as f32 - 0.5) / self.big_f;
        norm(add(add(self.f, mul(self.r, across)), mul(self.u, up)))
    }
}

/// Nothing nearer than this is drawn.
const NEAR: f32 = 0.05;
/// The light, from above, to the left and in front.
const LIGHT: V3 = [-0.420_542, 0.747_631, -0.513_996];
const INK: Rgb = [0, 0, 0];

/// A picture being drawn, at twice the screen's size: its colors, how far each pixel's surface is, and which outlined face it is.
pub(crate) struct Frame {
    pub rw: usize,
    pub rh: usize,
    pub c: Vec<Rgb>,
    z: Vec<f32>,
    id: Vec<u16>,
}

/// A corner, in the camera's space, with what's carried across the face: u, v, and the point in the thing's own space.
type Corner = (V3, [f32; 5]);

fn clip_near(poly: &[Corner]) -> Vec<Corner> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let ((p, a), (q, b)) = (poly[i], poly[(i + 1) % poly.len()]);
        let (pin, qin) = (p[2] >= NEAR, q[2] >= NEAR);
        if pin {
            out.push((p, a));
        }
        if pin != qin {
            let t = (NEAR - p[2]) / (q[2] - p[2]);
            out.push((lerp(p, q, t), std::array::from_fn(|j| a[j] + (b[j] - a[j]) * t)));
        }
    }
    out
}

impl Frame {
    /// A frame `rw` by `rh`, each pixel what `bg` says, with nothing in front of it.
    pub(crate) fn new(rw: usize, rh: usize, bg: impl Fn(usize, usize) -> Rgb) -> Frame {
        let mut c = Vec::with_capacity(rw * rh);
        for y in 0..rh {
            for x in 0..rw {
                c.push(bg(x, y));
            }
        }
        Frame { rw, rh, c, z: vec![f32::MAX; rw * rh], id: vec![0; rw * rh] }
    }

    /// A triangle: its corners on the picture (x, y, and how far), what's carried across it, and how lit.
    #[allow(clippy::too_many_arguments)]
    fn tri(&mut self, p: [V3; 3], a: [[f32; 5]; 3], mat: &Mat, k: f32, id: u16, ctx: &Ctx) {
        let ([x0, y0, z0], [x1, y1, z1], [x2, y2, z2]) = (p[0], p[1], p[2]);
        let area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
        if area.abs() < 1e-9 {
            return;
        }
        let ia = 1.0 / area;
        let minx = x0.min(x1).min(x2).floor().max(0.0) as usize;
        let maxx = (x0.max(x1).max(x2).floor()).min(self.rw as f32 - 1.0);
        let miny = y0.min(y1).min(y2).floor().max(0.0) as usize;
        let maxy = (y0.max(y1).max(y2).floor()).min(self.rh as f32 - 1.0);
        if maxx < 0.0 || maxy < 0.0 || minx as f32 > maxx || miny as f32 > maxy {
            return;
        }
        let (maxx, maxy) = (maxx as usize, maxy as usize);
        let (q0, q1, q2) = (1.0 / z0, 1.0 / z1, 1.0 / z2);
        let flat = shade(mat.color, k);
        let (a0, a1, a2) = (a[0].map(|v| v * q0), a[1].map(|v| v * q1), a[2].map(|v| v * q2));
        for py in miny..=maxy {
            let fy = py as f32 + 0.5;
            let row = py * self.rw;
            for px in minx..=maxx {
                let fx = px as f32 + 0.5;
                let w0 = ((x1 - fx) * (y2 - fy) - (x2 - fx) * (y1 - fy)) * ia;
                if w0 < 0.0 {
                    continue;
                }
                let w1 = ((x2 - fx) * (y0 - fy) - (x0 - fx) * (y2 - fy)) * ia;
                if w1 < 0.0 {
                    continue;
                }
                let w2 = 1.0 - w0 - w1;
                if w2 < 0.0 {
                    continue;
                }
                let z = 1.0 / (w0 * q0 + w1 * q1 + w2 * q2);
                let i = row + px;
                if z >= self.z[i] {
                    continue;
                }
                self.z[i] = z;
                self.c[i] = match mat.tex {
                    None => flat,
                    Some(tex) => {
                        let at: [f32; 5] = std::array::from_fn(|j| (w0 * a0[j] + w1 * a1[j] + w2 * a2[j]) * z);
                        shade(tex(ctx, mat.param, at[0], at[1], [at[2], at[3], at[4]]), k)
                    }
                };
                self.id[i] = id;
            }
        }
    }

    /// Draws `faces` as `cam` sees them, lit by the light with `ambient` of it everywhere.
    pub(crate) fn draw(&mut self, faces: &[Face], cam: &Cam, ambient: f32, ctx: &Ctx) {
        for face in faces {
            if dot(face.n, sub(face.pts[0], cam.eye)) >= 0.0 {
                continue;
            }
            let k = if face.mat.lit { ambient + (1.0 - ambient) * dot(face.n, LIGHT).max(0.0) } else { 1.0 };
            let corners: Vec<Corner> = (0..4).map(|i| (cam.to(face.pts[i]), [UV[i][0], UV[i][1], face.obj[i][0], face.obj[i][1], face.obj[i][2]])).collect();
            let poly = clip_near(&corners);
            if poly.len() < 3 {
                continue;
            }
            let sp: Vec<V3> = poly.iter().map(|(q, _)| cam.project(*q)).collect();
            for i in 1..poly.len() - 1 {
                self.tri([sp[0], sp[i], sp[i + 1]], [poly[0].1, poly[i].1, poly[i + 1].1], &face.mat, k, face.id, ctx);
            }
        }
    }

    /// Ink where one outlined face meets another, or what isn't outlined: `s` (the display's scale) times two pixels thick, so
    /// one at the screen's size at 100%.
    pub(crate) fn outline(&mut self, s: f32) {
        let (rw, rh) = (self.rw, self.rh);
        let mut edge = Vec::new();
        for y in 0..rh.saturating_sub(1) {
            for x in 0..rw.saturating_sub(1) {
                let i = y * rw + x;
                if self.id[i] != self.id[i + 1] || self.id[i] != self.id[i + rw] {
                    edge.push(i);
                }
            }
        }
        let t = ((2.0 * s).round() as usize).max(1);
        for i in edge {
            let (x0, y0) = (i % rw, i / rw);
            for dy in 0..t.min(rh - y0) {
                for dx in 0..t.min(rw - x0) {
                    self.c[(y0 + dy) * rw + x0 + dx] = INK;
                }
            }
        }
    }

    /// A round spot, `r` across from its middle, over everything.
    pub(crate) fn disc(&mut self, cx: f32, cy: f32, r: f32, col: Rgb) {
        let y0 = (cy - r).floor().max(0.0) as usize;
        let y1 = ((cy + r).floor() as i64 + 2).clamp(0, self.rh as i64) as usize;
        let x0 = (cx - r).floor().max(0.0) as usize;
        let x1 = ((cx + r).floor() as i64 + 2).clamp(0, self.rw as i64) as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                if dx * dx + dy * dy <= r * r {
                    self.c[y * self.rw + x] = col;
                }
            }
        }
    }

    /// A ball (a drop, a puff, a flame) at `p`, `r` across, hidden by whatever's nearer at its middle; and not drawn behind the camera.
    pub(crate) fn blob(&mut self, cam: &Cam, p: V3, r: f32, col: Rgb) {
        let q = cam.to(p);
        if q[2] < NEAR {
            return;
        }
        let [x, y, z] = cam.project(q);
        if x >= 0.0 && y >= 0.0 && (x as usize) < self.rw && (y as usize) < self.rh && self.z[y as usize * self.rw + x as usize] < z {
            return;
        }
        self.disc(x, y, (r / z * cam.big_f).max(0.7), col);
    }

    /// The picture at half its size, each pixel the four it was: what's shown.
    pub(crate) fn halved(&self, w: usize, h: usize) -> Vec<Rgb> {
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let i = (y * 2).min(self.rh - 1) * self.rw + (x * 2).min(self.rw - 1);
                let (right, below) = (usize::from(x * 2 + 1 < self.rw), if y * 2 + 1 < self.rh { self.rw } else { 0 });
                let p = [self.c[i], self.c[i + right], self.c[i + below], self.c[i + below + right]];
                out.push(std::array::from_fn(|k| ((p.iter().map(|c| u32::from(c[k])).sum::<u32>()) / 4) as u8));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgb = [200, 32, 42];

    fn frame() -> Frame {
        Frame::new(64, 48, |_, _| [0, 0, 255])
    }

    fn unit_box(m: Mat, ids: bool) -> Mesh {
        cube([-0.5, -0.5, -0.5], [0.5, 0.5, 0.5], all(m), ids)
    }

    #[test]
    fn a_box_seen_from_the_front_is_its_color() {
        let mut fr = frame();
        let cam = Cam::new([0.0, 0.0, -3.0], [0.0, 0.0, 0.0], 45.0, 64, 48);
        fr.draw(&unit_box(color(RED).unlit(), false), &cam, 1.0, &Ctx::default());
        assert_eq!(fr.c[24 * 64 + 32], RED, "its middle");
        assert_eq!(fr.c[0], [0, 0, 255], "and the background round it");
        // Lit, its front faces away from the light a little: darker than it is, but not black.
        let mut lit = frame();
        lit.draw(&unit_box(color(RED), false), &cam, 0.6, &Ctx::default());
        let c = lit.c[24 * 64 + 32];
        assert!(c[0] < RED[0] && c[0] > 100, "{c:?}");
    }

    #[test]
    fn nearer_hides_further() {
        let mut fr = frame();
        let cam = Cam::new([0.0, 0.0, -3.0], [0.0, 0.0, 0.0], 45.0, 64, 48);
        let far = at(&unit_box(color([0, 255, 0]).unlit(), false), [0.0, 0.0, 2.0]);
        let near = unit_box(color(RED).unlit(), false);
        fr.draw(&near, &cam, 1.0, &Ctx::default());
        fr.draw(&far, &cam, 1.0, &Ctx::default());
        assert_eq!(fr.c[24 * 64 + 32], RED, "drawn after, but further");
    }

    #[test]
    fn the_near_plane_cuts_and_nothing_behind_the_camera_is_drawn() {
        // A box behind the camera: nothing.
        let cam = Cam::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], 45.0, 64, 48);
        let mut fr = frame();
        fr.draw(&at(&unit_box(color(RED).unlit(), false), [0.0, 0.0, -3.0]), &cam, 1.0, &Ctx::default());
        fr.blob(&cam, [0.0, 0.0, -2.0], 0.5, RED);
        assert!(fr.c.iter().all(|c| *c == [0, 0, 255]));
        // A floor running from behind the camera to far ahead: cut at the near plane, and what's in front of it drawn.
        let mut fr = frame();
        let floor = cube([-5.0, -1.1, -5.0], [5.0, -1.0, 20.0], only(Side::Top, color(RED).unlit()), false);
        fr.draw(&floor, &cam, 1.0, &Ctx::default());
        assert_eq!(fr.c[47 * 64 + 32], RED, "the floor right under us");
        assert_eq!(fr.c[0], [0, 0, 255], "and the sky");
        assert!(fr.z.iter().all(|z| *z >= NEAR), "nothing nearer than the near plane");
        // Corners on both sides of it come back on the plane.
        let cut = clip_near(&[([0.0, 0.0, -1.0], [0.0; 5]), ([1.0, 0.0, 1.0], [1.0; 5]), ([0.0, 1.0, 1.0], [1.0; 5])]);
        assert_eq!(cut.len(), 4);
        assert!(cut.iter().all(|(p, _)| p[2] >= NEAR - 1e-6));
    }

    #[test]
    fn outlines_are_where_outlined_faces_meet() {
        let cam = Cam::new([1.5, 1.2, -3.0], [0.0, 0.0, 0.0], 45.0, 64, 48);
        let mut plain = frame();
        plain.draw(&unit_box(color(RED).unlit(), false), &cam, 1.0, &Ctx::default());
        plain.outline(1.0);
        assert!(plain.c.iter().all(|c| *c != INK), "nothing outlined, no ink");
        let mut inked = frame();
        inked.draw(&unit_box(color(RED).unlit(), true), &cam, 1.0, &Ctx::default());
        inked.outline(1.0);
        let ink = inked.c.iter().filter(|c| **c == INK).count();
        assert!(ink > 40, "round it, and between its faces: {ink}");
        let mut thick = frame();
        thick.draw(&unit_box(color(RED).unlit(), true), &cam, 1.0, &Ctx::default());
        thick.outline(2.0);
        assert!(thick.c.iter().filter(|c| **c == INK).count() > ink, "thicker at a bigger scale");
    }

    #[test]
    fn a_texture_is_asked_with_its_own_point() {
        fn halves(_: &Ctx, _: [i32; 2], _u: f32, _v: f32, o: V3) -> Rgb {
            if o[0] < 0.0 { [255, 0, 0] } else { [0, 255, 0] }
        }
        let mut fr = frame();
        let cam = Cam::new([0.0, 0.0, -3.0], [0.0, 0.0, 0.0], 45.0, 64, 48);
        // Moved and turned half round: its own left is on our right.
        fr.draw(&place(&unit_box(textured(halves).unlit(), false), [1.0; 3], [0.0, std::f32::consts::PI, 0.0], [0.0; 3]), &cam, 1.0, &Ctx::default());
        assert_eq!(fr.c[24 * 64 + 26], [0, 255, 0]);
        assert_eq!(fr.c[24 * 64 + 38], [255, 0, 0]);
    }

    #[test]
    fn halving_averages_four() {
        let mut fr = Frame::new(4, 2, |x, _| if x % 2 == 0 { [0, 0, 0] } else { [200, 100, 50] });
        fr.disc(-10.0, -10.0, 1.0, RED);
        assert_eq!(fr.halved(2, 1), [[100, 50, 25], [100, 50, 25]]);
        assert_eq!(noise(&[1, 2]), noise(&[1, 2]));
        assert_ne!((0..20).map(|i| noise(&[i])).collect::<Vec<_>>(), vec![noise(&[0]); 20]);
        assert_eq!(hue(0.0), [255, 0, 0]);
    }
}
