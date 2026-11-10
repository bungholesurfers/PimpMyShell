//! The boot animation to look at without a console.
//!
//!   frames [--size WxH] [--short] <folder> [time..]   a BMP for each time in seconds (a dozen across the whole of it, if none are given)
//!   frames [--size WxH] --video <file>      all of it at its own pace, as a video, through ffmpeg (which has to be on the PATH)
//!
//! 640x480 unless it's told another size: 1280x720, say, for how it looks on a wide screen. The video says how long a frame
//! takes to draw (which includes reading it back from the graphics card, which showing it wouldn't need).

use std::io::Write;

use pimpbox_boot::gpu::Gpu;
use pimpbox_boot::{FPS, H, LENGTH, W};

fn bmp(px: &[u32], (w, h): (usize, usize)) -> Vec<u8> {
    let size = 54 + w * h * 4;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    // Top row first.
    out.extend_from_slice(&(-(h as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    out.extend(px.iter().flat_map(|p| p.to_le_bytes()));
    out
}

fn video(mut gpu: Gpu, (w, h): (usize, usize), file: &str) {
    let mut ffmpeg = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "bgr0", "-s", &format!("{w}x{h}"), "-r", &FPS.to_string(), "-i", "-"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "17", file])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("ffmpeg, on the PATH");
    let mut to = ffmpeg.stdin.take().expect("its input");
    let mut px = vec![0; w * h];
    let frames = (LENGTH * FPS as f32).ceil() as u32 + FPS / 2;
    let mut drawing = std::time::Duration::ZERO;
    let mut slowest = std::time::Duration::ZERO;
    for i in 0..frames {
        let began = std::time::Instant::now();
        gpu.picture(i as f32 / FPS as f32, (w, h), &mut px).expect("a frame from the card");
        let took = began.elapsed();
        (drawing, slowest) = (drawing + took, slowest.max(took));
        let bytes: Vec<u8> = px.iter().flat_map(|p| p.to_le_bytes()).collect();
        to.write_all(&bytes).expect("ffmpeg taking a frame");
    }
    drop(to);
    ffmpeg.wait().expect("ffmpeg finishing");
    println!("{frames} frames: {:.1} ms each to draw, {:.1} ms the slowest ({:.1} ms is one at {FPS} a second)", drawing.as_secs_f64() * 1000.0 / f64::from(frames), slowest.as_secs_f64() * 1000.0, 1000.0 / f64::from(FPS));
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut size = (W, H);
    if args.first().is_some_and(|a| a == "--size") && args.len() >= 2 {
        size = args[1].split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))).expect("a size, as 1280x720");
        args.drain(..2);
    }
    let mut gpu = Gpu::new(pimpbox_boot::fonts::boot()).expect("a graphics card");
    if args.first().is_some_and(|a| a == "--short") {
        args.remove(0);
        gpu.set_short(true);
    }
    match args.first().map(String::as_str) {
        Some("--video") if args.len() == 2 => return video(gpu, size, &args[1]),
        Some(_) => {}
        None => {
            eprintln!("frames [--size WxH] <folder> [time..] | frames [--size WxH] --video <file>");
            std::process::exit(2);
        }
    }
    let folder = &args[0];
    let times: Vec<f32> = if args.len() > 1 { args[1..].iter().filter_map(|a| a.parse().ok()).collect() } else { (0..12).map(|i| (i as f32 + 0.5) * LENGTH / 12.0).collect() };
    let mut px = vec![0; size.0 * size.1];
    std::fs::create_dir_all(folder).expect("the folder");
    for (i, t) in times.iter().enumerate() {
        gpu.picture(*t, size, &mut px).expect("a frame from the card");
        std::fs::write(format!("{folder}/{i:02}-{t:.2}.bmp"), bmp(&px, size)).expect("a frame");
    }
    println!("{} frames", times.len());
}
