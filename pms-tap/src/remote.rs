//! A `screen` meter's tooltip, which is a remote control: channel up and down, shuffle, and power for the TV and the console,
//! in the plastic of its day. Unlike the other tooltips it takes the pointer, so it stays while the pointer is on the TV, on
//! the remote, or on the way from one to the other. UI thread only, but for the wait before it goes.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use pms_screen::{Button, Status};
use pms_xaml::{Border, FontFamily, GradientStop, LinearGradientBrush, Orientation, PointerEventHandler, Popup, SolidColorBrush, StackPanel, TappedEventHandler, TextBlock, Thickness, UIElement, XamlRoot};
use windows::Foundation::{Point, Size};
use windows::UI::Color;
use windows_core::{HSTRING, Interface};

use crate::commands::R;
use crate::said::{Said, t};

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { A: 0xFF, R: r, G: g, B: b }
}

const FACE: Color = rgb(0xC0, 0xC0, 0xC0);
const LIGHT: Color = rgb(0xFF, 0xFF, 0xFF);
const SHADE: Color = rgb(0x80, 0x80, 0x80);
const DARK: Color = rgb(0x00, 0x00, 0x00);
const TITLE: Color = rgb(0x00, 0x00, 0x80);
const LIST: Color = rgb(0xFF, 0xFF, 0xFF);
const TITLE_END: Color = rgb(0x10, 0x84, 0xD0);
const LED_ON: Color = rgb(0x30, 0xE0, 0x50);
const LED_OFF: Color = rgb(0x50, 0x30, 0x30);
const DISPLAY: Color = rgb(0x10, 0x20, 0x10);
const DISPLAY_TEXT: Color = rgb(0x40, 0xFF, 0x60);
/// How wide the remote is.
const WIDTH: f64 = 176.0;
/// How long the pointer has to get from the TV to the remote.
const GRACE: Duration = Duration::from_millis(450);
/// How long the pointer has to rest on a set before its remote opens: passing over on the way to the tray shouldn't open it.
const REST: Duration = Duration::from_millis(300);
/// How often to see that the pointer's still where a remote that's up thinks it is, and how many misses in a row that takes to
/// be sure: it's allowed to be on its way from the TV.
const LOOK_EVERY: Duration = Duration::from_millis(250);
const MISSES: u32 = 3;
/// How long the nuke key asks (`t::TV_SURE`, in place of `t::TV_NUKE`) before it stops asking. While it's asking, if this is the last
/// set and the console's running, the display says the console goes with it (`t::GAME_GOES`).
const STAYS_SURE: Duration = Duration::from_secs(3);

/// Which time a remote has been opened: an older watch stops.
static WATCHING: AtomicU32 = AtomicU32::new(0);
/// A set the pointer is resting on: its root, which, and where its cell starts and how wide it is.
type Resting = (XamlRoot, String, f64, f64, f64);

/// Which time the pointer has come to rest on a set: an older wait lets it be.
static RESTING: AtomicU32 = AtomicU32::new(0);
/// Which time the nuke key has asked: an older wait lets it be.
static ASKING: AtomicU32 = AtomicU32::new(0);
/// Which time the remote's been opened: its refreshing stops when this moves on.
static OPENED: AtomicU32 = AtomicU32::new(0);
/// How often an open remote looks at its set again: shuffling changes the channel, and the console comes and goes.
const REFRESH: Duration = Duration::from_millis(500);
/// How wide the console's own tooltip is, and the gap between it and the remote.
const CONSOLE_WIDTH: f64 = crate::pimpbox::WIDTH;
const CONSOLE_GAP: f64 = 4.0;

struct Remote {
    popup: Popup,
    title: TextBlock,
    /// Its title bar: what flashes, as a window's does, when the console's display beside it takes the keys and the mouse.
    bar: Border,
    display: TextBlock,
    tv_led: Border,
    shuffle_led: Border,
    jitter_led: Border,
    /// How long a shuffling TV stays on a channel.
    every: TextBlock,
    /// What the nuke key says.
    nuke: TextBlock,
    /// The words built in, said again when the language changes.
    said: Said,
    /// The console's own tooltip, beside the remote while the set's on its channel, and what it says.
    /// The console's own tooltip, beside it.
    console: crate::pimpbox::Tooltip,
    /// Number One's card, beside the remote while the set's on its channel, and its lines. The one beside the console's display
    /// has none: it would never be shown.
    iss: Option<Popup>,
    iss_lines: Vec<TextBlock>,
    /// It's the one beside the console's display: where the console's tooltip goes, the other side of that.
    dock: Option<(f64, f64)>,
    /// Where the remote went, for the console's tooltip to go beside it: left, top, and the room there is across.
    placed: Cell<(f64, f64, f64)>,
    /// Each machine's power light.
    lights: Vec<Border>,
    /// The channel list's rows, and each one's words.
    /// Each channel in the list: its number, its line, its words, and the mark that its picture pops out, if it does.
    channels: Vec<(u32, Border, TextBlock, Option<TextBlock>)>,
    source: String,
}

/// A machine plugged into the TV.
struct Machine {
    name: &'static str,
    power: Button,
    /// What the button that goes to its channel says, and does.
    watch: t::Id,
    channel: Button,
}

const MACHINES: [Machine; 1] = [Machine { name: "PIMPBOX", power: Button::GamePower, watch: t::GOTO_GAME, channel: Button::GameChannel }];

thread_local! {
    static REMOTE: RefCell<Option<Remote>> = const { RefCell::new(None) };
    /// Another, beside the console's display while that's out on the desktop.
    static DOCKED: RefCell<Option<Remote>> = const { RefCell::new(None) };
    /// The remote being built is that one.
    static BUILDING_DOCKED: Cell<bool> = const { Cell::new(false) };
    static ON_TILE: Cell<bool> = const { Cell::new(false) };
    static ON_REMOTE: Cell<bool> = const { Cell::new(false) };
    /// The set the pointer's resting on, while it waits to open its remote: where it is, and which.
    static WAITING: RefCell<Option<Resting>> = const { RefCell::new(None) };
    /// The nuke key has been pressed once, and is asking.
    static ARMED: Cell<bool> = const { Cell::new(false) };
    /// The TV under the pointer has just been clicked: its remote stays away until the pointer's left it, rather than come up
    /// again over whatever the click opened (the console's display, say).
    static CLICKED: Cell<bool> = const { Cell::new(false) };
    /// How many times the console's display had taken the keys and the mouse when last seen; -1 when it wasn't there. And
    /// whether it has them now.
    static TAKEN_SEEN: Cell<i32> = const { Cell::new(-1) };
    static HELD: Cell<bool> = const { Cell::new(false) };
}

/// A flash of the docked remote's title bar lasts this long, on and off; and is this green (the display's own).
const FLASH: Duration = Duration::from_millis(110);
const FLASH_GREEN: Color = rgb(0x33, 0xFF, 0x66);

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

fn font() -> R<FontFamily> {
    crate::stage::font("Tahoma, Segoe UI")
}

fn label(text: &str, size: f64, color: Color) -> R<TextBlock> {
    let t = TextBlock::new().map_err(w)?;
    t.SetText(&HSTRING::from(text)).map_err(w)?;
    t.SetFontSize(size).map_err(w)?;
    t.SetFontFamily(&font()?).map_err(w)?;
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
fn raised(child: &UIElement) -> R<(Border, [Border; 4])> {
    let face = Border::new().map_err(w)?;
    face.SetBackground(&brush(FACE)?).map_err(w)?;
    face.SetChild(child).map_err(w)?;
    let (shade, light) = edge(LIGHT, SHADE, &ui(&face)?)?;
    let (outer, top) = edge(FACE, DARK, &ui(&shade)?)?;
    Ok((outer.clone(), [outer, top, shade, light]))
}

/// The same edge the other way in: a thing set into the panel.
fn sunken(child: &UIElement) -> R<Border> {
    let (inner, _) = edge(DARK, FACE, child)?;
    let (outer, _) = edge(SHADE, LIGHT, &ui(&inner)?)?;
    Ok(outer)
}

fn set_edge(parts: &[Border; 4], pushed: bool) -> R<()> {
    let [outer, top, shade, light] = parts;
    let (o, t, s, l) = if pushed { (LIGHT, DARK, FACE, SHADE) } else { (DARK, FACE, SHADE, LIGHT) };
    outer.SetBorderBrush(&brush(o)?).map_err(w)?;
    top.SetBorderBrush(&brush(t)?).map_err(w)?;
    shade.SetBorderBrush(&brush(s)?).map_err(w)?;
    light.SetBorderBrush(&brush(l)?).map_err(w)
}

fn led() -> R<Border> {
    let b = Border::new().map_err(w)?;
    b.SetWidth(7.0).map_err(w)?;
    b.SetHeight(7.0).map_err(w)?;
    b.SetBackground(&brush(LED_OFF)?).map_err(w)?;
    b.SetBorderBrush(&brush(SHADE)?).map_err(w)?;
    b.SetBorderThickness(thickness(1.0, 1.0, 1.0, 1.0)).map_err(w)?;
    b.SetMargin(thickness(0.0, 0.0, 4.0, 0.0)).map_err(w)?;
    b.SetVerticalAlignment(pms_xaml::VerticalAlignment::Center).map_err(w)?;
    Ok(b)
}

/// A button, with an LED if it has one. It goes in when it's pressed.
fn button(text: &str, press: Button, light: Option<&Border>, width: f64) -> R<Border> {
    let docked = building_docked();
    let (b, _) = key(text, light, width, move || pressed_from(press, docked))?;
    Ok(b)
}

/// A button whose words are the language's, kept in `said`.
fn said_button(said: &mut Said, text: t::Id, press: Button, light: Option<&Border>, width: f64) -> R<Border> {
    let docked = building_docked();
    let (b, words) = key("", light, width, move || pressed_from(press, docked))?;
    said.keep(&words, text)?;
    Ok(b)
}

/// A button that does `then`, and the words on it.
fn key(text: &str, light: Option<&Border>, width: f64, then: impl Fn() + Send + 'static) -> R<(Border, TextBlock)> {
    let row = crate::stage::stack_panel()?;
    row.SetOrientation(Orientation::Horizontal).map_err(w)?;
    row.SetHorizontalAlignment(pms_xaml::HorizontalAlignment::Center).map_err(w)?;
    if let Some(light) = light {
        row.Children().map_err(w)?.Append(&ui(light)?).map_err(w)?;
    }
    let words = label(text, 10.0, DARK)?;
    row.Children().map_err(w)?.Append(&ui(&words)?).map_err(w)?;
    let padded = Border::new().map_err(w)?;
    padded.SetPadding(thickness(4.0, 2.0, 4.0, 2.0)).map_err(w)?;
    padded.SetChild(&ui(&row)?).map_err(w)?;
    let (b, parts) = raised(&ui(&padded)?)?;
    b.SetWidth(width).map_err(w)?;
    b.SetMargin(thickness(0.0, 0.0, 3.0, 3.0)).map_err(w)?;
    let el = ui(&b)?;
    let down = parts.clone();
    el.PointerPressed(&PointerEventHandler::new(move |_, _| crate::guard(|| {
        let _ = set_edge(&down, true);
        Ok(())
    })))
    .map_err(w)?;
    for release in [true, false] {
        let up = parts.clone();
        let handler = PointerEventHandler::new(move |_, _| crate::guard(|| {
            let _ = set_edge(&up, false);
            Ok(())
        }));
        if release {
            el.PointerReleased(&handler).map_err(w)?;
        } else {
            el.PointerExited(&handler).map_err(w)?;
        }
    }
    el.Tapped(&TappedEventHandler::new(move |_, _| crate::guard(|| {
        then();
        Ok(())
    })))
    .map_err(w)?;
    Ok((b, words))
}

/// The nuke key: it's right beside the one that adds a TV, so the first press only asks.
fn nuke_pressed(docked: bool) {
    if !ARMED.replace(true) {
        let mine = ASKING.fetch_add(1, Ordering::Relaxed) + 1;
        (if docked { &DOCKED } else { &REMOTE }).with_borrow(|r| {
            if let Some(r) = r {
                let _ = r.nuke.SetText(&HSTRING::from(t::TV_SURE.t()));
                let info = crate::screen::info(&r.source).unwrap_or_default();
                if info.lines().any(|l| l == "alone\t1") {
                    let _ = r.display.SetText(&HSTRING::from(t::GAME_GOES.t()));
                }
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
    pressed_from(Button::NukeTv, docked);
    close();
}

fn calm_down() {
    if !ARMED.replace(false) {
        return;
    }
    ASKING.fetch_add(1, Ordering::Relaxed);
    for remote in [&REMOTE, &DOCKED] {
        remote.with_borrow(|r| {
            if let Some(r) = r {
                let _ = r.nuke.SetText(&HSTRING::from(t::TV_NUKE.t()));
            }
        });
    }
    refresh();
}

/// Whether the remote that's being built just now is the one beside the console's display: for a key to know, as it's made,
/// which of the two remotes it's on, and so whose set it acts for.
pub(crate) fn building_docked() -> bool {
    BUILDING_DOCKED.get()
}

/// The set a remote's for: the one beside the console's display, or the one over the taskbar.
fn set_of(docked: bool) -> Option<String> {
    (if docked { &DOCKED } else { &REMOTE }).with_borrow(|r| r.as_ref().map(|r| r.source.clone()))
}

/// A key's been pressed on one of the remotes: `docked`, the one beside the console's display.
pub(crate) fn pressed_from(button: Button, docked: bool) {
    if let Some(source) = set_of(docked) {
        crate::screen::press(&source, button);
    }
    // What the press did comes back in the screen's status.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(120));
        crate::meters::on_ui(refresh);
    });
}

fn tuned(channel: u32, docked: bool) {
    if let Some(source) = set_of(docked) {
        crate::screen::tune(&source, channel);
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(120));
        crate::meters::on_ui(refresh);
    });
}

fn row(items: &[&Border]) -> R<StackPanel> {
    let r = crate::stage::stack_panel()?;
    r.SetOrientation(Orientation::Horizontal).map_err(w)?;
    for item in items {
        r.Children().map_err(w)?.Append(&ui(*item)?).map_err(w)?;
    }
    Ok(r)
}

/// The navy-to-blue of a title bar.
fn title_brush() -> R<LinearGradientBrush> {
    let gradient = LinearGradientBrush::new().map_err(w)?;
    gradient.SetStartPoint(Point { X: 0.0, Y: 0.5 }).map_err(w)?;
    gradient.SetEndPoint(Point { X: 1.0, Y: 0.5 }).map_err(w)?;
    let stops = gradient.GradientStops().map_err(w)?;
    for (color, at) in [(TITLE, 0.0), (TITLE_END, 1.0)] {
        let stop = GradientStop::new().map_err(w)?;
        stop.SetColor(color).map_err(w)?;
        stop.SetOffset(at).map_err(w)?;
        stops.Append(&stop).map_err(w)?;
    }
    Ok(gradient)
}

fn build(root: &XamlRoot, docked: bool) -> R<Remote> {
    // Every key made from here to the end of this knows which remote it's on.
    BUILDING_DOCKED.set(docked);
    let built = build_one(root, docked);
    BUILDING_DOCKED.set(false);
    built
}

fn build_one(root: &XamlRoot, docked: bool) -> R<Remote> {
    let mut said = Said::default();
    let title = label("TV", 10.0, LIGHT)?;
    title.SetFontWeight(windows::UI::Text::FontWeight { Weight: 700 }).map_err(w)?;
    let bar = Border::new().map_err(w)?;
    bar.SetBackground(&title_brush()?.cast::<pms_xaml::Brush>().map_err(w)?).map_err(w)?;
    bar.SetPadding(thickness(4.0, 2.0, 4.0, 2.0)).map_err(w)?;
    bar.SetMargin(thickness(2.0, 2.0, 2.0, 0.0)).map_err(w)?;
    bar.SetChild(&ui(&title)?).map_err(w)?;

    let display = label("CH 2", 12.0, DISPLAY_TEXT)?;
    display.SetFontFamily(&crate::stage::font("Cascadia Mono, Consolas")?).map_err(w)?;
    let glass = Border::new().map_err(w)?;
    glass.SetBackground(&brush(DISPLAY)?).map_err(w)?;
    glass.SetPadding(thickness(5.0, 2.0, 5.0, 2.0)).map_err(w)?;
    glass.SetChild(&ui(&display)?).map_err(w)?;
    let screen = sunken(&ui(&glass)?)?;
    screen.SetMargin(thickness(0.0, 0.0, 3.0, 4.0)).map_err(w)?;

    let (tv_led, shuffle_led, jitter_led) = (led()?, led()?, led()?);
    let power = said_button(&mut said, t::TV_POWER, Button::TvPower, Some(&tv_led), 64.0)?;
    let up = said_button(&mut said, t::CHANNEL_UP, Button::ChannelUp, None, 46.0)?;
    let down = said_button(&mut said, t::CHANNEL_DOWN, Button::ChannelDown, None, 46.0)?;
    let shuffle = said_button(&mut said, t::SHUFFLE, Button::Shuffle, Some(&shuffle_led), 84.0)?;
    let jitter = said_button(&mut said, t::JITTER, Button::Jitter, Some(&jitter_led), 75.0)?;
    let first = row(&[&power, &up, &down])?;
    let second = row(&[&shuffle, &jitter])?;

    // How long a shuffling TV stays on a channel: every TV, whichever remote says so.
    let every_label = label("", 10.0, DARK)?;
    said.keep(&every_label, t::EVERY)?;
    every_label.SetWidth(40.0).map_err(w)?;
    every_label.SetVerticalAlignment(pms_xaml::VerticalAlignment::Center).map_err(w)?;
    let every = label("", 11.0, DISPLAY_TEXT)?;
    every.SetFontFamily(&crate::stage::font("Cascadia Mono, Consolas")?).map_err(w)?;
    every.SetHorizontalAlignment(pms_xaml::HorizontalAlignment::Center).map_err(w)?;
    let every_glass = Border::new().map_err(w)?;
    every_glass.SetBackground(&brush(DISPLAY)?).map_err(w)?;
    every_glass.SetPadding(thickness(3.0, 1.0, 3.0, 1.0)).map_err(w)?;
    every_glass.SetChild(&ui(&every)?).map_err(w)?;
    let every_screen = sunken(&ui(&every_glass)?)?;
    every_screen.SetWidth(63.0).map_err(w)?;
    every_screen.SetMargin(thickness(0.0, 0.0, 3.0, 3.0)).map_err(w)?;
    let sooner = button("-", Button::ShuffleSooner, None, 26.0)?;
    let later = button("+", Button::ShuffleLater, None, 27.0)?;
    let third = crate::stage::stack_panel()?;
    third.SetOrientation(Orientation::Horizontal).map_err(w)?;
    for part in [ui(&every_label)?, ui(&every_screen)?, ui(&sooner)?, ui(&later)?] {
        third.Children().map_err(w)?.Append(&part).map_err(w)?;
    }

    // Every channel, to go straight to.
    let list = crate::stage::stack_panel()?;
    let mut channels = Vec::new();
    for (number, name) in pms_screen::CHANNELS {
        let words = label(&format!("{number:<5}{name}"), 10.0, DARK)?;
        words.SetFontFamily(&crate::stage::font("Cascadia Mono, Consolas")?).map_err(w)?;
        let line = Border::new().map_err(w)?;
        line.SetBackground(&brush(LIST)?).map_err(w)?;
        line.SetPadding(thickness(4.0, 1.0, 4.0, 1.0)).map_err(w)?;
        // A channel whose picture a click pops out into a window says so: the sign for that, at the end of its line.
        let pops = if pms_screen::POPS_OUT.contains(&number) {
            words.SetText(&HSTRING::from(format!("{number:<5}{name:<11}"))).map_err(w)?;
            let sign = label("\u{E8A7}", 10.0, DARK)?;
            sign.SetFontFamily(&crate::stage::font("Segoe Fluent Icons, Segoe MDL2 Assets")?).map_err(w)?;
            sign.SetVerticalAlignment(pms_xaml::VerticalAlignment::Center).map_err(w)?;
            let both = crate::stage::stack_panel()?;
            both.SetOrientation(Orientation::Horizontal).map_err(w)?;
            both.Children().map_err(w)?.Append(&ui(&words)?).map_err(w)?;
            both.Children().map_err(w)?.Append(&ui(&sign)?).map_err(w)?;
            line.SetChild(&ui(&both)?).map_err(w)?;
            Some(sign)
        } else {
            line.SetChild(&ui(&words)?).map_err(w)?;
            None
        };
        let docked = building_docked();
        ui(&line)?
            .Tapped(&TappedEventHandler::new(move |_, _| crate::guard(|| {
                tuned(number, docked);
                Ok(())
            })))
            .map_err(w)?;
        list.Children().map_err(w)?.Append(&ui(&line)?).map_err(w)?;
        channels.push((number, line, words, pops));
    }
    let listed = sunken(&ui(&list)?)?;
    listed.SetMargin(thickness(0.0, 1.0, 3.0, 3.0)).map_err(w)?;

    // The machines plugged in, under what a remote of the day called them.
    let machines = crate::stage::stack_panel()?;
    let heading = label("", 9.0, SHADE)?;
    said.keep(&heading, t::AUX)?;
    heading.SetMargin(thickness(1.0, 3.0, 0.0, 2.0)).map_err(w)?;
    machines.Children().map_err(w)?.Append(&ui(&heading)?).map_err(w)?;
    let mut lights = Vec::new();
    for machine in MACHINES {
        let light = led()?;
        let on = button(machine.name, machine.power, Some(&light), 84.0)?;
        let watch = said_button(&mut said, machine.watch, machine.channel, None, 75.0)?;
        machines.Children().map_err(w)?.Append(&ui(&row(&[&on, &watch])?)?).map_err(w)?;
        lights.push(light);
    }
    let another = said_button(&mut said, t::ADD_TV, Button::AddTv, None, 68.0)?;
    let docked_remote = building_docked();
    let (nuke_key, nuke) = key("", None, 91.0, move || nuke_pressed(docked_remote))?;
    said.keep(&nuke, t::TV_NUKE)?;
    machines.Children().map_err(w)?.Append(&ui(&row(&[&another, &nuke_key])?)?).map_err(w)?;

    let body = crate::stage::stack_panel()?;
    body.SetMargin(thickness(5.0, 5.0, 2.0, 2.0)).map_err(w)?;
    for part in [ui(&screen)?, ui(&first)?, ui(&second)?, ui(&third)?, ui(&listed)?, ui(&machines)?] {
        body.Children().map_err(w)?.Append(&part).map_err(w)?;
    }
    let window = crate::stage::stack_panel()?;
    window.Children().map_err(w)?.Append(&ui(&bar)?).map_err(w)?;
    window.Children().map_err(w)?.Append(&ui(&body)?).map_err(w)?;
    let (whole, _) = raised(&ui(&window)?)?;
    // Whole pixels, or the window it goes in rounds its right edge away.
    whole.SetWidth(WIDTH).map_err(w)?;

    let el = ui(&whole)?;
    follow_pointer(&el)?;

    let popup = Popup::new().map_err(w)?;
    popup.SetChild(&el).map_err(w)?;
    popup.SetShouldConstrainToRootBounds(false).map_err(w)?;
    popup.SetXamlRoot(root).map_err(w)?;
    let console = crate::pimpbox::build(root, docked)?;
    let (iss, iss_lines) = if docked { (None, Vec::new()) } else { iss_card(root, &mut said).map(|(p, l)| (Some(p), l))? };
    Ok(Remote {
        popup,
        title,
        bar,
        display,
        tv_led,
        shuffle_led,
        jitter_led,
        every,
        nuke,
        said,
        console,
        iss,
        iss_lines,
        dock: None,
        placed: Cell::new((0.0, 0.0, 0.0)),
        lights,
        channels,
        source: String::new(),
    })
}

/// The most lines Number One's card has.
const ISS_LINES: usize = 14;
/// Its title bar, pee-yellow to amber, as PTV's.
const ISS_TITLE: Color = rgb(0xF6, 0xD2, 0x2E);
const ISS_TITLE_END: Color = rgb(0xC4, 0x78, 0x12);

/// Number One's card, beside the remote while the set's on its channel: the ISS's water, as the set says it in its `info`, a
/// line each. Built once, with the remote, its lines all there and shown as they're wanted.
fn iss_card(root: &XamlRoot, said: &mut Said) -> R<(Popup, Vec<TextBlock>)> {
    let title = label("NUMBER ONE", 10.0, DARK)?;
    title.SetFontWeight(windows::UI::Text::FontWeight { Weight: 700 }).map_err(w)?;
    let gradient = LinearGradientBrush::new().map_err(w)?;
    gradient.SetStartPoint(Point { X: 0.0, Y: 0.5 }).map_err(w)?;
    gradient.SetEndPoint(Point { X: 1.0, Y: 0.5 }).map_err(w)?;
    for (color, at) in [(ISS_TITLE, 0.0), (ISS_TITLE_END, 1.0)] {
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
    let body = crate::stage::stack_panel()?;
    body.SetMargin(thickness(5.0, 4.0, 4.0, 4.0)).map_err(w)?;
    let mut lines = Vec::new();
    for _ in 0..ISS_LINES {
        let line = label("", 10.0, DARK)?;
        line.SetTextWrapping(pms_xaml::TextWrapping::Wrap).map_err(w)?;
        line.SetMargin(thickness(0.0, 0.0, 0.0, 3.0)).map_err(w)?;
        line.SetVisibility(pms_xaml::Visibility::Collapsed).map_err(w)?;
        body.Children().map_err(w)?.Append(&ui(&line)?).map_err(w)?;
        lines.push(line);
    }
    // PTV's bumps, on demand: the next of them all.
    let (bump, words) = key("", None, CONSOLE_WIDTH - 16.0, || tuned(pms_screen::BUMP_ANY, false))?;
    said.keep(&words, t::PLAY_BUMP)?;
    bump.SetMargin(thickness(0.0, 2.0, 3.0, 1.0)).map_err(w)?;
    body.Children().map_err(w)?.Append(&ui(&bump)?).map_err(w)?;
    let window = crate::stage::stack_panel()?;
    window.Children().map_err(w)?.Append(&ui(&bar)?).map_err(w)?;
    window.Children().map_err(w)?.Append(&ui(&body)?).map_err(w)?;
    let (whole, _) = raised(&ui(&window)?)?;
    whole.SetWidth(CONSOLE_WIDTH).map_err(w)?;
    let el = ui(&whole)?;
    follow_pointer(&el)?;
    let popup = Popup::new().map_err(w)?;
    popup.SetChild(&el).map_err(w)?;
    popup.SetShouldConstrainToRootBounds(false).map_err(w)?;
    popup.SetXamlRoot(root).map_err(w)?;
    Ok((popup, lines))
}

/// Shows or hides Number One's card, beside the remote: while the set's on its channel, saying what its `info` says.
fn show_iss(r: &Remote, s: &Status, info: &str) -> R<()> {
    let Some(card) = &r.iss else { return Ok(()) };
    let said: Vec<&str> = info.lines().filter_map(|l| l.strip_prefix("iss\t")).collect();
    if r.dock.is_some() || !s.tv_on || s.channel != pms_screen::ISS_CHANNEL || said.is_empty() || !r.popup.IsOpen().unwrap_or(false) {
        return card.SetIsOpen(false).map_err(w);
    }
    for (i, line) in r.iss_lines.iter().enumerate() {
        match said.get(i) {
            Some(text) => {
                line.SetText(&HSTRING::from(*text)).map_err(w)?;
                // The tank first, and big; where it's from last, and small.
                let (size, weight, color) = if i == 0 { (11.0, 700, DARK) } else if i + 1 == said.len() { (9.0, 400, SHADE) } else { (10.0, 400, DARK) };
                line.SetFontSize(size).map_err(w)?;
                line.SetFontWeight(windows::UI::Text::FontWeight { Weight: weight }).map_err(w)?;
                line.SetForeground(&brush(color)?).map_err(w)?;
                line.SetVisibility(pms_xaml::Visibility::Visible).map_err(w)?;
            }
            None => line.SetVisibility(pms_xaml::Visibility::Collapsed).map_err(w)?,
        }
    }
    let child = card.Child().map_err(w)?.cast::<pms_xaml::FrameworkElement>().map_err(w)?;
    child.Measure(Size { Width: f32::INFINITY, Height: f32::INFINITY }).map_err(w)?;
    let tall = child.DesiredSize().map_err(w)?.Height as f64;
    // Where the console's card would be: beside the remote, its bottom with the remote's, to the right if there's room.
    let (left, bottom, room) = r.placed.get();
    let right = left + WIDTH + CONSOLE_GAP;
    let x = if right + CONSOLE_WIDTH + 4.0 <= room { right } else { (left - CONSOLE_GAP - CONSOLE_WIDTH).max(0.0) };
    card.SetHorizontalOffset(x.floor()).map_err(w)?;
    card.SetVerticalOffset((bottom - tall).floor()).map_err(w)?;
    let was_open = card.IsOpen().unwrap_or(false);
    card.SetIsOpen(true).map_err(w)?;
    if !was_open {
        float_popups();
    }
    Ok(())
}

/// What the console's tooltip says: the game, how the console is, and what to do. None: it isn't shown, as the set isn't showing
/// the console. `info` is what the set says about itself.
fn console_words(s: &Status, info: &str, always: bool) -> Option<(String, String, String)> {
    if !always && (!s.tv_on || s.channel != pms_screen::GAME_CHANNEL) {
        return None;
    }
    let field = |name: &str| info.lines().find_map(|l| l.strip_prefix(name).and_then(|v| v.strip_prefix('\t'))).unwrap_or_default().to_string();
    let (game, evicted) = (field("game"), field("evicted"));
    if !s.has_game {
        return Some((t::NO_GAME.t().into(), t::NO_GAME_STATE.t().into(), t::NO_GAME_HELP.t().into()));
    }
    let state = if !evicted.is_empty() {
        t::CONSOLE_EVICTED.with(&[("why", &evicted.to_lowercase())])
    } else if s.game_on {
        t::CONSOLE_ON.t().into()
    } else {
        t::CONSOLE_OFF.t().into()
    };
    // Beside its display: whether it has the keys and the mouse, and how they go back or come to it, as a virtual machine's
    // window says it.
    let help = match (s.game_on && evicted.is_empty(), always) {
        (true, false) => t::CLICK_TO_PLAY.t(),
        (false, false) => t::CLICK_TO_POP_OUT.t(),
        (true, true) if HELD.get() => t::CAPTURED.t(),
        (true, true) => t::CLICK_TO_CAPTURE.t(),
        (false, true) => "",
    };
    Some((game.to_uppercase(), state, help.into()))
}

/// Shows or hides the console's tooltip, beside the remote.
fn show_console(r: &Remote, s: &Status, info: &str) -> R<()> {
    let Some((game, state, help)) = console_words(s, info, r.dock.is_some()).filter(|_| r.popup.IsOpen().unwrap_or(false)) else {
        return r.console.popup.SetIsOpen(false).map_err(w);
    };
    let line = |name: &str| info.lines().find_map(|l| l.strip_prefix(name).and_then(|v| v.strip_prefix('\t'))).unwrap_or_default();
    let video = line("video");
    // The speed demo the console's showing, while it's on and showing one.
    let showing = crate::pimpbox::Showing { demo: line("demo"), runner: line("runner"), about: line("demo_about") };
    let showing = (s.game_on && s.has_game && !showing.demo.is_empty()).then_some(&showing);
    r.console.say(&game, &state, &help, video, showing, s.game_on && s.has_game)?;
    if let Some((x, y)) = r.dock {
        r.console.popup.SetHorizontalOffset(x.floor()).map_err(w)?;
        r.console.popup.SetVerticalOffset(y.floor()).map_err(w)?;
        return r.console.popup.SetIsOpen(true).map_err(w);
    }
    let child = r.console.popup.Child().map_err(w)?.cast::<pms_xaml::FrameworkElement>().map_err(w)?;
    child.Measure(Size { Width: f32::INFINITY, Height: f32::INFINITY }).map_err(w)?;
    let tall = child.DesiredSize().map_err(w)?.Height as f64;
    // Beside the remote, its bottom with the remote's: to the right if there's room, else to the left.
    let (left, bottom, room) = r.placed.get();
    let right = left + WIDTH + CONSOLE_GAP;
    let x = if right + CONSOLE_WIDTH + 4.0 <= room { right } else { (left - CONSOLE_GAP - CONSOLE_WIDTH).max(0.0) };
    r.console.popup.SetHorizontalOffset(x.floor()).map_err(w)?;
    r.console.popup.SetVerticalOffset((bottom - tall).floor()).map_err(w)?;
    let was_open = r.console.popup.IsOpen().unwrap_or(false);
    r.console.popup.SetIsOpen(true).map_err(w)?;
    if !was_open {
        float_popups();
    }
    Ok(())
}

/// Whether the pointer's within one of this process's popup windows that's showing, by where they are: asking what's under the
/// pointer wouldn't do, because these let that question through to what's behind them. Any thread.
fn pointer_on_popup() -> bool {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetCursorPos, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible};
    let mut at = POINT::default();
    if unsafe { GetCursorPos(&mut at) }.is_err() {
        return true;
    }
    let mut after = None;
    while let Ok(window) = unsafe { FindWindowExW(None, after, windows_core::w!("Xaml_WindowedPopupClass"), windows_core::PCWSTR::null()) } {
        after = Some(window);
        let (mut process, mut area) = (0, RECT::default());
        unsafe { GetWindowThreadProcessId(window, Some(&mut process)) };
        let ours = process == unsafe { GetCurrentProcessId() } && unsafe { IsWindowVisible(window) }.as_bool();
        if ours && unsafe { GetWindowRect(window, &mut area) }.is_ok() && (area.left..area.right).contains(&at.x) && (area.top..area.bottom).contains(&at.y) {
            return true;
        }
    }
    false
}

/// Watches the pointer for as long as a remote that's just opened stays up. A window that comes over it, like the settings it can
/// open, takes the pointer with no word to it: it would stay up out of sight, believing it still has it, and hovering over the TV
/// would do nothing. This is another thread, so it costs the taskbar nothing.
pub(crate) fn watch_pointer() {
    let mine = WATCHING.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        let mut misses = 0;
        while WATCHING.load(Ordering::Relaxed) == mine {
            std::thread::sleep(LOOK_EVERY);
            misses = if pointer_on_popup() { 0 } else { misses + 1 };
            if misses >= MISSES {
                crate::meters::on_ui(|| {
                    ON_REMOTE.set(false);
                    if !ON_TILE.get() {
                        close();
                    }
                });
            }
        }
    });
}

/// Keeps a remote up while the pointer's on `el`.
pub(crate) fn follow_pointer(el: &UIElement) -> R<()> {
    el.PointerEntered(&PointerEventHandler::new(|_, _| crate::guard(|| {
        ON_REMOTE.set(true);
        Ok(())
    })))
    .map_err(w)?;
    el.PointerExited(&PointerEventHandler::new(|_, _| crate::guard(|| {
        ON_REMOTE.set(false);
        later();
        Ok(())
    })))
    .map_err(w)?;
    Ok(())
}

/// Puts a remote above a cell starting at `left` and `top` and `width` wide, on the screen, on whole pixels.
pub(crate) fn place(popup: &Popup, root: &XamlRoot, left: f64, top: f64, width: f64) -> R<()> {
    let child = popup.Child().map_err(w)?.cast::<pms_xaml::FrameworkElement>().map_err(w)?;
    child.Measure(Size { Width: f32::INFINITY, Height: f32::INFINITY }).map_err(w)?;
    let size = child.DesiredSize().map_err(w)?;
    let room = root.Size().map_or(f32::MAX, |s| s.Width) as f64;
    let x = (left + width / 2.0 - size.Width as f64 / 2.0).min(room - size.Width as f64 - 4.0).max(0.0);
    popup.SetHorizontalOffset(x.floor()).map_err(w)?;
    popup.SetVerticalOffset((top - size.Height as f64 - 6.0).floor()).map_err(w)
}

fn set_led(led: &Border, on: bool) -> R<()> {
    led.SetBackground(&brush(if on { LED_ON } else { LED_OFF })?).map_err(w)
}

fn refresh() {
    for remote in [&REMOTE, &DOCKED] {
        remote.with_borrow(|r| {
            let Some(r) = r.as_ref().filter(|r| r.popup.IsOpen().unwrap_or(false)) else { return };
            let s = crate::screen::status(&r.source).unwrap_or_default();
            let _ = show_status(r, s);
        });
    }
}

/// Where the console's display is on the screen, if it's out on the desktop (left, top, right, bottom), and where the
/// taskbar's own window starts, all in the screen's pixels; which window's in front just now (a number that's only for
/// telling when that changes); how many times the display's taken the keys and the mouse, and whether it has them now (1 or
/// 0): the properties it puts on its window for that. Any thread.
pub(crate) fn display_place() -> Option<[i32; 9]> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetForegroundWindow, GetPropW, GetWindowRect, IsIconic, IsWindowVisible};
    unsafe {
        let display = FindWindowW(windows_core::w!("PimpMyShellPimpbox"), windows_core::PCWSTR::null()).ok()?;
        if !IsWindowVisible(display).as_bool() || IsIconic(display).as_bool() {
            return None;
        }
        let taskbar = FindWindowW(windows_core::w!("Shell_TrayWnd"), windows_core::PCWSTR::null()).ok()?;
        let (mut at, mut bar) = (RECT::default(), RECT::default());
        GetWindowRect(display, &mut at).ok()?;
        GetWindowRect(taskbar, &mut bar).ok()?;
        // By number, as the display names them (pms-pimpbox's play.rs: its job has no global atoms to name them by string).
        let taken = GetPropW(display, windows_core::PCWSTR(0xB7A1 as *const u16)).0 as usize as i32;
        let held = GetPropW(display, windows_core::PCWSTR(0xB7A2 as *const u16)).0 as usize as i32;
        Some([at.left, at.top, at.right, at.bottom, bar.left, bar.top, GetForegroundWindow().0 as usize as i32, taken, held])
    }
}

/// Our popups' windows that are showing (each XAML popup is a window of its own, `Xaml_WindowedPopupClass`), and of those, which
/// are the ones docked beside the console's display: those touch its left or right edge, their tops with its top. Any thread.
fn our_popups() -> (Vec<windows::Win32::Foundation::HWND>, Vec<windows::Win32::Foundation::HWND>) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible};
    let (mut floating, mut beside) = (Vec::new(), Vec::new());
    unsafe {
        let mut display = RECT::default();
        let docked_to = FindWindowW(windows_core::w!("PimpMyShellPimpbox"), windows_core::PCWSTR::null()).ok().filter(|d| IsWindowVisible(*d).as_bool() && GetWindowRect(*d, &mut display).is_ok());
        let mut after = None;
        while let Ok(window) = FindWindowExW(None, after, windows_core::w!("Xaml_WindowedPopupClass"), windows_core::PCWSTR::null()) {
            after = Some(window);
            let (mut process, mut area) = (0, RECT::default());
            GetWindowThreadProcessId(window, Some(&mut process));
            if process != GetCurrentProcessId() || !IsWindowVisible(window).as_bool() || GetWindowRect(window, &mut area).is_err() {
                continue;
            }
            let touches = (area.right - display.left).abs() < 40 || (area.left - display.right).abs() < 40;
            if docked_to.is_some() && touches && (area.top - display.top).abs() < 40 {
                beside.push(window);
            } else {
                floating.push(window);
            }
        }
    }
    (floating, beside)
}

/// Every popup of ours that's showing and isn't docked beside the console's display goes over every window, as a tooltip should:
/// a remote over the taskbar was found under a maximized window, having been put under it (the windows XAML shows popups in are
/// its to reuse, and the docked ones are made not to float). Soon after it's opened, and again a moment later: XAML shows a
/// popup's window a little after it's told to open.
pub(crate) fn float_popups() {
    std::thread::spawn(|| {
        for wait in [40, 250] {
            std::thread::sleep(Duration::from_millis(wait));
            crate::meters::on_ui(|| {
                use windows::Win32::UI::WindowsAndMessaging::{HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos};
                for window in our_popups().0 {
                    let _ = unsafe { SetWindowPos(window, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
                }
            });
        }
    });
}

/// Puts the remote and the tooltip that are beside the console's display where the display is among the windows on the
/// desktop: just over it, and under whatever's over it. They're windows of their own, and made to float over everything;
/// beside a window that's behind another, that's where they should be too. UI thread.
fn layer_with_display() {
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GW_HWNDPREV, GetWindow, HWND_NOTOPMOST, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos};
    unsafe {
        let Ok(display) = FindWindowW(windows_core::w!("PimpMyShellPimpbox"), windows_core::PCWSTR::null()) else { return };
        // Ours that are showing, and right beside it: the two that are docked to it.
        for window in our_popups().1 {
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
            let _ = SetWindowPos(window, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);
            // Under whatever's just over the display; or on top, if nothing is.
            let over = GetWindow(display, GW_HWNDPREV).ok().filter(|over| *over != window);
            let _ = SetWindowPos(window, Some(over.unwrap_or(HWND_TOP)), 0, 0, 0, 0, flags);
        }
    }
}

/// Puts a remote and the console's tooltip beside the console's display, which is at `place` (`display_place`): the remote
/// to its left, the tooltip to its right, their tops with its top. They're for the set showing `source`. None: the
/// display's been put away, and so are they.
pub(crate) fn dock(source: &str, place: Option<[i32; 9]>) {
    let r = (|| -> R<()> {
        // The display's just taken the keys and the mouse again (its count's gone up since it was last seen here): the remote
        // beside it flashes, as the display does. Not the first time it's seen, which says nothing of when. And whether it has
        // them now, for the card beside it to say.
        let taken = place.map_or(-1, |p| p[7]);
        let before = TAKEN_SEEN.replace(taken);
        let flash = before >= 0 && taken > before;
        HELD.set(place.is_some_and(|p| p[8] == 1));
        let Some([left, top, right, _, bar_left, bar_top, _, _, _]) = place else {
            DOCKED.with_borrow(|d| {
                if let Some(d) = d {
                    let _ = d.popup.SetIsOpen(false);
                    let _ = d.console.popup.SetIsOpen(false);
                }
            });
            return Ok(());
        };
        let root = crate::meters::root().ok_or("no taskbar to hang it from")?;
        if DOCKED.with_borrow(Option::is_none) {
            let built = build(&root, true)?;
            DOCKED.with_borrow_mut(|d| *d = Some(built));
        }
        // The taskbar's window counts in its own units from its own corner: the screen's pixels, scaled, from there.
        let scale = root.RasterizationScale().map_or(1.0, |s| s.max(0.5));
        let at = |x: i32, y: i32| (f64::from(x - bar_left) / scale, f64::from(y - bar_top) / scale);
        DOCKED.with_borrow_mut(|d| -> R<()> {
            let d = d.as_mut().ok_or("no remote")?;
            d.source = source.to_string();
            let child = d.popup.Child().map_err(w)?.cast::<pms_xaml::FrameworkElement>().map_err(w)?;
            child.Measure(Size { Width: f32::INFINITY, Height: f32::INFINITY }).map_err(w)?;
            let wide = f64::from(child.DesiredSize().map_err(w)?.Width);
            let ((x, y), (beyond, _)) = (at(left, top), at(right, top));
            d.popup.SetHorizontalOffset((x - wide - CONSOLE_GAP).floor()).map_err(w)?;
            d.popup.SetVerticalOffset(y.floor()).map_err(w)?;
            d.dock = Some((beyond + CONSOLE_GAP, y));
            d.popup.SetIsOpen(true).map_err(w)?;
            show_status(d, crate::screen::status(source).unwrap_or_default())
        })?;
        layer_with_display();
        if flash {
            flash_docked();
        }
        Ok(())
    })();
    if let Err(e) = r {
        crate::log(format!("remote beside the display: {e}"));
    }
}

/// The docked remote's title bar flashes twice, green, as a window's does to say it wants you: the display beside it has just
/// taken the keys and the mouse. On another thread, for the waits; each step on the UI thread.
fn flash_docked() {
    std::thread::spawn(|| {
        for (i, green) in [true, false, true, false].into_iter().enumerate() {
            if i > 0 {
                std::thread::sleep(FLASH);
            }
            crate::meters::on_ui(move || {
                DOCKED.with_borrow(|d| {
                    if let Some(d) = d {
                        let fill = if green { brush(FLASH_GREEN).and_then(|b| b.cast::<pms_xaml::Brush>().map_err(w)) } else { title_brush().and_then(|b| b.cast::<pms_xaml::Brush>().map_err(w)) };
                        if let Ok(fill) = fill {
                            let _ = d.bar.SetBackground(&fill);
                        }
                    }
                });
            });
        }
    });
}

fn show_status(r: &Remote, s: Status) -> R<()> {
    // A nuke key asking whether it's sure goes on asking, in the new words.
    if r.said.follow()? && ARMED.get() {
        r.nuke.SetText(&HSTRING::from(t::TV_SURE.t())).map_err(w)?;
    }
    let info = crate::screen::info(&r.source).unwrap_or_default();
    show_console(r, &s, &info)?;
    show_iss(r, &s, &info)?;
    let name = pms_screen::CHANNELS.iter().find(|c| c.0 == s.channel).map_or("", |c| c.1);
    for (number, line, words, pops) in &r.channels {
        let on = *number == s.channel && s.tv_on;
        line.SetBackground(&brush(if on { TITLE } else { LIST })?).map_err(w)?;
        words.SetForeground(&brush(if on { LIGHT } else { DARK })?).map_err(w)?;
        if let Some(sign) = pops {
            sign.SetForeground(&brush(if on { LIGHT } else { DARK })?).map_err(w)?;
        }
    }
    // While the nuke key's asking, what it would do stays up.
    let text = if ARMED.get() && info.lines().any(|l| l == "alone\t1") {
        t::GAME_GOES.t().to_string()
    } else if s.tv_on {
        format!("CH {:<4} {name}", s.channel)
    } else {
        t::DISPLAY_OFF.t().to_string()
    };
    r.display.SetText(&HSTRING::from(text)).map_err(w)?;
    r.title.SetText(&HSTRING::from(r.source.as_str())).map_err(w)?;
    set_led(&r.tv_led, s.tv_on)?;
    set_led(&r.shuffle_led, s.shuffle)?;
    set_led(&r.jitter_led, s.jitter)?;
    r.every.SetText(&HSTRING::from(info.lines().find_map(|l| l.strip_prefix("every\t")).unwrap_or_default())).map_err(w)?;
    for light in &r.lights {
        set_led(light, s.game_on && s.has_game)?;
    }
    Ok(())
}

/// The pointer's on the TV showing `source`, whose cell starts at `left` and `top` (taskbar-window DIPs) and is `width` wide.
pub(crate) fn over_tile(root: &XamlRoot, source: &str, left: f64, top: f64, width: f64) {
    ON_TILE.set(true);
    if CLICKED.get() {
        return;
    }
    // This comes with every move of the pointer, and it's the taskbar's own thread: once it's up, it's up.
    if REMOTE.with_borrow(|r| r.as_ref().is_some_and(|r| r.source == source && r.popup.IsOpen().unwrap_or(false))) || crate::remote_modern::is_up(source) {
        return;
    }
    // With a remote up for another set, the pointer's moving between them: this one opens at once. Otherwise it waits for the
    // pointer to rest, the way the taskbar's own previews do.
    let one_up = REMOTE.with_borrow(|r| r.as_ref().is_some_and(|r| r.popup.IsOpen().unwrap_or(false))) || crate::remote_modern::any_up();
    if !one_up {
        let new = WAITING.with_borrow(|w| w.as_ref().is_none_or(|w| w.1 != source));
        if new {
            WAITING.set(Some((root.clone(), source.to_string(), left, top, width)));
            let mine = RESTING.fetch_add(1, Ordering::Relaxed) + 1;
            std::thread::spawn(move || {
                std::thread::sleep(REST);
                crate::meters::on_ui(move || {
                    if RESTING.load(Ordering::Relaxed) == mine
                        && ON_TILE.get()
                        && let Some((root, source, left, top, width)) = WAITING.take()
                    {
                        open(&root, &source, left, top, width);
                    }
                });
            });
        }
        return;
    }
    open(root, source, left, top, width);
}

/// Opens the remote of the set showing `source`, whose cell starts at `left` and `top` and is `width` wide.
fn open(root: &XamlRoot, source: &str, left: f64, top: f64, width: f64) {
    WAITING.set(None);
    let r = (|| -> R<()> {
        // A newer set has a remote of its own, and only one's up at a time.
        if crate::screen::status(source).is_some_and(|s| s.modern) {
            close_classic();
            return crate::remote_modern::show(root, source, left, top, width);
        }
        crate::remote_modern::close();
        if REMOTE.with_borrow(Option::is_none) {
            let built = build(root, false)?;
            REMOTE.with_borrow_mut(|r| *r = Some(built));
        }
        REMOTE.with_borrow_mut(|r| -> R<()> {
            let r = r.as_mut().ok_or("no remote")?;
            r.source = source.to_string();
            place(&r.popup, root, left, top, width)?;
            let room = root.Size().map_or(f32::MAX, |s| s.Width) as f64;
            r.placed.set((r.popup.HorizontalOffset().map_err(w)?, top - 6.0, room));
            r.popup.SetIsOpen(true).map_err(w)?;
            show_status(r, crate::screen::status(source).unwrap_or_default())
        })?;
        float_popups();
        watch_pointer();
        keep_fresh();
        Ok(())
    })();
    if let Err(e) = r {
        crate::log(format!("remote: {e}"));
    }
}

/// The pointer's left the TV.
pub(crate) fn off_tile() {
    ON_TILE.set(false);
    CLICKED.set(false);
    WAITING.set(None);
    RESTING.fetch_add(1, Ordering::Relaxed);
    later();
}

/// Gone in a moment, unless the pointer's got somewhere it counts by then.
fn later() {
    std::thread::spawn(|| {
        std::thread::sleep(GRACE);
        crate::meters::on_ui(|| {
            if !ON_TILE.get() && !ON_REMOTE.get() {
                close();
            }
        });
    });
}

/// Looks at the set again now and then, for as long as the remote that's just opened stays up.
fn keep_fresh() {
    let mine = OPENED.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        while OPENED.load(Ordering::Relaxed) == mine {
            std::thread::sleep(REFRESH);
            crate::meters::on_ui(move || {
                if OPENED.load(Ordering::Relaxed) == mine {
                    refresh();
                }
            });
        }
    });
}

fn close_classic() {
    OPENED.fetch_add(1, Ordering::Relaxed);
    REMOTE.with_borrow(|r| {
        if let Some(r) = r {
            let _ = r.popup.SetIsOpen(false);
            let _ = r.console.popup.SetIsOpen(false);
            if let Some(card) = &r.iss {
                let _ = card.SetIsOpen(false);
            }
        }
    });
}

/// A TV's been clicked: whatever the click opens is what's wanted now, so its remote and the console's tooltip beside it go,
/// and stay gone until the pointer's left the TV. (The ones beside the console's display stay: they're its.)
pub(crate) fn clicked() {
    CLICKED.set(true);
    WAITING.set(None);
    RESTING.fetch_add(1, Ordering::Relaxed);
    close();
}

pub(crate) fn close() {
    WATCHING.fetch_add(1, Ordering::Relaxed);
    ON_REMOTE.set(false);
    calm_down();
    close_classic();
    crate::remote_modern::close();
}

/// For reset/detach.
pub(crate) fn off() {
    close();
    crate::remote_modern::off();
    for remote in [&REMOTE, &DOCKED] {
        if let Some(r) = remote.with_borrow_mut(Option::take) {
            let _ = r.popup.SetIsOpen(false);
            let _ = r.console.popup.SetIsOpen(false);
            crate::stage::unbuild_popup(&r.popup);
            crate::stage::unbuild_popup(&r.console.popup);
            if let Some(card) = &r.iss {
                let _ = card.SetIsOpen(false);
                crate::stage::unbuild_popup(card);
            }
        }
    }
}
