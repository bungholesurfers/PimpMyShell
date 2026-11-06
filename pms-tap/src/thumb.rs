//! Toast pictures, scaled on the CPU to the very pixels they'll cover. A surface brush samples its image bilinearly
//! from the full-size texture, which jags and shimmers when it shrinks a big one (an app's 4x logo into 32 DIPs, a
//! photo into a thumbnail); WIC's high-quality cubic filter widens with the shrink, so every source pixel counts, and
//! it runs in premultiplied alpha (no dark fringes round transparent edges). Callers run this off the UI thread.

use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows_core::HSTRING;

use crate::commands::R;

/// How a picture meets its frame.
#[derive(Clone, Copy)]
pub(crate) enum Fit {
    /// All of it, inside the frame, in its own shape.
    Contain,
    /// Its middle, in the frame's shape, filling it.
    Cover,
}

/// The image file at `path`, scaled to a `w`×`h`-pixel frame, as a PNG.
pub(crate) fn png(path: &str, w: u32, h: u32, fit: Fit) -> R<Vec<u8>> {
    // A thread of our own: WIC wants COM, and the multithreaded apartment suits it.
    let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let out = scale(path, w.max(1), h.max(1), fit);
    if init.is_ok() {
        unsafe { CoUninitialize() };
    }
    out
}

fn scale(path: &str, w: u32, h: u32, fit: Fit) -> R<Vec<u8>> {
    let e = |e: windows_core::Error| format!("{path}: {e}");
    let wic: IWICImagingFactory = unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }.map_err(e)?;
    let decoder =
        unsafe { wic.CreateDecoderFromFilename(&HSTRING::from(path), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand) }.map_err(e)?;
    let frame = unsafe { decoder.GetFrame(0) }.map_err(e)?;
    let pbgra = unsafe { wic.CreateFormatConverter() }.map_err(e)?;
    unsafe {
        pbgra.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)
    }
    .map_err(e)?;
    let (mut sw, mut sh) = (0, 0);
    unsafe { pbgra.GetSize(&mut sw, &mut sh) }.map_err(e)?;
    if sw == 0 || sh == 0 {
        return Err(format!("{path}: empty image"));
    }
    let (rect, ow, oh) = crop(sw, sh, w, h, fit);
    let clipper = unsafe { wic.CreateBitmapClipper() }.map_err(e)?;
    unsafe { clipper.Initialize(&pbgra, &rect) }.map_err(e)?;
    let scaler = unsafe { wic.CreateBitmapScaler() }.map_err(e)?;
    unsafe { scaler.Initialize(&clipper, ow, oh, WICBitmapInterpolationModeHighQualityCubic) }.map_err(e)?;
    let stride = ow * 4;
    let mut px = vec![0u8; (stride * oh) as usize];
    unsafe { scaler.CopyPixels(std::ptr::null(), stride, &mut px) }.map_err(e)?;

    // Premultiplied BGRA to straight RGBA, for the PNG.
    for p in px.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        let straight = |c: u8| (u32::from(c) * 255 + a / 2).checked_div(a).map_or(0, |v| v.min(255) as u8);
        let (b, g, r) = (straight(p[0]), straight(p[1]), straight(p[2]));
        (p[0], p[1], p[2]) = (r, g, b);
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, ow, oh);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&px)).map_err(|e| format!("{path}: png: {e}"))?;
    Ok(out)
}

/// The part of an `sw`×`sh` image to take, and the size to scale it to, for a `w`×`h` frame.
fn crop(sw: u32, sh: u32, w: u32, h: u32, fit: Fit) -> (WICRect, u32, u32) {
    let (sw_f, sh_f, w_f, h_f) = (f64::from(sw), f64::from(sh), f64::from(w), f64::from(h));
    match fit {
        Fit::Contain => {
            let s = (w_f / sw_f).min(h_f / sh_f);
            let all = WICRect { X: 0, Y: 0, Width: sw as i32, Height: sh as i32 };
            (all, ((sw_f * s).round() as u32).clamp(1, w), ((sh_f * s).round() as u32).clamp(1, h))
        }
        Fit::Cover => {
            // Wider than the frame: trim the sides; taller: the top and bottom.
            let (cw, ch) = if sw_f * h_f > sh_f * w_f {
                ((sh_f * w_f / h_f).round().clamp(1.0, sw_f), sh_f)
            } else {
                (sw_f, (sw_f * h_f / w_f).round().clamp(1.0, sh_f))
            };
            let rect =
                WICRect { X: ((sw_f - cw) / 2.0) as i32, Y: ((sh_f - ch) / 2.0) as i32, Width: cw as i32, Height: ch as i32 };
            (rect, w, h)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops() {
        // A 4x logo into a 48-pixel square: all of it, at 48.
        let (r, w, h) = crop(176, 176, 48, 48, Fit::Contain);
        assert_eq!((r.Width, r.Height, w, h), (176, 176, 48, 48));
        // A wide banner into a square: all of it, letterboxed.
        let (_, w, h) = crop(400, 100, 48, 48, Fit::Contain);
        assert_eq!((w, h), (48, 12));
        // A square photo into a 16:9 thumbnail: its middle band.
        let (r, w, h) = crop(900, 900, 128, 72, Fit::Cover);
        assert_eq!((r.X, r.Y, r.Width, r.Height, w, h), (0, 197, 900, 506, 128, 72));
        // A panorama into the same: its middle.
        let (r, ..) = crop(3200, 900, 128, 72, Fit::Cover);
        assert_eq!((r.X, r.Y, r.Width, r.Height), (800, 0, 1600, 900));
    }

    /// Decodes `png`'s result: its size and RGBA pixels.
    fn decode(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut r = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        (info.width, info.height, buf)
    }

    #[test]
    fn scales_files() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
        // The 256-pixel logo into a 32-DIP square at 150%: transparent corners stay transparent, its middle opaque.
        let (w, h, px) = decode(&png(&format!("{dir}/assets/icon/pms-256.png"), 48, 48, Fit::Contain).unwrap());
        assert_eq!((w, h), (48, 48));
        assert_eq!(px[3], 0);
        assert_eq!(px[(24 * 48 + 24) * 4 + 3], 255);
        // The tall wizard picture into a thumbnail: filled, in the thumbnail's shape.
        let (w, h, _) = decode(&png(&format!("{dir}/installer/art/wizard-202x386.png"), 96, 54, Fit::Cover).unwrap());
        assert_eq!((w, h), (96, 54));
        assert!(png(&format!("{dir}/nothing-here.png"), 48, 48, Fit::Contain).is_err());
    }
}
