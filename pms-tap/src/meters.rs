//! Meter strip on the stage: cells right-aligned in the free space next to the tray, the
//! "microphone in use" label last. Values come from the daemon; this side draws and fits.
//! UI thread only.
//!
//!   meters height compact|tall|full      cell height: 75%, 87.5% (default), 100% of the taskbar
//!   meters add key=value..               label= style=history|bar|pie|cores color= colors=a,b
//!                                        max= (history) topo= (cores) wide=<widest value text>
//!   meters sep                           group separator before the next meter
//!   meters push <i> <text> <values..>    history: value [max] | bar: fraction
//!                                        pie: fraction per slice | cores: user,kernel pairs
//!   meters presence <i> on|off           show, or slowly shrink away (N/A meters)
//!   meters mic <apps>|off                the "MICROPHONE IN USE BY" label
//!
//! Cores topology: cores separated by `,`, L3 groups by `/`, NUMA nodes by `|`; a core is its
//! thread count, prefixed `e` for an efficiency core.
//!
//! Fitting: when the strip doesn't fit its free interval it squeezes in stages, each only as far
//! as needed: spacing, then graphic widths (sparklines, gauges, core bars), then horizontal text
//! scale (only where the text is what makes a cell wide), and finally hides meters from the end.
//! Cells are rebuilt only when the fit changes; presence changes animate in place, and the other
//! cells glide to their new spots.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use pms_xaml::{
    ArcSegment, Canvas, ElementCompositionPreview, Ellipse, Geometry, LineSegment, Orientation, Path, PathFigure, PathGeometry,
    Rectangle, SolidColorBrush, StackPanel, SweepDirection, TextBlock, Thickness, UIElement, Visibility,
};
use windows::Foundation::{Point, Rect, Size, TimeSpan, TypedEventHandler};
use windows::UI::Color;
use windows::UI::Composition::{CompositionBatchCompletedEventArgs, CompositionBatchTypes, CompositionColorBrush, CompositionColorGradientStop, CompositionObject, CompositionShadow, Compositor, ContainerVisual, SpriteVisual, Visual};
use windows::UI::Core::{CoreDispatcherPriority, DispatchedHandler};
use windows::UI::Text::FontWeight;
use windows_core::{HSTRING, IInspectable, Interface};
use windows_numerics::{Vector2, Vector3};

use crate::backdrop::parse_color;
use crate::commands::R;
use crate::layout::{SNAPSHOT, Snapshot, bounds};
use crate::{CTX, Ctx, lock, stage};

const EDGE_MARGIN: f32 = 8.0;
const SPACING: (f64, f64) = (1.5, 5.0);
const GRAPH_SCALE_MIN: f64 = 0.55;
const TEXT_SCALE_MIN: f64 = 0.72;
const TEXT_PAD: f64 = 3.0;
const HISTORY_W: f64 = 56.0;
const HISTORY_MAX: usize = 40;
const BAR_PITCH: f64 = 2.4;
const GAUGE_W: f64 = 52.0;
const GAUGE_H: f64 = 3.0;
const VALUE_H: f64 = 17.0;
const PEAK_HOLD: Duration = Duration::from_secs(3);
const LEAVE_MS: i64 = 1400;
const APPEAR_MS: i64 = 450;
const GLIDE_MS: i64 = 350;

const TEXT: Color = Color { A: 0xFF, R: 0xFF, G: 0xFF, B: 0xFF };
const TEXT_DIM: Color = Color { A: 0xB0, R: 0xFF, G: 0xFF, B: 0xFF };
const TRACK: Color = Color { A: 0x22, R: 0xFF, G: 0xFF, B: 0xFF };
const TRACK_HOT: Color = Color { A: 0x50, R: 0xFF, G: 0xFF, B: 0xFF };
const GLOW: Color = Color { A: 0x1A, R: 0xFF, G: 0xFF, B: 0xFF };
const SEP: Color = Color { A: 0x30, R: 0xFF, G: 0xFF, B: 0xFF };
const MIC_CAPTION: Color = Color { A: 0xFF, R: 0xFF, G: 0x8A, B: 0x80 };

const CAPTION: (f64, u16) = (10.0, 400);
const VALUE: (f64, u16) = (13.0, 600);
const MIC_TITLE: &str = "MICROPHONE IN USE BY";

#[derive(Clone, Copy, PartialEq)]
enum Style {
    History,
    Bar,
    Pie,
    Cores,
    /// One bar per GPU engine that has shown activity (`meters bars` names them).
    Engines,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Presence {
    /// Added, no value yet: not shown.
    Pending,
    Shown,
    Leaving,
    Hidden,
}

struct Spec {
    label: String,
    style: Style,
    colors: Vec<Color>,
    max: Option<f64>,
    topo: String,
    /// Natural widths of the caption and of the widest expected value text.
    caption_w: f64,
    value_w: f64,
    /// Group separator before this meter.
    sep: bool,
    /// Show the value text (history always does).
    value: bool,
    tooltip: bool,
    /// Cores/engines bar width, DIPs (before squeezing); None = by thread count.
    barw: Option<f64>,
    /// Engines: one name per bar, for tooltips.
    labels: Vec<String>,
    /// Extra tooltip text from the daemon (`meters tip`).
    tip: String,
    /// Pie: label inside the pie.
    inside: Inside,
    /// Pie: how full its first slice gets before the resource is under pressure: the rim's glow swells from there.
    pressure: f64,
    /// Font family of the value text (monospaced by default, so digits don't jiggle).
    font: String,
    /// Bars: what each bar is at, for its tooltip (`meters bartips`).
    bar_tips: Vec<String>,
    /// Bars: the next set, waiting for the dropped bars to finish falling.
    pending_labels: Option<Vec<String>>,
    /// Whether the label shows beside the meter (it's in the tooltip either way).
    caption: bool,
    /// Right against the meter before it: no spacing between.
    snug: bool,
    /// GPUs labelled NV, RAD rather than by ticker.
    short_vendors: bool,
    /// How the bars glow.
    glow: GlowMode,
    /// GPU engines: each GPU labelled by its maker in place of the caption, its bars in the maker's colors.
    vendors: bool,
    /// ... and per GPU (topology node), its label, the label's natural width, and its maker if known.
    gpus: Vec<(String, f64, Option<Vendor>)>,
    /// ... and its power state, under the label ("P8"; empty where unknown: only NVIDIA's are known).
    gpu_states: Vec<String>,
    /// History: log time axis (recent samples wide, older ones compressed), reaching `span`
    /// samples back.
    log_time: bool,
    span: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Inside {
    Never,
    Always,
    /// When space runs short (a squeeze stage).
    Auto,
}

fn inside(m: &Spec, fit: &Fit) -> bool {
    m.style == Style::Pie && m.caption && (m.inside == Inside::Always || (m.inside == Inside::Auto && fit.pies_inside))
}

struct Meter {
    spec: Spec,
    presence: Presence,
    text: String,
    history: VecDeque<f64>,
    pushed_max: Option<f64>,
    values: Vec<f64>,
    recent: VecDeque<(Instant, f64)>,
    cell: Option<MeterCell>,
    /// By heat: each bar's heat and when it was last brought up to date. Kept here rather than in the cell, so a
    /// rebuilt strip doesn't cool every bar down.
    heat: RefCell<Vec<(f32, Option<Instant>)>>,
}

struct MeterCell {
    root: Canvas,
    /// Rounded backdrop that lights up while the pointer is over the meter.
    glow: Rectangle,
    value: TextBlock,
    parts: Parts,
    sep: Option<Rectangle>,
    /// Implicit offset animation attached (only once it has a position, or it glides in from 0).
    gliding: Cell<bool>,
    /// What the value block shows; setting the same text again still costs a layout pass.
    shown_text: RefCell<String>,
}

enum Parts {
    /// `buckets[k]`: how many samples back bar k (counted from the right) reaches.
    History { bars: Vec<SpriteVisual>, xs: Vec<(f64, f64)>, buckets: Vec<usize>, h: f64 },
    Bar { fill: Rectangle, peak: Rectangle, span: f64 },
    /// `glow`: the rim's, and the (opacity, blur radius) it was last given.
    Pie { wedges: Vec<Path>, d: f64, glow: Option<Glow>, lit: Cell<(f32, f32)> },
    Cores { bars: Vec<CoreBar>, states: Vec<Option<TextBlock>> },
}

struct CoreBar {
    user: SpriteVisual,
    kernel: SpriteVisual,
    /// The dim column behind the bar; brightens while hovered.
    track: SpriteVisual,
    /// The glows its fill casts (GPUs of a known maker, CPU threads), off until busy (or hot) enough: by level, one
    /// off the whole fill; by heat, user's and kernel's each off its own part, in its own color.
    glows: Vec<BarGlow>,
    /// The fill's colors at rest (user, kernel), and their brushes, which heat brightens.
    colors: (Color, Color),
    brushes: Option<(CompositionColorBrush, CompositionColorBrush)>,
    /// The heat step its fill was last brightened to.
    lit_fill: Cell<f32>,
    /// x range within the cell (for tooltips and drawing), and the column's bottom and height.
    span: (f64, f64),
    bottom: f64,
    height: f64,
}

// ---------------------------------------------------------------------------------------------
// Bars on the compositor
//
// Bars that change often are composition sprites sized directly, with every edge on a whole
// physical pixel. (Full-size XAML rectangles scaled down on the compositor get filtered like
// textures and bleed a pixel past their ends; fractional positions add stray edge pixels.)

thread_local! {
    /// Physical pixels per DIP (the taskbar's rasterization scale).
    static PX: Cell<f64> = const { Cell::new(1.0) };
}

/// Round a DIP coordinate to the nearest physical pixel.
fn snap(v: f64) -> f64 {
    let px = PX.get();
    (v * px).round() / px
}

/// A composition layer over a (otherwise empty) host Canvas covering the cell, placed where the
/// host sits in z-order: after whatever is behind the bars, before the text.
fn bar_layer(cell: &Canvas, width: f64, h: f64) -> R<ContainerVisual> {
    let host = Canvas::new().map_err(w)?;
    host.SetWidth(width).map_err(w)?;
    host.SetHeight(h).map_err(w)?;
    put(cell, &host, 0.0, 0.0)?;
    let c = visual(&host)?.Compositor().map_err(w)?;
    let layer = c.CreateContainerVisual().map_err(w)?;
    ElementCompositionPreview::SetElementChildVisual(&ui(&host)?, &layer).map_err(w)?;
    Ok(layer)
}

fn bar_sprite(layer: &ContainerVisual, color: Color) -> R<SpriteVisual> {
    let c = layer.Compositor().map_err(w)?;
    let s = c.CreateSpriteVisual().map_err(w)?;
    s.SetBrush(&c.CreateColorBrushWithColor(crate::themed(color)).map_err(w)?).map_err(w)?;
    layer.Children().map_err(w)?.InsertAtTop(&s).map_err(w)?;
    Ok(s)
}

/// Size a bar standing on `bottom`, `fraction` of `full` tall, pixel-exact.
fn set_bar(s: &SpriteVisual, (x0, x1): (f64, f64), bottom: f64, full: f64, fraction: f64) -> R<()> {
    let (x0, x1, bottom) = (snap(x0), snap(x1), snap(bottom));
    let height = snap(full * fraction.clamp(0.0, 1.0));
    s.SetOffset(Vector3 { X: x0 as f32, Y: (bottom - height) as f32, Z: 0.0 }).map_err(w)?;
    s.SetSize(Vector2 { X: (x1 - x0) as f32, Y: height as f32 }).map_err(w)
}

/// Log time axis: bar 0 (rightmost) is the newest sample; going left each bar covers more
/// samples (geometric growth), so `n` bars reach `span` samples back. Returns, for each bar from
/// the right, the exclusive sample-age bound it reaches. Linear (one sample per bar) if
/// `span <= n`.
fn log_buckets(n: usize, span: usize) -> Vec<usize> {
    if span <= n {
        return (1..=n).collect();
    }
    let reach = |r: f64| (0..n).map(|k| r.powi(k as i32)).sum::<f64>();
    let (mut lo, mut hi) = (1.0f64, 4.0f64);
    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        if reach(mid) < span as f64 { lo = mid } else { hi = mid }
    }
    let mut out = Vec::with_capacity(n);
    let (mut acc, mut prev) = (0.0, 0usize);
    for k in 0..n {
        acc += lo.powi(k as i32);
        let end = (acc.round() as usize).max(prev + 1);
        out.push(end);
        prev = end;
    }
    out
}

struct Mic {
    apps: String,
    /// Windows has the microphone muted: a mic-off glyph before the names.
    muted: bool,
    caption_w: f64,
    apps_w: f64,
    cell: Option<Canvas>,
}

/// How the strip is laid out right now.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Fit {
    h: f64,
    spacing: f64,
    graph: f64,
    text: f64,
    /// Meters hidden from the end because nothing else made it fit.
    dropped: usize,
    /// Pies with `inside=auto` carry their label inside.
    pies_inside: bool,
}

struct Strip {
    panel: StackPanel,
    meters: Vec<Meter>,
    mic: Option<Mic>,
    height_mode: f64,
    fit: Option<Fit>,
    pending_sep: bool,
    /// The panel glides when the strip's width changes (attached after its first placement).
    panel_glides: bool,
}

thread_local! {
    static STRIP: RefCell<Option<Strip>> = const { RefCell::new(None) };
}

fn w(e: windows_core::Error) -> String {
    e.to_string()
}

fn ui<T: Interface>(e: &T) -> R<UIElement> {
    e.cast().map_err(w)
}

fn brush(c: Color) -> R<SolidColorBrush> {
    SolidColorBrush::CreateInstanceWithColor(crate::themed(c)).map_err(w)
}

fn ms(n: i64) -> TimeSpan {
    TimeSpan { Duration: n * 10_000 }
}

fn visual(e: &impl Interface) -> R<Visual> {
    ElementCompositionPreview::GetElementVisual(&ui(e)?).map_err(w)
}

pub fn command(ctx: &Ctx, args: &[String]) -> R<String> {
    let arg = |i: usize| args.get(i).map(String::as_str);
    match arg(0) {
        Some("clear") | Some("off") => clear(ctx),
        Some("height") => {
            let mode = match arg(1) {
                Some("compact") => 0.75,
                Some("tall") | None => 0.875,
                Some("full") => 1.0,
                Some(other) => return Err(format!("unknown height `{other}` (compact|tall|full)")),
            };
            strip(ctx)?;
            with_strip(|s| s.height_mode = mode);
            refit(true);
            Ok(format!("meter height {}", arg(1).unwrap_or("tall")))
        }
        Some("add") => add(ctx, &args[1..]),
        Some("sep") => {
            strip(ctx)?;
            with_strip(|s| s.pending_sep = true);
            Ok("separator".into())
        }
        Some("push") => push(&args[1..]),
        Some("presence") => {
            let i: usize = arg(1).ok_or("missing <i>")?.parse().map_err(|e| format!("bad index: {e}"))?;
            presence(i, arg(2) == Some("on"))
        }
        Some("mic") => mic(ctx, &args[1..]),
        Some("micmute") => {
            let muted = arg(1) == Some("1");
            with_strip(|s| s.mic.as_mut().map(|m| m.muted = muted));
            remic();
            Ok(format!("mic muted: {muted}"))
        }
        // bars <i> <name>..: an engines meter's current bars (names as "GPU — engine").
        Some("bars") => {
            let i: usize = arg(1).ok_or("missing <i>")?.parse().map_err(|e| format!("bad index: {e}"))?;
            let labels: Vec<String> = args[2..].to_vec();
            // Bars that go away fall off first; the new set lands when they're gone.
            let falling = with_strip(|s| -> R<bool> {
                let m = s.meters.get_mut(i).ok_or_else(|| format!("no meter #{i}"))?;
                let already = m.spec.pending_labels.is_some();
                m.spec.pending_labels = Some(labels.clone());
                if already {
                    return Ok(true);
                }
                let gone: Vec<usize> = (0..m.spec.labels.len()).filter(|&j| !labels.contains(&m.spec.labels[j])).collect();
                Ok(!gone.is_empty() && drop_bars(m, &gone, i))
            })
            .ok_or("no meters")??;
            if !falling {
                land_bars(i);
            }
            Ok(format!("meter #{i}: {} bar(s)", labels.len()))
        }
        // gpustate <i> <state|->..: each GPU's power state, under its label.
        Some("gpustate") => {
            let i: usize = arg(1).ok_or("missing <i>")?.parse().map_err(|e| format!("bad index: {e}"))?;
            let states: Vec<String> = args[2..].iter().map(|s| if s == "-" { String::new() } else { s.clone() }).collect();
            // Whether a state shows or not changes the label's layout (one line or two); a state that changes
            // only changes its text.
            let relayout = with_strip(|s| -> R<bool> {
                let m = s.meters.get_mut(i).ok_or_else(|| format!("no meter #{i}"))?;
                let shown = |v: &[String]| v.iter().map(|s| !s.is_empty()).collect::<Vec<_>>();
                let relayout = shown(&m.spec.gpu_states) != shown(&states);
                m.spec.gpu_states = states.clone();
                if relayout {
                    // The label column is as wide as the widest of the maker and a state can be.
                    let state_w = text_block("P12", GPU_STATE_FONT.0, GPU_STATE_FONT.1, TEXT).and_then(|t| measure(&t)).unwrap_or(16.0);
                    for (j, g) in m.spec.gpus.iter_mut().enumerate() {
                        let ticker_w = text_block(&g.0, VENDOR_FONT.0, VENDOR_FONT.1, TEXT).and_then(|t| measure(&t)).unwrap_or(24.0);
                        g.1 = if states.get(j).is_some_and(|s| !s.is_empty()) { ticker_w.max(state_w) } else { ticker_w };
                    }
                } else if let Some(Parts::Cores { states: blocks, .. }) = m.cell.as_ref().map(|c| &c.parts) {
                    for (b, s) in blocks.iter().zip(&states) {
                        if let Some(b) = b {
                            b.SetText(&HSTRING::from(s)).map_err(w)?;
                        }
                    }
                }
                Ok(relayout)
            })
            .ok_or("no meters")??;
            if relayout {
                refit(true);
            }
            Ok(String::new())
        }
        // bartips <i> <text>..: what each bar of a bars meter is at, for its tooltip.
        Some("bartips") => {
            let i: usize = arg(1).ok_or("missing <i>")?.parse().map_err(|e| format!("bad index: {e}"))?;
            let tips = args[2..].to_vec();
            with_strip(|s| s.meters.get_mut(i).map(|m| m.spec.bar_tips = tips)).flatten().ok_or_else(|| format!("no meter #{i}"))?;
            crate::tooltip::refresh(i);
            Ok(String::new())
        }
        Some("tip") => {
            let i: usize = arg(1).ok_or("missing <i>")?.parse().map_err(|e| format!("bad index: {e}"))?;
            let tip = args[2..].join(" ");
            with_strip(|s| s.meters.get_mut(i).map(|m| m.spec.tip = tip)).flatten().ok_or_else(|| format!("no meter #{i}"))?;
            crate::tooltip::refresh(i);
            Ok(String::new())
        }
        Some("status") | None => Ok(match count() {
            0 => "no meters".into(),
            n => format!("{n} meter(s)"),
        }),
        Some(other) => Err(format!("unknown meters subcommand `{other}`")),
    }
}

pub fn count() -> usize {
    STRIP.with_borrow(|s| s.as_ref().map_or(0, |s| s.meters.len()))
}

fn with_strip<T>(f: impl FnOnce(&mut Strip) -> T) -> Option<T> {
    STRIP.with_borrow_mut(|s| s.as_mut().map(f))
}

fn clear(ctx: &Ctx) -> R<String> {
    STRIP.with_borrow_mut(|s| *s = None);
    stage::drop_stage(ctx)?;
    Ok("meters cleared".into())
}

fn strip(ctx: &Ctx) -> R<StackPanel> {
    if let Some(p) = STRIP.with_borrow(|s| s.as_ref().map(|s| s.panel.clone())) {
        return Ok(p);
    }
    crate::layout::ensure_tracking(ctx)?;
    let panel = StackPanel::new().map_err(w)?;
    panel.SetOrientation(Orientation::Horizontal).map_err(w)?;
    stage::stage(ctx)?.Children().map_err(w)?.Append(&ui(&panel)?).map_err(w)?;
    STRIP.with_borrow_mut(|s| *s = Some(Strip { panel: panel.clone(), meters: Vec::new(), mic: None, height_mode: 0.875, fit: None, pending_sep: false, panel_glides: false }));
    Ok(panel)
}

/// Offset changes (layout moving the element) animate instead of jumping.
fn glide_on_move(e: &UIElement) -> R<()> {
    let v = ElementCompositionPreview::GetElementVisual(e).map_err(w)?;
    let c = v.Compositor().map_err(w)?;
    let anim = c.CreateVector3KeyFrameAnimation().map_err(w)?;
    anim.InsertExpressionKeyFrame(1.0, &HSTRING::from("this.FinalValue")).map_err(w)?;
    anim.SetDuration(ms(GLIDE_MS)).map_err(w)?;
    anim.SetTarget(&HSTRING::from("Offset")).map_err(w)?;
    let set = c.CreateImplicitAnimationCollection().map_err(w)?;
    set.Insert(&HSTRING::from("Offset"), &anim).map_err(w)?;
    v.SetImplicitAnimations(&set).map_err(w)
}

fn text_block(s: &str, size: f64, weight: u16, color: Color) -> R<TextBlock> {
    let t = TextBlock::new().map_err(w)?;
    t.SetText(&HSTRING::from(s)).map_err(w)?;
    t.SetFontSize(size).map_err(w)?;
    t.SetFontWeight(FontWeight { Weight: weight }).map_err(w)?;
    t.SetForeground(&brush(color)?).map_err(w)?;
    Ok(t)
}

/// Natural width of a text block.
fn measure(t: &TextBlock) -> R<f64> {
    let e = ui(t)?;
    e.Measure(Size { Width: f32::INFINITY, Height: f32::INFINITY }).map_err(w)?;
    Ok(e.DesiredSize().map_err(w)?.Width as f64)
}

/// A value text block in the meter's value font.
fn value_block(s: &str, font: &str) -> R<TextBlock> {
    let t = text_block(s, VALUE.0, VALUE.1, TEXT)?;
    t.SetFontFamily(&pms_xaml::FontFamily::CreateInstanceWithName(&HSTRING::from(font)).map_err(w)?).map_err(w)?;
    Ok(t)
}

fn mic_caption() -> R<TextBlock> {
    let t = text_block(MIC_TITLE, 9.0, 700, MIC_CAPTION)?;
    t.SetCharacterSpacing(60).map_err(w)?;
    Ok(t)
}

fn add(ctx: &Ctx, args: &[String]) -> R<String> {
    strip(ctx)?;
    let mut kv = std::collections::HashMap::new();
    for a in args {
        let (k, v) = a.split_once('=').ok_or_else(|| format!("expected key=value, got `{a}`"))?;
        kv.insert(k, v);
    }
    let get = |k: &str| kv.get(k).copied().unwrap_or("");
    let style = match get("style") {
        "history" | "" => Style::History,
        "bar" => Style::Bar,
        "pie" => Style::Pie,
        "cores" => Style::Cores,
        "engines" | "bars" => Style::Engines,
        other => return Err(format!("unknown style `{other}`")),
    };
    let colors = get("colors").split(',').chain([get("color")]).filter(|c| !c.is_empty()).map(parse_color).collect::<R<Vec<_>>>()?;
    let mut colors = colors;
    if style == Style::Engines && colors.len() == 1 {
        colors.push(colors[0]);
    }
    if colors.is_empty() || (style == Style::Cores && colors.len() < 2) {
        return Err("missing color= / colors= (cores: colors=<user>,<kernel>)".into());
    }
    // A GPU engines meter labels each GPU by its maker instead of having a caption of its own.
    let vendors = get("style") == "engines" && get("novendors") != "1";
    let label = if vendors { String::new() } else { get("label").to_string() };
    let wide = if get("wide").is_empty() { "100%" } else { get("wide") };
    // nocaption=1: the label only in the tooltip.
    let caption = get("nocaption") != "1";
    let caption_w = if caption { measure(&text_block(&label, CAPTION.0, CAPTION.1, TEXT)?)? } else { 0.0 };
    let font = if get("font").is_empty() { "Cascadia Mono, Consolas".to_string() } else { get("font").to_string() };
    let value_w = measure(&value_block(wide, &font)?)?;
    let sep = with_strip(|s| std::mem::take(&mut s.pending_sep)).unwrap_or(false);
    let spec = Spec {
        label: label.clone(),
        style,
        colors,
        max: get("max").parse().ok(),
        topo: match (style, get("topo")) {
            (Style::Engines, _) => String::new(),
            (_, "") => "1".into(),
            (_, t) => t.into(),
        },
        caption_w,
        value_w,
        sep,
        value: style == Style::History || get("novalue") != "1",
        tooltip: get("notip") != "1",
        barw: get("barw").parse().ok(),
        pressure: get("pressure").parse::<f64>().ok().filter(|p| (0.0..1.0).contains(p)).unwrap_or(0.8),
        labels: Vec::new(),
        tip: String::new(),
        font,
        bar_tips: Vec::new(),
        pending_labels: None,
        caption,
        snug: get("snug") == "1",
        short_vendors: get("vendorlabel") == "short",
        glow: match get("glow") {
            "heat" => GlowMode::Heat,
            "off" => GlowMode::Off,
            _ => GlowMode::Level,
        },
        vendors,
        gpus: Vec::new(),
        gpu_states: Vec::new(),
        log_time: get("time") != "linear",
        span: get("span").parse().unwrap_or(300),
        inside: match get("inside") {
            "always" => Inside::Always,
            "auto" => Inside::Auto,
            _ => Inside::Never,
        },
    };
    let meter = Meter {
        spec,
        presence: Presence::Pending,
        text: String::new(),
        history: VecDeque::with_capacity(HISTORY_MAX),
        pushed_max: None,
        values: Vec::new(),
        recent: VecDeque::new(),
        cell: None,
        heat: RefCell::new(Vec::new()),
    };
    let n = with_strip(|s| {
        s.meters.push(meter);
        s.meters.len()
    })
    .unwrap_or(0);
    refit(true);
    Ok(format!("meter #{} `{label}` added", n - 1))
}

// ---------------------------------------------------------------------------------------------
// Fitting

/// Cores bars: (bar width, gaps between threads, cores, L3 groups, NUMA nodes) at a graphic scale.
fn core_geometry(threads: usize, barw: Option<f64>, graph: f64) -> (f64, [f64; 4]) {
    let base = barw.unwrap_or(match threads {
        0..=16 => 5.0,
        17..=32 => 3.4,
        33..=64 => 2.4,
        _ => 1.6,
    });
    // Whole physical pixels, so every gap of a kind is the same width on screen: one pixel between
    // the threads of a core, wider steps between cores, L3 groups and NUMA nodes.
    let px = PX.get();
    let bar = (base * graph * px).round().max(1.0);
    let core = (bar * 0.8).round().max(2.0);
    let l3 = (bar * 1.6).round().max(core + 1.0);
    let numa = (bar * 2.8).round().max(l3 + 1.0);
    (bar / px, [1.0 / px, core / px, l3 / px, numa / px])
}

fn threads(topo: &str) -> usize {
    topo.split(['|', '/', ',']).map(|c| c.trim_start_matches('e').parse::<usize>().unwrap_or(1)).sum()
}

fn cores_width(topo: &str, barw: Option<f64>, graph: f64) -> f64 {
    if topo.is_empty() {
        return 0.0;
    }
    let (bar, [t, c, l3, numa]) = core_geometry(threads(topo), barw, graph);
    let mut x = 0.0;
    for (ni, node) in topo.split('|').enumerate() {
        x += if ni > 0 { numa } else { 0.0 };
        for (li, l) in node.split('/').enumerate() {
            x += if li > 0 { l3 } else { 0.0 };
            for (ci, core) in l.split(',').enumerate() {
                let n = core.trim_start_matches('e').parse::<usize>().unwrap_or(1) as f64;
                x += if ci > 0 { c } else { 0.0 } + n * bar + (n - 1.0).max(0.0) * t;
            }
        }
    }
    x
}

fn pie_d(h: f64) -> f64 {
    (h * 0.72).round()
}

/// Width of a cell's text at the fit's horizontal text scale.
fn text_w(m: &Spec, fit: &Fit) -> f64 {
    let caption = if inside(m, fit) { 0.0 } else { m.caption_w };
    let value = if m.value { m.value_w } else { 0.0 };
    caption.max(value) * fit.text
}

/// A cell's width, rounded up to whole pixels so every cell (and what it draws) starts on one.
fn cell_width(m: &Spec, fit: &Fit) -> f64 {
    let px = PX.get();
    (raw_cell_width(m, fit) * px).ceil() / px
}

fn raw_cell_width(m: &Spec, fit: &Fit) -> f64 {
    let t = text_w(m, fit);
    match m.style {
        Style::History => (HISTORY_W * fit.graph).max(t) + 2.0 * TEXT_PAD,
        Style::Bar => (GAUGE_W * fit.graph).max(t) + 2.0 * TEXT_PAD,
        Style::Pie if t == 0.0 => TEXT_PAD + pie_d(fit.h) + TEXT_PAD,
        Style::Pie => TEXT_PAD + pie_d(fit.h) + 5.0 + t + TEXT_PAD,
        Style::Cores | Style::Engines => TEXT_PAD + t + 4.0 + gpu_labels_w(m, fit) + cores_width(&m.topo, m.barw, fit.graph) + TEXT_PAD,
    }
}

// ---------------------------------------------------------------------------------------------
// GPU makers

/// A GPU's maker (vendors.rs), its colors as XAML's.
#[derive(Clone, Copy)]
struct Vendor {
    ticker: &'static str,
    short: &'static str,
    bar: Color,
    glow: Color,
}

fn vendor(adapter: &str) -> Option<Vendor> {
    let rgb = |[r, g, b]: [u8; 3]| Color { A: 0xFF, R: r, G: g, B: b };
    crate::vendors::vendor(adapter).map(|v| Vendor { ticker: v.ticker, short: v.short, bar: rgb(v.bar), glow: rgb(v.glow) })
}

const VENDOR_FONT: (f64, u16) = (9.5, 600);
const GPU_STATE_FONT: (f64, u16) = (8.0, 400);
/// Between a GPU's label and its bars.
const VENDOR_GAP: f64 = 3.0;
/// Bars' glows: how far and how bright at their peak, and how far as they start. A GPU engine's glow comes in from
/// 20% busy and peaks past 80%; a CPU meter's, by its threads' average, from 30% to 85%.
const BAR_GLOW_PEAK: f32 = 0.95;
const BAR_GLOW_MIN_BLUR: f32 = 3.0;
const BAR_GLOW_PEAK_BLUR: f32 = 8.0;
const GPU_GLOW_FROM: f64 = 0.2;
const GPU_GLOW_FULL: f64 = 0.8;
const CPU_GLOW_FROM: f64 = 0.3;
const CPU_GLOW_FULL: f64 = 0.85;
/// By heat: seconds to (nearly) full heat at full load, and back to (nearly) cold idle; the heat at which the glow
/// starts (it peaks at full heat); how much heat brightens a fill, and how far toward white.
/// Glows and heated fills change in this many steps from nothing to full.
const GLOW_STEPS: f32 = 24.0;
const HEAT_RISE: f32 = 8.0;
const HEAT_FALL: f32 = 30.0;
const HEAT_GLOW_FROM: f64 = 0.15;
const HEAT_BRIGHTEN: f32 = 0.6;
const HEAT_WHITEN: f32 = 0.25;

/// The room the GPUs' labels take.
fn gpu_labels_w(m: &Spec, fit: &Fit) -> f64 {
    m.gpus.iter().map(|(_, w, _)| w * fit.text + VENDOR_GAP).sum()
}

fn sep_width(fit: &Fit) -> f64 {
    1.0 + 2.0 * snap(fit.spacing)
}

fn mic_width(m: &Mic, fit: &Fit) -> f64 {
    let glyph = if m.muted { MUTE_GLYPH_W } else { 0.0 };
    m.caption_w.max(m.apps_w + glyph) * fit.text + 2.0 * TEXT_PAD + 4.0
}

const MUTE_GLYPH_W: f64 = 17.0;

fn visible(p: Presence) -> bool {
    matches!(p, Presence::Shown | Presence::Leaving)
}

impl Strip {
    /// Indices of meters on screen under `fit` (present, and not dropped for space).
    fn shown(&self, fit: &Fit) -> Vec<usize> {
        let present: Vec<usize> = (0..self.meters.len()).filter(|&i| visible(self.meters[i].presence)).collect();
        present[..present.len().saturating_sub(fit.dropped)].to_vec()
    }

    /// Width of the strip under `fit`: cells, separators (which carry their own spacing on
    /// both sides), and spacing between two adjacent cells. Hidden meters take no room at all.
    fn total(&self, fit: &Fit) -> f64 {
        let shown = self.shown(fit);
        let mut total = 0.0;
        for (k, &i) in shown.iter().enumerate() {
            let sep = k > 0 && self.meters[i].spec.sep;
            total += if sep { sep_width(fit) } else if k > 0 { snap(fit.spacing) } else { 0.0 };
            total += cell_width(&self.meters[i].spec, fit);
        }
        if let Some(m) = &self.mic {
            if !shown.is_empty() {
                total += sep_width(fit);
            }
            total += mic_width(m, fit);
        }
        total
    }

    /// The least squeezing that fits `avail`: knobs first; if even fully squeezed it doesn't fit,
    /// drop one meter from the end and start the knobs over (dropping frees room to relax them).
    fn solve(&self, avail: f64, h: f64) -> Fit {
        let present = self.meters.iter().filter(|m| visible(m.presence)).count();
        let mut dropped = 0;
        loop {
            match self.squeeze_to(avail, h, dropped) {
                Ok(fit) => return fit,
                Err(fit) if dropped >= present => return fit,
                Err(_) => dropped += 1,
            }
        }
    }

    /// Squeeze stage by stage with `dropped` meters hidden: Ok(least squeezing that fits), or
    /// Err(fully squeezed) if nothing does.
    fn squeeze_to(&self, avail: f64, h: f64, dropped: usize) -> Result<Fit, Fit> {
        let mut fit = Fit { h, spacing: SPACING.1, graph: 1.0, text: 1.0, dropped, pies_inside: false };
        if self.total(&fit) <= avail {
            return Ok(fit);
        }
        // Each stage pulls one knob to its floor; the stage where it fits gets the largest value
        // that still fits (binary search), quantized so small wobbles don't rebuild the cells.
        type Knob = fn(&mut Fit, f64);
        let knob = |fit: &mut Fit, set: Knob, lo: f64, hi: f64| -> bool {
            set(fit, lo);
            if self.total(fit) > avail {
                return false;
            }
            let (mut lo, mut hi) = (lo, hi);
            for _ in 0..12 {
                let mid = (lo + hi) / 2.0;
                let mut f = *fit;
                set(&mut f, mid);
                if self.total(&f) <= avail { lo = mid } else { hi = mid }
            }
            set(fit, (lo * 50.0).floor() / 50.0);
            true
        };
        let spacing: Knob = |f, v| f.spacing = v;
        if knob(&mut fit, spacing, SPACING.0, SPACING.1) {
            return Ok(fit);
        }
        // Next: labels into the pies that allow it (and spacing back up, if that frees enough).
        if self.meters.iter().any(|m| m.spec.style == Style::Pie && m.spec.inside == Inside::Auto) {
            fit.pies_inside = true;
            if knob(&mut fit, spacing, SPACING.0, SPACING.1) {
                return Ok(fit);
            }
        }
        if knob(&mut fit, |f, v| f.graph = v, GRAPH_SCALE_MIN, 1.0) || knob(&mut fit, |f, v| f.text = v, TEXT_SCALE_MIN, 1.0) {
            return Ok(fit);
        }
        Err(fit)
    }
}

fn target(snap: &Snapshot, s: &Strip) -> (f64, f64) {
    let avail = snap.free.last().map_or(0.0, |&(a, b)| (b - a - 2.0 * EDGE_MARGIN) as f64);
    (avail, (snap.height.max(32.0) as f64 * s.height_mode).round())
}

/// Re-solve the fit for the current free space; rebuild the cells if it changed (or `force`).
fn refit(force: bool) {
    let snap = lock(&SNAPSHOT).clone();
    STRIP.with(|cell| {
        let Ok(mut state) = cell.try_borrow_mut() else { return };
        let Some(s) = state.as_mut() else { return };
        let (avail, h) = target(&snap, s);
        let fit = s.solve(avail, h);
        if let Ok(scale) = s.panel.XamlRoot().and_then(|r| r.RasterizationScale()) {
            PX.set(scale.max(0.5));
        }
        if !force && s.fit == Some(fit) {
            return;
        }
        s.fit = Some(fit);
        if let Err(e) = rebuild(s, fit) {
            crate::log(format!("meters rebuild: {e}"));
        }
    });
    position(&snap);
}

// ---------------------------------------------------------------------------------------------
// Cells

fn rect(color: Color, width: f64, height: f64) -> R<Rectangle> {
    let r = Rectangle::new().map_err(w)?;
    r.SetWidth(width).map_err(w)?;
    r.SetHeight(height).map_err(w)?;
    r.SetFill(&brush(color)?).map_err(w)?;
    Ok(r)
}

fn put(canvas: &Canvas, e: &impl Interface, x: f64, y: f64) -> R<()> {
    let e = ui(e)?;
    Canvas::SetLeft(&e, x).map_err(w)?;
    Canvas::SetTop(&e, y).map_err(w)?;
    canvas.Children().map_err(w)?.Append(&e).map_err(w)
}

/// Squeeze text horizontally around its left edge. Layout is unaffected and the cell's Canvas
/// doesn't clip, so the text keeps its full natural width to draw into.
fn squeeze(t: &TextBlock, scale: f64) -> R<()> {
    visual(t)?.SetScale(Vector3 { X: scale as f32, Y: 1.0, Z: 1.0 }).map_err(w)
}

fn build_cell(m: &Spec, fit: &Fit) -> R<MeterCell> {
    let h = fit.h;
    let width = cell_width(m, fit);
    let root = Canvas::new().map_err(w)?;
    root.SetWidth(width).map_err(w)?;
    root.SetHeight(h).map_err(w)?;
    let glow = rect(GLOW, width, h)?;
    glow.SetRadiusX(4.0).map_err(w)?;
    glow.SetRadiusY(4.0).map_err(w)?;
    glow.SetOpacity(0.0).map_err(w)?;
    put(&root, &glow, 0.0, 0.0)?;

    let caption = text_block(if m.caption { &m.label } else { "" }, CAPTION.0, CAPTION.1, TEXT_DIM)?;
    let value = value_block("", &m.font)?;
    let pad_y = if h >= 44.0 { 2.0 } else { 0.0 };
    // Overlaid text squeezes only when it, not the graphic, is what makes the cell wide.
    let natural = if m.value { m.caption_w.max(m.value_w) } else { m.caption_w };
    let squeeze_if_wider = |graphic: f64| if natural > graphic { fit.text } else { 1.0 };

    let (parts, text_x, text_scale, value_lift) = match m.style {
        Style::History => {
            let n = ((HISTORY_W * fit.graph) / BAR_PITCH).floor().max(4.0) as usize;
            let layer = bar_layer(&root, width, h)?;
            layer.SetOpacity(0.55).map_err(w)?;
            let x0 = width - TEXT_PAD - n as f64 * BAR_PITCH;
            let mut bars = Vec::with_capacity(n);
            let mut xs = Vec::with_capacity(n);
            for j in 0..n {
                bars.push(bar_sprite(&layer, m.colors[0])?);
                // A pixel gap between bars, at any scale.
                let x = x0 + j as f64 * BAR_PITCH;
                xs.push((x, (snap(x + BAR_PITCH) - 1.0 / PX.get()).max(snap(x) + 1.0 / PX.get())));
            }
            let buckets = if m.log_time { log_buckets(n, m.span.max(n)) } else { (1..=n).collect() };
            (Parts::History { bars, xs, buckets, h }, TEXT_PAD, squeeze_if_wider(HISTORY_W * fit.graph), 0.0)
        }
        Style::Bar => {
            let span = width - 2.0 * TEXT_PAD;
            let y = h - GAUGE_H - 2.0 - pad_y;
            put(&root, &rect(TRACK, span, GAUGE_H)?, TEXT_PAD, y)?;
            let fill = rect(m.colors[0], 0.0, GAUGE_H)?;
            put(&root, &fill, TEXT_PAD, y)?;
            let peak = rect(m.colors[0], 1.5, GAUGE_H + 2.0)?;
            peak.SetOpacity(0.8).map_err(w)?;
            put(&root, &peak, TEXT_PAD, y - 1.0)?;
            (Parts::Bar { fill, peak, span }, TEXT_PAD, squeeze_if_wider(GAUGE_W * fit.graph), GAUGE_H + 2.0)
        }
        Style::Pie => {
            let d = pie_d(h);
            let pie = Canvas::new().map_err(w)?;
            let track = Ellipse::new().map_err(w)?;
            track.SetWidth(d).map_err(w)?;
            track.SetHeight(d).map_err(w)?;
            track.SetFill(&brush(TRACK)?).map_err(w)?;
            pie.Children().map_err(w)?.Append(&ui(&track)?).map_err(w)?;
            let mut wedges = Vec::new();
            for c in &m.colors {
                let p = Path::new().map_err(w)?;
                p.SetFill(&brush(*c)?).map_err(w)?;
                p.SetOpacity(0.85).map_err(w)?;
                pie.Children().map_err(w)?.Append(&ui(&p)?).map_err(w)?;
                wedges.push(p);
            }
            let glow = pie_rim(&pie, d, m.colors[0])?;
            put(&root, &pie, TEXT_PAD, ((h - d) / 2.0).round())?;
            (Parts::Pie { wedges, d, glow, lit: Cell::new((-1.0, -1.0)) }, TEXT_PAD + d + 5.0, fit.text, 0.0)
        }
        Style::Cores | Style::Engines => {
            let x0 = TEXT_PAD + text_w(m, fit) + 4.0;
            let (bars, states) = core_bars(&root, m, x0, h, fit, pad_y)?;
            (Parts::Cores { bars, states }, TEXT_PAD, fit.text, 0.0)
        }
    };
    if inside(m, fit) {
        // Label inside the pie, just below its center, squeezed to the pie's width there.
        let d = pie_d(h);
        let label = text_block(&m.label, 8.5, 400, TEXT)?;
        label.SetOpacity(0.7).map_err(w)?;
        let natural = measure(&label)?;
        let scale = (d * 0.8 / natural).min(1.0);
        let pie_top = ((h - d) / 2.0).round();
        put_text(&root, &label, TEXT_PAD + (d - natural * scale) / 2.0, pie_top + d / 2.0 + 0.5, scale, None)?;
        if m.value {
            put_text(&root, &value, text_x, ((h - VALUE_H) / 2.0).round(), text_scale, Some(m.value_w))?;
        }
    } else if m.value {
        put_text(&root, &caption, text_x, pad_y, text_scale, None)?;
        put_text(&root, &value, text_x, h - VALUE_H - pad_y - value_lift, text_scale, Some(m.value_w))?;
    } else {
        // Caption alone: centered beside the graphic, or just above a gauge.
        let y = if m.style == Style::Bar { h - value_lift - pad_y - 15.0 } else { ((h - 13.0) / 2.0).round() };
        put_text(&root, &caption, text_x, y, text_scale, None)?;
    }
    Ok(MeterCell { root, glow, value, parts, sep: None, gliding: Cell::new(false), shown_text: RefCell::new(String::new()) })
}

/// The bars, and each GPU's power-state text (where it has one), to update as it changes.
fn core_bars(canvas: &Canvas, m: &Spec, x0: f64, h: f64, fit: &Fit, pad_y: f64) -> R<(Vec<CoreBar>, Vec<Option<TextBlock>>)> {
    // Start on a pixel; the geometry is whole pixels from there.
    let x0 = snap(x0);
    let (bar, [tg, cg, lg, ng]) = core_geometry(threads(&m.topo), m.barw, fit.graph);
    if m.topo.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let full = h - 4.0 - 2.0 * pad_y;
    let bottom = 2.0 + pad_y + full;
    let mut x = x0;
    // Columns first (x, height, which node), then their tracks, then the bar layer above the tracks.
    let mut columns: Vec<(f64, f64, usize)> = Vec::new();
    let mut states = Vec::new();
    for (ni, node) in m.topo.split('|').enumerate() {
        x += if ni > 0 { ng } else { 0.0 };
        // A GPU's maker, left of its bars.
        if let Some((ticker, natural, v)) = m.gpus.get(ni) {
            let label = text_block(ticker, VENDOR_FONT.0, VENDOR_FONT.1, v.map_or(TEXT_DIM, |v| Color { A: 0xE0, ..v.bar }))?;
            // With its power state known, the maker sits higher, the state small beneath it.
            match m.gpu_states.get(ni).filter(|s| !s.is_empty()) {
                Some(state) => {
                    put_text(canvas, &label, x, (h / 2.0 - 13.0).round(), fit.text, None)?;
                    let t = text_block(state, GPU_STATE_FONT.0, GPU_STATE_FONT.1, TEXT_DIM)?;
                    put_text(canvas, &t, x, (h / 2.0 - 0.5).round(), fit.text, Some(*natural))?;
                    states.push(Some(t));
                }
                None => {
                    put_text(canvas, &label, x, ((h - 13.0) / 2.0).round(), fit.text, None)?;
                    states.push(None);
                }
            }
            x = snap(x + natural * fit.text + VENDOR_GAP);
        }
        for (li, l3) in node.split('/').enumerate() {
            x += if li > 0 { lg } else { 0.0 };
            for (ci, core) in l3.split(',').enumerate() {
                x += if ci > 0 { cg } else { 0.0 };
                let n: usize = core.trim_start_matches('e').parse().map_err(|_| format!("bad core `{core}` in topology"))?;
                // Efficiency cores: a little shorter.
                let height = if core.starts_with('e') { full * 0.78 } else { full };
                for t in 0..n {
                    x += if t > 0 { tg } else { 0.0 };
                    columns.push((x, height, ni));
                    x += bar;
                }
            }
        }
    }
    // Track, kernel and user all on one layer, so they snap to the same pixels.
    let width = canvas.Width().map_err(w)?;
    let layer = bar_layer(canvas, width, h)?;
    let mut bars = Vec::new();
    for &(x, height, ni) in &columns {
        let span = (snap(x), snap(x + bar).max(snap(x) + 1.0 / PX.get()));
        let track = bar_sprite(&layer, TRACK)?;
        set_bar(&track, span, bottom, height, 1.0)?;
        // Cores: user and kernel colors. Engines/bars: one color per bar, cycling.
        let (uc, kc) = match (m.style, m.gpus.get(ni).and_then(|g| g.2)) {
            (Style::Engines, Some(v)) => (v.bar, v.bar),
            (Style::Engines, None) => (m.colors[bars.len() % m.colors.len()], m.colors[0]),
            _ => (m.colors[0], m.colors[1]),
        };
        // Kernel behind user: by level, as tall as both; by heat, just its own part above user's.
        let kernel = bar_sprite(&layer, kc)?;
        let user = bar_sprite(&layer, uc)?;
        // The glows, soft and without offset, so they show around the bar, not over it. A GPU's in its maker's
        // glow color; a CPU thread's in user's color (by level, set as the meter runs) and by heat kernel's too.
        let maker = m.gpus.get(ni).and_then(|g| g.2);
        let c = layer.Compositor().map_err(w)?;
        let glows = match (m.glow, m.style, maker) {
            (GlowMode::Off, _, _) | (_, Style::Engines, None) | (_, Style::History | Style::Bar | Style::Pie, _) => Vec::new(),
            (GlowMode::Level, Style::Engines, Some(v)) => vec![BarGlow::new(&c, &kernel, v.glow)?],
            (GlowMode::Level, _, _) => vec![BarGlow::new(&c, &kernel, m.colors[0])?],
            (GlowMode::Heat, Style::Engines, Some(v)) => vec![BarGlow::new(&c, &user, v.glow)?],
            (GlowMode::Heat, _, _) => vec![BarGlow::new(&c, &user, m.colors[0])?, BarGlow::new(&c, &kernel, m.colors[1])?],
        };
        let brush = |s: &SpriteVisual| s.Brush().ok().and_then(|b| b.cast::<CompositionColorBrush>().ok());
        let brushes = brush(&user).zip(brush(&kernel));
        bars.push(CoreBar { user, kernel, track, glows, colors: (uc, kc), brushes, lit_fill: Cell::new(-1.0), span, bottom, height });
    }
    Ok((bars, states))
}

/// How a meter's bars glow: by how busy they are now; by how hot they've got (busy for a while); or not.
#[derive(Clone, Copy, PartialEq)]
enum GlowMode {
    Level,
    Heat,
    Off,
}

/// A glow off one sprite. The shadow is only on the sprite while it shows: the compositor draws a blur for every
/// shadow there is, however faint, so a quiet bar costs nothing.
struct BarGlow {
    sprite: SpriteVisual,
    shadow: windows::UI::Composition::DropShadow,
    on: Cell<bool>,
    /// The strength it was last given, in steps of 1/GLOW_STEPS: it's only set again when that changes, as every
    /// change has the compositor draw the blur again.
    lit: Cell<i32>,
}

impl BarGlow {
    fn new(c: &Compositor, sprite: &SpriteVisual, color: Color) -> R<BarGlow> {
        let shadow = c.CreateDropShadow().map_err(w)?;
        shadow.SetOffset(Vector3::default()).map_err(w)?;
        shadow.SetColor(Color { A: 0xFF, ..color }).map_err(w)?;
        Ok(BarGlow { sprite: sprite.clone(), shadow, on: Cell::new(false), lit: Cell::new(-1) })
    }

    /// `t` (0..1) of the way to its peak: brighter and further.
    fn set(&self, t: f32) -> R<()> {
        let step = (t * GLOW_STEPS).round() as i32;
        if self.lit.replace(step) == step {
            return Ok(());
        }
        let t = step as f32 / GLOW_STEPS;
        if step == 0 {
            if self.on.replace(false) {
                self.sprite.SetShadow(None::<&CompositionShadow>).map_err(w)?;
            }
            return Ok(());
        }
        if !self.on.replace(true) {
            self.sprite.SetShadow(&self.shadow).map_err(w)?;
        }
        set_glow(&self.shadow, t)
    }
}

/// A bar's heat `dt` seconds on, at `load`: it rises toward the load - nearly all the way (95%) in HEAT_RISE seconds -
/// and cools toward it more slowly, nearly all the way in HEAT_FALL. So the longer and the harder a bar works, the
/// hotter it gets, and it stays warm for a while after.
fn heat_up(heat: f32, load: f32, dt: f32) -> f32 {
    let tau = if load > heat { HEAT_RISE / 3.0 } else { HEAT_FALL / 3.0 };
    heat + (load - heat) * (1.0 - (-dt / tau).exp())
}

/// A fill color, hotter: brighter (scaled up, clipping at full) and a little toward white.
fn hotter(c: Color, heat: f32) -> Color {
    let up = |v: u8| ((v as f32 * (1.0 + HEAT_BRIGHTEN * heat)).min(255.0)) as u8;
    let scaled = Color { A: c.A, R: up(c.R), G: up(c.G), B: up(c.B) };
    let k = HEAT_WHITEN * heat;
    let to_white = |v: u8| (v as f32 + (255.0 - v as f32) * k).round() as u8;
    Color { A: c.A, R: to_white(scaled.R), G: to_white(scaled.G), B: to_white(scaled.B) }
}

/// Fill `s` from `from` to `to` (fractions of `full`, from the bottom), on the same pixels set_bar would use.
fn set_segment(s: &SpriteVisual, (x0, x1): (f64, f64), bottom: f64, full: f64, from: f64, to: f64) -> R<()> {
    let (x0, x1, bottom) = (snap(x0), snap(x1), snap(bottom));
    let (lo, hi) = (snap(full * from.clamp(0.0, 1.0)), snap(full * to.clamp(0.0, 1.0)));
    s.SetOffset(Vector3 { X: x0 as f32, Y: (bottom - hi) as f32, Z: 0.0 }).map_err(w)?;
    s.SetSize(Vector2 { X: (x1 - x0) as f32, Y: (hi - lo).max(0.0) as f32 }).map_err(w)
}

/// 0 up to `lo`, 1 from `hi`, smooth between.
fn ramp(v: f64, lo: f64, hi: f64) -> f32 {
    let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
    (t * t * (3.0 - 2.0 * t)) as f32
}

/// A bar's glow `t` (0..1) of the way to its peak: brighter and further.
fn set_glow(glow: &windows::UI::Composition::DropShadow, t: f32) -> R<()> {
    glow.SetOpacity(BAR_GLOW_PEAK * t).map_err(w)?;
    glow.SetBlurRadius(BAR_GLOW_MIN_BLUR + (BAR_GLOW_PEAK_BLUR - BAR_GLOW_MIN_BLUR) * t).map_err(w)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let c = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color { A: 0xFF, R: c(a.R, b.R), G: c(a.G, b.G), B: c(a.B, b.B) }
}

fn sep_cell(fit: &Fit) -> R<Rectangle> {
    let r = rect(SEP, 1.0, (fit.h * 0.5).round())?;
    r.SetMargin(Thickness { Left: snap(fit.spacing), Top: 0.0, Right: snap(fit.spacing), Bottom: 0.0 }).map_err(w)?;
    Ok(r)
}

fn build_mic(m: &Mic, fit: &Fit) -> R<Canvas> {
    let root = Canvas::new().map_err(w)?;
    root.SetWidth(mic_width(m, fit)).map_err(w)?;
    root.SetHeight(fit.h).map_err(w)?;
    let pad_y = if fit.h >= 44.0 { 3.0 } else { 1.0 };
    let caption = mic_caption()?;
    if m.muted {
        caption.SetForeground(&brush(TEXT_DIM)?).map_err(w)?;
    }
    let apps = text_block(&m.apps, 12.5, 600, TEXT)?;
    put_text(&root, &caption, TEXT_PAD + 2.0, pad_y + 2.0, fit.text, None)?;
    let mut x = TEXT_PAD + 2.0;
    if m.muted {
        let glyph = text_block("\u{EC54}", 13.0, 400, MIC_CAPTION)?;
        glyph.SetFontFamily(&pms_xaml::FontFamily::CreateInstanceWithName(&HSTRING::from("Segoe Fluent Icons, Segoe MDL2 Assets")).map_err(w)?).map_err(w)?;
        put(&root, &glyph, x, fit.h - 17.0 - pad_y)?;
        x += MUTE_GLYPH_W * fit.text;
    }
    put_text(&root, &apps, x, fit.h - 18.0 - pad_y, fit.text, None)?;
    Ok(root)
}

/// Throw the cells away and build them again for `fit`, restoring their last values.
fn rebuild(s: &mut Strip, fit: Fit) -> R<()> {
    HIGHLIGHT.set(None);
    NEEDS_POSITION.set(true);
    let children = s.panel.Children().map_err(w)?;
    children.Clear().map_err(w)?;
    s.panel.SetSpacing(0.0).map_err(w)?;
    let shown = s.shown(&fit);
    for i in 0..s.meters.len() {
        let m = &mut s.meters[i];
        let mut cell = build_cell(&m.spec, &fit)?;
        let on = if shown.contains(&i) { Visibility::Visible } else { Visibility::Collapsed };
        if m.spec.sep && i > 0 {
            let sep = sep_cell(&fit)?;
            children.Append(&ui(&sep)?).map_err(w)?;
            cell.sep = Some(sep);
        }
        cell.root.SetVisibility(on).map_err(w)?;
        children.Append(&ui(&cell.root)?).map_err(w)?;
        m.cell = Some(cell);
        if !m.text.is_empty() {
            draw(m)?;
        }
    }
    if let Some(mic) = s.mic.as_mut() {
        if !shown.is_empty() {
            children.Append(&ui(&sep_cell(&fit)?)?).map_err(w)?;
        }
        let c = build_mic(mic, &fit)?;
        children.Append(&ui(&c)?).map_err(w)?;
        mic.cell = Some(c);
    }
    cast_shadows();
    sync_seps(s)
}

/// A separator shows only between two meters that are both on screen.
fn sync_seps(s: &Strip) -> R<()> {
    let Some(fit) = s.fit else { return Ok(()) };
    let shown = s.shown(&fit);
    // Spacing goes on cells as a left margin, only after another visible cell; hidden meters
    // then leave no gap behind (StackPanel spacing would count them).
    let mut any = false;
    for (i, m) in s.meters.iter().enumerate() {
        let Some(c) = m.cell.as_ref() else { continue };
        let on = shown.contains(&i);
        let sep_on = on && any && c.sep.is_some();
        if let Some(sep) = &c.sep {
            sep.SetVisibility(if sep_on { Visibility::Visible } else { Visibility::Collapsed }).map_err(w)?;
        }
        let left = if on && any && !sep_on && !m.spec.snug { snap(fit.spacing) } else { 0.0 };
        c.root.SetMargin(Thickness { Left: left, Top: 0.0, Right: 0.0, Bottom: 0.0 }).map_err(w)?;
        any |= on;
    }
    Ok(())
}

/// A pie slice from fraction `a` to `b` of the way round, clockwise from 12 o'clock.
fn wedge(a: f64, b: f64, d: f64) -> R<Geometry> {
    let r = d / 2.0;
    let point = |f: f64| {
        let t = f * std::f64::consts::TAU;
        Point { X: (r + r * t.sin()) as f32, Y: (r - r * t.cos()) as f32 }
    };
    let fig = PathFigure::new().map_err(w)?;
    fig.SetStartPoint(Point { X: r as f32, Y: r as f32 }).map_err(w)?;
    fig.SetIsClosed(true).map_err(w)?;
    let segs = fig.Segments().map_err(w)?;
    let line = LineSegment::new().map_err(w)?;
    line.SetPoint(point(a)).map_err(w)?;
    segs.Append(&line.cast::<pms_xaml::PathSegment>().map_err(w)?).map_err(w)?;
    // An arc can't end where it starts; a full turn is two half turns.
    let mut from = a;
    while from < b - 1e-6 {
        let to = b.min(from + 0.5);
        let arc = ArcSegment::new().map_err(w)?;
        arc.SetPoint(point(to)).map_err(w)?;
        arc.SetSize(Size { Width: r as f32, Height: r as f32 }).map_err(w)?;
        arc.SetSweepDirection(SweepDirection::Clockwise).map_err(w)?;
        arc.SetIsLargeArc(to - from > 0.5).map_err(w)?;
        segs.Append(&arc.cast::<pms_xaml::PathSegment>().map_err(w)?).map_err(w)?;
        from = to;
    }
    let geo = PathGeometry::new().map_err(w)?;
    geo.Figures().map_err(w)?.Append(&fig).map_err(w)?;
    geo.cast().map_err(w)
}

/// Paint the meter's current data into its cell.
fn draw(m: &Meter) -> R<()> {
    let Some(cell) = &m.cell else { return Ok(()) };
    if *cell.shown_text.borrow() != m.text {
        cell.value.SetText(&HSTRING::from(m.text.as_str())).map_err(w)?;
        *cell.shown_text.borrow_mut() = m.text.clone();
    }
    match &cell.parts {
        Parts::History { bars, xs, buckets, h } => {
            let scale = m.pushed_max.or(m.spec.max).unwrap_or_else(|| m.history.iter().copied().fold(0.0, f64::max) * 1.1).max(1e-9);
            // Bar k from the right shows the peak of samples [buckets[k-1], buckets[k]) old, so a
            // spike stays visible as it ages into the compressed past.
            let n = bars.len();
            let age = |a: usize| m.history.len().checked_sub(a + 1).map(|i| m.history[i]);
            let mut from = 0;
            for (k, &to) in buckets.iter().enumerate() {
                let peak = (from..to).filter_map(age).fold(0.0, f64::max);
                from = to;
                let j = n - 1 - k;
                set_bar(&bars[j], xs[j], *h, *h, peak / scale)?;
            }
        }
        Parts::Bar { fill, peak, span } => {
            let f = m.values.first().copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let top = m.recent.iter().map(|(_, f)| *f).fold(0.0, f64::max);
            fill.SetWidth(f * span).map_err(w)?;
            Canvas::SetLeft(&ui(peak)?, TEXT_PAD + (top * span - 1.5).max(0.0)).map_err(w)?;
            peak.SetVisibility(if top > f + 0.005 { Visibility::Visible } else { Visibility::Collapsed }).map_err(w)?;
        }
        Parts::Pie { wedges, d, glow, lit } => {
            if let Some(g) = glow {
                pie_glow(g, lit, m.values.first().copied().unwrap_or(0.0), m.spec.pressure)?;
            }
            let mut from = 0.0;
            for (j, p) in wedges.iter().enumerate() {
                let to = (from + m.values.get(j).copied().unwrap_or(0.0).clamp(0.0, 1.0)).min(1.0);
                if to - from < 1e-4 {
                    p.SetVisibility(Visibility::Collapsed).map_err(w)?;
                } else {
                    p.SetData(&wedge(from, to, *d)?).map_err(w)?;
                    p.SetVisibility(Visibility::Visible).map_err(w)?;
                }
                from = to;
            }
        }
        Parts::Cores { bars, .. } => {
            let v = |k: usize| m.values.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let engines = m.spec.style == Style::Engines;
            let heat = m.spec.glow == GlowMode::Heat;
            let now = Instant::now();
            let mut heats = m.heat.borrow_mut();
            heats.resize(bars.len(), (0.0, None));
            let (mut user, mut kernel) = (0.0, 0.0);
            for (j, b) in bars.iter().enumerate() {
                let (u, k) = if engines { (v(j), 0.0) } else { (v(2 * j), v(2 * j + 1)) };
                set_bar(&b.user, b.span, b.bottom, b.height, u)?;
                if heat {
                    set_segment(&b.kernel, b.span, b.bottom, b.height, u, (u + k).min(1.0))?;
                } else {
                    set_bar(&b.kernel, b.span, b.bottom, b.height, (u + k).min(1.0))?;
                }
                (user, kernel) = (user + u, kernel + k);
                if heat {
                    // Each bar by its own heat: it glows (user's part and kernel's each in its color) and its fill
                    // burns brighter.
                    let (was, at) = heats[j];
                    let dt = at.map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(5.0));
                    let h = heat_up(was, (u + k).min(1.0) as f32, dt);
                    heats[j] = (h, Some(now));
                    let t = ramp(h as f64, HEAT_GLOW_FROM, 1.0);
                    for g in &b.glows {
                        g.set(t)?;
                    }
                    // The fill brightens in steps too, and only when it moves to another.
                    let step = (h * GLOW_STEPS).round();
                    if let Some((ub, kb)) = b.brushes.as_ref().filter(|_| b.lit_fill.replace(step) != step) {
                        ub.SetColor(crate::themed(hotter(b.colors.0, step / GLOW_STEPS))).map_err(w)?;
                        kb.SetColor(crate::themed(hotter(b.colors.1, step / GLOW_STEPS))).map_err(w)?;
                    }
                } else if engines && let Some(g) = b.glows.first() {
                    // By level, a GPU's engines glow each by itself, as busy as it is.
                    g.set(ramp(u, GPU_GLOW_FROM, GPU_GLOW_FULL))?;
                }
            }
            // By level, a CPU meter glows as one: as busy as its threads are on average, in user's and kernel's
            // colors mixed as much as each is running.
            if !heat && !engines && !bars.is_empty() && m.spec.colors.len() >= 2 {
                let busy = ((user + kernel) / bars.len() as f64).min(1.0);
                let t = ramp(busy, CPU_GLOW_FROM, CPU_GLOW_FULL);
                let color = mix(m.spec.colors[0], m.spec.colors[1], if user + kernel > 0.0 { (kernel / (user + kernel)) as f32 } else { 0.0 });
                for g in bars.iter().filter_map(|b| b.glows.first()) {
                    g.shadow.SetColor(color).map_err(w)?;
                    g.set(t)?;
                }
            }
        }
    }
    Ok(())
}

/// `push <i> <text> <values..>`.
fn push(args: &[String]) -> R<String> {
    let i: usize = args.first().ok_or("missing <meter index>")?.parse().map_err(|e| format!("bad index: {e}"))?;
    let text = args.get(1).ok_or("missing <text>")?.clone();
    let values: Vec<f64> = args[2..].iter().map(|v| v.parse::<f64>().unwrap_or(f64::NAN)).collect();
    let finite = |v: Option<&f64>| v.copied().filter(|v| v.is_finite());
    let first_value = STRIP.with_borrow_mut(|s| -> R<bool> {
        let s = s.as_mut().ok_or("no meters; `meters add` first")?;
        let m = s.meters.get_mut(i).ok_or_else(|| format!("no meter #{i}"))?;
        m.text = text;
        match m.spec.style {
            Style::History => {
                if m.history.len() >= m.spec.span.max(HISTORY_MAX) {
                    m.history.pop_front();
                }
                m.history.push_back(finite(values.first()).unwrap_or(0.0).max(0.0));
                m.pushed_max = finite(values.get(1)).filter(|v| *v > 0.0);
            }
            Style::Bar => {
                let f = finite(values.first()).unwrap_or(0.0).clamp(0.0, 1.0);
                let now = Instant::now();
                m.recent.push_back((now, f));
                while m.recent.front().is_some_and(|(t, _)| now.duration_since(*t) > PEAK_HOLD) {
                    m.recent.pop_front();
                }
                m.values = vec![f];
            }
            Style::Pie | Style::Cores | Style::Engines => {
                let clean: Vec<f64> = values.iter().map(|v| if v.is_finite() { *v } else { 0.0 }).collect();
                // Bars are falling off: values come in the new order; map them onto the bars still shown.
                m.values = match &m.spec.pending_labels {
                    Some(next) => m.spec.labels.iter().map(|l| next.iter().position(|n| n == l).and_then(|k| clean.get(k).copied()).unwrap_or(0.0)).collect(),
                    None => clean,
                };
            }
        }
        draw(m)?;
        let first = m.presence == Presence::Pending;
        if first {
            m.presence = Presence::Shown;
        }
        Ok(first)
    })?;
    request_frame();
    crate::tooltip::refresh(i);
    if first_value {
        appear(i);
    }
    Ok(String::new())
}

// ---------------------------------------------------------------------------------------------
// Presence

fn presence(i: usize, on: bool) -> R<String> {
    let change = with_strip(|s| {
        let m = s.meters.get_mut(i)?;
        let before = m.presence;
        m.presence = match (on, before) {
            (true, Presence::Hidden | Presence::Leaving) => Presence::Shown,
            (false, Presence::Shown) => Presence::Leaving,
            (false, Presence::Pending) => Presence::Hidden,
            (_, p) => p,
        };
        Some((before, m.presence))
    })
    .flatten()
    .ok_or_else(|| format!("no meter #{i}"))?;
    match change {
        (Presence::Hidden | Presence::Leaving, Presence::Shown) => appear(i),
        (Presence::Shown, Presence::Leaving) => leave(i),
        _ => {}
    }
    Ok(format!("meter #{i}: {:?}", change.1))
}

/// Make room for meter `i` (refit) and grow it in.
fn appear(i: usize) {
    refit(false);
    // Only if the fit has room for it; a meter dropped for space stays out until there is.
    let target = STRIP.with_borrow(|s| {
        let s = s.as_ref()?;
        s.shown(&s.fit?).contains(&i).then(|| s.meters.get(i)?.cell.as_ref().map(|c| c.root.clone()))?
    });
    let Some(root) = target else { return };
    let r = (|| -> R<()> {
        root.SetVisibility(Visibility::Visible).map_err(w)?;
        NEEDS_POSITION.set(true);
        STRIP.with_borrow(|s| s.as_ref().map(sync_seps)).transpose()?;
        let v = visual(&root)?;
        let c = v.Compositor().map_err(w)?;
        v.SetCenterPoint(Vector3::default()).map_err(w)?;
        let grow = c.CreateVector3KeyFrameAnimation().map_err(w)?;
        grow.InsertKeyFrame(0.0, Vector3 { X: 0.0, Y: 1.0, Z: 1.0 }).map_err(w)?;
        grow.InsertKeyFrame(1.0, Vector3 { X: 1.0, Y: 1.0, Z: 1.0 }).map_err(w)?;
        grow.SetDuration(ms(APPEAR_MS)).map_err(w)?;
        v.StartAnimation(&HSTRING::from("Scale"), &grow).map_err(w)?;
        let fade = c.CreateScalarKeyFrameAnimation().map_err(w)?;
        fade.InsertKeyFrame(0.0, 0.0).map_err(w)?;
        fade.InsertKeyFrame(1.0, 1.0).map_err(w)?;
        fade.SetDuration(ms(APPEAR_MS)).map_err(w)?;
        v.StartAnimation(&HSTRING::from("Opacity"), &fade).map_err(w)
    })();
    if let Err(e) = r {
        crate::log(format!("meter appear: {e}"));
    }
}

/// Slowly shrink meter `i` to nothing, then take it out of the layout (the rest glide over).
fn leave(i: usize) {
    let target = STRIP.with_borrow(|s| s.as_ref().and_then(|s| s.meters.get(i)).and_then(|m| m.cell.as_ref().map(|c| c.root.clone())));
    // Collapse in place (no rebuild) so the rest of the strip glides over the gap.
    let finish = move || {
        let left = STRIP.with_borrow_mut(|s| {
            let s = s.as_mut()?;
            let m = s.meters.get_mut(i)?;
            if m.presence != Presence::Leaving {
                return None;
            }
            m.presence = Presence::Hidden;
            let c = m.cell.as_ref()?;
            let _ = c.root.SetVisibility(Visibility::Collapsed);
            NEEDS_POSITION.set(true);
            let _ = sync_seps(s);
            Some(())
        });
        if left.is_some() {
            refit(false);
        }
    };
    let Some(root) = target else { return finish() };
    let r = (|| -> R<()> {
        let v = visual(&root)?;
        let c = v.Compositor().map_err(w)?;
        let ease = c.CreateCubicBezierEasingFunction(Vector2 { X: 0.4, Y: 0.0 }, Vector2 { X: 0.6, Y: 1.0 }).map_err(w)?;
        let batch = c.CreateScopedBatch(CompositionBatchTypes::Animation).map_err(w)?;
        v.SetCenterPoint(Vector3 { X: (root.Width().map_err(w)? / 2.0) as f32, Y: (root.Height().map_err(w)? / 2.0) as f32, Z: 0.0 }).map_err(w)?;
        let shrink = c.CreateVector3KeyFrameAnimation().map_err(w)?;
        shrink.InsertKeyFrameWithEasingFunction(1.0, Vector3 { X: 0.0, Y: 0.0, Z: 1.0 }, &ease).map_err(w)?;
        shrink.SetDuration(ms(LEAVE_MS)).map_err(w)?;
        v.StartAnimation(&HSTRING::from("Scale"), &shrink).map_err(w)?;
        let fade = c.CreateScalarKeyFrameAnimation().map_err(w)?;
        fade.InsertKeyFrameWithEasingFunction(1.0, 0.0, &ease).map_err(w)?;
        fade.SetDuration(ms(LEAVE_MS)).map_err(w)?;
        v.StartAnimation(&HSTRING::from("Opacity"), &fade).map_err(w)?;
        batch.End().map_err(w)?;
        batch
            .Completed(&TypedEventHandler::<IInspectable, CompositionBatchCompletedEventArgs>::new(move |_, _| {
                on_ui(finish);
                Ok(())
            }))
            .map_err(w)?;
        Ok(())
    })();
    if let Err(e) = r {
        crate::log(format!("meter leave: {e}"));
    }
}

/// Run on the XAML UI thread (composition callbacks may arrive elsewhere).
fn on_ui(f: impl FnOnce() + Send + 'static) {
    let Some(ctx) = CTX.get() else { return };
    let job = std::sync::Mutex::new(Some(f));
    let handler = DispatchedHandler::new(move || {
        if let Some(f) = lock(&job).take() {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        }
        Ok(())
    });
    let _ = ctx.dispatcher.RunAsync(CoreDispatcherPriority::Normal, &handler);
}

// ---------------------------------------------------------------------------------------------
// Microphone label

fn mic(ctx: &Ctx, args: &[String]) -> R<String> {
    strip(ctx)?;
    match args.first().map(String::as_str) {
        None | Some("off") => {
            with_strip(|s| s.mic = None);
            refit(true);
            Ok("mic label off".into())
        }
        Some(_) => {
            let apps = args.join(" ");
            let caption_w = measure(&mic_caption()?)?;
            let apps_w = measure(&text_block(&apps, 12.5, 600, TEXT)?)?;
            let fresh = with_strip(|s| {
                let fresh = s.mic.is_none();
                let muted = s.mic.as_ref().is_some_and(|m| m.muted);
                let cell = s.mic.as_mut().and_then(|m| m.cell.take());
                s.mic = Some(Mic { apps: apps.clone(), muted, caption_w, apps_w, cell });
                fresh
            })
            .unwrap_or(true);
            if fresh { refit(true) } else { remic() }
            let cell = STRIP.with_borrow(|s| s.as_ref().and_then(|s| s.mic.as_ref()).and_then(|m| m.cell.clone()));
            if let (true, Some(c)) = (fresh, cell) {
                let v = visual(&c)?;
                let comp = v.Compositor().map_err(w)?;
                let fade = comp.CreateScalarKeyFrameAnimation().map_err(w)?;
                fade.InsertKeyFrame(0.0, 0.0).map_err(w)?;
                fade.InsertKeyFrame(1.0, 1.0).map_err(w)?;
                fade.SetDuration(ms(APPEAR_MS)).map_err(w)?;
                v.StartAnimation(&HSTRING::from("Opacity"), &fade).map_err(w)?;
            }
            Ok(format!("mic label: {apps}"))
        }
    }
}

/// The mic label changed (its names, "stopped N sec. ago" counting, mute): when the strip's layout stays as it is -
/// nearly always - only the label's cell is built again, so the meters beside it don't flicker or lose their heat;
/// otherwise the whole strip is refitted.
fn remic() {
    let snap = lock(&SNAPSHOT).clone();
    let swapped = STRIP.with(|cell| -> R<bool> {
        let Ok(mut state) = cell.try_borrow_mut() else { return Ok(false) };
        let Some(s) = state.as_mut() else { return Ok(false) };
        let (avail, h) = target(&snap, s);
        let fit = s.solve(avail, h);
        let Some(old) = s.mic.as_ref().and_then(|m| m.cell.clone()).filter(|_| s.fit == Some(fit)) else { return Ok(false) };
        let children = s.panel.Children().map_err(w)?;
        let mut at = 0u32;
        if !children.IndexOf(&ui(&old)?, &mut at).map_err(w)? {
            return Ok(false);
        }
        let Some(mic) = s.mic.as_mut() else { return Ok(false) };
        let new = build_mic(mic, &fit)?;
        children.SetAt(at, &ui(&new)?).map_err(w)?;
        mic.cell = Some(new);
        cast_shadows();
        NEEDS_POSITION.set(true);
        Ok(true)
    });
    match swapped {
        Ok(true) => position(&snap),
        Ok(false) => refit(true),
        Err(e) => {
            crate::log(format!("mic label: {e}"));
            refit(true);
        }
    }
}

/// The mic label, for the alert to hold on to.
pub struct MicAnchor {
    /// The strip's visual and the label's: their Offsets add up to where the label is drawn, in the stage, glides
    /// and all, frame by frame on the compositor (where layout's own idea of it lags a pass behind a rebuild).
    pub panel: Visual,
    pub cell: Visual,
    /// The stage's origin, in taskbar-window DIPs.
    pub origin: Vector2,
    pub size: Vector2,
    /// Where layout put the label, in taskbar-window DIPs.
    pub rect: Rect,
    /// Whether layout has placed the label yet: a label just rebuilt sits at 0,0 until it has.
    pub placed: bool,
}

pub fn mic_anchor() -> Option<MicAnchor> {
    STRIP.with_borrow(|s| {
        let s = s.as_ref()?;
        let cell = s.mic.as_ref()?.cell.clone()?;
        let stage = ui(&s.panel.Parent().ok()?).ok()?;
        let origin = bounds(&stage, 0.0, 0.0).ok()?;
        let shown = ui(&s.panel).ok()?.Visibility().ok()? == Visibility::Visible;
        let rect = bounds(&ui(&cell).ok()?, cell.Width().ok()?, cell.Height().ok()?).ok()?;
        Some(MicAnchor {
            panel: visual(&s.panel).ok()?,
            cell: visual(&cell).ok()?,
            origin: Vector2 { X: origin.X, Y: origin.Y },
            size: Vector2 { X: cell.Width().ok()? as f32, Y: cell.Height().ok()? as f32 },
            rect,
            placed: shown && cell.ActualWidth().ok()? > 0.0,
        })
    })
}

// ---------------------------------------------------------------------------------------------
// Placement

/// Fade the strip out (a toast is using its spot) or back in.
pub fn set_hidden(hidden: bool) {
    crate::tooltip::set_enabled(!hidden);
    STRIP.with(|cell| {
        let Ok(state) = cell.try_borrow() else { return };
        let Some(s) = state.as_ref() else { return };
        let r = visual(&s.panel).and_then(|v| {
            let c = v.Compositor().map_err(w)?;
            let fade = c.CreateScalarKeyFrameAnimation().map_err(w)?;
            fade.InsertKeyFrame(1.0, if hidden { 0.0 } else { 1.0 }).map_err(w)?;
            fade.SetDuration(ms(200)).map_err(w)?;
            v.StartAnimation(&HSTRING::from("Opacity"), &fade).map_err(w)
        });
        if let Err(e) = r {
            crate::log(format!("fade meters: {e}"));
        }
    });
}

/// Called on layout changes: refit to the new free space if needed, then position the strip.
pub fn place(snapshot: &Snapshot) {
    let stale = STRIP.with(|cell| {
        let Ok(state) = cell.try_borrow() else { return false };
        let Some(s) = state.as_ref() else { return false };
        let (avail, h) = target(snapshot, s);
        s.fit != Some(s.solve(avail, h))
    });
    if stale { refit(false) } else { position(snapshot) }
}

/// Right-align the strip in the free interval nearest the tray.
fn position(snapshot: &Snapshot) {
    STRIP.with(|cell| {
        let Ok(mut state) = cell.try_borrow_mut() else { return };
        let Some(s) = state.as_mut() else { return };
        let Some(fit) = s.fit else { return };
        let result = (|| -> R<()> {
            let panel = ui(&s.panel)?;
            let Some(&(_, end)) = snapshot.free.last() else {
                return panel.SetVisibility(Visibility::Collapsed).map_err(w);
            };
            let x = snap((end - EDGE_MARGIN) as f64 - s.total(&fit)) as f32;
            // Snapshot is window-relative; Canvas.Left is relative to the stage.
            let stage_el = ui(&s.panel.Parent().map_err(w)?)?;
            let origin = bounds(&stage_el, 0.0, 0.0)?;
            Canvas::SetLeft(&panel, (x - origin.X) as f64).map_err(w)?;
            Canvas::SetTop(&panel, ((snapshot.height as f64 - fit.h) / 2.0).round() - origin.Y as f64).map_err(w)?;
            panel.SetVisibility(Visibility::Visible).map_err(w)?;
            if !s.panel_glides {
                glide_on_move(&panel)?;
            }
            // Glide when neighbours come and go; attached once cells have a position, so they
            // don't slide in from the left edge when (re)built.
            // Hover zones over every shown meter (bars too, for per-CPU / per-engine tooltips).
            let mut zones = Vec::new();
            for i in s.shown(&fit) {
                let m = &s.meters[i];
                let Some(c) = m.cell.as_ref().filter(|_| m.spec.tooltip) else { continue };
                let rect = bounds(&ui(&c.root)?, c.root.Width().map_err(w)?, c.root.Height().map_err(w)?)?;
                let bars = match &c.parts {
                    Parts::Cores { bars, .. } => bars.iter().map(|b| b.span).collect(),
                    _ => Vec::new(),
                };
                zones.push(crate::tooltip::ZoneSpec { meter: i, rect, bars });
            }
            crate::tooltip::set_zones(zones);
            s.panel_glides = true;
            for c in s.meters.iter().filter_map(|m| m.cell.as_ref()).filter(|c| !c.gliding.get()) {
                if c.root.ActualWidth().map_err(w)? > 0.0 {
                    glide_on_move(&ui(&c.root)?)?;
                    c.gliding.set(true);
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            crate::log(format!("place meters: {e}"));
        }
    });
    crate::alert::follow();
}


thread_local! {
    static FRAME_REQUEST: Cell<Option<i64>> = const { Cell::new(None) };
}

/// Make XAML render the next frame. The taskbar's XAML only renders when its own layout wakes
/// up (here: about once a second), so changes made in between, ours included, otherwise sit
/// unpainted. A `CompositionTarget.Rendering` handler forces a frame; it unhooks itself in
/// that frame, so this costs one frame per request, not a render loop.
fn request_frame() {
    if FRAME_REQUEST.get().is_some() {
        return;
    }
    let handler = windows::Foundation::EventHandler::<IInspectable>::new(|_, _| {
        if let Some(token) = FRAME_REQUEST.take() {
            let _ = pms_xaml::CompositionTarget::RemoveRendering(token);
        }
        Ok(())
    });
    match pms_xaml::CompositionTarget::Rendering(&handler) {
        Ok(token) => FRAME_REQUEST.set(Some(token)),
        Err(e) => crate::log(format!("request frame: {e}")),
    }
}

/// Tooltip text for meter `i`, or for one of its bars (CPU, GPU engine).
pub fn tip_text(i: usize, bar: Option<usize>) -> Option<String> {
    STRIP.with_borrow(|s| {
        let m = s.as_ref()?.meters.get(i)?;
        if !m.spec.tooltip {
            return None;
        }
        let pct = |v: f64| format!("{:.0}%", v.clamp(0.0, 1.0) * 100.0);
        let v = |k: usize| m.values.get(k).copied().unwrap_or(0.0);
        Some(match (m.spec.style, bar) {
            (Style::Cores, Some(j)) => format!("{}\nuser {} · kernel {}", cpu_name(&m.spec.topo, j), pct(v(2 * j)), pct(v(2 * j + 1))),
            // Plain bars carry their own reading (NET up: 1.2 MB/s); GPU engines show their load.
            (Style::Engines, Some(j)) => match m.spec.bar_tips.get(j) {
                Some(tip) => format!("{} {}\n{tip}", m.spec.label, m.spec.labels.get(j)?),
                None => format!("{}\n{}", m.spec.labels.get(j)?, pct(v(j))),
            },
            _ if !m.spec.tip.is_empty() => format!("{}\n{}", m.spec.label, m.spec.tip),
            _ => format!("{}: {}", m.spec.label, m.text),
        })
    })
}

/// "CPU 5 · core 2, thread 1 · efficiency core · L3 group 0", from the topology string.
fn cpu_name(topo: &str, cpu: usize) -> String {
    let (nodes, l3s) = (topo.split('|').count(), topo.split(['|', '/']).count());
    let (mut logical, mut core, mut l3) = (0, 0, 0);
    for (ni, node) in topo.split('|').enumerate() {
        for group in node.split('/') {
            for c in group.split(',') {
                let n: usize = c.trim_start_matches('e').parse().unwrap_or(1);
                if cpu < logical + n {
                    let mut s = format!("CPU {cpu} · core {core}");
                    if n > 1 {
                        s += &format!(", thread {}", cpu - logical);
                    }
                    if c.starts_with('e') {
                        s += " · efficiency core";
                    }
                    if l3s > nodes {
                        s += &format!(" · L3 group {l3}");
                    }
                    if nodes > 1 {
                        s += &format!(" · NUMA node {ni}");
                    }
                    return s;
                }
                logical += n;
                core += 1;
            }
            l3 += 1;
        }
    }
    format!("CPU {cpu}")
}

thread_local! {
    /// Cells were (re)built or shown/hidden: their real positions only exist after XAML's next
    /// layout pass, so whatever hangs off them (tooltip zones, the mic pill) must be placed again
    /// then. Without this they keep the pre-layout positions until the taskbar itself changes.
    static NEEDS_POSITION: Cell<bool> = const { Cell::new(false) };
}

/// Called after every layout pass in which the taskbar's own buttons didn't move.
pub fn after_layout() {
    if NEEDS_POSITION.replace(false) {
        position(&lock(&SNAPSHOT).clone());
    }
}

thread_local! {
    /// The meter (and bar) under the pointer, lit up.
    static HIGHLIGHT: Cell<Option<(usize, Option<usize>)>> = const { Cell::new(None) };
}

/// Light up meter `target` (and its bar, if it has bars) while the pointer is over it; `None`
/// puts the last one back.
pub fn highlight(target: Option<(usize, Option<usize>)>) {
    let previous = HIGHLIGHT.replace(target);
    if previous == target {
        return;
    }
    STRIP.with(|cell| {
        let Ok(state) = cell.try_borrow() else { return };
        let Some(s) = state.as_ref() else { return };
        for (t, on) in [(previous, false), (target, true)] {
            let Some((i, bar)) = t else { continue };
            let Some(c) = s.meters.get(i).and_then(|m| m.cell.as_ref()) else { continue };
            let _ = c.glow.SetOpacity(if on { 1.0 } else { 0.0 });
            if let (Parts::Cores { bars, .. }, Some(j)) = (&c.parts, bar)
                && let Some(b) = bars.get(j) {
                    let color = if on { TRACK_HOT } else { TRACK };
                    let _ = b.track.Compositor().and_then(|c| c.CreateColorBrushWithColor(crate::themed(color))).and_then(|br| b.track.SetBrush(&br));
                }
        }
    });
    request_frame();
}

/// Bars `gone` of meter `i` fall off the bottom: a steep ease-in, a little comical. When they're
/// gone, `land_bars` puts the new set in. False if there was nothing on screen to animate.
fn drop_bars(m: &Meter, gone: &[usize], i: usize) -> bool {
    let Some(Parts::Cores { bars, .. }) = m.cell.as_ref().map(|c| &c.parts) else { return false };
    let r = (|| -> R<bool> {
        let Some(first) = gone.iter().find_map(|&j| bars.get(j)) else { return Ok(false) };
        let c = first.track.Compositor().map_err(w)?;
        let ease = c.CreateCubicBezierEasingFunction(Vector2 { X: 0.7, Y: 0.0 }, Vector2 { X: 0.84, Y: 0.0 }).map_err(w)?;
        let batch = c.CreateScopedBatch(CompositionBatchTypes::Animation).map_err(w)?;
        for b in gone.iter().filter_map(|&j| bars.get(j)) {
            for s in [&b.track, &b.kernel, &b.user] {
                let fall = c.CreateScalarKeyFrameAnimation().map_err(w)?;
                fall.InsertExpressionKeyFrameWithEasingFunction(1.0, &HSTRING::from(format!("this.StartingValue + {}", b.height + 8.0)), &ease)
                    .map_err(w)?;
                fall.SetDuration(ms(FALL_MS)).map_err(w)?;
                s.StartAnimation(&HSTRING::from("Offset.Y"), &fall).map_err(w)?;
            }
        }
        batch.End().map_err(w)?;
        batch
            .Completed(&TypedEventHandler::<IInspectable, CompositionBatchCompletedEventArgs>::new(move |_, _| {
                on_ui(move || land_bars(i));
                Ok(())
            }))
            .map_err(w)?;
        request_frame();
        Ok(true)
    })();
    r.unwrap_or_else(|e| {
        crate::log(format!("drop bars: {e}"));
        false
    })
}

/// Install meter `i`'s pending bar set and rebuild.
fn land_bars(i: usize) {
    let landed = with_strip(|s| {
        let m = s.meters.get_mut(i)?;
        let labels = m.spec.pending_labels.take()?;
        // GPU engines ("GPU — Copy (engine 9)"), laid out like CPU threads: engines of one type
        // sit together like a core's threads, types a core apart, adapters further apart (the
        // daemon sends them sorted that way). Plain bars (NET up/down) are one tight group.
        let plain = !labels.iter().any(|l| l.contains(" — "));
        let topo = if plain {
            labels.len().to_string()
        } else {
            let split = |s: &str| {
                let (gpu, rest) = s.split_once(" — ").unwrap_or(("", s));
                (gpu.to_string(), rest.split(" (engine").next().unwrap_or(rest).to_string())
            };
            let mut topo = String::new();
            let mut run = 0;
            for (j, l) in labels.iter().enumerate() {
                let here = split(l);
                if j > 0 {
                    let prev = split(&labels[j - 1]);
                    if prev == here {
                        run += 1;
                        continue;
                    }
                    topo += &run.to_string();
                    topo.push(if prev.0 == here.0 { ',' } else { '|' });
                }
                run = 1;
            }
            topo += &run.to_string();
            topo
        };
        m.values.clear();
        if m.spec.vendors && !plain {
            // One label per GPU, in the topology's order (the daemon sends adapters together).
            let mut gpus: Vec<String> = Vec::new();
            for l in &labels {
                let gpu = l.split_once(" — ").map_or("", |(g, _)| g).to_string();
                if gpus.last() != Some(&gpu) {
                    gpus.push(gpu);
                }
            }
            // A label column is as wide as the maker, or a power state under it if there's one.
            let state_w = text_block("P12", GPU_STATE_FONT.0, GPU_STATE_FONT.1, TEXT).and_then(|t| measure(&t)).unwrap_or(16.0);
            m.spec.gpus = gpus
                .iter()
                .enumerate()
                .map(|(j, g)| {
                    let v = vendor(g);
                    let ticker = v.as_ref().map_or("GPU", |v| if m.spec.short_vendors { v.short } else { v.ticker }).to_string();
                    let w = text_block(&ticker, VENDOR_FONT.0, VENDOR_FONT.1, TEXT).and_then(|t| measure(&t)).unwrap_or(24.0);
                    let stated = m.spec.gpu_states.get(j).is_some_and(|s| !s.is_empty());
                    (ticker, if stated { w.max(state_w) } else { w }, v)
                })
                .collect();
        }
        (m.spec.labels, m.spec.topo) = (labels, topo);
        Some(())
    });
    if landed.flatten().is_some() {
        refit(true);
    }
}

const FALL_MS: i64 = 1000;

/// The theme changed: rebuild the cells in the new ink.
pub fn retheme(_ctx: &Ctx) -> R<String> {
    refit(true);
    Ok(format!("theme {}", if crate::LIGHT.load(std::sync::atomic::Ordering::Relaxed) { "light" } else { "dark" }))
}

// ---------------------------------------------------------------------------------------------
// Shadows

const TEXT_SHADOW_BLUR: f32 = 3.0;
/// How far a pie's glow reaches (DIPs), how far when its resource is nearly gone, and how bright it gets.
const PIE_GLOW_REACH: f32 = 3.5;
const PIE_GLOW_SWELL: f32 = 14.0;
const PIE_GLOW_OPACITY: f32 = 0.85;
const PIE_GLOW_MS: i64 = 500;

thread_local! {
    /// Text shadows still waiting for their text's alpha mask.
    static UNCAST: RefCell<Vec<(TextBlock, windows::UI::Composition::DropShadow)>> = const { RefCell::new(Vec::new()) };
}

/// Give the text shadows made since the last time their masks, now that their cells are in the tree: XAML hands
/// out a mask for an element only once it's live.
fn cast_shadows() {
    for (t, shadow) in UNCAST.with_borrow_mut(std::mem::take) {
        let r = (|| -> R<()> {
            let mask = t.GetAlphaMask().map_err(w)?;
            // The mask is the text's own size; anchored top-left so changing text never stretches it.
            if let Ok(surface) = mask.cast::<windows::UI::Composition::CompositionSurfaceBrush>() {
                surface.SetStretch(windows::UI::Composition::CompositionStretch::None).map_err(w)?;
                surface.SetHorizontalAlignmentRatio(0.0).map_err(w)?;
                surface.SetVerticalAlignmentRatio(0.0).map_err(w)?;
            }
            shadow.SetMask(&mask).map_err(w)
        })();
        if let Err(e) = r {
            crate::log(format!("text shadow: {e}"));
        }
    }
}

/// Text has no shadow of its own here: cast one on the compositor from the text's own glyphs
/// (its alpha mask), with no offset, so it's a soft halo that keeps it legible over anything.
/// `width` reserves room for text that will change (values); otherwise the text's own width.
fn put_text(canvas: &Canvas, t: &TextBlock, x: f64, y: f64, scale: f64, width: Option<f64>) -> R<()> {
    let host = Canvas::new().map_err(w)?;
    put(canvas, &host, x, y)?;
    put(canvas, t, x, y)?;
    let e = ui(t)?;
    e.Measure(Size { Width: f32::INFINITY, Height: f32::INFINITY }).map_err(w)?;
    let natural = e.DesiredSize().map_err(w)?;
    let size = Vector2 { X: width.map_or(natural.Width, |w| w as f32 + 8.0).max(natural.Width), Y: natural.Height.max(VALUE_H as f32) };
    // The host is the shadow's size: XAML doesn't draw the child visual of an element with none.
    host.SetWidth(size.X as f64).map_err(w)?;
    host.SetHeight(size.Y as f64).map_err(w)?;
    host.SetIsHitTestVisible(false).map_err(w)?;
    let r = (|| -> R<()> {
        let c = visual(&host)?.Compositor().map_err(w)?;
        let shadow = c.CreateDropShadow().map_err(w)?;
        shadow.SetBlurRadius(TEXT_SHADOW_BLUR).map_err(w)?;
        shadow.SetOffset(Vector3::default()).map_err(w)?;
        let dark = !crate::LIGHT.load(std::sync::atomic::Ordering::Relaxed);
        shadow.SetColor(if dark { Color { A: 0xFF, R: 0, G: 0, B: 0 } } else { Color { A: 0xFF, R: 0xFF, G: 0xFF, B: 0xFF } }).map_err(w)?;
        shadow.SetOpacity(if dark { 0.75 } else { 0.9 }).map_err(w)?;
        // The text's alpha mask comes once it's on screen (an element off the tree has none): see `cast_shadows`.
        UNCAST.with_borrow_mut(|u| u.push((t.clone(), shadow.clone())));
        let sprite = c.CreateSpriteVisual().map_err(w)?;
        sprite.SetSize(size).map_err(w)?;
        sprite.SetShadow(&shadow).map_err(w)?;
        ElementCompositionPreview::SetElementChildVisual(&ui(&host)?, &sprite).map_err(w)
    })();
    if let Err(e) = r {
        crate::log(format!("text shadow: {e}"));
    }
    squeeze(t, scale)?;
    visual(&host)?.SetScale(Vector3 { X: scale as f32, Y: 1.0, Z: 1.0 }).map_err(w)
}

/// A pie's glow: a ring of its main color just outside its edge, fading out over the glow's reach - a radial gradient
/// on a sprite behind the pie, clear inside the edge, so none of it shows through the pie's face. `mid` and `outer`
/// are the stops that move out as the reach grows.
struct Glow {
    sprite: SpriteVisual,
    mid: CompositionColorGradientStop,
    outer: CompositionColorGradientStop,
    /// The sprite's half size, and the pie's edge as a gradient offset.
    half: f32,
    edge: f32,
}

/// A pie's rim: a thin ring in its main color, and behind the pie its glow, dark until `pie_glow` lights it. No
/// glow if the compositor wouldn't make one.
fn pie_rim(pie: &Canvas, d: f64, color: Color) -> R<Option<Glow>> {
    let glow = (|| -> R<Glow> {
        let reach = PIE_GLOW_SWELL as f64 + 2.0;
        // Behind the track and wedges, and its own size, so XAML sees something there to draw.
        let host = Canvas::new().map_err(w)?;
        let size = d + 2.0 * reach;
        host.SetWidth(size).map_err(w)?;
        host.SetHeight(size).map_err(w)?;
        host.SetIsHitTestVisible(false).map_err(w)?;
        let e = ui(&host)?;
        Canvas::SetLeft(&e, -reach).map_err(w)?;
        Canvas::SetTop(&e, -reach).map_err(w)?;
        pie.Children().map_err(w)?.InsertAt(0, &e).map_err(w)?;
        let c = visual(&host)?.Compositor().map_err(w)?;
        let half = (size / 2.0) as f32;
        let edge = (d / 2.0) as f32 / half;
        let brush = c.CreateRadialGradientBrush().map_err(w)?;
        let stops = brush.ColorStops().map_err(w)?;
        let stop = |at: f32, alpha: u8| c.CreateColorGradientStopWithOffsetAndColor(at, Color { A: alpha, ..color }).map_err(w);
        let (mid, outer) = (stop(edge, 0x58)?, stop(edge, 0)?);
        for s in [stop(edge * 0.97, 0)?, stop(edge, 0xD0)?, mid.clone(), outer.clone()] {
            stops.Append(&s).map_err(w)?;
        }
        let sprite = c.CreateSpriteVisual().map_err(w)?;
        sprite.SetSize(Vector2 { X: size as f32, Y: size as f32 }).map_err(w)?;
        sprite.SetBrush(&brush).map_err(w)?;
        sprite.SetOpacity(0.0).map_err(w)?;
        ElementCompositionPreview::SetElementChildVisual(&e, &sprite).map_err(w)?;
        Ok(Glow { sprite, mid, outer, half, edge })
    })();
    let ring = Ellipse::new().map_err(w)?;
    ring.SetWidth(d).map_err(w)?;
    ring.SetHeight(d).map_err(w)?;
    ring.SetStroke(&brush(Color { A: 0x90, ..color })?).map_err(w)?;
    ring.SetStrokeThickness(1.0).map_err(w)?;
    pie.Children().map_err(w)?.Append(&ui(&ring)?).map_err(w)?;
    Ok(glow.map_err(|e| crate::log(format!("pie glow: {e}"))).ok())
}

/// The glow for a pie whose first slice (RAM's "in use") is `f` full: none up to a third, fading in to full
/// strength at two thirds; and from `pressure` on, where the resource gets tight, its reach swells, slowly at first
/// and then fast, to PIE_GLOW_SWELL three quarters of the way from there to full.
fn pie_glow(glow: &Glow, lit: &Cell<(f32, f32)>, f: f64, pressure: f64) -> R<()> {
    let f = if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 };
    let t = ((f - 1.0 / 3.0) * 3.0).clamp(0.0, 1.0);
    let opacity = (PIE_GLOW_OPACITY as f64 * t * t * (3.0 - 2.0 * t)) as f32;
    let s = ((f - pressure) / ((1.0 - pressure) * 0.75)).clamp(0.0, 1.0);
    let reach = PIE_GLOW_REACH + (PIE_GLOW_SWELL - PIE_GLOW_REACH) * (s * s) as f32;
    // Quantized, so a pie that hardly moved doesn't restart its animations every update.
    let (opacity, reach) = ((opacity * 50.0).round() / 50.0, (reach * 4.0).round() / 4.0);
    let was = lit.replace((opacity, reach));
    if was == (opacity, reach) {
        return Ok(());
    }
    let (mid, outer) = (glow.edge + 0.3 * reach / glow.half, glow.edge + reach / glow.half);
    // A pie just (re)built takes it at once: strips rebuild whenever the mic label changes.
    if was.0 < 0.0 {
        glow.sprite.SetOpacity(opacity).map_err(w)?;
        glow.mid.SetOffset(mid).map_err(w)?;
        return glow.outer.SetOffset(outer).map_err(w);
    }
    let c = glow.sprite.Compositor().map_err(w)?;
    let ease = |obj: CompositionObject, prop: &str, v: f32| -> R<()> {
        let anim = c.CreateScalarKeyFrameAnimation().map_err(w)?;
        anim.InsertKeyFrame(1.0, v).map_err(w)?;
        anim.SetDuration(ms(PIE_GLOW_MS)).map_err(w)?;
        obj.StartAnimation(&HSTRING::from(prop), &anim).map_err(w)
    };
    ease(glow.sprite.cast().map_err(w)?, "Opacity", opacity)?;
    ease(glow.mid.cast().map_err(w)?, "Offset", mid)?;
    ease(glow.outer.cast().map_err(w)?, "Offset", outer)
}
