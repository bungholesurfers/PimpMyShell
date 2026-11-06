//! In-taskbar toasts. The tray (and the meter strip) slide away and notification cards take
//! their place, with a thin bar along the front card's bottom counting down to auto-dismiss.
//!
//!   Grid (holds TaskbarFrame and SystemTrayFrame)
//!     |- TaskbarFrame ...
//!     |- SystemTrayFrame               <- faded out + nudged down while a toast shows (with the clock
//!     |                                   showing, just the part left of the clock)
//!     `- host Grid (ours, last = on top, over the tray and the free space left of it)
//!         |- slot Grid per card, oldest first (the deck; the newest is in front, on top)
//!         |   |- the logo's glow
//!         |   `- card Border
//!         |       |- Grid: [logo] [app · attribution / title / body] [image] [buttons] [x]
//!         |       `- countdown Rectangle (bottom edge, scale-x animated to 0)
//!         `- ...
//!
//! A card that comes while another shows goes on top of it, and the one before steps back: shorter, fainter, a
//! little to the left, its insides hidden and all but a sliver of it clipped (the cards are translucent), so the
//! ones behind peek out at the front card's left like a deck seen side-on. When the front card goes, the next one
//! comes forward. The front card can be dragged: let go far enough right, or flicked right, it flies off (dismissed,
//! as with its x); otherwise it springs back.
//!
//! A card can ask before it does what's clicked (`ask=`): its insides become the question, with NO and YES.
//!
//! Clicks and swipes are queued as events (`<id> open`, `<id> action <i>`, `<id> close`, `<id> swipe`) for the
//! daemon, which owns the notifications and knows how to activate their apps; the card leaves right away, and the
//! next one's countdown waits for the daemon's `toast front`. The daemon decides what to show and for how long;
//! this side only draws. UI thread only, except the event queue.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pms_xaml::{
    Border, ColumnDefinition, CornerRadius, ElementCompositionPreview, FontFamily, FrameworkElement, Grid, GridLength, GridUnitType,
    HorizontalAlignment, LineStackingStrategy, LoadedImageSurface, Orientation, Pointer, PointerEventHandler, PointerRoutedEventArgs,
    Rectangle, SolidColorBrush, StackPanel, TappedEventHandler, TextBlock, TextTrimming, Thickness, UIElement, VerticalAlignment, Visibility,
};
use windows::Foundation::{IReference, PropertyValue, TimeSpan, TypedEventHandler};
use windows::Storage::Streams::IRandomAccessStream;
use windows::UI::Color;
use windows::UI::Composition::*;
use windows::UI::Text::FontWeight;
use windows::Win32::System::WinRT::{BSOS_DEFAULT, CreateRandomAccessStreamOverStream};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
use windows_core::{HSTRING, IInspectable, Interface};
use windows_numerics::{Vector2, Vector3};

use crate::commands::{R, resolve_elem};
use crate::layout::{SNAPSHOT, Snapshot, bounds, element};
use crate::thumb::Fit;
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

/// The deck: each card behind the front one peeks out at its left this much, the first PEEKS of them (the rest wait
/// out of sight); each step back is PEEK_SHRINK of the height shorter, and fainter. STACK_MS: the cards' moves.
/// Behind, a card's fill is CARD_BACK: brighter than the front's, or its sliver wouldn't show.
const PEEK: f32 = 7.0;
const PEEKS: usize = 2;
const PEEK_SHRINK: f32 = 0.16;
const PEEK_OPACITY: [f32; PEEKS + 1] = [1.0, 0.85, 0.6];
const STACK_MS: i64 = 340;

/// Swiping: a drag starts this far from the press (DIPs). Let go past DISMISS_AT of the card's width, or flicked right
/// faster than FLING (DIPs/ms), the card flies off; otherwise it springs back. Dragged left, it gives a little, up
/// to RUBBER.
const DRAG_START: f32 = 6.0;
const DISMISS_AT: f32 = 0.4;
const FLING: f32 = 0.5;
const RUBBER: f32 = 18.0;
/// How far back the release speed is measured.
const VELOCITY_WINDOW: Duration = Duration::from_millis(90);

const CARD: Color = Color { A: 0x18, R: 0xFF, G: 0xFF, B: 0xFF };
const CARD_HOVER: Color = Color { A: 0x2C, R: 0xFF, G: 0xFF, B: 0xFF };
const CARD_BACK: Color = Color { A: 0x38, R: 0xFF, G: 0xFF, B: 0xFF };
const BUTTON: Color = Color { A: 0x1C, R: 0xFF, G: 0xFF, B: 0xFF };
const BUTTON_HOVER: Color = Color { A: 0x40, R: 0xFF, G: 0xFF, B: 0xFF };
const TEXT: Color = Color { A: 0xFF, R: 0xFF, G: 0xFF, B: 0xFF };
const TEXT_DIM: Color = Color { A: 0xC0, R: 0xFF, G: 0xFF, B: 0xFF };
const TEXT_FAINT: Color = Color { A: 0x90, R: 0xFF, G: 0xFF, B: 0xFF };
const COUNTDOWN: Color = Color { A: 0x90, R: 0xFF, G: 0xFF, B: 0xFF };
const PROGRESS: Color = Color { A: 0xFF, R: 0x60, G: 0xCD, B: 0xFF };
const TRANSPARENT: Color = Color { A: 0, R: 0, G: 0, B: 0 };

/// A pimped card (`style=pimped`): gold where a standard one is white, and every SHEEN_MS a sheen crosses it.
const GOLD: Color = Color { A: 0xFF, R: 0xFF, G: 0xD3, B: 0x5C };
const GOLD_DIM: Color = Color { A: 0xB0, R: 0xFF, G: 0xD3, B: 0x5C };
const GOLD_RIM: Color = Color { A: 0xC8, R: 0xE8, G: 0xB0, B: 0x30 };
const GOLD_CARD: Color = Color { A: 0x34, R: 0xFF, G: 0xB8, B: 0x30 };
const GOLD_HOVER: Color = Color { A: 0x50, R: 0xFF, G: 0xB8, B: 0x30 };
const GOLD_BACK: Color = Color { A: 0x60, R: 0xFF, G: 0xB8, B: 0x30 };
const SHEEN_MS: i64 = 3200;
/// *Pimp hand!*: the note beside *Shut up, computer*, as in the window that asks.
const PIMP_HAND: Color = Color { A: 0xFF, R: 0xFF, G: 0xD6, B: 0x00 };

/// What `toast show` asks for; every field is optional (the daemon always gives the duration).
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
    /// `style=pimped`: gold, with a sheen sweeping across.
    pimped: bool,
    /// What to ask before each thing that can be clicked (`ask=`).
    asks: Vec<Ask>,
}

/// A question the card asks in place of doing `what`, the name a click on it goes by: `open`, `action <i>`.
struct Ask {
    what: String,
    question: String,
    detail: String,
}

/// A card's way of asking: clicking what has an `Ask` turns its insides into the question and its answers. The daemon
/// hears `<id> asking`, so that the card stays, and then the answer as an ordinary click, or `close` for no.
#[derive(Clone)]
struct Asker {
    id: u32,
    asks: Arc<Vec<Ask>>,
    normal: UIElement,
    panel: UIElement,
    question: TextBlock,
    detail: TextBlock,
    countdown: Rectangle,
    /// What's being asked about.
    pending: Arc<Mutex<Option<String>>>,
    quiet: Arc<AtomicBool>,
}

impl Asker {
    fn tapped(&self, what: &str) {
        if lock(&self.pending).is_some() {
            return;
        }
        match self.asks.iter().find(|a| a.what == what) {
            Some(ask) => {
                if let Err(e) = self.ask(ask) {
                    crate::log(format!("toast question: {e}"));
                    clicked(self.id, what);
                }
            }
            None => clicked(self.id, what),
        }
    }

    fn ask(&self, ask: &Ask) -> R<()> {
        self.question.SetText(&HSTRING::from(&ask.question)).map_err(w)?;
        self.detail.SetText(&HSTRING::from(&ask.detail)).map_err(w)?;
        self.normal.SetVisibility(Visibility::Collapsed).map_err(w)?;
        self.panel.SetVisibility(Visibility::Visible).map_err(w)?;
        let bar = visual(&ui(&self.countdown)?)?;
        let _ = bar.StopAnimation(&HSTRING::from("Scale"));
        bar.SetScale(Vector3 { X: 0.0, Y: 1.0, Z: 1.0 }).map_err(w)?;
        *lock(&self.pending) = Some(ask.what.clone());
        lock(&EVENTS).push(format!("{} asking", self.id));
        Ok(())
    }

    /// The answer: `Some` of what was asked about for yes.
    fn answer(&self, yes: bool) {
        let Some(what) = lock(&self.pending).take() else { return };
        if self.quiet.load(Ordering::Relaxed) {
            lock(&EVENTS).push(format!("{} quiet", self.id));
        }
        clicked(self.id, if yes { &what } else { "close" });
    }
}

/// One notification's card in the deck.
struct Card {
    /// The daemon's id for it, which its events carry.
    id: u32,
    /// The logo's glow and the card: what stacks, drags and flies.
    slot: Grid,
    /// The card's insides (text, images, buttons, countdown), hidden while it's behind another.
    inner: UIElement,
    glow: Option<UIElement>,
    /// On the card: behind the front one, all but the sliver that peeks out is clipped away.
    clip: InsetClip,
    card: Border,
    /// Behind the front card (its fill is CARD_BACK, which hovering leaves alone).
    behind: Arc<AtomicBool>,
    countdown: Rectangle,
    progress: Option<f64>,
    /// Width it would like.
    want: f32,
    /// The logo's color, for the ripple.
    color: Option<Color>,
    /// Its fill in front, and behind the others.
    fill: Color,
    fill_back: Color,
}

struct Toast {
    host: Grid,
    tray: UIElement,
    /// What makes way while a toast shows: the whole tray; or, with the clock showing, the tray left of it.
    faded: Vec<UIElement>,
    /// The clock's button (with the bell), which the cards stop short of, when the clock shows.
    clock: Option<UIElement>,
    shown: bool,
    /// The cards on screen, oldest first: the last one is in front.
    deck: Vec<Card>,
    /// Where place() last put the host's left edge, in taskbar-window DIPs (the ripple starts at the front logo; a
    /// glow lies under it), and how wide that leaves each card.
    left: Cell<f32>,
    card_w: Cell<f32>,
    /// A `drop` the daemon sent for a card the pointer was dragging, done when it lets go: (id, next countdown).
    held_drop: Option<(u32, Option<i64>)>,
}

impl Toast {
    /// How many cards peek out behind the front one, and the room they take at its left.
    fn peeking(&self) -> (usize, f32) {
        let n = self.deck.len().saturating_sub(1).min(PEEKS);
        (n, PEEK * n as f32)
    }
}

/// The front card while the pointer holds it.
struct Drag {
    id: u32,
    pointer: Pointer,
    /// Where it was pressed (host DIPs, x).
    x0: f32,
    /// Past DRAG_START: the card follows the pointer.
    moving: bool,
    /// Recent (time, x), for the release speed.
    samples: Vec<(Instant, f32)>,
    /// How far the card is off its place now.
    tx: f32,
}

thread_local! {
    static TOAST: RefCell<Option<Toast>> = const { RefCell::new(None) };
    static DRAG: RefCell<Option<Drag>> = const { RefCell::new(None) };
    /// The last press turned into a drag: the tap that may follow it isn't a click.
    static DRAGGED: Cell<bool> = const { Cell::new(false) };
    /// Ids for cards shown without one (by hand, from the REPL).
    static NEXT_ID: Cell<u32> = const { Cell::new(u32::MAX) };
}

/// Clicks and swipes waiting for the daemon (`toast events`). Not UI-thread-bound.
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn w(e: windows_core::Error) -> String {
    e.to_string()
}

fn ui<T: Interface>(e: &T) -> R<UIElement> {
    e.cast().map_err(w)
}

fn visual(e: &UIElement) -> R<Visual> {
    ElementCompositionPreview::GetElementVisual(e).map_err(w)
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

fn bezier(c: &Compositor, a: (f32, f32), b: (f32, f32)) -> R<CompositionEasingFunction> {
    c.CreateCubicBezierEasingFunction(Vector2 { X: a.0, Y: a.1 }, Vector2 { X: b.0, Y: b.1 }).and_then(|e| e.cast()).map_err(w)
}

/// Animate `o`'s scalar `prop` (or a vector's channel, "Scale.Y") to `to`.
fn tween<T: Interface>(o: &T, prop: &str, to: f32, ms_: i64, ease: &CompositionEasingFunction) -> R<()> {
    let o: CompositionObject = o.cast().map_err(w)?;
    let a = o.Compositor().and_then(|c| c.CreateScalarKeyFrameAnimation()).map_err(w)?;
    a.InsertKeyFrameWithEasingFunction(1.0, to, ease).map_err(w)?;
    a.SetDuration(ms(ms_)).map_err(w)?;
    o.StartAnimation(&HSTRING::from(prop), &a).map_err(w)
}

pub fn command(ctx: &Ctx, args: &[String]) -> R<String> {
    let fields = || -> R<Vec<(&str, &str)>> {
        args[1..].iter().map(|kv| kv.split_once('=').ok_or_else(|| format!("expected key=value, got `{kv}`"))).collect()
    };
    let id = |f: &[(&str, &str)]| -> R<u32> {
        let v = f.iter().find(|(k, _)| *k == "id").map(|(_, v)| *v).ok_or("missing id=")?;
        v.parse().map_err(|e| format!("bad id: {e}"))
    };
    let ms_of = |f: &[(&str, &str)]| -> R<Option<i64>> {
        f.iter().find(|(k, _)| *k == "ms").map(|(_, v)| v.parse().map_err(|e| format!("bad ms: {e}"))).transpose()
    };
    match args.first().map(String::as_str) {
        // show [id=<n>] ms=<n> [app=title= body= attribution= icon= logo= circle=1 image= b=<label>.. progress=]
        Some("show") => {
            let mut c = Content::default();
            let mut card = None;
            for (k, v) in fields()? {
                let path = || Some(v.to_string()).filter(|p| !p.is_empty() && p != "-");
                match k {
                    "id" => card = Some(v.parse().map_err(|e| format!("bad id: {e}"))?),
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
                    "style" => c.pimped = v == "pimped",
                    "ask" => {
                        let mut parts = v.splitn(3, '\u{1f}').map(String::from);
                        if let (Some(what), Some(question), Some(detail)) = (parts.next(), parts.next(), parts.next()) {
                            c.asks.push(Ask { what, question, detail });
                        }
                    }
                    other => return Err(format!("unknown toast field `{other}`")),
                }
            }
            let card = card.unwrap_or_else(|| NEXT_ID.replace(NEXT_ID.get().wrapping_sub(1)));
            show(ctx, card, c)
        }
        // front id=<n> ms=<n>: the card that came forward starts counting down.
        Some("front") => {
            let f = fields()?;
            let (card, ms_) = (id(&f)?, ms_of(&f)?.ok_or("missing ms=")?);
            front(card, ms_).map(|on| if on { "counting down".into() } else { format!("card {card} isn't in front") })
        }
        // drop id=<n> [ms=<n>]: take a card away; the next front one counts down over ms.
        Some("drop") => {
            let f = fields()?;
            drop_card(id(&f)?, ms_of(&f)?).map(|()| "dropped".into())
        }
        Some("hide") => hide().map(|()| "toast hidden".into()),
        Some("status") | None => Ok(if is_shown() { "shown".into() } else { "hidden".into() }),
        Some(other) => Err(format!("unknown toast subcommand `{other}` (show|front|drop|hide|status|events)")),
    }
}

/// `toast events`: drain clicks and swipes since the last call, one per line. Runs on the pipe thread.
pub fn drain_events() -> String {
    std::mem::take(&mut *lock(&EVENTS)).join("\n")
}

/// The pointer clicked something on card `id`: tell the daemon, and the card leaves.
fn clicked(id: u32, what: &str) {
    // We just got user input, so we may hand out foreground rights; the app the daemon is about
    // to activate can then bring its window to the front.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    lock(&EVENTS).push(format!("{id} {what}"));
    if let Err(e) = std::panic::catch_unwind(|| remove(id, Exit::Fade, None)).unwrap_or(Err("panicked".into())) {
        crate::log(format!("toast click: {e}"));
    }
}

pub fn is_shown() -> bool {
    TOAST.with_borrow(|t| t.as_ref().is_some_and(|t| t.shown))
}

/// Animate `e`'s composition visual to `opacity`, sliding it to `dy` DIPs below its layout spot.
fn glide(e: &UIElement, opacity: f32, dy: f32, ms_: i64) -> R<()> {
    ElementCompositionPreview::SetIsTranslationEnabled(e, true).map_err(w)?;
    let v = visual(e)?;
    let ease = bezier(&v.Compositor().map_err(w)?, (0.1, 0.9), (0.2, 1.0))?;
    tween(&v, "Opacity", opacity, ms_, &ease)?;
    tween(&v, "Translation.Y", dy, ms_, &ease)
}

/// Jump `e` to `opacity` / `dy` without animating.
fn snap_to(e: &UIElement, opacity: f32, dy: f32) -> R<()> {
    ElementCompositionPreview::SetIsTranslationEnabled(e, true).map_err(w)?;
    let v = visual(e)?;
    v.SetOpacity(opacity).map_err(w)?;
    v.Properties().and_then(|p| p.InsertVector3(&HSTRING::from("Translation"), Vector3 { X: 0.0, Y: dy, Z: 0.0 })).map_err(w)
}

/// The persistent part: our host Grid, last child of the Grid above the tray.
fn build_host(ctx: &Ctx) -> R<Toast> {
    let tray_h = resolve_elem("SystemTrayFrame")?;
    let tray: UIElement = element(ctx, tray_h)?.cast().map_err(w)?;
    let parent = crate::layout::grid_above(ctx, tray_h)?;

    let host = Grid::new().map_err(w)?;
    host.SetHorizontalAlignment(HorizontalAlignment::Left).map_err(w)?;
    host.SetVerticalAlignment(VerticalAlignment::Stretch).map_err(w)?;
    host.SetIsHitTestVisible(false).map_err(w)?;
    let host_el = ui(&host)?;
    snap_to(&host_el, 0.0, SLIDE)?;
    parent.Children().map_err(w)?.Append(&host_el).map_err(w)?;
    Ok(Toast {
        host,
        tray,
        faded: Vec::new(),
        clock: None,
        shown: false,
        deck: Vec::new(),
        left: Cell::new(0.0),
        card_w: Cell::new(0.0),
        held_drop: None,
    })
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

fn surface(bytes: &[u8]) -> R<LoadedImageSurface> {
    let stream = unsafe { SHCreateMemStream(Some(bytes)) }.ok_or("SHCreateMemStream failed")?;
    let stream: IRandomAccessStream = unsafe { CreateRandomAccessStreamOverStream(&stream, BSOS_DEFAULT) }.map_err(w)?;
    LoadedImageSurface::StartLoadFromStream(&stream).map_err(w)
}

/// Fills `brush` with the picture at `path`, scaled for a `width`×`height`-pixel frame (thumb.rs), on a thread of its
/// own; the picture shows once it's ready. One WIC can't scale goes in as it is, for the brush to scale.
fn fill_later(brush: CompositionSurfaceBrush, path: String, width: u32, height: u32, fit: Fit) {
    std::thread::spawn(move || {
        let scaled = crate::thumb::png(&path, width, height, fit);
        crate::meters::on_ui(move || {
            let loaded = scaled
                .or_else(|e| {
                    crate::log(format!("toast picture, unscaled: {e}"));
                    std::fs::read(&path).map_err(|e| format!("{path}: {e}"))
                })
                .and_then(|bytes| surface(&bytes));
            // The brush holds the surface from here on.
            match loaded.and_then(|s| brush.SetSurface(&s).map_err(w)) {
                Ok(()) => {}
                Err(e) => crate::log(format!("toast picture: {e}")),
            }
        });
    });
}

#[derive(Clone, Copy)]
enum Clip {
    None,
    Circle,
    Rounded(f32),
}

/// A fixed-size element showing an image file, optionally clipped to a circle or rounded rect. `px` is pixels per DIP.
fn picture(path: &str, width: f64, height: f64, clip: Clip, px: f64) -> R<Grid> {
    // A file that isn't there leaves its column out (the picture itself comes later).
    if !std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?.is_file() {
        return Err(format!("{path}: not a file"));
    }
    let g = Grid::new().map_err(w)?;
    g.SetWidth(width).map_err(w)?;
    g.SetHeight(height).map_err(w)?;
    g.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let c = ElementCompositionPreview::GetElementVisual(&g).map_err(w)?.Compositor().map_err(w)?;
    let b = c.CreateSurfaceBrush().map_err(w)?;
    // Thumbnails fill their frame; logos and icons fit inside it.
    let fit = if width != height { Fit::Cover } else { Fit::Contain };
    b.SetStretch(match fit {
        Fit::Cover => CompositionStretch::UniformToFill,
        Fit::Contain => CompositionStretch::Uniform,
    })
    .map_err(w)?;
    fill_later(b.clone(), path.to_owned(), (width * px).round() as u32, (height * px).round() as u32, fit);
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

/// Tapped -> queue `<id> <name>` for the daemon; stops the tap from also reaching the card. Not after a drag.
fn on_tap(e: &UIElement, id: u32, name: &str) -> R<()> {
    let name = name.to_string();
    e.Tapped(&TappedEventHandler::new(move |_, args| {
        if let Some(a) = args.as_ref() {
            let _ = a.SetHandled(true);
        }
        if !DRAGGED.get() {
            clicked(id, &name);
        }
        Ok(())
    }))
    .map(drop)
    .map_err(w)
}

/// As `on_tap`, for what the card may ask about first.
fn on_tap_asking(e: &UIElement, asker: &Asker, what: &str) -> R<()> {
    let (asker, what) = (asker.clone(), what.to_string());
    e.Tapped(&TappedEventHandler::new(move |_, args| {
        if let Some(a) = args.as_ref() {
            let _ = a.SetHandled(true);
        }
        if !DRAGGED.get() {
            asker.tapped(&what);
        }
        Ok(())
    }))
    .map(drop)
    .map_err(w)
}

/// A tap on `e`, for what isn't a click on the card.
fn on_press(e: &UIElement, f: impl Fn() + Send + 'static) -> R<()> {
    e.Tapped(&TappedEventHandler::new(move |_, args| {
        if let Some(a) = args.as_ref() {
            let _ = a.SetHandled(true);
        }
        if !DRAGGED.get() {
            f();
        }
        Ok(())
    }))
    .map(drop)
    .map_err(w)
}

/// What a card turns into to ask: the question and what it's about, *Shut up, computer*, NO and YES. Hidden until then.
fn ask_panel(id: u32, c: &Content, normal: UIElement, countdown: Rectangle) -> R<(Grid, Asker)> {
    let panel = Grid::new().map_err(w)?;
    panel.SetColumnSpacing(10.0).map_err(w)?;
    for star in [true, false, false, false] {
        column(&panel, star)?;
    }
    let lines = StackPanel::new().map_err(w)?;
    lines.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    lines.SetMargin(Thickness { Left: 0.0, Top: 0.0, Right: 0.0, Bottom: 3.0 }).map_err(w)?;
    let question = text("", 12.5, 15.0, 600, if c.pimped { GOLD } else { TEXT })?;
    let detail = text("", 11.0, 13.5, 400, TEXT_DIM)?;
    lines.Children().map_err(w)?.Append(&ui(&question)?).map_err(w)?;
    lines.Children().map_err(w)?.Append(&ui(&detail)?).map_err(w)?;
    put(&panel, &ui(&lines)?, 0)?;

    let quiet = Arc::new(AtomicBool::new(false));
    let hush = StackPanel::new().map_err(w)?;
    hush.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let label = text("\u{2610} SHUT UP, COMPUTER", 9.5, 12.0, 600, TEXT_DIM)?;
    let note = text("PIMP HAND!", 9.0, 11.0, 700, PIMP_HAND)?;
    note.SetHorizontalAlignment(HorizontalAlignment::Right).map_err(w)?;
    ui(&note)?.SetOpacity(0.0).map_err(w)?;
    hush.Children().map_err(w)?.Append(&ui(&label)?).map_err(w)?;
    hush.Children().map_err(w)?.Append(&ui(&note)?).map_err(w)?;
    let (on, shown, tick) = (quiet.clone(), ui(&note)?, label.clone());
    let hush_el = ui(&hush)?;
    on_press(&hush_el, move || {
        let now = !on.fetch_xor(true, Ordering::Relaxed);
        let _ = tick.SetText(&HSTRING::from(if now { "\u{2611} SHUT UP, COMPUTER" } else { "\u{2610} SHUT UP, COMPUTER" }));
        let _ = shown.SetOpacity(if now { 1.0 } else { 0.0 });
    })?;
    put(&panel, &hush_el, 1)?;

    let no = button("NO")?;
    let yes = button("YES")?;
    yes.SetBackground(&brush(if c.pimped { GOLD_RIM } else { PROGRESS })?).map_err(w)?;
    if let Ok(t) = yes.Child().and_then(|t| t.cast::<TextBlock>()) {
        t.SetForeground(&brush(Color { A: 0xFF, R: 0, G: 0, B: 0 })?).map_err(w)?;
    }
    put(&panel, &ui(&no)?, 2)?;
    put(&panel, &ui(&yes)?, 3)?;
    let panel_el = ui(&panel)?;
    panel_el.SetVisibility(Visibility::Collapsed).map_err(w)?;
    let asks = c.asks.iter().map(|a| Ask { what: a.what.clone(), question: a.question.clone(), detail: a.detail.clone() }).collect();
    let asker = Asker { id, asks: Arc::new(asks), normal, panel: panel_el, question, detail, countdown, pending: Arc::default(), quiet };
    let answering = asker.clone();
    on_press(&ui(&no)?, move || answering.answer(false))?;
    let answering = asker.clone();
    on_press(&ui(&yes)?, move || answering.answer(true))?;
    Ok((panel, asker))
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

/// What build_card makes: the card, its insides, its countdown bar, the width it wants, and the color its logo
/// glows in (None without a logo).
struct Built {
    fill: Color,
    fill_back: Color,
    card: Border,
    behind: Arc<AtomicBool>,
    inner: UIElement,
    countdown: Rectangle,
    want: f32,
    glow: Option<Color>,
}

fn build_card(id: u32, c: &Content, px: f64) -> R<Built> {
    let (fill, fill_hover, fill_back) = if c.pimped { (GOLD_CARD, GOLD_HOVER, GOLD_BACK) } else { (CARD, CARD_HOVER, CARD_BACK) };
    let card = Border::new().map_err(w)?;
    card.SetCornerRadius(radius(6.0)).map_err(w)?;
    card.SetBackground(&brush(fill)?).map_err(w)?;
    if c.pimped {
        card.SetBorderBrush(&brush(GOLD_RIM)?).map_err(w)?;
        card.SetBorderThickness(Thickness { Left: 1.0, Top: 1.0, Right: 1.0, Bottom: 1.0 }).map_err(w)?;
    }
    card.SetMargin(Thickness { Left: 0.0, Top: 2.0, Right: 0.0, Bottom: 2.0 }).map_err(w)?;
    card.SetPadding(Thickness { Left: 8.0, Top: 0.0, Right: 6.0, Bottom: 0.0 }).map_err(w)?;
    let card_el = ui(&card)?;
    // Hover brightens the card, unless it has stepped behind another (the pointer leaves it as one comes on top).
    let behind = Arc::new(AtomicBool::new(false));
    let (target, back) = (card.clone(), behind.clone());
    on_hover(&card_el, move |entered| {
        let now = if back.load(Ordering::Relaxed) { fill_back } else if entered { fill_hover } else { fill };
        if let Ok(b) = brush(now) {
            let _ = target.SetBackground(&b);
        }
    })?;
    let grid = Grid::new().map_err(w)?;
    grid.SetColumnSpacing(10.0).map_err(w)?;
    let countdown = Rectangle::new().map_err(w)?;
    let (panel, asker) = ask_panel(id, c, ui(&grid)?, countdown.clone())?;
    on_tap_asking(&card_el, &asker, "open")?;
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
        match picture(path, LOGO, LOGO, clip, px) {
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
        && let Ok(badge) = picture(icon, BADGE, BADGE, Clip::None, px) {
            header.Children().map_err(w)?.Append(&ui(&badge)?).map_err(w)?;
        }
    let caption = match (c.app.is_empty(), c.attribution.is_empty()) {
        (_, true) => c.app.clone(),
        (true, false) => c.attribution.clone(),
        (false, false) => format!("{} · {}", c.app, c.attribution),
    };
    if !caption.is_empty() {
        header.Children().map_err(w)?.Append(&ui(&text(&caption, 10.5, 12.0, 400, if c.pimped { GOLD_DIM } else { TEXT_FAINT })?)?).map_err(w)?;
        lines.Children().map_err(w)?.Append(&ui(&header)?).map_err(w)?;
    }
    lines.Children().map_err(w)?.Append(&ui(&text(&c.title, 12.0, 14.5, 600, if c.pimped { GOLD } else { TEXT })?)?).map_err(w)?;
    if !c.body.is_empty() {
        lines.Children().map_err(w)?.Append(&ui(&text(&c.body, 11.5, 13.5, 400, TEXT_DIM)?)?).map_err(w)?;
    }
    put(&grid, &ui(&lines)?, 1)?;

    // Column 2: hero/inline image thumbnail.
    if let Some(path) = &c.image {
        match picture(path, IMAGE_W, IMAGE_H, Clip::Rounded(4.0), px) {
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
            on_tap_asking(&ui(&b)?, &asker, &format!("action {i}"))?;
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
    on_tap(&close_el, id, "close")?;
    close_el.SetOpacity(0.0).map_err(w)?;
    let target = close_el.clone();
    on_hover(&card_el, move |entered| {
        let _ = target.SetOpacity(if entered { 1.0 } else { 0.0 });
    })?;
    put(&grid, &close_el, 4)?;
    want += 24.0 + 10.0;

    countdown.SetHeight(if c.progress.is_some() { 2.5 } else { 2.0 }).map_err(w)?;
    countdown.SetFill(&brush(match (c.progress.is_some(), c.pimped) {
        (true, _) => PROGRESS,
        (false, true) => GOLD,
        (false, false) => COUNTDOWN,
    })?)
    .map_err(w)?;
    countdown.SetVerticalAlignment(VerticalAlignment::Bottom).map_err(w)?;
    countdown.SetMargin(Thickness { Left: -2.0, Top: 0.0, Right: 0.0, Bottom: 0.5 }).map_err(w)?;
    countdown.SetRadiusX(1.0).map_err(w)?;
    countdown.SetRadiusY(1.0).map_err(w)?;

    let outer = Grid::new().map_err(w)?;
    outer.Children().map_err(w)?.Append(&ui(&grid)?).map_err(w)?;
    if !c.asks.is_empty() {
        outer.Children().map_err(w)?.Append(&ui(&panel)?).map_err(w)?;
    }
    outer.Children().map_err(w)?.Append(&ui(&countdown)?).map_err(w)?;
    card.SetChild(&ui(&outer)?).map_err(w)?;
    // A pimped card's logo glows, and ripples out, in gold.
    let glow = if c.pimped { Some(GOLD) } else { glow };
    Ok(Built { card, behind, inner: ui(&outer)?, countdown, want, glow, fill, fill_back })
}

/// A card for the deck: the card in its slot, with the logo's glow behind it (rather than in it, where the card's
/// rounded corners would clip it to an edge), draggable.
fn make_card(id: u32, c: &Content, host: &Grid) -> R<Card> {
    // Pictures are scaled for the pixels they cover.
    let px = host.XamlRoot().and_then(|r| r.RasterizationScale()).unwrap_or(1.0).max(0.5);
    let b = build_card(id, c, px)?;
    let slot = Grid::new().map_err(w)?;
    let mut glow = None;
    if let Some(color) = b.glow {
        let g: FrameworkElement = crate::ripple::glow(LOGO, color)?.cast().map_err(w)?;
        g.SetHorizontalAlignment(HorizontalAlignment::Left).map_err(w)?;
        g.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
        g.SetMargin(Thickness { Left: 8.0, Top: 0.0, Right: 0.0, Bottom: 0.0 }).map_err(w)?;
        let g = ui(&g)?;
        slot.Children().map_err(w)?.Append(&g).map_err(w)?;
        glow = Some(g);
    }
    let card_el = ui(&b.card)?;
    slot.Children().map_err(w)?.Append(&card_el).map_err(w)?;
    ElementCompositionPreview::SetIsTranslationEnabled(&ui(&slot)?, true).map_err(w)?;
    let card_v = visual(&card_el)?;
    let clip = card_v.Compositor().and_then(|c| c.CreateInsetClip()).map_err(w)?;
    card_v.SetClip(&clip).map_err(w)?;
    if c.pimped {
        sheen(&card_el, &card_v)?;
    }
    draggable(&card_el, host, id)?;
    Ok(Card {
        id,
        slot,
        inner: b.inner,
        glow,
        clip,
        card: b.card,
        behind: b.behind,
        countdown: b.countdown,
        progress: c.progress,
        want: b.want,
        color: b.glow,
        fill: b.fill,
        fill_back: b.fill_back,
    })
}

/// A band of light that crosses a pimped card, then waits: a child of the card's visual, so inside its clip, and over
/// its content without taking its clicks.
fn sheen(card: &UIElement, v: &Visual) -> R<()> {
    let c = v.Compositor().map_err(w)?;
    let gradient = c.CreateLinearGradientBrush().map_err(w)?;
    gradient.SetStartPoint(Vector2 { X: 0.0, Y: 0.0 }).map_err(w)?;
    gradient.SetEndPoint(Vector2 { X: 1.0, Y: 0.25 }).map_err(w)?;
    let stops = gradient.ColorStops().map_err(w)?;
    for (at, a) in [(0.0, 0x00), (0.5, 0x58), (1.0, 0x00)] {
        stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(at, Color { A: a, R: 0xFF, G: 0xF0, B: 0xC0 }).map_err(w)?).map_err(w)?;
    }
    let band = c.CreateSpriteVisual().map_err(w)?;
    band.SetBrush(&gradient).map_err(w)?;
    band.SetRelativeSizeAdjustment(Vector2 { X: 0.3, Y: 1.0 }).map_err(w)?;
    let across = c.CreateVector3KeyFrameAnimation().map_err(w)?;
    let linear = c.CreateLinearEasingFunction().map_err(w)?;
    across.InsertKeyFrame(0.0, Vector3 { X: -0.3, Y: 0.0, Z: 0.0 }).map_err(w)?;
    across.InsertKeyFrameWithEasingFunction(0.4, Vector3 { X: 1.0, Y: 0.0, Z: 0.0 }, &linear).map_err(w)?;
    across.InsertKeyFrameWithEasingFunction(1.0, Vector3 { X: 1.0, Y: 0.0, Z: 0.0 }, &linear).map_err(w)?;
    across.SetDuration(ms(SHEEN_MS)).map_err(w)?;
    across.SetIterationBehavior(AnimationIterationBehavior::Forever).map_err(w)?;
    band.StartAnimation(&HSTRING::from("RelativeOffsetAdjustment"), &across).map_err(w)?;
    ElementCompositionPreview::SetElementChildVisual(card, &band).map_err(w)
}

fn show(ctx: &Ctx, id: u32, c: Content) -> R<String> {
    crate::layout::ensure_tracking(ctx)?;
    if TOAST.with_borrow(Option::is_none) {
        let t = build_host(ctx)?;
        TOAST.with_borrow_mut(|s| *s = Some(t));
    }
    let host = TOAST.with_borrow(|s| s.as_ref().map(|t| t.host.clone())).ok_or("toast vanished")?;
    let card = make_card(id, &c, &host)?;
    let slot = ui(&card.slot)?;
    let color = card.color;
    let (was_shown, replaced) = TOAST.with_borrow_mut(|s| -> R<(bool, Option<Card>)> {
        let t = s.as_mut().ok_or("toast vanished")?;
        if !t.shown {
            // Whether the clock shows is decided as each toast comes (it can be switched on and off meanwhile).
            (t.faded, t.clock) = tray_parts(ctx, &t.tray);
            t.host.Children().map_err(w)?.Clear().map_err(w)?;
            t.deck.clear();
            t.held_drop = None;
        }
        // The same notification again replaces its old card, coming in on top.
        let replaced = t.deck.iter().position(|d| d.id == id).map(|i| t.deck.remove(i));
        t.host.Children().map_err(w)?.Append(&slot).map_err(w)?;
        t.deck.push(card);
        Ok((std::mem::replace(&mut t.shown, true), replaced))
    })?;
    if let Some(old) = replaced {
        leave(&old, Exit::Fade)?;
    }
    // A card coming in on top ends a drag of the one before, which springs back as it steps behind.
    if let Some(d) = DRAG.with_borrow_mut(Option::take).filter(|d| d.moving) {
        release(d.id, d.tx, 0.0, false);
    }
    place(&lock(&SNAPSHOT).clone());
    TOAST.with_borrow(|s| s.as_ref().map_or(Ok(()), |t| restack(t, Some(id))))?;
    ripple(color);

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
            // On top of the deck: the new card rises in as the one before steps back.
            snap_to(&slot, 0.0, SLIDE / 2.0)?;
            glide(&slot, 1.0, 0.0, IN_MS)?;
        }
        t.deck.last().map_or(Ok(()), |card| countdown(card, Some(c.ms)))
    })?;
    Ok(if c.ms > 0 { format!("toast {id} shown for {}ms", c.ms) } else { format!("toast {id} shown until it's answered") })
}

/// The ripple from the front card's logo (and the glow under the card), in the logo's color; none without one.
fn ripple(color: Option<Color>) {
    let Some(color) = color else { return crate::ripple::stop() };
    let Some((left, width)) = TOAST.with_borrow(|s| s.as_ref().map(|t| (t.left.get() + t.peeking().1, t.card_w.get()))) else { return };
    let Some(ctx) = crate::CTX.get() else { return };
    // From the logo's center: the card's left edge, its padding, half the logo.
    let y = lock(&SNAPSHOT).height / 2.0;
    let at = Vector2 { X: left + 8.0 + LOGO as f32 / 2.0, Y: y };
    let card = (Vector2 { X: left + width / 2.0, Y: y }, width);
    if let Err(e) = crate::ripple::start(ctx, at, card, color) {
        crate::log(format!("toast ripple: {e}"));
    }
}

/// The bottom bar of the front card: the countdown over `ms_` (none yet: full, waiting), or the progress it shows.
fn countdown(card: &Card, ms_: Option<i64>) -> R<()> {
    let v = visual(&ui(&card.countdown)?)?;
    let comp = v.Compositor().map_err(w)?;
    match (card.progress, ms_) {
        (Some(p), _) if p >= 0.0 => v.SetScale(Vector3 { X: p as f32, Y: 1.0, Z: 1.0 }).map_err(w),
        (Some(_), _) => {
            let breathe = comp.CreateScalarKeyFrameAnimation().map_err(w)?;
            breathe.InsertKeyFrame(0.0, 0.25).map_err(w)?;
            breathe.InsertKeyFrame(0.5, 1.0).map_err(w)?;
            breathe.InsertKeyFrame(1.0, 0.25).map_err(w)?;
            breathe.SetDuration(ms(1400)).map_err(w)?;
            breathe.SetIterationBehavior(AnimationIterationBehavior::Forever).map_err(w)?;
            v.StartAnimation(&HSTRING::from("Opacity"), &breathe).map_err(w)
        }
        (None, None) => {
            let _ = v.StopAnimation(&HSTRING::from("Scale"));
            v.SetScale(Vector3 { X: 1.0, Y: 1.0, Z: 1.0 }).map_err(w)
        }
        // No time limit: nothing to count down.
        (None, Some(n)) if n <= 0 => {
            let _ = v.StopAnimation(&HSTRING::from("Scale"));
            v.SetScale(Vector3 { X: 0.0, Y: 1.0, Z: 1.0 }).map_err(w)
        }
        // Shrink toward the left edge (the right end recedes) over the toast's lifetime.
        (None, Some(n)) => {
            let shrink = comp.CreateVector3KeyFrameAnimation().map_err(w)?;
            shrink.InsertKeyFrame(0.0, Vector3 { X: 1.0, Y: 1.0, Z: 1.0 }).map_err(w)?;
            shrink.InsertKeyFrameWithEasingFunction(1.0, Vector3 { X: 0.0, Y: 1.0, Z: 1.0 }, &comp.CreateLinearEasingFunction().map_err(w)?).map_err(w)?;
            shrink.SetDuration(ms(n.max(1))).map_err(w)?;
            v.StartAnimation(&HSTRING::from("Scale"), &shrink).map_err(w)
        }
    }
}

/// Put every card where its place in the deck says: the front one whole; each one behind it shorter, fainter and
/// a step left, its insides and glow faded and all but a sliver clipped; past PEEKS, out of sight. `entering` (a
/// card that brings itself in) keeps its own opacity and translation.
fn restack(t: &Toast, entering: Option<u32>) -> R<()> {
    let Some(front) = t.deck.last() else { return Ok(()) };
    let c = visual(&ui(&front.slot)?)?.Compositor().map_err(w)?;
    let ease = bezier(&c, (0.2, 0.9), (0.25, 1.0))?;
    let h = t.host.ActualHeight().map_err(w)? as f32;
    let sliver = (t.card_w.get() - PEEK).max(0.0);
    let n = t.deck.len();
    for (i, card) in t.deck.iter().enumerate() {
        let k = n - 1 - i;
        let slot = ui(&card.slot)?;
        slot.SetIsHitTestVisible(k == 0).map_err(w)?;
        let back = k > 0;
        if card.behind.swap(back, Ordering::Relaxed) != back {
            card.card.SetBackground(&brush(if back { card.fill_back } else { card.fill })?).map_err(w)?;
        }
        let v = visual(&slot)?;
        let (x, height, opacity) = pose(k as f32);
        v.SetCenterPoint(Vector3 { X: 0.0, Y: h / 2.0, Z: 0.0 }).map_err(w)?;
        tween(&v, "Scale.Y", height, STACK_MS, &ease)?;
        tween(&card.clip, "RightInset", if k == 0 { 0.0 } else { sliver }, STACK_MS, &ease)?;
        // Insides out quickly as a card steps back, in at the stack's pace as it comes forward.
        let (inside, inside_ms) = if k == 0 { (1.0, STACK_MS) } else { (0.0, STACK_MS / 2) };
        tween(&visual(&card.inner)?, "Opacity", inside, inside_ms, &ease)?;
        if let Some(g) = &card.glow {
            tween(&visual(g)?, "Opacity", inside, inside_ms, &ease)?;
        }
        if entering != Some(card.id) {
            tween(&v, "Translation.X", x, STACK_MS, &ease)?;
            tween(&v, "Translation.Y", 0.0, STACK_MS, &ease)?;
            tween(&v, "Opacity", opacity, STACK_MS, &ease)?;
        }
    }
    Ok(())
}

/// Where a card `depth` cards back sits: its shift left, its height (as a scale), its opacity. Fractional while the
/// front card is dragged away and the ones behind come forward.
fn pose(depth: f32) -> (f32, f32, f32) {
    let d = depth.clamp(0.0, PEEKS as f32 + 1.0);
    let steps = d.min(PEEKS as f32);
    let opacity = |k: usize| PEEK_OPACITY.get(k).copied().unwrap_or(0.0);
    let (i, f) = (d.floor() as usize, d.fract());
    (-PEEK * steps, 1.0 - PEEK_SHRINK * steps, opacity(i) + (opacity(i + 1) - opacity(i)) * f)
}

/// While the front card is dragged `tx` right, what's under it shows: the cards behind come forward, the next one
/// whole wherever the front card has moved off it (no further: the cards are translucent), its insides fading in;
/// or, with no card behind, the tray fades back in. `first`: the drag just started, so stop what's animating.
fn reveal(t: &Toast, tx: f32, first: bool) -> R<()> {
    let width = t.card_w.get().max(1.0);
    // All the way once the card is as far as it goes before letting go would throw it off.
    let p = (tx / (width * DISMISS_AT)).clamp(0.0, 1.0);
    let stop = |v: &Visual, props: &[&str]| {
        for prop in props {
            let _ = v.StopAnimation(&HSTRING::from(*prop));
        }
    };
    let n = t.deck.len();
    if n == 1 {
        for e in &t.faded {
            let v = visual(e)?;
            if first {
                stop(&v, &["Opacity", "Translation.Y"]);
            }
            v.SetOpacity(p).map_err(w)?;
            v.Properties().and_then(|s| s.InsertVector3(&HSTRING::from("Translation"), Vector3 { X: 0.0, Y: SLIDE * (1.0 - p), Z: 0.0 })).map_err(w)?;
        }
        return Ok(());
    }
    for (i, card) in t.deck.iter().enumerate().take(n - 1) {
        let k = n - 1 - i;
        let v = visual(&ui(&card.slot)?)?;
        let inner = visual(&card.inner)?;
        let glow = card.glow.as_ref().map(visual).transpose()?;
        if first {
            stop(&v, &["Translation.X", "Translation.Y", "Scale.Y", "Opacity"]);
            if k == 1 {
                let _ = card.clip.StopAnimation(&HSTRING::from("RightInset"));
                stop(&inner, &["Opacity"]);
                if let Some(g) = &glow {
                    stop(g, &["Opacity"]);
                }
                // Coming out from behind: the front card's fill.
                card.behind.store(false, Ordering::Relaxed);
                card.card.SetBackground(&brush(card.fill)?).map_err(w)?;
            }
        }
        let (x, height, opacity) = pose(k as f32 - p);
        v.Properties().and_then(|s| s.InsertVector3(&HSTRING::from("Translation"), Vector3 { X: x, Y: 0.0, Z: 0.0 })).map_err(w)?;
        v.SetScale(Vector3 { X: 1.0, Y: height, Z: 1.0 }).map_err(w)?;
        v.SetOpacity(opacity).map_err(w)?;
        if k == 1 {
            // The front card's left edge, in this card's own coordinates, is where it stops showing.
            card.clip.SetRightInset((width - (tx - x)).clamp(0.0, width)).map_err(w)?;
            inner.SetOpacity(p).map_err(w)?;
            if let Some(g) = &glow {
                g.SetOpacity(p).map_err(w)?;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Exit {
    /// Fade and sink, like the whole toast does.
    Fade,
    /// Thrown right from `from` (DIPs off its place) at `speed` (DIPs/ms).
    Fling { from: f32, speed: f32 },
}

/// Animate `card` away, then take its slot out of the host.
fn leave(card: &Card, exit: Exit) -> R<()> {
    let slot = ui(&card.slot)?;
    slot.SetIsHitTestVisible(false).map_err(w)?;
    let v = visual(&slot)?;
    let c = v.Compositor().map_err(w)?;
    let batch = c.CreateScopedBatch(CompositionBatchTypes::Animation).map_err(w)?;
    match exit {
        Exit::Fade => glide(&slot, 0.0, SLIDE, OUT_MS)?,
        Exit::Fling { from, speed } => {
            let to = TOAST.with_borrow(|s| s.as_ref().map_or(400.0, |t| t.card_w.get())) + 40.0;
            let distance = (to - from).max(1.0);
            // Quadratic ease-out leaves at twice its average speed: so it leaves at the speed it was let go at (as
            // near as the bounds on the time allow), and slows as it goes.
            let speed = speed.max(FLING);
            let t = (2.0 * distance / speed).clamp(160.0, 420.0) as i64;
            let ease = bezier(&c, (1.0 / 3.0, 2.0 / 3.0), (2.0 / 3.0, 1.0))?;
            tween(&v, "Translation.X", to, t, &ease)?;
            tween(&v, "Opacity", 0.0, t, &ease)?;
        }
    }
    batch.End().map_err(w)?;
    let slot = card.slot.clone();
    batch
        .Completed(&TypedEventHandler::<IInspectable, CompositionBatchCompletedEventArgs>::new(move |_, _| {
            let slot = slot.clone();
            crate::meters::on_ui(move || detach(&slot));
            Ok(())
        }))
        .map_err(w)?;
    Ok(())
}

/// Take `slot` out of the host, if it's still in it.
fn detach(slot: &Grid) {
    let Ok(el) = ui(slot) else { return };
    let Some(parent) = slot.Parent().ok().and_then(|p| p.cast::<pms_xaml::Panel>().ok()) else { return };
    let Ok(children) = parent.Children() else { return };
    let mut i = 0;
    if children.IndexOf(&el, &mut i).unwrap_or(false) {
        let _ = children.RemoveAt(i);
    }
}

/// Take card `id` out of the deck: it leaves (`exit`) and the ones behind move up. If it was in front, the next one's
/// countdown runs over `next_ms`, or (None) waits for `toast front`. The last card out takes the toast down. False
/// if `id` isn't in the deck.
fn remove(id: u32, exit: Exit, next_ms: Option<i64>) -> R<bool> {
    let gone = TOAST.with_borrow_mut(|s| {
        let t = s.as_mut()?;
        let i = t.deck.iter().position(|c| c.id == id)?;
        let front = i + 1 == t.deck.len();
        Some((t.deck.remove(i), front, t.deck.is_empty()))
    });
    let Some((card, was_front, empty)) = gone else { return Ok(false) };
    leave(&card, exit)?;
    if empty {
        hide()?;
        return Ok(true);
    }
    place(&lock(&SNAPSHOT).clone());
    TOAST.with_borrow(|s| -> R<()> {
        let Some(t) = s.as_ref() else { return Ok(()) };
        restack(t, None)?;
        t.deck.last().filter(|_| was_front).map_or(Ok(()), |f| countdown(f, next_ms))
    })?;
    if was_front {
        // The ripple (and the glow under the card) greeted the card that left.
        crate::ripple::stop();
    }
    Ok(true)
}

/// `toast front`: card `id` is in front now; it counts down over `ms_`.
fn front(id: u32, ms_: i64) -> R<bool> {
    TOAST.with_borrow(|s| match s.as_ref().and_then(|t| t.deck.last()).filter(|c| c.id == id) {
        Some(c) => countdown(c, Some(ms_)).map(|()| true),
        None => Ok(false),
    })
}

/// `toast drop`: the daemon takes card `id` away (its time is up, or the deck is full and it's the oldest); the next front one counts down over `next_ms`.
/// While the pointer holds that card, it waits for the pointer to let go.
fn drop_card(id: u32, next_ms: Option<i64>) -> R<()> {
    if DRAG.with_borrow(|d| d.as_ref().is_some_and(|d| d.id == id && d.moving)) {
        TOAST.with_borrow_mut(|s| s.as_mut().map(|t| t.held_drop = Some((id, next_ms))));
        return Ok(());
    }
    if !remove(id, Exit::Fade, next_ms)? && let Some(n) = next_ms {
        // Already gone (clicked away just before): the front card is the one the daemon means.
        TOAST.with_borrow(|s| s.as_ref().and_then(|t| t.deck.last()).map_or(Ok(()), |c| countdown(c, Some(n))))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Swiping

/// The front card follows the pointer when dragged sideways; let go, it flies off or springs back.
fn draggable(card: &UIElement, host: &Grid, id: u32) -> R<()> {
    // Where the pointer is (host DIPs, x), if it's down.
    fn at(host: &Grid, a: Option<&PointerRoutedEventArgs>) -> Option<f32> {
        let p = a?.GetCurrentPoint(host).ok()?;
        p.IsInContact().ok()?.then_some(())?;
        p.Position().ok().map(|p| p.X)
    }
    let h = host.clone();
    card.PointerPressed(&PointerEventHandler::new(move |_, a| {
        if let (Some(x), Some(pointer)) = (at(&h, a.as_ref()), a.as_ref().and_then(|a| a.Pointer().ok())) {
            DRAGGED.set(false);
            let front = TOAST.with_borrow(|s| s.as_ref().and_then(|t| t.deck.last()).is_some_and(|c| c.id == id));
            if front {
                DRAG.set(Some(Drag { id, pointer, x0: x, moving: false, samples: vec![(Instant::now(), x)], tx: 0.0 }));
            }
        }
        Ok(())
    }))
    .map_err(w)?;
    let (h, el) = (host.clone(), card.clone());
    card.PointerMoved(&PointerEventHandler::new(move |_, a| {
        let Some(x) = at(&h, a.as_ref()) else {
            // Let go somewhere we didn't hear of.
            if let Some(d) = DRAG.with_borrow_mut(|d| d.take_if(|d| d.id == id)).filter(|d| d.moving) {
                release(d.id, d.tx, 0.0, true);
            }
            return Ok(());
        };
        let step = DRAG.with_borrow_mut(|d| {
            let d = d.as_mut().filter(|d| d.id == id)?;
            let dx = x - d.x0;
            let starts = !d.moving && dx.abs() >= DRAG_START;
            if !d.moving && !starts {
                return None;
            }
            d.moving = true;
            let now = Instant::now();
            d.samples.push((now, x));
            d.samples.retain(|(t, _)| now.duration_since(*t) <= VELOCITY_WINDOW);
            d.tx = if dx >= 0.0 { dx } else { -RUBBER * (1.0 - (dx / (3.0 * RUBBER)).exp()) };
            Some((d.tx, starts.then(|| d.pointer.clone())))
        });
        let Some((tx, start)) = step else { return Ok(()) };
        if let Some(pointer) = &start {
            DRAGGED.set(true);
            let _ = el.CapturePointer(pointer);
        }
        follow(id, tx, start.is_some());
        if let Some(a) = a.as_ref() {
            let _ = a.SetHandled(true);
        }
        Ok(())
    }))
    .map_err(w)?;
    let el = card.clone();
    let up = PointerEventHandler::new(move |_, _| {
        let Some(d) = DRAG.with_borrow_mut(|d| d.take_if(|d| d.id == id)) else { return Ok(()) };
        if d.moving {
            let _ = el.ReleasePointerCapture(&d.pointer);
            release(d.id, d.tx, speed(&d.samples), true);
        }
        Ok(())
    });
    card.PointerReleased(&up).map_err(w)?;
    card.PointerCanceled(&up).map_err(w)?;
    card.PointerCaptureLost(&up).map_err(w)?;
    Ok(())
}

/// Release speed (DIPs/ms, right is positive) over the last moments of a drag.
fn speed(samples: &[(Instant, f32)]) -> f32 {
    match (samples.first(), samples.last()) {
        (Some((t0, x0)), Some((t1, x1))) => {
            let dt = t1.duration_since(*t0).as_secs_f32() * 1000.0;
            if dt < 1.0 { 0.0 } else { (x1 - x0) / dt }
        }
        _ => 0.0,
    }
}

/// Card `id` `tx` DIPs off its place, fading as it goes right. `first`: it was resting (or rising in) until now.
fn follow(id: u32, tx: f32, first: bool) {
    let r = TOAST.with_borrow(|s| -> R<()> {
        let Some(t) = s.as_ref() else { return Ok(()) };
        let Some(card) = t.deck.iter().find(|c| c.id == id) else { return Ok(()) };
        let v = visual(&ui(&card.slot)?)?;
        if first {
            for p in ["Translation.X", "Translation.Y", "Opacity"] {
                let _ = v.StopAnimation(&HSTRING::from(p));
            }
        }
        v.Properties().and_then(|p| p.InsertVector3(&HSTRING::from("Translation"), Vector3 { X: tx, Y: 0.0, Z: 0.0 })).map_err(w)?;
        let width = t.card_w.get().max(1.0);
        v.SetOpacity(1.0 - 0.7 * (tx / width).clamp(0.0, 1.0)).map_err(w)?;
        reveal(t, tx, first)
    });
    if let Err(e) = r {
        crate::log(format!("toast drag: {e}"));
    }
}

/// The pointer let go of card `id`, `tx` DIPs off its place, moving at `speed`. Far enough or fast enough right
/// (and `may_dismiss`), it flies off and the daemon hears `swipe`; otherwise it springs back, carrying on a little
/// the way it was going first.
fn release(id: u32, tx: f32, speed: f32, may_dismiss: bool) {
    let r = (|| -> R<()> {
        let (width, held) = TOAST.with_borrow_mut(|s| {
            let t = s.as_mut()?;
            Some((t.card_w.get(), t.held_drop.take_if(|(h, _)| *h == id).map(|(_, ms)| ms)))
        })
        .ok_or("toast vanished")?;
        if may_dismiss && (tx > width * DISMISS_AT || (speed > FLING && tx > 0.0)) {
            lock(&EVENTS).push(format!("{id} swipe"));
            remove(id, Exit::Fling { from: tx, speed }, held.flatten())?;
            return Ok(());
        }
        TOAST.with_borrow(|s| -> R<()> {
            let Some(t) = s.as_ref() else { return Ok(()) };
            let Some(card) = t.deck.iter().find(|c| c.id == id) else { return Ok(()) };
            // What it had uncovered goes back where it was: the cards behind, or the tray (out again).
            restack(t, Some(id))?;
            for e in &t.faded {
                glide(e, 0.0, SLIDE, OUT_MS)?;
            }
            let v = visual(&ui(&card.slot)?)?;
            let c = v.Compositor().map_err(w)?;
            let spring = c.CreateSpringScalarAnimation().map_err(w)?;
            spring.SetFinalValue(&PropertyValue::CreateSingle(0.0).and_then(|p| p.cast::<IReference<f32>>()).map_err(w)?).map_err(w)?;
            // DIPs per second.
            spring.SetInitialVelocity(speed.clamp(-3.0, 3.0) * 1000.0).map_err(w)?;
            spring.SetDampingRatio(0.72).map_err(w)?;
            spring.SetPeriod(ms(55)).map_err(w)?;
            v.StartAnimation(&HSTRING::from("Translation.X"), &spring).map_err(w)?;
            tween(&v, "Opacity", 1.0, 220, &bezier(&c, (0.2, 0.9), (0.25, 1.0))?)
        })?;
        if let Some(next_ms) = held {
            remove(id, Exit::Fade, next_ms)?;
        }
        Ok(())
    })();
    if let Err(e) = r {
        crate::log(format!("toast release: {e}"));
    }
}

/// Slide the cards out and the tray back. The host stays (transparent, not hit-testable).
pub(crate) fn hide() -> R<()> {
    DRAG.set(None);
    TOAST.with_borrow_mut(|s| -> R<()> {
        let Some(t) = s.as_mut().filter(|t| t.shown) else { return Ok(()) };
        t.shown = false;
        t.deck.clear();
        t.held_drop = None;
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
    DRAG.set(None);
    let Some(t) = TOAST.with_borrow_mut(Option::take) else { return Ok("toast off".into()) };
    lock(&EVENTS).clear();
    crate::meters::set_hidden(false);
    // Best effort per element: one that can't be stopped (a visual that never had a Translation to animate
    // says "property not found") must not leave the rest faded, or our host in the tree.
    for e in t.faded.iter().chain([&t.tray]) {
        if let Ok(v) = ElementCompositionPreview::GetElementVisual(e) {
            let _ = v.StopAnimation(&HSTRING::from("Opacity"));
            let _ = v.StopAnimation(&HSTRING::from("Translation.Y"));
        }
        if let Err(e) = snap_to(e, 1.0, 0.0) {
            crate::log(format!("toast off: restoring a tray element: {e}"));
        }
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

/// Cover the tray, growing left into the free space as far as the cards want (the widest of them, and room for
/// the ones peeking out behind). Called on layout changes.
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
            let (_, peek) = t.peeking();
            let want = t.deck.iter().map(|c| c.want).fold(MIN_W, f32::max) + peek;
            let free_start = snapshot.free.iter().rev().find(|(_, end)| *end <= tr.X + 1.0).map_or(tr.X, |&(s, _)| s);
            let left = (right - EDGE_MARGIN - want).max(free_start + EDGE_MARGIN).min(right - MIN_W - peek);
            let host = ui(&t.host)?;
            let origin = bounds(&host.cast::<FrameworkElement>().map_err(w)?.Parent().map_err(w)?.cast().map_err(w)?, 0.0, 0.0)?;
            let width = right - left - EDGE_MARGIN;
            t.host.SetMargin(Thickness { Left: (left - origin.X) as f64, Top: 0.0, Right: 0.0, Bottom: 0.0 }).map_err(w)?;
            t.host.SetWidth(width as f64).map_err(w)?;
            t.left.set(left);
            // Every card the same width, right-aligned; the room at the left is where the ones behind peek out.
            for c in &t.deck {
                c.slot.SetMargin(Thickness { Left: peek as f64, Top: 0.0, Right: 0.0, Bottom: 0.0 }).map_err(w)?;
            }
            let card_w = width - peek;
            if t.card_w.replace(card_w) != card_w {
                // Behind the front card, the sliver that peeks out stays a sliver.
                for c in t.deck.iter().rev().skip(1) {
                    c.clip.SetRightInset((card_w - PEEK).max(0.0)).map_err(w)?;
                }
            }
            Ok(())
        })();
        if let Err(e) = r {
            crate::log(format!("place toast: {e}"));
        }
    });
}
