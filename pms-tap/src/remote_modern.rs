//! A Twatch TV's remote: the same idea as remote.rs, for a newer set. What's on and how well, what the set will do next and how it
//! decides that, power, sound, channel up and down, the highest quality, and the favorites to go straight to. All of what it shows
//! comes from the screen's `info`, which the set keeps up to date. For a button beside the sets, it's only the logo and what the
//! button's for; and for the A, how many TwaTVs may come by themselves, with keys for more and fewer, and a switch for whether
//! they do, every word of it from the A's `info` too.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use pms_screen::Button;
use pms_xaml::{Border, CornerRadius, DoubleTappedEventHandler, HorizontalAlignment, Orientation, PointerEventHandler, Popup, SolidColorBrush, StackPanel, TappedEventHandler, TextBlock, TextTrimming, TextWrapping, Thickness, UIElement, VerticalAlignment, Visibility, XamlRoot};
use windows::UI::Color;
use windows_core::{HSTRING, Interface};

use crate::commands::R;
use crate::said::{Said, t};

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { A: 0xFF, R: r, G: g, B: b }
}

const BODY: Color = rgb(0x15, 0x17, 0x1C);
const EDGE: Color = rgb(0x34, 0x38, 0x42);
const KEY: Color = rgb(0x25, 0x28, 0x30);
const KEY_OVER: Color = rgb(0x32, 0x36, 0x40);
const KEY_DOWN: Color = rgb(0x1B, 0x1D, 0x23);
const TEXT: Color = rgb(0xF2, 0xF3, 0xF5);
const QUIET: Color = rgb(0x9A, 0xA0, 0xAC);
const ACCENT: Color = rgb(0xFF, 0x96, 0x28);
const ACCENT_OVER: Color = rgb(0xFF, 0xAA, 0x50);
const ACCENT_DOWN: Color = rgb(0xE0, 0x80, 0x18);
const LIVE: Color = rgb(0xE5, 0x3A, 0x3A);
/// Something's missing: a dark amber box, with the words in a lighter one.
const WARN: Color = rgb(0x3A, 0x2C, 0x12);
const WARN_EDGE: Color = rgb(0x8A, 0x63, 0x1E);
const WARN_TEXT: Color = rgb(0xFF, 0xD6, 0x8A);
const NOT_LIVE: Color = rgb(0x4A, 0x4E, 0x58);
/// The light a set shows while its quality's lowered (pms's twatch/look.rs), for the words that say why.
const LOWERED: Color = rgb(0xFF, 0xB0, 0x20);
const CLEAR: Color = Color { A: 0, R: 0, G: 0, B: 0 };
const WIDTH: f64 = 257.0;
/// What's inside its edge (1) and padding (12) on each side, which the widest rows fill exactly: four 54-wide keys with three
/// gaps of 5, and a name, its value and two small keys (120 + 50 + 28 + 5 + 28). A row any wider is cut off at the right.
const INSIDE: f64 = WIDTH - 2.0 * (1.0 + 12.0);
/// How many favorites it lists.
const ROWS: usize = 12;
const REFRESH: Duration = Duration::from_millis(1500);

/// The logo, in pixels and on screen.
const LOGO_PIXELS: (u32, u32) = (176, 72);
const LOGO_SIZE: (f64, f64) = (88.0, 36.0);

const POWER: &str = "\u{E7E8}";
const SOUND_ON: &str = "\u{E767}";
const SOUND_OFF: &str = "\u{E74F}";
const UP: &str = "\u{E70E}";
const DOWN: &str = "\u{E70D}";
const PLUS: &str = "\u{E710}";
const MINUS: &str = "\u{E738}";

/// What each of the three ways does, said under them: for the one that's chosen, or the one the pointer's on.
const WAYS: [t::Id; 3] = [t::FOLLOW_SAYS, t::STAY_SAYS, t::ONLY_SAYS];
/// The key that takes this set off the taskbar (`t::NUKE`): it asks first (`t::SURE`), for a few seconds.
const STAYS_SURE: Duration = Duration::from_secs(3);

struct Remote {
    popup: Popup,
    /// What's on, and the controls: all a button beside the sets does without.
    head: StackPanel,
    controls: StackPanel,
    handle: TextBlock,
    state: TextBlock,
    light: Border,
    title: TextBlock,
    about: TextBlock,
    /// Why the quality's lowered, if it is.
    lowered: TextBlock,
    /// Where the picture comes from, and what's missing, if anything is.
    via: TextBlock,
    warn: Border,
    warn_words: TextBlock,
    /// What it'll do next.
    plan: TextBlock,
    power: Border,
    follow: Border,
    stay: Border,
    only: Border,
    /// What the way that's chosen, or pointed at, does.
    way_hint: TextBlock,
    nuke: TextBlock,
    top: TextBlock,
    volume: TextBlock,
    sound: TextBlock,
    heading: TextBlock,
    /// The heading and its key for another favorite; or, with none yet, the big key for the first.
    heading_row: StackPanel,
    first: Border,
    rows: Vec<(Border, Border, TextBlock)>,
    /// The A's: how many TwaTVs may come by themselves, the keys for fewer and more, and the switch for whether they do (what it's
    /// called, the track and its knob, and the word for on or off).
    sets: StackPanel,
    sets_name: TextBlock,
    /// The words built in, said again when the language changes.
    said: Said,
    sets_value: TextBlock,
    fewer: Border,
    more: Border,
    switch_name: TextBlock,
    track: Border,
    knob: Border,
    switch_state: TextBlock,
    source: String,
    /// Where it was put, to put it there again when it changes height: the root, and the cell's left, top and width.
    placed: Option<(XamlRoot, f64, f64, f64)>,
}

thread_local! {
    static REMOTE: RefCell<Option<Remote>> = const { RefCell::new(None) };
    /// What each row of the list does: tune to that favorite (its place in the list, from 1), or (None) fold or unfold the offline.
    static ROWS_DO: RefCell<Vec<Option<u32>>> = const { RefCell::new(Vec::new()) };
    /// The offline favorites are showing.
    static UNFOLDED: Cell<bool> = const { Cell::new(false) };
    /// Which of the three ways is chosen, and whether the pointer's on one.
    static CHOSEN: Cell<usize> = const { Cell::new(0) };
    static POINTED: Cell<Option<usize>> = const { Cell::new(None) };
    /// What the key the pointer's on does, said in the ways' place meanwhile.
    static EXPLAINING: Cell<Option<t::Id>> = const { Cell::new(None) };
    /// The remove key has been pressed once, and is asking.
    static ARMED: Cell<bool> = const { Cell::new(false) };
    /// The A's switch is on, as the A last said, and whether the pointer's on it.
    static SETS_ON: Cell<bool> = const { Cell::new(false) };
    static ON_SWITCH: Cell<bool> = const { Cell::new(false) };
}

/// Which time the remove key has asked: an older wait lets it be.
static ASKING: AtomicU32 = AtomicU32::new(0);

/// Which time it's been opened: its refreshing stops when this moves on.
static OPENED: AtomicU32 = AtomicU32::new(0);

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

fn corners(r: f64) -> CornerRadius {
    CornerRadius { TopLeft: r, TopRight: r, BottomRight: r, BottomLeft: r }
}

fn label(text: &str, size: f64, color: Color, font: &str) -> R<TextBlock> {
    let t = TextBlock::new().map_err(w)?;
    t.SetText(&HSTRING::from(text)).map_err(w)?;
    t.SetFontSize(size).map_err(w)?;
    t.SetFontFamily(&crate::stage::font(font)?).map_err(w)?;
    t.SetForeground(&brush(color)?).map_err(w)?;
    t.SetTextTrimming(TextTrimming::CharacterEllipsis).map_err(w)?;
    Ok(t)
}

fn words(text: &str, size: f64, color: Color) -> R<TextBlock> {
    label(text, size, color, "Segoe UI Variable Text, Segoe UI")
}

fn light(size: f64) -> R<Border> {
    let b = Border::new().map_err(w)?;
    b.SetWidth(size).map_err(w)?;
    b.SetHeight(size).map_err(w)?;
    b.SetCornerRadius(corners(size / 2.0)).map_err(w)?;
    b.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    b.SetMargin(thickness(0.0, 0.0, 7.0, 0.0)).map_err(w)?;
    Ok(b)
}

fn across(items: &[UIElement]) -> R<StackPanel> {
    let row = crate::stage::stack_panel()?;
    row.SetOrientation(Orientation::Horizontal).map_err(w)?;
    for item in items {
        row.Children().map_err(w)?.Append(item).map_err(w)?;
    }
    Ok(row)
}

/// Shows what the set says a moment after a press, when it has acted on it; and again a little later, for one that took longer.
fn soon() {
    std::thread::spawn(|| {
        for wait in [120, 300] {
            std::thread::sleep(Duration::from_millis(wait));
            crate::meters::on_ui(refresh);
        }
    });
}

/// Says `button` was pressed, to the set the remote's for.
fn press(button: Button) {
    if let Some(source) = source() {
        crate::screen::press(&source, button);
    }
    soon();
}

/// A second tap quick on the first is a double tap to XAML, and no tap at all: on key `k`, where every press counts, it's
/// `tapped` again.
fn every_tap_counts(k: &Border, tapped: impl Fn() + Send + 'static) -> R<()> {
    ui(k)?
        .DoubleTapped(&DoubleTappedEventHandler::new(move |_, _| crate::guard(|| {
            tapped();
            Ok(())
        })))
        .map_err(w)?;
    Ok(())
}

fn source() -> Option<String> {
    REMOTE.with_borrow(|r| r.as_ref().map(|r| r.source.clone()))
}

/// A flat key showing `glyph`.
fn key(glyph: &TextBlock, press: Button) -> R<Border> {
    colored_key(glyph, press, [KEY, KEY_OVER, KEY_DOWN])
}

/// A key showing `glyph`, in `colors`: as it is, pointed at, and pressed.
fn colored_key(glyph: &TextBlock, button: Button, colors: [Color; 3]) -> R<Border> {
    tap_key(glyph, colors, move || press(button))
}

/// The same, doing `tapped` when it's tapped.
fn tap_key(glyph: &TextBlock, colors: [Color; 3], tapped: impl Fn() + Send + 'static) -> R<Border> {
    let [rest, over, down] = colors;
    glyph.SetHorizontalAlignment(HorizontalAlignment::Center).map_err(w)?;
    glyph.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let b = Border::new().map_err(w)?;
    b.SetBackground(&brush(rest)?).map_err(w)?;
    b.SetCornerRadius(corners(6.0)).map_err(w)?;
    b.SetWidth(40.0).map_err(w)?;
    b.SetHeight(32.0).map_err(w)?;
    b.SetMargin(thickness(0.0, 0.0, 5.0, 0.0)).map_err(w)?;
    b.SetChild(&ui(glyph)?).map_err(w)?;
    let el = ui(&b)?;
    for (color, event) in [(over, 0), (rest, 1), (down, 2), (over, 3)] {
        let face = b.clone();
        let handler = PointerEventHandler::new(move |_, _| crate::guard(|| {
            if let Ok(fill) = brush(color) {
                let _ = face.SetBackground(&fill);
            }
            Ok(())
        }));
        match event {
            0 => el.PointerEntered(&handler),
            1 => el.PointerExited(&handler),
            2 => el.PointerPressed(&handler),
            _ => el.PointerReleased(&handler),
        }
        .map_err(w)?;
    }
    el.Tapped(&TappedEventHandler::new(move |_, _| crate::guard(|| {
        tapped();
        Ok(())
    })))
    .map_err(w)?;
    Ok(b)
}

/// While the pointer's on way `i`, `hint` says what it does; after, what the chosen one does.
fn hints(k: &Border, i: usize, hint: &TextBlock) -> R<()> {
    let el = ui(k)?;
    for entering in [true, false] {
        let hint = hint.clone();
        let handler = PointerEventHandler::new(move |_, _| crate::guard(|| {
            POINTED.set(entering.then_some(i));
            let _ = hint.SetText(&HSTRING::from(WAYS[if entering { i } else { CHOSEN.get() }].t()));
            Ok(())
        }));
        if entering { el.PointerEntered(&handler) } else { el.PointerExited(&handler) }.map_err(w)?;
    }
    Ok(())
}

/// While the pointer's on key `k`, `hint` says what it does (`says`); after, what the chosen way does.
fn explains(k: &Border, says: t::Id, hint: &TextBlock) -> R<()> {
    let el = ui(k)?;
    for entering in [true, false] {
        let hint = hint.clone();
        let handler = PointerEventHandler::new(move |_, _| crate::guard(|| {
            EXPLAINING.set(entering.then_some(says));
            let _ = hint.SetText(&HSTRING::from(hint_text()));
            Ok(())
        }));
        if entering { el.PointerEntered(&handler) } else { el.PointerExited(&handler) }.map_err(w)?;
    }
    Ok(())
}

/// What the hint under the ways says: what the key the pointer's on does, or the way it's on, or the way that's chosen.
fn hint_text() -> &'static str {
    EXPLAINING.get().unwrap_or(WAYS[POINTED.get().unwrap_or(CHOSEN.get())]).t()
}

/// The remove key: the first press only asks, and stops asking after a few seconds.
fn nuke_pressed() {
    if !ARMED.replace(true) {
        let mine = ASKING.fetch_add(1, Ordering::Relaxed) + 1;
        REMOTE.with_borrow(|r| {
            if let Some(r) = r {
                let _ = r.nuke.SetText(&HSTRING::from(t::SURE.t()));
            }
        });
        std::thread::spawn(move || {
            std::thread::sleep(STAYS_SURE);
            crate::meters::on_ui(move || {
                if ASKING.load(Ordering::Relaxed) == mine {
                    calm_down();
                }
            });
        });
        return;
    }
    calm_down();
    if let Some(source) = source() {
        crate::screen::press(&source, Button::NukeTv);
    }
    close();
}

fn calm_down() {
    ARMED.set(false);
    ASKING.fetch_add(1, Ordering::Relaxed);
    REMOTE.with_borrow(|r| {
        if let Some(r) = r {
            let _ = r.nuke.SetText(&HSTRING::from(t::NUKE.t()));
        }
    });
}

/// One of the three ways a set can go.
fn way(said: &mut Said, name: t::Id, press: Button, width: f64, last: bool) -> R<Border> {
    let words = words("", 11.0, TEXT)?;
    said.keep(&words, name)?;
    let k = key(&words, press)?;
    k.SetWidth(width).map_err(w)?;
    k.SetHeight(28.0).map_err(w)?;
    if last {
        k.SetMargin(thickness(0.0, 0.0, 0.0, 0.0)).map_err(w)?;
    }
    Ok(k)
}

/// A small key beside the highest quality.
fn stepper(glyph: &str, press: Button) -> R<Border> {
    let k = key(&icon(glyph)?, press)?;
    k.SetWidth(28.0).map_err(w)?;
    k.SetHeight(24.0).map_err(w)?;
    Ok(k)
}

fn logo() -> R<UIElement> {
    const SIDE: usize = 4;
    let (width, height) = (LOGO_PIXELS.0 as usize, LOGO_PIXELS.1 as usize);
    let mut bgra = vec![255u8; width * height * SIDE];
    pms_screen::logo(width, height, 1.0, [BODY.R, BODY.G, BODY.B], |x, y, [r, g, b]| bgra[(y * width + x) * SIDE..][..3].copy_from_slice(&[b, g, r]));
    let image = crate::screen::still_image(&bgra, LOGO_PIXELS, LOGO_SIZE)?;
    let el = ui(&image)?;
    image.SetHorizontalAlignment(HorizontalAlignment::Center).map_err(w)?;
    image.SetMargin(thickness(0.0, 0.0, 0.0, 6.0)).map_err(w)?;
    Ok(el)
}

fn panel(parts: &[UIElement]) -> R<StackPanel> {
    let p = crate::stage::stack_panel()?;
    for part in parts {
        p.Children().map_err(w)?.Append(part).map_err(w)?;
    }
    Ok(p)
}

fn icon(glyph: &str) -> R<TextBlock> {
    label(glyph, 14.0, TEXT, "Segoe Fluent Icons, Segoe MDL2 Assets")
}

/// The A's switch, as Windows' own are: on, filled in the accent with the knob to the right; off, an outline with the knob to the
/// left. Lighter while the pointer's on it.
fn paint_switch(track: &Border, knob: &Border) -> R<()> {
    let (on, over) = (SETS_ON.get(), ON_SWITCH.get());
    track.SetBackground(&brush(match (on, over) {
        (true, false) => ACCENT,
        (true, true) => ACCENT_OVER,
        (false, false) => CLEAR,
        (false, true) => KEY_OVER,
    })?)
    .map_err(w)?;
    track.SetBorderBrush(&brush(if on { ACCENT } else if over { TEXT } else { QUIET })?).map_err(w)?;
    knob.SetBackground(&brush(if on { BODY } else if over { TEXT } else { QUIET })?).map_err(w)?;
    knob.SetHorizontalAlignment(if on { HorizontalAlignment::Right } else { HorizontalAlignment::Left }).map_err(w)
}

/// The A's own: how many TwaTVs may come by themselves, with keys for fewer and more, and under it the switch for whether they do,
/// which is the same as selecting the A. Their words are the A's to say, in `show_status`.
fn sets_part() -> R<(StackPanel, [TextBlock; 4], [Border; 4])> {
    let name = words("", 11.0, QUIET)?;
    name.SetWidth(120.0).map_err(w)?;
    name.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let value = words("", 12.0, TEXT)?;
    value.SetWidth(50.0).map_err(w)?;
    value.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let (fewer, more) = (stepper(MINUS, Button::FewerSets)?, stepper(PLUS, Button::MoreSets)?);
    more.SetMargin(thickness(0.0, 0.0, 0.0, 0.0)).map_err(w)?;
    every_tap_counts(&fewer, || press(Button::FewerSets))?;
    every_tap_counts(&more, || press(Button::MoreSets))?;
    let how_many = across(&[ui(&name)?, ui(&value)?, ui(&fewer)?, ui(&more)?])?;

    let switch_name = words("", 11.0, QUIET)?;
    switch_name.SetWidth(120.0).map_err(w)?;
    switch_name.SetTextWrapping(TextWrapping::Wrap).map_err(w)?;
    switch_name.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let knob = Border::new().map_err(w)?;
    knob.SetWidth(12.0).map_err(w)?;
    knob.SetHeight(12.0).map_err(w)?;
    knob.SetCornerRadius(corners(6.0)).map_err(w)?;
    knob.SetMargin(thickness(3.0, 0.0, 3.0, 0.0)).map_err(w)?;
    knob.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let track = Border::new().map_err(w)?;
    track.SetWidth(40.0).map_err(w)?;
    track.SetHeight(20.0).map_err(w)?;
    track.SetCornerRadius(corners(10.0)).map_err(w)?;
    track.SetBorderThickness(thickness(1.0, 1.0, 1.0, 1.0)).map_err(w)?;
    track.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    track.SetChild(&ui(&knob)?).map_err(w)?;
    let state = words("", 12.0, TEXT)?;
    state.SetMargin(thickness(8.0, 0.0, 0.0, 0.0)).map_err(w)?;
    state.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    // All of the line is the switch, as a checkbox's words are; a clear background, so that it takes the pointer between them too.
    let switch = Border::new().map_err(w)?;
    switch.SetBackground(&brush(CLEAR)?).map_err(w)?;
    switch.SetMinHeight(28.0).map_err(w)?;
    switch.SetMargin(thickness(0.0, 6.0, 0.0, 0.0)).map_err(w)?;
    switch.SetChild(&ui(&across(&[ui(&switch_name)?, ui(&track)?, ui(&state)?])?)?).map_err(w)?;
    paint_switch(&track, &knob)?;
    let el = ui(&switch)?;
    for entering in [true, false] {
        let (track, knob) = (track.clone(), knob.clone());
        let handler = PointerEventHandler::new(move |_, _| crate::guard(|| {
            ON_SWITCH.set(entering);
            let _ = paint_switch(&track, &knob);
            Ok(())
        }));
        if entering { el.PointerEntered(&handler) } else { el.PointerExited(&handler) }.map_err(w)?;
    }
    // A click on the A itself, which switches them.
    let switched = || {
        if let Some(source) = source() {
            crate::screen::click(&source);
        }
        soon();
    };
    el.Tapped(&TappedEventHandler::new(move |_, _| crate::guard(|| {
        switched();
        Ok(())
    })))
    .map_err(w)?;
    every_tap_counts(&switch, switched)?;

    let part = panel(&[ui(&how_many)?, ui(&switch)?])?;
    part.SetMargin(thickness(0.0, 10.0, 0.0, 0.0)).map_err(w)?;
    part.SetVisibility(Visibility::Collapsed).map_err(w)?;
    Ok((part, [name, value, switch_name, state], [fewer, more, track, knob]))
}

/// Words that say `text`, kept in `said`.
fn kept(said: &mut Said, text: t::Id, size: f64, color: Color) -> R<TextBlock> {
    let block = words("", size, color)?;
    said.keep(&block, text)?;
    Ok(block)
}

fn build(root: &XamlRoot) -> R<Remote> {
    let mut said = Said::default();
    let state = words("", 11.0, QUIET)?;
    let dot = light(8.0)?;
    let top = across(&[ui(&dot)?, ui(&state)?])?;
    let handle = words("", 15.0, TEXT)?;
    handle.SetFontWeight(windows::UI::Text::FontWeight { Weight: 600 }).map_err(w)?;
    handle.SetMargin(thickness(0.0, 3.0, 0.0, 0.0)).map_err(w)?;
    let title = words("", 12.0, TEXT)?;
    title.SetTextWrapping(TextWrapping::Wrap).map_err(w)?;
    title.SetMaxLines(2).map_err(w)?;
    title.SetMargin(thickness(0.0, 2.0, 0.0, 0.0)).map_err(w)?;
    let about = words("", 11.0, QUIET)?;
    about.SetTextWrapping(TextWrapping::Wrap).map_err(w)?;
    about.SetMargin(thickness(0.0, 3.0, 0.0, 0.0)).map_err(w)?;
    let lowered = words("", 11.0, LOWERED)?;
    lowered.SetTextWrapping(TextWrapping::Wrap).map_err(w)?;
    lowered.SetMargin(thickness(0.0, 2.0, 0.0, 0.0)).map_err(w)?;
    let via = words("", 11.0, QUIET)?;
    via.SetTextWrapping(TextWrapping::Wrap).map_err(w)?;
    via.SetMargin(thickness(0.0, 2.0, 0.0, 0.0)).map_err(w)?;
    // Live video wanted and no Streamlink: what's wrong, and the way to the settings that fix it.
    let warn_words = words("", 12.0, WARN_TEXT)?;
    warn_words.SetTextWrapping(TextWrapping::Wrap).map_err(w)?;
    let fix_words = kept(&mut said, t::GET_STREAMLINK, 12.0, BODY)?;
    fix_words.SetFontWeight(windows::UI::Text::FontWeight { Weight: 600 }).map_err(w)?;
    let fix = colored_key(&fix_words, Button::AddFavorite, [ACCENT, ACCENT_OVER, ACCENT_DOWN])?;
    fix.SetWidth(f64::NAN).map_err(w)?;
    fix.SetHorizontalAlignment(HorizontalAlignment::Stretch).map_err(w)?;
    fix.SetMargin(thickness(0.0, 8.0, 0.0, 0.0)).map_err(w)?;
    let warn = Border::new().map_err(w)?;
    warn.SetBackground(&brush(WARN)?).map_err(w)?;
    warn.SetBorderBrush(&brush(WARN_EDGE)?).map_err(w)?;
    warn.SetBorderThickness(thickness(1.0, 1.0, 1.0, 1.0)).map_err(w)?;
    warn.SetCornerRadius(corners(6.0)).map_err(w)?;
    warn.SetPadding(thickness(8.0, 6.0, 8.0, 8.0)).map_err(w)?;
    warn.SetMargin(thickness(0.0, 8.0, 0.0, 0.0)).map_err(w)?;
    warn.SetChild(&ui(&panel(&[ui(&warn_words)?, ui(&fix)?])?)?).map_err(w)?;
    warn.SetVisibility(Visibility::Collapsed).map_err(w)?;

    let plan = words("", 11.0, TEXT)?;
    plan.SetTextWrapping(TextWrapping::Wrap).map_err(w)?;
    plan.SetMargin(thickness(0.0, 6.0, 0.0, 0.0)).map_err(w)?;

    let power_glyph = icon(POWER)?;
    let sound = icon(SOUND_OFF)?;
    let power = key(&power_glyph, Button::TvPower)?;
    let mute = key(&sound, Button::Mute)?;
    let up = key(&icon(UP)?, Button::ChannelUp)?;
    let down = key(&icon(DOWN)?, Button::ChannelDown)?;
    for k in [&power, &mute, &up, &down] {
        k.SetWidth(54.0).map_err(w)?;
    }
    down.SetMargin(thickness(0.0, 0.0, 0.0, 0.0)).map_err(w)?;
    let keys = across(&[ui(&power)?, ui(&mute)?, ui(&up)?, ui(&down)?])?;
    keys.SetMargin(thickness(0.0, 10.0, 0.0, 0.0)).map_err(w)?;

    let (follow, stay, only) = (way(&mut said, t::FOLLOW, Button::Follow, 66.0, false)?, way(&mut said, t::STAY, Button::Stay, 74.0, false)?, way(&mut said, t::ONLY, Button::Only, INSIDE - 66.0 - 74.0 - 2.0 * 5.0, true)?);
    let ways = across(&[ui(&follow)?, ui(&stay)?, ui(&only)?])?;
    ways.SetMargin(thickness(0.0, 2.0, 0.0, 0.0)).map_err(w)?;
    // Read across, it's a sentence: this TwaTV plays favorites, this stream, or this channel. Under it, what that means.
    let plays = kept(&mut said, t::PLAYS, 11.0, QUIET)?;
    plays.SetMargin(thickness(0.0, 10.0, 0.0, 0.0)).map_err(w)?;
    let way_hint = words(WAYS[0].t(), 11.0, QUIET)?;
    way_hint.SetTextWrapping(TextWrapping::Wrap).map_err(w)?;
    way_hint.SetMargin(thickness(0.0, 4.0, 0.0, 0.0)).map_err(w)?;
    for (i, k) in [&follow, &stay, &only].into_iter().enumerate() {
        hints(k, i, &way_hint)?;
    }
    for (k, says) in [(&power, t::POWER_SAYS), (&mute, t::SOUND_SAYS), (&up, t::UP_SAYS), (&down, t::DOWN_SAYS)] {
        explains(k, says, &way_hint)?;
    }

    let every = kept(&mut said, t::ALL_TWATVS, 11.0, QUIET)?;
    every.SetMargin(thickness(0.0, 10.0, 0.0, 2.0)).map_err(w)?;
    let name = kept(&mut said, t::TOP_QUALITY, 11.0, QUIET)?;
    name.SetWidth(120.0).map_err(w)?;
    name.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let top_value = words("", 12.0, TEXT)?;
    top_value.SetWidth(50.0).map_err(w)?;
    top_value.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let (lower, higher) = (stepper(MINUS, Button::QualityDown)?, stepper(PLUS, Button::QualityUp)?);
    higher.SetMargin(thickness(0.0, 0.0, 0.0, 0.0)).map_err(w)?;
    let quality = across(&[ui(&name)?, ui(&top_value)?, ui(&lower)?, ui(&higher)?])?;
    quality.SetMargin(thickness(0.0, 2.0, 0.0, 0.0)).map_err(w)?;

    let loud_name = kept(&mut said, t::VOLUME, 11.0, QUIET)?;
    loud_name.SetWidth(120.0).map_err(w)?;
    loud_name.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let loud_value = words("", 12.0, TEXT)?;
    loud_value.SetWidth(50.0).map_err(w)?;
    loud_value.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let (quieter, louder) = (stepper(MINUS, Button::VolumeDown)?, stepper(PLUS, Button::VolumeUp)?);
    louder.SetMargin(thickness(0.0, 0.0, 0.0, 0.0)).map_err(w)?;
    let loudness = across(&[ui(&loud_name)?, ui(&loud_value)?, ui(&quieter)?, ui(&louder)?])?;
    loudness.SetMargin(thickness(0.0, 4.0, 0.0, 0.0)).map_err(w)?;

    let heading = words(t::FAVORITES.t(), 11.0, QUIET)?;
    heading.SetWidth(INSIDE - 28.0).map_err(w)?;
    heading.SetTextWrapping(TextWrapping::Wrap).map_err(w)?;
    heading.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let another = stepper(PLUS, Button::AddFavorite)?;
    another.SetMargin(thickness(0.0, 0.0, 0.0, 0.0)).map_err(w)?;
    let heading_row = across(&[ui(&heading)?, ui(&another)?])?;
    heading_row.SetMargin(thickness(0.0, 10.0, 0.0, 3.0)).map_err(w)?;
    // With no favorites, there's nothing for it to show: this is what it needs first, so it's the biggest thing there.
    let first_words = kept(&mut said, t::FIRST_FAVORITE, 12.0, BODY)?;
    first_words.SetFontWeight(windows::UI::Text::FontWeight { Weight: 600 }).map_err(w)?;
    let first = colored_key(&first_words, Button::AddFavorite, [ACCENT, ACCENT_OVER, ACCENT_DOWN])?;
    first.SetWidth(f64::NAN).map_err(w)?;
    first.SetHorizontalAlignment(HorizontalAlignment::Stretch).map_err(w)?;
    first.SetHeight(36.0).map_err(w)?;
    first.SetMargin(thickness(0.0, 12.0, 0.0, 0.0)).map_err(w)?;
    let nuke = kept(&mut said, t::NUKE, 11.0, QUIET)?;
    let nuke_key = tap_key(&nuke, [KEY, KEY_OVER, KEY_DOWN], nuke_pressed)?;
    nuke_key.SetWidth(f64::NAN).map_err(w)?;
    nuke_key.SetHorizontalAlignment(HorizontalAlignment::Stretch).map_err(w)?;
    nuke_key.SetHeight(28.0).map_err(w)?;
    nuke_key.SetMargin(thickness(0.0, 10.0, 0.0, 0.0)).map_err(w)?;
    let list = crate::stage::stack_panel()?;
    let mut rows = Vec::new();
    // One more row than there are favorites shown: the one that folds the offline away.
    for i in 0..=ROWS {
        let (dot, name) = (light(6.0)?, words("", 12.0, TEXT)?);
        let line = Border::new().map_err(w)?;
        line.SetCornerRadius(corners(4.0)).map_err(w)?;
        line.SetPadding(thickness(6.0, 3.0, 6.0, 3.0)).map_err(w)?;
        line.SetBackground(&brush(CLEAR)?).map_err(w)?;
        line.SetChild(&ui(&across(&[ui(&dot)?, ui(&name)?])?)?).map_err(w)?;
        ui(&line)?
            .Tapped(&TappedEventHandler::new(move |_, _| crate::guard(|| {
                match ROWS_DO.with_borrow(|rows| rows.get(i).copied()) {
                    Some(Some(pick)) => {
                        if let Some(source) = source() {
                            crate::screen::tune(&source, pick);
                        }
                        soon();
                    }
                    Some(None) => {
                        UNFOLDED.set(!UNFOLDED.get());
                        refresh();
                    }
                    None => {}
                }
                Ok(())
            })))
            .map_err(w)?;
        list.Children().map_err(w)?.Append(&ui(&line)?).map_err(w)?;
        rows.push((line, dot, name));
    }

    let head = panel(&[ui(&top)?, ui(&handle)?, ui(&title)?, ui(&about)?, ui(&lowered)?, ui(&via)?, ui(&warn)?])?;
    let controls = panel(&[ui(&keys)?, ui(&plays)?, ui(&ways)?, ui(&way_hint)?, ui(&every)?, ui(&quality)?, ui(&loudness)?, ui(&first)?, ui(&heading_row)?, ui(&list)?, ui(&nuke_key)?])?;
    let (sets, [sets_name, sets_value, switch_name, switch_state], [fewer, more, track, knob]) = sets_part()?;
    let body = panel(&[logo()?, ui(&head)?, ui(&plan)?, ui(&sets)?, ui(&controls)?])?;
    let whole = Border::new().map_err(w)?;
    whole.SetBackground(&brush(BODY)?).map_err(w)?;
    whole.SetBorderBrush(&brush(EDGE)?).map_err(w)?;
    whole.SetBorderThickness(thickness(1.0, 1.0, 1.0, 1.0)).map_err(w)?;
    whole.SetCornerRadius(corners(10.0)).map_err(w)?;
    whole.SetPadding(thickness(12.0, 10.0, 12.0, 12.0)).map_err(w)?;
    whole.SetWidth(WIDTH).map_err(w)?;
    whole.SetChild(&ui(&body)?).map_err(w)?;
    let el = ui(&whole)?;
    crate::remote::follow_pointer(&el)?;

    let popup = Popup::new().map_err(w)?;
    popup.SetChild(&el).map_err(w)?;
    popup.SetShouldConstrainToRootBounds(false).map_err(w)?;
    popup.SetXamlRoot(root).map_err(w)?;
    Ok(Remote { popup, head, controls, handle, state, light: dot, title, about, lowered, via, warn, warn_words, plan, power, follow, stay, only, way_hint, nuke, top: top_value, volume: loud_value, sound, heading, heading_row, first, rows, sets, sets_name, said, sets_value, fewer, more, switch_name, track, knob, switch_state, source: String::new(), placed: None })
}

/// What a set says about itself.
#[derive(Default, PartialEq, Debug)]
struct Info {
    handle: String,
    title: String,
    category: String,
    quality: String,
    /// Why the quality's lowered, if it is.
    low: String,
    state: String,
    plan: String,
    /// Where the picture comes from, and what's missing.
    via: String,
    warn: String,
    /// The highest quality, for every set, and how loud they are.
    top: String,
    volume: String,
    /// How many seconds ago it last looked at who's live.
    seen: Option<u64>,
    favorites: Vec<(String, bool)>,
    /// The A's: how many TwaTVs may come by themselves, whether they do, and the fewest and most there can be; and what it calls
    /// the number and the switch, and on and off.
    sets: Option<(u32, bool, u32, u32)>,
    sets_words: [String; 4],
    /// It's showing a channel live, as its light says.
    live: bool,
}

fn read(info: &str) -> Info {
    let mut out = Info::default();
    for line in info.lines() {
        match line.split('\t').collect::<Vec<_>>().as_slice() {
            ["now", handle, title, category, quality, state] => {
                (out.handle, out.title, out.category, out.quality, out.state) = (handle.to_string(), title.to_string(), category.to_string(), quality.to_string(), state.to_string());
            }
            // A line separator is where a new line starts: the info's lines can't hold one.
            ["how", plan] => out.plan = plan.replace('\u{2028}', "\n"),
            ["low", why] => out.low = why.to_string(),
            ["via", via] => out.via = via.to_string(),
            ["warn", warn] => out.warn = warn.to_string(),
            ["top", top] => out.top = top.to_string(),
            ["vol", volume] => out.volume = volume.to_string(),
            ["seen", ago] => out.seen = ago.parse().ok(),
            ["live", live] => out.live = *live == "1",
            ["fav", handle, live] => out.favorites.push((handle.to_string(), *live != "0")),
            ["sets", more, on, fewest, most] => {
                out.sets = more.parse().ok().zip(fewest.parse().ok()).zip(most.parse().ok()).map(|((more, fewest), most)| (more, *on != "0", fewest, most));
            }
            ["sets_words", how_many, switch, yes, no] => out.sets_words = [how_many, switch, yes, no].map(|t| t.to_string()),
            _ => {}
        }
    }
    out
}

/// A line that isn't there takes no room.
fn say(t: &TextBlock, text: &str) -> R<()> {
    t.SetText(&HSTRING::from(text)).map_err(w)?;
    t.SetVisibility(if text.is_empty() { Visibility::Collapsed } else { Visibility::Visible }).map_err(w)
}

/// The heading over the favorites, with how fresh they are.
fn heading_text(seen: Option<u64>) -> String {
    match seen {
        Some(ago) if ago < 5 => t::CHECKED_NOW.t().into(),
        Some(ago) if ago < 90 => t::CHECKED_SECONDS.with(&[("n", &ago)]),
        Some(ago) => t::CHECKED_MINUTES.with(&[("n", &(ago / 60))]),
        None => t::FAVORITES.t().into(),
    }
}

fn show_status(r: &Remote) -> R<()> {
    let Some(screen) = crate::screen::status(&r.source) else { return Ok(()) };
    // A key asking whether it's sure goes on asking, in the new words: what it asks doesn't go back to what it was.
    if r.said.follow()? && ARMED.get() {
        r.nuke.SetText(&HSTRING::from(t::SURE.t())).map_err(w)?;
    }
    let info = read(&crate::screen::info(&r.source).unwrap_or_default());
    let shown = |on: bool| if on { Visibility::Visible } else { Visibility::Collapsed };
    r.head.SetVisibility(shown(!screen.note)).map_err(w)?;
    r.controls.SetVisibility(shown(!screen.note)).map_err(w)?;
    let sets = info.sets.filter(|_| screen.note);
    r.sets.SetVisibility(shown(sets.is_some())).map_err(w)?;
    if let Some((more, on, fewest, most)) = sets {
        let [how_many, switch, yes, no] = &info.sets_words;
        r.sets_name.SetText(&HSTRING::from(how_many.as_str())).map_err(w)?;
        r.sets_value.SetText(&HSTRING::from(more.to_string())).map_err(w)?;
        // A key that can go no further is dimmed; pressing it does nothing.
        ui(&r.fewer)?.SetOpacity(if more > fewest { 1.0 } else { 0.4 }).map_err(w)?;
        ui(&r.more)?.SetOpacity(if more < most { 1.0 } else { 0.4 }).map_err(w)?;
        r.switch_name.SetText(&HSTRING::from(switch.as_str())).map_err(w)?;
        r.switch_state.SetText(&HSTRING::from(if on { yes.as_str() } else { no.as_str() })).map_err(w)?;
        SETS_ON.set(on);
        paint_switch(&r.track, &r.knob)?;
    }
    if screen.note {
        return say(&r.plan, &info.plan);
    }
    let live = info.live;
    r.light.SetBackground(&brush(if live { LIVE } else { NOT_LIVE })?).map_err(w)?;
    say(&r.state, &info.state)?;
    say(&r.handle, &info.handle)?;
    say(&r.title, &info.title)?;
    let about: Vec<&str> = [info.category.as_str(), info.quality.as_str()].into_iter().filter(|t| !t.is_empty()).collect();
    say(&r.about, &about.join("  ·  "))?;
    say(&r.lowered, &info.low)?;
    say(&r.via, &info.via)?;
    say(&r.warn_words, &info.warn)?;
    r.warn.SetVisibility(if info.warn.is_empty() { Visibility::Collapsed } else { Visibility::Visible }).map_err(w)?;
    say(&r.plan, if screen.tv_on { &info.plan } else { "" })?;
    r.follow.SetBackground(&brush(if screen.shuffle { KEY } else { ACCENT })?).map_err(w)?;
    r.stay.SetBackground(&brush(if screen.shuffle && !screen.only { ACCENT } else { KEY })?).map_err(w)?;
    r.only.SetBackground(&brush(if screen.only { ACCENT } else { KEY })?).map_err(w)?;
    CHOSEN.set(if screen.only { 2 } else if screen.shuffle { 1 } else { 0 });
    r.way_hint.SetText(&HSTRING::from(hint_text())).map_err(w)?;
    r.top.SetText(&HSTRING::from(info.top.as_str())).map_err(w)?;
    r.volume.SetText(&HSTRING::from(info.volume.as_str())).map_err(w)?;
    r.heading.SetText(&HSTRING::from(heading_text(info.seen))).map_err(w)?;
    r.sound.SetText(&HSTRING::from(if screen.muted { SOUND_OFF } else { SOUND_ON })).map_err(w)?;
    r.power.SetBackground(&brush(if screen.tv_on { KEY } else { ACCENT })?).map_err(w)?;
    let none = info.favorites.is_empty();
    r.heading_row.SetVisibility(if none { Visibility::Collapsed } else { Visibility::Visible }).map_err(w)?;
    r.first.SetVisibility(if none { Visibility::Visible } else { Visibility::Collapsed }).map_err(w)?;
    let shown = listed(&info.favorites, UNFOLDED.get(), ROWS);
    ROWS_DO.with_borrow_mut(|rows| *rows = shown.iter().map(|row| row.pick).collect());
    for (i, (line, dot, name)) in r.rows.iter().enumerate() {
        match shown.get(i) {
            Some(row) => {
                let on = row.pick.is_some() && row.words == info.handle;
                line.SetVisibility(Visibility::Visible).map_err(w)?;
                line.SetBackground(&brush(if on { KEY } else { CLEAR })?).map_err(w)?;
                dot.SetVisibility(if row.pick.is_some() { Visibility::Visible } else { Visibility::Collapsed }).map_err(w)?;
                dot.SetBackground(&brush(if row.live { LIVE } else { NOT_LIVE })?).map_err(w)?;
                name.SetText(&HSTRING::from(row.words.as_str())).map_err(w)?;
                name.SetForeground(&brush(if on { ACCENT } else if row.live { TEXT } else { QUIET })?).map_err(w)?;
            }
            None => line.SetVisibility(Visibility::Collapsed).map_err(w)?,
        }
    }
    Ok(())
}

/// A row of the favorites as the remote lists them.
#[derive(PartialEq, Debug)]
struct Row {
    words: String,
    live: bool,
    /// Its place among your favorites, from 1, to tune to; None: the row that folds the offline ones away, or shows them.
    pick: Option<u32>,
}

/// Your favorites as the remote lists them: the live ones in your order, then the offline ones folded into one row, or, `unfolded`,
/// listed after it. At most `room` favorites.
fn listed(favorites: &[(String, bool)], unfolded: bool, room: usize) -> Vec<Row> {
    let fav = |(i, (handle, live)): (usize, &(String, bool))| Row { words: handle.clone(), live: *live, pick: Some(i as u32 + 1) };
    let mut rows: Vec<Row> = favorites.iter().enumerate().filter(|(_, f)| f.1).map(fav).take(room).collect();
    let offline: Vec<Row> = favorites.iter().enumerate().filter(|(_, f)| !f.1).map(fav).collect();
    if !offline.is_empty() && rows.len() < room {
        let words = if unfolded { t::HIDE_OFFLINE.t().to_string() } else { t::OFFLINE.with(&[("n", &offline.len())]) };
        rows.push(Row { words, live: false, pick: None });
        if unfolded {
            let left = room - (rows.len() - 1);
            rows.extend(offline.into_iter().take(left));
        }
    }
    rows
}

fn refresh() {
    REMOTE.with_borrow(|r| {
        let Some(r) = r else { return };
        // What's in it can make it taller or shorter: it stays above the set either way.
        let done = show_status(r).and_then(|()| match &r.placed {
            Some((root, left, top, width)) => crate::remote::place(&r.popup, root, *left, *top, *width),
            None => Ok(()),
        });
        if let Err(e) = done {
            crate::log(format!("remote: {e}"));
        }
    });
}

/// Whether it's open, for any screen.
pub(crate) fn any_up() -> bool {
    REMOTE.with_borrow(|r| r.as_ref().is_some_and(|r| r.popup.IsOpen().unwrap_or(false)))
}

/// Whether it's open, for the screen called `source`.
pub(crate) fn is_up(source: &str) -> bool {
    REMOTE.with_borrow(|r| r.as_ref().is_some_and(|r| r.source == source && r.popup.IsOpen().unwrap_or(false)))
}

/// Opens it for the screen called `source`, whose cell starts at `left` and `top` and is `width` wide.
pub(crate) fn show(root: &XamlRoot, source: &str, left: f64, top: f64, width: f64) -> R<()> {
    if REMOTE.with_borrow(Option::is_none) {
        let built = build(root)?;
        REMOTE.with_borrow_mut(|r| *r = Some(built));
    }
    let was_open = REMOTE.with_borrow_mut(|r| -> R<bool> {
        let r = r.as_mut().ok_or("no remote")?;
        r.source = source.to_string();
        r.placed = Some((root.clone(), left, top, width));
        show_status(r)?;
        crate::remote::place(&r.popup, root, left, top, width)?;
        let open = r.popup.IsOpen().map_err(w)?;
        r.popup.SetIsOpen(true).map_err(w)?;
        Ok(open)
    })?;
    if !was_open {
        crate::remote::float_popups();
        crate::remote::watch_pointer();
        // What's on and who's live change while it's up.
        let mine = OPENED.fetch_add(1, Ordering::Relaxed) + 1;
        std::thread::spawn(move || {
            while OPENED.load(Ordering::Relaxed) == mine {
                std::thread::sleep(REFRESH);
                crate::meters::on_ui(refresh);
            }
        });
    }
    Ok(())
}

pub(crate) fn close() {
    OPENED.fetch_add(1, Ordering::Relaxed);
    UNFOLDED.set(false);
    POINTED.set(None);
    EXPLAINING.set(None);
    ON_SWITCH.set(false);
    if ARMED.get() {
        calm_down();
    }
    REMOTE.with_borrow(|r| {
        if let Some(r) = r {
            let _ = r.popup.SetIsOpen(false);
        }
    });
}

/// For reset/detach.
pub(crate) fn off() {
    close();
    if let Some(r) = REMOTE.with_borrow_mut(Option::take) {
        crate::stage::unbuild_popup(&r.popup);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_set_says_is_read() {
        let info = read("now\tcrab\tpinching things\tJust Chatting\t720p60\tLive\nlive\t1\nhow\tSwitches if lobster goes live\ntop\t480p\nseen\t40\nfav\tcrab\t1\nfav\tlobster\t0\n");
        assert!(info.live, "the light's own line, whatever the words say");
        assert_eq!(info.top, "480p");
        assert_eq!(info.plan, "Switches if lobster goes live");
        assert_eq!(info.seen, Some(40));
        assert_eq!(info.handle, "crab");
        assert_eq!(info.state, "Live");
        assert_eq!(info.favorites, [("crab".to_string(), true), ("lobster".to_string(), false)]);
        assert_eq!(read(""), Info::default());
        assert_eq!(read("now\tcut sho").handle, "", "half a line is no line");
        let missing = read("via\t\nwarn\tLive video needs Streamlink, which isn't installed\n");
        assert_eq!((missing.via.as_str(), missing.warn.as_str()), ("", "Live video needs Streamlink, which isn't installed"));
        assert_eq!(read("via\tLive video through Streamlink 8.6.3-1\nwarn\t\n").via, "Live video through Streamlink 8.6.3-1");
    }

    #[test]
    fn what_the_a_says_is_read() {
        let info = read("how\tUp to 3 more come\nsets\t3\t1\t1\t9\nsets_words\tHow many\tCome by themselves\tOn\tOff\n");
        assert_eq!((info.plan.as_str(), info.sets), ("Up to 3 more come", Some((3, true, 1, 9))));
        assert_eq!(info.sets_words, ["How many", "Come by themselves", "On", "Off"].map(String::from));
        assert_eq!(read("sets\t3\t0\t1\t9\n").sets, Some((3, false, 1, 9)));
        assert_eq!(read("sets\tlots\t1\t1\t9\n").sets, None, "a number that isn't one is no number");
        assert_eq!(read("how\tSelect to add a TwaTV for good\n").sets, None, "the plus beside the sets has none of it");
    }

    #[test]
    fn live_favorites_come_first_and_the_offline_fold_away() {
        let favs: Vec<(String, bool)> = [("a", false), ("b", true), ("c", false), ("d", true)].iter().map(|(h, l)| (h.to_string(), *l)).collect();
        let words = |rows: &[Row]| rows.iter().map(|r| r.words.clone()).collect::<Vec<_>>();
        let folded = listed(&favs, false, 12);
        assert_eq!(words(&folded), ["b", "d", "2 offline"]);
        assert_eq!(folded.iter().map(|r| r.pick).collect::<Vec<_>>(), [Some(2), Some(4), None], "each tunes to its own place");
        assert_eq!(words(&listed(&favs, true, 12)), ["b", "d", "Hide offline", "a", "c"]);
        assert_eq!(words(&listed(&favs, true, 3)), ["b", "d", "Hide offline", "a"], "no more than there's room for");
        let all_live: Vec<(String, bool)> = favs.iter().map(|(h, _)| (h.clone(), true)).collect();
        assert_eq!(words(&listed(&all_live, false, 12)), ["a", "b", "c", "d"], "nothing to fold");
    }

    #[test]
    fn the_heading_says_how_fresh_it_is() {
        assert_eq!(heading_text(None), "Live Twatchlist channels");
        assert_eq!(heading_text(Some(2)), "Live Twatchlist channels, checked just now");
        assert_eq!(heading_text(Some(40)), "Live Twatchlist channels, checked 40 s ago");
        assert_eq!(heading_text(Some(200)), "Live Twatchlist channels, checked 3 min ago");
    }
}
