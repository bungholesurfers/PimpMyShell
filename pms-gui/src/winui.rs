//! The settings as a WinUI 3 window (windows-reactor), on a PC that has the Windows App Runtime: Windows 11 doesn't
//! ship it, but many PCs have it (Store apps bring it). PimpMyShell doesn't install it; without it, the same exe shows
//! the egui window instead (main.rs). Both edit the same model (model.rs) and say the same things (copy.rs).
//!
//! Its look is the Settings app's: a pane of pages with icons, and on each page cards, one per setting - its title
//! and what it does, the key the file keeps it under, its control, and Reset when it isn't the default.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pms::config::{self, Agg, AppGlow, Backdrop, Exprs, Hotmic, Look, Max, ShowWhen, StartGlow, Style, Toasts};
use pms::update::Status;
use windows_reactor::*;

use crate::copy;
use crate::dollars::Layer;
use crate::model::{self, Model, Section, Update};

/// The Windows App Runtime's framework package, at the version windows-reactor is built for or later.
const RUNTIME_FAMILY: &str = "Microsoft.WindowsAppRuntime.2_8wekyb3d8bbwe";
const RUNTIME_MIN: u64 = (2 << 48) | (4 << 32);

/// Whether the Windows App Runtime is installed for this user, asked quietly (windows-reactor would offer to
/// download it): what decides between this window and the egui one.
pub fn available() -> bool {
    use windows::Win32::Storage::Packaging::Appx::*;
    let version = PACKAGE_VERSION { Anonymous: PACKAGE_VERSION_0 { Version: RUNTIME_MIN } };
    let architectures = PackageDependencyProcessorArchitectures_X64 | PackageDependencyProcessorArchitectures_Neutral;
    let id = unsafe {
        TryCreatePackageDependency(
            windows::Win32::Security::PSID::default(),
            &windows_core::HSTRING::from(RUNTIME_FAMILY),
            version,
            architectures,
            PackageDependencyLifetimeKind_Process,
            None,
            CreatePackageDependencyOptions_None,
        )
    };
    let Ok(id) = id else { return false };
    unsafe {
        use windows::Win32::System::Memory::{GetProcessHeap, HEAP_FLAGS, HeapFree};
        if let Ok(heap) = GetProcessHeap() {
            let _ = HeapFree(heap, HEAP_FLAGS(0), Some(id.0 as _));
        }
    }
    true
}

/// Runs the window until it's closed.
pub fn run(path: PathBuf) -> Result<(), String> {
    App::run_component::<Settings>(path).map_err(|e| e.to_string())
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    General,
    Meters,
    Counters,
    Look,
    Hotmic,
    Toasts,
}

/// Each page: its tag, its icon (Segoe Fluent Icons), and its title.
const PAGES: [(Page, &str, &str, (&str, &str)); 6] = [
    (Page::General, "general", "\u{E713}", copy::GENERAL),
    (Page::Meters, "meters", "\u{E9D9}", copy::METERS),
    (Page::Counters, "counters", "\u{E8EF}", copy::COUNTERS),
    (Page::Look, "look", "\u{E771}", copy::LOOK),
    (Page::Hotmic, "hotmic", "\u{E720}", copy::HOTMIC),
    (Page::Toasts, "toasts", "\u{E91C}", copy::TOASTS),
];

/// How often the window looks in on its model: saves once edits settle, PimpMyShell's state, an update's progress.
const TICK: Duration = Duration::from_millis(300);

/// The logo, for the General page's header.
const LOGO: &[u8] = include_bytes!("../../assets/icon/pms-128.png");

pub struct Settings {
    m: Model,
    page: Page,
    meter: usize,
    pane_open: bool,
    pane_mode: NavigationViewDisplayMode,
    /// The section whose reset is waiting to be confirmed.
    confirm: Option<Section>,
    /// What floats behind the controls (dollars.rs), and where.
    dollars: Layer,
    panel: ElementRef<SwapChainPanel>,
}

pub enum Msg {
    Page(Option<String>),
    /// The pane opened or closed (its menu button).
    Pane(bool),
    /// The pane changed form, with the window's width.
    PaneMode(NavigationViewDisplayMode),
    /// The answer to whether to reset the section in `Settings::confirm`.
    Reset(ContentDialogResult),
    Tick,
    /// What a control did: a change to the settings.
    Edit(Box<dyn FnOnce(&mut Settings) + Send>),
}

type Cx = ViewContext<Settings>;

/// A control's callback that changes the settings with the control's new value.
fn edit<T: Send + 'static>(cx: &Cx, f: impl Fn(&mut Settings, T) + Send + Sync + 'static) -> Callback<T> {
    let f = Arc::new(f);
    cx.callback(move |v: T| {
        let f = f.clone();
        Msg::Edit(Box::new(move |s| f(s, v)))
    })
}

/// A button's callback that changes the settings.
fn act(cx: &Cx, f: impl Fn(&mut Settings) + Send + Sync + 'static) -> Callback<()> {
    edit(cx, move |s, ()| f(s))
}

fn tick_later(context: &ComponentContext<Settings>) {
    context.spawn_background(|_| {
        std::thread::sleep(TICK);
        Msg::Tick
    });
}

impl Component for Settings {
    type Input = PathBuf;
    type Message = Msg;

    fn create(path: &PathBuf, context: &ComponentContext<Self>) -> Self {
        tick_later(context);
        Settings { m: Model::open(path.clone()), page: Page::General, meter: 0, pane_open: true, pane_mode: NavigationViewDisplayMode::Expanded, confirm: None, dollars: Layer::default(), panel: ElementRef::new() }
    }

    fn update(&mut self, msg: Msg, context: &ComponentContext<Self>) {
        match msg {
            Msg::Page(tag) => {
                if let Some((page, ..)) = PAGES.iter().find(|(_, t, ..)| Some(*t) == tag.as_deref()) {
                    self.page = *page;
                }
            }
            Msg::Pane(open) => self.pane_open = open,
            Msg::PaneMode(mode) => self.pane_mode = mode,
            Msg::Reset(answer) => {
                if let Some(section) = self.confirm.take().filter(|_| answer == ContentDialogResult::Primary) {
                    self.m.reset(section);
                }
            }
            Msg::Tick => {
                if self.m.polled.elapsed() > Duration::from_secs(2) {
                    self.m.poll_daemon();
                }
                self.m.autosave();
                self.dollars.attach(&self.panel);
                tick_later(context);
            }
            Msg::Edit(f) => {
                f(self);
                self.m.autosave();
            }
        }
    }

    fn view(&self, _input: &PathBuf, cx: &mut Cx) -> View {
        cx.window_title("PimpMyShell");
        // Only an expanded pane takes room of its own when it's open: a compact one that opens goes over the settings.
        self.dollars.set_pane(match (self.pane_mode, self.pane_open) {
            (NavigationViewDisplayMode::Minimal, _) => 0,
            (NavigationViewDisplayMode::Expanded, true) => 240,
            _ => 48,
        });
        let visuals = WindowVisuals::new().backdrop(WindowBackdrop::Acrylic).client_size(1080.0, 780.0);
        cx.window_visuals(match icon() {
            Some(icon) => visuals.icon(icon),
            None => visuals,
        });
        let items = PAGES.iter().map(|(page, tag, glyph, (name, _))| {
            let content: View = if *page == Page::General {
                // The first stop, with the one thing to know at a glance: whether PimpMyShell is running.
                StackPanel::new().margin(Thickness::xy(0.0, 4.0)).children((TextBlock::new().text(*name).font_weight(FontWeight::SEMI_BOLD), state(self.m.daemon_running).font_size(12.0)))
            } else {
                TextBlock::new().text(*name).into()
            };
            let item = NavigationViewItem::new().tag(*tag).is_selected(self.page == *page);
            KeyedView::new(*tag, item.slots([SlotView::new(NavigationViewItemSlot::Icon, FontIcon::new().glyph(*glyph)), SlotView::new(NavigationViewItemSlot::Content, content)]))
        });
        let page: View = match self.page {
            Page::General => self.general(cx),
            Page::Meters => self.meters(cx),
            Page::Counters => self.counters(cx),
            Page::Look => self.look(cx),
            Page::Hotmic => self.hotmic(cx),
            Page::Toasts => self.toasts(cx),
        };
        let (text, error) = &self.m.message;
        let status = TextBlock::new()
            .text(text.clone())
            .text_trimming(TextTrimming::CharacterEllipsis)
            .font_size(12.0)
            .foreground(if *error { Brush::Theme(ThemeBrush::SystemCritical) } else { Brush::Theme(ThemeBrush::PrimaryText) })
            .opacity(if *error { 1.0 } else { 0.6 })
            .margin(Thickness::new(36.0, 6.0, 36.0, 10.0))
            .grid_row(1);
        // A scroll area per page (keyed by it), so that each page opens at its top; Meters has its own.
        let tag = PAGES.iter().find(|(p, ..)| *p == self.page).map_or("general", |(_, t, ..)| *t);
        let content: View = if self.page == Page::Meters {
            page
        } else {
            ScrollViewer::new().content(StackPanel::new().max_width(1000.0).margin(Thickness::new(36.0, 24.0, 36.0, 36.0)).children([page]))
        };
        let body = Grid::new().rows([GridLength::STAR, GridLength::Auto]).keyed_children([KeyedView::new(tag, Border::new().grid_row(0).content(content)), KeyedView::new("status", status)]);
        // The small print, where nobody can claim not to have seen it; there's no room for it in a narrow pane.
        let footer: View = if self.pane_open && self.pane_mode == NavigationViewDisplayMode::Expanded {
            ScrollViewer::new().height(300.0).vertical_scroll_bar_visibility(ScrollBarVisibility::Auto).content(TextBlock::new().text(copy::LEGAL).font_size(9.0).opacity(0.55).text_wrapping(TextWrapping::Wrap).margin(Thickness::new(14.0, 6.0, 12.0, 8.0)))
        } else {
            Border::new().into()
        };
        let window = NavigationView::new()
            .pane_display_mode(NavigationViewPaneDisplayMode::Auto)
            .is_settings_visible(false)
            .is_back_button_visible(NavigationViewBackButtonVisible::Collapsed)
            .pane_title("PimpMyShell")
            .open_pane_length(240.0)
            .is_pane_open(self.pane_open)
            .on_is_pane_open_changed(cx.callback(Msg::Pane))
            .on_display_mode_changed(cx.callback(Msg::PaneMode))
            .on_selected_tag_changed(cx.callback(Msg::Page))
            .slots([SlotView::collection(NavigationViewSlot::MenuItems, items), SlotView::new(NavigationViewSlot::Content, body), SlotView::new(NavigationViewSlot::PaneFooter, footer)]);
        // Asking before a section goes back to its defaults.
        let question = self.confirm.unwrap_or(Section::Taskbar);
        let confirm = ContentDialog::new()
            .is_open(self.confirm.is_some())
            .title(format!("Reset {}?", question.name()))
            .primary_button_text("Reset")
            .close_button_text("Cancel")
            .on_closed(cx.callback(Msg::Reset))
            .content(TextBlock::new().text(self.m.warning(question)).text_wrapping(TextWrapping::Wrap));
        // The dollars first: over the acrylic, under everything else.
        Grid::new().children((SwapChainPanel::new().element_ref(&self.panel), window, confirm))
    }
}

// ---------------------------------------------------------------------------------------------
// Building blocks: page headings, groups, and a card per setting.

const GREEN: Color = Color::rgb(0x6C, 0xCB, 0x5F);
const AMBER: Color = Color::rgb(0xFC, 0xB9, 0x00);

/// Whether PimpMyShell is running, as a colored line.
fn state(running: bool) -> TextBlock {
    let (text, color) = if running { ("● Running", GREEN) } else { ("● Not running", AMBER) };
    TextBlock::new().text(text).foreground(Brush::Solid(color))
}

fn heading((title, help): (&str, &str)) -> View {
    let mut parts = vec![TextBlock::new().text(title).font_size(28.0).font_weight(FontWeight::SEMI_BOLD).into()];
    if !help.is_empty() {
        parts.push(TextBlock::new().text(help).text_wrapping(TextWrapping::Wrap).opacity(0.75).max_width(760.0).horizontal_alignment(HorizontalAlignment::Left).into());
    }
    children(StackPanel::new().spacing(6.0).margin(Thickness::new(0.0, 0.0, 0.0, 8.0)), parts)
}

/// A group of cards under a heading.
/// The button that puts a section back, once it's been asked; greyed out while it's as it ships.
fn reset_section(cx: &Cx, section: Section, at_default: bool, margin: Thickness) -> View {
    Button::new()
        .grid_column(1)
        .margin(margin)
        .vertical_alignment(VerticalAlignment::Top)
        .is_enabled(!at_default)
        .on_click(act(cx, move |s| s.confirm = Some(section)))
        .content(TextBlock::new().text("Reset").font_size(12.0))
}

/// A group of cards under a heading, with a way back to the defaults.
fn group_reset(cx: &Cx, title: &str, section: Section, at_default: bool, cards: Vec<View>) -> View {
    let head = Grid::new().columns([GridLength::STAR, GridLength::Auto]).margin(Thickness::new(1.0, 24.0, 0.0, 6.0)).children((
        TextBlock::new().grid_column(0).text(title).font_weight(FontWeight::SEMI_BOLD).vertical_alignment(VerticalAlignment::Center),
        reset_section(cx, section, at_default, Thickness::uniform(0.0)),
    ));
    let mut all: Vec<View> = vec![head];
    all.extend(cards);
    children(StackPanel::new().spacing(4.0), all)
}

/// A page's heading, for a page that's a section of its own.
fn heading_reset(cx: &Cx, text: (&str, &str), section: Section, at_default: bool) -> View {
    Grid::new().columns([GridLength::STAR, GridLength::Auto]).children((
        Border::new().grid_column(0).content(heading(text)),
        reset_section(cx, section, at_default, Thickness::new(0.0, 8.0, 0.0, 0.0)),
    ))
}

fn group(title: &str, cards: Vec<View>) -> View {
    let mut all = vec![TextBlock::new().text(title).font_weight(FontWeight::SEMI_BOLD).margin(Thickness::new(1.0, 24.0, 0.0, 6.0)).into()];
    all.extend(cards);
    children(StackPanel::new().spacing(4.0), all)
}

fn card(content: impl Into<View>) -> View {
    Border::new()
        .padding(Thickness::xy(16.0, 14.0))
        .min_height(68.0)
        .corner_radius(CornerRadius::uniform(4.0))
        .background(Brush::Theme(ThemeBrush::CardBackground))
        .border_brush(Brush::Theme(ThemeBrush::CardStroke))
        .border_thickness(Thickness::uniform(1.0))
        .content(content)
}

/// The panel a card's text sits in.
fn text_panel() -> StackPanel {
    StackPanel::new().spacing(2.0).vertical_alignment(VerticalAlignment::Center)
}
/// A card's lines: its title over what it does, and (for a setting) the key the file keeps it under.
fn card_lines(title: &str, help: &str, key: Option<&str>) -> Vec<View> {
    let mut lines = vec![TextBlock::new().text(title).text_wrapping(TextWrapping::Wrap).into()];
    if !help.is_empty() {
        lines.push(TextBlock::new().text(help).text_wrapping(TextWrapping::Wrap).font_size(12.0).opacity(0.7).into());
    }
    if let Some(key) = key {
        lines.push(TextBlock::new().text(key).font_size(11.0).opacity(0.45).into());
    }
    lines
}

/// A card: an optional icon, its text, and its controls on the right.
fn settings_card(glyph: Option<&str>, title: &str, help: &str, key: Option<&str>, controls: impl Into<View>) -> View {
    let lines = card_lines(title, help, key);
    let Some(glyph) = glyph else {
        return card(Grid::new().columns([GridLength::STAR, GridLength::Auto]).column_spacing(16.0).children((
            children(text_panel().grid_column(0), lines),
            Border::new().grid_column(1).vertical_alignment(VerticalAlignment::Center).content(controls),
        )));
    };
    card(Grid::new().columns([GridLength::Auto, GridLength::STAR, GridLength::Auto]).column_spacing(16.0).children((
        FontIcon::new().glyph(glyph).grid_column(0).vertical_alignment(VerticalAlignment::Center).margin(Thickness::new(4.0, 0.0, 4.0, 0.0)),
        children(text_panel().grid_column(1), lines),
        Border::new().grid_column(2).vertical_alignment(VerticalAlignment::Center).content(controls),
    )))
}

/// The Reset button of a setting that isn't the default.
/// The icon, and the padding a button keeps around it.
const RESET_W: f64 = 46.0;

fn reset_button(changed: bool, reset: Option<Callback<()>>) -> Option<View> {
    let reset = reset.filter(|_| changed)?;
    Some(Button::new().width(RESET_W).on_click(reset).content(SymbolIcon::new().symbol(Symbol::Undo)).tooltip("Reset to default"))
}

/// Where Reset goes, whether it shows or not: so that controls line up from card to card.
fn reset_slot(changed: bool, reset: Option<Callback<()>>) -> View {
    reset_button(changed, reset).unwrap_or_else(|| Border::new().width(RESET_W).into())
}

/// A setting: its title, description and key (copy.rs), its control, and Reset when it isn't the default.
fn row(id: &str, changed: bool, reset: Option<Callback<()>>, control: impl Into<View>) -> View {
    let (title, help) = copy::setting(id);
    let controls = StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((Border::new().vertical_alignment(VerticalAlignment::Center).content(control), reset_slot(changed, reset)));
    settings_card(None, title, help, Some(copy::key(id)), controls)
}

/// A setting whose control is too wide to sit beside its text (a list, say): the control goes under it.
fn wide_row(id: &str, changed: bool, reset: Option<Callback<()>>, control: impl Into<View>) -> View {
    let (title, help) = copy::setting(id);
    let text = children(text_panel().grid_column(0), card_lines(title, help, Some(copy::key(id))));
    card(StackPanel::new().spacing(12.0).children((
        Grid::new().columns([GridLength::STAR, GridLength::Auto]).column_spacing(16.0).children((text, Border::new().grid_column(1).vertical_alignment(VerticalAlignment::Top).content(reset_slot(changed, reset)))),
        control.into(),
    )))
}

/// A table that's in the file or not (`[hotmic]`, say): a card with its icon and switch.
fn switch_card(glyph: &str, title: &str, help: &str, on: bool, set: Callback<bool>) -> View {
    settings_card(Some(glyph), title, help, None, switch(on, set))
}

// ---------------------------------------------------------------------------------------------
// Controls.

/// A switch as the Settings app has it: its On or Off to the left, so that switches line up on the right.
fn switch(on: bool, set: Callback<bool>) -> View {
    let toggle = ToggleSwitch::new()
        .is_on(on)
        .min_width(0.0)
        .on_toggled(set)
        .slots([SlotView::new(ToggleSwitchSlot::OnContent, TextBlock::new()), SlotView::new(ToggleSwitchSlot::OffContent, TextBlock::new())]);
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(12.0)
        .children((TextBlock::new().text(if on { "On" } else { "Off" }).vertical_alignment(VerticalAlignment::Center), Border::new().width(44.0).content(toggle)))
}

fn toggle(cx: &Cx, value: bool, set: impl Fn(&mut Settings, bool) + Send + Sync + 'static) -> View {
    switch(value, edit(cx, set))
}

/// One of `options` (the file's value, what the window calls it).
fn choice(cx: &Cx, value: &str, options: &'static [(&'static str, &'static str)], set: impl Fn(&mut Settings, String) + Send + Sync + 'static) -> View {
    let selected = options.iter().position(|(o, _)| *o == value);
    ComboBox::new()
        .items_source(options.iter().map(|(_, label)| label.to_string()))
        .selected_index(selected)
        .min_width(200.0)
        .on_selection_changed(edit(cx, move |s, i: Option<usize>| {
            if let Some((o, _)) = i.and_then(|i| options.get(i)) {
                set(s, o.to_string());
            }
        }))
        .into()
}

/// A number, and its unit in a slot of its own (empty for none), so that number boxes line up.
fn number(cx: &Cx, value: f64, min: f64, max: f64, suffix: &str, set: impl Fn(&mut Settings, f64) + Send + Sync + 'static) -> View {
    let boxed = NumberBox::new().minimum(min).maximum(max).value(value).width(120.0).on_value_changed(edit(cx, move |s, v: Option<f64>| {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            set(s, v.clamp(min, max));
        }
    }));
    StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((boxed, TextBlock::new().text(suffix).width(28.0).vertical_alignment(VerticalAlignment::Center).opacity(0.7)))
}

fn text(cx: &Cx, value: &str, hint: &str, width: f64, set: impl Fn(&mut Settings, String) + Send + Sync + 'static) -> View {
    TextBox::new().text(value).placeholder_text(hint).width(width).horizontal_alignment(HorizontalAlignment::Left).on_text_changed(edit(cx, set)).into()
}

/// A 0..`full` level, as a slider and its percentage (the egui window's knob).
fn level(cx: &Cx, value: f32, full: f32, set: impl Fn(&mut Settings, f32) + Send + Sync + 'static) -> View {
    let slider = Slider::new()
        .minimum(0.0)
        .maximum(f64::from(full))
        .step_frequency(f64::from(full) / 100.0)
        .value(f64::from(value))
        .width(200.0)
        .on_value_changed(edit(cx, move |s, v: f64| set(s, v as f32)));
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(12.0)
        .children((slider, TextBlock::new().text(format!("{:.0}%", value / full * 100.0)).width(40.0).vertical_alignment(VerticalAlignment::Center)))
}

fn parse_color(s: &str) -> Color {
    let hex = s.trim().trim_start_matches('#');
    let n = u32::from_str_radix(hex, 16).unwrap_or(0xFFFFFF);
    match hex.len() {
        8 => Color::argb((n >> 24) as u8, (n >> 16) as u8, (n >> 8) as u8, n as u8),
        _ => Color::rgb((n >> 16) as u8, (n >> 8) as u8, n as u8),
    }
}

fn format_color(c: Color) -> String {
    if c.a == 255 { format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b) } else { format!("#{:02X}{:02X}{:02X}{:02X}", c.a, c.r, c.g, c.b) }
}

/// A swatch that opens a color picker, and the hex the file keeps (`#RRGGBB` / `#AARRGGBB`).
fn color(cx: &Cx, value: &str, set: impl Fn(&mut Settings, String) + Send + Sync + 'static) -> View {
    let set = Arc::new(set);
    let picked = set.clone();
    let picker = ColorPicker::new()
        .color(parse_color(value))
        .is_alpha_enabled(true)
        .is_hex_input_visible(true)
        .on_color_changed(edit(cx, move |s, c: Color| picked(s, format_color(c))));
    let swatch = Border::new()
        .width(24.0)
        .height(24.0)
        .corner_radius(CornerRadius::uniform(4.0))
        .background(Brush::Solid(parse_color(value)))
        .border_brush(Brush::Theme(ThemeBrush::CardStroke))
        .border_thickness(Thickness::uniform(1.0));
    let button = Button::new().content(swatch).flyout_with(Flyout::rich(picker));
    StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((button, TextBox::new().text(value).width(110.0).on_text_changed(edit(cx, move |s, v: String| set(s, v)))))
}

/// Where a list of strings lives, for the list editor.
#[derive(Clone, Copy)]
enum ListAt {
    MeterNames(usize),
    MeterExprs(usize),
    MeterColors(usize),
    HotmicIgnore,
    ToastsIgnore,
}

/// Changes the list at `at`.
fn with_list(s: &mut Settings, at: ListAt, f: impl FnOnce(&mut Vec<String>)) {
    let c = &mut s.m.config;
    match at {
        ListAt::MeterNames(i) => {
            if let Some(m) = c.meters.get_mut(i) {
                f(&mut m.names);
            }
        }
        ListAt::MeterColors(i) => {
            if let Some(m) = c.meters.get_mut(i) {
                f(&mut m.colors);
            }
        }
        ListAt::MeterExprs(i) => {
            if let Some(m) = c.meters.get_mut(i) {
                let mut list = m.expr.as_ref().map(Exprs::list).unwrap_or_default();
                f(&mut list);
                m.expr = match list.len() {
                    0 => None,
                    1 => Some(Exprs::One(list.remove(0))),
                    _ => Some(Exprs::Many(list)),
                };
            }
        }
        ListAt::HotmicIgnore => {
            if let Some(h) = c.hotmic.as_mut() {
                f(&mut h.ignore);
            }
        }
        ListAt::ToastsIgnore => {
            if let Some(t) = c.toasts.as_mut() {
                f(&mut t.ignore);
            }
        }
    }
}

/// A list of strings (or colors): one line each with Remove, and Add.
fn list(cx: &Cx, items: &[String], at: ListAt, hint: &str, colors: bool) -> View {
    let mut lines: Vec<View> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let set = move |s: &mut Settings, v: String| {
                with_list(s, at, |l| {
                    if let Some(x) = l.get_mut(i) {
                        *x = v;
                    }
                })
            };
            let field = if colors { color(cx, item, set) } else { text(cx, item, hint, 320.0, set) };
            let remove = Button::new()
                .on_click(act(cx, move |s| {
                    with_list(s, at, |l| {
                        if i < l.len() {
                            l.remove(i);
                        }
                    })
                }))
                .content(SymbolIcon::new().symbol(Symbol::Delete))
                .tooltip("Remove");
            StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((field, remove))
        })
        .collect();
    let new = if colors { "#4FC3F7" } else { "" };
    lines.push(
        Button::new()
            .on_click(act(cx, move |s| with_list(s, at, |l| l.push(new.into()))))
            .content(StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((SymbolIcon::new().symbol(Symbol::Add), TextBlock::new().text("Add")))),
    );
    children(StackPanel::new().spacing(8.0), lines)
}

/// A color per part meter `i` draws (`parts`, by name). No Add or Remove: how many there are is the meter's.
fn color_slots(cx: &Cx, i: usize, parts: &[String], colors: &[String]) -> View {
    const NEW: &str = "#4FC3F7";
    let lines = parts
        .iter()
        .enumerate()
        .map(|(k, part)| {
            let value = colors.get(k).cloned().unwrap_or_else(|| NEW.into());
            let set = move |s: &mut Settings, v: String| {
                with_list(s, ListAt::MeterColors(i), |l| {
                    if l.len() <= k {
                        l.resize(k + 1, NEW.into());
                    }
                    l[k] = v;
                })
            };
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(12.0)
                .children((TextBlock::new().text(part.clone()).width(150.0).text_trimming(TextTrimming::CharacterEllipsis).vertical_alignment(VerticalAlignment::Center), color(cx, &value, set)))
        })
        .collect();
    children(StackPanel::new().spacing(8.0), lines)
}

/// `max` / `show_above`: none, a number, or an expression.
fn max_editor(cx: &Cx, value: &Option<Max>, set: impl Fn(&mut Settings, Option<Max>) + Send + Sync + 'static) -> View {
    let set = Arc::new(set);
    let kind = match value {
        None => 0,
        Some(Max::Number(_)) => 1,
        Some(Max::Expr(_)) => 2,
    };
    let to_kind = set.clone();
    let kinds = ComboBox::new().items_source(["None", "Number", "Expression"]).selected_index(kind).width(150.0).on_selection_changed(edit(cx, move |s, i: Option<usize>| match i {
        Some(1) if kind != 1 => to_kind(s, Some(Max::Number(100.0))),
        Some(2) if kind != 2 => to_kind(s, Some(Max::Expr(String::new()))),
        Some(0) if kind != 0 => to_kind(s, None),
        _ => {}
    }));
    let field: View = match value {
        Some(Max::Number(n)) => number(cx, *n, -1e15, 1e15, "", move |s, v| set(s, Some(Max::Number(v)))),
        Some(Max::Expr(e)) => text(cx, e, "For example, ram_total", 240.0, move |s, v| set(s, Some(Max::Expr(v)))),
        None => TextBlock::new().into(),
    };
    StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((kinds, field))
}

/// The meter being edited, for a setting's closure.
fn meter_mut(s: &mut Settings, i: usize) -> Option<&mut pms::config::MeterSpec> {
    s.m.config.meters.get_mut(i)
}

/// An optional setting of meter `i`: shows what's in effect (`default`, the code's, when unset). It's "changed" when it
/// differs from what the shipped meter of the same name has (`$base`, see `Model::baseline_meter`), and Reset puts
/// that back. Set to the code's default, it's left out of the file again.
macro_rules! meter_opt {
    ($cx:expr, $meters:expr, $base:expr, $i:expr, $field:ident, $default:expr, $id:expr, |$v:ident, $set:ident| $control:expr) => {{
        let i = $i;
        let default = $default;
        let current = $meters.get(i).and_then(|m| m.$field.clone());
        let shipped = $base.and_then(|b| b.$field.clone()).unwrap_or_else(|| default.clone());
        let $v = current.clone().unwrap_or_else(|| default.clone());
        let changed = $v != shipped;
        let d = default.clone();
        let $set = move |s: &mut Settings, v| {
            if let Some(m) = meter_mut(s, i) {
                m.$field = (v != d).then_some(v);
            }
        };
        let (d, to) = (default.clone(), shipped.clone());
        let reset = act($cx, move |s| {
            if let Some(m) = meter_mut(s, i) {
                m.$field = (to != d).then(|| to.clone());
            }
        });
        row($id, changed, Some(reset), $control)
    }};
}

// ---------------------------------------------------------------------------------------------
// Pages.

impl Settings {
    fn general(&self, cx: &Cx) -> View {
        let m = &self.m;
        let c = &m.config;
        // What ships, which Reset puts back.
        let b = &m.baseline;
        macro_rules! top {
            ($field:ident) => {{
                let to = b.$field.clone();
                Some(act(cx, move |s| s.m.config.$field = to.clone()))
            }};
        }
        let running = m.daemon_running;
        // The header: the logo, the version, whether PimpMyShell is running, and Start or Stop.
        let start_stop: View = if running {
            Button::new().on_click(act(cx, |s| s.m.stop_daemon())).content("Stop").tooltip("Stop PimpMyShell and restore the standard taskbar")
        } else {
            Button::new().style(ButtonStyle::Accent).on_click(act(cx, |s| s.m.start_daemon())).content("Start")
        };
        let header = card(Grid::new().columns([GridLength::Auto, GridLength::STAR, GridLength::Auto]).column_spacing(20.0).children((
            Image::new().grid_column(0).source_data(EncodedImage::new(LOGO)).width(64.0).height(64.0),
            StackPanel::new().grid_column(1).spacing(2.0).vertical_alignment(VerticalAlignment::Center).children((
                TextBlock::new().text("PimpMyShell").font_size(20.0).font_weight(FontWeight::SEMI_BOLD),
                TextBlock::new().text(format!("Version {}", pms::version::CURRENT)).font_size(12.0).opacity(0.7),
                state(running).margin(Thickness::new(0.0, 4.0, 0.0, 0.0)),
            )),
            Border::new().grid_column(2).vertical_alignment(VerticalAlignment::Center).content(start_stop),
        )));
        let startup = group(
            "Startup",
            vec![
                settings_card(Some("\u{E7E8}"), "Start when I sign in", "Run PimpMyShell automatically when you sign in to Windows", None, switch(m.autostarts(), edit(cx, |s, on: bool| s.m.set_autostart(on)))),
                settings_card(Some("\u{E81C}"), "Last log entry", &m.last_log, None, TextBlock::new()),
                TextBlock::new().text(copy::LOG_PRIVACY).text_wrapping(TextWrapping::Wrap).font_size(12.0).opacity(0.7).margin(Thickness::new(1.0, 2.0, 0.0, 0.0)).into(),
            ],
        );
        let taskbar = group_reset(
            cx,
            "Taskbar",
            Section::Taskbar,
            m.at_default(Section::Taskbar),
            vec![
                row("theme", c.theme != b.theme, top!(theme), choice(cx, &c.theme, copy::THEMES, |s, v| s.m.config.theme = v)),
                row("meter_height", c.meter_height != b.meter_height, top!(meter_height), choice(cx, &c.meter_height, copy::HEIGHTS, |s, v| s.m.config.meter_height = v)),
                row("bar_height", c.bar_height != b.bar_height, top!(bar_height), choice(cx, &c.bar_height, copy::HEIGHTS, |s, v| s.m.config.bar_height = v)),
                row("interval_ms", c.interval_ms != b.interval_ms, top!(interval_ms), number(cx, c.interval_ms as f64, 16.0, 60_000.0, "ms", |s, v| s.m.config.interval_ms = v as u64)),
                row("value_font", c.value_font != b.value_font, top!(value_font), text(cx, c.value_font.as_deref().unwrap_or(""), "Cascadia Mono", 220.0, |s, v| s.m.config.value_font = Some(v).filter(|v| !v.is_empty()))),
            ],
        );
        let advanced = group_reset(
            cx,
            "Performance",
            Section::Performance,
            m.at_default(Section::Performance),
            vec![
                row(
                    "max_fps",
                    c.max_fps != b.max_fps,
                    top!(max_fps),
                    StackPanel::new().orientation(Orientation::Horizontal).spacing(12.0).children((
                        match c.max_fps {
                            Some(f) => number(cx, f64::from(f), 0.0, 240.0, "fps", |s, v| s.m.config.max_fps = Some(v as u32)),
                            None => TextBlock::new().into(),
                        },
                        toggle(cx, c.max_fps.is_some(), |s, on| s.m.config.max_fps = on.then_some(30)),
                    )),
                ),
                row(
                    "battery_fps",
                    c.battery_fps != b.battery_fps,
                    top!(battery_fps),
                    number(cx, f64::from(c.battery_fps.unwrap_or(30)), 0.0, 240.0, "fps", |s, v| {
                        let v = v as u32;
                        s.m.config.battery_fps = (v != 30).then_some(v);
                    }),
                ),
                row("software_gpus", c.software_gpus != b.software_gpus, top!(software_gpus), toggle(cx, c.software_gpus, |s, v| s.m.config.software_gpus = v)),
            ],
        );
        let window = group(
            "Settings",
            vec![
                settings_card(Some("\u{E8A5}"), "Settings file", &m.path.display().to_string(), None, Button::new().on_click(act(cx, |s| s.m.show_config())).content("Show in File Explorer")),
                settings_card(
                    Some("\u{E789}"),
                    "Welcome",
                    "The first-run questions, again",
                    None,
                    Button::new().on_click(act(cx, |s| s.m.open_welcome())).content("Open"),
                ),
                settings_card(
                    Some("\u{E8A7}"),
                    "Classic settings window",
                    "The simpler window PCs without the Windows App Runtime use. It changes the same settings",
                    None,
                    Button::new().on_click(act(cx, |s| s.m.open_classic())).content("Open"),
                ),
            ],
        );
        let legal = group("Legal", vec![card(TextBlock::new().text(copy::LEGAL).text_wrapping(TextWrapping::Wrap).font_size(11.0).opacity(0.7))]);
        StackPanel::new().spacing(4.0).children((heading(copy::GENERAL), header, startup, self.software_update(cx), taskbar, advanced, window, legal))
    }

    fn software_update(&self, cx: &Cx) -> View {
        let c = &self.m.config;
        let latest = c.latest_channel();
        let on = c.update_check != Some(false);
        let state = self.m.update.lock().unwrap().clone();
        let installed = pms::update::installed();
        let busy = self.m.updating();
        // What the last check found, and what can be done about it.
        let bar = |title: String, message: String, severity: InfoBarSeverity| -> View { InfoBar::new().title(title).message(message).severity(severity).is_open(true).is_closable(false).into() };
        let mut found: Vec<View> = Vec::new();
        match state {
            Update::Idle if !installed => found.push(bar("This is a portable copy".into(), "It can check for updates, but you'll need to install them yourself.".into(), InfoBarSeverity::Informational)),
            Update::Idle => {}
            Update::Checking => found.push(StackPanel::new().spacing(6.0).children((TextBlock::new().text("Checking for updates…"), ProgressBar::new().is_indeterminate(true).width(320.0).horizontal_alignment(HorizontalAlignment::Left)))),
            Update::Downloading(done, total) => found.push(StackPanel::new().spacing(6.0).children((
                TextBlock::new().text(format!("Downloading… {} of {} KB", done / 1024, total / 1024)),
                ProgressBar::new().minimum(0.0).maximum(total.max(1) as f64).value(done as f64).is_indeterminate(false).width(320.0).horizontal_alignment(HorizontalAlignment::Left),
            ))),
            Update::Done(Status::UpToDate { stable }) => {
                let which = if latest { "The latest release" } else { "The latest stable version" };
                found.push(bar("You're up to date".into(), format!("{which} is {stable}."), InfoBarSeverity::Success));
            }
            Update::Done(Status::Available { version }) => found.push(StackPanel::new().spacing(8.0).children((
                bar(format!("PimpMyShell {version} is available"), "This copy can't install updates. You can get it from its release page.".into(), InfoBarSeverity::Informational),
                Button::new().on_click(act(cx, move |_| model::open_page(&pms::update::release_page(&version)))).content("Go to the release page"),
            ))),
            Update::Done(Status::Stuck { version }) => found.push(StackPanel::new().spacing(8.0).children((
                bar(format!("PimpMyShell {version} didn't install"), "It was installed, but you still have the earlier version. We won't install it again automatically.".into(), InfoBarSeverity::Warning),
                Button::new().on_click(act(cx, |_| model::open_page(pms::update::RELEASES_PAGE))).content("Go to the releases page"),
            ))),
            Update::Done(Status::Ready { version, path, sha256 }) => {
                let note = if version == pms::version::CURRENT {
                    " It's the version you have now.".to_string()
                } else if pms::version::newer(pms::version::CURRENT, &version) {
                    format!(" It's older than the version you have ({}).", pms::version::CURRENT)
                } else {
                    String::new()
                };
                let v = version.clone();
                found.push(StackPanel::new().spacing(8.0).children((
                    bar(format!("PimpMyShell {version} is ready to install"), format!("It's downloaded and verified.{note}"), InfoBarSeverity::Success),
                    StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
                        Button::new()
                            .style(ButtonStyle::Accent)
                            .on_click(act(cx, move |s| s.m.install(&path, &v, &sha256)))
                            .content("Install and restart")
                            .tooltip("The taskbar returns to normal for a moment while PimpMyShell installs, then PimpMyShell starts again"),
                        Button::new().on_click(act(cx, |s| *s.m.update.lock().unwrap() = Update::Idle)).content("Not now"),
                    )),
                )));
            }
            Update::Failed(e) => found.push(bar("We couldn't check for updates".into(), e, InfoBarSeverity::Error)),
        }
        let check = Button::new()
            .style(ButtonStyle::Accent)
            .is_enabled(!busy)
            .on_click(act(cx, move |s| s.m.start_check(pms::update::Ask { install: installed, automatic: false, force: false, latest })))
            .content("Check for updates");
        let mut cards = vec![
            settings_card(Some("\u{E895}"), &format!("Version {}", pms::version::CURRENT), &format!("Checks start at {}", pms::update::first_url(latest)), None, check),
        ];
        if !found.is_empty() {
            cards.push(card(children(StackPanel::new().spacing(8.0), found)));
        }
        let (title, help) = copy::setting("update_check");
        let help = if c.update_check.is_none() { format!("{help}. You haven't chosen yet") } else { help.to_string() };
        cards.push(settings_card(None, title, &help, Some("update_check"), {
            let mut controls = vec![toggle(cx, on, |s, v| s.m.config.update_check = Some(v))];
            controls.extend(reset_button(!on, Some(act(cx, |s| s.m.config.update_check = Some(true)))));
            children(StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0), controls)
        }));
        cards.push(row("update_channel", c.update_channel != self.m.baseline.update_channel, {
            let to = self.m.baseline.update_channel.clone();
            Some(act(cx, move |s| s.m.config.update_channel = to.clone()))
        }, choice(cx, &c.update_channel, copy::CHANNELS, |s, v| s.m.config.update_channel = v)));
        let reinstall = |label: &str, latest: bool| {
            Button::new()
                .is_enabled(!busy && installed)
                .on_click(act(cx, move |s| s.m.start_check(pms::update::Ask { install: true, automatic: false, force: true, latest })))
                .content(label)
                .tooltip(if installed { "Download this version, even if it's the same as or older than yours, and choose whether to install it" } else { "Only an installed copy can install a version" })
        };
        cards.push(settings_card(
            None,
            "Reinstall",
            "Repair PimpMyShell, or go back to the stable version",
            None,
            StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((reinstall("Stable version", false), reinstall("Latest version", true))),
        ));
        group_reset(cx, "Software update", Section::Updates, self.m.at_default(Section::Updates), cards)
    }

    fn meters(&self, cx: &Cx) -> View {
        let meters = &self.m.config.meters;
        let selected = self.meter.min(meters.len().saturating_sub(1));
        // The list: a swatch, the name, the style; a gap between groups.
        let mut list: Vec<View> = Vec::new();
        let mut group_of: Option<&str> = None;
        for (i, m) in meters.iter().enumerate() {
            let swatch = m.color.as_ref().or(m.colors.first()).map(|c| parse_color(c)).unwrap_or(Color::rgb(0x80, 0x80, 0x80));
            let style = copy::STYLES.iter().find(|(s, _)| *s == format!("{:?}", m.style).to_lowercase()).map_or("", |(_, l)| *l);
            let line = Grid::new().columns([GridLength::Pixel(6.0), GridLength::STAR, GridLength::Auto]).column_spacing(12.0).children((
                Border::new().grid_column(0).width(4.0).height(22.0).corner_radius(CornerRadius::uniform(2.0)).background(Brush::Solid(Color::rgb(swatch.r, swatch.g, swatch.b))),
                TextBlock::new().grid_column(1).text(m.label.clone()).font_weight(FontWeight::SEMI_BOLD).vertical_alignment(VerticalAlignment::Center),
                TextBlock::new().grid_column(2).text(style).opacity(0.65).font_size(12.0).vertical_alignment(VerticalAlignment::Center),
            ));
            let top = if i > 0 && m.group.as_deref() != group_of { 12.0 } else { 0.0 };
            group_of = m.group.as_deref();
            let button = Button::new()
                .horizontal_alignment(HorizontalAlignment::Stretch)
                .horizontal_content_alignment(HorizontalAlignment::Stretch)
                .min_height(40.0)
                .margin(Thickness::new(0.0, top, 0.0, 0.0))
                .on_click(act(cx, move |s| s.meter = i))
                .style(if i == selected { ButtonStyle::Accent } else { ButtonStyle::Default })
                .content(line);
            list.push(button);
        }
        // What's shipped that you've removed, a click from coming back.
        let missing = self.m.missing_meters();
        if !missing.is_empty() {
            list.push(TextBlock::new().text("Add back").font_weight(FontWeight::SEMI_BOLD).margin(Thickness::new(1.0, 20.0, 0.0, 2.0)).into());
        }
        for shipped in missing {
            let swatch = shipped.color.as_ref().or(shipped.colors.first()).map(|c| parse_color(c)).unwrap_or(Color::rgb(0x80, 0x80, 0x80));
            let label = shipped.label.clone();
            let line = Grid::new().columns([GridLength::Pixel(6.0), GridLength::STAR, GridLength::Auto]).column_spacing(12.0).children((
                Border::new().grid_column(0).width(4.0).height(22.0).corner_radius(CornerRadius::uniform(2.0)).background(Brush::Solid(Color::rgb(swatch.r, swatch.g, swatch.b))).opacity(0.6),
                TextBlock::new().grid_column(1).text(label.clone()).opacity(0.8).vertical_alignment(VerticalAlignment::Center),
                SymbolIcon::new().grid_column(2).symbol(Symbol::Add),
            ));
            list.push(
                Button::new()
                    .horizontal_alignment(HorizontalAlignment::Stretch)
                    .horizontal_content_alignment(HorizontalAlignment::Stretch)
                    .min_height(40.0)
                    .on_click(act(cx, move |s| {
                        if let Some(at) = s.m.restore_meter(&label) {
                            s.meter = at;
                        }
                    }))
                    .content(line)
            );
        }
        let n = meters.len();
        let add = Button::new()
            .horizontal_alignment(HorizontalAlignment::Stretch)
            .on_click(act(cx, |s| {
                s.m.config.meters.push(model::toml_meter(model::NEW_METER));
                s.meter = s.m.config.meters.len() - 1;
            }))
            .content(StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((SymbolIcon::new().symbol(Symbol::Add), TextBlock::new().text("Add meter"))));
        let arrange = StackPanel::new().orientation(Orientation::Horizontal).spacing(6.0).children((
            Button::new()
                .is_enabled(selected > 0)
                .on_click(act(cx, move |s| {
                    if selected > 0 && selected < s.m.config.meters.len() {
                        s.m.config.meters.swap(selected, selected - 1);
                        s.meter = selected - 1;
                    }
                }))
                .content(FontIcon::new().glyph("\u{E70E}"))
                .tooltip("Move up"),
            Button::new()
                .is_enabled(selected + 1 < n)
                .on_click(act(cx, move |s| {
                    if selected + 1 < s.m.config.meters.len() {
                        s.m.config.meters.swap(selected, selected + 1);
                        s.meter = selected + 1;
                    }
                }))
                .content(FontIcon::new().glyph("\u{E70D}"))
                .tooltip("Move down"),
            Button::new()
                .is_enabled(n > 0)
                .on_click(act(cx, move |s| {
                    if selected < s.m.config.meters.len() {
                        s.m.config.meters.remove(selected);
                    }
                }))
                .content(SymbolIcon::new().symbol(Symbol::Delete))
                .tooltip("Remove meter"),
        ));
        let editor = if meters.is_empty() { TextBlock::new().text("You don't have any meters. Select Add meter to make one.").into() } else { self.meter_editor(cx, selected) };
        // The list and the selected meter's settings scroll on their own, with the buttons under the list: selecting a
        // meter of another length moves nothing but what's on the right. (One scroll area for the page moved it all
        // whenever the shorter settings of another meter made the page shorter than where it was scrolled to.)
        let left = Grid::new().grid_column(0).rows([GridLength::STAR, GridLength::Auto]).row_spacing(12.0).children((
            ScrollViewer::new().grid_row(0).vertical_scroll_bar_visibility(ScrollBarVisibility::Auto).content(children(StackPanel::new().spacing(4.0).margin(Thickness::new(0.0, 0.0, 14.0, 0.0)), list)),
            StackPanel::new().grid_row(1).spacing(6.0).margin(Thickness::new(0.0, 0.0, 14.0, 0.0)).children((add, arrange)),
        ));
        let right = ScrollViewer::new().grid_column(1).vertical_scroll_bar_visibility(ScrollBarVisibility::Auto).content(Border::new().margin(Thickness::new(0.0, 0.0, 14.0, 12.0)).content(editor));
        Grid::new().max_width(1000.0).margin(Thickness::new(36.0, 24.0, 22.0, 0.0)).rows([GridLength::Auto, GridLength::STAR]).children((
            Border::new().grid_row(0).content(heading_reset(cx, copy::METERS, Section::Meters, self.m.at_default(Section::Meters))),
            Grid::new()
                .grid_row(1)
                .columns([GridLength::Pixel(264.0), GridLength::STAR])
                .column_spacing(10.0)
                .margin(Thickness::new(0.0, 16.0, 0.0, 0.0))
                .keyed_children([KeyedView::new("list", left), KeyedView::new(format!("meter {selected}"), right)]),
        ))
    }

    /// The `[[meter]]` table's settings, those that apply to its style. What counts as changed, and what Reset puts
    /// back, is the shipped meter of the same name (`base`); a meter of your own goes by the code's defaults.
    fn meter_editor(&self, cx: &Cx, i: usize) -> View {
        let meters = &self.m.config.meters;
        let m = &meters[i];
        let base = self.m.baseline_meter(&m.label);
        let style = m.style;
        let interval = self.m.config.interval_ms;
        let set_meter = |f: fn(&mut pms::config::MeterSpec, String)| {
            move |s: &mut Settings, v: String| {
                if let Some(m) = meter_mut(s, i) {
                    f(m, v);
                }
            }
        };
        // A whole field of the shipped meter (or the code's default, for a meter of your own), and a Reset to it.
        macro_rules! shipped {
            ($field:ident, $default:expr) => {
                base.map_or($default, |b| b.$field.clone())
            };
        }
        macro_rules! reset_to {
            ($field:ident, $value:expr) => {{
                let to = $value;
                Some(act(cx, move |s| {
                    if let Some(m) = meter_mut(s, i) {
                        m.$field = to.clone();
                    }
                }))
            }};
        }
        let shipped_style = shipped!(style, Style::History);
        let mut rows = vec![
            row("meter.label", false, None, text(cx, &m.label, "", 220.0, set_meter(|m, v| m.label = v))),
            meter_opt!(cx, meters, base, i, group, String::new(), "meter.group", |v, set| text(cx, &v, "For example, memory", 220.0, set)),
            meter_opt!(cx, meters, base, i, snug, false, "meter.snug", |v, set| toggle(cx, v, set)),
            meter_opt!(cx, meters, base, i, caption, true, "meter.caption", |v, set| toggle(cx, v, set)),
            row(
                "meter.style",
                style != shipped_style,
                reset_to!(style, shipped_style),
                choice(cx, &format!("{style:?}").to_lowercase(), copy::STYLES, move |s, v| {
                    if let Some(m) = meter_mut(s, i)
                        && let Some(style) = style_named(&v)
                    {
                        m.style = style;
                    }
                }),
            ),
        ];
        let per_style = |id: &str| match style {
            Style::Disks => format!("{id}.disks"),
            Style::Nics => format!("{id}.nics"),
            Style::Cores => format!("{id}.cores"),
            _ => id.to_string(),
        };
        let shipped_names = shipped!(names, Vec::new());
        let names_row = |id: &str| wide_row(id, m.names != shipped_names, reset_to!(names, shipped_names.clone()), list(cx, &m.names, ListAt::MeterNames(i), "Name", false));
        if matches!(style, Style::Disks | Style::Nics) {
            rows.push(names_row(&per_style("meter.names")));
        }
        let exprs = m.expr.as_ref().map(Exprs::list).unwrap_or_default();
        if !matches!(style, Style::Cores | Style::Engines | Style::Disks | Style::Nics) {
            let many = matches!(style, Style::Pie | Style::Bars);
            let shipped_expr = shipped!(expr, None);
            let control = if many {
                list(cx, &exprs, ListAt::MeterExprs(i), "Expression", false)
            } else {
                text(cx, exprs.first().map_or("", String::as_str), "Expression", 360.0, move |s, v| with_list(s, ListAt::MeterExprs(i), |l| *l = vec![v]))
            };
            rows.push(wide_row("meter.expr", base.is_some() && m.expr != shipped_expr, reset_to!(expr, shipped_expr.clone()), control));
            if many {
                rows.push(names_row("meter.names"));
            }
        }
        if matches!(style, Style::Cores | Style::Pie | Style::Bars | Style::Disks | Style::Nics) {
            // A color per part the meter draws: two for threads, disks and adapters; one per slice or bar.
            let parts: Vec<String> = match style {
                Style::Cores => vec!["User time".into(), "Kernel time".into()],
                Style::Disks => vec!["Write".into(), "Read".into()],
                Style::Nics => vec!["Sent".into(), "Received".into()],
                _ => exprs.iter().enumerate().map(|(k, e)| m.names.get(k).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| e.clone())).collect(),
            };
            let shipped_colors = shipped!(colors, Vec::new());
            rows.push(wide_row(&per_style("meter.colors"), base.is_some() && m.colors != shipped_colors, reset_to!(colors, shipped_colors.clone()), color_slots(cx, i, &parts, &m.colors)));
        } else {
            rows.push(meter_opt!(cx, meters, base, i, color, "#4FC3F7".to_string(), "meter.color", |v, set| color(cx, &v, set)));
        }
        if !matches!(style, Style::Cores | Style::Engines) {
            let shipped_max = shipped!(max, None);
            rows.push(wide_row(
                "meter.max",
                m.max != shipped_max,
                reset_to!(max, shipped_max.clone()),
                max_editor(cx, &m.max, move |s, v| {
                    if let Some(m) = meter_mut(s, i) {
                        m.max = v;
                    }
                }),
            ));
            let shipped_unit = shipped!(unit, String::new());
            rows.push(wide_row(
                "meter.unit",
                m.unit != shipped_unit,
                reset_to!(unit, shipped_unit.clone()),
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(8.0)
                    .children((choice(cx, &m.unit, copy::UNITS, set_meter(|m, v| m.unit = v)), text(cx, &m.unit, "Or another suffix", 140.0, set_meter(|m, v| m.unit = v)))),
            ));
            let decimals = if m.unit == "%" { 0 } else { 1 };
            rows.push(meter_opt!(cx, meters, base, i, decimals, decimals, "meter.decimals", |v, set| number(cx, v as f64, 0.0, 4.0, "", move |s, x| set(s, x as usize))));
        }
        rows.push(meter_opt!(cx, meters, base, i, interval_ms, interval, "meter.interval_ms", |v, set| number(cx, v as f64, 16.0, 60_000.0, "ms", move |s, x| set(s, x as u64))));
        if style != Style::History {
            rows.push(meter_opt!(cx, meters, base, i, value, true, "meter.value", |v, set| toggle(cx, v, set)));
        }
        rows.push(meter_opt!(cx, meters, base, i, tooltip, true, "meter.tooltip", |v, set| toggle(cx, v, set)));
        if matches!(style, Style::Cores | Style::Engines) {
            rows.push(meter_opt!(cx, meters, base, i, glow, "level".to_string(), "meter.glow", |v, set| choice(cx, &v, copy::GLOWS, set)));
        }
        if style == Style::Cores {
            rows.push(meter_opt!(cx, meters, base, i, kernel, "bottom".to_string(), "meter.kernel", |v, set| choice(cx, &v, copy::KERNELS, set)));
        }
        if matches!(style, Style::Cores | Style::Engines | Style::Bars | Style::Disks | Style::Nics) {
            let shipped_width = shipped!(bar_width, None);
            rows.push(row(
                "meter.bar_width",
                m.bar_width != shipped_width,
                reset_to!(bar_width, shipped_width),
                StackPanel::new().orientation(Orientation::Horizontal).spacing(12.0).children((
                    match m.bar_width {
                        Some(w) => number(cx, w, 1.0, 20.0, "px", move |s, v| {
                            if let Some(m) = meter_mut(s, i) {
                                m.bar_width = Some(v);
                            }
                        }),
                        None => TextBlock::new().text("Automatic").opacity(0.7).vertical_alignment(VerticalAlignment::Center).into(),
                    },
                    toggle(cx, m.bar_width.is_some(), move |s, on| {
                        if let Some(m) = meter_mut(s, i) {
                            m.bar_width = on.then_some(5.0);
                        }
                    }),
                )),
            ));
        }
        if style == Style::Engines {
            rows.push(meter_opt!(cx, meters, base, i, vendors, true, "meter.vendors", |v, set| toggle(cx, v, set)));
            rows.push(meter_opt!(cx, meters, base, i, vendor_label, "short".to_string(), "meter.vendor_label", |v, set| choice(cx, &v, copy::VENDOR_LABELS, set)));
            rows.push(meter_opt!(cx, meters, base, i, hold_seconds, 15, "meter.hold_seconds", |v, set| number(cx, v as f64, 0.0, 3600.0, "s", move |s, x| set(s, x as u64))));
            rows.push(meter_opt!(cx, meters, base, i, engines_fall_off, false, "meter.engines_fall_off", |v, set| toggle(cx, v, set)));
        }
        if matches!(style, Style::Disks | Style::Nics) {
            let id = per_style("meter.hold_seconds");
            rows.push(meter_opt!(cx, meters, base, i, hold_seconds, 15, &id, |v, set| number(cx, v as f64, 0.0, 3600.0, "s", move |s, x| set(s, x as u64))));
        }
        if style == Style::Pie {
            rows.push(meter_opt!(cx, meters, base, i, per_gpu, false, "meter.per_gpu", |v, set| toggle(cx, v, set)));
            rows.push(meter_opt!(cx, meters, base, i, label_inside, "never".to_string(), "meter.label_inside", |v, set| choice(cx, &v, copy::INSIDE, set)));
            rows.push(meter_opt!(cx, meters, base, i, pressure, 0.8, "meter.pressure", |v, set| number(cx, v, 0.5, 0.99, "", move |s, x| set(s, x))));
        }
        if style == Style::History {
            rows.push(meter_opt!(cx, meters, base, i, time_axis, "log".to_string(), "meter.time_axis", |v, set| choice(cx, &v, copy::TIME_AXES, set)));
            rows.push(meter_opt!(cx, meters, base, i, history_seconds, 300, "meter.history_seconds", |v, set| number(cx, v as f64, 10.0, 86_400.0, "s", move |s, x| set(s, x as u64))));
        }
        let shipped_above = shipped!(show_above, None);
        rows.push(wide_row(
            "meter.show_above",
            m.show_above != shipped_above,
            reset_to!(show_above, shipped_above.clone()),
            max_editor(cx, &m.show_above, move |s, v| {
                if let Some(m) = meter_mut(s, i) {
                    m.show_above = v;
                }
            }),
        ));
        let shipped_when = shipped!(show_when, None);
        rows.push(wide_row("meter.show_when", m.show_when != shipped_when, reset_to!(show_when, shipped_when.clone()), show_when_editor(cx, i, m.show_when.as_ref())));
        children(StackPanel::new().spacing(4.0), rows)
    }

    fn counters(&self, cx: &Cx) -> View {
        let column = |i: i32, id: &str| {
            let (title, help) = copy::setting(id);
            TextBlock::new().grid_column(i).text(title).font_size(12.0).opacity(0.7).tooltip(help)
        };
        let header = Grid::new().columns(COUNTER_COLUMNS).column_spacing(12.0).margin(Thickness::new(17.0, 16.0, 17.0, 2.0)).children((
            column(0, "counter.name"),
            column(1, "counter.path"),
            column(2, "counter.agg"),
            column(3, "counter.default"),
        ));
        let mut rows = vec![header];
        for (i, r) in self.m.counters.iter().enumerate() {
            let agg = format!("{:?}", r.agg).to_lowercase();
            rows.push(card(Grid::new().columns(COUNTER_COLUMNS).column_spacing(12.0).children((
                Border::new().grid_column(0).vertical_alignment(VerticalAlignment::Center).content(text(cx, &r.name, "Name", 120.0, move |s, v| {
                    if let Some(r) = s.m.counters.get_mut(i) {
                        r.name = v;
                    }
                })),
                Border::new().grid_column(1).vertical_alignment(VerticalAlignment::Center).content(TextBox::new().text(r.path.clone()).placeholder_text("\\Object(*)\\Counter").on_text_changed(edit(
                    cx,
                    move |s, v| {
                        if let Some(r) = s.m.counters.get_mut(i) {
                            r.path = v;
                        }
                    },
                ))),
                Border::new().grid_column(2).vertical_alignment(VerticalAlignment::Center).content(choice(cx, &agg, copy::AGGS, move |s, v| {
                    if let Some(r) = s.m.counters.get_mut(i) {
                        r.agg = match v.as_str() {
                            "max" => Agg::Max,
                            "min" => Agg::Min,
                            "avg" => Agg::Avg,
                            "engine" => Agg::Engine,
                            _ => Agg::Sum,
                        };
                    }
                })),
                Border::new().grid_column(3).vertical_alignment(VerticalAlignment::Center).content(StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
                    toggle(cx, r.default.is_some(), move |s, on| {
                        if let Some(r) = s.m.counters.get_mut(i) {
                            r.default = on.then_some(0.0);
                        }
                    }),
                    match r.default {
                        Some(d) => number(cx, d, -1e15, 1e15, "", move |s, v| {
                            if let Some(r) = s.m.counters.get_mut(i) {
                                r.default = Some(v);
                            }
                        }),
                        None => TextBlock::new().into(),
                    },
                ))),
                Border::new().grid_column(4).vertical_alignment(VerticalAlignment::Center).content(
                    Button::new()
                        .on_click(act(cx, move |s| {
                            if i < s.m.counters.len() {
                                s.m.counters.remove(i);
                            }
                        }))
                        .content(SymbolIcon::new().symbol(Symbol::Delete))
                        .tooltip("Remove counter"),
                ),
            ))));
        }
        rows.push(
            Button::new()
                .margin(Thickness::new(0.0, 12.0, 0.0, 0.0))
                .on_click(act(cx, |s| s.m.counters.push(model::new_counter())))
                .content(StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((SymbolIcon::new().symbol(Symbol::Add), TextBlock::new().text("Add counter")))),
        );
        StackPanel::new().spacing(4.0).children((heading_reset(cx, copy::COUNTERS, Section::Counters, self.m.at_default(Section::Counters)), children(StackPanel::new().spacing(4.0), rows)))
    }

    fn look(&self, cx: &Cx) -> View {
        let c = &self.m.config;
        let mut parts = vec![
            heading(copy::LOOK),
            group_reset(
                cx,
                "Background",
                Section::Backdrop,
                self.m.at_default(Section::Backdrop),
                vec![switch_card(
                    "\u{E771}",
                    "Use a custom taskbar background",
                    "Blur, tint, and grain in place of the standard acrylic",
                    c.backdrop.is_some(),
                    edit(cx, |s, on: bool| s.m.config.backdrop = on.then(|| Backdrop { tint: Some(config::DARK_TINT.into()), grain: Some(config::DEFAULT_GRAIN), gaps: None, light: None })),
                )],
            ),
        ];
        if let Some(b) = &c.backdrop {
            parts.push(group("Dark mode", look_rows(cx, false, b.tint.as_deref(), b.grain, b.gaps.as_deref())));
            let light = b.light.clone().unwrap_or_default();
            parts.push(group("Light mode", look_rows(cx, true, light.tint.as_deref(), light.grain, light.gaps.as_deref())));
        }
        parts.push(self.start_glow(cx));
        parts.push(self.app_glow(cx));
        children(StackPanel::new().spacing(4.0), parts)
    }

    fn start_glow(&self, cx: &Cx) -> View {
        let g = &self.m.config.start_glow;
        let mut cards = vec![switch_card("\u{E790}", "Glow behind the Start button", "Light up the Start button when you point to it or press it", g.is_some(), edit(cx, |s, on: bool| s.m.config.start_glow = on.then(StartGlow::default)))];
        if let Some(g) = g {
            let d = self.m.baseline.start_glow.clone().unwrap_or_default();
            macro_rules! glow {
                ($field:ident) => {
                    |s: &mut Settings, v| {
                        if let Some(g) = s.m.config.start_glow.as_mut() {
                            g.$field = v;
                        }
                    }
                };
            }
            macro_rules! reset {
                ($field:ident) => {
                    {{
                    let to = d.$field.clone();
                    Some(act(cx, move |s| {
                        if let Some(g) = s.m.config.start_glow.as_mut() {
                            g.$field = to.clone();
                        }
                    }))
                    }}
                };
            }
            cards.extend([
                row("start_glow.color", g.color != d.color, reset!(color), color(cx, &g.color, glow!(color))),
                row("start_glow.hover", g.hover != d.hover, reset!(hover), level(cx, g.hover, 1.0, glow!(hover))),
                row("start_glow.press", g.press != d.press, reset!(press), level(cx, g.press, 1.0, glow!(press))),
                row("start_glow.open", g.open != d.open, reset!(open), level(cx, g.open, 1.0, glow!(open))),
                row("start_glow.hover_ms", g.hover_ms != d.hover_ms, reset!(hover_ms), number(cx, f64::from(g.hover_ms), 0.0, 2000.0, "ms", |s, v| glow!(hover_ms)(s, v as u32))),
                row("start_glow.press_ms", g.press_ms != d.press_ms, reset!(press_ms), number(cx, f64::from(g.press_ms), 0.0, 2000.0, "ms", |s, v| glow!(press_ms)(s, v as u32))),
                row("start_glow.hover_radius", g.hover_radius != d.hover_radius, reset!(hover_radius), number(cx, f64::from(g.hover_radius), 0.0, 600.0, "px", |s, v| glow!(hover_radius)(s, v as f32))),
                row("start_glow.press_radius", g.press_radius != d.press_radius, reset!(press_radius), number(cx, f64::from(g.press_radius), 0.0, 600.0, "px", |s, v| glow!(press_radius)(s, v as f32))),
            ]);
        }
        group_reset(cx, "Start button", Section::StartGlow, self.m.at_default(Section::StartGlow), cards)
    }

    fn app_glow(&self, cx: &Cx) -> View {
        let g = &self.m.config.app_glow;
        let mut cards = vec![switch_card("\u{E790}", "Glow behind app buttons", "Each glow takes the color of its app's icon", g.is_some(), edit(cx, |s, on: bool| s.m.config.app_glow = on.then(AppGlow::default)))];
        if let Some(g) = g {
            let d = self.m.baseline.app_glow.clone().unwrap_or_default();
            macro_rules! glow {
                ($field:ident) => {
                    |s: &mut Settings, v| {
                        if let Some(g) = s.m.config.app_glow.as_mut() {
                            g.$field = v;
                        }
                    }
                };
            }
            macro_rules! reset {
                ($field:ident) => {
                    {{
                    let to = d.$field.clone();
                    Some(act(cx, move |s| {
                        if let Some(g) = s.m.config.app_glow.as_mut() {
                            g.$field = to.clone();
                        }
                    }))
                    }}
                };
            }
            cards.extend([
                row("app_glow.hover", g.hover != d.hover, reset!(hover), level(cx, g.hover, 1.0, glow!(hover))),
                row("app_glow.press", g.press != d.press, reset!(press), level(cx, g.press, 1.0, glow!(press))),
                row("app_glow.hover_ms", g.hover_ms != d.hover_ms, reset!(hover_ms), number(cx, f64::from(g.hover_ms), 0.0, 2000.0, "ms", |s, v| glow!(hover_ms)(s, v as u32))),
                row("app_glow.press_ms", g.press_ms != d.press_ms, reset!(press_ms), number(cx, f64::from(g.press_ms), 0.0, 2000.0, "ms", |s, v| glow!(press_ms)(s, v as u32))),
                row("app_glow.hover_radius", g.hover_radius != d.hover_radius, reset!(hover_radius), number(cx, f64::from(g.hover_radius), 0.0, 600.0, "px", |s, v| glow!(hover_radius)(s, v as f32))),
                row("app_glow.press_radius", g.press_radius != d.press_radius, reset!(press_radius), number(cx, f64::from(g.press_radius), 0.0, 600.0, "px", |s, v| glow!(press_radius)(s, v as f32))),
            ]);
        }
        group_reset(cx, "App buttons", Section::AppGlow, self.m.at_default(Section::AppGlow), cards)
    }

    fn hotmic(&self, cx: &Cx) -> View {
        let h = &self.m.config.hotmic;
        let mut cards = vec![switch_card("\u{E720}", "Alert me when an app uses my microphone", "Shows on the taskbar while any app has the microphone open", h.is_some(), edit(cx, |s, on: bool| s.m.config.hotmic = on.then(Hotmic::default)))];
        if let Some(h) = h {
            let d = self.m.baseline.hotmic.clone().unwrap_or_default();
            macro_rules! mic {
                ($field:ident) => {
                    |s: &mut Settings, v| {
                        if let Some(h) = s.m.config.hotmic.as_mut() {
                            h.$field = v;
                        }
                    }
                };
            }
            macro_rules! reset {
                ($field:ident) => {
                    {{
                    let to = d.$field.clone();
                    Some(act(cx, move |s| {
                        if let Some(h) = s.m.config.hotmic.as_mut() {
                            h.$field = to.clone();
                        }
                    }))
                    }}
                };
            }
            cards.extend([
                row("hotmic.color", h.color != d.color, reset!(color), color(cx, &h.color, mic!(color))),
                row("hotmic.period_ms", h.period_ms != d.period_ms, reset!(period_ms), number(cx, f64::from(h.period_ms), 0.0, 10_000.0, "ms", |s, v| mic!(period_ms)(s, v as u32))),
                row("hotmic.low", h.low != d.low, reset!(low), level(cx, h.low, 1.0, mic!(low))),
                row("hotmic.high", h.high != d.high, reset!(high), level(cx, h.high, 1.0, mic!(high))),
                row("hotmic.broad_ms", h.broad_ms != d.broad_ms, reset!(broad_ms), number(cx, f64::from(h.broad_ms), 0.0, 60_000.0, "ms", |s, v| mic!(broad_ms)(s, v as u32))),
                wide_row("hotmic.ignore", !h.ignore.is_empty(), Some(act(cx, |s| {
                    if let Some(h) = s.m.config.hotmic.as_mut() {
                        h.ignore.clear();
                    }
                })), list(cx, &h.ignore, ListAt::HotmicIgnore, "For example, obs64.exe", false)),
            ]);
        }
        StackPanel::new().spacing(4.0).children((heading(copy::HOTMIC), group_reset(cx, "Alert", Section::Hotmic, self.m.at_default(Section::Hotmic), cards)))
    }

    fn toasts(&self, cx: &Cx) -> View {
        let t = &self.m.config.toasts;
        // Off with its settings kept (`enabled = false`, as a notice's *Keep Windows notifications* leaves it), it comes
        // on again with them.
        let on = t.as_ref().is_some_and(|t| t.enabled);
        let mut cards = vec![switch_card(
            "\u{E91C}",
            "Show notifications in the taskbar",
            "When it's off, notifications appear as standard pop-ups",
            on,
            edit(cx, |s, on: bool| match (on, s.m.config.toasts.as_mut()) {
                (true, Some(t)) => t.enabled = true,
                (true, None) => s.m.config.toasts = Some(Toasts::default()),
                (false, _) => s.m.config.toasts = None,
            }),
        )];
        if let Some(t) = t.as_ref().filter(|t| t.enabled) {
            let d = self.m.baseline.toasts.clone().unwrap_or_default();
            let to_duration = d.duration_ms;
            cards.extend([
                row(
                    "toasts.duration_ms",
                    t.duration_ms != d.duration_ms,
                    Some(act(cx, move |s| {
                        if let Some(t) = s.m.config.toasts.as_mut() {
                            t.duration_ms = to_duration;
                        }
                    })),
                    number(cx, f64::from(t.duration_ms), 1000.0, 60_000.0, "ms", |s, v| {
                        if let Some(t) = s.m.config.toasts.as_mut() {
                            t.duration_ms = v as u32;
                        }
                    }),
                ),
                row(
                    "toasts.confirm_clicks",
                    t.confirm_clicks != d.confirm_clicks,
                    {
                        let to = d.confirm_clicks.clone();
                        Some(act(cx, move |s| {
                            if let Some(t) = s.m.config.toasts.as_mut() {
                                t.confirm_clicks = to.clone();
                            }
                        }))
                    },
                    choice(cx, &t.confirm_clicks, copy::CONFIRMS, |s, v| {
                        if let Some(t) = s.m.config.toasts.as_mut() {
                            t.confirm_clicks = v;
                        }
                    }),
                ),
                row(
                    "toasts.style",
                    t.style != d.style,
                    {
                        let to = d.style.clone();
                        Some(act(cx, move |s| {
                            if let Some(t) = s.m.config.toasts.as_mut() {
                                t.style = to.clone();
                            }
                        }))
                    },
                    choice(cx, &t.style, copy::TOAST_STYLES, |s, v| {
                        if let Some(t) = s.m.config.toasts.as_mut() {
                            t.style = v;
                        }
                    }),
                ),
                wide_row("toasts.ignore", !t.ignore.is_empty(), Some(act(cx, |s| {
                    if let Some(t) = s.m.config.toasts.as_mut() {
                        t.ignore.clear();
                    }
                })), list(cx, &t.ignore, ListAt::ToastsIgnore, "For example, Teams", false)),
            ]);
        }
        StackPanel::new().spacing(4.0).children((heading(copy::TOASTS), group_reset(cx, "Taskbar notifications", Section::Toasts, self.m.at_default(Section::Toasts), cards)))
    }
}

const COUNTER_COLUMNS: [GridLength; 5] = [GridLength::Pixel(130.0), GridLength::STAR, GridLength::Pixel(170.0), GridLength::Pixel(190.0), GridLength::Pixel(44.0)];

fn style_named(name: &str) -> Option<Style> {
    Some(match name {
        "history" => Style::History,
        "bar" => Style::Bar,
        "pie" => Style::Pie,
        "bars" => Style::Bars,
        "cores" => Style::Cores,
        "engines" => Style::Engines,
        "disks" => Style::Disks,
        "nics" => Style::Nics,
        _ => return None,
    })
}

/// The look being edited: `[backdrop]`'s own fields, or `[backdrop.light]`'s (made when first set, dropped again when
/// back to all defaults).
fn with_look(s: &mut Settings, light: bool, f: impl FnOnce(&mut Option<String>, &mut Option<f32>, &mut Option<String>)) {
    let Some(b) = s.m.config.backdrop.as_mut() else { return };
    if light {
        let l = b.light.get_or_insert_with(Look::default);
        f(&mut l.tint, &mut l.grain, &mut l.gaps);
        if *l == Look::default() {
            b.light = None;
        }
    } else {
        f(&mut b.tint, &mut b.grain, &mut b.gaps);
    }
}

/// Tint, grain and gaps, for the dark look (`[backdrop]`) or the light one (`[backdrop.light]`).
fn look_rows(cx: &Cx, light: bool, tint: Option<&str>, grain: Option<f32>, gaps: Option<&str>) -> Vec<View> {
    let default_tint = if light { config::LIGHT_TINT } else { config::DARK_TINT };
    vec![
        row(
            "look.tint",
            tint.is_some_and(|t| t != default_tint),
            Some(act(cx, move |s| with_look(s, light, |t, _, _| *t = None))),
            color(cx, tint.unwrap_or(default_tint), move |s, v| with_look(s, light, |t, _, _| *t = (v != default_tint).then_some(v))),
        ),
        row(
            "look.grain",
            grain.is_some_and(|g| g != config::DEFAULT_GRAIN),
            Some(act(cx, move |s| with_look(s, light, |_, g, _| *g = None))),
            level(cx, grain.unwrap_or(config::DEFAULT_GRAIN), 0.2, move |s, v| with_look(s, light, |_, g, _| *g = (v != config::DEFAULT_GRAIN).then_some(v))),
        ),
        row(
            "look.gaps",
            gaps.is_some(),
            Some(act(cx, move |s| with_look(s, light, |_, _, g| *g = None))),
            StackPanel::new().orientation(Orientation::Horizontal).spacing(12.0).children((
                match gaps {
                    Some(c) => color(cx, c, move |s, v| with_look(s, light, |_, _, g| *g = Some(v))),
                    None => TextBlock::new().into(),
                },
                toggle(cx, gaps.is_some(), move |s, on| with_look(s, light, |_, _, g| *g = on.then(|| "#18FFFFFF".into()))),
            )),
        ),
    ]
}

/// The `show_when` being edited, of meter `i`.
fn with_when(s: &mut Settings, i: usize, f: impl FnOnce(&mut ShowWhen)) {
    if let Some(w) = meter_mut(s, i).and_then(|m| m.show_when.as_mut()) {
        f(w);
    }
}

/// `show_when`: off, or an expression with the level it shows at and the one it hides below.
fn show_when_editor(cx: &Cx, i: usize, value: Option<&ShowWhen>) -> View {
    let on = toggle(cx, value.is_some(), move |s, on| {
        if let Some(m) = meter_mut(s, i) {
            m.show_when = on.then(|| ShowWhen { expr: "1 - mem_avail / ram_total".into(), at: 0.85, until: Some(0.8) });
        }
    });
    let Some(w) = value else { return on };
    StackPanel::new().spacing(8.0).children((
        on,
        text(cx, &w.expr, "For example, 1 - mem_avail / ram_total", 300.0, move |s, v| with_when(s, i, |w| w.expr = v)),
        StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
            TextBlock::new().text("Show at").vertical_alignment(VerticalAlignment::Center),
            number(cx, w.at, -1e15, 1e15, "", move |s, v| with_when(s, i, |w| w.at = v)),
            TextBlock::new().text("Hide below").vertical_alignment(VerticalAlignment::Center).margin(Thickness::new(12.0, 0.0, 0.0, 0.0)),
            number(cx, w.until.unwrap_or(w.at), -1e15, 1e15, "", move |s, v| with_when(s, i, |w| w.until = Some(v))),
        )),
    ))
}

/// A panel's children when their count varies (positional, keyed by place).
fn children(panel: StackPanel, views: Vec<View>) -> View {
    panel.keyed_children(views.into_iter().enumerate().map(|(i, v)| KeyedView::new(i.to_string(), v)))
}

/// The logo as an .ico file, for the window's title bar and taskbar button (WinUI takes a file): written once into
/// %LOCALAPPDATA%\PimpMyShell beside the notifications' PNG.
fn icon() -> Option<&'static str> {
    static ICON: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        const ICO: &[u8] = include_bytes!("../../assets/icon/pms.ico");
        let path = pms_tap::data_dir().join("pms.ico");
        if std::fs::read(&path).ok().as_deref() != Some(ICO) {
            std::fs::create_dir_all(pms_tap::data_dir()).ok()?;
            std::fs::write(&path, ICO).ok()?;
        }
        Some(path.display().to_string())
    })
    .as_deref()
}

// ---------------------------------------------------------------------------------------------
// Questions: PimpMyShell's dialogs (software update's question, a notice's explanation, Install), as a small window
// of their own: `pms-gui --ask --heading .. --text .. [--link URL] --button .. [--button ..]`. The exit code is the
// button pressed, 1 for the first; 0 when the window was closed instead. The first button is the default (the accent).

#[derive(Clone, PartialEq)]
pub struct Question {
    pub heading: String,
    pub text: String,
    pub link: Option<String>,
    pub buttons: Vec<String>,
    /// A checkbox, and the note beside it once it's checked. Checked, the answer is `CHECKED` higher.
    pub check: Option<String>,
    pub check_note: String,
    /// Exact details, each a label and a value: with these it's a question about something specific, drawn as such.
    pub rows: Vec<(String, String)>,
}

/// See `Question::check`.
const CHECKED: i32 = 16;

impl Question {
    /// From `--ask`'s arguments.
    pub fn from_args(args: &[String]) -> Question {
        let mut q = Question { heading: String::new(), text: String::new(), link: None, buttons: Vec::new(), check: None, check_note: String::new(), rows: Vec::new() };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--heading" => q.heading = it.next().cloned().unwrap_or_default(),
                "--check" => q.check = it.next().cloned(),
                "--row" => {
                    if let (Some(label), Some(value)) = (it.next(), it.next()) {
                        q.rows.push((label.clone(), value.clone()));
                    }
                }
                "--check-note" => q.check_note = it.next().cloned().unwrap_or_default(),
                "--text" => q.text = it.next().cloned().unwrap_or_default(),
                "--link" => q.link = it.next().cloned(),
                "--button" => q.buttons.extend(it.next().cloned()),
                _ => {}
            }
        }
        q
    }
}

/// Asks `q`, waiting for the answer: the exit code (see above).
pub fn ask(q: Question) -> i32 {
    if !q.rows.is_empty() {
        std::thread::spawn(to_the_corner);
    }
    match App::run_component::<Asking>(q) {
        Ok(()) => 0,
        Err(_) => 255,
    }
}

/// Shows the welcome, for the settings at `path`: the exit code says only that it ran (255 where there's no WinUI 3).
pub fn welcome(path: PathBuf) -> i32 {
    match App::run_component::<Welcome>(path) {
        Ok(()) => 0,
        Err(_) => 255,
    }
}

/// The first run: a few choices, with what they look like, in the settings' clothes. Nothing's kept until the end, so
/// closing it leaves the defaults.
struct Welcome {
    m: Model,
    step: usize,
    style: String,
    confirm: String,
    updates: bool,
    startup: bool,
    dollars: Layer,
    panel: ElementRef<SwapChainPanel>,
}

enum WelcomeMsg {
    Tick,
    Next,
    Back,
    Style(String),
    Confirm(String),
    Updates(bool),
    Startup(bool),
}

const WELCOME_STEPS: usize = 4;

fn welcome_tick(context: &ComponentContext<Welcome>) {
    context.spawn_background(|_| {
        std::thread::sleep(TICK);
        WelcomeMsg::Tick
    });
}

impl Component for Welcome {
    type Input = PathBuf;
    type Message = WelcomeMsg;

    fn create(path: &PathBuf, context: &ComponentContext<Self>) -> Self {
        welcome_tick(context);
        let m = Model::open(path.clone());
        let toasts = m.config.toasts.clone().unwrap_or_default();
        let (updates, startup) = (m.config.update_check.unwrap_or(true), true);
        Welcome { m, step: 0, style: toasts.style, confirm: toasts.confirm_clicks, updates, startup, dollars: Layer::default(), panel: ElementRef::new() }
    }

    fn update(&mut self, msg: WelcomeMsg, context: &ComponentContext<Self>) {
        match msg {
            WelcomeMsg::Tick => {
                self.dollars.attach(&self.panel);
                welcome_tick(context);
            }
            WelcomeMsg::Back => self.step = self.step.saturating_sub(1),
            WelcomeMsg::Next if self.step + 1 < WELCOME_STEPS => self.step += 1,
            WelcomeMsg::Next => self.finish(),
            WelcomeMsg::Style(v) => self.style = v,
            WelcomeMsg::Confirm(v) => self.confirm = v,
            WelcomeMsg::Updates(v) => self.updates = v,
            WelcomeMsg::Startup(v) => self.startup = v,
        }
    }

    fn view(&self, _: &PathBuf, cx: &mut ViewContext<Self>) -> View {
        cx.window_title("Welcome to PimpMyShell");
        let visuals = WindowVisuals::new().backdrop(WindowBackdrop::Acrylic).client_size(640.0, 600.0);
        cx.window_visuals(match icon() {
            Some(icon) => visuals.icon(icon),
            None => visuals,
        });
        self.dollars.set_pane(0);
        let (title, help, body): (&str, &str, View) = match self.step {
            0 => ("Welcome to the good life", "Three quick choices, and your taskbar is pimped. You can change all of it later in Settings.", Image::new().source_data(EncodedImage::new(LOGO)).width(160.0).height(160.0).horizontal_alignment(HorizontalAlignment::Left).into()),
            1 => ("To pimp, or not to pimp?", "Notifications show up in your taskbar. Here's how they can look.", {
                let pick = |value: &str, name: &str, words: &str, pimped: bool| {
                    option(cx, self.style == value, name, words, Some(sample(pimped)), WelcomeMsg::Style(value.to_string()))
                };
                children(StackPanel::new().spacing(10.0), vec![pick("standard", "Standard", "Clean, like the rest of Windows", false), pick("pimped", "Pimped", "Gold, with a sheen that sweeps across", true)])
            }),
            2 => ("Look before you leap?", "Selecting a notification can open programs and links. We can show you exactly what first.", {
                let pick = |value: &str, name: &str, words: &str| option(cx, self.confirm == value, name, words, None, WelcomeMsg::Confirm(value.to_string()));
                children(
                    StackPanel::new().spacing(10.0),
                    vec![
                        pick("card", "Ask in the notification", "A quick question, right where you clicked"),
                        pick("window", "Ask in a window", "Room for every path and argument"),
                        pick("off", "Don't ask", "Notifications open what they open"),
                    ],
                )
            }),
            _ => ("Stay fresh", "Last thing. Both are easy to change later.", {
                let line = |title: &str, words: &str, on: bool, msg: fn(bool) -> WelcomeMsg| {
                    settings_card(None, title, words, None, ToggleSwitch::new().is_on(on).on_toggled(cx.callback(msg)))
                };
                children(
                    StackPanel::new().spacing(10.0),
                    vec![
                        line("Check for updates", "We check GitHub every day or two, and you choose when to install. We don't send anything about you", self.updates, WelcomeMsg::Updates),
                        line("Start when I sign in", "Keep your taskbar pimped after a restart", self.startup, WelcomeMsg::Startup),
                    ],
                )
            }),
        };
        let last = self.step + 1 == WELCOME_STEPS;
        let next = match self.step {
            0 => "Let's ride",
            _ if last => "Get pimped",
            _ => "Next",
        };
        let dots = (0..WELCOME_STEPS).map(|i| {
            let dot = Border::new().width(8.0).height(8.0).corner_radius(CornerRadius::uniform(4.0)).background(Brush::Theme(ThemeBrush::PrimaryText)).opacity(if i == self.step { 0.9 } else { 0.25 });
            KeyedView::new(i.to_string(), dot)
        });
        let back = Button::new().grid_column(0).is_enabled(self.step > 0).opacity(if self.step > 0 { 1.0 } else { 0.0 }).min_width(90.0).on_click(cx.callback(|()| WelcomeMsg::Back)).content("Back");
        let forward = Button::new().grid_column(2).style(ButtonStyle::Accent).min_width(120.0).on_click(cx.callback(|()| WelcomeMsg::Next)).content(next);
        let page = Grid::new().rows([GridLength::Auto, GridLength::STAR, GridLength::Auto]).margin(Thickness::new(44.0, 40.0, 44.0, 28.0)).row_spacing(24.0).children((
            StackPanel::new().grid_row(0).spacing(8.0).children((
                TextBlock::new().text(title).font_size(32.0).font_weight(FontWeight::SEMI_BOLD).text_wrapping(TextWrapping::Wrap),
                TextBlock::new().text(help).font_size(15.0).opacity(0.75).text_wrapping(TextWrapping::Wrap),
            )),
            Border::new().grid_row(1).content(body),
            Grid::new().grid_row(2).columns([GridLength::Auto, GridLength::STAR, GridLength::Auto]).children((
                back,
                StackPanel::new().grid_column(1).orientation(Orientation::Horizontal).spacing(8.0).horizontal_alignment(HorizontalAlignment::Center).vertical_alignment(VerticalAlignment::Center).keyed_children(dots),
                forward,
            )),
        ));
        Grid::new().children((SwapChainPanel::new().element_ref(&self.panel), page))
    }
}

impl Welcome {
    /// Keeps the choices, and that's that.
    fn finish(&mut self) {
        let toasts = self.m.config.toasts.get_or_insert_with(Toasts::default);
        toasts.style = self.style.clone();
        toasts.confirm_clicks = self.confirm.clone();
        self.m.config.update_check = Some(self.updates);
        if self.startup != self.m.autostarts() {
            self.m.set_autostart(self.startup);
        }
        self.m.save_now();
        std::process::exit(0);
    }
}

/// A notification as it looks in the taskbar, for choosing one.
fn sample(pimped: bool) -> View {
    let gold = parse_color("#FFD35C");
    let title = TextBlock::new().text("Your taskbar called").font_size(12.0).font_weight(FontWeight::SEMI_BOLD);
    let title = if pimped { title.foreground(Brush::Solid(gold)) } else { title };
    let words = StackPanel::new().vertical_alignment(VerticalAlignment::Center).children((
        TextBlock::new().text("PimpMyShell").font_size(10.5).opacity(0.65),
        title,
        TextBlock::new().text("It says it's looking good.").font_size(11.5).opacity(0.8),
    ));
    let inside = StackPanel::new().orientation(Orientation::Horizontal).spacing(10.0).children((Image::new().source_data(EncodedImage::new(LOGO)).width(32.0).height(32.0).vertical_alignment(VerticalAlignment::Center), words));
    let card = Border::new().padding(Thickness::xy(10.0, 6.0)).corner_radius(CornerRadius::uniform(6.0)).background(Brush::Solid(if pimped { parse_color("#34FFB830") } else { parse_color("#18FFFFFF") }));
    let card = if pimped { card.border_brush(Brush::Solid(parse_color("#C8E8B030"))).border_thickness(Thickness::uniform(1.0)) } else { card };
    card.content(inside)
}

/// One of a welcome's choices: a tick when it's the one, what it's called, and maybe what it looks like.
fn option(cx: &ViewContext<Welcome>, chosen: bool, title: &str, words: &str, look: Option<View>, pick: WelcomeMsg) -> View {
    let pick = std::sync::Mutex::new(Some(pick));
    let mut lines: Vec<View> = vec![
        TextBlock::new().text(title).font_weight(FontWeight::SEMI_BOLD).into(),
        TextBlock::new().text(words).font_size(12.0).opacity(0.7).text_wrapping(TextWrapping::Wrap).into(),
    ];
    lines.extend(look.map(|l| Border::new().margin(Thickness::new(0.0, 8.0, 0.0, 0.0)).content(l)));
    let line = Grid::new().margin(Thickness::xy(4.0, 6.0)).columns([GridLength::Pixel(22.0), GridLength::STAR]).column_spacing(10.0).children((
        FontIcon::new().glyph("\u{E73E}").opacity(if chosen { 1.0 } else { 0.0 }).vertical_alignment(VerticalAlignment::Top).margin(Thickness::new(0.0, 2.0, 0.0, 0.0)).grid_column(0),
        children(StackPanel::new().grid_column(1).spacing(2.0), lines),
    ));
    let chose = match pick.lock().unwrap().take() {
        Some(WelcomeMsg::Style(v)) => cx.callback(move |()| WelcomeMsg::Style(v.clone())),
        Some(WelcomeMsg::Confirm(v)) => cx.callback(move |()| WelcomeMsg::Confirm(v.clone())),
        _ => cx.callback(|()| WelcomeMsg::Tick),
    };
    Button::new().horizontal_alignment(HorizontalAlignment::Stretch).horizontal_content_alignment(HorizontalAlignment::Stretch).on_click(chose).content(line)
}

/// A question about a notification comes up where notifications do: the bottom right of the work area, next to the
/// cards, and on top, as it's the answer to a click that's waiting. WinUI has no say in where a window starts, so it's
/// moved as it appears, and kept there while it takes its size.
fn to_the_corner() {
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows_core::BOOL;
    const MARGIN: i32 = 12;
    unsafe extern "system" fn each(hwnd: HWND, out: LPARAM) -> BOOL {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        let mut class = [0u16; 32];
        let n = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
        if pid == std::process::id() && String::from_utf16_lossy(&class[..n]).starts_with("WinUIDesktop") {
            unsafe { *(out.0 as *mut isize) = hwnd.0 as isize };
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut work = RECT::default();
    if unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut RECT as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) }.is_err() {
        return;
    }
    let mut on_top = false;
    for _ in 0..150 {
        std::thread::sleep(Duration::from_millis(8));
        let mut found = 0isize;
        let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut found as *mut isize as isize)) };
        if found == 0 {
            continue;
        }
        let hwnd = HWND(found as *mut _);
        let mut r = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut r) }.is_err() {
            continue;
        }
        let (x, y) = (work.right - (r.right - r.left) - MARGIN, work.bottom - (r.bottom - r.top) - MARGIN);
        if (r.left, r.top) != (x, y) || !on_top {
            let _ = unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, 0, 0, SWP_NOSIZE) };
            on_top = unsafe { SetForegroundWindow(hwnd) }.as_bool() || on_top;
        }
    }
}

struct Asking {
    checked: bool,
}

impl Asking {
    /// A question about something specific: the question, who's asking, the thing itself exactly, and the answer.
    /// Nothing else, with room around it; the first button (the yes) goes last, under the thumb of a reading eye.
    fn exact(&self, q: &Question, cx: &mut ViewContext<Self>) -> View {
        const WIDTH: f64 = 480.0;
        const MARGIN: f64 = 28.0;
        // A line of a detail holds about this many characters at this width.
        const PER_LINE: usize = 58;
        let lines: usize = q.rows.iter().flat_map(|(_, value)| value.split('\n')).map(|line| line.chars().count().div_ceil(PER_LINE).max(1)).sum();
        let height = 196.0 + q.rows.len() as f64 * 34.0 + lines as f64 * 19.0;
        cx.window_title("PimpMyShell");
        let visuals = WindowVisuals::new().backdrop(WindowBackdrop::Mica).client_size(WIDTH, height.min(560.0));
        cx.window_visuals(match icon() {
            Some(icon) => visuals.icon(icon),
            None => visuals,
        });
        let rows = q.rows.iter().enumerate().map(|(i, (label, value))| {
            KeyedView::new(
                i.to_string(),
                StackPanel::new().spacing(3.0).children((
                    TextBlock::new().text(label.clone()).font_size(11.0).opacity(0.6),
                    TextBlock::new().text(value.clone()).font_size(13.0).text_wrapping(TextWrapping::Wrap).is_text_selection_enabled(true),
                )),
            )
        });
        // The card scrolls when there's more than fits: the question and its answers stay where they are.
        let details = Border::new()
            .grid_row(1)
            .margin(Thickness::new(MARGIN, 18.0, MARGIN, 0.0))
            .corner_radius(CornerRadius::uniform(8.0))
            .background(Brush::Theme(ThemeBrush::CardBackground))
            .border_brush(Brush::Theme(ThemeBrush::CardStroke))
            .border_thickness(Thickness::uniform(1.0))
            .content(ScrollViewer::new().vertical_scroll_bar_visibility(ScrollBarVisibility::Auto).content(StackPanel::new().spacing(12.0).margin(Thickness::xy(16.0, 14.0)).keyed_children(rows)));
        let top = StackPanel::new().grid_row(0).spacing(2.0).margin(Thickness::new(MARGIN, 22.0, MARGIN, 0.0)).children((
            TextBlock::new().text(q.heading.clone()).font_size(24.0).font_weight(FontWeight::SEMI_BOLD).text_wrapping(TextWrapping::Wrap),
            TextBlock::new().text(q.text.clone()).opacity(0.6).text_trimming(TextTrimming::CharacterEllipsis),
        ));
        let label = q.check.clone().unwrap_or_default();
        let check = CheckBox::new().is_checked(self.checked).on_is_checked_changed(cx.callback(AskMsg::Checked)).content(TextBlock::new().text(label).font_size(12.0).opacity(0.75));
        let note = TextBlock::new().text(q.check_note.clone()).font_size(10.5).font_weight(FontWeight::BOLD).foreground(Brush::Solid(Color::rgb(0xFF, 0xD6, 0x00))).vertical_alignment(VerticalAlignment::Center).opacity(if self.checked { 1.0 } else { 0.0 });
        let left = StackPanel::new().grid_column(0).orientation(Orientation::Horizontal).spacing(10.0).vertical_alignment(VerticalAlignment::Center).children((check, note));
        let buttons = q.buttons.iter().enumerate().rev().map(|(i, label)| {
            let style = if i == 0 { ButtonStyle::Accent } else { ButtonStyle::Default };
            KeyedView::new(i.to_string(), Button::new().style(style).min_width(76.0).on_click(cx.callback(move |()| AskMsg::Pressed(i))).content(label.clone()))
        });
        let right = StackPanel::new().grid_column(1).orientation(Orientation::Horizontal).spacing(8.0).keyed_children(buttons);
        Grid::new().rows([GridLength::Auto, GridLength::STAR, GridLength::Auto]).children((
            top,
            details,
            Grid::new().grid_row(2).columns([GridLength::STAR, GridLength::Auto]).margin(Thickness::new(MARGIN, 14.0, MARGIN, 22.0)).children((left, right)),
        ))
    }
}

enum AskMsg {
    Pressed(usize),
    Checked(bool),
}

impl Component for Asking {
    type Input = Question;
    type Message = AskMsg;

    fn create(_: &Question, _: &ComponentContext<Self>) -> Self {
        Asking { checked: false }
    }

    fn update(&mut self, msg: AskMsg, _: &ComponentContext<Self>) {
        match msg {
            AskMsg::Pressed(i) => std::process::exit(i as i32 + 1 + if self.checked { CHECKED } else { 0 }),
            AskMsg::Checked(on) => self.checked = on,
        }
    }

    fn view(&self, q: &Question, cx: &mut ViewContext<Self>) -> View {
        if !q.rows.is_empty() {
            return self.exact(q, cx);
        }
        // Sized for its text (a line holds about 70 characters at this width), as a dialog is.
        let lines: usize = q.text.split('\n').map(|l| l.chars().count() / 70 + 1).sum();
        let height = 150.0 + lines as f64 * 20.0 + if q.link.is_some() { 36.0 } else { 0.0 } + if q.check.is_some() { 44.0 } else { 0.0 };
        cx.window_title("PimpMyShell");
        let visuals = WindowVisuals::new().backdrop(WindowBackdrop::Mica).client_size(520.0, height.min(640.0));
        cx.window_visuals(match icon() {
            Some(icon) => visuals.icon(icon),
            None => visuals,
        });
        let mut body: Vec<View> = vec![TextBlock::new().text(q.heading.clone()).font_size(20.0).font_weight(FontWeight::SEMI_BOLD).text_wrapping(TextWrapping::Wrap).into()];
        for paragraph in q.text.split("\n\n") {
            body.push(TextBlock::new().text(paragraph).text_wrapping(TextWrapping::Wrap).is_text_selection_enabled(true).into());
        }
        if let Some(link) = &q.link
            && let Ok(button) = HyperlinkButton::new().navigate_uri(link.clone())
        {
            body.push(button.content(TextBlock::new().text(link.clone()).text_wrapping(TextWrapping::Wrap)));
        }
        if let Some(label) = &q.check {
            let note = TextBlock::new().text(q.check_note.clone()).font_size(11.0).font_weight(FontWeight::SEMI_BOLD).foreground(Brush::Solid(Color::rgb(0xFF, 0xD6, 0x00))).vertical_alignment(VerticalAlignment::Center).opacity(if self.checked { 1.0 } else { 0.0 });
            let check = CheckBox::new().is_checked(self.checked).on_is_checked_changed(cx.callback(AskMsg::Checked)).content(label.clone());
            body.push(StackPanel::new().orientation(Orientation::Horizontal).spacing(12.0).children((check, note)));
        }
        let buttons = q.buttons.iter().enumerate().map(|(i, label)| {
            let style = if i == 0 { ButtonStyle::Accent } else { ButtonStyle::Default };
            KeyedView::new(i.to_string(), Button::new().style(style).min_width(110.0).on_click(cx.callback(move |()| AskMsg::Pressed(i))).content(label.clone()))
        });
        Grid::new().rows([GridLength::STAR, GridLength::Auto]).children((
            children(StackPanel::new().grid_row(0).spacing(12.0).margin(Thickness::new(24.0, 20.0, 24.0, 12.0)), body),
            Border::new().grid_row(1).padding(Thickness::xy(24.0, 16.0)).background(Brush::Theme(ThemeBrush::SolidBackground)).content(
                StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).horizontal_alignment(HorizontalAlignment::Right).keyed_children(buttons),
            ),
        ))
    }
}
