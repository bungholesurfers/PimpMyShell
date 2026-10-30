//! The hot-mic alert, in its own slot above the backdrop and below the stage (so it works over
//! both the stock acrylic and ours, and never covers meters or buttons).
//!
//!   alert broad <#color> <period_ms> <low> <high>
//!       the whole taskbar breathes in the color, sonar rings echo from its center
//!   alert local
//!       the wash shrinks into a pill around the mic label (meters.rs) and the rings echo from
//!       there instead: "still on, but I'll keep to my corner"
//!   alert off
//!
//!   host Grid (Alert slot)
//!     `- ShapeVisual
//!         |- wash: rounded rect, fill alpha breathing between low and high
//!         `- rings: 3 white ellipse strokes of different weights, growing and fading, staggered
//!
//! UI thread only.

use std::cell::RefCell;
use std::time::Instant;

use pms_xaml::{ElementCompositionPreview, FrameworkElement, Grid, UIElement};
use windows::Foundation::{Rect, TimeSpan};
use windows::UI::Color;
use windows::UI::Composition::*;
use windows_core::{HSTRING, Interface};
use windows_numerics::Vector2;

use crate::Ctx;
use crate::backdrop::parse_color;
use crate::commands::R;
use crate::layout::{SNAPSHOT, bounds};
use crate::stage::{self, Slot};
use crate::lock;
use crate::pace::{self, Bezier};
use crate::rings::{Cycle, Emitter};

const RINGS: [f32; 3] = [1.2, 2.4, 1.6];
const RING_MS: i64 = 2600;
const RING_ALPHA: u8 = 0x60;
const FAR_RINGS: [f32; 2] = [1.4, 2.6];
const FAR_RING_MS: i64 = 4800;
/// What the far rings blush to as they travel: white toward red, pinkish.
const BLUSH: Color = Color { A: 0, R: 0xFF, G: 0x70, B: 0x8C };
const MORPH_MS: i64 = 750;
/// The local pill reaches this far around the label.
const PILL_PAD: f32 = 6.0;

struct Alert {
    host: UIElement,
    compositor: Compositor,
    wash: CompositionRoundedRectangleGeometry,
    wash_brush: CompositionColorBrush,
    /// The rings around the alert's center, and the far ones (local phase), crossing the whole taskbar.
    near: Emitter,
    far: Emitter,
    tide: SpriteVisual,
    /// "Level" (mic loudness 0..1, from the daemon: the flicker's strength), "Tide" (the same, in
    /// decibels: the rising glow's height) and "Noise" (30 Hz flicker pattern).
    level: CompositionPropertySet,
    muted: bool,
    color: Color,
    period_ms: i64,
    low: f32,
    high: f32,
    local: bool,
    /// Where the rings echo from and how far they reach, last applied, and in which mode.
    focus: (Vector2, f32),
    focus_local: bool,
    /// Where the wash and the rings' center were last put (a morph starts from there), whether they ride on the mic
    /// label's visuals, and when the running morph ends.
    last: (Vector2, Vector2),
    anchored: bool,
    morph_until: Instant,
}

thread_local! {
    static ALERT: RefCell<Option<Alert>> = const { RefCell::new(None) };
}

fn w(e: windows_core::Error) -> String {
    e.to_string()
}

fn ms(n: i64) -> TimeSpan {
    TimeSpan { Duration: n * 10_000 }
}

pub fn command(ctx: &Ctx, args: &[String]) -> R<String> {
    let num = |i: usize, default: f32| args.get(i).map_or(Ok(default), |s| s.parse::<f32>().map_err(|e| format!("{s}: {e}")));
    match args.first().map(String::as_str) {
        Some("off") => off(ctx),
        Some("status") | None => Ok(match ALERT.with_borrow(|a| a.as_ref().map(|a| a.local)) {
            None => "off".into(),
            Some(false) => "broad".into(),
            Some(true) => "local".into(),
        }),
        Some("local") => {
            if !is_on() {
                broad(ctx, parse_color("#E53935")?, 1600, 0.25, 0.7)?;
            }
            set_local(true)?;
            Ok("alert local".into())
        }
        // level <0..1>: mic loudness drives the flicker. muted <0|1>: no flicker while muted.
        // level <flicker 0..1> [tide 0..1]
        Some("level") => {
            let v = num(1, 0.0)?.clamp(0.0, 1.0);
            set_level(v, num(2, v)?.clamp(0.0, 1.0));
            Ok(String::new())
        }
        Some("muted") => {
            let muted = args.get(1).map(String::as_str) == Some("1");
            ALERT.with_borrow_mut(|s| s.as_mut().map(|a| a.muted = muted));
            if muted {
                set_level(0.0, 0.0);
            }
            Ok(format!("alert muted: {muted}"))
        }
        Some("broad") => {
            let color = parse_color(args.get(1).ok_or("missing <#color>")?)?;
            broad(ctx, color, num(2, 1600.0)? as i64, num(3, 0.25)?, num(4, 0.7)?)
        }
        // Old form: alert <#color> [ms] [low] [high].
        Some(color) => broad(ctx, parse_color(color)?, num(1, 1600.0)? as i64, num(2, 0.25)?, num(3, 0.7)?),
    }
}

pub fn is_on() -> bool {
    ALERT.with_borrow(Option::is_some)
}

fn alpha(c: Color, a: f32) -> Color {
    Color { A: (a.clamp(0.0, 1.0) * 255.0) as u8, ..c }
}

/// The host's size, and its origin in taskbar-window coordinates.
fn frame(host: &UIElement) -> R<(Vector2, Rect)> {
    let fe: FrameworkElement = host.cast().map_err(w)?;
    let (mut width, mut height) = (fe.ActualWidth().map_err(w)? as f32, fe.ActualHeight().map_err(w)? as f32);
    if width <= 0.0 {
        let snap = lock(&SNAPSHOT);
        (width, height) = (snap.width, snap.height);
    }
    Ok((Vector2 { X: width, Y: height }, bounds(host, 0.0, 0.0)?))
}

fn broad(ctx: &Ctx, color: Color, period_ms: i64, low: f32, high: f32) -> R<String> {
    if !is_on() {
        let host: UIElement = Grid::new().and_then(|g| g.cast()).map_err(w)?;
        stage::insert(ctx, Slot::Alert, &host)?;
        match build(&host, color) {
            Ok(a) => ALERT.with_borrow_mut(|s| *s = Some(a)),
            Err(e) => {
                let _ = stage::remove(ctx, &host);
                return Err(e);
            }
        }
    }
    ALERT.with_borrow_mut(|s| -> R<()> {
        let a = s.as_mut().ok_or("alert vanished")?;
        (a.color, a.period_ms, a.low, a.high) = (color, period_ms, low, high);
        breathe(a)
    })?;
    set_local(false)?;
    Ok(format!("alert broad, breathing {low}..{high} every {period_ms}ms"))
}

fn build(host: &UIElement, color: Color) -> R<Alert> {
    let compositor = ElementCompositionPreview::GetElementVisual(host).map_err(w)?.Compositor().map_err(w)?;
    let shapes = compositor.CreateShapeVisual().map_err(w)?;
    shapes.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 }).map_err(w)?;
    let list = shapes.Shapes().map_err(w)?;

    let wash = compositor.CreateRoundedRectangleGeometry().map_err(w)?;
    let wash_brush = compositor.CreateColorBrushWithColor(alpha(color, 0.0)).map_err(w)?;
    let wash_shape = compositor.CreateSpriteShapeWithGeometry(&wash).map_err(w)?;
    wash_shape.SetFillBrush(&wash_brush).map_err(w)?;
    list.Append(&wash_shape).map_err(w)?;

    // Flicker: a brighter copy of the wash whose opacity is mic level x noise, evaluated on the
    // compositor every frame; we only feed it the level.
    let flicker = compositor.CreateShapeVisual().map_err(w)?;
    flicker.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 }).map_err(w)?;
    let bright = Color { A: 0xB0, R: lighten(color.R), G: lighten(color.G), B: lighten(color.B) };
    let flicker_shape = compositor.CreateSpriteShapeWithGeometry(&wash).map_err(w)?;
    flicker_shape.SetFillBrush(&compositor.CreateColorBrushWithColor(bright).map_err(w)?).map_err(w)?;
    flicker.Shapes().map_err(w)?.Append(&flicker_shape).map_err(w)?;
    let level = compositor.CreatePropertySet().map_err(w)?;
    level.InsertScalar(&HSTRING::from("Level"), 0.0).map_err(w)?;
    level.InsertScalar(&HSTRING::from("Noise"), 0.0).map_err(w)?;
    noise(&compositor, &level)?;
    let expr = compositor.CreateExpressionAnimationWithExpression(&HSTRING::from("p.Level * p.Noise")).map_err(w)?;
    expr.SetReferenceParameter(&HSTRING::from("p"), &level).map_err(w)?;
    flicker.StartAnimation(&HSTRING::from("Opacity"), &expr).map_err(w)?;

    let ring_layer = compositor.CreateShapeVisual().map_err(w)?;
    ring_layer.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 }).map_err(w)?;
    let list = ring_layer.Shapes().map_err(w)?;

    let near = Emitter::new(&compositor, &list, &RINGS)?;
    // Far rings (local phase): fewer, slower, crossing the whole taskbar, white blushing to pink.
    let far = Emitter::new(&compositor, &list, &FAR_RINGS)?;

    // The tide: red rising from the bottom edge as high as the mic is loud (log scale, so
    // near-silence still shows a little). Sized in apply(); its height is Scale.Y = p.Tide.
    let tide = compositor.CreateSpriteVisual().map_err(w)?;
    let gradient = compositor.CreateLinearGradientBrush().map_err(w)?;
    gradient.SetStartPoint(Vector2 { X: 0.0, Y: 1.0 }).map_err(w)?;
    gradient.SetEndPoint(Vector2 { X: 0.0, Y: 0.0 }).map_err(w)?;
    let stops = gradient.ColorStops().map_err(w)?;
    for (at, a) in [(0.0, 0.62), (0.55, 0.28), (1.0, 0.0)] {
        stops.Append(&compositor.CreateColorGradientStopWithOffsetAndColor(at, alpha(color, a)).map_err(w)?).map_err(w)?;
    }
    tide.SetBrush(&gradient).map_err(w)?;
    level.InsertScalar(&HSTRING::from("Tide"), 0.0).map_err(w)?;
    // The morph toward the mic label (apply), and where it runs from and to; set before anything animates them.
    level.InsertScalar(&HSTRING::from("T"), 1.0).map_err(w)?;
    for name in ["FromOff", "FromCenter", "ToOff", "ToCenter"] {
        level.InsertVector2(&HSTRING::from(name), Vector2::default()).map_err(w)?;
    }
    let rise = compositor.CreateExpressionAnimationWithExpression(&HSTRING::from("Vector3(1, p.Tide, 1)")).map_err(w)?;
    rise.SetReferenceParameter(&HSTRING::from("p"), &level).map_err(w)?;
    tide.StartAnimation(&HSTRING::from("Scale"), &rise).map_err(w)?;

    let root = compositor.CreateContainerVisual().map_err(w)?;
    root.SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 }).map_err(w)?;
    for v in [shapes.cast::<Visual>().map_err(w)?, tide.cast().map_err(w)?, flicker.cast().map_err(w)?, ring_layer.cast().map_err(w)?] {
        root.Children().map_err(w)?.InsertAtTop(&v).map_err(w)?;
    }
    ElementCompositionPreview::SetElementChildVisual(host, &root).map_err(w)?;
    Ok(Alert {
        host: host.clone(),
        compositor,
        wash,
        wash_brush,
        near,
        far,
        focus_local: false,
        tide,
        level,
        muted: false,
        color,
        period_ms: 1600,
        low: 0.25,
        high: 0.7,
        local: false,
        focus: (Vector2::default(), 0.0),
        last: (Vector2::default(), Vector2::default()),
        anchored: false,
        morph_until: Instant::now(),
    })
}

/// The wash's fill alpha, low -> high -> low, forever (or steady at `high`).
fn breathe(a: &Alert) -> R<()> {
    let color = HSTRING::from("Color");
    if a.period_ms <= 0 {
        a.wash_brush.StopAnimation(&color).map_err(w)?;
        return a.wash_brush.SetColor(alpha(a.color, a.high)).map_err(w);
    }
    let anim = a.compositor.CreateColorKeyFrameAnimation().map_err(w)?;
    let (low, high) = (alpha(a.color, a.low), alpha(a.color, a.high));
    pace::color(&a.compositor, &anim, &[(0.0, low, None), (0.5, high, None), (1.0, low, None)], a.period_ms)?;
    anim.SetDuration(ms(a.period_ms)).map_err(w)?;
    anim.SetIterationBehavior(AnimationIterationBehavior::Forever).map_err(w)?;
    a.wash_brush.StartAnimation(&color, &anim).map_err(w)
}

/// Where the wash and the rings go in the current mode.
struct Target {
    off: Vector2,
    size: Vector2,
    radius: f32,
    center: Vector2,
    reach: f32,
    /// Local, around the label: the wash's offset and the rings' center follow the label's visuals on the
    /// compositor (`off` and `center` are where it is now).
    anchor: Option<Anchor>,
}

struct Anchor {
    panel: Visual,
    cell: Visual,
    /// From the label's drawn position to the wash's offset, and to the rings' center.
    to_off: Vector2,
    to_center: Vector2,
}

/// Where the wash and rings go for the current mode; None to stay put (a label just rebuilt, not yet placed, while
/// the old one's visuals still hold its position).
fn target(a: &Alert) -> R<Option<Target>> {
    let (size, origin) = frame(&a.host)?;
    if !a.local {
        let center = Vector2 { X: size.X / 2.0, Y: size.Y / 2.0 };
        return Ok(Some(Target { off: Vector2::default(), size, radius: 0.0, center, reach: size.X / 2.0 + 24.0, anchor: None }));
    }
    match crate::meters::mic_anchor() {
        Some(m) if m.placed => {
            // The stage, relative to our host; then the label within it.
            let base = Vector2 { X: m.origin.X - origin.X, Y: m.origin.Y - origin.Y };
            let to_off = Vector2 { X: base.X - PILL_PAD, Y: base.Y + 1.0 };
            let to_center = Vector2 { X: base.X + m.size.X / 2.0, Y: base.Y + m.size.Y / 2.0 };
            // Where the label is now, in the stage, from layout: a label just placed has its visual's offset only
            // from the next frame (the expressions below catch up by themselves; a value read now wouldn't).
            let at = Vector2 { X: m.rect.X - m.origin.X, Y: m.rect.Y - m.origin.Y };
            Ok(Some(Target {
                off: Vector2 { X: at.X + to_off.X, Y: at.Y + to_off.Y },
                size: Vector2 { X: m.size.X + 2.0 * PILL_PAD, Y: (m.size.Y - 2.0).max(8.0) },
                radius: 7.0,
                center: Vector2 { X: at.X + to_center.X, Y: at.Y + to_center.Y },
                reach: (m.size.X * 0.75).max(60.0),
                anchor: Some(Anchor { panel: m.panel, cell: m.cell, to_off, to_center }),
            }))
        }
        Some(_) if a.anchored => Ok(None),
        // No label on screen (no room): a small pill at the tray's edge.
        _ => {
            let snap = lock(&SNAPSHOT).clone();
            let end = snap.free.last().map_or(size.X, |f| f.1) - origin.X;
            let off = Vector2 { X: end - 40.0, Y: 4.0 };
            let center = Vector2 { X: end - 24.0, Y: size.Y / 2.0 };
            Ok(Some(Target { off, size: Vector2 { X: 32.0, Y: size.Y - 8.0 }, radius: 7.0, center, reach: 60.0, anchor: None }))
        }
    }
}

fn set_local(local: bool) -> R<()> {
    ALERT.with_borrow_mut(|s| -> R<()> {
        let Some(a) = s.as_mut() else { return Ok(()) };
        let changed = a.local != local;
        a.local = local;
        apply(a, if changed { MORPH_MS } else { 0 })
    })
}

/// Move the wash and the rings' origin to where the mode wants them, animated over `dur` ms.
fn apply(a: &mut Alert, dur: i64) -> R<()> {
    let Some(t) = target(a)? else {
        // The label was rebuilt and isn't placed yet; its size is known already, so the pill takes that now.
        if let Some(m) = crate::meters::mic_anchor().filter(|_| Instant::now() >= a.morph_until) {
            a.wash.StopAnimation(&HSTRING::from("Size")).map_err(w)?;
            a.wash.SetSize(Vector2 { X: m.size.X + 2.0 * PILL_PAD, Y: (m.size.Y - 2.0).max(8.0) }).map_err(w)?;
        }
        return Ok(());
    };
    let compositor = a.compositor.clone();
    let c = &compositor;
    let ease = || c.CreateCubicBezierEasingFunction(Vector2 { X: 0.2, Y: 0.0 }, Vector2 { X: 0.0, Y: 1.0 }).map_err(w);
    let glide = |obj: &CompositionObject, prop: &str, v: Vector2, dur: i64| -> R<()> {
        let anim = c.CreateVector2KeyFrameAnimation().map_err(w)?;
        anim.InsertKeyFrameWithEasingFunction(1.0, v, &ease()?).map_err(w)?;
        anim.SetDuration(ms(dur.max(1))).map_err(w)?;
        obj.StartAnimation(&HSTRING::from(prop), &anim).map_err(w)
    };
    let wash: CompositionObject = a.wash.cast().map_err(w)?;
    // The rings start over when the mode changes, or when how far they reach does by a fifth (the label's width
    // changes it a little with every name): the ones in flight finish their journey first, from where they are.
    let restart = a.local != a.focus_local || (a.focus.1 - t.reach).abs() > a.focus.1.max(1.0) / 5.0;
    if restart {
        a.near.retire(c)?;
        a.far.retire(c)?;
    }
    let centers: Vec<CompositionObject> = a.near.centers()?.into_iter().chain(a.far.centers()?).collect();
    // A plain set loses to an animation running on the property, so stop that first.
    let stop = |obj: &CompositionObject, prop: &str| obj.StopAnimation(&HSTRING::from(prop)).map_err(w);
    let now = Instant::now();
    let morphing = now < a.morph_until;
    if dur > 0 {
        a.morph_until = now + std::time::Duration::from_millis(dur as u64);
    }
    // What's left of a running morph, for what changes during it.
    let left = if dur > 0 { dur } else { a.morph_until.saturating_duration_since(now).as_millis() as i64 };
    let radius = Vector2 { X: t.radius, Y: t.radius };
    if dur > 0 || morphing {
        glide(&wash, "Size", t.size, left)?;
        glide(&wash, "CornerRadius", radius, left)?;
    } else {
        stop(&wash, "Size")?;
        stop(&wash, "CornerRadius")?;
        a.wash.SetSize(t.size).map_err(w)?;
        a.wash.SetCornerRadius(radius).map_err(w)?;
    }
    let props = &a.level;
    match &t.anchor {
        Some(anchor) => {
            // Offset = Lerp(where it was, the label's drawn position + ToOff, T): T runs 0 -> 1 through a morph and
            // stays at 1 after, so the wash and the rings ride along with the label as it glides.
            if dur > 0 || !a.anchored {
                props.InsertVector2(&HSTRING::from("FromOff"), a.last.0).map_err(w)?;
                props.InsertVector2(&HSTRING::from("FromCenter"), a.last.1).map_err(w)?;
                let anim = c.CreateScalarKeyFrameAnimation().map_err(w)?;
                anim.InsertKeyFrame(0.0, 0.0).map_err(w)?;
                anim.InsertKeyFrameWithEasingFunction(1.0, 1.0, &ease()?).map_err(w)?;
                anim.SetDuration(ms(left.max(1))).map_err(w)?;
                props.StartAnimation(&HSTRING::from("T"), &anim).map_err(w)?;
            }
            props.InsertVector2(&HSTRING::from("ToOff"), anchor.to_off).map_err(w)?;
            props.InsertVector2(&HSTRING::from("ToCenter"), anchor.to_center).map_err(w)?;
            let bind = |obj: &CompositionObject, prop: &str, from: &str, to: &str| -> R<()> {
                let label = "Vector2(pv.Offset.X + cv.Offset.X, pv.Offset.Y + cv.Offset.Y)";
                let e = c.CreateExpressionAnimationWithExpression(&HSTRING::from(format!("Lerp(p.{from}, {label} + p.{to}, p.T)"))).map_err(w)?;
                e.SetReferenceParameter(&HSTRING::from("p"), props).map_err(w)?;
                e.SetReferenceParameter(&HSTRING::from("pv"), &anchor.panel).map_err(w)?;
                e.SetReferenceParameter(&HSTRING::from("cv"), &anchor.cell).map_err(w)?;
                obj.StartAnimation(&HSTRING::from(prop), &e).map_err(w)
            };
            bind(&wash, "Offset", "FromOff", "ToOff")?;
            for o in &centers {
                bind(o, "Center", "FromCenter", "ToCenter")?;
            }
            a.anchored = true;
        }
        None => {
            if dur > 0 || morphing {
                glide(&wash, "Offset", t.off, left)?;
                for o in &centers {
                    glide(o, "Center", t.center, left)?;
                }
            } else {
                stop(&wash, "Offset")?;
                a.wash.SetOffset(t.off).map_err(w)?;
                for (o, r) in centers.iter().zip(a.near.rings().iter().chain(a.far.rings())) {
                    stop(o, "Center")?;
                    r.geo.SetCenter(t.center).map_err(w)?;
                }
            }
            a.anchored = false;
        }
    }
    a.last = (t.off, t.center);
    // The tide spans the whole taskbar and grows from its bottom edge.
    let (frame_size, _) = frame(&a.host)?;
    a.tide.SetSize(frame_size).map_err(w)?;
    a.tide.SetCenterPoint(windows_numerics::Vector3 { X: 0.0, Y: frame_size.Y, Z: 0.0 }).map_err(w)?;
    if restart {
        echo(a, t.reach)?;
        echo_far(a, (t.center.X.max(frame_size.X - t.center.X) + 30.0).max(t.reach))?;
    }
    a.focus = (t.center, t.reach);
    a.focus_local = a.local;
    Ok(())
}

/// The far rings, local phase only: two, staggered, each crossing the whole taskbar in
/// `FAR_RING_MS`, white at first and blushing pinkish-red as they go.
fn echo_far(a: &mut Alert, reach: f32) -> R<()> {
    if !a.local {
        return Ok(());
    }
    let white = |alpha: u8| Color { A: alpha, R: 0xFF, G: 0xFF, B: 0xFF };
    let blush = |alpha: u8| Color { A: alpha, ..BLUSH };
    let cycle = Cycle {
        period: FAR_RING_MS,
        grow: vec![(0.0, Vector2 { X: 6.0, Y: 6.0 }, None), (1.0, Vector2 { X: reach, Y: reach }, Some(Bezier(0.25, 0.4, 0.55, 1.0)))],
        fade: vec![(0.0, white(0), None), (0.06, white(0x58), None), (0.55, blush(0x48), None), (1.0, blush(0), None)],
    };
    a.far.start(&a.compositor, cycle)
}

/// Start the rings: each grows from a dot to `reach` while fading out, a third of a cycle apart.
fn echo(a: &mut Alert, reach: f32) -> R<()> {
    let white = |alpha: u8| Color { A: alpha, R: 0xFF, G: 0xFF, B: 0xFF };
    let cycle = Cycle {
        // Rings crossing the whole taskbar travel slower, so they don't look rushed.
        period: if a.local { RING_MS } else { RING_MS * 3 / 2 },
        grow: vec![(0.0, Vector2 { X: 4.0, Y: 4.0 }, None), (1.0, Vector2 { X: reach, Y: reach }, Some(Bezier(0.3, 0.6, 0.5, 1.0)))],
        fade: vec![(0.0, white(0), None), (0.08, white(RING_ALPHA), None), (1.0, white(0), None)],
    };
    a.near.start(&a.compositor, cycle)
}
/// Called after the meter strip is positioned: keep the local pill on the label.
pub fn follow() {
    ALERT.with(|cell| {
        let Ok(mut state) = cell.try_borrow_mut() else { return };
        if let Some(a) = state.as_mut().filter(|a| a.local)
            && let Err(e) = apply(a, 0) {
                crate::log(format!("alert follow: {e}"));
            }
    });
}

pub(crate) fn off(ctx: &Ctx) -> R<String> {
    let Some(a) = ALERT.with_borrow_mut(Option::take) else { return Ok("alert already off".into()) };
    stage::remove(ctx, &a.host)?;
    Ok("alert off".into())
}

fn lighten(c: u8) -> u8 {
    c + ((255 - c as u16) * 45 / 100) as u8
}

/// The flicker pattern: a fresh random brightness every 1/30 s, held (step easing), looping
/// over two seconds. Deterministic noise is plenty here.
fn noise(c: &Compositor, props: &CompositionPropertySet) -> R<()> {
    const STEPS: usize = 60;
    let anim = c.CreateScalarKeyFrameAnimation().map_err(w)?;
    let step = c.CreateStepEasingFunctionWithStepCount(1).map_err(w)?;
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    for i in 0..=STEPS {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let v = 0.25 + 0.75 * ((seed >> 40) as f32 / (1u64 << 24) as f32);
        anim.InsertKeyFrameWithEasingFunction(i as f32 / STEPS as f32, v, &step).map_err(w)?;
    }
    anim.SetDuration(ms(2000)).map_err(w)?;
    anim.SetIterationBehavior(AnimationIterationBehavior::Forever).map_err(w)?;
    props.StartAnimation(&HSTRING::from("Noise"), &anim).map_err(w)
}

/// Glide the flicker's strength to `v` over a few frames, so 15 Hz updates don't step.
fn set_level(v: f32, tide: f32) {
    ALERT.with_borrow(|s| {
        let Some(a) = s else { return };
        let (v, tide) = if a.muted { (0.0, 0.0) } else { (v, tide) };
        let r = (|| -> R<()> {
            for (name, value, dur) in [("Level", v, 70), ("Tide", tide, 140)] {
                // Paced, the values just step at the rate they come in (about 15 a second), well under any cap.
                if pace::on() {
                    a.level.StopAnimation(&HSTRING::from(name)).map_err(w)?;
                    a.level.InsertScalar(&HSTRING::from(name), value).map_err(w)?;
                    continue;
                }
                let anim = a.compositor.CreateScalarKeyFrameAnimation().map_err(w)?;
                anim.InsertKeyFrame(1.0, value).map_err(w)?;
                anim.SetDuration(ms(dur)).map_err(w)?;
                a.level.StartAnimation(&HSTRING::from(name), &anim).map_err(w)?;
            }
            Ok(())
        })();
        if let Err(e) = r {
            crate::log(format!("alert level: {e}"));
        }
    });
}
