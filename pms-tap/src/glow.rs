//! Glows for the taskbar's own buttons, Start and the apps': none at rest; fading in while the pointer is over a
//! button; brighter, wider and quicker while it's pressed; and, for Start, while its menu is open (however it was
//! opened: the Windows key too). They lie in the background, under the buttons, so they light them from behind.
//! Start glows in its color; an app in its icon's most prominent color, taken from the icon as the taskbar draws it.
//!
//!   buttonglow start <#color> <hover> <press> <open> <hover_ms> <press_ms> <hover_radius> <press_radius>
//!   buttonglow apps <hover> <press> <hover_ms> <press_ms> <hover_radius> <press_radius>
//!   buttonglow off
//!
//! Levels are opacities 0..1, radii DIPs. The buttons say themselves when they're hovered, pressed and (Start)
//! checked - its menu open - in their IsPointerOver, IsPressed and IsChecked properties; we listen for those
//! changing. App buttons come and go: on_layout() keeps up. UI thread only.

use std::cell::RefCell;
use std::collections::HashMap;

use pms_xaml::{
    ButtonBase, DependencyObject, DependencyProperty, DependencyPropertyChangedCallback, ElementCompositionPreview, FrameworkElement, Grid, RenderTargetBitmap,
    ToggleButton, UIElement,
};
use windows::Storage::Streams::DataReader;
use windows::UI::Color;
use windows::UI::Composition::*;
use windows_core::{HSTRING, Interface};
use windows_future::{AsyncActionCompletedHandler, AsyncOperationCompletedHandler, AsyncStatus};
use windows_numerics::{Vector2, Vector3};

use crate::backdrop::parse_color;
use crate::commands::{R, matches, resolve_elem};
use crate::layout::{bounds, element};
use crate::stage::{self, Slot};
use crate::xamlom::InstanceHandle;
use crate::{Ctx, TREE, lock};

fn w(e: windows_core::Error) -> String {
    e.message()
}

/// How a kind of button glows: opacities at hover, press and (Start) with its menu open; fade times; radii.
#[derive(Clone, Copy)]
struct Feel {
    hover: f32,
    press: f32,
    open: f32,
    hover_ms: i64,
    press_ms: i64,
    hover_radius: f32,
    press_radius: f32,
}

/// How long a changed icon gets to show before its color is taken.
const ICON_SETTLE_MS: u64 = 400;

/// An app's glow until its icon's color is known.
const UNKNOWN: Color = Color { A: 0xFF, R: 0xE8, G: 0xEC, B: 0xF2 };

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Start,
    App(InstanceHandle),
}

/// How a button lets on that it's hovered and pressed.
enum Source {
    /// Start: a real button, whose properties say so (and IsChecked: its menu open).
    Button { button: ButtonBase, tokens: Vec<(DependencyProperty, i64)> },
    /// An app's: a control of the taskbar's own, which handles its press itself. Pointer events tell: over it and
    /// gone (on it), pressed and let go (on what it's made of inside, which hears them before it does), and lost.
    Events { inner: UIElement, tokens: Vec<(Ev, i64)>, over: bool, pressed: bool },
}

#[derive(Clone, Copy, PartialEq)]
enum Ev {
    Entered,
    Exited,
    Pressed,
    Released,
    Lost,
    Canceled,
}

struct Glow {
    element: UIElement,
    source: Source,
    sprite: SpriteVisual,
    brush: CompositionRadialGradientBrush,
    /// The opacity it's heading to.
    level: f32,
    /// An app's: its icon, to take the color from, and the rendering of it under way. The icon changes as the app
    /// runs (File Explorer's folder turns into a computer): `watched` is the image whose Source we follow.
    icon: Option<UIElement>,
    rendering: Option<RenderTargetBitmap>,
    colored: bool,
    watched: Option<(pms_xaml::Image, i64)>,
}

struct Glows {
    host: UIElement,
    root: ContainerVisual,
    compositor: Compositor,
    start: Option<(Glow, Feel)>,
    apps: Option<Feel>,
    app: HashMap<InstanceHandle, Glow>,
    /// App buttons that can't be watched (said once in the log, not on every layout pass).
    failed: std::collections::HashSet<InstanceHandle>,
}

thread_local! {
    static GLOWS: RefCell<Option<Glows>> = const { RefCell::new(None) };
}

pub fn command(ctx: &Ctx, args: &[String]) -> R<String> {
    let num = |i: usize, d: f64| args.get(i).map_or(Ok(d), |s| s.parse::<f64>().map_err(|_| format!("bad number `{s}`")));
    match args.first().map(String::as_str) {
        Some("start") => {
            let color = parse_color(args.get(1).ok_or("missing <#color>")?)?;
            let feel = Feel {
                hover: num(2, 0.45)? as f32,
                press: num(3, 0.9)? as f32,
                open: num(4, 0.6)? as f32,
                hover_ms: num(5, 220.0)? as i64,
                press_ms: num(6, 70.0)? as i64,
                hover_radius: num(7, 120.0)? as f32,
                press_radius: num(8, 170.0)? as f32,
            };
            stop_start();
            hosts(ctx)?;
            let glow = watch(ctx, Key::Start, resolve_elem("LaunchListButton")?, feel, Some(color))?;
            GLOWS.with_borrow_mut(|g| g.as_mut().map(|g| g.start = Some((glow, feel))));
            Ok("start glow on".into())
        }
        Some("apps") => {
            let feel = Feel {
                hover: num(1, 0.4)? as f32,
                press: num(2, 0.8)? as f32,
                open: 0.0,
                hover_ms: num(3, 220.0)? as i64,
                press_ms: num(4, 70.0)? as i64,
                hover_radius: num(5, 90.0)? as f32,
                press_radius: num(6, 130.0)? as f32,
            };
            stop_apps();
            hosts(ctx)?;
            GLOWS.with_borrow_mut(|g| g.as_mut().map(|g| g.apps = Some(feel)));
            on_layout(ctx);
            Ok("app glows on".into())
        }
        Some("off") | None => off(ctx),
        Some(other) => Err(format!("buttonglow: unknown `{other}`")),
    }
}

/// The host in the background slot, made on first use.
fn hosts(ctx: &Ctx) -> R<()> {
    if GLOWS.with_borrow(Option::is_some) {
        return Ok(());
    }
    let host: UIElement = Grid::new().and_then(|g| g.cast()).map_err(w)?;
    host.SetIsHitTestVisible(false).map_err(w)?;
    stage::insert(ctx, Slot::Alert, &host)?;
    let compositor = ElementCompositionPreview::GetElementVisual(&host).map_err(w)?.Compositor().map_err(w)?;
    let root = compositor.CreateContainerVisual().map_err(w)?;
    ElementCompositionPreview::SetElementChildVisual(&host, &root).map_err(w)?;
    GLOWS.with_borrow_mut(|g| *g = Some(Glows { host, root, compositor, start: None, apps: None, app: HashMap::new(), failed: Default::default() }));
    Ok(())
}

/// A glow for the button at `handle`, listening to it.
fn watch(ctx: &Ctx, key: Key, handle: InstanceHandle, feel: Feel, color: Option<Color>) -> R<Glow> {
    let el: UIElement = element(ctx, handle)?.cast().map_err(w)?;
    let source = match key {
        Key::Start => listen_button(key, el.cast().map_err(w)?)?,
        Key::App(_) => listen_events(ctx, key, handle, &el)?,
    };
    let (compositor, root) = GLOWS.with_borrow(|g| g.as_ref().map(|g| (g.compositor.clone(), g.root.clone()))).ok_or("no glow host")?;
    let brush = compositor.CreateRadialGradientBrush().map_err(w)?;
    let sprite = compositor.CreateSpriteVisual().map_err(w)?;
    sprite.SetBrush(&brush).map_err(w)?;
    sprite.SetOpacity(0.0).map_err(w)?;
    // As wide as it ever gets; Scale shrinks it to the radius of the moment.
    let most = feel.hover_radius.max(feel.press_radius).max(1.0);
    sprite.SetSize(Vector2 { X: 2.0 * most, Y: 2.0 * most }).map_err(w)?;
    sprite.SetCenterPoint(Vector3 { X: most, Y: most, Z: 0.0 }).map_err(w)?;
    root.Children().map_err(w)?.InsertAtTop(&sprite).map_err(w)?;
    paint(&compositor, &brush, color.unwrap_or(UNKNOWN))?;
    let mut glow = Glow { element: el, source, sprite, brush, level: 0.0, icon: None, rendering: None, colored: color.is_some(), watched: None };
    if let Key::App(h) = key {
        find_icon(ctx, key, h, &mut glow);
    }
    Ok(glow)
}

/// An app button's icon as it is now, to take the color from, and a watch on it: a new picture means a new color,
/// taken once it's had a moment to show.
fn find_icon(ctx: &Ctx, key: Key, button: InstanceHandle, glow: &mut Glow) {
    let icon: Option<UIElement> = icon_of(button).and_then(|i| element(ctx, i).ok()).and_then(|e| e.cast().ok());
    let image = icon.as_ref().and_then(|i| i.cast::<pms_xaml::Image>().ok());
    let same = matches!((&glow.watched, &image), (Some((a, _)), Some(b)) if a == b);
    if !same {
        unwatch(glow);
        if let Some(image) = image {
            let callback = DependencyPropertyChangedCallback::new(move |_, _| {
                later(key, ICON_SETTLE_MS);
                Ok(())
            });
            let token = pms_xaml::Image::SourceProperty()
                .and_then(|dp| image.cast::<DependencyObject>().and_then(|d| d.RegisterPropertyChangedCallback(&dp, &callback)));
            if let Ok(token) = token {
                glow.watched = Some((image, token));
            }
        }
    }
    glow.icon = icon;
}

fn unwatch(glow: &mut Glow) {
    if let Some((image, token)) = glow.watched.take() {
        let _ = pms_xaml::Image::SourceProperty().and_then(|dp| image.cast::<DependencyObject>().and_then(|d| d.UnregisterPropertyChangedCallback(&dp, token)));
    }
}

/// Take an app's color again in `ms`: its icon may have changed (a new picture, or a click that starts the app).
fn later(key: Key, ms: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        let Some(ctx) = crate::CTX.get() else { return };
        let handler = windows::UI::Core::DispatchedHandler::new(move || {
            let _ = std::panic::catch_unwind(|| recolor(key));
            Ok(())
        });
        let _ = ctx.dispatcher.RunAsync(windows::UI::Core::CoreDispatcherPriority::Low, &handler);
    });
}

fn recolor(key: Key) {
    let Key::App(h) = key else { return };
    let Some(ctx) = crate::CTX.get() else { return };
    let r = with_glow(key, |g| {
        find_icon(ctx, key, h, g);
        if g.rendering.is_none() { take_color(key, g) } else { Ok(()) }
    });
    if let Some(Err(e)) = r {
        crate::log(format!("app glow color: {e}"));
    }
}

/// Start: its IsPointerOver and IsPressed, and IsChecked - its menu open, however it was opened.
fn listen_button(key: Key, button: ButtonBase) -> R<Source> {
    let target: DependencyObject = button.cast().map_err(w)?;
    let mut props = vec![ButtonBase::IsPointerOverProperty().map_err(w)?, ButtonBase::IsPressedProperty().map_err(w)?];
    if button.cast::<ToggleButton>().is_ok() {
        props.push(ToggleButton::IsCheckedProperty().map_err(w)?);
    }
    let mut tokens = Vec::new();
    for dp in props {
        let callback = DependencyPropertyChangedCallback::new(move |_, _| {
            // A panic unwinding into XAML would take explorer down with it.
            let _ = std::panic::catch_unwind(|| update(key));
            Ok(())
        });
        tokens.push((dp.clone(), target.RegisterPropertyChangedCallback(&dp, &callback).map_err(w)?));
    }
    Ok(Source::Button { button, tokens })
}

/// An app button: pointer events on it, and on the first thing inside it (where presses arrive first).
fn listen_events(ctx: &Ctx, key: Key, handle: InstanceHandle, el: &UIElement) -> R<Source> {
    let inner = {
        let tree = lock(&TREE);
        tree.iter().filter(|(_, n)| n.parent == handle).min_by_key(|(_, n)| n.index).map(|(h, _)| *h)
    };
    let inner: UIElement = match inner {
        Some(h) => element(ctx, h)?.cast().map_err(w)?,
        None => el.clone(),
    };
    let handler = move |ev: Ev| {
        pms_xaml::PointerEventHandler::new(move |_, _| {
            let _ = std::panic::catch_unwind(|| {
                with_glow(key, |g| {
                    if let Source::Events { over, pressed, .. } = &mut g.source {
                        match ev {
                            Ev::Entered => *over = true,
                            Ev::Exited => *over = false,
                            Ev::Pressed => {
                                *pressed = true;
                                // A click may start the app, whose icon then changes: look again as it does.
                                later(key, 1500);
                                later(key, 4000);
                            }
                            Ev::Released | Ev::Lost => *pressed = false,
                            Ev::Canceled => (*over, *pressed) = (false, false),
                        }
                    }
                });
                update(key);
            });
            Ok(())
        })
    };
    let tokens = vec![
        (Ev::Entered, el.PointerEntered(&handler(Ev::Entered)).map_err(w)?),
        (Ev::Exited, el.PointerExited(&handler(Ev::Exited)).map_err(w)?),
        (Ev::Lost, el.PointerCaptureLost(&handler(Ev::Lost)).map_err(w)?),
        (Ev::Canceled, el.PointerCanceled(&handler(Ev::Canceled)).map_err(w)?),
        (Ev::Pressed, inner.PointerPressed(&handler(Ev::Pressed)).map_err(w)?),
        (Ev::Released, inner.PointerReleased(&handler(Ev::Released)).map_err(w)?),
    ];
    Ok(Source::Events { inner, tokens, over: false, pressed: false })
}

/// The Image named Icon inside a taskbar button.
fn icon_of(button: InstanceHandle) -> Option<InstanceHandle> {
    let tree = lock(&TREE);
    let under = |mut h: InstanceHandle| {
        for _ in 0..32 {
            let Some(n) = tree.get(&h) else { return false };
            if n.parent == button {
                return true;
            }
            h = n.parent;
        }
        false
    };
    tree.iter().find(|(h, n)| n.name == "Icon" && under(**h)).map(|(h, _)| *h)
}

fn paint(c: &Compositor, brush: &CompositionRadialGradientBrush, color: Color) -> R<()> {
    let stops = brush.ColorStops().map_err(w)?;
    stops.Clear().map_err(w)?;
    for (at, alpha) in [(0.0, 0xC0), (0.25, 0x80), (0.5, 0x38), (0.75, 0x10), (1.0, 0)] {
        stops.Append(&c.CreateColorGradientStopWithOffsetAndColor(at, Color { A: alpha, ..color }).map_err(w)?).map_err(w)?;
    }
    Ok(())
}

/// A button's state changed: head for the glow that goes with it.
fn update(key: Key) {
    GLOWS.with(|cell| {
        let Ok(mut state) = cell.try_borrow_mut() else { return };
        let Some(g) = state.as_mut() else { return };
        let compositor = g.compositor.clone();
        let host = g.host.clone();
        let (glow, feel) = match key {
            Key::Start => match g.start.as_mut() {
                Some((glow, feel)) => (glow, *feel),
                None => return,
            },
            Key::App(h) => match (g.app.get_mut(&h), g.apps) {
                (Some(glow), Some(feel)) => (glow, feel),
                _ => return,
            },
        };
        if let Err(e) = follow(&compositor, &host, glow, feel, key) {
            crate::log(format!("button glow: {e}"));
        }
    });
}

fn follow(c: &Compositor, host: &UIElement, glow: &mut Glow, feel: Feel, key: Key) -> R<()> {
    let (over, pressed, open) = match &glow.source {
        Source::Button { button, .. } => (
            button.IsPointerOver().map_err(w)?,
            button.IsPressed().map_err(w)?,
            button.cast::<ToggleButton>().ok().and_then(|t| t.IsChecked().ok()).and_then(|c| c.Value().ok()).unwrap_or(false),
        ),
        Source::Events { over, pressed, .. } => (*over, *pressed, false),
    };
    let (level, radius) = if pressed {
        (feel.press, feel.press_radius)
    } else if open {
        (feel.open.max(if over { feel.hover } else { 0.0 }), feel.hover_radius)
    } else if over {
        (feel.hover, feel.hover_radius)
    } else {
        (0.0, feel.hover_radius)
    };
    // Pressing and letting go are quick; coming and going with the pointer, slower.
    let ms = if pressed || glow.level == feel.press { feel.press_ms } else { feel.hover_ms };
    if glow.level == 0.0 && level > 0.0 {
        place(glow, host, feel)?;
        if !glow.colored && glow.rendering.is_none() {
            take_color(key, glow)?;
        }
    }
    glow.level = level;
    let most = feel.hover_radius.max(feel.press_radius).max(1.0);
    let duration = windows::Foundation::TimeSpan { Duration: ms.max(1) * 10_000 };
    let fade = c.CreateScalarKeyFrameAnimation().map_err(w)?;
    fade.InsertKeyFrame(1.0, level).map_err(w)?;
    fade.SetDuration(duration).map_err(w)?;
    glow.sprite.StartAnimation(&HSTRING::from("Opacity"), &fade).map_err(w)?;
    let grow = c.CreateVector3KeyFrameAnimation().map_err(w)?;
    let k = radius / most;
    grow.InsertKeyFrame(1.0, Vector3 { X: k, Y: k, Z: 1.0 }).map_err(w)?;
    grow.SetDuration(duration).map_err(w)?;
    glow.sprite.StartAnimation(&HSTRING::from("Scale"), &grow).map_err(w)
}

/// Center the glow on its button, wherever the taskbar has it now.
fn place(glow: &Glow, host: &UIElement, feel: Feel) -> R<()> {
    let fe: FrameworkElement = glow.element.cast().map_err(w)?;
    let r = bounds(&glow.element, fe.ActualWidth().map_err(w)?, fe.ActualHeight().map_err(w)?)?;
    let origin = bounds(host, 0.0, 0.0)?;
    let most = feel.hover_radius.max(feel.press_radius).max(1.0);
    let center = Vector2 { X: r.X + r.Width / 2.0 - origin.X, Y: r.Y + r.Height / 2.0 - origin.Y };
    glow.sprite.SetOffset(Vector3 { X: center.X - most, Y: center.Y - most, Z: 0.0 }).map_err(w)
}

/// Take an app's color from its icon, as the taskbar draws it: render it, read the pixels, find its most
/// prominent color (ripple.rs). Both steps finish later, on this thread.
fn take_color(key: Key, glow: &mut Glow) -> R<()> {
    let Some(icon) = glow.icon.clone() else { return Ok(()) };
    let rtb = RenderTargetBitmap::new().map_err(w)?;
    let render = rtb.RenderAsync(&icon).map_err(w)?;
    glow.rendering = Some(rtb);
    render
        .SetCompleted(&AsyncActionCompletedHandler::new(move |_, status| {
            let _ = std::panic::catch_unwind(|| rendered(key, status));
            Ok(())
        }))
        .map_err(w)
}

fn rendered(key: Key, status: AsyncStatus) {
    let rtb = with_glow(key, |g| g.rendering.clone()).flatten();
    let Some(rtb) = rtb.filter(|_| status == AsyncStatus::Completed) else { return };
    let r = rtb.GetPixelsAsync().and_then(|op| {
        op.SetCompleted(&AsyncOperationCompletedHandler::new(move |op, status| {
            let _ = std::panic::catch_unwind(|| {
                let pixels = op.as_ref().filter(|_| status == AsyncStatus::Completed).and_then(|op| op.GetResults().ok());
                let color = pixels.and_then(|buf: windows::Storage::Streams::IBuffer| {
                    let reader = DataReader::FromBuffer(&buf).ok()?;
                    let mut bytes = vec![0u8; buf.Length().ok()? as usize];
                    reader.ReadBytes(&mut bytes).ok()?;
                    crate::ripple::prominent_in(&bytes, 4, true)
                });
                with_glow(key, |g| {
                    g.rendering = None;
                    if let Some(color) = color {
                        g.colored = true;
                        let c = g.sprite.Compositor().ok();
                        if let Some(c) = c {
                            let _ = paint(&c, &g.brush, color);
                        }
                    }
                });
            });
            Ok(())
        }))
    });
    if let Err(e) = r {
        crate::log(format!("app glow color: {e}"));
    }
}

fn with_glow<T>(key: Key, f: impl FnOnce(&mut Glow) -> T) -> Option<T> {
    GLOWS.with(|cell| {
        let mut state = cell.try_borrow_mut().ok()?;
        let g = state.as_mut()?;
        match key {
            Key::Start => g.start.as_mut().map(|(glow, _)| f(glow)),
            Key::App(h) => g.app.get_mut(&h).map(f),
        }
    })
}

/// The taskbar's buttons changed: glows for new app buttons, none for gone ones; and lit glows follow their buttons,
/// moved, resized or hidden. A button the taskbar hides from under the pointer (a pinned app's, when the app starts
/// and a button of its window takes its place) never hears the pointer leave, so its glow goes out here.
pub fn on_layout(ctx: &Ctx) {
    follow_buttons();
    add_and_drop(ctx);
}

fn follow_buttons() {
    let lit: Vec<Key> = GLOWS.with_borrow(|g| {
        let Some(g) = g.as_ref() else { return Vec::new() };
        let start = g.start.as_ref().filter(|(glow, _)| glow.level > 0.0).map(|_| Key::Start);
        start.into_iter().chain(g.app.iter().filter(|(_, glow)| glow.level > 0.0).map(|(h, _)| Key::App(*h))).collect()
    });
    for key in lit {
        let shown = with_glow(key, |g| {
            let fe = g.element.cast::<FrameworkElement>().ok();
            let visible = g.element.Visibility().is_ok_and(|v| v == pms_xaml::Visibility::Visible);
            let wide = fe.and_then(|f| f.ActualWidth().ok()).is_some_and(|w| w > 0.0);
            let shown = visible && wide && bounds(&g.element, 1.0, 1.0).is_ok_and(|r| r.Width > 0.0);
            if !shown && let Source::Events { over, pressed, .. } = &mut g.source {
                (*over, *pressed) = (false, false);
            }
            shown
        });
        match shown {
            Some(true) => {
                // Still there: keep the glow centered on it (it may have moved or grown).
                GLOWS.with(|cell| {
                    let Ok(state) = cell.try_borrow() else { return };
                    let Some(g) = state.as_ref() else { return };
                    let (glow, feel) = match key {
                        Key::Start => match &g.start {
                            Some((glow, feel)) => (glow, *feel),
                            None => return,
                        },
                        Key::App(h) => match (g.app.get(&h), g.apps) {
                            (Some(glow), Some(feel)) => (glow, feel),
                            _ => return,
                        },
                    };
                    let _ = place(glow, &g.host, feel);
                });
            }
            Some(false) => update(key),
            None => {}
        }
    }
}

fn add_and_drop(ctx: &Ctx) {
    let Some(feel) = GLOWS.with_borrow(|g| g.as_ref().and_then(|g| g.apps)) else { return };
    let now: Vec<InstanceHandle> = matches("TaskListButton").into_iter().map(|(h, _)| h).collect();
    let (gone, new): (Vec<InstanceHandle>, Vec<InstanceHandle>) = GLOWS.with_borrow(|g| {
        let g = g.as_ref().expect("glows");
        let new = now.iter().filter(|h| !g.app.contains_key(h) && !g.failed.contains(h)).copied().collect();
        (g.app.keys().filter(|h| !now.contains(h)).copied().collect(), new)
    });
    for h in gone {
        if let Some(glow) = GLOWS.with_borrow_mut(|g| g.as_mut().and_then(|g| g.app.remove(&h))) {
            drop_glow(glow);
        }
    }
    for h in new {
        match watch(ctx, Key::App(h), h, feel, None) {
            Ok(glow) => {
                GLOWS.with_borrow_mut(|g| g.as_mut().map(|g| g.app.insert(h, glow)));
            }
            Err(e) => {
                crate::log(format!("app glow for {h:#x}: {e}"));
                GLOWS.with_borrow_mut(|g| g.as_mut().map(|g| g.failed.insert(h)));
            }
        }
    }
}

fn drop_glow(mut glow: Glow) {
    unwatch(&mut glow);
    match &glow.source {
        Source::Button { button, tokens } => {
            if let Ok(d) = button.cast::<DependencyObject>() {
                for (dp, token) in tokens {
                    let _ = d.UnregisterPropertyChangedCallback(dp, *token);
                }
            }
        }
        Source::Events { inner, tokens, .. } => {
            let el = &glow.element;
            for &(ev, token) in tokens {
                let _ = match ev {
                    Ev::Entered => el.RemovePointerEntered(token),
                    Ev::Exited => el.RemovePointerExited(token),
                    Ev::Lost => el.RemovePointerCaptureLost(token),
                    Ev::Canceled => el.RemovePointerCanceled(token),
                    Ev::Pressed => inner.RemovePointerPressed(token),
                    Ev::Released => inner.RemovePointerReleased(token),
                };
            }
        }
    }
    if let Ok(parent) = glow.sprite.Parent() {
        let _ = parent.Children().and_then(|c| c.Remove(&glow.sprite));
    }
}

fn stop_start() {
    if let Some((glow, _)) = GLOWS.with_borrow_mut(|g| g.as_mut().and_then(|g| g.start.take())) {
        drop_glow(glow);
    }
}

fn stop_apps() {
    let apps: Vec<Glow> = GLOWS.with_borrow_mut(|g| g.as_mut().map(|g| {
        g.apps = None;
        g.app.drain().map(|(_, v)| v).collect()
    }))
    .unwrap_or_default();
    apps.into_iter().for_each(drop_glow);
}

pub(crate) fn off(ctx: &Ctx) -> R<String> {
    stop_start();
    stop_apps();
    if let Some(g) = GLOWS.with_borrow_mut(Option::take) {
        stage::remove(ctx, &g.host)?;
    }
    Ok("button glows off".into())
}
