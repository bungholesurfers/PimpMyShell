//! Our replacement for the taskbar's stock AcrylicBrush.
//!
//! The stock brush bakes blur, tint and grain into one effect, so nothing can be drawn between
//! them. Instead we hide its fill and attach our own composition tree to a host element that
//! sits right above `BackgroundFill` (and so below every button):
//!
//!   host Grid (XAML, inserted after BackgroundFill)
//!     `- root ContainerVisual  (ElementCompositionPreview::SetElementChildVisual)
//!         |- backdrop  SpriteVisual  HostBackdropBrush (DWM's already-blurred backdrop)
//!         |- tint      SpriteVisual  color brush          <- `tint`
//!         |- layer     SpriteVisual  color brush          <- `layer`, `pulse`: our stuff goes here
//!         |- gaps      ContainerVisual over free space    <- `gaps` (follows layout.rs)
//!         `- grain     ContainerVisual of noise tiles     <- `grain`
//!
//! Everything here runs on the XAML UI thread, so state lives in a thread_local.

use std::cell::{Cell, RefCell};

use pms_xaml::{ElementCompositionPreview, Grid, LoadedImageSurface, UIElement};
use windows::Foundation::TimeSpan;
use windows::Storage::Streams::IRandomAccessStream;
use windows::UI::Color;
use windows::UI::Composition::*;
use windows::Win32::System::WinRT::{BSOS_DEFAULT, CreateRandomAccessStreamOverStream};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows_core::{HSTRING, IInspectable, Interface};
use windows_numerics::{Vector2, Vector3};

use crate::commands::{R, create_instance, hr, parse_handle, prop_index, resolve_elem};
use crate::xamlom::InstanceHandle;
use crate::stage::{self, Slot};
use crate::{Ctx, lock, props};

const NOISE_W: u32 = 512;
const NOISE_H: u32 = 128;
/// Grain tiles cover this many DIPs horizontally; the taskbar window clips the rest.
const GRAIN_SPAN: f32 = 8192.0;

const DEFAULT_TINT: Color = Color { A: 0xC0, R: 0x20, G: 0x20, B: 0x20 };
const DEFAULT_GRAIN: f32 = 0.01;

struct Backdrop {
    fill: InstanceHandle,
    stock_brush: InstanceHandle,
    /// Strong ref: once unparented, diagnostics forgets the handle and the brush may die.
    stock_object: IInspectable,
    /// Backdrop slot: blur, tint, layer, gaps.
    host: UIElement,
    /// Grain slot: above the stage, so stage content gets grained like the stock look.
    grain_host: UIElement,
    compositor: Compositor,
    tint: CompositionColorBrush,
    layer: SpriteVisual,
    layer_brush: CompositionColorBrush,
    /// Sprites over the free stretches of the taskbar, redrawn on every layout change.
    gaps: ContainerVisual,
    gap_brush: CompositionColorBrush,
    gaps_on: Cell<bool>,
    grain: ContainerVisual,
    // Keep the decoded noise alive for as long as the brush uses it.
    _noise: LoadedImageSurface,
}

thread_local! {
    static STATE: RefCell<Option<Backdrop>> = const { RefCell::new(None) };
}

fn w(e: windows_core::Error) -> String {
    e.to_string()
}

pub fn command(ctx: &Ctx, args: &[String]) -> R<String> {
    let sub = args.first().map(String::as_str).unwrap_or("status");
    let val = args.get(1).map(String::as_str);
    match sub {
        "on" => enable(ctx),
        "off" => disable(ctx),
        "status" => STATE.with_borrow(|s| {
            Ok(match s {
                Some(b) => format!("on; stock brush {:#x} parked", b.stock_brush),
                None => "off (stock acrylic)".into(),
            })
        }),
        "tint" => with(|b| {
            b.tint.SetColor(parse_color(val.ok_or("missing <#AARRGGBB>")?)?).map_err(w)?;
            Ok("tint set".into())
        }),
        "grain" => with(|b| {
            let v: f32 = val.ok_or("missing <opacity 0..1>")?.parse().map_err(|e| format!("{e}"))?;
            b.grain.SetOpacity(v.clamp(0.0, 1.0)).map_err(w)?;
            Ok(format!("grain opacity {v}"))
        }),
        "layer" => with(|b| {
            b.layer_brush.SetColor(parse_color(val.ok_or("missing <#AARRGGBB>")?)?).map_err(w)?;
            Ok("layer color set".into())
        }),
        "pulse" => with(|b| pulse(b, &args[1..])),
        "gaps" => match val.ok_or("missing <#AARRGGBB> or off")? {
            "off" => with(|b| {
                b.gaps_on.set(false);
                b.gaps.Children().and_then(|c| c.RemoveAll()).map_err(w)?;
                Ok("gaps off".into())
            }),
            color => {
                with(|b| {
                    b.gap_brush.SetColor(parse_color(color)?).map_err(w)?;
                    b.gaps_on.set(true);
                    Ok(String::new())
                })?;
                crate::layout::ensure_tracking(ctx)?;
                let snap = lock(&crate::layout::SNAPSHOT).clone();
                on_layout(&snap);
                Ok(format!("painting {} free interval(s); follows layout changes", snap.free.len()))
            }
        },
        other => Err(format!("unknown backdrop subcommand `{other}` (on|off|status|tint|grain|layer|pulse|gaps)")),
    }
}

pub fn is_on() -> bool {
    STATE.with_borrow(Option::is_some)
}

fn with(f: impl FnOnce(&Backdrop) -> R<String>) -> R<String> {
    STATE.with_borrow(|s| match s {
        Some(b) => f(b),
        None => Err("backdrop is off; `backdrop on` first".into()),
    })
}

pub(crate) fn parse_color(s: &str) -> R<Color> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    let n = u32::from_str_radix(hex, 16).map_err(|_| format!("bad color `{s}`"))?;
    let (a, rgb) = match hex.len() {
        6 => (0xFF, n),
        8 => ((n >> 24) as u8, n & 0xFF_FFFF),
        _ => return Err(format!("bad color `{s}`; use #RRGGBB or #AARRGGBB")),
    };
    Ok(Color { A: a, R: (rgb >> 16) as u8, G: (rgb >> 8) as u8, B: rgb as u8 })
}

fn sprite(compositor: &Compositor, brush: &CompositionBrush) -> windows_core::Result<SpriteVisual> {
    let s = compositor.CreateSpriteVisual()?;
    s.SetBrush(brush)?;
    s.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })?;
    Ok(s)
}

/// Grayscale speckle, full alpha; its strength is the grain visual's opacity.
fn noise_png() -> Vec<u8> {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut px = Vec::with_capacity((NOISE_W * NOISE_H * 4) as usize);
    for _ in 0..NOISE_W * NOISE_H {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let v = (seed >> 56) as u8;
        px.extend_from_slice(&[v, v, v, 0xFF]);
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, NOISE_W, NOISE_H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(&px)).expect("png encode into Vec");
    out
}

fn noise_surface() -> R<LoadedImageSurface> {
    let png = noise_png();
    let stream = unsafe { SHCreateMemStream(Some(&png)) }.ok_or("SHCreateMemStream failed")?;
    let stream: IRandomAccessStream = unsafe { CreateRandomAccessStreamOverStream(&stream, BSOS_DEFAULT) }.map_err(w)?;
    LoadedImageSurface::StartLoadFromStream(&stream).map_err(w)
}

fn enable(ctx: &Ctx) -> R<String> {
    if STATE.with_borrow(Option::is_some) {
        return Ok("already on".into());
    }
    let fill = resolve_elem("BackgroundFill")?;
    let stock = props::find_prop(&ctx.service, fill, "Fill").map_err(w)?.ok_or("BackgroundFill has no Fill")?;
    let stock_brush = parse_handle(&stock.value)?;
    let mut stock_object = None;
    hr(unsafe { ctx.diag.GetIInspectableFromHandle(stock_brush, &mut stock_object) }, "GetIInspectableFromHandle(stock)")?;
    let stock_object = stock_object.ok_or("stock brush has no object")?;

    // DWM only feeds a host backdrop to windows that opted in. XAML toggles this along with
    // its own HostBackdrop acrylic, so it drops out (black HostBackdropBrush) once we park the
    // stock brush. Opt in ourselves. We never opt out: the stock acrylic needs it too.
    crate::dwm::set_host_backdrop(true)?;

    // XAML side: two empty (so hit-test-invisible) Grids in the backdrop and grain slots.
    let host: UIElement = Grid::new().and_then(|g| g.cast()).map_err(w)?;
    let grain_host: UIElement = Grid::new().and_then(|g| g.cast()).map_err(w)?;
    stage::insert(ctx, Slot::Backdrop, &host)?;
    if let Err(e) = stage::insert(ctx, Slot::Grain, &grain_host) {
        let _ = stage::remove(ctx, &host);
        return Err(e);
    }

    let built = (|| -> R<Backdrop> {
        let compositor = ElementCompositionPreview::GetElementVisual(&host).map_err(w)?.Compositor().map_err(w)?;

        let root = compositor.CreateContainerVisual().map_err(w)?;
        root.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 }).map_err(w)?;
        let children = root.Children().map_err(w)?;

        let backdrop = compositor.CreateHostBackdropBrush().map_err(w)?;
        children.InsertAtTop(&sprite(&compositor, &backdrop.cast().map_err(w)?).map_err(w)?).map_err(w)?;

        let tint = compositor.CreateColorBrushWithColor(DEFAULT_TINT).map_err(w)?;
        children.InsertAtTop(&sprite(&compositor, &tint.cast().map_err(w)?).map_err(w)?).map_err(w)?;

        let layer_brush = compositor.CreateColorBrushWithColor(Color { A: 0, R: 0, G: 0, B: 0 }).map_err(w)?;
        let layer = sprite(&compositor, &layer_brush.cast().map_err(w)?).map_err(w)?;
        children.InsertAtTop(&layer).map_err(w)?;

        let gaps = compositor.CreateContainerVisual().map_err(w)?;
        gaps.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 }).map_err(w)?;
        children.InsertAtTop(&gaps).map_err(w)?;
        let gap_brush = compositor.CreateColorBrushWithColor(Color { A: 0, R: 0, G: 0, B: 0 }).map_err(w)?;

        let noise = noise_surface()?;
        let noise_brush = compositor.CreateSurfaceBrushWithSurface(&noise).map_err(w)?;
        noise_brush.SetStretch(CompositionStretch::None).map_err(w)?;
        noise_brush.SetHorizontalAlignmentRatio(0.0).map_err(w)?;
        noise_brush.SetVerticalAlignmentRatio(0.0).map_err(w)?;
        let noise_brush: CompositionBrush = noise_brush.cast().map_err(w)?;
        let grain = compositor.CreateContainerVisual().map_err(w)?;
        grain.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 }).map_err(w)?;
        grain.SetOpacity(DEFAULT_GRAIN).map_err(w)?;
        let tiles = grain.Children().map_err(w)?;
        let mut x = 0.0;
        while x < GRAIN_SPAN {
            let tile = compositor.CreateSpriteVisual().map_err(w)?;
            tile.SetBrush(&noise_brush).map_err(w)?;
            tile.SetSize(Vector2 { X: NOISE_W as f32, Y: NOISE_H as f32 }).map_err(w)?;
            tile.SetOffset(Vector3 { X: x, Y: 0.0, Z: 0.0 }).map_err(w)?;
            tiles.InsertAtTop(&tile).map_err(w)?;
            x += NOISE_W as f32;
        }
        ElementCompositionPreview::SetElementChildVisual(&host, &root).map_err(w)?;
        ElementCompositionPreview::SetElementChildVisual(&grain_host, &grain).map_err(w)?;
        Ok(Backdrop {
            fill,
            stock_brush,
            stock_object: stock_object.clone(),
            host: host.clone(),
            grain_host: grain_host.clone(),
            compositor,
            tint,
            layer,
            layer_brush,
            gaps,
            gap_brush,
            gaps_on: Cell::new(false),
            grain,
            _noise: noise,
        })
    })();
    let b = match built {
        Ok(b) => b,
        Err(e) => {
            let _ = stage::remove(ctx, &host);
            let _ = stage::remove(ctx, &grain_host);
            return Err(e);
        }
    };

    // Only now hide the stock acrylic.
    let transparent = create_instance(ctx, "Windows.UI.Xaml.Media.SolidColorBrush", "Transparent")?;
    let fill_prop = prop_index(ctx, fill, "Fill")?;
    hr(unsafe { ctx.service.SetProperty(fill, transparent, fill_prop) }, "SetProperty(Fill)")?;

    let msg = format!("on; stock brush {stock_brush:#x} parked");
    STATE.with_borrow_mut(|s| *s = Some(b));
    Ok(msg)
}

pub(crate) fn disable(ctx: &Ctx) -> R<String> {
    let Some(b) = STATE.with_borrow_mut(Option::take) else { return Ok("already off".into()) };
    let fill_prop = prop_index(ctx, b.fill, "Fill")?;
    let mut stock = 0;
    hr(unsafe { ctx.diag.RegisterInstance(b.stock_object.as_raw(), &mut stock) }, "RegisterInstance(stock)")?;
    hr(unsafe { ctx.service.SetProperty(b.fill, stock, fill_prop) }, "restore Fill")?;
    stage::remove(ctx, &b.host)?;
    stage::remove(ctx, &b.grain_host)?;
    Ok("off; stock acrylic restored".into())
}

fn pulse(b: &Backdrop, args: &[String]) -> R<String> {
    let opacity = HSTRING::from("Opacity");
    match args.first().map(String::as_str) {
        None | Some("off") => {
            b.layer.StopAnimation(&opacity).map_err(w)?;
            b.layer.SetOpacity(1.0).map_err(w)?;
            Ok("pulse off".into())
        }
        Some(color) => {
            let period_ms: i64 = args.get(1).map_or(Ok(1200), |p| p.parse()).map_err(|e| format!("{e}"))?;
            let low: f32 = args.get(2).map_or(Ok(0.15), |p| p.parse()).map_err(|e| format!("{e}"))?;
            b.layer_brush.SetColor(parse_color(color)?).map_err(w)?;
            let anim = b.compositor.CreateScalarKeyFrameAnimation().map_err(w)?;
            anim.InsertKeyFrame(0.0, low).map_err(w)?;
            anim.InsertKeyFrame(0.5, 1.0).map_err(w)?;
            anim.InsertKeyFrame(1.0, low).map_err(w)?;
            anim.SetDuration(TimeSpan { Duration: period_ms * 10_000 }).map_err(w)?;
            anim.SetIterationBehavior(AnimationIterationBehavior::Forever).map_err(w)?;
            b.layer.StartAnimation(&opacity, &anim).map_err(w)?;
            Ok(format!("pulsing {color} every {period_ms}ms (low opacity {low})"))
        }
    }
}

/// Called by layout.rs (UI thread) whenever the button layout changes.
pub fn on_layout(snapshot: &crate::layout::Snapshot) {
    STATE.with(|cell| {
        // Busy means a backdrop command triggered this layout pass; it redraws on its own.
        let Ok(state) = cell.try_borrow() else { return };
        if let Some(b) = state.as_ref().filter(|b| b.gaps_on.get())
            && let Err(e) = draw_gaps(b, snapshot) {
                crate::log(format!("draw gaps: {e}"));
            }
    });
}

fn draw_gaps(b: &Backdrop, s: &crate::layout::Snapshot) -> R<()> {
    const INSET_X: f32 = 2.0;
    const INSET_Y: f32 = 6.0;
    // Snapshot coordinates are relative to the taskbar window; ours to the host element.
    let origin = crate::layout::bounds(&b.host, 0.0, 0.0)?;
    let tiles = b.gaps.Children().map_err(w)?;
    tiles.RemoveAll().map_err(w)?;
    for &(start, end) in &s.free {
        let tile = b.compositor.CreateSpriteVisual().map_err(w)?;
        tile.SetBrush(&b.gap_brush).map_err(w)?;
        tile.SetOffset(Vector3 { X: start - origin.X + INSET_X, Y: INSET_Y - origin.Y, Z: 0.0 }).map_err(w)?;
        tile.SetSize(Vector2 { X: (end - start - 2.0 * INSET_X).max(0.0), Y: (s.height - 2.0 * INSET_Y).max(0.0) }).map_err(w)?;
        tiles.InsertAtTop(&tile).map_err(w)?;
    }
    Ok(())
}
