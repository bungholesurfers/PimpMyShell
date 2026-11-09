//! The Twatch TV logo: the name across a tilted oval. It's here because both sides draw it, a set when it starts up and its remote.

pub const LOGO_BLUE: [u8; 3] = [8, 78, 162];
pub const LOGO_NAME: [u8; 3] = [255, 255, 255];

/// The name, stroke by stroke, each from one corner of its letter to another: wide, and an A with no bar.
const NAME: [&[[f32; 4]]; 6] = [
    &[[0.0, 0.0, 1.0, 0.0], [0.5, 0.0, 0.5, 1.0]],
    &[[0.0, 0.0, 0.25, 1.0], [0.25, 1.0, 0.5, 0.2], [0.5, 0.2, 0.75, 1.0], [0.75, 1.0, 1.0, 0.0]],
    &[[0.0, 1.0, 0.5, 0.0], [0.5, 0.0, 1.0, 1.0]],
    &[[0.0, 0.0, 1.0, 0.0], [0.5, 0.0, 0.5, 1.0]],
    &[[1.0, 0.0, 0.2, 0.0], [0.2, 0.0, 0.0, 0.2], [0.0, 0.2, 0.0, 0.8], [0.0, 0.8, 0.2, 1.0], [0.2, 1.0, 1.0, 1.0]],
    &[[0.0, 0.0, 0.0, 1.0], [1.0, 0.0, 1.0, 1.0], [0.0, 0.5, 1.0, 0.5]],
];

/// Gives every pixel of a `w` by `h` logo on `background`, at `bright` of its full brightness.
pub fn logo(w: usize, h: usize, bright: f32, background: [u8; 3], mut set: impl FnMut(usize, usize, [u8; 3])) {
    const LOOKS: usize = 4;
    /// How wide the oval is, of the picture; how tall, of its width; and how far it leans, in radians: down to the right.
    const OVAL: (f32, f32, f32) = (0.47, 0.34, 0.13);
    /// How wide the name is, of the oval, and how tall, of its half height, and how thick its strokes are, of that.
    const NAME_SIZE: (f32, f32, f32) = (0.82, 0.84, 0.115);
    /// How much of its room a letter fills.
    const LETTER: f32 = 0.72;
    let (fw, fh) = (w as f32, h as f32);
    let (half_w, tilt) = (fw * OVAL.0, OVAL.2);
    let half_h = half_w * OVAL.1;
    let (sin, cos) = tilt.sin_cos();
    let (cx, cy) = (fw / 2.0, fh / 2.0);
    let (room, tall) = (2.0 * half_w * NAME_SIZE.0 / NAME.len() as f32, half_h * NAME_SIZE.1);
    let stroke = tall * NAME_SIZE.2;
    let (left_edge, top) = (cx - room * NAME.len() as f32 / 2.0, cy - tall / 2.0);
    let strokes: Vec<[f32; 4]> = NAME
        .iter()
        .enumerate()
        .flat_map(|(i, parts)| {
            let x = left_edge + room * (i as f32 + (1.0 - LETTER) / 2.0);
            parts.iter().map(move |[a, b, d, e]| [x + a * room * LETTER, top + b * tall, x + d * room * LETTER, top + e * tall])
        })
        .collect();
    let in_name = |x: f32, y: f32| {
        y > top - stroke
            && y < top + tall + stroke
            && strokes.iter().any(|[a, b, d, e]| {
                let (dx, dy) = (d - a, e - b);
                let k = (((x - a) * dx + (y - b) * dy) / (dx * dx + dy * dy).max(0.001)).clamp(0.0, 1.0);
                (x - a - k * dx).hypot(y - b - k * dy) < stroke
            })
    };
    let in_oval = |x: f32, y: f32| {
        let (dx, dy) = (cx - x, y - cy);
        let (along, across) = ((dx * cos - dy * sin) / half_w, (dx * sin + dy * cos) / half_h);
        along * along + across * across < 1.0
    };
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.0f32; 3];
            for dy in 0..LOOKS {
                for dx in 0..LOOKS {
                    let at = |p: usize, d: usize| p as f32 + (d as f32 + 0.5) / LOOKS as f32;
                    let (sx, sy) = (at(x, dx), at(y, dy));
                    let color = if in_name(sx, sy) {
                        LOGO_NAME
                    } else if in_oval(sx, sy) {
                        LOGO_BLUE
                    } else {
                        background
                    };
                    for (s, v) in sum.iter_mut().zip(color) {
                        *s += f32::from(v);
                    }
                }
            }
            set(x, y, sum.map(|v| (v / (LOOKS * LOOKS) as f32 * bright) as u8));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_oval_leans_down_to_the_right() {
        let (w, h) = (200, 100);
        let mut px = vec![[0u8; 3]; w * h];
        logo(w, h, 1.0, [0, 0, 0], |x, y, c| px[y * w + x] = c);
        let blue_at = |x: usize, y: usize| px[y * w + x] == LOGO_BLUE;
        // The far ends of the oval, away from the name: which one sits lower says which way it leans.
        let lowest = |x: usize| (0..h).rev().find(|y| blue_at(x, *y));
        let (left, right) = (lowest(14).unwrap_or(0), lowest(w - 15).unwrap_or(0));
        assert!(right > left, "left end reaches {left}, right end {right}");
    }
}
