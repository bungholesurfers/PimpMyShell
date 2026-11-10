//! The PIMPBOX's own tooltip, beside a TV's remote while the TV's on the console's channel: whether it's on, how to play, and
//! its settings: what it runs (POOM, so far), which game that plays, and how big a picture it puts out. It comes up in the remote's plastic. Put the
//! pointer on it, with the console on, and a beam of light goes across it and leaves it in the console's own colors: the
//! same window, another scheme, as a desktop of the day could be given. It stays that way until the console's switched off,
//! when the beam takes it back. There's another of it beside the console's display, when that's out on the desktop
//! (`remote::dock`): that one's in the console's colors whenever the console's on, pointer or none, and the beam's only
//! ever seen, not missed: it waits until the tooltip's there to be looked at.
//!
//! It's the one window built twice, a copy in each scheme, one over the other; the upper is cut off at the beam (a clip on
//! its visual, which the compositor moves: nothing here runs while it does). UI thread only.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pms_screen::Button;
use pms_xaml::{Border, ElementCompositionPreview, FontFamily, GradientStop, Grid, LinearGradientBrush, Orientation, PointerEventHandler, Popup, SolidColorBrush, StackPanel, TextBlock, Thickness, UIElement, XamlRoot};
use windows::Foundation::{Point, TimeSpan};
use windows::UI::Color;
use windows::UI::Composition::{CompositionObject, InsetClip};
use windows_core::{HSTRING, Interface};
use windows_numerics::Vector3;

use crate::commands::R;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { A: 0xFF, R: r, G: g, B: b }
}

/// A scheme: the colors a window of the day was made of.
struct Look {
    face: Color,
    light: Color,
    shade: Color,
    dark: Color,
    title: Color,
    title_end: Color,
    title_text: Color,
    text: Color,
    glass: Color,
    glass_text: Color,
    led_on: Color,
    led_off: Color,
}

/// The remote's plastic.
const PLASTIC: Look = Look {
    face: rgb(0xC0, 0xC0, 0xC0),
    light: rgb(0xFF, 0xFF, 0xFF),
    shade: rgb(0x80, 0x80, 0x80),
    dark: rgb(0x00, 0x00, 0x00),
    title: rgb(0x00, 0x00, 0x80),
    title_end: rgb(0x10, 0x84, 0xD0),
    title_text: rgb(0xFF, 0xFF, 0xFF),
    text: rgb(0x00, 0x00, 0x00),
    glass: rgb(0x10, 0x20, 0x10),
    glass_text: rgb(0x40, 0xFF, 0x60),
    led_on: rgb(0x30, 0xE0, 0x50),
    led_off: rgb(0x50, 0x30, 0x30),
};

/// The console's own: black, and an acid green.
const GREEN: Look = Look {
    face: rgb(0x12, 0x20, 0x0E),
    light: rgb(0x7C, 0xD0, 0x30),
    shade: rgb(0x06, 0x0E, 0x04),
    dark: rgb(0x00, 0x00, 0x00),
    title: rgb(0x0C, 0x5C, 0x0C),
    title_end: rgb(0x9B, 0xE4, 0x00),
    title_text: rgb(0xFF, 0xFF, 0xFF),
    text: rgb(0xC4, 0xF0, 0x90),
    glass: rgb(0x02, 0x08, 0x02),
    glass_text: rgb(0x9B, 0xE4, 0x00),
    led_on: rgb(0xD8, 0xFF, 0x60),
    led_off: rgb(0x1C, 0x30, 0x12),
};

/// How wide it is, and how long the beam takes to cross it.
pub(crate) const WIDTH: f64 = 176.0;
const BEAM_MS: i64 = 380;
/// The keys for how big a picture the console puts out: what's on each, the button it presses, and what the console calls it.
const VIDEO: [(&str, Button, &str); 2] = [("480p", Button::Video480, "480p"), ("720p", Button::Video720, "720p")];

fn w(e: windows_core::Error) -> String {
    e.to_string()
}

fn ui<T: Interface>(e: &T) -> R<UIElement> {
    e.cast().map_err(w)
}

fn brush(c: Color) -> R<SolidColorBrush> {
    SolidColorBrush::CreateInstanceWithColor(c).map_err(w)
}

fn thickness(left: f64, top: f64, right: f64, bottom: f64) -> Thickness {
    Thickness { Left: left, Top: top, Right: right, Bottom: bottom }
}

fn label(text: &str, size: f64, color: Color) -> R<TextBlock> {
    let t = TextBlock::new().map_err(w)?;
    t.SetText(&HSTRING::from(text)).map_err(w)?;
    t.SetFontSize(size).map_err(w)?;
    t.SetFontFamily(&FontFamily::CreateInstanceWithName(&HSTRING::from("Tahoma, Segoe UI")).map_err(w)?).map_err(w)?;
    t.SetForeground(&brush(color)?).map_err(w)?;
    Ok(t)
}

/// One pixel of a bevel: `first` above and left, `second` below and right.
fn edge(first: Color, second: Color, child: &UIElement) -> R<(Border, Border)> {
    let inner = Border::new().map_err(w)?;
    inner.SetBorderBrush(&brush(first)?).map_err(w)?;
    inner.SetBorderThickness(thickness(1.0, 1.0, 0.0, 0.0)).map_err(w)?;
    inner.SetChild(child).map_err(w)?;
    let outer = Border::new().map_err(w)?;
    outer.SetBorderBrush(&brush(second)?).map_err(w)?;
    outer.SetBorderThickness(thickness(0.0, 0.0, 1.0, 1.0)).map_err(w)?;
    outer.SetChild(&ui(&inner)?).map_err(w)?;
    Ok((outer, inner))
}

/// The two-tone raised edge of the day's windows and buttons: a light pixel inside a dark one.
fn raised(look: &Look, child: &UIElement) -> R<(Border, [Border; 4])> {
    let face = Border::new().map_err(w)?;
    face.SetBackground(&brush(look.face)?).map_err(w)?;
    face.SetChild(child).map_err(w)?;
    let (shade, light) = edge(look.light, look.shade, &ui(&face)?)?;
    let (outer, top) = edge(look.face, look.dark, &ui(&shade)?)?;
    Ok((outer.clone(), [outer, top, shade, light]))
}

/// The same edge the other way in: a thing set into the panel.
fn sunken(look: &Look, child: &UIElement) -> R<Border> {
    let (inner, _) = edge(look.dark, look.face, child)?;
    let (outer, _) = edge(look.shade, look.light, &ui(&inner)?)?;
    Ok(outer)
}

fn set_edge(colors: [Color; 4], parts: &[Border; 4], pushed: bool) -> R<()> {
    let [face, light, shade, dark] = colors;
    let [outer, top, low, high] = parts;
    let (o, t, s, l) = if pushed { (light, dark, face, shade) } else { (dark, face, shade, light) };
    outer.SetBorderBrush(&brush(o)?).map_err(w)?;
    top.SetBorderBrush(&brush(t)?).map_err(w)?;
    low.SetBorderBrush(&brush(s)?).map_err(w)?;
    high.SetBorderBrush(&brush(l)?).map_err(w)
}

/// Something to press: `inside` on a raised face `width` wide, which goes in when it's pressed, and presses `press` on the
/// set's screen, if there's anything for it to press.
fn pressable(look: &Look, inside: &UIElement, width: f64, press: Option<Button>) -> R<Border> {
    let padded = Border::new().map_err(w)?;
    padded.SetPadding(thickness(4.0, 2.0, 4.0, 2.0)).map_err(w)?;
    padded.SetChild(inside).map_err(w)?;
    let (b, parts) = raised(look, &ui(&padded)?)?;
    b.SetWidth(width).map_err(w)?;
    b.SetMargin(thickness(0.0, 0.0, 3.0, 3.0)).map_err(w)?;
    let el = ui(&b)?;
    let colors = [look.face, look.light, look.shade, look.dark];
    let down = parts.clone();
    el.PointerPressed(&PointerEventHandler::new(move |_, _| {
        let _ = set_edge(colors, &down, true);
        Ok(())
    }))
    .map_err(w)?;
    let up = parts.clone();
    // Which remote this tooltip's beside, and so whose set its keys are for.
    let docked = crate::remote::building_docked();
    el.PointerReleased(&PointerEventHandler::new(move |_, _| {
        let _ = set_edge(colors, &up, false);
        if let Some(press) = press {
            crate::remote::pressed_from(press, docked);
        }
        Ok(())
    }))
    .map_err(w)?;
    el.PointerExited(&PointerEventHandler::new(move |_, _| {
        let _ = set_edge(colors, &parts, false);
        Ok(())
    }))
    .map_err(w)?;
    Ok(b)
}

/// A key with a light on it.
fn key(look: &Look, text: &str, press: Button) -> R<(Border, Border)> {
    let light = Border::new().map_err(w)?;
    light.SetWidth(7.0).map_err(w)?;
    light.SetHeight(7.0).map_err(w)?;
    light.SetBackground(&brush(look.led_off)?).map_err(w)?;
    light.SetBorderBrush(&brush(look.shade)?).map_err(w)?;
    light.SetBorderThickness(thickness(1.0, 1.0, 1.0, 1.0)).map_err(w)?;
    light.SetMargin(thickness(0.0, 0.0, 4.0, 0.0)).map_err(w)?;
    light.SetVerticalAlignment(pms_xaml::VerticalAlignment::Center).map_err(w)?;
    let row = StackPanel::new().map_err(w)?;
    row.SetOrientation(Orientation::Horizontal).map_err(w)?;
    row.SetHorizontalAlignment(pms_xaml::HorizontalAlignment::Center).map_err(w)?;
    row.Children().map_err(w)?.Append(&ui(&light)?).map_err(w)?;
    row.Children().map_err(w)?.Append(&ui(&label(text, 10.0, look.text)?)?).map_err(w)?;
    Ok((pressable(look, &ui(&row)?, 76.0, Some(press))?, light))
}

/// Something to choose one of: what it is, said above it; which it is now, on a little screen; and a key each side of that
/// for the one before and the next (keys with nothing to press, where there's only the one to choose).
fn chooser(look: &Look, what: &str, now: &TextBlock, before: Option<Button>, next: Option<Button>) -> R<[UIElement; 2]> {
    let said = label(what, 9.0, look.text)?;
    said.SetMargin(thickness(1.0, 0.0, 0.0, 2.0)).map_err(w)?;
    now.SetFontFamily(&FontFamily::CreateInstanceWithName(&HSTRING::from("Cascadia Mono, Consolas")).map_err(w)?).map_err(w)?;
    now.SetTextTrimming(pms_xaml::TextTrimming::CharacterEllipsis).map_err(w)?;
    let glass = Border::new().map_err(w)?;
    glass.SetBackground(&brush(look.glass)?).map_err(w)?;
    glass.SetPadding(thickness(4.0, 2.0, 4.0, 2.0)).map_err(w)?;
    glass.SetWidth(104.0).map_err(w)?;
    glass.SetChild(&ui(now)?).map_err(w)?;
    let screen = sunken(look, &ui(&glass)?)?;
    screen.SetMargin(thickness(0.0, 0.0, 3.0, 3.0)).map_err(w)?;
    let row = StackPanel::new().map_err(w)?;
    row.SetOrientation(Orientation::Horizontal).map_err(w)?;
    for part in [ui(&pressable(look, &ui(&label("<", 10.0, look.text)?)?, 20.0, before)?)?, ui(&screen)?, ui(&pressable(look, &ui(&label(">", 10.0, look.text)?)?, 20.0, next)?)?] {
        row.Children().map_err(w)?.Append(&part).map_err(w)?;
    }
    Ok([ui(&said)?, ui(&row)?])
}

/// The window in one scheme: all of it, and the parts of it that change.
struct Copy {
    whole: UIElement,
    look: &'static Look,
    game: TextBlock,
    state: TextBlock,
    help: TextBlock,
    lights: Vec<Border>,
}

fn copy(look: &'static Look) -> R<Copy> {
    let title = label("PIMPBOX", 10.0, look.title_text)?;
    title.SetFontWeight(windows::UI::Text::FontWeight { Weight: 700 }).map_err(w)?;
    let gradient = LinearGradientBrush::new().map_err(w)?;
    gradient.SetStartPoint(Point { X: 0.0, Y: 0.5 }).map_err(w)?;
    gradient.SetEndPoint(Point { X: 1.0, Y: 0.5 }).map_err(w)?;
    for (color, at) in [(look.title, 0.0), (look.title_end, 1.0)] {
        let stop = GradientStop::new().map_err(w)?;
        stop.SetColor(color).map_err(w)?;
        stop.SetOffset(at).map_err(w)?;
        gradient.GradientStops().map_err(w)?.Append(&stop).map_err(w)?;
    }
    let bar = Border::new().map_err(w)?;
    bar.SetBackground(&gradient.cast::<pms_xaml::Brush>().map_err(w)?).map_err(w)?;
    bar.SetPadding(thickness(4.0, 2.0, 4.0, 2.0)).map_err(w)?;
    bar.SetMargin(thickness(2.0, 2.0, 2.0, 0.0)).map_err(w)?;
    bar.SetChild(&ui(&title)?).map_err(w)?;

    // What it runs, and which game that plays: each to be chosen.
    let program = label("POOM (ROOM)", 10.0, look.glass_text)?;
    let game = label("", 10.0, look.glass_text)?;
    let [program_said, program_row] = chooser(look, "PROGRAM", &program, None, None)?;
    let [game_said, game_row] = chooser(look, "GAME", &game, Some(Button::GamePrev), Some(Button::GameNext))?;

    let state = label("", 10.0, look.text)?;
    state.SetTextWrapping(pms_xaml::TextWrapping::Wrap).map_err(w)?;
    state.SetMargin(thickness(1.0, 0.0, 3.0, 4.0)).map_err(w)?;

    // Its settings: how big a picture it puts out.
    let video = label("VIDEO", 9.0, look.text)?;
    video.SetMargin(thickness(1.0, 0.0, 0.0, 2.0)).map_err(w)?;
    let keys = StackPanel::new().map_err(w)?;
    keys.SetOrientation(Orientation::Horizontal).map_err(w)?;
    let mut lights = Vec::new();
    for (text, press, _) in VIDEO {
        let (b, light) = key(look, text, press)?;
        keys.Children().map_err(w)?.Append(&ui(&b)?).map_err(w)?;
        lights.push(light);
    }

    let help = label("", 10.0, look.text)?;
    help.SetTextWrapping(pms_xaml::TextWrapping::Wrap).map_err(w)?;
    help.SetMargin(thickness(1.0, 2.0, 3.0, 2.0)).map_err(w)?;

    let body = StackPanel::new().map_err(w)?;
    body.SetMargin(thickness(5.0, 5.0, 2.0, 4.0)).map_err(w)?;
    for part in [ui(&state)?, program_said, program_row, game_said, game_row, ui(&video)?, ui(&keys)?, ui(&help)?] {
        body.Children().map_err(w)?.Append(&part).map_err(w)?;
    }
    let window = StackPanel::new().map_err(w)?;
    window.Children().map_err(w)?.Append(&ui(&bar)?).map_err(w)?;
    window.Children().map_err(w)?.Append(&ui(&body)?).map_err(w)?;
    let (whole, _) = raised(look, &ui(&window)?)?;
    whole.SetWidth(WIDTH).map_err(w)?;
    Ok(Copy { whole: ui(&whole)?, look, game, state, help, lights })
}

/// The tooltip.
pub(crate) struct Tooltip {
    pub(crate) popup: Popup,
    copies: [Copy; 2],
    clip: InsetClip,
    beam: UIElement,
    /// It's in the console's colors.
    greened: Arc<AtomicBool>,
    /// The console's on, as far as it was last told.
    console_on: Arc<AtomicBool>,
    /// It's the one beside the console's display: its colors go by the console, not by the pointer.
    docked: bool,
}

fn ms(n: i64) -> TimeSpan {
    TimeSpan { Duration: n * 10_000 }
}

/// Moves `o`'s `prop` evenly from one value to another, over as long as the beam takes.
fn run<T: Interface>(o: &T, prop: &str, frames: &[(f32, f32)]) -> R<()> {
    let o: CompositionObject = o.cast().map_err(w)?;
    let c = o.Compositor().map_err(w)?;
    let a = c.CreateScalarKeyFrameAnimation().map_err(w)?;
    let even = c.CreateLinearEasingFunction().map_err(w)?;
    for (at, value) in frames {
        a.InsertKeyFrameWithEasingFunction(*at, *value, &even).map_err(w)?;
    }
    a.SetDuration(ms(BEAM_MS)).map_err(w)?;
    o.StartAnimation(&HSTRING::from(prop), &a).map_err(w)
}

/// The beam goes across, left to right, and what's behind it is in the other scheme: the console's, or the plastic again.
fn sweep(clip: &InsetClip, beam: &UIElement, to_green: bool) -> R<()> {
    let wide = WIDTH as f32;
    let o: CompositionObject = clip.cast().map_err(w)?;
    o.StopAnimation(&HSTRING::from("LeftInset")).map_err(w)?;
    o.StopAnimation(&HSTRING::from("RightInset")).map_err(w)?;
    if to_green {
        // The upper copy's there from its left edge as far as the beam has got.
        clip.SetLeftInset(0.0).map_err(w)?;
        run(clip, "RightInset", &[(0.0, wide), (1.0, 0.0)])?;
    } else {
        // And it's gone from its left edge as far as the beam has got.
        clip.SetRightInset(0.0).map_err(w)?;
        run(clip, "LeftInset", &[(0.0, 0.0), (1.0, wide)])?;
    }
    let v = ElementCompositionPreview::GetElementVisual(beam).map_err(w)?;
    run(&v, "Translation.X", &[(0.0, -8.0), (1.0, wide - 8.0)])?;
    run(&v, "Opacity", &[(0.0, 1.0), (0.9, 1.0), (1.0, 0.0)])
}

/// As it is with nothing moving: all in one scheme or the other.
fn settle(clip: &InsetClip, beam: &UIElement, green: bool) -> R<()> {
    let o: CompositionObject = clip.cast().map_err(w)?;
    o.StopAnimation(&HSTRING::from("LeftInset")).map_err(w)?;
    o.StopAnimation(&HSTRING::from("RightInset")).map_err(w)?;
    clip.SetLeftInset(0.0).map_err(w)?;
    clip.SetRightInset(if green { 0.0 } else { WIDTH as f32 }).map_err(w)?;
    let v = ElementCompositionPreview::GetElementVisual(beam).map_err(w)?;
    v.StopAnimation(&HSTRING::from("Opacity")).map_err(w)?;
    v.SetOpacity(0.0).map_err(w)
}

/// The tooltip. `docked`: the one beside the console's display.
pub(crate) fn build(root: &XamlRoot, docked: bool) -> R<Tooltip> {
    let copies = [copy(&PLASTIC)?, copy(&GREEN)?];
    // The beam: a thin upright line of light, and a wider, fainter one round it.
    let line = Border::new().map_err(w)?;
    line.SetWidth(3.0).map_err(w)?;
    line.SetBackground(&brush(rgb(0xF0, 0xFF, 0xD0))?).map_err(w)?;
    let glow = Border::new().map_err(w)?;
    glow.SetWidth(16.0).map_err(w)?;
    glow.SetHorizontalAlignment(pms_xaml::HorizontalAlignment::Left).map_err(w)?;
    glow.SetBackground(&brush(Color { A: 0x50, R: 0x9B, G: 0xE4, B: 0x00 })?).map_err(w)?;
    glow.SetChild(&ui(&line)?).map_err(w)?;
    glow.SetIsHitTestVisible(false).map_err(w)?;
    let beam = ui(&glow)?;

    let layers = Grid::new().map_err(w)?;
    layers.SetWidth(WIDTH).map_err(w)?;
    for layer in [&copies[0].whole, &copies[1].whole, &beam] {
        layers.Children().map_err(w)?.Append(layer).map_err(w)?;
    }
    let green = ElementCompositionPreview::GetElementVisual(&copies[1].whole).map_err(w)?;
    let clip = green.Compositor().and_then(|c| c.CreateInsetClip()).map_err(w)?;
    green.SetClip(&clip).map_err(w)?;
    ElementCompositionPreview::SetIsTranslationEnabled(&beam, true).map_err(w)?;
    ElementCompositionPreview::GetElementVisual(&beam).and_then(|v| v.Properties()).and_then(|p| p.InsertVector3(&HSTRING::from("Translation"), Vector3 { X: -8.0, Y: 0.0, Z: 0.0 })).map_err(w)?;
    settle(&clip, &beam, false)?;
    let (greened, console_on) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));

    let el = ui(&layers)?;
    crate::remote::follow_pointer(&el)?;
    // The pointer on it, with the console on: the beam, once.
    if !docked {
        let (its_clip, its_beam, its_green, on) = (clip.clone(), beam.clone(), greened.clone(), console_on.clone());
        el.PointerEntered(&PointerEventHandler::new(move |_, _| {
            if on.load(Ordering::Relaxed) && !its_green.swap(true, Ordering::Relaxed) {
                let _ = sweep(&its_clip, &its_beam, true);
            }
            Ok(())
        }))
        .map_err(w)?;
    }
    let popup = Popup::new().map_err(w)?;
    popup.SetChild(&el).map_err(w)?;
    popup.SetShouldConstrainToRootBounds(false).map_err(w)?;
    popup.SetXamlRoot(root).map_err(w)?;
    Ok(Tooltip { popup, copies, clip, beam, greened, console_on, docked })
}

impl Tooltip {
    /// What it says: the game, how the console is, what to do, and which video mode it's in. `on`: the console's on. Switched
    /// off, it goes back to the plastic: with the beam, if it's there to be seen.
    pub(crate) fn say(&self, game: &str, state: &str, help: &str, video: &str, on: bool) -> R<()> {
        for copy in &self.copies {
            copy.game.SetText(&HSTRING::from(game)).map_err(w)?;
            copy.state.SetText(&HSTRING::from(state)).map_err(w)?;
            copy.help.SetText(&HSTRING::from(help)).map_err(w)?;
            for (light, (_, _, name)) in copy.lights.iter().zip(VIDEO) {
                light.SetBackground(&brush(if name == video { copy.look.led_on } else { copy.look.led_off })?).map_err(w)?;
            }
        }
        self.console_on.store(on, Ordering::Relaxed);
        let open = self.popup.IsOpen().unwrap_or(false);
        if !on && self.greened.swap(false, Ordering::Relaxed) {
            if open {
                sweep(&self.clip, &self.beam, false)?;
            } else {
                settle(&self.clip, &self.beam, false)?;
            }
        } else if on && self.docked && open && !self.greened.swap(true, Ordering::Relaxed) {
            // Beside the display, it's the console being on that does it: but only once it's there to be seen.
            sweep(&self.clip, &self.beam, true)?;
        }
        Ok(())
    }
}
