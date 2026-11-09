//! A Twatch TV's remote: the same idea as remote.rs, for a newer set. What's on and how well, what the set will do next and how it
//! decides that, power, sound, channel up and down, the highest quality, and the favorites to go straight to. All of what it shows
//! comes from the screen's `info`, which the set keeps up to date. For a button beside the sets, it's only the logo and what the
//! button's for.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use pms_screen::Button;
use pms_xaml::{Border, CornerRadius, FontFamily, HorizontalAlignment, Orientation, PointerEventHandler, Popup, SolidColorBrush, StackPanel, TappedEventHandler, TextBlock, TextTrimming, TextWrapping, Thickness, UIElement, VerticalAlignment, Visibility, XamlRoot};
use windows::UI::Color;
use windows_core::{HSTRING, Interface};

use crate::commands::R;

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
const CLEAR: Color = Color { A: 0, R: 0, G: 0, B: 0 };
const WIDTH: f64 = 256.0;
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
const WAYS: [&str; 3] = [
    "The best favorite that's live, switching when a better one goes live",
    "Stays on what's playing until the stream ends, whoever else goes live",
    "Only this channel. While it's offline, the set waits for it",
];
/// What the keys do, said where the ways are explained while the pointer's on one.
const POWER_SAYS: &str = "Turns this TwaTV off, or back on";
const SOUND_SAYS: &str = "This TwaTV's sound, on or off";
const UP_SAYS: &str = "The next favorite that's live. It stays on them until their stream ends";
const DOWN_SAYS: &str = "The live favorite before this one. It stays on them until their stream ends";
/// The key that takes this set off the taskbar: it asks first, for a few seconds.
const NUKE: &str = "Remove this TwaTV";
const SURE: &str = "Select again to remove it";
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
    static EXPLAINING: Cell<Option<&'static str>> = const { Cell::new(None) };
    /// The remove key has been pressed once, and is asking.
    static ARMED: Cell<bool> = const { Cell::new(false) };
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
    t.SetFontFamily(&FontFamily::CreateInstanceWithName(&HSTRING::from(font)).map_err(w)?).map_err(w)?;
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
    let row = StackPanel::new().map_err(w)?;
    row.SetOrientation(Orientation::Horizontal).map_err(w)?;
    for item in items {
        row.Children().map_err(w)?.Append(item).map_err(w)?;
    }
    Ok(row)
}

/// Runs `then` on the UI thread a moment after a press, when the set has acted on it.
fn soon() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(120));
        crate::meters::on_ui(refresh);
    });
}

fn source() -> Option<String> {
    REMOTE.with_borrow(|r| r.as_ref().map(|r| r.source.clone()))
}

/// A flat key showing `glyph`.
fn key(glyph: &TextBlock, press: Button) -> R<Border> {
    colored_key(glyph, press, [KEY, KEY_OVER, KEY_DOWN])
}

/// A key showing `glyph`, in `colors`: as it is, pointed at, and pressed.
fn colored_key(glyph: &TextBlock, press: Button, colors: [Color; 3]) -> R<Border> {
    tap_key(glyph, colors, move || {
        if let Some(source) = source() {
            crate::screen::press(&source, press);
        }
        soon();
    })
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
        let handler = PointerEventHandler::new(move |_, _| {
            if let Ok(fill) = brush(color) {
                let _ = face.SetBackground(&fill);
            }
            Ok(())
        });
        match event {
            0 => el.PointerEntered(&handler),
            1 => el.PointerExited(&handler),
            2 => el.PointerPressed(&handler),
            _ => el.PointerReleased(&handler),
        }
        .map_err(w)?;
    }
    el.Tapped(&TappedEventHandler::new(move |_, _| {
        tapped();
        Ok(())
    }))
    .map_err(w)?;
    Ok(b)
}

/// While the pointer's on way `i`, `hint` says what it does; after, what the chosen one does.
fn hints(k: &Border, i: usize, hint: &TextBlock) -> R<()> {
    let el = ui(k)?;
    for entering in [true, false] {
        let hint = hint.clone();
        let handler = PointerEventHandler::new(move |_, _| {
            POINTED.set(entering.then_some(i));
            let _ = hint.SetText(&HSTRING::from(WAYS[if entering { i } else { CHOSEN.get() }]));
            Ok(())
        });
        if entering { el.PointerEntered(&handler) } else { el.PointerExited(&handler) }.map_err(w)?;
    }
    Ok(())
}

/// While the pointer's on key `k`, `hint` says what it does (`says`); after, what the chosen way does.
fn explains(k: &Border, says: &'static str, hint: &TextBlock) -> R<()> {
    let el = ui(k)?;
    for entering in [true, false] {
        let hint = hint.clone();
        let handler = PointerEventHandler::new(move |_, _| {
            EXPLAINING.set(entering.then_some(says));
            let _ = hint.SetText(&HSTRING::from(hint_text()));
            Ok(())
        });
        if entering { el.PointerEntered(&handler) } else { el.PointerExited(&handler) }.map_err(w)?;
    }
    Ok(())
}

/// What the hint under the ways says: what the key the pointer's on does, or the way it's on, or the way that's chosen.
fn hint_text() -> &'static str {
    EXPLAINING.get().unwrap_or(WAYS[POINTED.get().unwrap_or(CHOSEN.get())])
}

/// The remove key: the first press only asks, and stops asking after a few seconds.
fn nuke_pressed() {
    if !ARMED.replace(true) {
        let mine = ASKING.fetch_add(1, Ordering::Relaxed) + 1;
        REMOTE.with_borrow(|r| {
            if let Some(r) = r {
                let _ = r.nuke.SetText(&HSTRING::from(SURE));
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
            let _ = r.nuke.SetText(&HSTRING::from(NUKE));
        }
    });
}

/// One of the three ways a set can go.
fn way(name: &str, press: Button, width: f64, last: bool) -> R<Border> {
    let k = key(&words(name, 11.0, TEXT)?, press)?;
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
    let p = StackPanel::new().map_err(w)?;
    for part in parts {
        p.Children().map_err(w)?.Append(part).map_err(w)?;
    }
    Ok(p)
}

fn icon(glyph: &str) -> R<TextBlock> {
    label(glyph, 14.0, TEXT, "Segoe Fluent Icons, Segoe MDL2 Assets")
}

fn build(root: &XamlRoot) -> R<Remote> {
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
    let via = words("", 11.0, QUIET)?;
    via.SetTextWrapping(TextWrapping::Wrap).map_err(w)?;
    via.SetMargin(thickness(0.0, 2.0, 0.0, 0.0)).map_err(w)?;
    // Live video wanted and no Streamlink: what's wrong, and the way to the settings that fix it.
    let warn_words = words("", 12.0, WARN_TEXT)?;
    warn_words.SetTextWrapping(TextWrapping::Wrap).map_err(w)?;
    let fix_words = words("Get Streamlink in Settings", 12.0, BODY)?;
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

    let (follow, stay, only) = (way("Favorites", Button::Follow, 66.0, false)?, way("This stream", Button::Stay, 74.0, false)?, way("This channel", Button::Only, 82.0, true)?);
    let ways = across(&[ui(&follow)?, ui(&stay)?, ui(&only)?])?;
    ways.SetMargin(thickness(0.0, 2.0, 0.0, 0.0)).map_err(w)?;
    // Read across, it's a sentence: this TwaTV plays favorites, this stream, or this channel. Under it, what that means.
    let plays = words("This TwaTV plays", 11.0, QUIET)?;
    plays.SetMargin(thickness(0.0, 10.0, 0.0, 0.0)).map_err(w)?;
    let way_hint = words(WAYS[0], 11.0, QUIET)?;
    way_hint.SetTextWrapping(TextWrapping::Wrap).map_err(w)?;
    way_hint.SetMargin(thickness(0.0, 4.0, 0.0, 0.0)).map_err(w)?;
    for (i, k) in [&follow, &stay, &only].into_iter().enumerate() {
        hints(k, i, &way_hint)?;
    }
    for (k, says) in [(&power, POWER_SAYS), (&mute, SOUND_SAYS), (&up, UP_SAYS), (&down, DOWN_SAYS)] {
        explains(k, says, &way_hint)?;
    }

    let every = words("All TwaTVs", 11.0, QUIET)?;
    every.SetMargin(thickness(0.0, 10.0, 0.0, 2.0)).map_err(w)?;
    let name = words("Top quality", 11.0, QUIET)?;
    name.SetWidth(120.0).map_err(w)?;
    name.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let top_value = words("", 12.0, TEXT)?;
    top_value.SetWidth(50.0).map_err(w)?;
    top_value.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let (lower, higher) = (stepper(MINUS, Button::QualityDown)?, stepper(PLUS, Button::QualityUp)?);
    higher.SetMargin(thickness(0.0, 0.0, 0.0, 0.0)).map_err(w)?;
    let quality = across(&[ui(&name)?, ui(&top_value)?, ui(&lower)?, ui(&higher)?])?;
    quality.SetMargin(thickness(0.0, 2.0, 0.0, 0.0)).map_err(w)?;

    let loud_name = words("Volume", 11.0, QUIET)?;
    loud_name.SetWidth(120.0).map_err(w)?;
    loud_name.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let loud_value = words("", 12.0, TEXT)?;
    loud_value.SetWidth(50.0).map_err(w)?;
    loud_value.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let (quieter, louder) = (stepper(MINUS, Button::VolumeDown)?, stepper(PLUS, Button::VolumeUp)?);
    louder.SetMargin(thickness(0.0, 0.0, 0.0, 0.0)).map_err(w)?;
    let loudness = across(&[ui(&loud_name)?, ui(&loud_value)?, ui(&quieter)?, ui(&louder)?])?;
    loudness.SetMargin(thickness(0.0, 4.0, 0.0, 0.0)).map_err(w)?;

    let heading = words("Favorites", 11.0, QUIET)?;
    heading.SetWidth(202.0).map_err(w)?;
    heading.SetVerticalAlignment(VerticalAlignment::Center).map_err(w)?;
    let another = stepper(PLUS, Button::AddFavorite)?;
    another.SetMargin(thickness(0.0, 0.0, 0.0, 0.0)).map_err(w)?;
    let heading_row = across(&[ui(&heading)?, ui(&another)?])?;
    heading_row.SetMargin(thickness(0.0, 10.0, 0.0, 3.0)).map_err(w)?;
    // With no favorites, there's nothing for it to show: this is what it needs first, so it's the biggest thing there.
    let first_words = words("+  Add a Twitch channel to watch", 12.0, BODY)?;
    first_words.SetFontWeight(windows::UI::Text::FontWeight { Weight: 600 }).map_err(w)?;
    let first = colored_key(&first_words, Button::AddFavorite, [ACCENT, ACCENT_OVER, ACCENT_DOWN])?;
    first.SetWidth(f64::NAN).map_err(w)?;
    first.SetHorizontalAlignment(HorizontalAlignment::Stretch).map_err(w)?;
    first.SetHeight(36.0).map_err(w)?;
    first.SetMargin(thickness(0.0, 12.0, 0.0, 0.0)).map_err(w)?;
    let nuke = words(NUKE, 11.0, QUIET)?;
    let nuke_key = tap_key(&nuke, [KEY, KEY_OVER, KEY_DOWN], nuke_pressed)?;
    nuke_key.SetWidth(f64::NAN).map_err(w)?;
    nuke_key.SetHorizontalAlignment(HorizontalAlignment::Stretch).map_err(w)?;
    nuke_key.SetHeight(28.0).map_err(w)?;
    nuke_key.SetMargin(thickness(0.0, 10.0, 0.0, 0.0)).map_err(w)?;
    let list = StackPanel::new().map_err(w)?;
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
            .Tapped(&TappedEventHandler::new(move |_, _| {
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
            }))
            .map_err(w)?;
        list.Children().map_err(w)?.Append(&ui(&line)?).map_err(w)?;
        rows.push((line, dot, name));
    }

    let head = panel(&[ui(&top)?, ui(&handle)?, ui(&title)?, ui(&about)?, ui(&via)?, ui(&warn)?])?;
    let controls = panel(&[ui(&keys)?, ui(&plays)?, ui(&ways)?, ui(&way_hint)?, ui(&every)?, ui(&quality)?, ui(&loudness)?, ui(&first)?, ui(&heading_row)?, ui(&list)?, ui(&nuke_key)?])?;
    let body = panel(&[logo()?, ui(&head)?, ui(&plan)?, ui(&controls)?])?;
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
    Ok(Remote { popup, head, controls, handle, state, light: dot, title, about, via, warn, warn_words, plan, power, follow, stay, only, way_hint, nuke, top: top_value, volume: loud_value, sound, heading, heading_row, first, rows, source: String::new(), placed: None })
}

/// What a set says about itself.
#[derive(Default, PartialEq, Debug)]
struct Info {
    handle: String,
    title: String,
    category: String,
    quality: String,
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
}

fn read(info: &str) -> Info {
    let mut out = Info::default();
    for line in info.lines() {
        match line.split('\t').collect::<Vec<_>>().as_slice() {
            ["now", handle, title, category, quality, state] => {
                (out.handle, out.title, out.category, out.quality, out.state) = (handle.to_string(), title.to_string(), category.to_string(), quality.to_string(), state.to_string());
            }
            ["how", plan] => out.plan = plan.to_string(),
            ["via", via] => out.via = via.to_string(),
            ["warn", warn] => out.warn = warn.to_string(),
            ["top", top] => out.top = top.to_string(),
            ["vol", volume] => out.volume = volume.to_string(),
            ["seen", ago] => out.seen = ago.parse().ok(),
            ["fav", handle, live] => out.favorites.push((handle.to_string(), *live != "0")),
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
        Some(ago) if ago < 5 => "Favorites, checked just now".into(),
        Some(ago) if ago < 90 => format!("Favorites, checked {ago} s ago"),
        Some(ago) => format!("Favorites, checked {} min ago", ago / 60),
        None => "Favorites".into(),
    }
}

fn show_status(r: &Remote) -> R<()> {
    let Some(screen) = crate::screen::status(&r.source) else { return Ok(()) };
    let info = read(&crate::screen::info(&r.source).unwrap_or_default());
    let shown = |on: bool| if on { Visibility::Visible } else { Visibility::Collapsed };
    r.head.SetVisibility(shown(!screen.note)).map_err(w)?;
    r.controls.SetVisibility(shown(!screen.note)).map_err(w)?;
    if screen.note {
        return say(&r.plan, &info.plan);
    }
    let live = info.state == "Live";
    r.light.SetBackground(&brush(if live { LIVE } else { NOT_LIVE })?).map_err(w)?;
    say(&r.state, &info.state)?;
    say(&r.handle, &info.handle)?;
    say(&r.title, &info.title)?;
    let about: Vec<&str> = [info.category.as_str(), info.quality.as_str()].into_iter().filter(|t| !t.is_empty()).collect();
    say(&r.about, &about.join("  ·  "))?;
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
        let words = if unfolded { "Hide offline".to_string() } else { format!("{} offline", offline.len()) };
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
        let info = read("now\tcrab\tpinching things\tJust Chatting\t720p60\tLive\nhow\tSwitches if lobster goes live\ntop\t480p\nseen\t40\nfav\tcrab\t1\nfav\tlobster\t0\n");
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
        assert_eq!(heading_text(None), "Favorites");
        assert_eq!(heading_text(Some(2)), "Favorites, checked just now");
        assert_eq!(heading_text(Some(40)), "Favorites, checked 40 s ago");
        assert_eq!(heading_text(Some(200)), "Favorites, checked 3 min ago");
    }
}
