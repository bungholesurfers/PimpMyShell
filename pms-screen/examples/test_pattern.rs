//! A moving test pattern on the screen `doom`, for trying a `screen` meter without any game:
//! `cargo run --release -p pms-screen --example test_pattern`.

use std::time::{Duration, Instant};

fn main() {
    let screen = pms_screen::Screen::create("doom").expect("screen");
    let began = Instant::now();
    let (mut frames, mut said) = (0u32, Instant::now());
    loop {
        let next = Instant::now() + Duration::from_millis(1000 / 35);
        let (w, h) = screen.wanted().unwrap_or((64, 48));
        let t = began.elapsed().as_secs_f32();
        let (bx, by) = (((t * 1.3).sin() * 0.5 + 0.5) * (w - 12) as f32, ((t * 1.9).cos() * 0.5 + 0.5) * (h - 12) as f32);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let at = ((y * w + x) * 4) as usize;
                let inside = (x as f32) >= bx && (x as f32) < bx + 12.0 && (y as f32) >= by && (y as f32) < by + 12.0;
                let wave = (((x as f32 / 6.0 + t * 3.0).sin() + (y as f32 / 5.0 - t * 2.0).cos()) * 60.0 + 120.0) as u8;
                px[at..at + 4].copy_from_slice(&if inside { [255, 255, 255, 255] } else { [wave / 3, wave, wave / 2, 255] });
            }
        }
        screen.publish(w, h, &px);
        frames += 1;
        if said.elapsed() >= Duration::from_secs(2) {
            println!("{w}x{h}, {:.1} frames a second", frames as f32 / said.elapsed().as_secs_f32());
            (frames, said) = (0, Instant::now());
        }
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
}
