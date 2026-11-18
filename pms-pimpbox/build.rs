//! The console's icon and version information. The icon is the boot animation's mark in the colors of its last look, "mark"
//! (boot/src/icon.rs and looks.txt, which the console draws its own icons with too, as it runs, and pms-art sets): drawn here
//! at each size Windows asks an icon for, and put into pms-pimpbox.exe as PimpMyShell's own programs have theirs put in
//! (pms-res). Nothing of it is a picture file.

use std::path::PathBuf;

#[path = "boot/src/icon.rs"]
#[allow(dead_code)]
mod icon;

/// The sizes, in pixels, that Windows picks among: the title bar, the taskbar, Explorer's views, at each scale.
const SIZES: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 128, 256];

fn main() {
    println!("cargo:rerun-if-changed=boot/src/dollar.txt");
    println!("cargo:rerun-if-changed=boot/src/looks.txt");
    println!("cargo:rerun-if-changed=boot/src/icon.rs");
    println!("cargo:rerun-if-changed=build.rs");
    let ico = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("pimpbox.ico");
    std::fs::write(&ico, ico_file()).expect("write the icon");
    pms_res::embed(&ico.to_string_lossy(), &[(pms_res::Target::Bin("pms-pimpbox"), "PIMPBOX")]);
}

/// The whole .ico: a directory of its images, then each image, 32-bit with alpha, as a bitmap.
fn ico_file() -> Vec<u8> {
    let mark = icon::colors(icon::Look::Mark);
    let shape = icon::shape();
    let images: Vec<(u32, Vec<u8>)> = SIZES.iter().map(|&s| (s, bitmap(s, &icon::pixels(s, mark, shape)))).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut at = 6 + 16 * images.len() as u32;
    for (size, data) in &images {
        let side = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&at.to_le_bytes());
        at += data.len() as u32;
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }
    out
}

/// An icon image as Windows takes one in a .ico: a BITMAPINFOHEADER (its height twice the image's, for the mask), the pixels as
/// BGRA from the bottom row up, then the mask (all clear: the alpha says what's there).
fn bitmap(size: u32, pixels: &[[u8; 4]]) -> Vec<u8> {
    let mut out = Vec::new();
    for v in [40u32, size, size * 2] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    for v in [0u32, size * size * 4, 0, 0, 0, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for y in (0..size).rev() {
        for x in 0..size {
            let [r, g, b, a] = pixels[(y * size + x) as usize];
            out.extend_from_slice(&[b, g, r, a]);
        }
    }
    let mask_row = size.div_ceil(32) * 4;
    out.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize));
    out
}
