//! What the console shows while it's getting something ready that takes a moment (its boot animation's shader being
//! compiled, the first time): a small wheel of eight green blocks low on a dark screen, a light going round them. Drawn
//! here, in plain arithmetic, as it's what's shown while the graphics card isn't ready to draw anything.

/// The wheel, on a dark picture `w` by `h` pixels, made in `out`, as it is `t` seconds after it began to turn.
pub fn draw(out: &mut Vec<u32>, (w, h): (usize, usize), t: f32) {
    out.clear();
    out.resize(w * h, 0);
    let (cx, cy) = (w as f32 / 2.0, h as f32 * 0.84);
    let (outer, inner) = (h as f32 * 0.058, h as f32 * 0.022);
    // Where the light's got to: once round every six tenths of a second, a block at a time.
    let lead = (t * 8.0 / 0.6).floor();
    let reach = outer.ceil() as usize + 1;
    for y in (cy as usize).saturating_sub(reach)..(cy as usize + reach).min(h) {
        for x in (cx as usize).saturating_sub(reach)..(cx as usize + reach).min(w) {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let far = dx.hypot(dy);
            // In the ring, soft for a pixel at each edge of it.
            let ring = (outer - far).clamp(0.0, 1.0) * (far - inner).clamp(0.0, 1.0);
            if ring <= 0.0 {
                continue;
            }
            // Which block, clockwise from the top; and not in the gap between two of them.
            let round = (dx.atan2(-dy) / std::f32::consts::TAU).rem_euclid(1.0) * 8.0;
            let (block, within) = (round.floor(), round.fract());
            let gap = 0.09 * outer / far.max(1.0);
            if within < gap || within > 1.0 - gap {
                continue;
            }
            // Brightest where the light is, dimmer the longer ago it was there.
            let ago = (lead - block).rem_euclid(8.0) / 8.0;
            let bright = (0.16 + 0.84 * (1.0 - ago) * (1.0 - ago)) * ring;
            let (r, g, b) = ((40.0 * bright) as u32, (245.0 * bright) as u32, (50.0 * bright) as u32);
            out[y * w + x] = r << 16 | g << 8 | b;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_turns() {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        super::draw(&mut a, (640, 480), 0.0);
        super::draw(&mut b, (640, 480), 0.3);
        // Low in the middle, green, with nothing elsewhere; and not the same picture a moment later.
        assert!(a[..640 * 300].iter().all(|p| *p == 0), "nothing up top");
        let lit = a.iter().filter(|p| **p != 0).count();
        assert!((1500..6000).contains(&lit), "{lit}");
        assert!(a.iter().all(|p| (p >> 8) & 0xFF >= (p >> 16) & 0xFF), "green");
        assert_ne!(a, b);
    }
}
