//! A toast's colors: its icon's most prominent color glowing softly behind the icon on the card; a faint, wide glow
//! under the whole toast; and rings in that color rippling out from the icon across the whole taskbar. The last two
//! are in the background, under the buttons, like the hot-mic alert, for as long as the card that brought them is in
//! front.
//!
//!   background Grid (stage.rs order) ... [Alert slot: the alert's, ripple's and button glows' hosts] ...
//!     |- SpriteVisual: the glow under the toast
//!     `- ShapeVisual: the rings (rings.rs)
//!
//! UI thread only.

use std::cell::RefCell;

use pms_xaml::{ElementCompositionPreview, UIElement};
use windows::UI::Color;
use windows::UI::Composition::*;
use windows_core::Interface;
use windows_numerics::Vector2;

use crate::Ctx;
use crate::commands::R;
use crate::layout::{SNAPSHOT, bounds};
use crate::lock;
use crate::pace::Bezier;
use crate::rings::{Cycle, Emitter};
use crate::stage::{self, Slot};

const RIPPLE_MS: i64 = 3600;
const RIPPLE_RINGS: [f32; 2] = [1.6, 2.6];
/// How far the glow reaches beyond the icon's edge, DIPs.
const GLOW_REACH: f64 = 34.0;
/// The glow under the toast: how wide, as a share of the taskbar; how strong at its middle; and how it falls off
/// toward its edge, alpha * (1 - t)^FALLOFF (1 = linear, more = a tighter core).
const UNDER_WIDTH: f32 = 0.5;
const UNDER_ALPHA: f32 = 0.30;
const UNDER_FALLOFF: f32 = 2.2;
const UNDER_IN_MS: i64 = 500;
const UNDER_OUT_MS: i64 = 700;

fn w(e: windows_core::Error) -> String {
    e.message()
}

/// What an icon is mostly colored: the hue that the most (and the most vivid) of its opaque pixels share, as a
/// bright color. Mostly gray icons give a soft white. None if the file isn't a PNG we can read.
pub fn prominent(path: &str) -> Option<Color> {
    let file = std::fs::File::open(path).ok()?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    prominent_in(&buf[..info.buffer_size()], info.color_type.samples(), false)
}

/// The same, for pixels already in hand: `channels` per pixel (gray, gray + alpha, RGB, RGBA; with `bgra`, 4 in
/// premultiplied BGRA order, as XAML renders them).
pub fn prominent_in(px: &[u8], channels: usize, bgra: bool) -> Option<Color> {
    const BINS: usize = 24;
    let mut bins = [(0.0f64, 0.0f64, 0.0f64, 0.0f64); BINS];
    let (mut opaque, mut vivid) = (0.0f64, 0.0f64);
    for p in px.chunks_exact(channels) {
        let (r, g, b, a) = match (channels, bgra) {
            (1, _) => (p[0], p[0], p[0], 255),
            (2, _) => (p[0], p[0], p[0], p[1]),
            (3, _) => (p[0], p[1], p[2], 255),
            (_, false) => (p[0], p[1], p[2], p[3]),
            (_, true) => {
                let un = |c: u8| ((c as u32 * 255) / (p[3] as u32).max(1)).min(255) as u8;
                (un(p[2]), un(p[1]), un(p[0]), p[3])
            }
        };
        let a = a as f64 / 255.0;
        if a < 0.5 {
            continue;
        }
        opaque += 1.0;
        let (rf, gf, bf) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
        let max = rf.max(gf).max(bf);
        let min = rf.min(gf).min(bf);
        let s = if max > 0.0 { (max - min) / max } else { 0.0 };
        if max < 0.15 || s < 0.15 {
            continue;
        }
        let h = if max == rf {
            ((gf - bf) / (max - min)).rem_euclid(6.0)
        } else if max == gf {
            (bf - rf) / (max - min) + 2.0
        } else {
            (rf - gf) / (max - min) + 4.0
        } / 6.0;
        let weight = a * s * max.sqrt();
        vivid += weight;
        let bin = &mut bins[((h * BINS as f64) as usize).min(BINS - 1)];
        *bin = (bin.0 + weight, bin.1 + weight * rf, bin.2 + weight * gf, bin.3 + weight * bf);
    }
    if opaque == 0.0 {
        return None;
    }
    if vivid < opaque * 0.04 {
        return Some(Color { A: 0xFF, R: 0xE8, G: 0xEC, B: 0xF2 });
    }
    // The strongest hue, with its neighbors (a hue sits across a bin's edge as often as not).
    let around = |i: usize| (0..3).map(|d| bins[(i + BINS + d - 1) % BINS]).fold((0.0, 0.0, 0.0, 0.0), |s, b| (s.0 + b.0, s.1 + b.1, s.2 + b.2, s.3 + b.3));
    let best = (0..BINS).map(around).max_by(|x, y| x.0.total_cmp(&y.0))?;
    let (r, g, b) = (best.1 / best.0, best.2 / best.0, best.3 / best.0);
    // As bright as it goes, hue and saturation kept: it's going to glow.
    let k = 0.96 / r.max(g).max(b).max(1e-3);
    let c = |v: f64| (v * k * 255.0).round().clamp(0.0, 255.0) as u8;
    Some(Color { A: 0xFF, R: c(r), G: c(g), B: c(b) })
}

/// How the glow fades out from its middle: (where, 0 the middle and 1 the edge; alpha).
const GLOW_STOPS: [(f32, u8); 5] = [(0.0, 0xB0), (0.34, 0x88), (0.6, 0x38), (0.82, 0x10), (1.0, 0)];

/// The glow behind a toast's icon, `size` square: an element the icon's size with the glow, larger, centered on it. It fades
/// in, flares a little, and settles. `px`: pixels per DIP. XAML draws it, a Border's picture brush (a picture of the glow,
/// made once for each color and size): a sprite of ours, with its gradient, stayed in Explorer for good, Close() or not
/// (2026-11-16, ~4 KB each card).
pub fn glow(size: f64, color: Color, px: f64) -> R<UIElement> {
    let host = crate::stage::canvas()?;
    // The host is the icon's size, so XAML lays it out as one; the glow reaches beyond it (a Canvas doesn't clip).
    host.SetWidth(size).map_err(w)?;
    host.SetHeight(size).map_err(w)?;
    host.SetIsHitTestVisible(false).map_err(w)?;
    let side = size + 2.0 * GLOW_REACH;
    let disc = pms_xaml::Border::new().map_err(w)?;
    disc.SetWidth(side).map_err(w)?;
    disc.SetHeight(side).map_err(w)?;
    let brush = pms_xaml::ImageBrush::new().map_err(w)?;
    brush.SetStretch(pms_xaml::Stretch::Fill).map_err(w)?;
    brush.SetImageSource(&glow_picture(color, (side * px).round().max(1.0) as u32)?).map_err(w)?;
    disc.SetBackground(&brush).map_err(w)?;
    let disc: UIElement = disc.cast().map_err(w)?;
    pms_xaml::Canvas::SetLeft(&disc, -GLOW_REACH).map_err(w)?;
    pms_xaml::Canvas::SetTop(&disc, -GLOW_REACH).map_err(w)?;
    host.Children().map_err(w)?.Append(&disc).map_err(w)?;
    let host: UIElement = host.cast().map_err(w)?;
    let v = ElementCompositionPreview::GetElementVisual(&host).map_err(w)?;
    let fade = v.Compositor().and_then(|c| c.CreateScalarKeyFrameAnimation()).map_err(w)?;
    fade.InsertKeyFrame(0.0, 0.0).map_err(w)?;
    fade.InsertKeyFrame(0.25, 1.0).map_err(w)?;
    fade.InsertKeyFrame(1.0, 0.85).map_err(w)?;
    fade.SetDuration(windows::Foundation::TimeSpan { Duration: 1800 * 10_000 }).map_err(w)?;
    v.StartAnimation(&windows_core::HSTRING::from("Opacity"), &fade).map_err(w)?;
    Ok(host)
}

/// A picture of the glow in `color`, `side` pixels square: GLOW_STOPS around its middle. Kept for the next card of the same
/// color, the last few. UI thread.
fn glow_picture(color: Color, side: u32) -> R<pms_xaml::BitmapImage> {
    const KEPT: usize = 8;
    thread_local! {
        static MADE: RefCell<Vec<((u32, u32), pms_xaml::BitmapImage)>> = const { RefCell::new(Vec::new()) };
    }
    let key = (u32::from_be_bytes([color.A, color.R, color.G, color.B]), side);
    if let Some(b) = MADE.with_borrow(|m| m.iter().find(|(k, _)| *k == key).map(|(_, b)| b.clone())) {
        return Ok(b);
    }
    let b = crate::toast::bitmap(&glow_png(color, side)?)?;
    MADE.with_borrow_mut(|m| {
        if m.len() >= KEPT {
            m.remove(0);
        }
        m.push((key, b.clone()));
    });
    Ok(b)
}

/// The glow's picture, as a PNG.
fn glow_png(color: Color, side: u32) -> R<Vec<u8>> {
    let half = side as f32 / 2.0;
    let alpha = |d: f32| -> u8 {
        let d = d.clamp(0.0, 1.0);
        let i = GLOW_STOPS.iter().position(|(at, _)| *at >= d).unwrap_or(GLOW_STOPS.len() - 1).max(1);
        let ((a0, v0), (a1, v1)) = (GLOW_STOPS[i - 1], GLOW_STOPS[i]);
        let t = if a1 > a0 { (d - a0) / (a1 - a0) } else { 1.0 };
        (f32::from(v0) + (f32::from(v1) - f32::from(v0)) * t).round() as u8
    };
    let mut px = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            let (dx, dy) = (x as f32 + 0.5 - half, y as f32 + 0.5 - half);
            px.extend([color.R, color.G, color.B, alpha((dx * dx + dy * dy).sqrt() / half)]);
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, side, side);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&px)).map_err(|e| format!("the glow's picture: {e}"))?;
    Ok(out)
}

struct Ripple {
    host: UIElement,
    compositor: Compositor,
    rings: Emitter,
    under: SpriteVisual,
    under_brush: CompositionRadialGradientBrush,
    /// Where the rings come from, in the host's coordinates.
    center: Vector2,
}

thread_local! {
    static RIPPLE: RefCell<Option<Ripple>> = const { RefCell::new(None) };
}

/// Ripple out from `at` (taskbar-window DIPs) in `color`, and glow under the toast's `card` (its middle and width,
/// the same way); rings from an earlier toast finish on their way.
pub fn start(ctx: &Ctx, at: Vector2, card: (Vector2, f32), color: Color) -> R<()> {
    if RIPPLE.with_borrow(Option::is_none) {
        let host: UIElement = crate::stage::grid()?.cast().map_err(w)?;
        host.SetIsHitTestVisible(false).map_err(w)?;
        stage::insert(ctx, Slot::Alert, &host)?;
        let compositor = ElementCompositionPreview::GetElementVisual(&host).map_err(w)?.Compositor().map_err(w)?;
        let shapes = compositor.CreateShapeVisual().map_err(w)?;
        shapes.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 }).map_err(w)?;
        let rings = Emitter::new(&compositor, &shapes.Shapes().map_err(w)?, &RIPPLE_RINGS)?;
        let under = stage::sprite(&compositor)?;
        under.SetOpacity(0.0).map_err(w)?;
        let under_brush = compositor.CreateRadialGradientBrush().map_err(w)?;
        under.SetBrush(&under_brush).map_err(w)?;
        let root = stage::container(&compositor)?;
        root.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 }).map_err(w)?;
        root.Children().map_err(w)?.InsertAtTop(&under).map_err(w)?;
        root.Children().map_err(w)?.InsertAtTop(&shapes).map_err(w)?;
        ElementCompositionPreview::SetElementChildVisual(&host, &root).map_err(w)?;
        RIPPLE.with_borrow_mut(|r| *r = Some(Ripple { host, compositor, rings, under, under_brush, center: Vector2::default() }));
    }
    RIPPLE.with_borrow_mut(|r| -> R<()> {
        let r = r.as_mut().ok_or("ripple vanished")?;
        r.glow_under(card, color)?;
        let origin = bounds(&r.host, 0.0, 0.0)?;
        let width = lock(&SNAPSHOT).width;
        r.rings.retire(&r.compositor)?;
        r.center = Vector2 { X: at.X - origin.X, Y: at.Y - origin.Y };
        for ring in r.rings.rings() {
            ring.geo.SetCenter(r.center).map_err(w)?;
        }
        let reach = r.center.X.max(width - r.center.X) + 30.0;
        let mix = |a: u8, b: u8| ((a as u16 + b as u16) / 2) as u8;
        let pale = |alpha: u8| Color { A: alpha, R: mix(color.R, 0xFF), G: mix(color.G, 0xFF), B: mix(color.B, 0xFF) };
        let tint = |alpha: u8| Color { A: alpha, ..color };
        r.rings.start(
            &r.compositor,
            Cycle {
                period: RIPPLE_MS,
                grow: vec![(0.0, Vector2 { X: 6.0, Y: 6.0 }, None), (1.0, Vector2 { X: reach, Y: reach }, Some(Bezier(0.25, 0.4, 0.55, 1.0)))],
                fade: vec![(0.0, pale(0), None), (0.05, pale(0x60), None), (0.45, tint(0x48), None), (1.0, tint(0), None)],
            },
        )
    })
}

impl Ripple {
    /// The wide glow under the toast: an ellipse half the taskbar wide, centered on the card, taller than the
    /// taskbar (whose edges cut it, so it fades sideways more than up and down), fading in.
    fn glow_under(&mut self, (middle, _width): (Vector2, f32), color: Color) -> R<()> {
        let origin = bounds(&self.host, 0.0, 0.0)?;
        let snap = lock(&SNAPSHOT).clone();
        let size = Vector2 { X: snap.width * UNDER_WIDTH, Y: snap.height * 3.0 };
        self.under.SetSize(size).map_err(w)?;
        let at = Vector2 { X: middle.X - origin.X - size.X / 2.0, Y: middle.Y - origin.Y - size.Y / 2.0 };
        self.under.SetOffset(windows_numerics::Vector3 { X: at.X, Y: at.Y, Z: 0.0 }).map_err(w)?;
        let stops = self.under_brush.ColorStops().map_err(w)?;
        stops.Clear().map_err(w)?;
        for i in 0..=8 {
            let t = i as f32 / 8.0;
            let alpha = (UNDER_ALPHA * (1.0 - t).powf(UNDER_FALLOFF) * 255.0).round() as u8;
            stops.Append(&self.compositor.CreateColorGradientStopWithOffsetAndColor(t, Color { A: alpha, ..color }).map_err(w)?).map_err(w)?;
        }
        self.fade_under(1.0, UNDER_IN_MS)
    }

    fn fade_under(&self, to: f32, ms: i64) -> R<()> {
        let fade = self.compositor.CreateScalarKeyFrameAnimation().map_err(w)?;
        fade.InsertKeyFrame(1.0, to).map_err(w)?;
        fade.SetDuration(windows::Foundation::TimeSpan { Duration: ms * 10_000 }).map_err(w)?;
        self.under.StartAnimation(&windows_core::HSTRING::from("Opacity"), &fade).map_err(w)
    }
}

/// The card it greeted left the front (or the toast went): the glow under it fades, the rings in flight finish their journey, and no more set out.
pub fn stop() {
    RIPPLE.with_borrow_mut(|r| {
        if let Some(r) = r.as_mut()
            && let Err(e) = r.rings.retire(&r.compositor).and_then(|()| r.fade_under(0.0, UNDER_OUT_MS))
        {
            crate::log(format!("ripple stop: {e}"));
        }
    });
}

/// Take the host away (`reset`).
pub fn off(ctx: &Ctx) -> R<()> {
    if let Some(r) = RIPPLE.with_borrow_mut(Option::take) {
        stage::remove(ctx, &r.host)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_glows_picture_fades_out_from_its_middle_as_its_stops_say() {
        let color = Color { A: 0xFF, R: 0x20, G: 0x80, B: 0xF0 };
        let side = 40;
        let png = glow_png(color, side).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(png)).read_info().unwrap();
        let mut px = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut px).unwrap();
        let at = |x: u32, y: u32| &px[((y * side + x) * 4) as usize..][..4];
        let middle = at(side / 2, side / 2);
        assert_eq!(&middle[..3], &[0x20, 0x80, 0xF0], "the glow's own color");
        assert!(middle[3] >= 0xAC, "nearly the first stop's alpha in the middle: {}", middle[3]);
        assert_eq!(at(0, 0)[3], 0, "nothing in the corners, past the edge");
        assert!(at(0, side / 2)[3] <= 3, "next to nothing at the edge (its middle's just inside it): {}", at(0, side / 2)[3]);
        let (near, far) = (at(side / 2 + 4, side / 2)[3], at(side / 2 + 12, side / 2)[3]);
        assert!(near > far && far > 0, "fainter further out: {near}, {far}");
    }
}
