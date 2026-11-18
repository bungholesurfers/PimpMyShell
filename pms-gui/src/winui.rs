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
use pms::lang::{self, settings as s, welcome as w};
use pms::update::Status;
use windows_reactor::*;

use crate::copy;
use crate::dollars::Layer;
use crate::model::{self, Model, Section, Update, WadJob, Widget};

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
pub fn run(path: PathBuf, page: Option<&str>) -> Result<(), String> {
    let _ = FIRST_PAGE.set(page.and_then(page_tagged).unwrap_or(Page::General));
    App::run_component::<Settings>(path).map_err(|e| e.to_string())
}

/// The page to open on.
static FIRST_PAGE: std::sync::OnceLock<Page> = std::sync::OnceLock::new();

fn page_tagged(tag: &str) -> Option<Page> {
    PAGES.iter().find(|(_, t, ..)| *t == tag).map(|(page, ..)| *page)
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    General,
    Widgets,
    Meters,
    Counters,
    Look,
    Tv,
    Pimpbox,
    NumberOne,
    Twatch,
    Hotmic,
    Toasts,
    Network,
    Power,
    Sandbox,
    Logs,
}

impl Page {
    /// The page it goes under in the list, as what it's on: the console and Number One are channels of the TV.
    fn parent(self) -> Option<Page> {
        match self {
            Page::Pimpbox | Page::NumberOne => Some(Page::Tv),
            _ => None,
        }
    }
}

/// Each page: its tag, its icon (Segoe Fluent Icons; none for one with a picture of its own, `PTV`), and its title.
const PAGES: [(Page, &str, &str, (s::Id, s::Id)); 15] = [
    (Page::General, "general", "\u{E713}", copy::GENERAL),
    (Page::Widgets, "widgets", "\u{F0E2}", copy::WIDGETS),
    (Page::Meters, "meters", "\u{E9D9}", copy::METERS),
    (Page::Counters, "counters", "\u{E8EF}", copy::COUNTERS),
    (Page::Look, "look", "\u{E771}", copy::LOOK),
    (Page::Tv, "tv", "\u{E7F4}", copy::TV),
    (Page::Pimpbox, "pimpbox", "\u{E7FC}", copy::PIMPBOX),
    (Page::NumberOne, "iss", "", copy::NUMBER_ONE),
    (Page::Twatch, "twatch", "\u{E714}", copy::TWATCH),
    (Page::Hotmic, "hotmic", "\u{E720}", copy::HOTMIC),
    (Page::Toasts, "toasts", "\u{E91C}", copy::TOASTS),
    (Page::Network, "network", "\u{E774}", copy::NETWORK),
    (Page::Power, "power", "\u{E945}", copy::POWER),
    (Page::Sandbox, "sandbox", "\u{E72E}", (s::SANDBOX, s::SANDBOX_ABOUT)),
    (Page::Logs, "logs", "\u{E70B}", copy::LOGS),
];

/// How often the window looks in on its model: saves once edits settle, PimpMyShell's state, an update's progress.
const TICK: Duration = Duration::from_millis(300);

/// The logo, for the General page's header.
const LOGO: &[u8] = include_bytes!("../../assets/icon/pms-128.png");

/// Number One's mark, PTV (pms-art), for its page in the list.
const PTV: &[u8] = include_bytes!("../../assets/icon/ptv-64.png");

pub struct Settings {
    m: Model,
    page: Page,
    meter: usize,
    pane_open: bool,
    pane_mode: NavigationViewDisplayMode,
    /// The pages under TV are listed (its little round key shows or hides them; selecting TV doesn't).
    tv_open: bool,
    /// The section whose reset is waiting to be confirmed.
    confirm: Option<Section>,
    /// Resetting every setting is waiting to be confirmed.
    confirm_all: bool,
    /// Clearing the console's compiled shaders is waiting to be confirmed.
    confirm_shaders: bool,
    /// All of what's said about the graphics figures is showing, not only its first line.
    gpu_note_open: bool,
    /// The box for editing favorites as text has been up long enough to be given its text (`favorites`).
    box_ready: bool,
    /// The row of the processes the pointer's over.
    over_row: Option<usize>,
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
    /// The answer to whether to reset every setting.
    ResetAll(ContentDialogResult),
    /// The answer to whether to switch the console off and clear what it's compiled.
    ClearShaders(ContentDialogResult),
    Tick,
    /// The pointer came over a row of the processes, or left it.
    Over(usize, bool),
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
        Settings { m: Model::open(path.clone()), page: FIRST_PAGE.get().copied().unwrap_or(Page::General), meter: 0, pane_open: true, pane_mode: NavigationViewDisplayMode::Expanded, tv_open: true, confirm: None, confirm_all: false, confirm_shaders: false, gpu_note_open: false, box_ready: false, over_row: None, dollars: Layer::default(), panel: ElementRef::new() }
    }

    fn update(&mut self, msg: Msg, context: &ComponentContext<Self>) {
        match msg {
            Msg::Page(tag) => {
                if let Some((page, ..)) = PAGES.iter().find(|(_, t, ..)| Some(*t) == tag.as_deref()) {
                    self.page = *page;
                    self.box_ready = false;
                }
            }
            Msg::Pane(open) => self.pane_open = open,
            Msg::PaneMode(mode) => self.pane_mode = mode,
            Msg::Reset(answer) => {
                if let Some(section) = self.confirm.take().filter(|_| answer == ContentDialogResult::Primary) {
                    self.m.reset(section);
                }
            }
            Msg::ResetAll(answer) => {
                if std::mem::take(&mut self.confirm_all) && answer == ContentDialogResult::Primary {
                    self.m.reset_all();
                }
            }
            Msg::ClearShaders(answer) => {
                if std::mem::take(&mut self.confirm_shaders) && answer == ContentDialogResult::Primary {
                    self.m.clear_shaders();
                }
            }
            Msg::Tick => {
                if self.m.polled.elapsed() > Duration::from_secs(2) {
                    self.m.watching_procs = self.page == Page::General;
                    self.m.poll_daemon();
                }
                if let Some(page) = crate::model::page_asked().as_deref().and_then(page_tagged) {
                    self.page = page;
                    self.box_ready = false;
                } else {
                    self.box_ready = self.m.favorites_text.is_some();
                }
                self.m.autosave();
                self.dollars.attach(&self.panel);
                tick_later(context);
            }
            Msg::Over(row, true) => self.over_row = Some(row),
            Msg::Over(row, false) => self.over_row = self.over_row.filter(|over| *over != row),
            Msg::Edit(f) => {
                f(self);
                self.m.autosave();
            }
        }
        // Whatever changed, what's drawn next is said in the language the settings have now.
        self.m.speak();
    }

    fn view(&self, _input: &PathBuf, cx: &mut Cx) -> View {
        cx.window_title(crate::model::WINDOW_TITLE);
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
        let issues = self.m.issues();
        // A meter's issue can't be put on one meter's card: the file says which meter, not where it is in the list.
        PROBLEMS.with_borrow_mut(|p| *p = issues.iter().filter(|(id, ..)| !id.starts_with("meter.")).map(|(id, _, words)| (id.clone(), words.clone())).collect());
        // Pages under another are listed after it, indented, while it's open: their own items, rather than NavigationView's
        // nesting, which opens and closes them whenever the page above is selected. Indented only where there's room for it.
        let wide = self.pane_open && self.pane_mode == NavigationViewDisplayMode::Expanded;
        let item = |(page, tag, glyph, (name, _)): &(Page, &'static str, &'static str, (s::Id, s::Id))| -> View {
            // A page with a setting that couldn't be used says so in red.
            let marked = issues.iter().any(|(_, on, _)| on == tag);
            let title = TextBlock::new().text(name.t());
            let title = if marked { title.foreground(Brush::Theme(ThemeBrush::SystemCritical)) } else { title };
            let title = if *page == Page::General { title.font_weight(FontWeight::SEMI_BOLD) } else { title };
            // And with a red dot: the page a window opens on doesn't take its words' color at first.
            let dot = Border::new().width(7.0).height(7.0).corner_radius(CornerRadius::uniform(3.5)).vertical_alignment(VerticalAlignment::Center).background(Brush::Theme(ThemeBrush::SystemCritical)).opacity(if marked { 1.0 } else { 0.0 });
            let title = StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((title, dot));
            let has_under = PAGES.iter().any(|c| c.0.parent() == Some(*page));
            let content: View = if *page == Page::General {
                // The first stop, with the one thing to know at a glance: whether PimpMyShell is running.
                StackPanel::new().margin(Thickness::xy(0.0, 4.0)).children((title, state(self.m.daemon_running).font_size(12.0)))
            } else if has_under && wide {
                // Its key for showing or hiding what's under it: a chevron on a small circle, at the end of the row.
                // Drawn, at the size it's to be: up while what's under it shows (it closes them), down while it doesn't.
                let chevron = PathIcon::new().data(if self.tv_open { "M0,4 L4,0 L8,4 L7,5 L4,2 L1,5 Z" } else { "M0,1 L1,0 L4,3 L7,0 L8,1 L4,5 Z" }).width(8.0).height(5.0);
                // The circle is the key's face: a button of no look of its own around it takes the click, so that it doesn't
                // select the page too, its padding taken back so that it's no bigger than the circle.
                let circle = Border::new()
                    .width(22.0)
                    .height(22.0)
                    .corner_radius(CornerRadius::uniform(11.0))
                    .background(Brush::Solid(Color::argb(0x40, 0x80, 0x80, 0x80)))
                    .margin(Thickness::new(-11.0, -5.0, -11.0, -5.0))
                    .content(Border::new().horizontal_alignment(HorizontalAlignment::Center).vertical_alignment(VerticalAlignment::Center).content(chevron));
                let key = Button::new().style(ButtonStyle::Subtle).width(22.0).height(22.0).on_click(act(cx, |s| s.tv_open = !s.tv_open)).content(circle);
                Grid::new().columns([GridLength::STAR, GridLength::Auto]).width(152.0).horizontal_alignment(HorizontalAlignment::Left).children((title, Border::new().grid_column(1).vertical_alignment(VerticalAlignment::Center).content(key)))
            } else {
                title
            };
            let icon: View = if glyph.is_empty() { ImageIcon::new().source_data(EncodedImage::from_static(PTV)).into() } else { FontIcon::new().glyph(*glyph).into() };
            let indent = if page.parent().is_some() && wide { 20.0 } else { 0.0 };
            NavigationViewItem::new().tag(*tag).is_selected(self.page == *page).margin(Thickness::new(indent, 0.0, 0.0, 0.0)).slots([SlotView::new(NavigationViewItemSlot::Icon, icon), SlotView::new(NavigationViewItemSlot::Content, content)])
        };
        // The pages under one that's closed are left out, but for the one that's showing.
        let items = PAGES.iter().filter(|p| p.0.parent().is_none() || self.tv_open || self.page == p.0).map(|p| KeyedView::new(p.1, item(p)));
        let page: View = match self.page {
            Page::General => self.general(cx),
            Page::Widgets => self.widgets(cx),
            Page::Power => self.power(cx),
            Page::Logs => self.logs(cx),
            Page::Sandbox => self.sandbox(cx),
            Page::Meters => self.meters(cx),
            Page::Counters => self.counters(cx),
            Page::Look => self.look(cx),
            Page::Tv => self.tv(cx),
            Page::Pimpbox => self.pimpbox(cx),
            Page::NumberOne => self.number_one(cx),
            Page::Twatch => self.twatch(cx),
            Page::Hotmic => self.hotmic(cx),
            Page::Toasts => self.toasts(cx),
            Page::Network => self.network(cx),
        };
        let (text, error) = &self.m.message;
        let status = TextBlock::new()
            .text(text.clone())
            .text_trimming(TextTrimming::CharacterEllipsis)
            .font_size(12.0)
            .foreground(if *error { Brush::Theme(ThemeBrush::SystemCritical) } else { Brush::Theme(ThemeBrush::PrimaryText) })
            .opacity(if *error { 1.0 } else { 0.6 })
            .margin(Thickness::new(36.0, 6.0, 36.0, 10.0))
            .grid_row(2);
        // A scroll area per page (keyed by it), so that each page opens at its top; Meters has its own.
        let tag = PAGES.iter().find(|(p, ..)| *p == self.page).map_or("general", |(_, t, ..)| *t);
        let content: View = if self.page == Page::Meters {
            page
        } else {
            ScrollViewer::new().content(StackPanel::new().max_width(1000.0).margin(Thickness::new(36.0, 24.0, 36.0, 36.0)).children([page]))
        };
        let body = Grid::new().rows([GridLength::Auto, GridLength::STAR, GridLength::Auto]).keyed_children([
            KeyedView::new("problem", self.problem_banner(cx)),
            KeyedView::new(tag, Border::new().grid_row(1).content(content)),
            KeyedView::new("status", status),
        ]);
        // The small print, where nobody can claim not to have seen it; there's no room for it in a narrow pane.
        let footer: View = if self.pane_open && self.pane_mode == NavigationViewDisplayMode::Expanded {
            ScrollViewer::new().height(90.0).vertical_scroll_bar_visibility(ScrollBarVisibility::Auto).content(TextBlock::new().text(copy::LEGAL).font_size(9.0).opacity(0.55).text_wrapping(TextWrapping::Wrap).margin(Thickness::new(14.0, 6.0, 12.0, 8.0)))
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
            .title(s::VIEW_RESET_SECTION.with(&[("section", &question.name())]))
            .primary_button_text(s::VIEW_RESET.t())
            .close_button_text(s::VIEW_CANCEL.t())
            .on_closed(cx.callback(Msg::Reset))
            .content(TextBlock::new().text(self.m.warning(question)).text_wrapping(TextWrapping::Wrap));
        let confirm_all = ContentDialog::new()
            .is_open(self.confirm_all)
            .title(s::VIEW_RESET_EVERY_SETTING.t())
            .primary_button_text(s::VIEW_RESET.t())
            .close_button_text(s::VIEW_CANCEL.t())
            .on_closed(cx.callback(Msg::ResetAll))
            .content(TextBlock::new().text(copy::RESET_ALL_ASK.t()).text_wrapping(TextWrapping::Wrap));
        let confirm_shaders = ContentDialog::new()
            .is_open(self.confirm_shaders)
            .title(s::VIEW_SWITCH_THE_PIMPBOX_OFF.t())
            .primary_button_text(s::VIEW_SWITCH_OFF_AND_CLEAR.t())
            .close_button_text(s::VIEW_CANCEL.t())
            .on_closed(cx.callback(Msg::ClearShaders))
            .content(TextBlock::new().text(copy::CLEAR_SHADERS_ASK.t()).text_wrapping(TextWrapping::Wrap));
        // The dollars first: over the acrylic, under everything else.
        Grid::new().children((SwapChainPanel::new().element_ref(&self.panel), window, confirm, confirm_all, confirm_shaders))
    }
}

/// The window's gone: what was waiting to be saved is saved. A favorite just moved, most of all, which waits longer than the rest.
impl Drop for Settings {
    fn drop(&mut self) {
        self.m.save_now();
    }
}

// ---------------------------------------------------------------------------------------------
// Building blocks: page headings, groups, and a card per setting.

const GREEN: Color = Color::rgb(0x6C, 0xCB, 0x5F);
const AMBER: Color = Color::rgb(0xFC, 0xB9, 0x00);

/// Whether PimpMyShell is running, as a colored line.
fn state(running: bool) -> TextBlock {
    let (text, color) = if running { (s::STATE_RUNNING.t(), GREEN) } else { (s::STATE_NOT_RUNNING.t(), AMBER) };
    TextBlock::new().text(text).foreground(Brush::Solid(color))
}

fn heading((title, help): (s::Id, s::Id)) -> View {
    let (title, help) = (title.t(), help.t());
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
        .content(TextBlock::new().text(s::RESET_SECTION_RESET.t()).font_size(12.0))
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
fn heading_reset(cx: &Cx, text: (s::Id, s::Id), section: Section, at_default: bool) -> View {
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

/// Something to download: what it is, a few words about it, where it comes from on a line of its own, and its button.
fn download_card(title: &str, help: &str, from: &str, controls: impl Into<View>) -> View {
    let mut lines = card_lines(title, help, None);
    lines.push(TextBlock::new().text(s::DOWNLOAD_CARD_FROM.with(&[("from", &from)])).font_size(11.0).opacity(0.55).text_wrapping(TextWrapping::Wrap).is_text_selection_enabled(true).into());
    card(Grid::new().columns([GridLength::STAR, GridLength::Auto]).column_spacing(16.0).children((
        children(text_panel().grid_column(0), lines),
        Border::new().grid_column(1).vertical_alignment(VerticalAlignment::Center).content(controls),
    )))
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
    Some(Button::new().width(RESET_W).on_click(reset).content(SymbolIcon::new().symbol(Symbol::Undo)).tooltip(s::RESET_BUTTON_RESET_TO_DEFAULT.t()))
}

/// Where Reset goes, whether it shows or not: so that controls line up from card to card.
fn reset_slot(changed: bool, reset: Option<Callback<()>>) -> View {
    reset_button(changed, reset).unwrap_or_else(|| Border::new().width(RESET_W).into())
}

thread_local! {
    /// The settings that couldn't be used, and what to say under each: for `row` to mark in red as it draws.
    static PROBLEMS: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A setting: its title, description and key (copy.rs), its control, and Reset when it isn't the default.
fn row(id: &str, changed: bool, reset: Option<Callback<()>>, control: impl Into<View>) -> View {
    let (title, help) = copy::setting(id);
    let controls = StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((Border::new().vertical_alignment(VerticalAlignment::Center).content(control), reset_slot(changed, reset)));
    marked(id, settings_card(None, title, help, Some(&copy::key(id)), controls))
}

/// `card`, the setting `id`'s, outlined in red with what was wrong beneath it, if its value couldn't be used.
fn marked(id: &str, card: View) -> View {
    let Some(words) = PROBLEMS.with_borrow(|p| p.iter().find(|(s, _)| s == id).map(|(_, w)| w.clone())) else { return card };
    StackPanel::new()
        .spacing(2.0)
        .children((
            Border::new().border_brush(Brush::Theme(ThemeBrush::SystemCritical)).border_thickness(Thickness::uniform(2.0)).corner_radius(CornerRadius::uniform(6.0)).content(card),
            TextBlock::new().text(words).text_wrapping(TextWrapping::Wrap).font_size(12.0).foreground(Brush::Theme(ThemeBrush::SystemCritical)).margin(Thickness::new(4.0, 0.0, 4.0, 4.0)),
        ))
}

/// A setting whose control is too wide to sit beside its text (a list, say): the control goes under it.
fn wide_row(id: &str, changed: bool, reset: Option<Callback<()>>, control: impl Into<View>) -> View {
    let (title, help) = copy::setting(id);
    let text = children(text_panel().grid_column(0), card_lines(title, help, Some(&copy::key(id))));
    let card = card(StackPanel::new().spacing(12.0).children((
        Grid::new().columns([GridLength::STAR, GridLength::Auto]).column_spacing(16.0).children((text, Border::new().grid_column(1).vertical_alignment(VerticalAlignment::Top).content(reset_slot(changed, reset)))),
        control.into(),
    )));
    marked(id, card)
}

/// A program of PimpMyShell's that isn't beside this one (pms::parts), said at the top of the page of what needs it.
fn missing_part(file: &'static str) -> View {
    if pms::parts::here(file) {
        return Border::new().into();
    }
    InfoBar::new().title(s::MISSING_PART_IS_MISSING.with(&[("file", &file)])).message(copy::part_missing(file)).severity(InfoBarSeverity::Error).is_open(true).is_closable(false).into()
}

/// A widget on the Widgets page: its icon, its name and what it is, and the arrows that move it along the taskbar.
fn widget_row(cx: &Cx, at: usize, glyph: &str, title: &str, what: &str, (left, right): (bool, bool)) -> View {
    let step = |left: bool| act(cx, move |s: &mut Settings| s.m.move_widget(at, left));
    let arrows = StackPanel::new().orientation(Orientation::Horizontal).spacing(4.0).children((
        Button::new().is_enabled(left).on_click(step(true)).content(FontIcon::new().glyph("\u{E70E}")).tooltip(s::WIDGET_ROW_FURTHER_LEFT.t()),
        Button::new().is_enabled(right).on_click(step(false)).content(FontIcon::new().glyph("\u{E70D}")).tooltip(s::WIDGET_ROW_FURTHER_RIGHT.t()),
    ));
    settings_card(Some(glyph), title, what, None, arrows)
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
        .children((TextBlock::new().text(if on { s::SWITCH_ON.t() } else { s::SWITCH_OFF.t() }).vertical_alignment(VerticalAlignment::Center), Border::new().width(44.0).content(toggle)))
}

fn toggle(cx: &Cx, value: bool, set: impl Fn(&mut Settings, bool) + Send + Sync + 'static) -> View {
    switch(value, edit(cx, set))
}

/// One of `options` (the file's value, what the window calls it).
fn choice(cx: &Cx, value: &str, options: &'static [(&'static str, s::Id)], set: impl Fn(&mut Settings, String) + Send + Sync + 'static) -> View {
    let selected = options.iter().position(|(o, _)| *o == value);
    ComboBox::new()
        .items_source(options.iter().map(|(_, label)| label.t().to_string()))
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
/// A file name as a fixed-width font would show it: each character in a place as wide as any other's. windows-reactor has no
/// way to choose a font for a TextBlock, so this is the nearest there is.
fn fixed_width(text: &str, size: f64) -> View {
    let wide = size * 0.62;
    let places = text.chars().enumerate().map(|(i, c)| KeyedView::new(i.to_string(), Border::new().width(wide).content(TextBlock::new().text(c.to_string()).font_size(size).horizontal_alignment(HorizontalAlignment::Center))));
    StackPanel::new().orientation(Orientation::Horizontal).vertical_alignment(VerticalAlignment::Top).keyed_children(places)
}

/// Under a use of the network: every program that does it, and what each does of it.
fn done_by(what: pms::net::Use) -> View {
    let mut lines: Vec<View> = vec![TextBlock::new().text(s::NET_DONE_BY.t()).font_size(11.0).opacity(0.6).into()];
    for (program, does) in copy::programs(what) {
        lines.push(
            Grid::new()
                .columns([GridLength::Pixel(124.0), GridLength::STAR])
                .column_spacing(8.0)
                .children((Border::new().grid_column(0).content(fixed_width(program, 12.0)), TextBlock::new().grid_column(1).text(does.t()).font_size(12.0).opacity(0.8).text_wrapping(TextWrapping::Wrap))),
        );
    }
    children(StackPanel::new().spacing(2.0).margin(Thickness::new(18.0, 0.0, 18.0, 8.0)), lines)
}

/// The speed demos for the game in the console: how many of those found have been checked, and each that plays here and has
/// shooting enough to be shown (`fired_at_least`), with its leaderboard on the archive.
fn demo_cards(fired_at_least: u32) -> Vec<View> {
    let Some(iwad) = pms::wads::chosen() else { return vec![card(TextBlock::new().text(s::PIMPBOX_NO_GAMES_YET_GET.t()).text_wrapping(TextWrapping::Wrap))] };
    if !pms::demos::might_have(&iwad) {
        return vec![card(TextBlock::new().text(s::PIMPBOX_DEMOS_NONE_FOR_GAME.t()).text_wrapping(TextWrapping::Wrap))];
    }
    let known = copy::demos_known(&iwad);
    let mut cards = vec![card(TextBlock::new().text(copy::demos_so_far(&known, fired_at_least)).text_wrapping(TextWrapping::Wrap))];
    for demo in known.iter().filter(|d| pms::demos::shown(d, fired_at_least)) {
        let link: View = match HyperlinkButton::new().navigate_uri(demo.page()) {
            Ok(link) => link.content(s::PIMPBOX_DEMO_PAGE.t()),
            Err(_) => Border::new().into(),
        };
        cards.push(settings_card(None, &copy::demo_title(demo), &copy::demo_about(demo), None, link));
    }
    cards
}

/// Under a use of the network: the SOCKS login it goes as now, user name and password in full, or why it has none.
fn login_view(n: &pms::config::Network, what: pms::net::Use) -> View {
    use pms::net::Login;
    let mut lines: Vec<View> = vec![TextBlock::new().text(s::NET_LOGIN.t()).font_size(11.0).opacity(0.6).into()];
    let words = |text: &str| -> View { TextBlock::new().text(text.to_string()).font_size(12.0).opacity(0.8).text_wrapping(TextWrapping::Wrap).into() };
    let pair = |user: String, password: String| -> Vec<View> {
        [(s::NET_LOGIN_USER.t(), user), (s::NET_LOGIN_PASSWORD.t(), password)]
            .into_iter()
            .map(|(label, value)| {
                // Each character in a place of its own, so that 0 and O, l and 1 can be told apart; one too long for that, wrapped.
                let shown: View = if value.chars().count() <= 48 { fixed_width(&value, 12.0) } else { words(&value) };
                Grid::new().columns([GridLength::Pixel(124.0), GridLength::STAR]).column_spacing(8.0).children((TextBlock::new().grid_column(0).text(label).font_size(12.0).opacity(0.8), Border::new().grid_column(1).content(shown)))
            })
            .collect()
    };
    match pms::net::login(n, what) {
        Login::As { user, password } => {
            lines.extend(pair(user, password));
            if n.isolate && !pms::daemon::running() {
                lines.push(words(s::NET_LOGIN_OWN.t()));
            }
            // Each stream's own, as the daemon has them now.
            if n.isolate && what == pms::net::Use::Streams {
                lines.push(words(s::NET_LOGIN_STREAMS.t()));
                let streams = pms::net::stream_logins(n);
                if streams.is_empty() && pms::daemon::running() {
                    lines.push(words(s::NET_LOGIN_NO_STREAMS.t()));
                }
                for (tv, user, password) in streams {
                    lines.push(TextBlock::new().text(s::NET_LOGIN_STREAM.with(&[("n", &tv)])).font_size(11.0).opacity(0.6).margin(Thickness::new(0.0, 4.0, 0.0, 0.0)).into());
                    lines.extend(pair(user, password));
                }
            }
        }
        Login::Direct => lines.push(words(s::NET_LOGIN_DIRECT.t())),
        Login::Nowhere => lines.push(words(s::NET_LOGIN_NOWHERE.t())),
        Login::Without => lines.push(words(s::NET_LOGIN_WITHOUT.t())),
    }
    children(StackPanel::new().spacing(2.0).margin(Thickness::new(18.0, 0.0, 18.0, 8.0)), lines)
}

/// The languages to choose from, each by the name it calls itself, after "the same as Windows": said in the language of
/// now, so the list is made each time it's drawn.
fn language_choice(cx: &Cx, value: &str) -> View {
    let codes: Vec<&'static str> = std::iter::once(lang::AUTO).chain(lang::LANGUAGES.iter().map(|l| l.code)).collect();
    let names = std::iter::once(s::LANGUAGE_AUTO.t().to_string()).chain(lang::LANGUAGES.iter().map(|l| l.name.to_string()));
    // A code spelled another way (en-rn) is still that language; one that's nobody's is Windows' own.
    let selected = lang::find(value).map_or(0, |at| at + 1);
    ComboBox::new()
        .items_source(names)
        .selected_index(Some(selected))
        .min_width(200.0)
        .on_selection_changed(edit(cx, move |s, i: Option<usize>| {
            if let Some(code) = i.and_then(|i| codes.get(i)) {
                s.m.config.language = code.to_string();
            }
        }))
        .into()
}

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
                .tooltip(s::LIST_REMOVE.t());
            StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((field, remove))
        })
        .collect();
    let new = if colors { "#4FC3F7" } else { "" };
    lines.push(
        Button::new()
            .on_click(act(cx, move |s| with_list(s, at, |l| l.push(new.into()))))
            .content(StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((SymbolIcon::new().symbol(Symbol::Add), TextBlock::new().text(s::LIST_ADD.t())))),
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
    let kinds = ComboBox::new().items_source([s::MAX_EDITOR_NONE.t(), s::MAX_EDITOR_NUMBER.t(), s::MAX_EDITOR_EXPRESSION.t()]).selected_index(kind).width(150.0).on_selection_changed(edit(cx, move |s, i: Option<usize>| match i {
        Some(1) if kind != 1 => to_kind(s, Some(Max::Number(100.0))),
        Some(2) if kind != 2 => to_kind(s, Some(Max::Expr(String::new()))),
        Some(0) if kind != 0 => to_kind(s, None),
        _ => {}
    }));
    let field: View = match value {
        Some(Max::Number(n)) => number(cx, *n, -1e15, 1e15, "", move |s, v| set(s, Some(Max::Number(v)))),
        Some(Max::Expr(e)) => text(cx, e, s::MAX_EDITOR_FOR_EXAMPLE_RAM_TOTAL.t(), 240.0, move |s, v| set(s, Some(Max::Expr(v)))),
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
            Button::new().on_click(act(cx, |s| s.m.stop_daemon())).content(s::GENERAL_STOP.t()).tooltip(s::GENERAL_STOP_PIMPMYSHELL_AND_RESTORE.t())
        } else {
            Button::new().style(ButtonStyle::Accent).on_click(act(cx, |s| s.m.start_daemon())).content(s::GENERAL_START.t())
        };
        // Where it comes from, small, for whoever wants the source or has something to report.
        let page = pms::update::PROJECT_PAGE;
        let project: View = match HyperlinkButton::new().navigate_uri(page) {
            Ok(link) => link.content(TextBlock::new().text(page.trim_start_matches("https://")).font_size(12.0)).tooltip(s::GENERAL_PIMPMYSHELLS_PAGE_ON_GITHUB.t()),
            Err(_) => Border::new().into(),
        };
        let top: View = Grid::new().columns([GridLength::Auto, GridLength::STAR, GridLength::Auto, GridLength::Auto]).column_spacing(20.0).children((
            Image::new().grid_column(0).source_data(EncodedImage::new(LOGO)).width(64.0).height(64.0),
            StackPanel::new().grid_column(1).spacing(2.0).vertical_alignment(VerticalAlignment::Center).children((
                TextBlock::new().text("PimpMyShell").font_size(20.0).font_weight(FontWeight::SEMI_BOLD),
                TextBlock::new().text(s::GENERAL_VERSION.with(&[("version", &pms::version::CURRENT)])).font_size(12.0).opacity(0.7),
                state(running).margin(Thickness::new(0.0, 4.0, 0.0, 0.0)),
            )),
            Border::new().grid_column(2).vertical_alignment(VerticalAlignment::Center).content(project),
            Border::new().grid_column(3).vertical_alignment(VerticalAlignment::Center).content(start_stop),
        ));
        // What's running of it: each process, under the one that started it, how busy it is and how much memory is its own.
        // A table: a line between each row and column and the next, every other row a shade apart, and the one the pointer's
        // over a shade more. `at` is the row's place among them all, and none for the headings.
        let over = self.over_row;
        let line = |at: Option<usize>, cells: [String; 7], fine_print: String, quiet: bool| -> View {
            let [name, cpu, memory, gpu, engines, video, sandbox] = cells;
            let said = |text: String, _column: i32, right: bool| -> View {
                let t = TextBlock::new().text(text).font_size(12.0).opacity(if quiet { 0.6 } else { 1.0 }).vertical_alignment(VerticalAlignment::Center);
                // A name too long for its column goes onto a second line; nothing else is that long.
                if right { t.horizontal_alignment(HorizontalAlignment::Right).into() } else { t.text_wrapping(TextWrapping::Wrap).into() }
            };
            // A figure: each of its digits in a place as wide as any other's, so that figures line up under each other and
            // don't shift about as they change; its unit after it, in a place of its own. Nothing at all is said quietly.
            let figure = |text: String, column: i32| -> View {
                let figures = text.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(text.len());
                let (figures, unit) = text.split_at(figures);
                if figures.is_empty() {
                    return said(text.clone(), column, true);
                }
                // Not how busy a processor's been, though: none at all, to the last place shown, isn't none.
                let nothing = column != 1 && figures.chars().all(|c| c == '0' || c == '.');
                let place = |c: String, wide: f64| -> View { Border::new().width(wide).content(TextBlock::new().text(c).font_size(12.0).horizontal_alignment(HorizontalAlignment::Center)) };
                let mut places: Vec<View> = figures.chars().map(|c| place(c.to_string(), if c == '.' { 3.5 } else { 7.0 })).collect();
                places.push(place(unit.trim().to_string(), if unit.trim() == "%" { 12.0 } else { 24.0 }));
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .horizontal_alignment(HorizontalAlignment::Right)
                    .vertical_alignment(VerticalAlignment::Center)
                    .opacity(if nothing { 0.32 } else { 1.0 })
                    .keyed_children(places.into_iter().enumerate().map(|(i, v)| KeyedView::new(i.to_string(), v)))
            };
            let value = |text: String, column: i32| if quiet { said(text, column, true) } else { figure(text, column) };
            // The engines: as many as fit, and all of them for whoever points at them.
            let kinds: View = if quiet || engines.is_empty() {
                said(engines, 4, false)
            } else {
                TextBlock::new().text(engines.clone()).font_size(12.0).vertical_alignment(VerticalAlignment::Center).text_trimming(TextTrimming::CharacterEllipsis).tooltip(engines)
            };
            // Each in a cell of its own, with a line at its left for every column but the first.
            let cell = |content: View, column: i32| -> View {
                Border::new()
                    .grid_column(column)
                    .padding(Thickness::xy(8.0, 4.0))
                    .border_brush(Brush::Solid(TABLE_LINE))
                    .border_thickness(Thickness::new(if column == 0 { 0.0 } else { 1.0 }, 0.0, 0.0, 0.0))
                    .content(content)
            };
            let shade = match at {
                Some(at) if over == Some(at) => TABLE_OVER,
                Some(at) if at % 2 == 1 => TABLE_OTHER,
                _ => TABLE_CLEAR,
            };
            // The sandbox: its kind and helper in short, and all of it (its full name, SID and capabilities) for whoever points.
            let boxed: View = if quiet || fine_print.is_empty() {
                said(sandbox, 6, false)
            } else {
                TextBlock::new().text(sandbox).font_size(12.0).vertical_alignment(VerticalAlignment::Center).text_trimming(TextTrimming::CharacterEllipsis).tooltip(fine_print)
            };
            let cells = Grid::new().columns(TABLE_COLUMNS).children((cell(said(name, 0, false), 0), cell(value(cpu, 1), 1), cell(value(memory, 2), 2), cell(value(gpu, 3), 3), cell(kinds, 4), cell(value(video, 5), 5), cell(boxed, 6)));
            // The line under it; and a fill, clear as it may be, which is what lets a row know the pointer's over it.
            let row = Border::new().background(Brush::Solid(shade)).border_brush(Brush::Solid(TABLE_LINE)).border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0));
            match at {
                Some(at) => row.on_pointer_entered(cx.callback(move |_| Msg::Over(at, true))).on_pointer_exited(cx.callback(move |_| Msg::Over(at, false))).content(cells),
                None => row.content(cells),
            }
        };
        let megabytes = |bytes: u64| format!("{:.1} MB", bytes as f64 / 1_048_576.0);
        let running = |at: usize, p: &model::Running| {
            let (gpu, engines, video) = p.gpu.as_ref().map_or_else(Default::default, |g| (format!("{:.1}%", g.busy), g.engines.join(", "), megabytes(g.memory)));
            let (sandbox, fine_print) = sandbox_said(p.sandbox.as_ref());
            line(Some(at), [format!("{}{}", "\u{2003}".repeat(p.depth), p.name), p.cpu.map_or_else(String::new, |c| format!("{c:.2}%")), megabytes(p.memory), gpu, engines, video, sandbox], fine_print, false)
        };
        // A frame around the rows, which have the line under each of them themselves.
        let table = |rows: Vec<View>, above: f64| -> View {
            let rows = StackPanel::new().keyed_children(rows.into_iter().enumerate().map(|(i, v)| KeyedView::new(i.to_string(), v)));
            Border::new().margin(Thickness::new(0.0, above, 0.0, 0.0)).border_brush(Brush::Solid(TABLE_LINE)).border_thickness(Thickness::new(1.0, 1.0, 1.0, 0.0)).content(rows)
        };
        let mut ours = vec![line(None, [s::GENERAL_NAME.t(), s::GENERAL_CPU.t(), s::GENERAL_MEMORY.t(), s::GENERAL_GPU.t(), s::GENERAL_GPU_ENGINES.t(), s::GENERAL_DEDI_GPU_MEM_COMMIT.t(), s::SANDBOX_COLUMN.t()].map(String::from), String::new(), true)];
        ours.extend(m.procs.iter().enumerate().map(|(at, p)| running(at, p)));
        let mut processes = vec![table(ours, 0.0)];
        // And, a little apart, what isn't ours but does our work: the Explorer the taskbar's in, and the Desktop Window Manager.
        if !m.theirs.is_empty() {
            processes.push(table(m.theirs.iter().enumerate().map(|(at, p)| running(m.procs.len() + at, p)).collect(), 6.0));
        }
        for p in m.theirs.iter().filter(|p| !p.about.is_empty()) {
            processes.push(TextBlock::new().text(p.about).font_size(11.0).opacity(0.6).text_wrapping(TextWrapping::Wrap).margin(Thickness::new(0.0, 0.0, 0.0, 2.0)).into());
        }
        // What the graphics figures can and can't be trusted for; and what the cards really have in use, to set them against.
        // In a line, with the rest of it for whoever asks.
        if m.procs.iter().chain(&m.theirs).any(|p| p.gpu.is_some()) {
            let quiet = |text: String| TextBlock::new().text(text).font_size(11.0).opacity(0.6).text_wrapping(TextWrapping::Wrap).vertical_alignment(VerticalAlignment::Center);
            let in_use = m.cards_in_use.map_or_else(String::new, |bytes| s::GENERAL_CARDS_IN_USE.with(&[("amount", &megabytes(bytes))]));
            let more = self.gpu_note_open;
            let ask = Button::new().on_click(act(cx, move |s| s.gpu_note_open = !more)).content(TextBlock::new().text(if more { s::GENERAL_SHOW_LESS.t() } else { s::GENERAL_SHOW_MORE.t() }).font_size(11.0));
            processes.push(
                Grid::new()
                    .columns([GridLength::STAR, GridLength::Auto])
                    .column_spacing(12.0)
                    .margin(Thickness::new(0.0, 4.0, 0.0, 0.0))
                    .children((quiet(s::GENERAL_GPU_FIGURES.with(&[("more", &in_use)])).grid_column(0), Border::new().grid_column(1).content(ask))),
            );
            if more {
                processes.push(quiet(pms::procs::GPU_MEMORY_IS.t().to_string()).into());
            }
        }
        let running_now: View = StackPanel::new().spacing(3.0).keyed_children(processes.into_iter().enumerate().map(|(i, v)| KeyedView::new(i.to_string(), v)));
        let header = card(StackPanel::new().spacing(12.0).children((top, running_now)));
        let speaks = group(s::LANGUAGE.t(), vec![row("language", c.language != b.language, top!(language), language_choice(cx, &c.language))]);
        let files = group(
            s::GENERAL_YOUR_FILES.t(),
            vec![
                settings_card(Some("\u{E8A5}"), s::GENERAL_SETTINGS_FILE.t(), &pms::shown(&m.path), None, Button::new().on_click(act(cx, |s| s.m.show_config())).content(s::GENERAL_SHOW_IN_FILE_EXPLORER.t())),
                settings_card(Some("\u{E728}"), s::GENERAL_TWATCH_TV_WATCHLIST.t(), &pms::shown(&pms::config::watchlist_path(&m.path)), None, Button::new().on_click(act(cx, |s| s.m.show_watchlist())).content(s::GENERAL_SHOW_IN_FILE_EXPLORER.t())),
                settings_card(Some("\u{E774}"), s::GENERAL_NETWORK_SETTINGS.t(), &pms::shown(&pms::config::network_path(&m.path)), None, Button::new().on_click(act(cx, |s| s.m.show_network())).content(s::GENERAL_SHOW_IN_FILE_EXPLORER.t())),
                TextBlock::new().text(copy::PRIVATE_FILES.t()).text_wrapping(TextWrapping::Wrap).font_size(12.0).opacity(0.7).margin(Thickness::new(1.0, 2.0, 0.0, 6.0)).into(),
            ],
        );
        let startup = group(
            s::GENERAL_STARTUP.t(),
            vec![settings_card(Some("\u{E7E8}"), s::GENERAL_START_WHEN_I_SIGN.t(), s::GENERAL_RUN_PIMPMYSHELL_AUTOMATICALLY_WHEN.t(), None, switch(m.autostarts(), edit(cx, |s, on: bool| s.m.set_autostart(on))))],
        );
        let taskbar = group_reset(
            cx,
            s::GENERAL_TASKBAR.t(),
            Section::Taskbar,
            m.at_default(Section::Taskbar),
            vec![
                row("theme", c.theme != b.theme, top!(theme), choice(cx, &c.theme, copy::THEMES, |s, v| s.m.config.theme = v)),
                row("meter_height", c.meter_height != b.meter_height, top!(meter_height), choice(cx, &c.meter_height, copy::HEIGHTS, |s, v| s.m.config.meter_height = v)),
                row("bar_height", c.bar_height != b.bar_height, top!(bar_height), choice(cx, &c.bar_height, copy::HEIGHTS, |s, v| s.m.config.bar_height = v)),
                row("interval_ms", c.interval_ms != b.interval_ms, top!(interval_ms), number(cx, c.interval_ms as f64, 16.0, 60_000.0, "ms", |s, v| s.m.config.interval_ms = v as u64)),
                row("value_font", c.value_font != b.value_font, top!(value_font), text(cx, c.value_font.as_deref().unwrap_or(""), "Cascadia Mono", 220.0, |s, v| s.m.config.value_font = Some(v).filter(|v| !v.is_empty()))),
                row("software_gpus", c.software_gpus != b.software_gpus, top!(software_gpus), toggle(cx, c.software_gpus, |s, v| s.m.config.software_gpus = v)),
            ],
        );
        let window = group(
            s::GENERAL_OTHER.t(),
            vec![
                row("notify_explorer", !self.m.config.notify_explorer, Some(act(cx, |s| s.m.config.notify_explorer = true)), toggle(cx, self.m.config.notify_explorer, |s, v| s.m.config.notify_explorer = v)),
                settings_card(Some("\u{E72C}"), s::GENERAL_RESTART_EXPLORER.t(), s::GENERAL_GIVES_BACK_WHAT_EXPLORERS.t(), None, Button::new().on_click(act(cx, |s| s.m.restart_explorer())).content(s::GENERAL_RESTART_NOW.t())),
                settings_card(
                    Some("\u{E789}"),
                    s::GENERAL_WELCOME.t(),
                    s::GENERAL_THE_FIRST_RUN_QUESTIONS.t(),
                    None,
                    Button::new().on_click(act(cx, |s| s.m.open_welcome())).content(s::GENERAL_OPEN.t()),
                ),
                settings_card(
                    Some("\u{E8A7}"),
                    s::GENERAL_CLASSIC_SETTINGS_WINDOW.t(),
                    s::GENERAL_THE_SIMPLER_WINDOW_PCS.t(),
                    None,
                    Button::new().on_click(act(cx, |s| s.m.open_classic())).content(s::GENERAL_OPEN.t()),
                ),
                settings_card(Some("\u{E777}"), copy::RESET_ALL.0.t(), copy::RESET_ALL.1.t(), None, Button::new().on_click(act(cx, |s| s.confirm_all = true)).content(s::GENERAL_RESET.t())),
            ],
        );
        let legal = group(s::GENERAL_LEGAL.t(), vec![card(TextBlock::new().text(copy::LEGAL).text_wrapping(TextWrapping::Wrap).font_size(11.0).opacity(0.7))]);
        StackPanel::new().spacing(4.0).children((heading(copy::GENERAL), header, speaks, files, startup, self.software_update(cx), taskbar, window, legal))
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
            Update::Idle if !installed => found.push(bar(s::SOFTWARE_UPDATE_THIS_IS_A_PORTABLE.t().into(), s::SOFTWARE_UPDATE_IT_CAN_CHECK_FOR.t().into(), InfoBarSeverity::Informational)),
            Update::Idle => {}
            Update::Checking => found.push(StackPanel::new().spacing(6.0).children((TextBlock::new().text(s::SOFTWARE_UPDATE_CHECKING_FOR_UPDATES.t()), ProgressBar::new().is_indeterminate(true).width(320.0).horizontal_alignment(HorizontalAlignment::Left)))),
            Update::Downloading(done, total) => found.push(StackPanel::new().spacing(6.0).children((
                TextBlock::new().text(s::SOFTWARE_UPDATE_DOWNLOADING.with(&[("done", &(done / 1024)), ("total", &(total / 1024))])),
                ProgressBar::new().minimum(0.0).maximum(total.max(1) as f64).value(done as f64).is_indeterminate(false).width(320.0).horizontal_alignment(HorizontalAlignment::Left),
            ))),
            Update::Done(Status::UpToDate { stable }) => {
                let which = if latest { s::SOFTWARE_UPDATE_THE_LATEST_RELEASE.t() } else { s::SOFTWARE_UPDATE_THE_LATEST_STABLE_VERSION.t() };
                found.push(bar(s::SOFTWARE_UPDATE_YOURE_UP_TO_DATE.t().into(), s::SOFTWARE_UPDATE_WHICH_IS.with(&[("which", &which), ("version", &stable)]), InfoBarSeverity::Success));
            }
            Update::Done(Status::Available { version }) => found.push(StackPanel::new().spacing(8.0).children((
                bar(s::SOFTWARE_UPDATE_AVAILABLE.with(&[("version", &version)]), s::SOFTWARE_UPDATE_THIS_COPY_CANT_INSTALL.t().into(), InfoBarSeverity::Informational),
                Button::new().on_click(act(cx, move |_| model::open_page(&pms::update::release_page(&version)))).content(s::SOFTWARE_UPDATE_GO_TO_THE_RELEASE.t()),
            ))),
            Update::Done(Status::Stuck { version }) => found.push(StackPanel::new().spacing(8.0).children((
                bar(s::SOFTWARE_UPDATE_DIDNT_INSTALL.with(&[("version", &version)]), s::SOFTWARE_UPDATE_IT_WAS_INSTALLED_BUT.t().into(), InfoBarSeverity::Warning),
                Button::new().on_click(act(cx, |_| model::open_page(pms::update::RELEASES_PAGE))).content(s::SOFTWARE_UPDATE_GO_TO_THE_RELEASES.t()),
            ))),
            Update::Done(Status::Ready { version, path, sha256 }) => {
                let note = if version == pms::version::CURRENT {
                    s::SOFTWARE_UPDATE_ITS_THE_VERSION_YOU.t().to_string()
                } else if pms::version::newer(pms::version::CURRENT, &version) {
                    s::SOFTWARE_UPDATE_OLDER.with(&[("version", &pms::version::CURRENT)])
                } else {
                    String::new()
                };
                let v = version.clone();
                found.push(StackPanel::new().spacing(8.0).children((
                    bar(s::SOFTWARE_UPDATE_READY.with(&[("version", &version)]), s::SOFTWARE_UPDATE_VERIFIED.with(&[("more", &note)]), InfoBarSeverity::Success),
                    StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
                        Button::new()
                            .style(ButtonStyle::Accent)
                            .on_click(act(cx, move |s| s.m.install(&path, &v, &sha256)))
                            .content(s::SOFTWARE_UPDATE_INSTALL_AND_RESTART.t())
                            .tooltip(s::SOFTWARE_UPDATE_THE_TASKBAR_RETURNS_TO.t()),
                        Button::new().on_click(act(cx, |s| *s.m.update.lock().unwrap() = Update::Idle)).content(s::SOFTWARE_UPDATE_NOT_NOW.t()),
                    )),
                )));
            }
            Update::Failed(e) => found.push(bar(s::SOFTWARE_UPDATE_WE_COULDNT_CHECK_FOR.t().into(), e, InfoBarSeverity::Error)),
        }
        let check = Button::new()
            .style(ButtonStyle::Accent)
            .is_enabled(!busy)
            .on_click(act(cx, move |s| s.m.start_check(pms::update::Ask { install: installed, automatic: false, force: false, latest })))
            .content(s::SOFTWARE_UPDATE_CHECK_FOR_UPDATES.t());
        let mut cards = vec![
            settings_card(Some("\u{E895}"), &s::GENERAL_VERSION.with(&[("version", &pms::version::CURRENT)]), &pms::update::addresses(), None, check),
        ];
        if !found.is_empty() {
            cards.push(card(children(StackPanel::new().spacing(8.0), found)));
        }
        let (title, help) = copy::setting("update_check");
        let help = if c.update_check.is_none() { s::SOFTWARE_UPDATE_NOT_CHOSEN.with(&[("help", &help)]) } else { help.to_string() };
        cards.push(settings_card(None, title, &help, Some("update_check"), {
            let mut controls = vec![toggle(cx, on, |s, v| s.m.config.update_check = Some(v))];
            controls.extend(reset_button(!on, Some(act(cx, |s| s.m.config.update_check = Some(true)))));
            children(StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0), controls)
        }));
        cards.push(row("update_channel", c.update_channel != self.m.baseline.update_channel, {
            let to = self.m.baseline.update_channel.clone();
            Some(act(cx, move |s| s.m.config.update_channel = to.clone()))
        }, choice(cx, &c.update_channel, copy::CHANNELS, |s, v| s.m.config.update_channel = v)));
        cards.push(row("update_warn", !c.update_warn, Some(act(cx, |s| s.m.config.update_warn = true)), toggle(cx, c.update_warn, |s, v| s.m.config.update_warn = v)));
        let reinstall = |label: &str, latest: bool| {
            Button::new()
                .is_enabled(!busy && installed)
                .on_click(act(cx, move |s| s.m.start_check(pms::update::Ask { install: true, automatic: false, force: true, latest })))
                .content(label)
                .tooltip(if installed { s::SOFTWARE_UPDATE_DOWNLOAD_THIS_VERSION_EVEN.t() } else { s::SOFTWARE_UPDATE_ONLY_AN_INSTALLED_COPY.t() })
        };
        cards.push(settings_card(
            None,
            s::SOFTWARE_UPDATE_REINSTALL.t(),
            s::SOFTWARE_UPDATE_REPAIR_PIMPMYSHELL_OR_GO.t(),
            None,
            StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((reinstall(s::SOFTWARE_UPDATE_STABLE_VERSION.t(), false), reinstall(s::SOFTWARE_UPDATE_LATEST_VERSION.t(), true))),
        ));
        group_reset(cx, s::SOFTWARE_UPDATE_SOFTWARE_UPDATE.t(), Section::Updates, self.m.at_default(Section::Updates), cards)
    }

    fn meters(&self, cx: &Cx) -> View {
        let meters = &self.m.config.meters;
        // The meters, not the TVs (they're widgets of their own), and which is selected.
        let plain = self.m.plain_meters();
        let selected = if plain.contains(&self.meter) { self.meter } else { plain.first().copied().unwrap_or(0) };
        // The list, in the taskbar's order: a swatch, the name, the style; a gap between groups.
        let mut list: Vec<View> = Vec::new();
        let mut group_of: Option<&str> = None;
        for &i in &plain {
            let m = &meters[i];
            let swatch = m.color.as_ref().or(m.colors.first()).map(|c| parse_color(c)).unwrap_or(Color::rgb(0x80, 0x80, 0x80));
            let style = copy::STYLES.iter().find(|(s, _)| *s == format!("{:?}", m.style).to_lowercase()).map_or("", |(_, l)| l.t());
            let line = Grid::new().columns([GridLength::Pixel(6.0), GridLength::STAR, GridLength::Auto]).column_spacing(12.0).children((
                Border::new().grid_column(0).width(4.0).height(22.0).corner_radius(CornerRadius::uniform(2.0)).background(Brush::Solid(Color::rgb(swatch.r, swatch.g, swatch.b))),
                TextBlock::new().grid_column(1).text(m.label.clone()).font_weight(FontWeight::SEMI_BOLD).vertical_alignment(VerticalAlignment::Center),
                TextBlock::new().grid_column(2).text(style).opacity(0.65).font_size(12.0).vertical_alignment(VerticalAlignment::Center),
            ));
            let top = if i != plain[0] && m.group.as_deref() != group_of { 12.0 } else { 0.0 };
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
            list.push(TextBlock::new().text(s::METERS_ADD_BACK.t()).font_weight(FontWeight::SEMI_BOLD).margin(Thickness::new(1.0, 20.0, 0.0, 2.0)).into());
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
        let (first, last) = (plain.first() == Some(&selected), plain.last() == Some(&selected));
        let add = Button::new()
            .horizontal_alignment(HorizontalAlignment::Stretch)
            .on_click(act(cx, |s| {
                let at = s.m.new_meter_at();
                s.m.config.meters.insert(at, model::toml_meter(model::NEW_METER));
                s.meter = at;
            }))
            .content(StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((SymbolIcon::new().symbol(Symbol::Add), TextBlock::new().text(s::METERS_ADD_METER.t()))));
        let arrange = StackPanel::new().orientation(Orientation::Horizontal).spacing(6.0).children((
            Button::new().is_enabled(!plain.is_empty() && !first).on_click(act(cx, move |s| s.meter = s.m.move_meter(selected, true))).content(FontIcon::new().glyph("\u{E70E}")).tooltip(s::METERS_MOVE_UP.t()),
            Button::new().is_enabled(!plain.is_empty() && !last).on_click(act(cx, move |s| s.meter = s.m.move_meter(selected, false))).content(FontIcon::new().glyph("\u{E70D}")).tooltip(s::METERS_MOVE_DOWN.t()),
            Button::new()
                .is_enabled(!plain.is_empty())
                .on_click(act(cx, move |s| {
                    if s.m.config.meters.get(selected).is_some_and(|m| !model::is_screen(m)) {
                        s.m.config.meters.remove(selected);
                    }
                }))
                .content(SymbolIcon::new().symbol(Symbol::Delete))
                .tooltip(s::METERS_REMOVE_METER.t()),
        ));
        let editor = if plain.is_empty() { TextBlock::new().text(s::METERS_YOU_DONT_HAVE_ANY.t()).into() } else { self.meter_editor(cx, selected) };
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
            meter_opt!(cx, meters, base, i, group, String::new(), "meter.group", |v, set| text(cx, &v, s::METER_EDITOR_FOR_EXAMPLE_MEMORY.t(), 220.0, set)),
            meter_opt!(cx, meters, base, i, snug, false, "meter.snug", |v, set| toggle(cx, v, set)),
            meter_opt!(cx, meters, base, i, caption, true, "meter.caption", |v, set| toggle(cx, v, set)),
            meter_opt!(cx, meters, base, i, sideways, matches!(m.style, Style::Disks | Style::Nics), "meter.sideways", |v, set| toggle(cx, v, set)),
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
        let names_row = |id: &str| wide_row(id, m.names != shipped_names, reset_to!(names, shipped_names.clone()), list(cx, &m.names, ListAt::MeterNames(i), s::METER_EDITOR_NAME.t(), false));
        if matches!(style, Style::Disks | Style::Nics) {
            rows.push(names_row(&per_style("meter.names")));
        }
        let exprs = m.expr.as_ref().map(Exprs::list).unwrap_or_default();
        if !matches!(style, Style::Cores | Style::Engines | Style::Disks | Style::Nics) {
            let many = matches!(style, Style::Pie | Style::Bars);
            let shipped_expr = shipped!(expr, None);
            let control = if many {
                list(cx, &exprs, ListAt::MeterExprs(i), s::METER_EDITOR_EXPRESSION.t(), false)
            } else {
                text(cx, exprs.first().map_or("", String::as_str), s::METER_EDITOR_EXPRESSION.t(), 360.0, move |s, v| with_list(s, ListAt::MeterExprs(i), |l| *l = vec![v]))
            };
            rows.push(wide_row("meter.expr", base.is_some() && m.expr != shipped_expr, reset_to!(expr, shipped_expr.clone()), control));
            if many {
                rows.push(names_row("meter.names"));
            }
        }
        if matches!(style, Style::Cores | Style::Pie | Style::Bars | Style::Disks | Style::Nics) {
            // A color per part the meter draws: two for threads, disks and adapters; one per slice or bar.
            let parts: Vec<String> = match style {
                Style::Cores => vec![s::METER_EDITOR_USER_TIME.t().into(), s::METER_EDITOR_KERNEL_TIME.t().into()],
                Style::Disks => vec![s::METER_EDITOR_WRITE.t().into(), s::METER_EDITOR_READ.t().into()],
                Style::Nics => vec![s::METER_EDITOR_SENT.t().into(), s::METER_EDITOR_RECEIVED.t().into()],
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
                    .children((choice(cx, &m.unit, copy::UNITS, set_meter(|m, v| m.unit = v)), text(cx, &m.unit, s::METER_EDITOR_OR_ANOTHER_SUFFIX.t(), 140.0, set_meter(|m, v| m.unit = v)))),
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
                        None => TextBlock::new().text(s::METER_EDITOR_AUTOMATIC.t()).opacity(0.7).vertical_alignment(VerticalAlignment::Center).into(),
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
            rows.push(meter_opt!(cx, meters, base, i, slice_glow, String::new(), "meter.slice_glow", |v, set| text(cx, &v, s::METER_EDITOR_FOR_EXAMPLE_MODIFIED.t(), 180.0, move |s, x| set(s, x))));
            rows.push(meter_opt!(cx, meters, base, i, slice_glow_extend, 2.5, "meter.slice_glow_extend", |v, set| number(cx, v, 0.0, 8.0, " px", move |s, x| set(s, x))));
            rows.push(meter_opt!(cx, meters, base, i, slice_glow_radius, 4.0, "meter.slice_glow_radius", |v, set| number(cx, v, 1.0, 24.0, " px", move |s, x| set(s, x))));
            rows.push(meter_opt!(cx, meters, base, i, slice_glow_strength, 1.0, "meter.slice_glow_strength", |v, set| number(cx, v, 0.1, 1.0, "", move |s, x| set(s, x))));
            rows.push(meter_opt!(cx, meters, base, i, slice_glow_color, String::new(), "meter.slice_glow_color", |v, set| text(cx, &v, "#FFB74D", 120.0, move |s, x| set(s, x))));
            rows.push(meter_opt!(cx, meters, base, i, slice_glow_pulse_ms, 0, "meter.slice_glow_pulse_ms", |v, set| number(cx, v as f64, 0.0, 10_000.0, " ms", move |s, x| set(s, x as u64))));
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
                Border::new().grid_column(0).vertical_alignment(VerticalAlignment::Center).content(text(cx, &r.name, s::COUNTERS_NAME.t(), 120.0, move |s, v| {
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
                        .tooltip(s::COUNTERS_REMOVE_COUNTER.t()),
                ),
            ))));
        }
        rows.push(
            Button::new()
                .margin(Thickness::new(0.0, 12.0, 0.0, 0.0))
                .on_click(act(cx, |s| s.m.counters.push(model::new_counter())))
                .content(StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((SymbolIcon::new().symbol(Symbol::Add), TextBlock::new().text(s::COUNTERS_ADD_COUNTER.t())))),
        );
        StackPanel::new().spacing(4.0).children((heading_reset(cx, copy::COUNTERS, Section::Counters, self.m.at_default(Section::Counters)), children(StackPanel::new().spacing(4.0), rows)))
    }

    fn look(&self, cx: &Cx) -> View {
        let c = &self.m.config;
        let mut parts = vec![
            heading(copy::LOOK),
            group_reset(
                cx,
                s::LOOK_BACKGROUND.t(),
                Section::Backdrop,
                self.m.at_default(Section::Backdrop),
                vec![switch_card(
                    "\u{E771}",
                    s::LOOK_USE_A_CUSTOM_TASKBAR.t(),
                    s::LOOK_BLUR_TINT_AND_GRAIN.t(),
                    c.backdrop.is_some(),
                    edit(cx, |s, on: bool| s.m.config.backdrop = on.then(|| Backdrop { tint: Some(config::DARK_TINT.into()), grain: Some(config::DEFAULT_GRAIN), gaps: None, light: None })),
                )],
            ),
        ];
        if let Some(b) = &c.backdrop {
            parts.push(group(s::LOOK_DARK_MODE.t(), look_rows(cx, false, b.tint.as_deref(), b.grain, b.gaps.as_deref())));
            let light = b.light.clone().unwrap_or_default();
            parts.push(group(s::LOOK_LIGHT_MODE.t(), look_rows(cx, true, light.tint.as_deref(), light.grain, light.gaps.as_deref())));
        }
        parts.push(self.start_glow(cx));
        parts.push(self.app_glow(cx));
        children(StackPanel::new().spacing(4.0), parts)
    }

    fn start_glow(&self, cx: &Cx) -> View {
        let g = &self.m.config.start_glow;
        let mut cards = vec![switch_card("\u{E790}", s::START_GLOW_GLOW_BEHIND_THE_START.t(), s::START_GLOW_LIGHT_UP_THE_START.t(), g.is_some(), edit(cx, |s, on: bool| s.m.config.start_glow = on.then(StartGlow::default)))];
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
        group_reset(cx, s::START_GLOW_START_BUTTON.t(), Section::StartGlow, self.m.at_default(Section::StartGlow), cards)
    }

    fn app_glow(&self, cx: &Cx) -> View {
        let g = &self.m.config.app_glow;
        let mut cards = vec![switch_card("\u{E790}", s::APP_GLOW_GLOW_BEHIND_APP_BUTTONS.t(), s::APP_GLOW_EACH_GLOW_TAKES_THE.t(), g.is_some(), edit(cx, |s, on: bool| s.m.config.app_glow = on.then(AppGlow::default)))];
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
        group_reset(cx, s::APP_GLOW_APP_BUTTONS.t(), Section::AppGlow, self.m.at_default(Section::AppGlow), cards)
    }

    fn hotmic(&self, cx: &Cx) -> View {
        let h = &self.m.config.hotmic;
        let mut cards = vec![switch_card("\u{E720}", s::HOTMIC_ALERT_ME_WHEN_AN.t(), s::HOTMIC_SHOWS_ON_THE_TASKBAR.t(), h.is_some(), edit(cx, |s, on: bool| s.m.config.hotmic = on.then(Hotmic::default)))];
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
                row("hotmic.position", h.position != d.position, reset!(position), choice(cx, &h.position, copy::MIC_POSITIONS, mic!(position))),
                wide_row("hotmic.ignore", !h.ignore.is_empty(), Some(act(cx, |s| {
                    if let Some(h) = s.m.config.hotmic.as_mut() {
                        h.ignore.clear();
                    }
                })), list(cx, &h.ignore, ListAt::HotmicIgnore, s::HOTMIC_FOR_EXAMPLE_OBS64_EXE.t(), false)),
            ]);
        }
        StackPanel::new().spacing(4.0).children((heading(copy::HOTMIC), group_reset(cx, s::HOTMIC_ALERT.t(), Section::Hotmic, self.m.at_default(Section::Hotmic), cards)))
    }

    /// When PimpMyShell gets out of the way: for a game, for the frame rate, on battery, and Twatch TV's pictures instead of video.
    fn power(&self, cx: &Cx) -> View {
        let (c, b) = (&self.m.config, &self.m.baseline);
        let (games, shipped_games) = (c.rest_for_games, b.rest_for_games);
        let (covered, shipped_covered) = (c.rest_when_covered, b.rest_when_covered);
        let unseen = vec![row("rest_when_covered", covered != shipped_covered, Some(act(cx, move |s| s.m.config.rest_when_covered = shipped_covered)), toggle(cx, covered, |s, v| s.m.config.rest_when_covered = v))];
        let resting = vec![row("rest_for_games", games != shipped_games, Some(act(cx, move |s| s.m.config.rest_for_games = shipped_games)), toggle(cx, games, |s, v| s.m.config.rest_for_games = v))];
        let (max, shipped_max, battery, shipped_battery) = (c.max_fps, b.max_fps, c.battery_fps, b.battery_fps);
        let frames = vec![
            row(
                "max_fps",
                max != shipped_max,
                Some(act(cx, move |s| s.m.config.max_fps = shipped_max)),
                StackPanel::new().orientation(Orientation::Horizontal).spacing(12.0).children((
                    match max {
                        Some(f) => number(cx, f64::from(f), 0.0, 240.0, "fps", |s, v| s.m.config.max_fps = Some(v as u32)),
                        None => TextBlock::new().into(),
                    },
                    toggle(cx, max.is_some(), |s, on| s.m.config.max_fps = on.then_some(30)),
                )),
            ),
            row(
                "battery_fps",
                battery != shipped_battery,
                Some(act(cx, move |s| s.m.config.battery_fps = shipped_battery)),
                number(cx, f64::from(battery.unwrap_or(30)), 0.0, 240.0, "fps", |s, v| {
                    let v = v as u32;
                    s.m.config.battery_fps = (v != 30).then_some(v);
                }),
            ),
        ];
        // Twatch TV's, read without making it a section of the settings: that's only for changing one.
        let (t, d) = (c.twatch.clone().unwrap_or_default(), b.twatch.clone().unwrap_or_default());
        let (pictures, metered) = (d.pictures.clone(), d.metered_pictures);
        let twatch = vec![
            row("twatch.pictures", t.pictures != pictures, Some(act(cx, move |s| s.m.twatch().pictures = pictures.clone())), choice(cx, &t.pictures, &copy::TWATCH_PICTURES, |s, v| s.m.twatch().pictures = v)),
            row("twatch.metered_pictures", t.metered_pictures != metered, Some(act(cx, move |s| s.m.twatch().metered_pictures = metered)), toggle(cx, t.metered_pictures, |s, v| s.m.twatch().metered_pictures = v)),
        ];
        // Efficiency mode: a switch for each of PimpMyShell's kinds of process, and when for the PIMPBOX.
        let (e, shipped) = (c.efficiency.clone().unwrap_or_default(), b.efficiency.clone().unwrap_or_default());
        let switch = |id: &str, now: bool, was: bool, set: fn(&mut pms::config::Efficiency, bool)| {
            row(id, now != was, Some(act(cx, move |s| set(s.m.efficiency(), was))), toggle(cx, now, move |s, v| set(s.m.efficiency(), v)))
        };
        let shipped_pimpbox = shipped.pimpbox.clone();
        let efficient = vec![
            switch("efficiency.daemon", e.daemon, shipped.daemon, |e, v| e.daemon = v),
            switch("efficiency.twatch", e.twatch, shipped.twatch, |e, v| e.twatch = v),
            switch("efficiency.helpers", e.helpers, shipped.helpers, |e, v| e.helpers = v),
            row("efficiency.pimpbox", e.pimpbox != shipped.pimpbox, Some(act(cx, move |s| s.m.efficiency().pimpbox = shipped_pimpbox.clone())), choice(cx, &e.pimpbox, &copy::EFFICIENCY_PIMPBOX, |s, v| s.m.efficiency().pimpbox = v)),
        ];
        let note = |text: &str| TextBlock::new().text(text.to_string()).text_wrapping(TextWrapping::Wrap).font_size(12.0).opacity(0.7);
        StackPanel::new().spacing(4.0).children((
            heading_reset(cx, copy::POWER, Section::Performance, self.m.at_default(Section::Performance)),
            group(s::POWER_WHEN_NOBODY_CAN_SEE.t(), unseen),
            group(s::POWER_WHILE_YOU_PLAY.t(), resting),
            group(s::POWER_ANIMATIONS.t(), frames),
            group(s::POWER_TWATCH_TV.t(), twatch),
            group(s::POWER_EFFICIENCY.t(), efficient),
            note(s::POWER_EFFICIENCY_NOTE.t()),
            note(copy::POWER_BY_ITSELF.t()),
        ))
    }

    /// The logs: where they are, the last thing written, and what they can say about you.
    fn logs(&self, cx: &Cx) -> View {
        let m = &self.m;
        let files = vec![
            settings_card(Some("\u{E81C}"), s::LOGS_LOGS.t(), &pms::shown(&pms_tap::logfile::dir()), None, Button::new().on_click(act(cx, |s| s.m.show_logs())).content(s::LOGS_SHOW_IN_FILE_EXPLORER.t())),
            settings_card(None, s::LOGS_LAST_LOG_ENTRY.t(), &m.last_log, None, TextBlock::new()),
        ];
        let privacy: View = InfoBar::new()
            .title(copy::LOG_PRIVACY.0.t())
            .message(format!("{}\n\n{}", copy::LOG_PRIVACY.1.t(), copy::log_contents()))
            .severity(InfoBarSeverity::Warning)
            .is_open(true)
            .is_closable(false)
            .margin(Thickness::new(0.0, 8.0, 0.0, 0.0))
            .into();
        StackPanel::new().spacing(4.0).children((heading(copy::LOGS), group(s::LOGS_YOUR_LOGS.t(), files), privacy))
    }

    /// What's along the taskbar, in its order: the meters, together; each TV and Twatch TV; and the Hot Mic's label at an end.
    fn widgets(&self, cx: &Cx) -> View {
        let meters = &self.m.config.meters;
        let mut rows: Vec<View> = Vec::new();
        for (at, w) in self.m.widgets().into_iter().enumerate() {
            let arrows = (self.m.can_move_widget(at, true), self.m.can_move_widget(at, false));
            rows.push(match w {
                Widget::Meters => {
                    let names: Vec<&str> = self.m.plain_meters().iter().map(|&i| meters[i].label.as_str()).collect();
                    widget_row(cx, at, "\u{E9D9}", s::WIDGETS_METERS.t(), &names.join(", "), arrows)
                }
                Widget::Screen(i) if model::is_twatch(&meters[i]) => widget_row(cx, at, "\u{E714}", &meters[i].label, s::WIDGETS_TWATCH_TV.t(), arrows),
                Widget::Screen(i) => widget_row(cx, at, "\u{E7F4}", &meters[i].label, s::WIDGETS_TV.t(), arrows),
                Widget::Mic => widget_row(cx, at, "\u{E720}", s::WIDGETS_HOT_MIC.t(), if at == 0 { copy::WIDGET_MIC_FIRST.t() } else { copy::WIDGET_MIC_LAST.t() }, arrows),
            });
        }
        let note = |text: &str| TextBlock::new().text(text.to_string()).text_wrapping(TextWrapping::Wrap).font_size(12.0).opacity(0.7);
        let mut parts: Vec<View> = vec![heading(copy::WIDGETS)];
        parts.push(if rows.is_empty() { note(copy::WIDGETS_NONE.t()).into() } else { group(s::WIDGETS_LEFT_TO_RIGHT.t(), rows) });
        if self.m.config.hotmic.is_none() {
            parts.push(note(copy::WIDGETS_NO_MIC.t()).into());
        }
        parts.push(note(copy::ORDER_NOTE.t()).into());
        children(StackPanel::new().spacing(4.0), parts)
    }

    fn tv(&self, cx: &Cx) -> View {
        let sets = self.m.tvs().len();
        let tvs = settings_card(
            Some("\u{E7F4}"),
            s::TV_TVS_ON_YOUR_TASKBAR.t(),
            &if sets == 1 && pms::tv::console_running() {
                s::TV_YOU_HAVE_1_REMOVING.t().to_string()
            } else {
                s::TV_YOU_HAVE_SETS.with(&[("sets", &sets)])
            },
            None,
            StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
                Button::new().is_enabled(sets > 0).on_click(act(cx, |s| s.m.remove_tv())).content(s::TV_REMOVE_ONE.t()),
                Button::new().style(ButtonStyle::Accent).on_click(act(cx, |s| s.m.add_tv())).content(s::TV_ADD_A_TV.t()),
            )),
        );

        let (intro, shipped) = (self.m.config.tv_intro, self.m.baseline.tv_intro);
        let mut surf = vec![tvs, row("tv_intro", intro != shipped, Some(act(cx, move |s| s.m.config.tv_intro = shipped)), toggle(cx, intro, |s, v| s.m.config.tv_intro = v))];
        if intro {
            let (from, to) = (self.m.config.tv_intro_from, self.m.config.tv_intro_to);
            let (usual_from, usual_to) = (self.m.baseline.tv_intro_from, self.m.baseline.tv_intro_to);
            surf.push(row("tv_intro_from", from != usual_from, Some(act(cx, move |s| s.m.config.tv_intro_from = usual_from)), choice(cx, &from.to_string(), &copy::TV_CHANNELS, |s, v| s.m.config.tv_intro_from = v.parse().unwrap_or(pms::tv::INTRO_FROM))));
            surf.push(row("tv_intro_to", to != usual_to, Some(act(cx, move |s| s.m.config.tv_intro_to = usual_to)), choice(cx, &to.to_string(), &copy::TV_CHANNELS, |s, v| s.m.config.tv_intro_to = v.parse().unwrap_or(pms::tv::INTRO_TO))));
        }

        let (every, jitter) = (self.m.config.shuffle_seconds, self.m.config.shuffle_jitter);
        let (usual_every, usual_jitter) = (self.m.baseline.shuffle_seconds, self.m.baseline.shuffle_jitter);
        let shuffling = vec![
            row("shuffle_seconds", every != usual_every, Some(act(cx, move |s| s.m.config.shuffle_seconds = usual_every)), number(cx, f64::from(every), 5.0, 3600.0, "s", |s, v| s.m.config.shuffle_seconds = v as u32)),
            row("shuffle_jitter", jitter != usual_jitter, Some(act(cx, move |s| s.m.config.shuffle_jitter = usual_jitter)), toggle(cx, jitter, |s, v| s.m.config.shuffle_jitter = v)),
        ];
        let note = |text: &str| TextBlock::new().text(text.to_string()).text_wrapping(TextWrapping::Wrap).font_size(12.0).opacity(0.7);
        StackPanel::new().spacing(4.0).children((heading(copy::TV), group(s::TV_YOUR_TVS.t(), surf), group(s::TV_SHUFFLING.t(), shuffling), group(copy::TV_NUMBER_ONE.t(), self.number_one_rows(cx)), note(copy::TV_HELP.t()), note(copy::ORDER_HINT.t())))
    }

    /// Channel 37: whether it goes online, its bumps, and a notification when the station flushes. On the TV page, and on a page
    /// of its own.
    fn number_one_rows(&self, cx: &Cx) -> Vec<View> {
        let (iss, usual) = (self.m.config.iss.clone().unwrap_or_default(), pms::config::Iss::default());
        let mut rows = vec![row("iss.online", iss.online != usual.online, Some(act(cx, |s| s.m.iss().online = pms::config::Iss::default().online)), toggle(cx, iss.online, |s, v| s.m.iss().online = v))];
        if iss.online {
            rows.push(row("iss.bumps", iss.bumps != usual.bumps, Some(act(cx, |s| s.m.iss().bumps = pms::config::Iss::default().bumps)), choice(cx, &iss.bumps, &copy::ISS_BUMPS, |s, v| s.m.iss().bumps = v)));
            rows.push(row("iss.notify_flush", iss.notify_flush != usual.notify_flush, Some(act(cx, |s| s.m.iss().notify_flush = pms::config::Iss::default().notify_flush)), toggle(cx, iss.notify_flush, |s, v| s.m.iss().notify_flush = v)));
        }
        rows
    }

    fn number_one(&self, cx: &Cx) -> View {
        StackPanel::new().spacing(4.0).children((heading(copy::NUMBER_ONE), group(copy::TV_NUMBER_ONE.t(), self.number_one_rows(cx))))
    }

    /// The console on channel 304: which game is in it, more to get, and whether a TV tuning in switches it on.
    fn pimpbox(&self, cx: &Cx) -> View {
        use pms::wads::Wad;
        let wads = pms::wads::list();
        let playing = pms::wads::chosen().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        let job = self.m.wad_job.lock().unwrap().clone();
        let busy = matches!(job, WadJob::Getting(..));
        let have = |name: &str| wads.iter().any(|w| w.eq_ignore_ascii_case(name));

        // What's in the console: pick one.
        let mut games: Vec<View> = wads
            .iter()
            .map(|file| {
                let is_on = playing.as_deref() == Some(file.as_str());
                let (to_play, to_remove) = (file.clone(), file.clone());
                settings_card(
                    None,
                    &pms::wads::title(file),
                    if is_on { s::PIMPBOX_IN_THE_CONSOLE_NOW.t() } else { file },
                    None,
                    StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
                        Button::new().on_click(act(cx, move |s| s.m.remove_wad(&to_remove))).content(s::PIMPBOX_REMOVE.t()),
                        Button::new().is_enabled(!is_on).style(ButtonStyle::Accent).on_click(act(cx, move |s| s.m.choose_wad(&to_play))).content(if is_on { s::PIMPBOX_PLAYING.t() } else { s::PIMPBOX_PLAY_THIS.t() }),
                    )),
                )
            })
            .collect();
        if games.is_empty() {
            games.push(card(TextBlock::new().text(s::PIMPBOX_NO_GAMES_YET_GET.t()).text_wrapping(TextWrapping::Wrap)));
        }

        // More games.
        let mut more: Vec<View> = Vec::new();
        if !have("freedoom1.wad") {
            more.push(download_card("Freedoom", s::PIMPBOX_FREE_WITH_ITS_OWN.t(), pms::wads::url(Wad::Freedoom), Button::new().is_enabled(!busy).on_click(act(cx, |s| s.m.get_wad(Wad::Freedoom))).content(s::PIMPBOX_DOWNLOAD.t())));
        }
        if !have("doom1.wad") {
            more.push(download_card(s::PIMPBOX_DOOM_SHAREWARE_EPISODE.t(), s::PIMPBOX_THE_FIRST_EPISODE_UNMODIFIED.t(), pms::wads::url(Wad::Shareware), Button::new().is_enabled(!busy).on_click(act(cx, |s| s.m.get_wad(Wad::Shareware))).content(s::PIMPBOX_DOWNLOAD.t())));
        }
        more.push(settings_card(None, s::PIMPBOX_A_GAME_OF_YOUR.t(), s::PIMPBOX_PUT_AN_IWAD_LIKE.t(), None, Button::new().on_click(act(cx, |s| s.m.show_wads())).content(s::PIMPBOX_OPEN_FOLDER.t())));
        match job {
            WadJob::Getting(done, total) => more.push(card(StackPanel::new().spacing(6.0).children((
                TextBlock::new().text(if total == 0 { s::PIMPBOX_WAITING_FOR_YOU.t().to_string() } else { s::PIMPBOX_DOWNLOADING.with(&[("done", &(done / 1024)), ("total", &(total / 1024))]) }),
                ProgressBar::new().minimum(0.0).maximum(total.max(1) as f64).value(done as f64).is_indeterminate(total == 0),
            )))),
            WadJob::Failed(e) => more.push(card(TextBlock::new().text(s::PIMPBOX_DIDNT_WORK.with(&[("why", &e)])).text_wrapping(TextWrapping::Wrap))),
            WadJob::Idle => {}
        }

        let auto = self.m.config.console_autostart;
        let shipped = self.m.baseline.console_autostart;
        let (video, usual_video) = (self.m.config.console_video.clone(), self.m.baseline.console_video.clone());
        let demos_on = self.m.config.console_demos;
        let console = vec![
            row("console_autostart", auto != shipped, Some(act(cx, move |s| s.m.config.console_autostart = shipped)), toggle(cx, auto, |s, v| s.m.config.console_autostart = v)),
            row("console_video", video != usual_video, Some(act(cx, move |s| s.m.config.console_video = usual_video.clone())), choice(cx, &video, copy::CONSOLE_VIDEO, |s, v| s.m.config.console_video = v)),
            row("console_demos", demos_on, Some(act(cx, |s| s.m.config.console_demos = false)), toggle(cx, demos_on, |s, v| s.m.config.console_demos = v)),
            settings_card(Some("\u{E74D}"), copy::CLEAR_SHADERS.0.t(), copy::CLEAR_SHADERS.1.t(), None, Button::new().on_click(act(cx, |s| s.confirm_shaders = true)).content(s::PIMPBOX_CLEAR.t())),
        ];
        let note = |text: &str| TextBlock::new().text(text.to_string()).text_wrapping(TextWrapping::Wrap).font_size(12.0).opacity(0.7);
        let demos = if demos_on {
            // How much the archive's asked of, and when: the settings for that, then what's come of it.
            let (d, usual) = (self.m.config.demos(), pms::config::Demos::default());
            let mut cards = vec![
                row("demos.when", d.when != usual.when, Some(act(cx, |s| s.m.demos().when = pms::config::Demos::default().when)), choice(cx, &d.when, copy::DEMOS_WHEN, |s, v| s.m.demos().when = v)),
                row("demos.every_hours", d.every_hours != usual.every_hours, Some(act(cx, move |s| s.m.demos().every_hours = usual.every_hours)), number(cx, f64::from(d.every_hours), 1.0, 168.0, "h", |s, v| s.m.demos().every_hours = v as u32)),
                row("demos.at_once", d.at_once != usual.at_once, Some(act(cx, move |s| s.m.demos().at_once = usual.at_once)), number(cx, f64::from(d.at_once), 1.0, 50.0, "", |s, v| s.m.demos().at_once = v as u32)),
                row("demos.gap_seconds", d.gap_seconds != usual.gap_seconds, Some(act(cx, move |s| s.m.demos().gap_seconds = usual.gap_seconds)), number(cx, f64::from(d.gap_seconds), 1.0, 60.0, "s", |s, v| s.m.demos().gap_seconds = v as u32)),
                row("demos.most", d.most != usual.most, Some(act(cx, move |s| s.m.demos().most = usual.most)), number(cx, f64::from(d.most), 1.0, 500.0, "", |s, v| s.m.demos().most = v as u32)),
                row("demos.list_days", d.list_days != usual.list_days, Some(act(cx, move |s| s.m.demos().list_days = usual.list_days)), number(cx, f64::from(d.list_days), 1.0, 90.0, "", |s, v| s.m.demos().list_days = v as u32)),
                row("demos.fired_at_least", d.fired_at_least != usual.fired_at_least, Some(act(cx, move |s| s.m.demos().fired_at_least = usual.fired_at_least)), number(cx, f64::from(d.fired_at_least), 0.0, 1000.0, "", |s, v| s.m.demos().fired_at_least = v as u32)),
            ];
            cards.extend(demo_cards(d.fired_at_least));
            group(s::PIMPBOX_DEMOS.t(), cards)
        } else {
            Border::new().into()
        };
        StackPanel::new().spacing(4.0).children((heading(copy::PIMPBOX), missing_part(pms::parts::PIMPBOX), missing_part(pms::parts::POOM), group(s::PIMPBOX_WHICH_GAME_PLAYS.t(), games), group(s::PIMPBOX_THE_CONSOLE.t(), console), demos, group(s::PIMPBOX_GET_MORE_GAMES.t(), more), note(copy::PIMPBOX_HELP.t()), note(s::TV_NOTICE.t())))
    }

    /// What's wrong with the settings file, where, and the ways out, above every page.
    fn problem_banner(&self, cx: &Cx) -> View {
        let Some(p) = &self.m.problem else { return self.ignored_banner(cx) };
        let name = self.m.path.file_name().map_or(s::PROBLEM_BANNER_YOUR_SETTINGS_FILE.t().into(), |n| n.to_string_lossy().into_owned());
        let place = p.line.map_or(String::new(), |n| s::PROBLEM_BANNER_LINE.with(&[("line", &n)]));
        let source = p.source.clone().unwrap_or_default();
        let what = [Some(p.what.clone()), p.hint.clone()].into_iter().flatten().collect::<Vec<_>>().join(" ");
        let restore = self.m.has_last_good();
        let lines: [View; 5] = [
            InfoBar::new().title(s::PROBLEM_BANNER_PROBLEM_IN.with(&[("name", &name)])).message(s::PROBLEM_BANNER_YOUR_FILE_HASNT_BEEN.t()).severity(InfoBarSeverity::Error).is_open(true).is_closable(false).into(),
            TextBlock::new().text(place).font_size(12.0).opacity(0.7).into(),
            TextBlock::new().text(source).text_wrapping(TextWrapping::Wrap).into(),
            TextBlock::new().text(what).text_wrapping(TextWrapping::Wrap).into(),
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .children((
                    Button::new().style(ButtonStyle::Accent).on_click(act(cx, |s| s.m.open_config())).content(s::PROBLEM_BANNER_OPEN_SETTINGS_FILE.t()),
                    Button::new().on_click(act(cx, |s| s.m.show_config())).content(s::PROBLEM_BANNER_SHOW_IN_FILE_EXPLORER.t()),
                    Button::new().is_enabled(restore).on_click(act(cx, |s| s.m.restore_last_good())).content(s::PROBLEM_BANNER_RESTORE_LAST_WORKING_SETTINGS.t()),
                )),
        ];
        Border::new().grid_row(0).padding(Thickness::new(36.0, 16.0, 36.0, 12.0)).content(StackPanel::new().max_width(1000.0).spacing(6.0).children(lines))
    }

    /// Lines of the file that aren't settings: left out, with everything else loaded, and said in yellow, above every page.
    fn ignored_banner(&self, cx: &Cx) -> View {
        let issues = self.m.issues();
        if issues.is_empty() {
            return Border::new().grid_row(0).into();
        }
        let name = self.m.path.file_name().map_or(s::IGNORED_BANNER_YOUR_SETTINGS_FILE.t().into(), |n| n.to_string_lossy().into_owned());
        // A value that couldn't be used is more than a line nobody knows: red, as its setting is.
        let unusable = self.m.config.issues.iter().any(|e| !pms::config::issue_is_unknown(e));
        let (title, severity) = if unusable { (s::IGNORED_BANNER_UNUSABLE.with(&[("name", &name)]), InfoBarSeverity::Error) } else { (s::IGNORED_BANNER_NOT_SETTINGS.with(&[("name", &name)]), InfoBarSeverity::Warning) };
        let mut lines: Vec<View> = vec![InfoBar::new().title(title).message(copy::IGNORED_LINES.t()).severity(severity).is_open(true).is_closable(false).into()];
        for (_, on, words) in issues {
            let page = PAGES.iter().find(|(_, tag, ..)| *tag == on).map_or(Page::General, |(p, ..)| *p);
            let name = PAGES.iter().find(|(_, tag, ..)| *tag == on).map_or(copy::GENERAL.0.t(), |(.., (name, _))| name.t());
            lines.push(
                Grid::new()
                    .columns([GridLength::STAR, GridLength::Auto])
                    .column_spacing(12.0)
                    .children((
                        TextBlock::new().grid_column(0).text(words).text_wrapping(TextWrapping::Wrap).font_size(12.0),
                        Button::new().grid_column(1).vertical_alignment(VerticalAlignment::Center).on_click(act(cx, move |s| s.page = page)).content(s::IGNORED_BANNER_GO_TO.with(&[("name", &name)])),
                    )),
            );
        }
        lines.push(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(8.0)
                .children((Button::new().on_click(act(cx, |s| s.m.open_config())).content(s::IGNORED_BANNER_OPEN_SETTINGS_FILE.t()), Button::new().on_click(act(cx, |s| s.m.show_config())).content(s::IGNORED_BANNER_SHOW_IN_FILE_EXPLORER.t()))),
        );
        Border::new().grid_row(0).padding(Thickness::new(36.0, 16.0, 36.0, 12.0)).content(children(StackPanel::new().max_width(1000.0).spacing(6.0), lines))
    }

    /// Your favorites, as a list to put in order, or as text. A name only becomes a favorite when it's added or saved: one typed
    /// halfway could be somebody else's channel.
    fn favorites(&self, cx: &Cx, list: &[String]) -> View {
        let note = self.m.favorites_note.clone();
        let about = if self.m.favorites_text.is_some() { copy::TWATCH_FAVORITES.t().to_string() } else { format!("{} {}", copy::TWATCH_FAVORITES.t(), copy::TWATCH_DRAG.t()) };
        let mut parts: Vec<View> = vec![TextBlock::new().text(about).text_wrapping(TextWrapping::Wrap).font_size(12.0).opacity(0.7).into()];
        if let Some(text) = &self.m.favorites_text {
            parts.push(TextBlock::new().text(copy::TWATCH_AS_TEXT.t()).text_wrapping(TextWrapping::Wrap).font_size(12.0).opacity(0.7).into());
            // A new box is given its text before it's told it may have more than one line, and keeps only the first, which saving
            // would make your whole list. So it starts empty, and gets the text a moment later, knowing what it is by then.
            let ready = self.box_ready;
            let on_change = edit(cx, |s, v: String| {
                if s.box_ready {
                    s.m.favorites_text = Some(v);
                }
            });
            parts.push(TextBox::new().text(if ready { text.as_str() } else { "" }).accepts_return(true).text_wrapping(TextWrapping::Wrap).height(220.0).on_text_changed(on_change).into());
            parts.push(StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
                Button::new().is_enabled(ready).style(ButtonStyle::Accent).on_click(act(cx, |s| s.m.save_favorites_text())).content(s::FAVORITES_SAVE.t()),
                Button::new().on_click(act(cx, |s| s.m.cancel_favorites_text())).content(s::FAVORITES_CANCEL.t()),
            )));
        } else {
            // A list whose rows can be dragged into another order, each with its arrows as well: for the keyboard, and for a step.
            let mut rows = Vec::new();
            for (i, handle) in list.iter().enumerate() {
                let swap = move |s: &mut Settings, other: usize| s.m.swap_favorites(i, other);
                let line = Grid::new().columns([GridLength::Pixel(28.0), GridLength::Pixel(230.0), GridLength::Auto]).children((
                    TextBlock::new().grid_column(0).text(format!("{}", i + 1)).opacity(0.6).vertical_alignment(VerticalAlignment::Center),
                    TextBlock::new().grid_column(1).text(handle.clone()).vertical_alignment(VerticalAlignment::Center),
                    StackPanel::new().grid_column(2).orientation(Orientation::Horizontal).spacing(8.0).children((
                        Button::new().is_enabled(i > 0).on_click(act(cx, move |s| swap(s, i.wrapping_sub(1)))).content("\u{25B2}").tooltip(s::FAVORITES_HIGHER.t()),
                        Button::new().is_enabled(i + 1 < list.len()).on_click(act(cx, move |s| swap(s, i + 1))).content("\u{25BC}").tooltip(s::FAVORITES_LOWER.t()),
                        Button::new()
                            .on_click(act(cx, move |s| {
                                let l = &mut s.m.twatch().favorites;
                                if i < l.len() {
                                    l.remove(i);
                                }
                            }))
                            .content(SymbolIcon::new().symbol(Symbol::Delete))
                            .tooltip(s::FAVORITES_REMOVE.t()),
                    )),
                ));
                // A row is known by its channel, and by which of them it is should a channel be there twice.
                let again = list[..i].iter().filter(|h| *h == handle).count();
                rows.push(KeyedView::new(format!("{handle} {again}"), ListViewItem::new().tag(handle.clone()).content(line)));
            }
            if !rows.is_empty() {
                let dragged = edit(cx, |s, order: Vec<String>| s.m.reorder_favorites(order));
                // A row keeps room at its left for a mark this list hasn't: the list starts that much further left.
                let list = ListView::new().selection_mode(ListViewSelectionMode::None).can_drag_items(true).can_reorder_items(true).allow_drop(true).on_reordered(dragged);
                parts.push(list.margin(Thickness::new(-16.0, 0.0, 0.0, 0.0)).slots([SlotView::collection(ListViewSlot::Items, rows)]));
            }
            parts.push(StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
                text(cx, &self.m.new_favorite, s::FAVORITES_A_CHANNELS_NAME_OR.t(), 258.0, |s, v| s.m.new_favorite = v),
                Button::new().is_enabled(!self.m.new_favorite.trim().is_empty()).on_click(act(cx, |s| s.m.add_favorite())).content(s::FAVORITES_ADD.t()),
                Button::new().on_click(act(cx, |s| s.m.favorites_as_text())).content(s::FAVORITES_EDIT_AS_TEXT.t()),
            )));
        }
        if !note.is_empty() {
            parts.push(TextBlock::new().text(note).text_wrapping(TextWrapping::Wrap).font_size(12.0).foreground(Brush::Theme(ThemeBrush::SystemCritical)).into());
        }
        card(children(StackPanel::new().spacing(8.0), parts))
    }

    fn twatch(&self, cx: &Cx) -> View {
        let on = self.m.twatch_on();
        let t = self.m.config.twatch.clone().unwrap_or_default();
        let job = self.m.streamlink_job.lock().unwrap().clone();
        let busy = matches!(job, WadJob::Getting(..));
        let have = pms::streamlink::exe().is_some();

        let set = settings_card(
            Some("\u{E714}"),
            s::TWATCH_TWATCH_TV_ON_YOUR.t(),
            s::TWATCH_A_16_9_SET.t(),
            None,
            toggle(cx, on, |s, v| s.m.set_twatch_on(v)),
        );
        let favorites = self.favorites(cx, &t.favorites);
        // What ships, which Reset puts back.
        let d = self.m.baseline.twatch.clone().unwrap_or_default();
        let (top, more, more_on) = (d.top_quality.clone(), d.more_sets, d.more_sets_on);
        let (notify, reset_top) = (d.notify_live, top.clone());
        let (volume, fps) = (d.volume, d.frame_rate);
        let (higher, check, check_extra) = (d.higher_favorite.clone(), d.check_seconds, d.check_extra_seconds);
        let reset_higher = higher.clone();
        let (media, reset_media) = (d.media_controls.clone(), d.media_controls.clone());
        let playing = vec![
            row("twatch.higher_favorite", t.higher_favorite != higher, Some(act(cx, move |s| s.m.twatch().higher_favorite = reset_higher.clone())), choice(cx, &t.higher_favorite, &copy::TWATCH_HIGHER, |s, v| s.m.twatch().higher_favorite = v)),
            row("twatch.check_seconds", t.check_seconds != check, Some(act(cx, move |s| s.m.twatch().check_seconds = check)), choice(cx, &t.check_seconds.to_string(), &copy::TWATCH_CHECKS, |s, v| s.m.twatch().check_seconds = v.parse().unwrap_or(pms::twatch::live::USUAL_CHECK))),
            row("twatch.check_extra_seconds", t.check_extra_seconds != check_extra, Some(act(cx, move |s| s.m.twatch().check_extra_seconds = check_extra)), choice(cx, &t.check_extra_seconds.to_string(), &copy::TWATCH_CHECK_EXTRAS, |s, v| s.m.twatch().check_extra_seconds = v.parse().unwrap_or(pms::twatch::live::USUAL_CHECK_EXTRA))),
            row("twatch.notify_live", t.notify_live != notify, Some(act(cx, move |s| s.m.twatch().notify_live = notify)), toggle(cx, t.notify_live, |s, v| s.m.twatch().notify_live = v)),
            row("twatch.volume", t.volume != volume, Some(act(cx, move |s| s.m.twatch().volume = volume)), number(cx, f64::from(t.volume), 0.0, 100.0, "%", |s, v| s.m.twatch().volume = v as u32)),
            row("twatch.top_quality", t.top_quality != top, Some(act(cx, move |s| s.m.twatch().top_quality = reset_top.clone())), choice(cx, &t.top_quality, &copy::TWATCH_QUALITIES, |s, v| s.m.twatch().top_quality = v)),
            row("twatch.frame_rate", t.frame_rate != fps, Some(act(cx, move |s| s.m.twatch().frame_rate = fps)), choice(cx, &t.frame_rate.to_string(), &copy::TWATCH_FRAME_RATES, move |s, v| s.m.twatch().frame_rate = v.parse().unwrap_or(fps))),
            row("twatch.media_controls", t.media_controls != media, Some(act(cx, move |s| s.m.twatch().media_controls = reset_media.clone())), choice(cx, &t.media_controls, &copy::TWATCH_MEDIA_CONTROLS, |s, v| s.m.twatch().media_controls = v)),
            row("twatch.more_sets_on", t.more_sets_on != more_on, Some(act(cx, move |s| s.m.twatch().more_sets_on = more_on)), toggle(cx, t.more_sets_on, |s, v| s.m.twatch().more_sets_on = v)),
            row("twatch.more_sets", t.more_sets != more, Some(act(cx, move |s| s.m.twatch().more_sets = more)), number(cx, f64::from(t.more_sets), 0.0, f64::from(pms::twatch::MOST_MORE_SETS), "", |s, v| s.m.twatch().more_sets = v as u32)),
        ];

        let note_now = self.m.streamlink_note.lock().unwrap().clone();
        let mut stream = vec![if have {
            let version = pms::streamlink::version().unwrap_or_default();
            settings_card(
                None,
                &s::TWATCH_STREAMLINK_INSTALLED.with(&[("version", &version)]),
                &s::TWATCH_STREAMLINK_FROM.with(&[("from", &pms::streamlink::URL), ("note", &note_now)]),
                None,
                StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
                    Button::new().is_enabled(!busy).on_click(act(cx, |s| s.m.remove_streamlink())).content(s::TWATCH_REMOVE.t()),
                    Button::new().is_enabled(!busy).on_click(act(cx, |s| s.m.get_streamlink())).content(s::TWATCH_GET_THE_NEWEST.t()),
                )),
            )
        } else {
            settings_card(None, "Streamlink", &s::TWATCH_STREAMLINK_NEEDED.with(&[("from", &pms::streamlink::URL)]), None, Button::new().is_enabled(!busy).style(ButtonStyle::Accent).on_click(act(cx, |s| s.m.get_streamlink())).content(s::TWATCH_DOWNLOAD.t()))
        }];
        match job {
            WadJob::Getting(done, total) => stream.push(card(StackPanel::new().spacing(6.0).children((
                TextBlock::new().text(s::TWATCH_DOWNLOADING.with(&[("done", &(done >> 20)), ("total", &(total >> 20))])),
                ProgressBar::new().minimum(0.0).maximum(total.max(1) as f64).value(done as f64).is_indeterminate(total == 0),
            )))),
            WadJob::Failed(e) => stream.push(card(TextBlock::new().text(s::TWATCH_DIDNT_WORK.with(&[("why", &e)])).text_wrapping(TextWrapping::Wrap))),
            WadJob::Idle => {}
        }
        let note = |text: &str| TextBlock::new().text(text.to_string()).text_wrapping(TextWrapping::Wrap).font_size(12.0).opacity(0.7);
        // Favorites and no way to play them: the first thing on the page says so, until there's a way.
        let stuck = on && !have && !busy && !t.favorites.is_empty() && t.pictures != "always";
        let (title, words) = copy::TWATCH_NO_STREAMLINK;
        let warning: View = if stuck { InfoBar::new().title(title.t()).message(words.t()).severity(InfoBarSeverity::Warning).is_open(true).is_closable(false).into() } else { Border::new().into() };
        StackPanel::new().spacing(4.0).children((heading(copy::TWATCH), missing_part(pms::parts::TWATCH), warning, group("Streamlink", stream), group(s::TWATCH_YOUR_TWATCH_TV.t(), vec![set]), group(s::TWATCH_FAVORITES_2.t(), vec![favorites]), group_reset(cx, s::TWATCH_HOW_IT_PLAYS.t(), Section::Twatch, self.m.at_default(Section::Twatch), playing), note(copy::TWATCH_NETWORK.t()), note(copy::TWATCH_POWER.t()), note(copy::ORDER_HINT.t()), note(copy::TWATCH_NOTICE.t())))
    }

    /// The proxy, and what goes through it: every way PimpMyShell uses the network, each with its switch.
    fn network(&self, cx: &Cx) -> View {
        use pms::net::Use;
        let n = self.m.config.network.clone().unwrap_or_default();
        let typed = self.m.proxy_text.trim().to_string();
        let wrong = (!typed.is_empty()).then(|| pms::net::check(&typed).err()).flatten();
        // While the file won't load there's no proxy to compare with: only one typed in can replace it.
        let applied = if n.proxy == pms::config::NETWORK_UNREAD { "" } else { n.proxy.as_str() };
        let pending = typed != applied;
        let proxy_card = settings_card(
            Some("\u{E968}"),
            copy::PROXY.0.t(),
            copy::PROXY.1.t(),
            Some("proxy"),
            StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
                text(cx, &self.m.proxy_text, "socks5h://127.0.0.1:9050", 250.0, |s, v| s.m.proxy_text = v),
                Button::new().style(ButtonStyle::Accent).is_enabled(pending && wrong.is_none()).on_click(act(cx, |s| s.m.apply_proxy())).content(if typed.is_empty() && pending { s::NETWORK_REMOVE.t() } else { s::NETWORK_APPLY.t() }),
                Button::new().is_enabled(!applied.is_empty() && !pending).on_click(act(cx, |s| s.m.test_proxy())).content(s::NETWORK_TEST.t()),
            )),
        );
        let mut proxy = vec![proxy_card];
        if let Some(e) = pms::config::network_trouble(&self.m.path) {
            let file = pms::config::network_path(&self.m.path);
            proxy.insert(0, InfoBar::new().title(s::NETWORK_YOUR_NETWORK_SETTINGS_DIDNT.t()).message(copy::NETWORK_UNREAD.t()).severity(InfoBarSeverity::Error).is_open(true).is_closable(false).into());
            proxy.insert(1, TextBlock::new().text(pms::problem::Problem::of(&file, &e).words()).text_wrapping(TextWrapping::Wrap).font_size(12.0).margin(Thickness::new(4.0, 2.0, 4.0, 2.0)).into());
            proxy.insert(2, Button::new().on_click(act(cx, |s| s.m.show_network())).content(s::NETWORK_SHOW_IN_FILE_EXPLORER.t()));
        }
        if !n.proxy.is_empty() && !self.m.uses_proxy() {
            proxy.push(InfoBar::new().title(copy::PROXY_UNUSED.0.t()).message(copy::PROXY_UNUSED.1.t()).severity(InfoBarSeverity::Warning).is_open(true).is_closable(false).into());
        }
        let said = self.m.proxy_note.lock().unwrap().clone();
        for (line, bad) in [(wrong.unwrap_or_default(), true), (said, false)] {
            if !line.is_empty() {
                let words = TextBlock::new().text(line).text_wrapping(TextWrapping::Wrap).font_size(12.0).margin(Thickness::new(4.0, 2.0, 4.0, 0.0));
                proxy.push(if bad { words.foreground(Brush::Theme(ThemeBrush::SystemCritical)).into() } else { words.opacity(0.8).into() });
            }
        }
        proxy.push(settings_card(None, copy::PROXY_ISOLATE.0.t(), copy::PROXY_ISOLATE.1.t(), Some("isolate"), toggle(cx, n.isolate, |s, v| s.m.network().isolate = v)));
        proxy.push(InfoBar::new().title(s::NETWORK_NOTHING_IS_AIRTIGHT.t()).message(copy::PROXY_LIMITS.t()).severity(InfoBarSeverity::Informational).is_open(true).is_closable(false).into());
        let on = |what: Use| match what {
            Use::Live => n.live_checks,
            Use::Streams | Use::Video => n.streams,
            Use::Updates => n.updates,
            Use::Downloads => n.downloads,
            Use::Iss => n.iss,
        };
        let all_on = pms::net::USES.iter().all(|(what, ..)| on(*what));
        let head = Grid::new().columns([GridLength::STAR, GridLength::Auto]).margin(Thickness::new(1.0, 24.0, 0.0, 2.0)).children((
            TextBlock::new().grid_column(0).text(s::NETWORK_WHAT_PIMPMYSHELL_USES_THE.t()).font_weight(FontWeight::SEMI_BOLD).vertical_alignment(VerticalAlignment::Center),
            Button::new().grid_column(1).on_click(act(cx, move |s| s.m.all_through_proxy(!all_on))).content(if all_on { s::NETWORK_TURN_OFF_ALL.t() } else { s::NETWORK_TURN_ON_ALL.t() }),
        ));
        let notes = self.m.use_notes.lock().unwrap().clone();
        let failed = self.m.use_failed.lock().unwrap().clone();
        let mut uses: Vec<View> = vec![head];
        // Grouped by what they're part of, in the order they first come.
        let mut groups = Vec::new();
        for (_, _, who, _) in &pms::net::USES {
            if !groups.contains(who) {
                groups.push(*who);
            }
        }
        for who in groups {
            uses.push(TextBlock::new().text(who.t()).font_size(13.0).opacity(0.8).margin(Thickness::new(1.0, 12.0, 0.0, 2.0)).into());
            for (at, (what, title, _, words)) in pms::net::USES.iter().enumerate().filter(|(_, u)| u.2 == who) {
                let (title, words) = (title.t(), words.t());
                let what = *what;
                let switch = toggle(cx, on(what), move |s, v| {
                    let n = s.m.network();
                    match what {
                        Use::Live => n.live_checks = v,
                        Use::Streams | Use::Video => n.streams = v,
                        Use::Updates => n.updates = v,
                        Use::Downloads => n.downloads = v,
                        Use::Iss => n.iss = v,
                    }
                });
                let controls = StackPanel::new().orientation(Orientation::Horizontal).spacing(10.0).children((
                    Button::new().is_enabled(notes[at] != s::MODEL_TRYING.t()).on_click(act(cx, move |s| s.m.try_use(at))).content(s::NETWORK_TEST.t()),
                    TextBlock::new().text(s::NETWORK_USE_PROXY.t()).opacity(if n.proxy.is_empty() { 0.5 } else { 0.8 }).vertical_alignment(VerticalAlignment::Center),
                    switch,
                ));
                uses.push(settings_card(None, title, words, None, controls));
                uses.push(done_by(what));
                uses.push(login_view(&n, what));
                if !notes[at].is_empty() {
                    let bad = failed[at];
                    let line = TextBlock::new().text(notes[at].clone()).text_wrapping(TextWrapping::Wrap).font_size(12.0).margin(Thickness::new(4.0, 0.0, 4.0, 4.0));
                    uses.push(if bad { line.foreground(Brush::Theme(ThemeBrush::SystemCritical)).into() } else { line.opacity(0.8).into() });
                }
            }
        }
        let note = |text: &str| TextBlock::new().text(text.to_string()).text_wrapping(TextWrapping::Wrap).font_size(12.0).opacity(0.7);
        StackPanel::new().spacing(4.0).children((heading(copy::NETWORK), group(s::NETWORK_PROXY.t(), proxy), children(StackPanel::new().spacing(4.0), uses), note(copy::PROXY_STRICT.t()), note(copy::NETWORK_ELSE.t())))
    }

    /// Using protection: a switch for each helper's sandbox, what it keeps the helper from, and what went wrong, if anything did.
    fn sandbox(&self, cx: &Cx) -> View {
        use pms::sandbox::Helper;
        let wanted = self.m.config.sandbox.clone().unwrap_or_default();
        let mut cards: Vec<View> = Vec::new();
        for helper in Helper::ALL {
            let id = format!("sandbox.{}", helper.key());
            let on = wanted.on(helper);
            let set = move |s: &mut Settings, v: bool| {
                let b = s.m.sandboxes();
                match helper {
                    Helper::Twatch => b.twatch = v,
                    Helper::Streamlink => b.streamlink = v,
                    Helper::Poom => b.poom = v,
                    Helper::Fetch => b.fetch = v,
                }
            };
            cards.push(row(&id, on, Some(act(cx, move |s| set(s, false))), toggle(cx, on, set)));
            if let Some(why) = pms::sandbox::trouble(helper).filter(|_| on) {
                cards.push(TextBlock::new().text(s::SANDBOX_TROUBLE.with(&[("why", &why)])).text_wrapping(TextWrapping::Wrap).font_size(12.0).foreground(Brush::Theme(ThemeBrush::SystemCritical)).margin(Thickness::new(18.0, 0.0, 18.0, 6.0)).into());
            }
        }
        let note = |text: &str| TextBlock::new().text(text.to_string()).text_wrapping(TextWrapping::Wrap).font_size(12.0).opacity(0.7);
        StackPanel::new().spacing(4.0).children((heading((s::SANDBOX, s::SANDBOX_ABOUT)), group(s::SANDBOX_HELPERS.t(), cards), note(s::SANDBOX_ALWAYS.t()), note(s::SANDBOX_CANT.t()), note(s::SANDBOX_SEE.t())))
    }

    fn toasts(&self, cx: &Cx) -> View {
        let t = &self.m.config.toasts;
        // Off with its settings kept (`enabled = false`, as a notice's *Keep Windows notifications* leaves it), it comes
        // on again with them.
        let on = t.as_ref().is_some_and(|t| t.enabled);
        let mut cards = vec![switch_card(
            "\u{E91C}",
            s::TOASTS_SHOW_NOTIFICATIONS_IN_THE.t(),
            s::TOASTS_WHEN_ITS_OFF_NOTIFICATIONS.t(),
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
                    "toasts.position",
                    t.position != d.position,
                    {
                        let to = d.position.clone();
                        Some(act(cx, move |s| {
                            if let Some(t) = s.m.config.toasts.as_mut() {
                                t.position = to.clone();
                            }
                        }))
                    },
                    choice(cx, &t.position, copy::TOAST_POSITIONS, |s, v| {
                        if let Some(t) = s.m.config.toasts.as_mut() {
                            t.position = v;
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
                })), list(cx, &t.ignore, ListAt::ToastsIgnore, s::TOASTS_FOR_EXAMPLE_TEAMS.t(), false)),
            ]);
        }
        StackPanel::new().spacing(4.0).children((heading(copy::TOASTS), group_reset(cx, s::TOASTS_TASKBAR_NOTIFICATIONS.t(), Section::Toasts, self.m.at_default(Section::Toasts), cards)))
    }
}

/// The processes' table: a name, and each figure in a column as wide as it can need. Its lines and shades are grey with
/// little to them, which shows as a shade lighter on a dark window and a shade darker on a light one.
const TABLE_COLUMNS: [GridLength; 7] = [GridLength::STAR, GridLength::Pixel(72.0), GridLength::Pixel(92.0), GridLength::Pixel(68.0), GridLength::Pixel(118.0), GridLength::Pixel(122.0), GridLength::Pixel(96.0)];

/// A process's sandbox, as the processes' table shows it: in short, and all of it, for its tooltip.
fn sandbox_said(sandbox: Option<&pms::sandbox::Sandbox>) -> (String, String) {
    match sandbox {
        None => (String::new(), String::new()),
        Some(pms::sandbox::Sandbox::None) => (s::SANDBOX_NONE.t().to_string(), String::new()),
        Some(pms::sandbox::Sandbox::Container { name, lpac, sid, capabilities }) => {
            // PimpMyShell.<helper>.<mark>: the helper is what's worth saying in a narrow column.
            let helper = name.split('.').nth(1).filter(|_| name.starts_with("PimpMyShell.")).unwrap_or(name);
            let (short, kind) = if *lpac == Some(true) { ("LPAC", "LPAC") } else { ("AC", "AppContainer") };
            (format!("{short}: {helper}"), s::SANDBOX_DETAILS.with(&[("kind", &kind), ("name", name), ("sid", sid), ("capabilities", &capabilities.join(", "))]))
        }
    }
}
const TABLE_LINE: Color = Color::argb(0x30, 0x80, 0x80, 0x80);
const TABLE_OTHER: Color = Color::argb(0x18, 0x80, 0x80, 0x80);
const TABLE_OVER: Color = Color::argb(0x38, 0x80, 0x80, 0x80);
const TABLE_CLEAR: Color = Color::argb(0x00, 0x80, 0x80, 0x80);

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
        "screen" => Style::Screen,
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
    // Each in its own table, which the card says where to find.
    let (tint_id, grain_id, gaps_id) = if light { ("look.light.tint", "look.light.grain", "look.light.gaps") } else { ("look.tint", "look.grain", "look.gaps") };
    vec![
        row(
            tint_id,
            tint.is_some_and(|t| t != default_tint),
            Some(act(cx, move |s| with_look(s, light, |t, _, _| *t = None))),
            color(cx, tint.unwrap_or(default_tint), move |s, v| with_look(s, light, |t, _, _| *t = (v != default_tint).then_some(v))),
        ),
        row(
            grain_id,
            grain.is_some_and(|g| g != config::DEFAULT_GRAIN),
            Some(act(cx, move |s| with_look(s, light, |_, g, _| *g = None))),
            level(cx, grain.unwrap_or(config::DEFAULT_GRAIN), 0.2, move |s, v| with_look(s, light, |_, g, _| *g = (v != config::DEFAULT_GRAIN).then_some(v))),
        ),
        row(
            gaps_id,
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
        text(cx, &w.expr, s::SHOW_WHEN_EDITOR_FOR_EXAMPLE_1_MEM.t(), 300.0, move |s, v| with_when(s, i, |w| w.expr = v)),
        StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).children((
            TextBlock::new().text(s::SHOW_WHEN_EDITOR_SHOW_AT.t()).vertical_alignment(VerticalAlignment::Center),
            number(cx, w.at, -1e15, 1e15, "", move |s, v| with_when(s, i, |w| w.at = v)),
            TextBlock::new().text(s::SHOW_WHEN_EDITOR_HIDE_BELOW.t()).vertical_alignment(VerticalAlignment::Center).margin(Thickness::new(12.0, 0.0, 0.0, 0.0)),
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
    /// Facts that go with an ordinary question, each a label and a value, grouped on a card under its words.
    pub details: Vec<(String, String)>,
    /// A last line, quieter than the rest.
    pub note: String,
}

/// See `Question::check`.
const CHECKED: i32 = 16;

impl Question {
    /// From `--ask`'s arguments.
    pub fn from_args(args: &[String]) -> Question {
        let mut q = Question { heading: String::new(), text: String::new(), link: None, buttons: Vec::new(), check: None, check_note: String::new(), rows: Vec::new(), details: Vec::new(), note: String::new() };
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
                "--detail" => {
                    if let (Some(label), Some(value)) = (it.next(), it.next()) {
                        q.details.push((label.clone(), value.clone()));
                    }
                }
                "--note" => q.note = it.next().cloned().unwrap_or_default(),
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

/// Shows the welcome, for the settings at `path`: the exit code says only that it ran (255 where there's no WinUI 3). `news`: only
/// what's new since the last version, for somebody who's been here before.
pub fn welcome(path: PathBuf, news: bool) -> i32 {
    WHATS_NEW.store(news, std::sync::atomic::Ordering::Relaxed);
    match App::run_component::<Welcome>(path) {
        Ok(()) => 0,
        Err(_) => 255,
    }
}

/// The welcome is only what's new.
static WHATS_NEW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The welcome's pages.
#[derive(Clone, Copy, PartialEq)]
enum Stop {
    /// Which language, before anything's said in one.
    Language,
    Hello,
    Style,
    Confirm,
    Tv,
    Pimpbox,
    Twatch,
    Fresh,
    /// What's new, for somebody who's been here before.
    News,
}

/// A first run's pages, and what's new since the version before for everybody else.
const FIRST_RUN: [Stop; 8] = [Stop::Language, Stop::Hello, Stop::Style, Stop::Confirm, Stop::Tv, Stop::Pimpbox, Stop::Twatch, Stop::Fresh];
const NEWS: [Stop; 4] = [Stop::News, Stop::Tv, Stop::Pimpbox, Stop::Twatch];

/// The first run: a few choices, with what they look like, in the settings' clothes. Nothing's kept until the end, so
/// closing it leaves the defaults.
struct Welcome {
    m: Model,
    pages: &'static [Stop],
    step: usize,
    /// There's a TV.
    tv: bool,
    style: String,
    confirm: String,
    updates: bool,
    startup: bool,
    autostart: bool,
    twatch: bool,
    dollars: Layer,
    panel: ElementRef<SwapChainPanel>,
}

enum WelcomeMsg {
    Tick,
    Next,
    Back,
    /// A language, by its place in `lang::LANGUAGES`.
    Language(usize),
    Style(String),
    Confirm(String),
    Updates(bool),
    Startup(bool),
    Autostart(bool),
    Twatch(bool),
    Tv(bool),
    Get(pms::wads::Wad),
    ShowWads,
}

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
        let news = WHATS_NEW.load(std::sync::atomic::Ordering::Relaxed);
        // A first run (settings all as shipped), and what's new, offer a Twatch TV; asked again later, it says whether there is one.
        let twatch = m.twatch_on() || pms::config::untouched(&m.config) || news;
        let (updates, startup, autostart) = (m.config.update_check.unwrap_or(true), true, m.config.console_autostart);
        // A first run suggests not asking; asked again later, it says what you chose.
        let confirm = if pms::config::untouched(&m.config) { "off".to_string() } else { toasts.confirm_clicks };
        let pages: &'static [Stop] = if news { &NEWS } else { &FIRST_RUN };
        let tv = !m.tvs().is_empty();
        Welcome { m, pages, step: 0, tv, style: toasts.style, confirm, updates, startup, autostart, twatch, dollars: Layer::default(), panel: ElementRef::new() }
    }

    fn update(&mut self, msg: WelcomeMsg, context: &ComponentContext<Self>) {
        match msg {
            WelcomeMsg::Tick => {
                self.dollars.attach(&self.panel);
                welcome_tick(context);
            }
            WelcomeMsg::Back => self.step = self.step.saturating_sub(1),
            WelcomeMsg::Next if self.step + 1 < self.pages.len() => self.step += 1,
            WelcomeMsg::Next => self.finish(),
            // Said in it from here on, and kept with the rest at the end.
            WelcomeMsg::Language(chosen) => {
                self.m.config.language = pms::language_setting(chosen);
                self.m.speak();
            }
            WelcomeMsg::Style(v) => self.style = v,
            WelcomeMsg::Confirm(v) => self.confirm = v,
            WelcomeMsg::Updates(v) => self.updates = v,
            WelcomeMsg::Startup(v) => self.startup = v,
            WelcomeMsg::Autostart(v) => self.autostart = v,
            WelcomeMsg::Twatch(v) => self.twatch = v,
            WelcomeMsg::Tv(v) => self.tv = v,
            WelcomeMsg::Get(wad) => self.m.get_wad(wad),
            WelcomeMsg::ShowWads => self.m.show_wads(),
        }
    }

    fn view(&self, _: &PathBuf, cx: &mut ViewContext<Self>) -> View {
        let news = self.pages[0] == Stop::News;
        cx.window_title(if news { w::WINDOW_TITLE_NEWS.t() } else { w::WINDOW_TITLE.t() });
        let visuals = WindowVisuals::new().backdrop(WindowBackdrop::Acrylic).client_size(680.0, 720.0);
        cx.window_visuals(match icon() {
            Some(icon) => visuals.icon(icon),
            None => visuals,
        });
        let page = self.pages.get(self.step).copied().unwrap_or(Stop::Fresh);
        // The page that says hello shows the bills as they are; the rest, where there's reading to do, has them blurred.
        let hello = matches!(page, Stop::Hello | Stop::News);
        self.dollars.set_pane(if hello { u32::MAX } else { 0 });
        let logo = || -> View { Image::new().source_data(EncodedImage::new(LOGO)).width(160.0).height(160.0).horizontal_alignment(HorizontalAlignment::Left).into() };
        let (title, help, body): (&str, &str, View) = match page {
            // Each language by the name it calls itself, with its code under it: en_RN is what Red Hat called its Redneck.
            Stop::Language => (w::LANGUAGE_TITLE.t(), w::LANGUAGE_HELP.t(), {
                let languages = lang::LANGUAGES.iter().enumerate().map(|(at, l)| option(cx, lang::current() == at, l.name, l.code, None, WelcomeMsg::Language(at)));
                children(StackPanel::new().spacing(10.0), languages.collect())
            }),
            Stop::News => (w::NEWS_TITLE.t(), w::NEWS_HELP.t(), logo()),
            Stop::Hello => (w::HELLO_TITLE.t(), w::HELLO_HELP.t(), logo()),
            Stop::Style => (w::STYLE_TITLE.t(), w::STYLE_HELP.t(), {
                let pick = |value: &str, name: &str, words: &str, pimped: bool| {
                    option(cx, self.style == value, name, words, Some(sample(pimped)), WelcomeMsg::Style(value.to_string()))
                };
                children(StackPanel::new().spacing(10.0), vec![pick("standard", w::STYLE_STANDARD.t(), w::STYLE_STANDARD_HELP.t(), false), pick("pimped", w::STYLE_PIMPED.t(), w::STYLE_PIMPED_HELP.t(), true)])
            }),
            Stop::Confirm => (w::CONFIRM_TITLE.t(), w::CONFIRM_HELP.t(), {
                let pick = |value: &str, name: &str, words: &str| option(cx, self.confirm == value, name, words, None, WelcomeMsg::Confirm(value.to_string()));
                children(
                    StackPanel::new().spacing(10.0),
                    vec![
                        pick("card", w::CONFIRM_CARD.t(), w::CONFIRM_CARD_HELP.t()),
                        pick("window", w::CONFIRM_WINDOW.t(), w::CONFIRM_WINDOW_HELP.t()),
                        pick("off", w::CONFIRM_OFF.t(), w::CONFIRM_OFF_HELP.t()),
                    ],
                )
            }),
            Stop::Tv => {
                let on = settings_card(None, w::TV_KEEP.t(), w::TV_KEEP_HELP.t(), None, switch(self.tv, cx.callback(WelcomeMsg::Tv)));
                let about = [("\u{E8B2}", w::TV_REMOTE.t(), w::TV_REMOTE_HELP.t()), ("\u{E7F4}", w::TV_CHANNELS.t(), w::TV_CHANNELS_HELP.t()), ("\u{E7E8}", w::TV_OFF.t(), w::TV_OFF_HELP.t())];
                (w::TV_TITLE.t(), w::TV_HELP.t(), points(&about, vec![on]))
            }
            Stop::Pimpbox => (w::PIMPBOX_TITLE.t(), w::PIMPBOX_HELP.t(), self.pimpbox(cx)),
            Stop::Twatch => {
                let words = match (self.no_channels(), self.twatch) {
                    (true, true) => w::TWATCH_NEXT.t(),
                    (true, false) => w::TWATCH_OFF.t(),
                    (false, _) => w::TWATCH_ON_HELP.t(),
                };
                let on = settings_card(None, w::TWATCH_ON.t(), words, None, switch(self.twatch, cx.callback(WelcomeMsg::Twatch)));
                let about = [("\u{E734}", w::TWATCH_BEST.t(), w::TWATCH_BEST_HELP.t()), ("\u{E945}", w::TWATCH_PC.t(), w::TWATCH_PC_HELP.t()), ("\u{E83F}", w::TWATCH_BATTERY.t(), w::TWATCH_BATTERY_HELP.t())];
                (w::TWATCH_TITLE.t(), w::TWATCH_HELP.t(), points(&about, vec![on]))
            }
            Stop::Fresh => (w::FRESH_TITLE.t(), w::FRESH_HELP.t(), {
                let line = |title: &str, words: &str, on: bool, msg: fn(bool) -> WelcomeMsg| {
                    settings_card(None, title, words, None, switch(on, cx.callback(msg)))
                };
                children(
                    StackPanel::new().spacing(10.0),
                    vec![
                        line(w::UPDATES.t(), w::UPDATES_HELP.t(), self.updates, WelcomeMsg::Updates),
                        line(w::STARTUP.t(), w::STARTUP_HELP.t(), self.startup, WelcomeMsg::Startup),
                    ],
                )
            }),
        };
        let last = self.step + 1 == self.pages.len();
        let next = match () {
            () if hello => w::RIDE.t(),
            () if last && news => w::DONE.t(),
            () if last => w::GET_PIMPED.t(),
            () => w::NEXT.t(),
        };
        let dots = (0..self.pages.len()).map(|i| {
            let dot = Border::new().width(8.0).height(8.0).corner_radius(CornerRadius::uniform(4.0)).background(Brush::Theme(ThemeBrush::PrimaryText)).opacity(if i == self.step { 0.9 } else { 0.25 });
            KeyedView::new(i.to_string(), dot)
        });
        let back = Button::new().grid_column(0).is_enabled(self.step > 0).opacity(if self.step > 0 { 1.0 } else { 0.0 }).min_width(90.0).on_click(cx.callback(|()| WelcomeMsg::Back)).content(w::BACK.t());
        // Going on, or closing, would take a download with it.
        let getting = matches!(*self.m.wad_job.lock().unwrap(), WadJob::Getting(..));
        let forward = Button::new().grid_column(2).style(ButtonStyle::Accent).min_width(120.0).is_enabled(!getting).on_click(cx.callback(|()| WelcomeMsg::Next)).content(next);
        let page = Grid::new().rows([GridLength::Auto, GridLength::STAR, GridLength::Auto]).margin(Thickness::new(44.0, 40.0, 44.0, 28.0)).row_spacing(24.0).children((
            StackPanel::new().grid_row(0).spacing(8.0).children((
                TextBlock::new().text(title).font_size(32.0).font_weight(FontWeight::SEMI_BOLD).text_wrapping(TextWrapping::Wrap),
                TextBlock::new().text(help).font_size(15.0).opacity(0.75).text_wrapping(TextWrapping::Wrap),
            )),
            ScrollViewer::new().grid_row(1).vertical_scroll_bar_visibility(ScrollBarVisibility::Auto).content(body),
            Grid::new().grid_row(2).columns([GridLength::Auto, GridLength::STAR, GridLength::Auto]).children((
                back,
                StackPanel::new().grid_column(1).orientation(Orientation::Horizontal).spacing(8.0).horizontal_alignment(HorizontalAlignment::Center).vertical_alignment(VerticalAlignment::Center).keyed_children(dots),
                forward,
            )),
        ));
        Grid::new().children((SwapChainPanel::new().element_ref(&self.panel), page))
    }
}

/// What's worth knowing about something, a card each, and then `after`.
fn points(items: &[(&str, &str, &str)], after: Vec<View>) -> View {
    let cards = items.iter().map(|(glyph, title, words)| settings_card(Some(glyph), title, words, None, TextBlock::new()));
    children(StackPanel::new().spacing(8.0), cards.chain(after).collect())
}

impl Welcome {
    /// The choices of game: one to fetch now, or the place to put your own.
    fn pimpbox(&self, cx: &mut ViewContext<Self>) -> View {
        use pms::wads::Wad;
        let wads = pms::wads::list();
        let have = |name: &str| wads.iter().any(|w| w.eq_ignore_ascii_case(name));
        let job = self.m.wad_job.lock().unwrap().clone();
        let busy = matches!(job, WadJob::Getting(..));
        let get = |cx: &ViewContext<Self>, wad: Wad, words: &str, owned: bool| -> View {
            if owned {
                TextBlock::new().text(w::HAVE_IT.t()).vertical_alignment(VerticalAlignment::Center).opacity(0.7).into()
            } else {
                Button::new().is_enabled(!busy).on_click(cx.callback(move |()| WelcomeMsg::Get(wad))).content(words.to_string())
            }
        };
        let mut cards = vec![
            download_card("Freedoom", w::FREEDOOM_HELP.t(), pms::wads::url(Wad::Freedoom), get(cx, Wad::Freedoom, w::DOWNLOAD.t(), have("freedoom1.wad"))),
            download_card(w::SHAREWARE.t(), w::SHAREWARE_HELP.t(), pms::wads::url(Wad::Shareware), get(cx, Wad::Shareware, w::DOWNLOAD.t(), have("doom1.wad"))),
            settings_card(None, w::OWN_GAME.t(), w::OWN_GAME_HELP.t(), None, Button::new().on_click(cx.callback(|()| WelcomeMsg::ShowWads)).content(w::OPEN_FOLDER.t())),
        ];
        match job {
            WadJob::Getting(done, total) => cards.push(card(StackPanel::new().spacing(6.0).children((
                TextBlock::new().text(if total == 0 { w::WAITING.t().to_string() } else { w::DOWNLOADING.with(&[("done", &(done / 1024)), ("total", &(total / 1024))]) }),
                ProgressBar::new().minimum(0.0).maximum(total.max(1) as f64).value(done as f64).is_indeterminate(total == 0),
            )))),
            WadJob::Failed(e) => cards.push(card(TextBlock::new().text(w::FAILED.with(&[("why", &e)])).text_wrapping(TextWrapping::Wrap))),
            WadJob::Idle => {}
        }
        let (title, words) = copy::setting("console_autostart");
        cards.push(settings_card(None, title, words, None, switch(self.autostart, cx.callback(WelcomeMsg::Autostart))));
        cards.push(TextBlock::new().text(s::TV_NOTICE.t()).text_wrapping(TextWrapping::Wrap).font_size(12.0).opacity(0.7).margin(Thickness::new(4.0, 6.0, 4.0, 0.0)).into());
        // This one scrolls: room for the scroll bar, so that it covers nothing.
        children(StackPanel::new().spacing(8.0).margin(Thickness::new(0.0, 0.0, 14.0, 0.0)), cards)
    }

    /// There are no Twatch TV channels yet.
    fn no_channels(&self) -> bool {
        self.m.config.twatch.as_ref().is_none_or(|t| t.favorites.is_empty())
    }

    /// Keeps the choices, and that's that. What's new keeps only what it asked about. A Twatch TV with no channels has nothing to
    /// show, so that's where Settings opens next.
    fn finish(&mut self) {
        let next = self.twatch && self.no_channels();
        if !self.pages.contains(&Stop::News) {
            let toasts = self.m.config.toasts.get_or_insert_with(Toasts::default);
            toasts.style = self.style.clone();
            toasts.confirm_clicks = self.confirm.clone();
            self.m.config.update_check = Some(self.updates);
            if self.startup != self.m.autostarts() {
                self.m.set_autostart(self.startup);
            }
        }
        self.m.config.console_autostart = self.autostart;
        if self.twatch != self.m.twatch_on() {
            self.m.set_twatch_on(self.twatch);
        }
        match (self.tv, self.m.tvs().is_empty()) {
            (true, true) => self.m.add_tv(),
            (false, false) => self.m.remove_tvs(),
            _ => {}
        }
        self.m.save_now();
        if next && let Ok(me) = std::env::current_exe() {
            let _ = std::process::Command::new(me).arg("--page=twatch").arg(&self.m.path).spawn();
        }
        std::process::exit(0);
    }
}

/// A notification as it looks in the taskbar, for choosing one.
fn sample(pimped: bool) -> View {
    let gold = parse_color("#FFD35C");
    let title = TextBlock::new().text(s::SAMPLE_YOUR_TASKBAR_CALLED.t()).font_size(12.0).font_weight(FontWeight::SEMI_BOLD);
    let title = if pimped { title.foreground(Brush::Solid(gold)) } else { title };
    let words = StackPanel::new().vertical_alignment(VerticalAlignment::Center).children((
        TextBlock::new().text("PimpMyShell").font_size(10.5).opacity(0.65),
        title,
        TextBlock::new().text(s::SAMPLE_IT_SAYS_ITS_LOOKING.t()).font_size(11.5).opacity(0.8),
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
        Some(WelcomeMsg::Language(chosen)) => cx.callback(move |()| WelcomeMsg::Language(chosen)),
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
        let detail_lines: usize = q.details.iter().map(|(_, value)| value.chars().count() / 62 + 1).sum();
        let details_h = if q.details.is_empty() { 0.0 } else { 16.0 + q.details.len() as f64 * 18.0 + detail_lines as f64 * 19.0 };
        let note_h = if q.note.is_empty() { 0.0 } else { 8.0 + (q.note.chars().count() / 80 + 1) as f64 * 17.0 };
        let height = 150.0 + lines as f64 * 20.0 + details_h + note_h + if q.link.is_some() { 36.0 } else { 0.0 } + if q.check.is_some() { 44.0 } else { 0.0 };
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
        // The facts, together on a card of their own: each a small label over its value.
        if !q.details.is_empty() {
            let rows = q.details.iter().enumerate().map(|(i, (label, value))| {
                KeyedView::new(
                    i.to_string(),
                    StackPanel::new().spacing(2.0).children((
                        TextBlock::new().text(label.clone()).font_size(11.0).opacity(0.6),
                        TextBlock::new().text(value.clone()).font_size(13.0).text_wrapping(TextWrapping::Wrap).is_text_selection_enabled(true),
                    )),
                )
            });
            body.push(
                Border::new()
                    .corner_radius(CornerRadius::uniform(8.0))
                    .background(Brush::Theme(ThemeBrush::CardBackground))
                    .border_brush(Brush::Theme(ThemeBrush::CardStroke))
                    .border_thickness(Thickness::uniform(1.0))
                    .content(StackPanel::new().spacing(10.0).margin(Thickness::xy(14.0, 12.0)).keyed_children(rows)),
            );
        }
        if !q.note.is_empty() {
            body.push(TextBlock::new().text(q.note.clone()).font_size(12.0).opacity(0.7).text_wrapping(TextWrapping::Wrap).into());
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
        // Its words scroll when there are more than fit, a license say: the buttons stay where they are.
        Grid::new().rows([GridLength::STAR, GridLength::Auto]).children((
            ScrollViewer::new().grid_row(0).vertical_scroll_bar_visibility(ScrollBarVisibility::Auto).content(children(StackPanel::new().spacing(12.0).margin(Thickness::new(24.0, 20.0, 24.0, 12.0)), body)),
            Border::new().grid_row(1).padding(Thickness::xy(24.0, 16.0)).background(Brush::Theme(ThemeBrush::SolidBackground)).content(
                StackPanel::new().orientation(Orientation::Horizontal).spacing(8.0).horizontal_alignment(HorizontalAlignment::Right).keyed_children(buttons),
            ),
        ))
    }
}
