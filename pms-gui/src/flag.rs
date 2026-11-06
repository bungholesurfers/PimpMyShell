//! The flag bill: a dollar bill with a digital trail of blocks at its left, bent like a flag in the wind. Laid out flat
//! here, and bent by `bend`, which is all that the settings' animation (dollars.rs) and pms-art's still pictures (which
//! take this file by `#[path]`) share, so that they can't come to disagree. All in DIPs, or the units of the 288 by 184 tile.

use std::f32::consts::TAU;

/// The flat picture: the trail, then the bill at its right.
pub const FLAT_W: f32 = 132.0;
pub const FLAT_H: f32 = 48.0;
pub const BILL_X: f32 = 56.0;
pub const BILL_Y: f32 = 5.0;
pub const BILL_W: f32 = 72.0;
pub const BILL_H: f32 = 38.0;
/// How far the bending can take it above or below the flat picture.
#[allow(dead_code)]
pub const PAD: f32 = 14.0;
/// How opaque the bill's paper is, under its lines.
pub const PAPER: f32 = 0.3;

/// Where each flag's flat picture goes in the tile, its middle's height, and how far its wave is ahead of the other's.
pub const FLAGS: [(f32, f32, f32); 2] = [(8.0, 48.0, 0.0), (152.0, 140.0, 2.4)];
/// Where the slogan goes between them.
pub const SLOGANS: [(f32, f32); 2] = [(215.0, 48.0), (76.0, 140.0)];

/// One wave's time, in seconds.
#[allow(dead_code)]
pub const PERIOD: f32 = 2.8;
/// The wave's phase in a still picture.
pub const REST: f32 = 1.1;
const CYCLES: f32 = 1.15;
const LIFT: f32 = 5.0;
const SLANT: f32 = 10.0;

/// The trail's blocks, as x, y, width, height and opacity: three rows of five, smaller and fainter the further they are
/// from the bill.
pub fn blocks() -> Vec<(f32, f32, f32, f32, f32)> {
    let columns = [(42.0, 10.0, 8.0, 0.9), (30.0, 8.0, 6.5, 0.75), (19.5, 6.5, 5.0, 0.6), (10.5, 5.0, 4.0, 0.45), (3.0, 3.5, 3.0, 0.3)];
    let mut all = Vec::new();
    for (x, w, h, opacity) in columns {
        for row in [-1.0, 0.0, 1.0] {
            all.push((x, FLAT_H / 2.0 + row * 12.0 - h / 2.0, w, h, opacity));
        }
    }
    all
}

/// What the wave does at a place across the flat picture.
pub struct Bend {
    /// How far down it moves (up, if negative).
    pub lift: f32,
    /// How much taller it is, as the sheet swings toward the eye.
    pub stretch: f32,
    /// How much of its light it keeps, as the sheet turns from it.
    pub light: f32,
}

/// The bend `u` of the way across the flat picture, at `phase` (radians). It rises to the right, and swings most at the
/// trail's free end.
pub fn bend(u: f32, phase: f32) -> Bend {
    let theta = TAU * CYCLES * u - phase;
    Bend { lift: -SLANT * (u - 0.5) + LIFT * (0.55 + 0.45 * (1.0 - u)) * theta.sin(), stretch: 1.0 + 0.08 * theta.cos(), light: 0.84 + 0.16 * (theta + 0.7).cos() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bend_stays_inside_what_room_it_has() {
        for step in 0..=200 {
            for turn in 0..32 {
                let b = bend(step as f32 / 200.0, turn as f32 * TAU / 32.0);
                assert!(b.lift.abs() + (b.stretch - 1.0).abs() * FLAT_H / 2.0 <= PAD);
                assert!((0.0..=1.0).contains(&b.light));
            }
        }
    }

    #[test]
    fn everything_fits_in_its_tile() {
        for (x, y, _) in FLAGS {
            assert!(x >= 0.0 && x + FLAT_W <= 288.0);
            assert!(y - FLAT_H / 2.0 - PAD >= 0.0 && y + FLAT_H / 2.0 + PAD <= 184.0);
        }
        assert!(blocks().iter().all(|b| b.0 >= 0.0 && b.0 + b.2 <= BILL_X && b.1 >= 0.0 && b.1 + b.3 <= FLAT_H));
    }
}
