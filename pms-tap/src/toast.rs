//! In-taskbar toasts. The tray (and the meter strip) slide away and a notification card takes
//! their place, with a thin bar along its bottom counting down to auto-dismiss.
//!
//!   Grid (holds TaskbarFrame and SystemTrayFrame)
//!     |- TaskbarFrame ...
//!     |- SystemTrayFrame               <- faded out + nudged down while a toast shows
//!     `- host Grid (ours, last = on top, over the tray and the free space left of it)
//!         `- card Border, rebuilt per toast
//!             |- Grid: [logo] [app · attribution / title / body] [image] [buttons] [x]
//!             `- countdown Rectangle (bottom edge, scale-x animated to 0)
//!
//! Clicks are queued as events (`open`, `action <i>`, `close`) for the daemon, which owns the
//! notification and knows how to activate its app; the card hides itself right away. The
//! daemon decides what to show and for how long; this side only draws. UI thread only, except
//! the event queue.

use std::cell::RefCell;
use std::sync::Mutex;

use pms_xaml::{
    Border, ColumnDefinition, CornerRadius, ElementCompositionPreview, FontFamily, FrameworkElement, Grid, GridLength, GridUnitType,
    HorizontalAlignment, LineStackingStrategy, LoadedImageSurface, Orientation, PointerEventHandler, Rectangle, SolidColorBrush,
    StackPanel, TappedEventHandler, TextBlock, TextTrimming, Thickness, UIElement, VerticalAlignment,
};
use windows::Foundation::TimeSpan;
use windows::Storage::Streams::IRandomAccessStream;
use windows::UI::Color;
use windows::UI::Composition::*;
use windows::UI::Text::FontWeight;
use windows::Win32::System::WinRT::{BSOS_DEFAULT, CreateRandomAccessStreamOverStream};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
use windows_core::{HSTRING, Interface};
use windows_numerics::{Vector2, Vector3};

use crate::commands::{R, resolve_elem};
use crate::layout::{SNAPSHOT, Snapshot, bounds, element};
use crate::{Ctx, lock};

/// Text column's natural width; images and buttons widen the card beyond it.
const TEXT_W: f32 = 380.0;
const MIN_W: f32 = 220.0;
const EDGE_MARGIN: f32 = 8.0;
const LOGO: f64 = 32.0;
const BADGE: f64 = 14.0;
const IMAGE_W: f64 = 64.0;
const IMAGE_H: f64 = 36.0;
const BUTTON_H: f64 = 28.0;
const BUTTON_MAX_W: f64 = 120.0;
const SLIDE: f32 = 14.0;
const IN_MS: i64 = 260;
const OUT_MS: i64 = 200;

const CARD: Color = Color { A: 0x18, R: 0xFF, G: 0xFF, B: 0xFF };
const CARD_HOVER: Color = Color { A: 0x2C, R: 0xFF, G: 0xFF, B: 0xFF };
const BUTTON: Color = Color { A: 0x1C, R: 0xFF, G: 0xFF, B: 0xFF };
const BUTTON_HOVER: Color = Color { A: 0x40, R: 0xFF, G: 0xFF, B: 0xFF };
const TEXT: Color = Color { A: 0xFF, R: 0xFF, G: 0xFF, B: 0xFF };
const TEXT_DIM: Color = Color { A: 0xC0, R: 0xFF, G: 0xFF, B: 0xFF };
const TEXT_FAINT: Color = Color { A: 0x90, R: 0xFF, G: 0xFF, B: 0xFF };
const COUNTDOWN: Color = Color { A: 0x90, R: 0xFF, G: 0xFF, B: 0xFF };
const PROGRESS: Color = Color { A: 0xFF, R: 0x60, G: 0xCD, B: 0xFF };
const TRANSPARENT: Color = Color { A: 0, R: 0, G: 0, B: 0 };

/// What `toast show` asks for; every field but the duration is optional.
#[derive(Default)]
struct Content {
    ms: i64,
    app: String,
    attribution: String,
    title: String,
    body: String,
    /// The app's icon; a small badge next to the app name when a logo override takes the big spot.
    icon: Option<String>,
    logo: Option<String>,
    circle: bool,
    image: Option<String>,
    buttons: Vec<String>,
    /// 0..1 fill for the bottom bar instead of the countdown; negative = indeterminate.
    progress: Option<f64>,
}

struct Toast {
    host: Grid,
    tray: UIElement,
    /// What makes way while a toast shows: the whole tray; or, with the clock showing, the tray left of it.
    faded: Vec<UIElement>,
    /// The clock's button (with the bell), which the card stops short of, when the clock shows.
    clock: Option<UIElement>,
    shown: bool,
    /// Width the current content would like (place() fits it into the free space).
    want_w: f32,
    // Decoded images stay alive while the card shows them.
    _surfaces: Vec<LoadedImageSurface>,
    /// Where place() last put the card's left edge, in taskbar-window DIPs, and how wide (the ripple starts at its
    /// logo; a glow lies under it).
    left: std::cell::Cell<f32>,
    width: std::cell::Cell<f32>,
}

thread_local! {
    static TOAST: RefCell<Option<Toast>> = const { RefCell::new(None) };
}

/// Clicks waiting for the daemon (`toast events`). Not UI-thread-bound.
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

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

fn radius(r: f64) -> CornerRadius {
    CornerRadius { TopLeft: r, TopRight: r, BottomRight: r, BottomLeft: r }
}

pub fn command(ctx: &Ctx, args: &[String]) -> R<String> {
    match args.first().map(String::as_str) {
        // show ms=<n> [app= title= body= attribution= icon= logo= circle=1 image= b=<label>..]
        Some("show") => {
            let mut c = Content::default();
            for kv in &args[1..] {
                let (k, v) = kv.split_once('=').ok_or_else(|| format!("expected key=value, got `{kv}`"))?;
                let path = || Some(v.to_string()).filter(|p| !p.is_empty() && p != "-");
                match k {
                    "ms" => c.ms = v.parse().map_err(|e| format!("bad ms: {e}"))?,
                    "app" => c.app = v.into(),
                    "attribution" => c.attribution = v.into(),
                    "title" => c.title = v.into(),
                    "body" => c.body = v.into(),
                    "icon" => c.icon = path(),
                    "logo" => c.logo = path(),
                    "circle" => c.circle = v == "1",
                    "image" => c.image = path(),
                    "b" => c.buttons.push(v.into()),
                    "progress" => c.progress = v.parse().ok(),
                    other => return Err(format!("unknown toast field `{other}`")),
                }
            }
            show(ctx, c)
        }
        Some("hide") => hide().map(|()| "toast hidden".into()),
        Some("status") | None => Ok(if is_shown() { "shown".into() } else { "hidden".into() }),
        Some(other) => Err(format!("unknown toast subcommand `{other}` (show|hide|status|events)")),
    }
}

/// `toast events`: drain clicks since the last call, one per line. Runs on the pipe thread.
pub fn drain_events() -> String {
    std::mem::take(&mut *lock(&EVENTS)).join("\n")
}

fn event(e: String) {
    // We just got user input, so we may hand out foreground rights; the app the daemon is about
    // to activate can then bring its window to the front.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    lock(&EVENTS).push(e);
    if let Err(e) = std::panic::catch_unwind(hide).unwrap_or(Err("panicked".into())) {
        crate::log(format!("toast click: {e}"));
    }
}

pub fn is_shown() -> bool {
    TOAST.with_borrow(|t| t.as_ref().is_some_and(|t| t.shown))
}

/// Animate `e`'s composition visual to `opacity`, sliding it to `dy` DIPs below its layout spot.
fn glide(e: &UIElement, opacity: f32, dy: f32, ms_: i64) -> R<()> {
    ElementCompositionPreview::SetIsTranslationEnabled(e, true).map_err(w)?;
    let v = ElementCompositionPreview::GetElementVisual(e).map_err(w)?;
    let c = v.Compositor().map_err(w)?;
    let ease = c.CreateCubicBezierEasingFunction(Vector2 { X: 0.1, Y: 0.9 }, Vector2 { X: 0.2, Y: 1.0 }).map_err(w)?;
    let fade = c.CreateScalarKeyFrameAnimation().map_err(w)?;
    fade.InsertKeyFrameWithEasingFunction(1.0, opacity, &ease).map_err(w)?;
    fade.SetDuration(ms(ms_)).map_err(w)?;
    v.StartAnimation(&HSTRING::from("Opacity"), &fade).map_err(w)?;
    let slide = c.CreateScalarKeyFrameAnimation().map_err(w)?;
    slide.InsertKeyFrameWithEasingFunction(1.0, dy, &ease).map_err(w)?;
    slide.SetDuration(ms(ms_)).map_err(w)?;
    v.StartAnimation(&HSTRING::from("Translation.Y"), &slide).map_err(w)
}

/// Jump `e` to `opacity` / `dy` without animating.
fn snap_to(e: &UIElement, opacity: f32, dy: f32) -> R<()> {
    ElementCompositionPreview::SetIsTranslationEnabled(e, true).map_err(w)?;
    let v = ElementCompositionPreview::GetElementVisual(e).map_err(w)?;
    v.SetOpacity(opacity).map_err(w)?;
    v.Properties().and_then(|p| p.InsertVector3(&HSTRING::from("Translation"), Vector3 { X: 0.0, Y: dy, Z: 0.0 })).map_err(w)
}

/// The persistent part: our host Grid, last child of the Grid that holds the tray.
fn build_host(ctx: &Ctx) -> R<Toast> {
    let tray_h = resolve_elem("SystemTrayFrame")?;
    let tray: UIElement = element(ctx, tray_h)?.cast().map_err(w)?;
    let parent_h = lock(&crate::TREE).get(&tray_h).map(|n| n.parent).ok_or("SystemTrayFrame has no parent")?;
    let parent: pms_xaml::Panel = element(ctx, parent_h)?.cast().map_err(w)?;

    let host = Grid::new().map_err(w)?;
    host.SetHorizontalAlignment(HorizontalAlignment::Left).map_err(w)?;
    host.SetVerticalAlignment(VerticalAlignment::Stretch).map_err(w)?;
    host.SetIsHitTestVisible(false).map_err(w)?;
    let host_el = ui(&host)?;
    snap_to(&host_el, 0.0, SLIDE)?;
    parent.Children().map_err(w)?.Append(&host_el).map_err(w)?;
    Ok(Toast { host, tray, faded: Vec::new(), clock: None, shown: false, want_w: TEXT_W, _surfaces: Vec::new(), left: std::cell::Cell::new(0.0), width: std::cell::Cell::new(0.0) })
}

/// The tray's parts that make way for a toast, and the clock's button if the clock shows: people keep the time in
/// sight, so then a toast covers the tray icons and the network/volume button, not the clock (or the bell beside
/// it). Without a clock, the whole tray.
fn tray_parts(ctx: &Ctx, tray: &UIElement) -> (Vec<UIElement>, Option<UIElement>) {
    let find = |name: &str| -> Option<UIElement> { resolve_elem(name).ok().and_then(|h| element(ctx, h).ok()).and_then(|e| e.cast().ok()) };
    let width = |e: &UIElement| e.cast::<FrameworkElement>().and_then(|f| f.ActualWidth()).unwrap_or(0.0);
    let clock_shows = find("TimeInnerTextBlock").is_some_and(|t| width(&t) > 1.0);
    // Everything in the tray's row before the clock's button (icon stacks, which vary, and the network/volume one).
    let before = |clock: &UIElement| -> Option<Vec<UIElement>> {
        let row: pms_xaml::Panel = clock.cast::<FrameworkElement>().ok()?.Parent().ok()?.cast().ok()?;
        let children = row.Children().ok()?;
        let mut at = 0u32;
        children.IndexOf(clock, &mut at).ok()?.then_some(())?;
        Some((0..at).filter_map(|i| children.GetAt(i).ok()).collect())
    };
    match find("NotificationCenterButton").filter(|_| clock_shows).and_then(|c| before(&c).map(|b| (b, c))) {
        Some((faded, clock)) => (faded, Some(clock)),
        None => (vec![tray.clone()], None),
    }
}

fn surface(path: &str) -> R<LoadedImageSurface> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let stream = unsafe { SHCreateMemStream(Some(&bytes)) }.ok_or("SHCreateMemStream failed")?;
    let stream: IRandomAccessStream = unsafe { CreateRandomAccessStreamOverStream(&stream, BSOS_DEFAULT) }.map_err(w)?;
    LoadedImageSurface::StartLoadFromStream(&stream).map_err(w)
}

#[derive(Clone, Copy)]
enum Clip {
    None,
    Circle,
    Rounded(f32),
}

/// A fixed-size element showing an image file, optionally clipped to a circle or rounded rect.
fn picture(path: &str, width: f64, height: f64, clip: Clip, keep: &mut Vec<LoadedImageSurface>) -> R<Grid> {
    let g = Grid::new().map_err(w)?;
    g.SetWidth(width).map_err(w)?;
    g.SetHeight(height).map_err(w)?;
    g.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let s = surface(path)?;
    let c = ElementCompositionPreview::GetElementVisual(&g).map_err(w)?.Compositor().map_err(w)?;
    let b = c.CreateSurfaceBrushWithSurface(&s).map_err(w)?;
    // Thumbnails fill their frame; logos and icons fit inside it.
    b.SetStretch(if width != height { CompositionStretch::UniformToFill } else { CompositionStretch::Uniform }).map_err(w)?;
    let sprite = c.CreateSpriteVisual().map_err(w)?;
    sprite.SetBrush(&b).map_err(w)?;
    let size = Vector2 { X: width as f32, Y: height as f32 };
    sprite.SetSize(size).map_err(w)?;
    let geometry: Option<CompositionGeometry> = match clip {
        Clip::None => None,
        Clip::Circle => {
            let e = c.CreateEllipseGeometry().map_err(w)?;
            e.SetCenter(Vector2 { X: size.X / 2.0, Y: size.Y / 2.0 }).map_err(w)?;
            e.SetRadius(Vector2 { X: size.X / 2.0, Y: size.Y / 2.0 }).map_err(w)?;
            Some(e.cast().map_err(w)?)
        }
        Clip::Rounded(r) => {
            let rr = c.CreateRoundedRectangleGeometry().map_err(w)?;
            rr.SetSize(size).map_err(w)?;
            rr.SetCornerRadius(Vector2 { X: r, Y: r }).map_err(w)?;
            Some(rr.cast().map_err(w)?)
        }
    };
    if let Some(geo) = geometry {
        sprite.SetClip(&c.CreateGeometricClipWithGeometry(&geo).map_err(w)?).map_err(w)?;
    }
    ElementCompositionPreview::SetElementChildVisual(&g, &sprite).map_err(w)?;
    keep.push(s);
    Ok(g)
}

fn text(s: &str, size: f64, line: f64, weight: u16, color: Color) -> R<TextBlock> {
    let t = TextBlock::new().map_err(w)?;
    t.SetText(&HSTRING::from(s)).map_err(w)?;
    t.SetFontSize(size).map_err(w)?;
    t.SetLineHeight(line).map_err(w)?;
    t.SetLineStackingStrategy(LineStackingStrategy::BlockLineHeight).map_err(w)?;
    t.SetFontWeight(FontWeight { Weight: weight }).map_err(w)?;
    t.SetForeground(&brush(color)?).map_err(w)?;
    t.SetTextTrimming(TextTrimming::CharacterEllipsis).map_err(w)?;
    t.SetMaxLines(1).map_err(w)?;
    Ok(t)
}

/// Run `f(entered)` when the pointer enters (true) or leaves (false) `e`.
fn on_hover(e: &UIElement, f: impl Fn(bool) + Clone + Send + 'static) -> R<()> {
    for entered in [true, false] {
        let f = f.clone();
        let h = PointerEventHandler::new(move |_, _| {
            f(entered);
            Ok(())
        });
        if entered { e.PointerEntered(&h) } else { e.PointerExited(&h) }.map_err(w)?;
    }
    Ok(())
}

/// Background swaps on hover, for the card and its buttons.
fn hoverable(b: &Border, normal: Color, hover: Color) -> R<()> {
    let target = b.clone();
    on_hover(&ui(b)?, move |entered| {
        if let Ok(br) = brush(if entered { hover } else { normal }) {
            let _ = target.SetBackground(&br);
        }
    })
}

/// Tapped -> queue `name` for the daemon; stops the tap from also reaching the card.
fn on_tap(e: &UIElement, name: &str) -> R<()> {
    let name = name.to_string();
    e.Tapped(&TappedEventHandler::new(move |_, args| {
        if let Some(a) = args.as_ref() {
            let _ = a.SetHandled(true);
        }
        event(name.clone());
        Ok(())
    }))
    .map(drop)
    .map_err(w)
}

fn column(g: &Grid, star: bool) -> R<()> {
    let col = ColumnDefinition::new().map_err(w)?;
    let unit = if star { GridUnitType::Star } else { GridUnitType::Auto };
    col.SetWidth(GridLength { Value: 1.0, GridUnitType: unit }).map_err(w)?;
    g.ColumnDefinitions().map_err(w)?.Append(&col).map_err(w)
}

fn put(g: &Grid, e: &UIElement, col: i32) -> R<()> {
    Grid::SetColumn(&e.cast::<FrameworkElement>().map_err(w)?, col).map_err(w)?;
    g.Children().map_err(w)?.Append(e).map_err(w)
}

fn button(label: &str) -> R<Border> {
    let b = Border::new().map_err(w)?;
    b.SetHeight(BUTTON_H).map_err(w)?;
    b.SetMaxWidth(BUTTON_MAX_W).map_err(w)?;
    b.SetCornerRadius(radius(4.0)).map_err(w)?;
    b.SetBackground(&brush(BUTTON)?).map_err(w)?;
    b.SetPadding(Thickness { Left: 10.0, Top: 0.0, Right: 10.0, Bottom: 0.0 }).map_err(w)?;
    let t = text(label, 11.5, 14.0, 400, TEXT)?;
    t.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    b.SetChild(&ui(&t)?).map_err(w)?;
    hoverable(&b, BUTTON, BUTTON_HOVER)?;
    Ok(b)
}

/// The card for one toast, its countdown bar, and the width it would like.
/// The card, its countdown bar, the width it wants, and the color its logo glows in (None without a logo).
fn build_card(c: &Content, keep: &mut Vec<LoadedImageSurface>) -> R<(Border, Rectangle, f32, Option<Color>)> {
    let card = Border::new().map_err(w)?;
    card.SetCornerRadius(radius(6.0)).map_err(w)?;
    card.SetBackground(&brush(CARD)?).map_err(w)?;
    card.SetMargin(Thickness { Left: 0.0, Top: 2.0, Right: 0.0, Bottom: 2.0 }).map_err(w)?;
    card.SetPadding(Thickness { Left: 8.0, Top: 0.0, Right: 6.0, Bottom: 0.0 }).map_err(w)?;
    hoverable(&card, CARD, CARD_HOVER)?;
    let card_el = ui(&card)?;
    on_tap(&card_el, "open")?;

    let grid = Grid::new().map_err(w)?;
    grid.SetColumnSpacing(10.0).map_err(w)?;
    for star in [false, true, false, false, false] {
        column(&grid, star)?;
    }
    let mut want = 8.0 + 6.0 + TEXT_W;

    // Column 0: the logo override, else the app icon.
    let big = match (&c.logo, &c.icon) {
        (Some(logo), _) => Some((logo, if c.circle { Clip::Circle } else { Clip::Rounded(4.0) })),
        (None, Some(icon)) => Some((icon, Clip::None)),
        (None, None) => None,
    };
    let mut glow = None;
    if let Some((path, clip)) = big {
        match picture(path, LOGO, LOGO, clip, keep) {
            Ok(p) => {
                // The logo's color glows behind it and ripples out across the taskbar (show()); a logo that isn't
                // a PNG we can read lends the app icon's color.
                glow = crate::ripple::prominent(path).or_else(|| c.icon.as_deref().and_then(crate::ripple::prominent));
                put(&grid, &ui(&p)?, 0)?;
                want += LOGO as f32 + 10.0;
            }
            Err(e) => crate::log(format!("toast logo: {e}")),
        }
    }

    // Column 1: header (badge + app · attribution), title, body.
    let lines = StackPanel::new().map_err(w)?;
    lines.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    // Keep the bottom 3 DIPs for the countdown bar.
    lines.SetMargin(Thickness { Left: 0.0, Top: 0.0, Right: 0.0, Bottom: 3.0 }).map_err(w)?;
    let header = StackPanel::new().map_err(w)?;
    header.SetOrientation(Orientation::Horizontal).map_err(w)?;
    header.SetSpacing(5.0).map_err(w)?;
    if let (Some(_), Some(icon)) = (&c.logo, &c.icon)
        && let Ok(badge) = picture(icon, BADGE, BADGE, Clip::None, keep) {
            header.Children().map_err(w)?.Append(&ui(&badge)?).map_err(w)?;
        }
    let caption = match (c.app.is_empty(), c.attribution.is_empty()) {
        (_, true) => c.app.clone(),
        (true, false) => c.attribution.clone(),
        (false, false) => format!("{} · {}", c.app, c.attribution),
    };
    if !caption.is_empty() {
        header.Children().map_err(w)?.Append(&ui(&text(&caption, 10.5, 12.0, 400, TEXT_FAINT)?)?).map_err(w)?;
        lines.Children().map_err(w)?.Append(&ui(&header)?).map_err(w)?;
    }
    lines.Children().map_err(w)?.Append(&ui(&text(&c.title, 12.0, 14.5, 600, TEXT)?)?).map_err(w)?;
    if !c.body.is_empty() {
        lines.Children().map_err(w)?.Append(&ui(&text(&c.body, 11.5, 13.5, 400, TEXT_DIM)?)?).map_err(w)?;
    }
    put(&grid, &ui(&lines)?, 1)?;

    // Column 2: hero/inline image thumbnail.
    if let Some(path) = &c.image {
        match picture(path, IMAGE_W, IMAGE_H, Clip::Rounded(4.0), keep) {
            Ok(p) => {
                put(&grid, &ui(&p)?, 2)?;
                want += IMAGE_W as f32 + 10.0;
            }
            Err(e) => crate::log(format!("toast image: {e}")),
        }
    }

    // Column 3: buttons.
    if !c.buttons.is_empty() {
        let row = StackPanel::new().map_err(w)?;
        row.SetOrientation(Orientation::Horizontal).map_err(w)?;
        row.SetSpacing(6.0).map_err(w)?;
        row.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
        for (i, label) in c.buttons.iter().enumerate() {
            let b = button(label)?;
            on_tap(&ui(&b)?, &format!("action {i}"))?;
            row.Children().map_err(w)?.Append(&ui(&b)?).map_err(w)?;
            // Rough: ~6.5 DIPs per character at 11.5px, plus padding and spacing.
            want += ((label.chars().count() as f64 * 6.5 + 20.0).min(BUTTON_MAX_W) + 6.0) as f32;
        }
        put(&grid, &ui(&row)?, 3)?;
        want += 4.0;
    }

    // Column 4: close, visible while the pointer is over the card.
    let close = Border::new().map_err(w)?;
    close.SetWidth(24.0).map_err(w)?;
    close.SetHeight(24.0).map_err(w)?;
    close.SetCornerRadius(radius(4.0)).map_err(w)?;
    close.SetBackground(&brush(TRANSPARENT)?).map_err(w)?;
    close.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let glyph = text("\u{E8BB}", 10.0, 12.0, 400, TEXT_DIM)?;
    glyph.SetFontFamily(&FontFamily::CreateInstanceWithName(&HSTRING::from("Segoe Fluent Icons, Segoe MDL2 Assets")).map_err(w)?).map_err(w)?;
    glyph.SetHorizontalAlignment(HorizontalAlignment::Center).map_err(w)?;
    close.SetChild(&ui(&glyph)?).map_err(w)?;
    hoverable(&close, TRANSPARENT, BUTTON_HOVER)?;
    let close_el = ui(&close)?;
    on_tap(&close_el, "close")?;
    close_el.SetOpacity(0.0).map_err(w)?;
    let target = close_el.clone();
    on_hover(&card_el, move |entered| {
        let _ = target.SetOpacity(if entered { 1.0 } else { 0.0 });
    })?;
    put(&grid, &close_el, 4)?;
    want += 24.0 + 10.0;

    let countdown = Rectangle::new().map_err(w)?;
    countdown.SetHeight(if c.progress.is_some() { 2.5 } else { 2.0 }).map_err(w)?;
    countdown.SetFill(&brush(if c.progress.is_some() { PROGRESS } else { COUNTDOWN })?).map_err(w)?;
    countdown.SetVerticalAlignment(VerticalAlignment::Bottom).map_err(w)?;
    countdown.SetMargin(Thickness { Left: -2.0, Top: 0.0, Right: 0.0, Bottom: 0.5 }).map_err(w)?;
    countdown.SetRadiusX(1.0).map_err(w)?;
    countdown.SetRadiusY(1.0).map_err(w)?;

    let outer = Grid::new().map_err(w)?;
    outer.Children().map_err(w)?.Append(&ui(&grid)?).map_err(w)?;
    outer.Children().map_err(w)?.Append(&ui(&countdown)?).map_err(w)?;
    card.SetChild(&ui(&outer)?).map_err(w)?;
    Ok((card, countdown, want, glow))
}

fn show(ctx: &Ctx, c: Content) -> R<String> {
    crate::layout::ensure_tracking(ctx)?;
    if TOAST.with_borrow(Option::is_none) {
        let t = build_host(ctx)?;
        TOAST.with_borrow_mut(|s| *s = Some(t));
    }
    let mut surfaces = Vec::new();
    let (card, countdown, want, glow) = build_card(&c, &mut surfaces)?;
    let was_shown = TOAST.with_borrow_mut(|s| -> R<bool> {
        let t = s.as_mut().ok_or("toast vanished")?;
        // Whether the clock shows is decided as each toast comes (it can be switched on and off meanwhile).
        if !t.shown {
            (t.faded, t.clock) = tray_parts(ctx, &t.tray);
        }
        let children = t.host.Children().map_err(w)?;
        children.Clear().map_err(w)?;
        // The glow goes behind the card, where the logo is, rather than in it: the card's rounded corners would
        // clip it to an edge.
        if let Some(color) = glow {
            let g: FrameworkElement = crate::ripple::glow(LOGO, color)?.cast().map_err(w)?;
            g.SetHorizontalAlignment(HorizontalAlignment::Left).map_err(w)?;
            g.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
            g.SetMargin(Thickness { Left: 8.0, Top: 0.0, Right: 0.0, Bottom: 0.0 }).map_err(w)?;
            children.Append(&ui(&g)?).map_err(w)?;
        }
        children.Append(&ui(&card)?).map_err(w)?;
        t.want_w = want;
        t._surfaces = surfaces;
        Ok(std::mem::replace(&mut t.shown, true))
    })?;
    place(&lock(&SNAPSHOT).clone());
    if let Some(color) = glow {
        // From the logo's center: the card's left edge, its padding, half the logo.
        let (left, width) = TOAST.with_borrow(|s| s.as_ref().map_or((0.0, 0.0), |t| (t.left.get(), t.width.get())));
        let y = lock(&SNAPSHOT).height / 2.0;
        let at = Vector2 { X: left + 8.0 + LOGO as f32 / 2.0, Y: y };
        let card = (Vector2 { X: left + width / 2.0, Y: y }, width);
        if let Err(e) = crate::ripple::start(ctx, at, card, color) {
            crate::log(format!("toast ripple: {e}"));
        }
    } else {
        crate::ripple::stop();
    }

    TOAST.with_borrow(|s| -> R<()> {
        let t = s.as_ref().ok_or("toast vanished")?;
        t.host.SetIsHitTestVisible(true).map_err(w)?;
        if !was_shown {
            glide(&ui(&t.host)?, 1.0, 0.0, IN_MS)?;
            for e in &t.faded {
                glide(e, 0.0, SLIDE, OUT_MS)?;
            }
            crate::meters::set_hidden(true);
        } else {
            // Replacing a toast that is still up: let the new content rise in.
            let card = ui(&card)?;
            snap_to(&card, 0.0, SLIDE / 2.0)?;
            glide(&card, 1.0, 0.0, IN_MS)?;
        }
        let v = ElementCompositionPreview::GetElementVisual(&ui(&countdown)?).map_err(w)?;
        let comp = v.Compositor().map_err(w)?;
        match c.progress {
            Some(p) if p >= 0.0 => return v.SetScale(Vector3 { X: p as f32, Y: 1.0, Z: 1.0 }).map_err(w),
            Some(_) => {
                let breathe = comp.CreateScalarKeyFrameAnimation().map_err(w)?;
                breathe.InsertKeyFrame(0.0, 0.25).map_err(w)?;
                breathe.InsertKeyFrame(0.5, 1.0).map_err(w)?;
                breathe.InsertKeyFrame(1.0, 0.25).map_err(w)?;
                breathe.SetDuration(ms(1400)).map_err(w)?;
                breathe.SetIterationBehavior(AnimationIterationBehavior::Forever).map_err(w)?;
                return v.StartAnimation(&HSTRING::from("Opacity"), &breathe).map_err(w);
            }
            None => {}
        }
        // Countdown: shrink from the left edge over the toast's lifetime.
        let shrink = comp.CreateVector3KeyFrameAnimation().map_err(w)?;
        shrink.InsertKeyFrame(0.0, Vector3 { X: 1.0, Y: 1.0, Z: 1.0 }).map_err(w)?;
        shrink.InsertKeyFrameWithEasingFunction(1.0, Vector3 { X: 0.0, Y: 1.0, Z: 1.0 }, &comp.CreateLinearEasingFunction().map_err(w)?).map_err(w)?;
        shrink.SetDuration(ms(c.ms.max(1))).map_err(w)?;
        v.StartAnimation(&HSTRING::from("Scale"), &shrink).map_err(w)
    })?;
    Ok(format!("toast shown for {}ms", c.ms))
}

/// Slide the card out and the tray back. The host stays (transparent, not hit-testable).
pub(crate) fn hide() -> R<()> {
    TOAST.with_borrow_mut(|s| -> R<()> {
        let Some(t) = s.as_mut().filter(|t| t.shown) else { return Ok(()) };
        t.shown = false;
        t.host.SetIsHitTestVisible(false).map_err(w)?;
        glide(&ui(&t.host)?, 0.0, SLIDE, OUT_MS)?;
        for e in &t.faded {
            glide(e, 1.0, 0.0, IN_MS)?;
        }
        Ok(())
    })?;
    crate::ripple::stop();
    crate::meters::set_hidden(false);
    Ok(())
}

/// Remove the host and put the tray back exactly as it was. For `reset`/`detach`.
pub(crate) fn off() -> R<String> {
    let Some(t) = TOAST.with_borrow_mut(Option::take) else { return Ok("toast off".into()) };
    lock(&EVENTS).clear();
    crate::meters::set_hidden(false);
    for e in t.faded.iter().chain([&t.tray]) {
        let v = ElementCompositionPreview::GetElementVisual(e).map_err(w)?;
        v.StopAnimation(&HSTRING::from("Opacity")).map_err(w)?;
        v.StopAnimation(&HSTRING::from("Translation.Y")).map_err(w)?;
        snap_to(e, 1.0, 0.0)?;
    }
    let host = ui(&t.host)?;
    let parent: pms_xaml::Panel = t.host.Parent().map_err(w)?.cast().map_err(w)?;
    let children = parent.Children().map_err(w)?;
    let mut i = 0;
    if children.IndexOf(&host, &mut i).map_err(w)? {
        children.RemoveAt(i).map_err(w)?;
    }
    Ok("toast off; tray restored".into())
}

/// Cover the tray, growing left into the free space as far as the content wants. Called on
/// layout changes.
pub fn place(snapshot: &Snapshot) {
    TOAST.with(|cell| {
        let Ok(state) = cell.try_borrow() else { return };
        let Some(t) = state.as_ref() else { return };
        let r = (|| -> R<()> {
            let tray: FrameworkElement = t.tray.cast().map_err(w)?;
            let tr = bounds(&t.tray, tray.ActualWidth().map_err(w)?, tray.ActualHeight().map_err(w)?)?;
            // Up to the clock when it shows, else over the whole tray.
            let right = match &t.clock {
                Some(c) => bounds(c, 0.0, 0.0)?.X + EDGE_MARGIN / 2.0,
                None => tr.X + tr.Width,
            };
            let free_start = snapshot.free.iter().rev().find(|(_, end)| *end <= tr.X + 1.0).map_or(tr.X, |&(s, _)| s);
            let left = (right - EDGE_MARGIN - t.want_w).max(free_start + EDGE_MARGIN).min(right - MIN_W);
            let host = ui(&t.host)?;
            let origin = bounds(&host.cast::<FrameworkElement>().map_err(w)?.Parent().map_err(w)?.cast().map_err(w)?, 0.0, 0.0)?;
            t.host.SetMargin(Thickness { Left: (left - origin.X) as f64, Top: 0.0, Right: 0.0, Bottom: 0.0 }).map_err(w)?;
            t.left.set(left);
            t.width.set(right - left - EDGE_MARGIN);
            t.host.SetWidth((right - left - EDGE_MARGIN) as f64).map_err(w)
        })();
        if let Err(e) = r {
            crate::log(format!("place toast: {e}"));
        }
    });
}
