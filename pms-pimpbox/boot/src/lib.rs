//! PIMPBOX's boot animation. A set warms up, a green line and a moment of its name raining down; small lumps drift across the
//! dark, far off, as a soft ball thrown in from the left bounces toward the middle; and as it gets there, still going, it's a
//! mirror ball, soft as ever, bouncing on down a hall ruled in light, gold underfoot and purple overhead, a dollar sign in every
//! square, inside a great dark ball of a place; dice going by, alone and in schools, and thrown, knocking on the floor; a dolphin
//! leaping round it, once; the camera reeling after it and swinging round it; then it crouches, and springs at us, dice flying with
//! it, turning to gold as it comes; and at the moment it would hit, there's only the dark and the mark, a glowing $, shaking
//! from the blow. Its name wipes on under it, a gold chain comes up round it, a purple hat drops onto it, and it says who made it.
//!
//! All of it is arithmetic: there's no picture, model or font file. (The maker's line can be handed in already lettered,
//! `Lettering`, by whoever has a font to letter it with: `fonts`, with the `system-fonts` feature, asks Windows. Without one,
//! it's in the dots below.) What moves is worked out here, once a frame; the graphics card draws every pixel from that, with
//! Direct3D 11 (`gpu`, and boot.hlsl). `Gpu::draw` draws the frame for a time in seconds, the same every time for the same
//! time, onto whatever it's handed at whatever size that is, so whoever shows it sets the pace: `FPS` is what it's meant for,
//! and `W` by `H` the picture it's laid out on. The beats' times are the constants below, to be tuned by feel.

pub const W: usize = 640;
pub const H: usize = 480;
/// What it's drawn for. Nothing here counts frames: only whoever shows it does.
pub const FPS: u32 = 60;

/// When each beat begins, in seconds, and when it's all over: the lumps and the one that bounces in, the ball, its landings,
/// its spring at us, the moment it would hit (and the mark and its name begin), the hat.
pub const GOO: f32 = RAIN + 11.0 / FPS as f32;
/// A little before that, half a second in: the name, raining, green on black; and for the last sixtieth of a second, `FLIP`, black on green.
pub const RAIN: f32 = 0.5;
pub const FLIP: f32 = GOO - 1.0 / FPS as f32;
pub const BALL: f32 = GOO + 1.7;
pub const LANDS: [f32; 5] = [BALL + 0.5, BALL + 1.4, BALL + 2.2, BALL + 2.9, BALL + 3.6];
pub const LAUNCH: f32 = BALL + 3.95;
pub const HIT: f32 = BALL + 4.45;
pub const NAME: f32 = HIT;
pub const HAT: f32 = NAME + 1.1;
/// When the maker's line appears: all at once, a while after everything else has settled.
pub const MADE: f32 = HAT + 1.1;
pub const LENGTH: f32 = HIT + 3.55;
/// How long the short one lasts: the console coming back on when it's only just been on. The mark and the name come up out
/// of the dark, with no blow to shake them, and from there it's the end of the long one.
pub const SHORT: f32 = LENGTH - HIT;
/// What goes on in the hall was timed for a ball that came at 2: how much later than that it comes.
const LATER: f32 = BALL - 2.0;
/// The last time it comes down, and how high each bounce after a landing takes its middle above where it rests.
const LAST: f32 = LANDS[LANDS.len() - 1];
const PEAKS: [f32; 4] = [1.1, 0.95, 0.8, 0.6];

/// The mark, as dots.
const DOLLAR: [&str; 13] = [
    ".....#.....",
    "..#######..",
    ".##..#..##.",
    "##...#.....",
    "##...#.....",
    ".##..#.....",
    "..#######..",
    ".....#..##.",
    ".....#...##",
    ".....#...##",
    ".##..#..##.",
    "..#######..",
    ".....#.....",
];

/// The hat, as dots: its crown and brim (p), the shade under the brim (d), its band (g), and a feather (f).
const HAT_DOTS: [&str; 9] = [
    "...........ff....",
    ".....pppp...ff...",
    "....pppppp..ff...",
    "....pppppp.ff....",
    "....ggggggff.....",
    "....pppppp.......",
    ".pppppppppppp....",
    "ppppppppppppppp..",
    ".dddddddddddd....",
];

/// The letters the lockup needs, five dots by seven.
fn letter(c: char) -> [u8; 7] {
    match c {
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'I' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00100],
        _ => [0; 7],
    }
}

fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// 0 at `a`, 1 at `b`, eased at both ends.
fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = clamp01((x - a) / (b - a));
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// How far each point around the mark is from it, in dots: 0 on it. Worked out once, a few cells to the dot, so that the mark
/// has an edge to soften and a glow to cast at any size.
struct Mark {
    w: usize,
    h: usize,
    far: Vec<f32>,
}

/// Cells to a dot, and dots of room around the mark for its glow.
const FINE: usize = 4;
const ROOM: usize = 8;
const COLS: usize = 11;
const ROWS: usize = 13;

impl Mark {
    fn new() -> Mark {
        let (w, h) = ((COLS + 2 * ROOM) * FINE, (ROWS + 2 * ROOM) * FINE);
        let on = |cx: usize, cy: usize| {
            let (x, y) = ((cx / FINE).wrapping_sub(ROOM), (cy / FINE).wrapping_sub(ROOM));
            x < COLS && y < ROWS && DOLLAR[y].as_bytes()[x] == b'#'
        };
        let lit: Vec<(f32, f32)> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| on(x, y)).map(|(x, y)| (x as f32, y as f32)).collect();
        let far = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x as f32, y as f32)))
            .map(|(x, y)| lit.iter().map(|(lx, ly)| (lx - x).hypot(ly - y)).fold(f32::MAX, f32::min) / FINE as f32)
            .collect();
        Mark { w, h, far }
    }
}

/// Words already lettered, to be laid on in a color: how much of each pixel they cover, row by row. Twice the size they're to
/// be shown at, so that they're sharp on a screen bigger than the picture's laid out for.
#[derive(Clone)]
pub struct Lettering {
    pub w: usize,
    pub h: usize,
    pub cover: Vec<u8>,
}

/// The name's letters, each as strokes from point to point in a box one tall (down is 1), and how wide it is. Our own drawing:
/// straight strokes meeting in sharp corners, with a corner cut off here and there and nothing round.
fn strokes(c: char) -> (&'static [[f32; 4]], f32) {
    match c {
        'P' => (&[[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.8, 0.0], [0.8, 0.0, 0.8, 0.34], [0.8, 0.34, 0.6, 0.54], [0.6, 0.54, 0.0, 0.54]], 0.8),
        'I' => (&[[0.0, 0.0, 0.0, 1.0]], 0.0),
        'M' => (&[[0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 0.47, 0.6], [0.47, 0.6, 0.94, 0.0], [0.94, 0.0, 0.94, 1.0]], 0.94),
        'B' => (&[[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.76, 0.0], [0.76, 0.0, 0.76, 0.34], [0.76, 0.34, 0.58, 0.48], [0.58, 0.48, 0.0, 0.48], [0.58, 0.48, 0.84, 0.66], [0.84, 0.66, 0.84, 1.0], [0.84, 1.0, 0.0, 1.0]], 0.84),
        'O' => (&[[0.22, 0.0, 0.86, 0.0], [0.86, 0.0, 0.86, 0.8], [0.86, 0.8, 0.64, 1.0], [0.64, 1.0, 0.0, 1.0], [0.0, 1.0, 0.0, 0.2], [0.0, 0.2, 0.22, 0.0]], 0.86),
        'X' => (&[[0.0, 0.0, 0.86, 1.0], [0.86, 0.0, 0.0, 1.0]], 0.86),
        _ => (&[], 0.4),
    }
}

type Point = (f32, f32);

/// How far `p` is from the line from `a` to `b`.
fn from_line(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let along = clamp01(((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy).max(1e-6));
    (p.0 - a.0 - dx * along).hypot(p.1 - a.1 - dy * along)
}

/// How far `p` is from the edge of the shape whose corners are `v`, in order: less than nothing, inside it.
fn from_shape(p: Point, v: &[Point]) -> f32 {
    let (mut far, mut inside) = (f32::MAX, false);
    for (i, a) in v.iter().enumerate() {
        let b = v[(i + v.len() - 1) % v.len()];
        far = far.min(from_line(p, *a, b));
        let side = (b.0 - a.0) * (p.1 - a.1) > (b.1 - a.1) * (p.0 - a.0);
        if (p.1 >= a.1) == (p.1 < b.1) && (p.1 >= a.1) == side {
            inside = !inside;
        }
    }
    if inside { -far } else { far }
}

/// The name, lettered: how far each pixel around it is from its letters, and `WORD_THICK` more (so that it's that at their
/// edge, and less inside them), worked out once.
struct Word {
    w: usize,
    h: usize,
    /// How wide the letters are in all, without the room round them.
    wide: f32,
    far: Vec<f32>,
}

const NAME_WORD: &str = "PIMPBOX";
/// How tall its letters are, how far they lean (they don't, now), how thick a stroke is to each side of its middle, the space between letters, and
/// the room round it all for its glow: pixels, but for the lean and the space, which are parts of the height.
const WORD_TALL: f32 = 58.0;
const WORD_LEAN: f32 = 0.0;
const WORD_THICK: f32 = 6.5;
const WORD_SPACE: f32 = 0.36;
const WORD_ROOM: f32 = 28.0;

impl Word {
    fn new() -> Word {
        let thick = WORD_THICK;
        // Each letter's a heap of shapes with straight sides: a bar for each stroke, and where two strokes meet, the corner
        // they'd come to if their outer edges went on until they met. A point, not a round.
        let mut shapes: Vec<Vec<Point>> = Vec::new();
        let mut pen = 0.0;
        for c in NAME_WORD.chars() {
            let (strokes, wide) = strokes(c);
            // Leaning forward: the higher up, the further right.
            let place = |u: f32, v: f32| (pen + (u + (1.0 - v) * WORD_LEAN) * WORD_TALL, v * WORD_TALL);
            let lines: Vec<[Point; 2]> = strokes.iter().map(|s| [place(s[0], s[1]), place(s[2], s[3])]).collect();
            let toward = |from: Point, to: Point| {
                let long = (to.0 - from.0).hypot(to.1 - from.1).max(1e-6);
                ((to.0 - from.0) / long, (to.1 - from.1) / long)
            };
            for (i, l) in lines.iter().enumerate() {
                let (dx, dy) = toward(l[0], l[1]);
                // An end that meets nothing goes on past where it's drawn to: it's cut off level with the rest, below.
                let on = |end: Point| if lines.iter().enumerate().any(|(j, o)| j != i && from_line(end, o[0], o[1]) < 0.5) { 0.0 } else { 2.0 * thick };
                let (back, forth) = (on(l[0]), on(l[1]));
                let (a, b) = ((l[0].0 - dx * back, l[0].1 - dy * back), (l[1].0 + dx * forth, l[1].1 + dy * forth));
                shapes.push(vec![(a.0 - dy * thick, a.1 + dx * thick), (b.0 - dy * thick, b.1 + dx * thick), (b.0 + dy * thick, b.1 - dx * thick), (a.0 + dy * thick, a.1 - dx * thick)]);
                for o in &lines[i + 1..] {
                    for (at, ours, theirs) in [(l[0], l[1], o), (l[1], l[0], o)].into_iter().flat_map(|(at, ours, o)| [(at, ours, (o[0], o[1])), (at, ours, (o[1], o[0]))]) {
                        if (at.0 - theirs.0.0).hypot(at.1 - theirs.0.1) >= 0.5 {
                            continue;
                        }
                        // Which way each goes from where they meet; and the corner's on the other side from both.
                        let (u, v) = (toward(at, ours), toward(at, theirs.1));
                        let (sx, sy) = (u.0 + v.0, u.1 + v.1);
                        let both = sx.hypot(sy);
                        if (u.0 * v.1 - u.1 * v.0).abs() < 1e-3 {
                            continue;
                        }
                        let (mx, my) = (-sx / both, -sy / both);
                        let reach = thick / (1.0 - (both / 2.0) * (both / 2.0)).sqrt();
                        let out = |w: Point| if -w.1 * mx + w.0 * my > 0.0 { (-w.1, w.0) } else { (w.1, -w.0) };
                        let (pu, pv) = (out(u), out(v));
                        // From the corner inside the bend to the one outside it, so that it laps over both bars and leaves no seam.
                        shapes.push(vec![(at.0 - mx * reach, at.1 - my * reach), (at.0 + pu.0 * thick, at.1 + pu.1 * thick), (at.0 + mx * reach, at.1 + my * reach), (at.0 + pv.0 * thick, at.1 + pv.1 * thick)]);
                    }
                }
            }
            pen += (wide + WORD_SPACE) * WORD_TALL;
        }
        let wide = pen - WORD_SPACE * WORD_TALL + WORD_LEAN * WORD_TALL;
        let (w, h) = ((wide + 2.0 * WORD_ROOM) as usize + 1, (WORD_TALL + 2.0 * WORD_ROOM) as usize + 1);
        // The letters' edge is what's left of the shapes' edges once every bit of one that's inside another is thrown away:
        // where they lap over each other there's no edge, and no seam.
        let mut edges: Vec<(Point, Point)> = Vec::new();
        for (i, s) in shapes.iter().enumerate() {
            for (k, a) in s.iter().enumerate() {
                let b = s[(k + 1) % s.len()];
                let bits = ((b.0 - a.0).hypot(b.1 - a.1) / 0.25).ceil().max(1.0) as usize;
                let at = |j: f32| (mix(a.0, b.0, j / bits as f32), mix(a.1, b.1, j / bits as f32));
                let mut since = None;
                for j in 0..=bits {
                    let shows = j < bits && !shapes.iter().enumerate().any(|(o, other)| o != i && from_shape(at(j as f32 + 0.5), other) < -0.05);
                    match (since, shows) {
                        (None, true) => since = Some(j),
                        (Some(from), false) => {
                            edges.push((at(from as f32), at(j as f32)));
                            since = None;
                        }
                        _ => {}
                    }
                }
            }
        }
        let far = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x as f32 - WORD_ROOM, y as f32 - WORD_ROOM)))
            .map(|p| {
                let from = edges.iter().map(|(a, b)| from_line(p, *a, *b)).fold(f32::MAX, f32::min);
                let from = if shapes.iter().any(|s| from_shape(p, s) < 0.0) { -from } else { from };
                // Nothing above the tops of the letters or below their feet: whatever went past is cut off level.
                from.max(-thick - p.1).max(p.1 - WORD_TALL - thick) + thick
            })
            .collect();
        Word { w, h, wide, far }
    }
}

const CX: f32 = W as f32 / 2.0;
const CY: f32 = H as f32 / 2.0;
/// How far away the floor is a pixel below the horizon: `GROUND` over the pixels below. The camera's eye is 1 above the floor,
/// and the ceiling `CEILING` above the eye.
const GROUND: f32 = 0.5 * H as f32;
const CEILING: f32 = 1.4;
/// The ball: how big, and how far ahead of the camera it keeps.
const BALL_R: f32 = 0.6;
const AHEAD: f32 = 2.6;
/// How fast the camera flies down the hall.
const SPEED: f32 = 2.4;

/// How lively the camera's sway is: not at all until the ball's there, and calm again by the time it crouches.
fn lively(t: f32) -> f32 {
    smooth(BALL, BALL + 0.5, t) * (1.0 - smooth(LAST, LAST + 0.3, t))
}

/// How wild the camera is, rolling and swinging round: it follows along behind the ball for a second and a half first.
fn wild(t: f32) -> f32 {
    smooth(BALL + 1.5, BALL + 2.8, t) * (1.0 - smooth(LAST, LAST + 0.3, t))
}

/// How squashed something soft is, `s` seconds after it's landed (or before, if less than none): all at once as it touches,
/// and then wobbling its way back, out past round and in again.
fn jelly(s: f32) -> f32 {
    if s < 0.0 { (s * 45.0).exp() } else { (-s * 5.5).exp() * (s * 21.0).cos() }
}

/// The ball that's thrown in: how much of its speed each bounce keeps (up and down, and along), and how many times it comes
/// down before the bounce that takes it to the middle of the picture.
const THROW_KEEPS: f32 = 0.78;
const THROW_DRAGS: f32 = 0.85;
const THROW_BOUNCES: i32 = 3;

/// How long each stretch of the throw lasts, in first falls: the fall, each whole bounce, and the half of one it ends on.
fn stretch(i: i32) -> f32 {
    match i {
        0 => 1.0,
        i if i <= THROW_BOUNCES => 2.0 * THROW_KEEPS.powi(i),
        i => THROW_KEEPS.powi(i),
    }
}

/// The throw, worked back from where it has to end, at the top of a bounce at the eye's height in the middle of the picture at
/// `BALL`: how long its first fall takes, how far that is, and how fast it's going along at first. In pixels.
fn throw() -> (f32, f32, f32) {
    let r = BALL_R * GROUND / AHEAD;
    let drop = (1.0 - BALL_R) * GROUND / AHEAD / THROW_KEEPS.powi(2 * (THROW_BOUNCES + 1));
    let fall = (BALL - GOO) / (0..=THROW_BOUNCES + 1).map(stretch).sum::<f32>();
    let pace = (CX + r + 30.0) / (0..=THROW_BOUNCES + 1).map(|i| stretch(i) * fall * THROW_DRAGS.powi(i)).sum::<f32>();
    (fall, drop, pace)
}

/// How fast the thrown ball's going across as the mirror ball takes over from it, in the hall's measure: the mirror ball
/// carries on at that.
fn carried() -> f32 {
    throw().2 * THROW_DRAGS.powi(THROW_BOUNCES + 1) * AHEAD / GROUND
}

/// Where the ball is across the hall, from the middle of the picture: off to the right first, as it was going, and then to
/// and fro.
fn ball_across(t: f32) -> f32 {
    const WEAVE: f32 = 1.9;
    carried() / WEAVE * (WEAVE * (t - BALL)).sin()
}

/// How far ahead of the camera the ball is: where it keeps, until it springs at us.
fn ball_away(t: f32) -> f32 {
    if t < LAUNCH { AHEAD } else { mix(AHEAD, 0.14, ((t - LAUNCH) / (HIT - LAUNCH)).powf(1.8)) }
}

/// How far along the hall the ball is: what goes along with it keeps to this.
fn abreast(t: f32) -> f32 {
    SPEED * (t.clamp(BALL, LAST) - BALL) + AHEAD
}

/// The camera: its roll, its zoom, and where in the hall it is.
#[derive(Clone, Copy)]
struct Cam {
    sin: f32,
    cos: f32,
    zoom: f32,
    x: f32,
    z: f32,
    /// How far the picture's shaken, across and down, in pixels.
    shake: (f32, f32),
    /// How far round the ball it's swung (and that, as a sine and cosine), and where the ball is from it: across and ahead.
    swung: f32,
    round: (f32, f32),
    pivot: (f32, f32),
}

impl Cam {
    /// Where on the picture (before its roll and zoom) something in the hall is, `across` it, `high` off its floor and `along`
    /// it; and how far off. None if it's behind us, or all but.
    fn see(&self, across: f32, high: f32, along: f32) -> Option<(f32, f32, f32)> {
        let (dx, dz) = (across - self.x - self.pivot.0, along - self.z - self.pivot.1);
        let (rx, rz) = (dx * self.round.1 + dz * self.round.0, dz * self.round.1 - dx * self.round.0);
        let away = rz + self.pivot.1;
        (away > 0.35).then(|| (CX + (rx + self.pivot.0) * GROUND / away, CY + (1.0 - high) * GROUND / away, away))
    }
}

fn cam(t: f32) -> Cam {
    let roll = wild(t) * (0.5 * (t * 1.9).sin() + 0.2 * (t * 3.1 + 2.0).sin());
    // Each landing bounces how much it takes in: in, and out past where it was, and back; and shakes it, a little.
    let since = || LANDS.iter().enumerate().filter(|(_, at)| t >= **at && t < HIT).map(|(i, at)| (i as f32, t - at));
    let punch: f32 = since().map(|(_, s)| (-s * 6.0).exp() * (s * 17.0).cos()).sum();
    let shake = since().map(|(i, s)| (3.5 * (-s * 8.0).exp() * (s * 53.0 + i * 2.0).sin(), 3.5 * (-s * 8.0).exp() * (s * 44.0 + i * 1.3).cos())).fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
    // Down the hall until the last landing, and there it stops, facing the ball squarely.
    let z = SPEED * (t.clamp(BALL, LAST) - BALL);
    let sway = |t: f32| 0.6 * (t * 1.1).sin() * lively(t);
    let down = sway(LAST) + ball_across(LAST);
    let x = if t < LAST { sway(t) } else { mix(sway(LAST), down, smooth(LAST, LAST + 0.3, t)) };
    let pivot = if t < LAST { (ball_across(t), AHEAD) } else { (down - x, AHEAD) };
    // Round it it goes, once it's done following: steadily, with a swing to and fro on top, and to rest as the ball crouches.
    let swung = 5.6 * smooth(BALL + 1.5, LAST + 0.3, t) + 0.45 * (t * 2.3).sin() * wild(t);
    // It breathes in and out a little, as well as punching in.
    let zoom = 1.0 + 0.045 * punch + 0.025 * (t * 2.7).sin() * lively(t);
    Cam { sin: roll.sin(), cos: roll.cos(), zoom, x, z, shake, swung, round: swung.sin_cos(), pivot }
}

/// Where the ball came down, each time: across the hall and along it.
fn landing(i: usize) -> (f32, f32) {
    let at = LANDS[i];
    let c = cam(at - 0.0001);
    (c.x + ball_across(at), c.z + AHEAD)
}

/// The ball at a moment: where on the picture, how big, how squashed, how solid, and how far it's turned.
struct Ball {
    x: f32,
    y: f32,
    r: f32,
    wide: f32,
    tall: f32,
    alpha: f32,
    turn: f32,
    /// Across the hall and along it, and how high its middle is off the floor: for the light it throws down.
    at: (f32, f32),
    high: f32,
    /// How far its facets have run together into plain gold, as it comes at us.
    melt: f32,
}

fn ball(t: f32, c: &Cam) -> Option<Ball> {
    if !(BALL..HIT).contains(&t) {
        return None;
    }
    // It's soft: flattened where it meets the floor, each time, and wobbling after; and drawn out long by its own speed.
    const FLATTENS: f32 = 0.42;
    const DRAWN_OUT: f32 = 0.16;
    let soft: f32 = LANDS.iter().map(|at| jelly(t - at)).sum();
    // How much it's on the floor, where being flatter or longer leaves its underside where it was.
    let mut down = LANDS.iter().map(|at| (-(t - at).abs() * 10.0).exp()).sum::<f32>().min(1.0);
    let (mut away, mut melt) = (AHEAD, 0.0);
    let (high, tall) = if t < LANDS[0] {
        // It falls from the eye's height, where the one thrown in left off.
        let p = (t - BALL) / (LANDS[0] - BALL);
        (mix(1.0, BALL_R, p * p), 1.0 + DRAWN_OUT * p - FLATTENS * soft)
    } else if t < LAST {
        // Then bounces, lower each time.
        let i = LANDS.iter().rposition(|at| t >= *at).unwrap_or(0);
        let p = (t - LANDS[i]) / (LANDS[i + 1] - LANDS[i]);
        (BALL_R + PEAKS[i] * 4.0 * p * (1.0 - p), 1.0 + DRAWN_OUT * PEAKS[i] * (1.0 - 2.0 * p).abs() - FLATTENS * soft)
    } else if t < LAUNCH {
        // Down for good, it crouches: flatter and flatter against the floor, winding up.
        let crouch = smooth(0.0, (LAUNCH - LAST) * 0.85, t - LAST);
        down = 1.0;
        (BALL_R, (1.0 + DRAWN_OUT * PEAKS[PEAKS.len() - 1] * (-(t - LAST) * 30.0).exp() - FLATTENS * soft - 0.36 * crouch).max(0.45))
    } else {
        // And springs at us: up out of its crouch and past round, long and thin, wobbling, and gold by the time it's here.
        let (since, p) = (t - LAUNCH, (t - LAUNCH) / (HIT - LAUNCH));
        (away, melt) = (ball_away(t), smooth(0.25, 0.9, p));
        down = 1.0 - smooth(0.0, 0.5, p);
        (mix(BALL_R, 1.0, smooth(0.0, 0.7, p)), 1.0 - 0.38 * (-since * 30.0).exp() + 0.4 * (-since * 9.0).exp() * (since * 26.0).sin())
    };
    // As much wider as it's flatter, near enough: there's as much of it either way.
    let wide = tall.powf(-0.5);
    let across = ball_across(t);
    let r = BALL_R * GROUND / away;
    Some(Ball {
        x: CX + c.pivot.0 * GROUND / away,
        y: CY + (1.0 - high + down * BALL_R * (1.0 - tall)) * GROUND / away,
        r,
        wide,
        tall,
        alpha: 1.0,
        turn: t * 2.6,
        at: (c.x + if t < LAST { across } else { c.pivot.0 }, c.z + AHEAD),
        high,
        melt,
    })
}

/// The ball that's thrown in from off the left, before there's a mirror ball: it falls, and bounces lower and quicker each time
/// as a ball does, soft as it is, on the floor the hall will have; and it's at the top of a bounce in the middle of the picture,
/// still going, when the mirror ball takes over. Where its middle is, how wide and tall it's squashed, and how far it's rolled.
struct Thrown {
    x: f32,
    y: f32,
    wide: f32,
    tall: f32,
    turn: f32,
}

fn thrown(t: f32) -> Thrown {
    let r = BALL_R * GROUND / AHEAD;
    let (fall, drop, pace) = throw();
    // Where its middle is when it's on the floor, and where it starts from.
    let (rests, from) = (CY + (1.0 - BALL_R) * GROUND / AHEAD, -r - 30.0);
    let gravity = 2.0 * drop / (fall * fall);
    let since = (t - GOO).clamp(0.0, BALL - GOO);
    let (mut began, mut gone, mut up, mut fast) = (0.0, 0.0, 0.0, 0.0f32);
    let (mut soft, mut down) = (0.0f32, 0.0f32);
    for i in 0..=THROW_BOUNCES + 1 {
        let lasts = stretch(i) * fall;
        let into = (since - began).clamp(0.0, lasts);
        gone += into * THROW_DRAGS.powi(i);
        if since >= began && (since < began + lasts || i == THROW_BOUNCES + 1) {
            // Falling from where it was let go, or going up from the floor as fast as the bounce before left it.
            let (from, off) = if i == 0 { (drop, 0.0) } else { (0.0, THROW_KEEPS.powi(i) * gravity * fall) };
            up = from + off * into - 0.5 * gravity * into * into;
            fast = (off - gravity * into) / (gravity * fall);
        }
        began += lasts;
        if i <= THROW_BOUNCES {
            // It flattens as it lands, the harder the faster it came down, and wobbles after.
            soft += THROW_KEEPS.powi(i) * jelly((since - began) * 1.7);
            down += (-(since - began).abs() * 14.0).exp();
        }
    }
    let tall = (1.0 + 0.14 * fast.abs() - 0.4 * soft).max(0.5);
    let x = from + pace * gone;
    Thrown { x, y: rests - up.max(0.0) + down.min(1.0) * r * (1.0 - tall), wide: tall.powf(-0.6), tall, turn: (x - from) / r }
}

/// Where a die's middle is when it lies on the floor.
const LIES: f32 = 0.2;

/// Something hard, thrown: `since` it was let go `from` high, going `up` so fast. How far along its way it's got (in seconds
/// of going at the pace it set out at: each time it comes down it loses some, and in the end it lies where it is), how high
/// it is, and, once it's come down at all, how long ago it last did and how far along that was.
fn tossed(since: f32, from: f32, up: f32) -> (f32, f32, Option<(f32, f32)>) {
    const PULL: f32 = 7.0;
    const KEEPS: f32 = 0.62;
    const DRAGS: f32 = 0.72;
    const SLIDES: f32 = 0.3;
    let mut fast = (up * up + 2.0 * PULL * (from - LIES).max(0.0)).sqrt();
    let mut lasts = (up + fast) / PULL;
    if since < lasts {
        return (since, from + up * since - 0.5 * PULL * since * since, None);
    }
    let (mut began, mut gone, mut pace) = (lasts, lasts, DRAGS);
    loop {
        fast *= KEEPS;
        let into = since - began;
        if fast < 0.5 {
            // Too slow to leave the floor again: it slides to a stop.
            let slid = into.min(SLIDES);
            return (gone + pace * (slid - slid * slid / (2.0 * SLIDES)), LIES, Some((into, gone)));
        }
        lasts = 2.0 * fast / PULL;
        if into < lasts {
            return (gone + pace * into, LIES + fast * into - 0.5 * PULL * into * into, Some((into, gone)));
        }
        (began, gone, pace) = (began + lasts, gone + pace * lasts, pace * DRAGS);
    }
}

/// Somewhere on the floor that's just been knocked on: where, across the hall and along it, how long since, and how hard.
struct Knock {
    at: (f32, f32),
    since: f32,
    hard: f32,
}

/// A die on the picture at a moment: where, how big (half its width), how it's turned, how solid, and whether it's behind the ball.
struct Die {
    x: f32,
    y: f32,
    /// How far off it is.
    away: f32,
    half: f32,
    turn: (f32, f32),
    alpha: f32,
    behind: bool,
}

/// The dice there are at `t`, the furthest first, and where the floor's been knocked on. All of them are somewhere in the hall,
/// as the ball is, and are seen from wherever the camera's got to: the ones going their own way across it, the schools, the
/// ones thrown down it; and the ones that come out from behind the ball when it springs.
fn dice_at(t: f32, c: &Cam, ball: Option<&Ball>) -> (Vec<Die>, Vec<Knock>) {
    let (mut all, mut knocks) = (Vec::new(), Vec::new());
    if !(BALL..HIT).contains(&t) {
        return (Vec::new(), knocks);
    }
    // One of them, somewhere in the hall, as it's seen: if it is.
    let mut put = |across: f32, high: f32, along: f32, size: f32, turn: (f32, f32), alpha: f32| {
        if let Some((x, y, away)) = c.see(across, high, along) {
            // The further off, the fainter, and gone before it's as far as the dark beyond.
            all.push((away, Die { x, y, away, half: size * GROUND / away, turn, alpha: alpha * smooth(13.0, 7.5, away), behind: away > ball_away(t) }));
        }
    };
    // Going their own way across the hall, past the ball: when each sets out, how far beyond the ball it keeps (or short of
    // it), how high, which way it's going, and how it starts out turned.
    const PASSING: [(f32, f32, f32, f32, f32); 5] = [(2.1, -0.9, 0.65, 1.0, 1.0), (2.9, 1.0, 1.5, -1.0, 2.3), (3.5, -0.6, 1.45, -1.0, 4.1), (4.0, 0.7, 0.7, 1.0, 5.6), (4.9, 1.4, 0.45, -1.0, 3.0)];
    for (when, beyond, high, way, seed) in PASSING {
        let since = t - LATER - when;
        // Across at a steady pace, bobbing a little, turning slowly.
        let across = way * (since * 1.9 - 3.4);
        if since >= 0.0 && across.abs() < 3.6 {
            put(across, high + 0.07 * (since * 2.1 + seed).sin(), abreast(t) + beyond, 0.17, (since * 1.3 + seed, since * 0.9 + seed * 2.0), 1.0);
        }
    }
    // Schools of them, each die trailing the one ahead along the same weaving way, as fish do: when each school sets out, how
    // long it takes to cross, where from and to (across the hall, and beyond the ball), and how high.
    type Place = (f32, f32);
    const SCHOOLS: [(f32, f32, Place, Place, f32); 2] = [(2.3, 2.6, (-7.0, 4.6), (7.0, 3.2), 1.7), (3.7, 2.4, (6.5, -5.2), (-6.5, -3.4), 1.25)];
    const EACH: usize = 18;
    for (s, (when, lasts, from, to, high)) in SCHOOLS.iter().enumerate() {
        for i in 0..EACH {
            let k = i as f32;
            // Each a little behind the one ahead, and a little to one side of the way, by which it is.
            let p = (t - LATER - when) / lasts - k * 0.016;
            if !(-0.1..1.1).contains(&p) {
                continue;
            }
            let weave = (p * 9.0 + s as f32 * 2.0).sin();
            let across = mix(from.0, to.0, p) + (k * 2.4).sin() * 0.22;
            let beyond = mix(from.1, to.1, p) + weave * 0.35 + (k * 3.1).sin() * 0.2;
            let high = high + (k * 1.7).cos() * 0.26 + 0.2 * (p * 7.0 + k * 0.3).sin();
            // All turned much alike, as a school is, each a touch off the rest.
            put(across, high, abreast(t) + beyond, 0.13 * (1.0 + 0.2 * (k * 1.3).sin()), (p * 5.0 + k * 0.25, p * 3.0 + weave * 0.5), 0.9);
        }
    }
    // Thrown down the hall, to knock on its floor and tumble to a stop there: when each is let go, where (across the hall, and
    // beyond where the ball is then), which way and how fast, how high and how fast upward, and how it starts out turned.
    const TOSSED: [(f32, Place, Place, f32, f32, f32); 6] = [
        (2.15, (-1.7, -2.2), (0.8, 4.6), 1.5, 1.2, 0.7),
        (2.6, (1.9, -1.8), (-0.9, 4.9), 1.3, 2.0, 2.9),
        (3.1, (-2.4, 0.6), (1.6, 3.4), 1.8, 0.5, 4.4),
        (3.7, (2.2, 1.5), (-1.9, 2.6), 1.2, 2.2, 1.3),
        (4.3, (-0.6, 2.6), (0.9, 0.8), 1.9, 1.0, 5.2),
        (4.8, (1.2, -2.4), (-1.4, 1.5), 1.6, 1.6, 3.6),
    ];
    for (when, from, pace, high, up, seed) in TOSSED {
        let when = when + LATER;
        if t < when {
            continue;
        }
        let (gone, high, last) = tossed(t - when, high, up);
        let at = |gone: f32| (from.0 + pace.0 * gone, abreast(when) + from.1 + pace.1 * gone);
        put(at(gone).0, high, at(gone).1, 0.17, (seed + gone * 4.2, seed * 2.0 + gone * 3.1), 1.0);
        if let Some((since, landed)) = last.filter(|(since, _)| *since < 0.9) {
            knocks.push(Knock { at: at(landed), since, hard: 0.9 });
        }
    }
    // The dolphin, coming up through the floor and going back down through it.
    for p in [LEAP_OUT, 1.0 - LEAP_OUT] {
        let when = LEAPS.0 + p * LEAPS.1;
        if (when..when + 1.2).contains(&t) {
            let (across, _, beyond) = leap(p);
            knocks.push(Knock { at: (across, abreast(when) + beyond), since: t - when, hard: 1.5 });
        }
    }
    // With the ball: which way out from behind it each goes, across and up, and how it starts out turned. Each starts right
    // behind the ball, where it can't be seen, and comes out round it as they both come at us, the ball a little ahead.
    const WITH: [(f32, f32, f32); 5] = [(-1.5, -0.5, 3.2), (1.35, -0.75, 1.7), (1.0, 0.6, 4.8), (-0.9, 0.55, 0.9), (0.15, -1.2, 2.6)];
    if let (Some(b), true) = (ball, t >= LAUNCH) {
        let p = (t - LAUNCH) / (HIT - LAUNCH);
        for (i, (across, up, seed)) in WITH.iter().enumerate() {
            let out = smooth(0.0, 0.75, p - i as f32 * 0.04);
            let away = ball_away(t) + 0.5 * (1.0 - p) + 0.08;
            all.push((
                away,
                Die {
                    away,
                    x: CX + (c.pivot.0 + across * out) * GROUND / away,
                    y: CY + (1.0 - b.high + up * out) * GROUND / away,
                    half: 0.17 * GROUND / away,
                    turn: (p * 4.0 + seed, p * 3.0 + seed * 2.0),
                    alpha: 1.0,
                    behind: true,
                },
            ));
        }
    }
    all.sort_by(|a, b| b.0.total_cmp(&a.0));
    (all.into_iter().map(|(_, die)| die).collect(), knocks)
}

/// The dolphin's leap: when it begins and how long it takes; and how far through it the dolphin's middle comes up through
/// the floor (and that far from its end, goes back down).
const LEAPS: (f32, f32) = (BALL + 0.8, 1.6);
const LEAP_OUT: f32 = 0.064;

/// Where the dolphin is, `p` of the way through its leap: across the hall, how high, and how far beyond the ball. Up out of the
/// floor on one side, over in an arc round the back of the ball, and down into the floor on the other.
fn leap(p: f32) -> (f32, f32, f32) {
    let arc = (std::f32::consts::PI * p).sin();
    (mix(-3.4, 3.4, p), -0.4 + 2.0 * arc, 1.4 + 0.7 * arc)
}

/// The dolphin on the picture at a moment: where its middle is, half its length, which way it's headed, where the floor is at
/// its distance (what's below that is under it, unseen), and whether it's behind the ball.
struct Fish {
    x: f32,
    y: f32,
    half: f32,
    heading: (f32, f32),
    floor: f32,
    behind: bool,
}

fn fish(t: f32, c: &Cam) -> Option<Fish> {
    let p = (t - LEAPS.0) / LEAPS.1;
    if !(0.0..1.0).contains(&p) {
        return None;
    }
    let at = |p: f32| {
        let (across, high, beyond) = leap(p);
        c.see(across, high, abreast(t) + beyond)
    };
    let ((x, y, away), (nx, ny, _)) = (at(p)?, at(p + 0.02)?);
    let far = (nx - x).hypot(ny - y).max(1e-3);
    Some(Fish { x, y, half: 1.35 * GROUND / away, heading: ((nx - x) / far, (ny - y) / far), floor: CY + GROUND / away, behind: away > AHEAD })
}

/// A moment: where everything in it is.
struct Scene {
    cam: Cam,
    ball: Option<Ball>,
    dice: Vec<Die>,
    knocks: Vec<Knock>,
    fish: Option<Fish>,
}

/// The animation, ready to draw.
pub struct Boot {
    mark: Mark,
    word: Word,
    maker: Option<Lettering>,
    small: Option<Lettering>,
    lands: [(f32, f32); LANDS.len()],
}

impl Default for Boot {
    fn default() -> Self {
        Boot::new()
    }
}

impl Boot {
    pub fn new() -> Boot {
        Boot { mark: Mark::new(), word: Word::new(), maker: None, small: None, lands: std::array::from_fn(landing) }
    }

    /// With the maker's line lettered already, rather than in dots.
    pub fn with_maker(self, maker: Lettering) -> Boot {
        Boot { maker: Some(maker), ..self }
    }

    /// With the small print under that lettered already, too.
    pub fn with_small_print(self, small: Lettering) -> Boot {
        Boot { small: Some(small), ..self }
    }

    /// Whether it's over at `t`.
    pub fn over(t: f32) -> bool {
        t >= LENGTH
    }

    /// The moment at `t`: where everything is, how much the picture's whitened, and how far it's gone out.
    fn scene(&self, t: f32) -> (Scene, f32, f32) {
        let cam = cam(t);
        let ball = ball(t, &cam);
        let (dice, knocks) = dice_at(t, &cam, ball.as_ref());
        let scene = Scene { dice, knocks, fish: fish(t, &cam), ball, cam };
        // Each landing whitens the picture for an instant, until the cut to the mark. It ends by going out, quickly.
        let flash = if t < HIT { LANDS.iter().filter(|at| t >= **at).map(|at| (-(t - at) * 9.0).exp()).sum::<f32>() * 0.22 } else { 0.0 };
        (scene, flash, 1.0 - smooth(LENGTH - 0.35, LENGTH, t))
    }
}

/// Lettering from the fonts Windows has, for whoever shows this on Windows.
#[cfg(feature = "system-fonts")]
pub mod fonts {
    use windows::Win32::Foundation::{COLORREF, SIZE};
    use windows::Win32::Graphics::Gdi::*;
    use windows::core::HSTRING;

    /// The animation as the console shows it: the maker's line in Arial Black, leaning, and the small print in Arial, if
    /// Windows has them. In dots, otherwise.
    pub fn boot() -> super::Boot {
        match lettering("Purple Hat", "Arial Black", 900, true, 60).zip(lettering("All whites reserved.", "Arial", 400, false, 20)) {
            Some((maker, small)) => super::Boot::new().with_maker(maker).with_small_print(small),
            None => super::Boot::new(),
        }
    }

    /// `text` in the font called `face`, `tall` pixels tall, as heavy as `weight` (400 is regular, 900 the heaviest) and leaning
    /// if `italic`. None if Windows won't.
    pub fn lettering(text: &str, face: &str, weight: i32, italic: bool, tall: i32) -> Option<super::Lettering> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let font = CreateFontW(-tall, 0, 0, 0, weight, u32::from(italic), 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, 0, &HSTRING::from(face));
            let old_font = SelectObject(dc, font.into());
            let wide: Vec<u16> = text.encode_utf16().collect();
            let mut size = SIZE::default();
            let _ = GetTextExtentPoint32W(dc, &wide, &mut size);
            // Room for letters that lean out past where they're measured to.
            let (w, h) = (size.cx + tall, size.cy);
            let info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() }, ..Default::default() };
            let mut bits = std::ptr::null_mut();
            let lettered = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).ok().filter(|_| !bits.is_null() && w > 0 && h > 0).map(|bitmap| {
                let old = SelectObject(dc, bitmap.into());
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, COLORREF(0x00FF_FFFF));
                let _ = TextOutW(dc, tall / 2, 0, &wide);
                let _ = GdiFlush();
                let pixels = std::slice::from_raw_parts(bits as *const u32, (w * h) as usize);
                let cover = pixels.iter().map(|p| (p >> 8 & 0xFF) as u8).collect();
                SelectObject(dc, old);
                let _ = DeleteObject(bitmap.into());
                super::Lettering { w: w as usize, h: h as usize, cover }
            });
            SelectObject(dc, old_font);
            let _ = DeleteObject(font.into());
            let _ = DeleteDC(dc);
            lettered
        }
    }
}

/// The graphics card drawing it, with Direct3D 11: every pixel worked out there, by a shader (boot.hlsl), from what moves,
/// which is worked out here once a frame and handed to it. It draws into whatever it's given at whatever size that is, so on
/// a big screen it's as sharp as the screen.
pub mod gpu {
    use super::*;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob};
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R32_FLOAT, DXGI_SAMPLE_DESC};
    use windows::core::{Error, PCSTR, Result, s};

    /// As many dice and knocks as the shader has room for.
    const DICE: usize = 72;
    const KNOCKS: usize = 8;

    /// A moment, as the shader's handed it: see the top of boot.hlsl, which this has to match.
    #[repr(C)]
    struct Moment {
        clock: [f32; 4],
        cam0: [f32; 4],
        cam1: [f32; 4],
        cam2: [f32; 4],
        cam3: [f32; 4],
        ball0: [f32; 4],
        ball1: [f32; 4],
        ball2: [f32; 4],
        thrown0: [f32; 4],
        thrown1: [f32; 4],
        fish0: [f32; 4],
        fish1: [f32; 4],
        lands: [[f32; 4]; LANDS.len()],
        knocks: [[f32; 4]; KNOCKS],
        dice: [[f32; 4]; DICE * 2],
    }

    fn moment(boot: &Boot, t: f32) -> Moment {
        let (scene, flash, dim) = boot.scene(t);
        let c = &scene.cam;
        let mut m = Moment {
            clock: [t, flash, dim, 0.0],
            cam0: [c.sin, c.cos, c.zoom, c.swung],
            cam1: [c.x, c.z, c.pivot.0, c.pivot.1],
            cam2: [c.round.0, c.round.1, scene.dice.len().min(DICE) as f32, scene.knocks.len().min(KNOCKS) as f32],
            cam3: [c.shake.0, c.shake.1, 0.0, 0.0],
            ball0: [0.0; 4],
            ball1: [1.0, 1.0, 0.0, 0.0],
            ball2: [0.0, 0.0, 1.0, AHEAD],
            thrown0: [0.0; 4],
            thrown1: [0.0; 4],
            fish0: [0.0; 4],
            fish1: [0.0; 4],
            lands: boot.lands.map(|(x, z)| [x, z, 0.0, 0.0]),
            knocks: [[0.0; 4]; KNOCKS],
            dice: [[0.0; 4]; DICE * 2],
        };
        if let Some(b) = &scene.ball {
            (m.ball0, m.ball1, m.ball2) = ([b.x, b.y, b.r, b.alpha], [b.wide, b.tall, b.turn, b.melt], [b.at.0, b.at.1, b.high, ball_away(t)]);
        }
        let b = thrown(t);
        (m.thrown0, m.thrown1) = ([b.x, b.y, b.wide, b.tall], [b.turn, 0.0, 0.0, 0.0]);
        if let Some(f) = &scene.fish {
            (m.fish0, m.fish1) = ([f.x, f.y, f.half, f.floor], [f.heading.0, f.heading.1, if f.behind { 1.0 } else { 2.0 }, 0.0]);
        }
        for (to, k) in m.knocks.iter_mut().zip(&scene.knocks) {
            *to = [k.at.0, k.at.1, k.since, k.hard];
        }
        for (to, d) in m.dice.chunks_mut(2).zip(&scene.dice) {
            (to[0], to[1]) = ([d.x, d.y, d.half, d.alpha], [d.turn.0, d.turn.1, if d.behind { 1.0 } else { 0.0 }, d.away]);
        }
        m
    }

    /// What's written above the shader: the pace, the sizes and the dots, from the ones here, so there's one of each.
    fn header(boot: &Boot, maker: &Lettering, small: &Lettering) -> String {
        let mut h = String::new();
        let mut say = |name: &str, value: f32| h.push_str(&format!("#define {name} ({value:?})\n"));
        say("W", W as f32);
        say("H", H as f32);
        say("GOO", GOO);
        say("RAIN", RAIN);
        say("FLIP", FLIP);
        say("FPS", FPS as f32);
        say("BALL", BALL);
        say("LAUNCH", LAUNCH);
        say("HIT", HIT);
        say("NAME", NAME);
        say("HAT", HAT);
        say("MADE", MADE);
        say("LENGTH", LENGTH);
        say("GROUND", GROUND);
        say("CEILING", CEILING);
        say("BALL_R", BALL_R);
        say("AHEAD", AHEAD);
        say("FINE", FINE as f32);
        say("ROOM", ROOM as f32);
        say("COLS", COLS as f32);
        say("ROWS", ROWS as f32);
        say("MARK_W", boot.mark.w as f32);
        say("MARK_H", boot.mark.h as f32);
        say("WORD_W", boot.word.w as f32);
        say("WORD_H", boot.word.h as f32);
        say("WORD_WIDE", boot.word.wide);
        say("WORD_TALL", WORD_TALL);
        say("WORD_THICK", WORD_THICK);
        say("WORD_ROOM", WORD_ROOM);
        say("MAKER_W", maker.w as f32);
        say("MAKER_H", maker.h as f32);
        say("SMALL_W", small.w as f32);
        say("SMALL_H", small.h as f32);
        let list = |all: Vec<String>| all.join(", ");
        h.push_str(&format!("static const float LANDS[{}] = {{ {} }};\n", LANDS.len(), list(LANDS.iter().map(|at| format!("{at:?}")).collect())));
        let rows = DOLLAR.iter().map(|row| format!("{}u", row.bytes().fold(0u32, |bits, dot| bits << 1 | u32::from(dot == b'#'))));
        h.push_str(&format!("static const uint DOLLAR[{ROWS}] = {{ {} }};\n", list(rows.collect())));
        let hat = HAT_DOTS.iter().flat_map(|row| row.bytes()).map(|dot| format!("{}u", b"pdgf".iter().position(|c| *c == dot).map_or(0, |i| i + 1)));
        h.push_str(&format!("static const uint HAT_DOTS[{}] = {{ {} }};\n", HAT_DOTS.len() * HAT_DOTS[0].len(), list(hat.collect())));
        let name = NAME_WORD.chars().flat_map(letter).map(|row| format!("{row}u"));
        h.push_str(&format!("static const uint NAME_ROWS[{}] = {{ {} }};\n", NAME_WORD.len() * 7, list(name.collect())));
        h
    }

    /// Words in dots, `dot` pixels to the dot, for when nobody's lettered them: what the shader's handed either way.
    fn dotted(words: &str, dot: usize) -> Lettering {
        let (w, h) = ((words.len() * 6 - 1) * dot, 7 * dot);
        let mut cover = vec![0; w * h];
        for (i, c) in words.chars().enumerate() {
            for (row, bits) in letter(c).iter().enumerate() {
                for col in (0..5).filter(|col| bits >> (4 - col) & 1 == 1) {
                    for y in 0..dot {
                        let from = (row * dot + y) * w + (i * 6 + col) * dot;
                        cover[from..from + dot].fill(255);
                    }
                }
            }
        }
        Lettering { w, h, cover }
    }

    /// The animation, and a graphics card to draw it with.
    pub struct Gpu {
        boot: Boot,
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        corners: ID3D11VertexShader,
        pixels: ID3D11PixelShader,
        moment: ID3D11Buffer,
        tables: [Option<ID3D11ShaderResourceView>; 4],
        /// A picture of its own to draw into, of the size last asked for, and one it can be read back from.
        own: Option<((usize, usize), ID3D11Texture2D, ID3D11RenderTargetView, ID3D11Texture2D)>,
        /// It's the short one that's drawn: `SHORT` long, not `LENGTH`.
        short: bool,
    }

    /// D3DCOMPILE_SKIP_OPTIMIZATION: the shader's ready in a quarter of the time, and the card's driver makes as good a job
    /// of it as it would have: it draws no slower. Whoever turns the console on is waiting for this.
    const QUICKLY: u32 = 1 << 2;

    fn compiled(source: &str, entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
        let (mut code, mut said) = (None, None);
        let done = unsafe { D3DCompile(source.as_ptr().cast(), source.len(), s!("boot.hlsl"), None, None, entry, target, QUICKLY, 0, &mut code, Some(&mut said)) };
        if let (Err(e), Some(said)) = (&done, &said) {
            let why = unsafe { std::slice::from_raw_parts(said.GetBufferPointer().cast::<u8>(), said.GetBufferSize()) };
            // What stopped it, not what it only grumbled about: there's only so much room to say it in.
            let why = String::from_utf8_lossy(why);
            let stopped: Vec<&str> = why.lines().filter(|line| line.contains("error")).collect();
            return Err(Error::new(e.code(), if stopped.is_empty() { why.trim_end_matches('\0').to_string() } else { stopped.join("\n") }));
        }
        done?;
        code.ok_or_else(Error::empty)
    }

    /// Where a shader's kept compiled, in the folder `kept`: under a name that's its own, so that another shader (another
    /// version of this, or other lettering) is another file.
    fn kept_as(source: &str, kept: &std::path::Path) -> [std::path::PathBuf; 2] {
        // FNV-1a: the same name for the same shader, every time, on every machine.
        let name = source.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
        ["vs", "ps"].map(|stage| kept.join(format!("boot-{name:016x}-{QUICKLY}.{stage}")))
    }

    /// The shader, compiled: its two stages, and how long compiling took, if it had to be compiled. With somewhere to keep
    /// it, it's taken from there when it's there, and put there when it isn't.
    fn shaders(source: &str, kept: Option<&std::path::Path>) -> Result<(Vec<u8>, Vec<u8>, Option<std::time::Duration>)> {
        let files = kept.map(|kept| kept_as(source, kept));
        if let Some([vs, ps]) = &files
            && let (Ok(vs), Ok(ps)) = (std::fs::read(vs), std::fs::read(ps))
            && !vs.is_empty()
            && !ps.is_empty()
        {
            return Ok((vs, ps, None));
        }
        let began = std::time::Instant::now();
        let (vs, ps) = (bytes(&compiled(source, s!("vs"), s!("vs_5_0"))?).to_vec(), bytes(&compiled(source, s!("ps"), s!("ps_5_0"))?).to_vec());
        let took = began.elapsed();
        // Kept for next time, if it can be: it's only time that's lost if it can't.
        if let (Some(kept), Some([to_vs, to_ps])) = (kept, &files)
            && std::fs::create_dir_all(kept).is_ok()
        {
            let _ = std::fs::write(to_vs, &vs).and_then(|()| std::fs::write(to_ps, &ps));
        }
        Ok((vs, ps, Some(took)))
    }

    /// Whether the shader for `boot` is kept compiled in the folder `kept`: whether showing it means compiling it first.
    pub fn is_kept(boot: &Boot, kept: &std::path::Path) -> bool {
        let lettered = boot.maker.clone().unwrap_or_else(|| dotted("PURPLE HAT", 6));
        let small = boot.small.clone().unwrap_or_else(|| dotted("ALL WHITES RESERVED.", 2));
        let source = header(boot, &lettered, &small) + include_str!("boot.hlsl");
        kept_as(&source, kept).iter().all(|file| std::fs::metadata(file).is_ok_and(|m| m.len() > 0))
    }

    fn bytes(blob: &ID3DBlob) -> &[u8] {
        unsafe { std::slice::from_raw_parts(blob.GetBufferPointer().cast(), blob.GetBufferSize()) }
    }

    fn texture(device: &ID3D11Device, (w, h): (usize, usize), format: DXGI_FORMAT, usage: D3D11_USAGE, bind: u32, cpu: u32, data: Option<(*const u8, usize)>) -> Result<ID3D11Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w as u32,
            Height: h as u32,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: usage,
            BindFlags: bind,
            CPUAccessFlags: cpu,
            MiscFlags: 0,
        };
        let data = data.map(|(from, pitch)| D3D11_SUBRESOURCE_DATA { pSysMem: from.cast(), SysMemPitch: pitch as u32, SysMemSlicePitch: 0 });
        let mut made = None;
        unsafe { device.CreateTexture2D(&desc, data.as_ref().map(|d| d as *const _), Some(&mut made))? };
        made.ok_or_else(Error::empty)
    }

    /// Numbers for the shader to look up: a table of them, `w` by `h`.
    fn table<P>(device: &ID3D11Device, w: usize, h: usize, format: DXGI_FORMAT, of: &[P]) -> Result<Option<ID3D11ShaderResourceView>> {
        assert_eq!(of.len(), w * h);
        let made = texture(device, (w, h), format, D3D11_USAGE_IMMUTABLE, D3D11_BIND_SHADER_RESOURCE.0 as u32, 0, Some((of.as_ptr().cast(), w * size_of::<P>())))?;
        let mut view = None;
        unsafe { device.CreateShaderResourceView(&made, None, Some(&mut view))? };
        Ok(view)
    }

    impl Gpu {
        /// With the graphics card there is, or failing that with Windows' own stand-in for one. Its shader's compiled there
        /// and then, which takes a moment.
        pub fn new(boot: Boot) -> Result<Gpu> {
            Self::made(boot, None).map(|(gpu, _)| gpu)
        }

        /// The same, with its shader kept compiled in the folder `kept`: taken from there if it's there (and is this very
        /// shader, lettering and all), compiled and put there if it isn't. How long compiling it took, if it was compiled.
        pub fn cached(boot: Boot, kept: &std::path::Path) -> Result<(Gpu, Option<std::time::Duration>)> {
            Self::made(boot, Some(kept))
        }

        /// Draws a frame from every part of it, long and short, at each of `sizes`, to nowhere anyone sees: whatever the
        /// card's driver puts off until a shader's first used for something is done with now, and not while it's shown.
        pub fn warm(&mut self, sizes: &[(usize, usize)]) -> Result<()> {
            let was = self.short;
            let mut out = Vec::new();
            for size in sizes {
                out.resize(size.0 * size.1, 0);
                for short in [false, true] {
                    self.short = short;
                    let times: &[f32] = if short { &[0.1, 0.6, 1.3, 2.5, SHORT - 0.2] } else { &[0.2, RAIN + 0.05, FLIP + 0.005, GOO + 0.8, BALL + 0.3, BALL + 1.0, BALL + 2.4, LAUNCH + 0.25, HIT + 0.05, HIT + 1.2, MADE + 0.5, LENGTH - 0.2] };
                    for t in times {
                        self.picture(*t, *size, &mut out)?;
                    }
                }
            }
            self.short = was;
            Ok(())
        }

        fn made(boot: Boot, kept: Option<&std::path::Path>) -> Result<(Gpu, Option<std::time::Duration>)> {
            let (mut device, mut context) = (None, None);
            let with = |kind, device: &mut Option<ID3D11Device>, context: &mut Option<ID3D11DeviceContext>| unsafe {
                D3D11CreateDevice(None, kind, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, Some(&[D3D_FEATURE_LEVEL_11_0]), D3D11_SDK_VERSION, Some(device), None, Some(context))
            };
            if with(D3D_DRIVER_TYPE_HARDWARE, &mut device, &mut context).is_err() {
                with(D3D_DRIVER_TYPE_WARP, &mut device, &mut context)?;
            }
            let (device, context) = (device.ok_or_else(Error::empty)?, context.ok_or_else(Error::empty)?);
            let lettered = boot.maker.clone().unwrap_or_else(|| dotted("PURPLE HAT", 6));
            let small = boot.small.clone().unwrap_or_else(|| dotted("ALL WHITES RESERVED.", 2));
            let source = header(&boot, &lettered, &small) + include_str!("boot.hlsl");
            let (vs, ps, took) = shaders(&source, kept)?;
            let (mut corners, mut pixels, mut buffer) = (None, None, None);
            let room = D3D11_BUFFER_DESC { ByteWidth: size_of::<Moment>() as u32, Usage: D3D11_USAGE_DEFAULT, BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32, ..Default::default() };
            unsafe {
                device.CreateVertexShader(&vs, None, Some(&mut corners))?;
                device.CreatePixelShader(&ps, None, Some(&mut pixels))?;
                device.CreateBuffer(&room, None, Some(&mut buffer))?;
            }
            let tables = [
                table(&device, boot.mark.w, boot.mark.h, DXGI_FORMAT_R32_FLOAT, &boot.mark.far)?,
                table(&device, boot.word.w, boot.word.h, DXGI_FORMAT_R32_FLOAT, &boot.word.far)?,
                table(&device, lettered.w, lettered.h, DXGI_FORMAT_R8_UNORM, &lettered.cover)?,
                table(&device, small.w, small.h, DXGI_FORMAT_R8_UNORM, &small.cover)?,
            ];
            Ok((Gpu { boot, device, context, corners: corners.ok_or_else(Error::empty)?, pixels: pixels.ok_or_else(Error::empty)?, moment: buffer.ok_or_else(Error::empty)?, tables, own: None, short: false }, took))
        }

        /// Draws the short one from now on, or the long one again.
        pub fn set_short(&mut self, short: bool) {
            self.short = short;
        }

        /// The card it draws with: for whoever shows it to make somewhere to draw to.
        pub fn device(&self) -> &ID3D11Device {
            &self.device
        }

        /// The frame at `t` seconds, onto `to`, which is `w` by `h` pixels: all of it, if that's anything from 4:3 to 16:9
        /// (there's more to each side, the wider it is); as big as will fit in the middle of it, otherwise, black round it.
        /// As sharp as that is, either way.
        pub fn draw(&self, t: f32, to: &ID3D11RenderTargetView, w: u32, h: u32) {
            // The short one is the end of the long one, coming up out of the dark over its first third of a second.
            let (t, comes) = if self.short { (HIT + t, (t / 0.35).clamp(0.0, 1.0)) } else { (t, 1.0) };
            let mut moment = moment(&self.boot, t);
            if self.short {
                (moment.clock[2], moment.clock[3]) = (moment.clock[2] * comes, 1.0);
            }
            let (w, h) = (w as f32, h as f32);
            // It fills anything from 4:3 to 16:9: laid out for 4:3, with more of the same to each side on a wider screen.
            let shape = (w / h).clamp(4.0 / 3.0, 16.0 / 9.0);
            let tall = h.min(w / shape);
            let wide = tall * shape;
            moment.cam3[2] = H as f32 * shape / 2.0;
            let view = D3D11_VIEWPORT { TopLeftX: ((w - wide) / 2.0).floor(), TopLeftY: ((h - tall) / 2.0).floor(), Width: wide, Height: tall, MinDepth: 0.0, MaxDepth: 1.0 };
            unsafe {
                let c = &self.context;
                c.UpdateSubresource(&self.moment, 0, None, (&raw const moment).cast(), 0, 0);
                c.ClearRenderTargetView(to, &[0.0, 0.0, 0.0, 1.0]);
                c.OMSetRenderTargets(Some(&[Some(to.clone())]), None);
                c.RSSetViewports(Some(&[view]));
                c.IASetInputLayout(None);
                c.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                c.VSSetShader(&self.corners, None);
                c.PSSetShader(&self.pixels, None);
                c.VSSetConstantBuffers(0, Some(&[Some(self.moment.clone())]));
                c.PSSetConstantBuffers(0, Some(&[Some(self.moment.clone())]));
                c.PSSetShaderResources(0, Some(&self.tables));
                c.Draw(3, 0);
                // Let go of it: whoever owns it may want to change its size.
                c.OMSetRenderTargets(None, None);
            }
        }

        /// The frame at `t` seconds, into `out`: `W` by `H` pixels, 0x00RRGGBB, drawn by the card and read back.
        pub fn frame(&mut self, t: f32, out: &mut [u32]) -> Result<()> {
            self.picture(t, (W, H), out)
        }

        /// The frame at `t` seconds, into `out`, `w` by `h` pixels of it (0x00RRGGBB): drawn by the card at that size, as
        /// `draw` draws, and read back. For whoever shows it has pixels to put somewhere, not a card to draw to.
        pub fn picture(&mut self, t: f32, (w, h): (usize, usize), out: &mut [u32]) -> Result<()> {
            assert_eq!(out.len(), w * h);
            if self.own.as_ref().is_none_or(|(size, ..)| *size != (w, h)) {
                let picture = texture(&self.device, (w, h), DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET.0 as u32, 0, None)?;
                let read = texture(&self.device, (w, h), DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ.0 as u32, None)?;
                let mut view = None;
                unsafe { self.device.CreateRenderTargetView(&picture, None, Some(&mut view))? };
                self.own = Some(((w, h), picture, view.ok_or_else(Error::empty)?, read));
            }
            let Some((_, picture, view, read)) = &self.own else { unreachable!() };
            self.draw(t, view, w as u32, h as u32);
            unsafe {
                self.context.CopyResource(read, picture);
                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                self.context.Map(read, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                for (y, row) in out.chunks_mut(w).enumerate() {
                    let from = std::slice::from_raw_parts(mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize).cast::<u32>(), w);
                    for (to, pixel) in row.iter_mut().zip(from) {
                        *to = pixel & 0x00FF_FFFF;
                    }
                }
                self.context.Unmap(read, 0);
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn lit(frame: &[u32]) -> usize {
            frame.iter().filter(|p| **p != 0).count()
        }

        fn card() -> Gpu {
            Gpu::new(Boot::new()).expect("a graphics card, or Windows' stand-in for one")
        }

        #[test]
        fn a_shader_kept_is_the_shader_compiled() {
            let kept = std::env::temp_dir().join(format!("pimpbox-boot-test-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&kept);
            assert!(!is_kept(&Boot::new(), &kept));
            let (mut first, took) = Gpu::cached(Boot::new(), &kept).expect("a graphics card");
            assert!(took.is_some() && is_kept(&Boot::new(), &kept), "compiled, and kept");
            let (mut again, took) = Gpu::cached(Boot::new(), &kept).expect("a graphics card");
            assert!(took.is_none(), "taken from where it's kept, the second time");
            let (mut a, mut b) = (vec![0; W * H], vec![0; W * H]);
            first.frame(BALL + 1.1, &mut a).expect("a frame");
            again.warm(&[(W, H)]).expect("every part of it");
            again.frame(BALL + 1.1, &mut b).expect("a frame");
            assert_eq!(a, b);
            let _ = std::fs::remove_dir_all(&kept);
        }

        #[test]
        fn it_starts_and_ends_in_the_dark_and_shows_something_between() {
            let mut gpu = card();
            let mut px = vec![0; W * H];
            gpu.frame(0.0, &mut px).expect("a frame");
            assert_eq!(lit(&px), 0, "nothing yet");
            gpu.frame(LENGTH, &mut px).expect("a frame");
            assert_eq!(lit(&px), 0, "and nothing after");
            for t in [0.3, 1.5, BALL + 1.0, BALL + 2.0, LAUNCH + 0.3, NAME + 0.01, NAME + 2.0] {
                gpu.frame(t, &mut px).expect("a frame");
                assert!(lit(&px) > 500, "something to see at {t}");
            }
            assert!(Boot::over(LENGTH) && !Boot::over(LENGTH - 0.1));
        }

        #[test]
        fn the_same_moment_is_the_same_picture() {
            let mut gpu = card();
            let (mut a, mut b) = (vec![0; W * H], vec![0; W * H]);
            gpu.frame(BALL + 1.1, &mut a).expect("a frame");
            gpu.frame(BALL + 1.1, &mut b).expect("a frame");
            assert_eq!(a, b);
        }

        #[test]
        fn the_name_rains_one_way_and_then_inside_out() {
            let mut gpu = card();
            let (mut a, mut b) = (vec![0; W * H], vec![0; W * H]);
            gpu.frame(RAIN + 0.001, &mut a).expect("a frame");
            gpu.frame(FLIP + 0.001, &mut b).expect("a frame");
            let (green_on_black, black_on_green) = (lit(&a), lit(&b));
            assert!(green_on_black > 5_000 && green_on_black < W * H / 4, "{green_on_black}");
            assert!(black_on_green > W * H * 3 / 4, "{black_on_green}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_knows_how_far_off_it_is() {
        // In cells, `FINE` to the dot, with `ROOM` dots round it.
        let mark = Mark::new();
        let far = |x: f32, y: f32| mark.far[((y + ROOM as f32) * FINE as f32) as usize * mark.w + ((x + ROOM as f32) * FINE as f32) as usize];
        assert!(far(5.5, 1.5) < 0.3, "on its top stroke");
        assert!(far(-3.0, 6.0) > 2.0, "well to its left");
    }

    #[test]
    fn the_ball_comes_down_where_the_rings_go_out_from() {
        // On the floor at each landing, and off it between.
        for (i, at) in LANDS.iter().enumerate() {
            let c = cam(*at - 0.0001);
            let b = ball(*at - 0.0001, &c).unwrap();
            assert!((b.high - BALL_R).abs() < 0.01, "down at landing {i}");
            assert!((b.at.0 - landing(i).0).abs() < 0.01 && (b.at.1 - landing(i).1).abs() < 0.01);
        }
        let mid = (LANDS[0] + LANDS[1]) / 2.0;
        assert!(ball(mid, &cam(mid)).unwrap().high > 1.2, "up between them");
        // Crouched, it's lower than it sits; and by the time it would hit, it fills the picture, all gold.
        let crouched = ball(LAUNCH - 0.01, &cam(LAUNCH - 0.01)).unwrap();
        assert!(crouched.tall < 0.7 && crouched.wide > 1.2);
        let here = ball(HIT - 0.001, &cam(HIT - 0.001)).unwrap();
        assert!(here.r > W as f32 && here.melt > 0.99 && (here.x - CX).abs() < 2.0 && (here.y - CY).abs() < 2.0);
        assert!(ball(HIT, &cam(HIT)).is_none());
    }

    #[test]
    fn the_one_that_bounces_in_is_still_going_where_the_ball_begins() {
        // At the top of a bounce, in the middle of the picture, as the ball takes its place: same place, same size, same pace.
        let (b, begins) = (thrown(BALL - 0.0005), ball(BALL, &cam(BALL)).unwrap());
        assert!((b.x - begins.x).abs() < 1.0 && (b.y - begins.y).abs() < 1.5, "{},{} and {},{}", b.x, b.y, begins.x, begins.y);
        assert!((b.wide - 1.0).abs() < 0.05 && (b.tall - 1.0).abs() < 0.05, "{} by {}", b.wide, b.tall);
        let was = (thrown(BALL).x - thrown(BALL - 0.05).x) / 0.05;
        let is = (ball(BALL + 0.05, &cam(BALL + 0.05)).unwrap().x - begins.x) / 0.05;
        assert!(was > 100.0 && (is - was).abs() < 0.15 * was, "{was} a second, and then {is}");
        assert!(thrown(GOO).x < 0.0, "it starts off the left of the picture");
        // Each bounce is lower than the one before.
        let top = |from: f32, to: f32| (0..200).map(|i| thrown(mix(from, to, i as f32 / 200.0)).y).fold(f32::MAX, f32::min);
        assert!(top(GOO + 0.34, GOO + 0.85) < top(GOO + 0.86, GOO + 1.25) - 20.0);
    }

    #[test]
    fn what_is_in_the_hall_is_seen_where_the_hall_is() {
        // The ball's own place in the hall is where the ball's drawn, however far round the camera's swung.
        for at in [BALL + 0.2, BALL + 1.9, BALL + 2.7, BALL + 3.5] {
            let c = cam(at);
            let b = ball(at, &c).unwrap();
            let (x, _, away) = c.see(b.at.0, 1.0, b.at.1).unwrap();
            assert!((x - b.x).abs() < 0.5 && (away - AHEAD).abs() < 0.01, "at {at}");
        }
        // And nothing thrown goes through the floor, or goes on for ever.
        for i in 0..400 {
            let (gone, high, _) = tossed(i as f32 * 0.02, 1.5, 1.2);
            assert!(high >= LIES - 0.001 && gone < 4.0, "{high} up, {gone} along");
        }
        assert!(tossed(6.0, 1.5, 1.2).1 == LIES, "it lies still in the end");
    }
}
