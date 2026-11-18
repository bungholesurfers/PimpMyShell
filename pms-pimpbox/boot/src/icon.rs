//! The console's icon: the mark (dollar.txt), in one color, on a rounded square of another, drawn at whatever size is asked for
//! from those dots. Each part of the boot animation has its own pair of colors (`Look`, and looks.txt beside this, which pms-art
//! writes): the console draws them as it runs, and its build draws the program file's icon ("mark"). Arithmetic only, so that
//! it's all one drawing. On its own, with nothing of the rest of this crate, so that whatever draws it can take it as it is.

/// The mark's dots, a row a line, `#` for a dot.
pub const DOTS: &str = include_str!("dollar.txt");
/// The colors of each look, a line each: its name, the mark's color, the square's.
pub const LOOKS: &str = include_str!("looks.txt");

/// The looks the icon takes, after the animation's own colors: as it warms up and its name rains down; for each flash of that
/// rain; from when the ball's a mirror ball in its hall; and from the mark on, as the console's program file has it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    Rain,
    Flash,
    Hall,
    Mark,
}

impl Look {
    pub const ALL: [Look; 4] = [Look::Rain, Look::Flash, Look::Hall, Look::Mark];

    /// Its name in looks.txt.
    pub fn name(self) -> &'static str {
        match self {
            Look::Rain => "rain",
            Look::Flash => "flash",
            Look::Hall => "hall",
            Look::Mark => "mark",
        }
    }
}

/// The colors: the mark's (ink), and the square's (paper).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Colors {
    pub ink: [u8; 3],
    pub paper: [u8; 3],
}

/// `#RRGGBB` as its three.
pub fn rgb(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.trim().strip_prefix('#')?;
    let n = u32::from_str_radix(hex, 16).ok().filter(|_| hex.len() == 6)?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// A look's colors, as `looks` (looks.txt's text) has them; where it hasn't them, black on orange.
pub fn colors_in(looks: &str, look: Look) -> Colors {
    looks
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .find_map(|l| {
            let mut words = l.split_whitespace();
            (words.next()? == look.name()).then_some(())?;
            Some(Colors { ink: rgb(words.next()?)?, paper: rgb(words.next()?)? })
        })
        .unwrap_or(Colors { ink: [0, 0, 0], paper: [0xFF, 0x9A, 0x22] })
}

/// A look's colors, as looks.txt has them.
pub fn colors(look: Look) -> Colors {
    colors_in(LOOKS, look)
}

/// How big the mark is: how much of the icon's height it spans (0.4 to 0.92, past which it would run off the rounded square),
/// and whether its dots are whole pixels (crisp: it can only be as big as a whole number of pixels to a dot makes it, so at the
/// smallest sizes it doesn't change) or not (smooth: exactly that big, its edges a little soft where a dot falls between pixels).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shape {
    pub scale: f32,
    pub crisp: bool,
}

impl Default for Shape {
    fn default() -> Shape {
        Shape { scale: 0.75, crisp: true }
    }
}

/// How small and how big the mark can be, of the icon's height: the square is inset 1/32 of it all round, so 0.92 keeps the
/// mark on it.
pub const MIN_SCALE: f32 = 0.4;
pub const MAX_SCALE: f32 = 0.92;

/// The mark's size, as `looks` (looks.txt's text) has it: `size 0.75 crisp` (or `smooth`); where it hasn't, the default.
pub fn shape_in(looks: &str) -> Shape {
    looks
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .find_map(|l| {
            let mut words = l.split_whitespace();
            (words.next()? == "size").then_some(())?;
            let scale: f32 = words.next()?.parse().ok()?;
            let crisp = words.next() != Some("smooth");
            Some(Shape { scale: scale.clamp(MIN_SCALE, MAX_SCALE), crisp })
        })
        .unwrap_or_default()
}

/// The mark's size, as looks.txt has it.
pub fn shape() -> Shape {
    shape_in(LOOKS)
}

/// The icon `size` pixels square, in `colors`, with the mark as big as `shape` says: RGBA, top row first. The square with its
/// corners rounded, and the mark's dots on it, in the middle. Each pixel's sampled 4 by 4, so that the round corners (and a
/// smooth mark's edges) are smooth.
pub fn pixels(size: u32, colors: Colors, shape: Shape) -> Vec<[u8; 4]> {
    let dots: Vec<Vec<bool>> = DOTS.lines().map(|l| l.bytes().map(|b| b == b'#').collect()).collect();
    let (cols, rows) = (dots[0].len(), dots.len());
    let s = size as f32;
    let (inset, radius) = (s / 32.0, s / 5.0);
    // How big a dot is: as the shape says, or as many whole pixels to it as fit in that (one at least).
    let wanted = shape.scale.clamp(MIN_SCALE, MAX_SCALE) * s / rows as f32;
    let dot = if shape.crisp { wanted.floor().max(1.0) } else { wanted };
    let (wide, tall) = (dot * cols as f32, dot * rows as f32);
    let (left, top) = if shape.crisp { (((s - wide) / 2.0).floor(), ((s - tall) / 2.0).floor()) } else { ((s - wide) / 2.0, (s - tall) / 2.0) };
    let in_square = |x: f32, y: f32| {
        // Outside the corners' circles, past the square's straight edges: none of it.
        let (lo, hi) = (inset + radius, s - inset - radius);
        let (dx, dy) = ((lo - x).max(x - hi).max(0.0), (lo - y).max(y - hi).max(0.0));
        x >= inset && x <= s - inset && y >= inset && y <= s - inset && dx.hypot(dy) <= radius
    };
    let on_mark = |x: f32, y: f32| {
        let (col, row) = ((x - left) / dot, (y - top) / dot);
        col >= 0.0 && row >= 0.0 && (col as usize) < cols && (row as usize) < rows && dots[row as usize][col as usize]
    };
    let mix = |paper: u8, ink: u8, k: f32| (f32::from(paper) + (f32::from(ink) - f32::from(paper)) * k).round() as u8;
    let mut out = Vec::with_capacity((size * size) as usize);
    for y in 0..size {
        for x in 0..size {
            let samples = (0..16).map(|i| (x as f32 + (i % 4) as f32 / 4.0 + 0.125, y as f32 + (i / 4) as f32 / 4.0 + 0.125));
            let (covered, inked) = samples.fold((0, 0), |(c, k), (sx, sy)| (c + usize::from(in_square(sx, sy)), k + usize::from(on_mark(sx, sy))));
            let alpha = (covered * 255 / 16) as u8;
            let k = inked as f32 / 16.0;
            let [r, g, b] = [0, 1, 2].map(|i| mix(colors.paper[i], colors.ink[i], k));
            out.push([r, g, b, alpha]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_look_has_its_colors_and_a_missing_one_is_black_on_orange() {
        for look in Look::ALL {
            assert!(LOOKS.lines().any(|l| l.split_whitespace().next() == Some(look.name())), "looks.txt has {}", look.name());
        }
        assert_eq!(colors_in("hall #B073FF #000000\n", Look::Hall), Colors { ink: [0xB0, 0x73, 0xFF], paper: [0, 0, 0] });
        assert_eq!(colors_in("# hall #FFFFFF #FFFFFF\n", Look::Hall).paper, [0xFF, 0x9A, 0x22], "a comment isn't a look");
        assert_eq!(colors_in("hall #B073FF nope\n", Look::Hall).paper, [0xFF, 0x9A, 0x22]);
        assert_eq!(rgb("#9eff99"), Some([0x9E, 0xFF, 0x99]));
        assert_eq!(rgb("#9EFF9"), None);
    }

    #[test]
    fn the_mark_is_in_ink_and_the_corners_are_clear() {
        let c = Colors { ink: [1, 2, 3], paper: [9, 9, 9] };
        let px = pixels(32, c, Shape::default());
        assert_eq!(px[0][3], 0, "the corner's rounded off");
        assert!(px.iter().any(|p| p[..3] == [1, 2, 3] && p[3] == 255) && px.iter().any(|p| p[..3] == [9, 9, 9] && p[3] == 255));
    }

    #[test]
    fn a_bigger_mark_has_more_ink_and_a_crisp_one_has_no_half_dots() {
        let c = Colors { ink: [0, 0, 0], paper: [255, 255, 255] };
        let ink = |px: &[[u8; 4]]| px.iter().filter(|p| p[0] < 128).count();
        let (small, big) = (pixels(64, c, Shape { scale: 0.5, crisp: true }), pixels(64, c, Shape { scale: MAX_SCALE, crisp: true }));
        assert!(ink(&big) > ink(&small) * 2);
        assert!(big.iter().all(|p| p[0] == 0 || p[0] == 255), "crisp: every pixel is all ink or none");
        let smooth = pixels(64, c, Shape { scale: 0.63, crisp: false });
        assert!(smooth.iter().any(|p| p[0] > 0 && p[0] < 255), "smooth: some pixels are part ink");
    }

    #[test]
    fn the_size_is_read_from_looks() {
        assert_eq!(shape_in("size 0.9 smooth\n"), Shape { scale: 0.9, crisp: false });
        assert_eq!(shape_in("size 0.9\n"), Shape { scale: 0.9, crisp: true });
        assert_eq!(shape_in("size 3 crisp\n").scale, MAX_SCALE, "no bigger than the square");
        assert_eq!(shape_in("rain #000000 #FFFFFF\n"), Shape::default());
        assert_eq!(shape(), shape_in(LOOKS));
    }

    #[test]
    fn the_default_size_draws_what_it_always_did() {
        // As it was before the size could change: (size - 2 * (size / 8)) / 13 whole pixels to a dot, in the middle.
        for size in [16u32, 24, 32, 48, 64, 256] {
            let pad = (size / 8).max(1);
            let dot = ((size - 2 * pad) / 13).max(1);
            let wanted = (0.75 * size as f32 / 13.0).floor().max(1.0) as u32;
            assert_eq!(dot, wanted, "{size}");
        }
    }
}
