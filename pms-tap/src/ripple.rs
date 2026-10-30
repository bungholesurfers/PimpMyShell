//! A toast's colors: its icon's most prominent color glowing softly behind the icon on the card; a faint, wide glow
//! under the whole toast; and rings in that color rippling out from the icon across the whole taskbar. The last two
//! are in the background, under the buttons, like the hot-mic alert, for as long as the toast is up.
//!
//!   background Grid (stage.rs order) ... [Alert host] [ripple host] ...
//!     |- SpriteVisual: the glow under the toast
//!     `- ShapeVisual: the rings (rings.rs)
//!
//! UI thread only.

use std::cell::RefCell;

use pms_xaml::{Canvas, ElementCompositionPreview, Grid, UIElement};
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

/// The glow behind a toast's icon, `size` square: an element the icon's size with the glow, larger, centered on
/// it. It fades in, flares a little, and settles.
pub fn glow(size: f64, color: Color) -> R<UIElement> {
    let host = Canvas::new().map_err(w)?;
    // The host is the icon's size, so XAML draws it; the glow reaches beyond it.
    host.SetWidth(size).map_err(w)?;
    host.SetHeight(size).map_err(w)?;
    host.SetIsHitTestVisible(false).map_err(w)?;
    let host: UIElement = host.cast().map_err(w)?;
    let c = ElementCompositionPreview::GetElementVisual(&host).map_err(w)?.Compositor().map_err(w)?;
    let brush = c.CreateRadialGradientBrush().map_err(w)?;
    let stops = brush.ColorStops().map_err(w)?;
    for (at, alpha) in [(0.0, 0xB0), (0.34, 0x88), (0.6, 0x38), (0.82, 0x10), (1.0, 0)] {
        stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(at, Color { A: alpha, ..color }).map_err(w)?).map_err(w)?;
    }
    let side = (size + 2.0 * GLOW_REACH) as f32;
    let sprite = c.CreateSpriteVisual().map_err(w)?;
    sprite.SetSize(Vector2 { X: side, Y: side }).map_err(w)?;
    sprite.SetOffset(windows_numerics::Vector3 { X: -GLOW_REACH as f32, Y: -GLOW_REACH as f32, Z: 0.0 }).map_err(w)?;
    sprite.SetBrush(&brush).map_err(w)?;
    let fade = c.CreateScalarKeyFrameAnimation().map_err(w)?;
    fade.InsertKeyFrame(0.0, 0.0).map_err(w)?;
    fade.InsertKeyFrame(0.25, 1.0).map_err(w)?;
    fade.InsertKeyFrame(1.0, 0.85).map_err(w)?;
    fade.SetDuration(windows::Foundation::TimeSpan { Duration: 1800 * 10_000 }).map_err(w)?;
    sprite.StartAnimation(&windows_core::HSTRING::from("Opacity"), &fade).map_err(w)?;
    ElementCompositionPreview::SetElementChildVisual(&host, &sprite).map_err(w)?;
    Ok(host)
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
        let host: UIElement = Grid::new().and_then(|g| g.cast()).map_err(w)?;
        host.SetIsHitTestVisible(false).map_err(w)?;
        stage::insert(ctx, Slot::Alert, &host)?;
        let compositor = ElementCompositionPreview::GetElementVisual(&host).map_err(w)?.Compositor().map_err(w)?;
        let shapes = compositor.CreateShapeVisual().map_err(w)?;
        shapes.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 }).map_err(w)?;
        let rings = Emitter::new(&compositor, &shapes.Shapes().map_err(w)?, &RIPPLE_RINGS)?;
        let under = compositor.CreateSpriteVisual().map_err(w)?;
        under.SetOpacity(0.0).map_err(w)?;
        let under_brush = compositor.CreateRadialGradientBrush().map_err(w)?;
        under.SetBrush(&under_brush).map_err(w)?;
        let root = compositor.CreateContainerVisual().map_err(w)?;
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

/// The toast went: the glow under it fades, the rings in flight finish their journey, and no more set out.
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
