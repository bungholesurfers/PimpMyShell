//! PimpMyShell's settings: edits `pms.toml` while you watch. Every change is validated and saved half a second after
//! you stop, and the running daemon applies it within a second, so the taskbar follows along.
//!
//! Like Nagare's panel, settings go by their config names: what you see here is what the file says, and a name can be
//! looked up in pms.example.toml for the whole story. Hover a name for a hint; a name in amber differs from its
//! default, and ↺ puts the default back.

#![windows_subsystem = "windows"]

mod copy;
mod model;
mod dollars;
mod flag;
mod winui;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText};
use pms::config::{self, Agg, AppGlow, Backdrop, Config, Exprs, Hotmic, Look, Max, MeterSpec, ShowWhen, StartGlow, Style, Toasts};
use pms::update::Status;

use model::{Model, Section, Update, Widget, toml_meter};


const AMBER: Color32 = Color32::from_rgb(0xFF, 0xC1, 0x5A);
const ERROR: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x80);
const OK: Color32 = Color32::from_rgb(0x81, 0xC7, 0x84);
const DIM: Color32 = Color32::from_gray(150);
/// The config-name column.
const KEY_WIDTH: f32 = 170.0;

/// `pms-gui [--egui | --winui] [config]`: the WinUI 3 window where the PC has the Windows App Runtime (winui.rs),
/// else the egui one; `--egui` asks for the egui one, `--winui` for WinUI 3 whatever (failing without the runtime).
fn main() -> eframe::Result {
    // The daemon's questions (winui.rs): answered by the exit code; 255 where there's no WinUI 3 (the daemon then asks
    // with a task dialog of its own).
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().is_some_and(|a| a == "--ask") {
        std::process::exit(if winui::available() { winui::ask(winui::Question::from_args(&raw[1..])) } else { 255 });
    }
    pms::daemon::migrate();
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let path = config::given(args.iter().find(|a| !a.to_string_lossy().starts_with("--")).cloned());
    // `--page=tv`: to that page, in the window that's open if there is one.
    let page = args.iter().find_map(|a| a.to_string_lossy().strip_prefix("--page=").map(str::to_string));
    if page.as_deref().is_some_and(model::go_to_page) {
        return Ok(());
    }
    if flag("--welcome") || flag("--whats-new") {
        std::process::exit(if winui::available() { winui::welcome(path, flag("--whats-new")) } else { 255 });
    }
    // `--no-winui` acts as if the Windows App Runtime weren't here.
    let runtime = !flag("--no-winui") && winui::available();
    let mut fallback = None;
    if !flag("--egui") {
        if flag("--winui") || runtime {
            match winui::run(path.clone(), page.as_deref()) {
                Ok(()) => return Ok(()),
                // It can't start (only once a process: Application::Start), so the egui one does.
                Err(e) => {
                    eprintln!("WinUI 3: {e}; the egui window instead");
                    fallback = Some(copy::CLASSIC_FAILED);
                }
            }
        } else {
            fallback = Some(copy::CLASSIC_NO_RUNTIME);
        }
    }
    egui_main(path, runtime, fallback, page.as_deref().and_then(page_tagged).unwrap_or(Page::General))
}

fn page_tagged(tag: &str) -> Option<Page> {
    match tag {
        "general" => Some(Page::General),
        "widgets" => Some(Page::Widgets),
        "meters" => Some(Page::Meters),
        "counters" => Some(Page::Counters),
        "look" => Some(Page::Backdrop),
        "tv" => Some(Page::Tv),
        "poom" => Some(Page::Poom),
        "twatch" => Some(Page::Twatch),
        "hotmic" => Some(Page::Hotmic),
        "toasts" => Some(Page::Toasts),
        "network" => Some(Page::Network),
        "power" => Some(Page::Power),
        "logs" => Some(Page::Logs),
        _ => None,
    }
}

fn egui_main(path: PathBuf, runtime: bool, fallback: Option<&'static str>, page: Page) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: {
            let v = egui::ViewportBuilder::default().with_title(model::WINDOW_TITLE).with_inner_size([960.0, 700.0]).with_min_inner_size([760.0, 460.0]);
            // The logo in the title bar and Alt+Tab (pms-art renders it).
            match eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon/pms-64.png")) {
                Ok(icon) => v.with_icon(icon),
                Err(_) => v,
            }
        },
        ..Default::default()
    };
    eframe::run_native(
        "PimpMyShell",
        options,
        Box::new(move |cc| {
            fonts(&cc.egui_ctx);
            style(&cc.egui_ctx);
            Ok(Box::new(Gui::open(path, runtime, fallback, page)))
        }),
    )
}

/// Segoe UI, like the rest of Windows, Cascadia Mono for config names, and Segoe UI Symbol for arrows and such.
fn fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let dir = std::path::Path::new(r"C:\Windows\Fonts");
    for (name, file, family, first) in [
        ("segoe", "segoeui.ttf", egui::FontFamily::Proportional, true),
        ("cascadia", "CascadiaMono.ttf", egui::FontFamily::Monospace, true),
        ("symbols", "seguisym.ttf", egui::FontFamily::Proportional, false),
        ("symbols", "seguisym.ttf", egui::FontFamily::Monospace, false),
    ] {
        let Ok(data) = std::fs::read(dir.join(file)) else { continue };
        fonts.font_data.entry(name.into()).or_insert_with(|| Arc::new(egui::FontData::from_owned(data)));
        let list = fonts.families.entry(family).or_default();
        if first { list.insert(0, name.into()) } else { list.push(name.into()) }
    }
    ctx.set_fonts(fonts);
}

/// Flat and dark, square widgets, a quiet stripe: in the manner of Nagare's panel.
fn style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        s.spacing.interact_size.y = 22.0;
        let v = &mut s.visuals;
        v.panel_fill = Color32::from_rgb(0x1B, 0x1C, 0x20);
        v.window_fill = Color32::from_rgb(0x22, 0x23, 0x28);
        v.faint_bg_color = Color32::from_rgb(0x20, 0x21, 0x26);
        v.extreme_bg_color = Color32::from_rgb(0x12, 0x13, 0x16);
        let w = &mut v.widgets;
        for w in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open, &mut w.noninteractive] {
            w.corner_radius = egui::CornerRadius::ZERO;
        }
        w.inactive.weak_bg_fill = Color32::from_rgb(0x2A, 0x2B, 0x31);
        w.inactive.bg_fill = Color32::from_rgb(0x2E, 0x2F, 0x35);
        w.inactive.fg_stroke.color = Color32::from_gray(230);
        w.hovered.weak_bg_fill = Color32::from_rgb(0x37, 0x38, 0x3F);
        w.hovered.bg_fill = Color32::from_rgb(0x3B, 0x3C, 0x43);
        w.hovered.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_gray(0x60));
        w.hovered.fg_stroke.color = Color32::from_gray(250);
        w.active.weak_bg_fill = Color32::from_rgb(0x48, 0x49, 0x52);
        w.active.bg_fill = Color32::from_rgb(0x4C, 0x4D, 0x56);
        w.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(0x2C, 0x2D, 0x33));
        w.noninteractive.fg_stroke.color = Color32::from_gray(205);
        v.selection.bg_fill = Color32::from_rgb(0x2C, 0x6B, 0xA3);
        v.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    });
}

#[derive(PartialEq, Clone, Copy)]
enum Page {
    General,
    Widgets,
    Meters,
    Counters,
    Backdrop,
    Tv,
    Poom,
    Twatch,
    Hotmic,
    Toasts,
    Network,
    Power,
    Logs,
}

const PAGES: [(Page, &str, &str); 13] = [
    (Page::General, "General", "Status, startup, and updates"),
    (Page::Widgets, "Widgets", "What's along the taskbar, in its order"),
    (Page::Meters, "Meters", "The meters next to the system tray"),
    (Page::Counters, "Performance counters", "What your meters read"),
    (Page::Backdrop, "Taskbar look", "Background and glows"),
    (Page::Tv, "TV", "The little screen in your taskbar"),
    (Page::Poom, "POOM", "The console on channel 6, and its games"),
    (Page::Twatch, "Twatch TV", "Your favorite channels, live"),
    (Page::Hotmic, "Hot Mic", "Know when an app uses your microphone"),
    (Page::Toasts, "Notification Toast", "Notifications in the taskbar"),
    (Page::Network, "Network", "What goes online, and a proxy"),
    (Page::Power, "Power & performance", "When PimpMyShell gets out of the way"),
    (Page::Logs, "Logs", "What it writes down, and what that says about you"),
];

/// The egui window: the model (model.rs, which the WinUI 3 window shares), and which page and meter it shows.
/// A small button under a section that puts it back, once `update` has asked.
fn reset_line(ui: &mut egui::Ui, ask: &mut Option<Section>, section: Section) {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        if ui.small_button(format!("Reset {}…", section.name())).clicked() {
            *ask = Some(section);
        }
    });
    ui.add_space(6.0);
}

struct Gui {
    m: Model,
    page: Page,
    meter: usize,
    /// This PC can show the new (WinUI 3) window: the General page offers it.
    winui: bool,
    /// The section whose reset is waiting to be confirmed.
    confirm: Option<Section>,
    /// Resetting every setting is waiting to be confirmed.
    confirm_all: bool,
    /// Why this window is showing instead of the WinUI 3 one, unless it was asked for.
    fallback: Option<&'static str>,
}

impl Gui {
    fn open(path: PathBuf, runtime: bool, fallback: Option<&'static str>, page: Page) -> Gui {
        Gui { m: Model::open(path), page, meter: 0, winui: runtime, confirm: None, confirm_all: false, fallback }
    }
}

impl std::ops::Deref for Gui {
    type Target = Model;
    fn deref(&self) -> &Model {
        &self.m
    }
}

impl std::ops::DerefMut for Gui {
    fn deref_mut(&mut self) -> &mut Model {
        &mut self.m
    }
}

impl eframe::App for Gui {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The panels go on the window's own ui; what else wants the context (windows, styles) has it.
        let ctx = &ui.ctx().clone();
        if self.polled.elapsed() > Duration::from_secs(2) {
            self.poll_daemon();
            if let Some(page) = model::page_asked().as_deref().and_then(page_tagged) {
                self.page = page;
            }
        }
        let status_bar = egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 6)).fill(Color32::from_rgb(0x15, 0x16, 0x19));
        egui::Panel::bottom("status").frame(status_bar).show(ui, |ui| {
            let (text, error) = &self.message;
            ui.label(RichText::new(text).color(if *error { ERROR } else { DIM }));
        });
        let sidebar = egui::Frame::new().fill(Color32::from_rgb(0x16, 0x17, 0x1A)).inner_margin(12);
        egui::Panel::left("pages").resizable(false).exact_size(180.0).frame(sidebar).show(ui, |ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("PimpMyShell").size(22.0).strong().color(Color32::WHITE));
            ui.label(RichText::new(format!("Version {}", pms::version::CURRENT)).small().color(DIM));
            ui.add_space(14.0);
            let issues = self.m.issues();
            for (page, name, hint) in PAGES {
                let selected = self.page == page;
                // A page with a setting that couldn't be used says so in red.
                let marked = issues.iter().any(|(_, on, _)| page_tagged(on) == Some(page));
                let r = if page == Page::General {
                    // The first stop, larger, with the one thing to know at a glance: whether the daemon runs.
                    let (state, color) = if self.daemon_running { ("● Running", OK) } else { ("● Not running", AMBER) };
                    let mut job = egui::text::LayoutJob::default();
                    let (style, align) = (ui.style().clone(), egui::Align::LEFT);
                    RichText::new(name).size(17.0).strong().color(if marked { Color32::LIGHT_RED } else { Color32::WHITE }).append_to(&mut job, &style, egui::FontSelection::Default, align);
                    RichText::new(format!("\n{state}")).size(12.5).color(color).append_to(&mut job, &style, egui::FontSelection::Default, align);
                    let fill = if selected { Color32::from_rgb(0x2C, 0x30, 0x38) } else { Color32::from_rgb(0x21, 0x23, 0x28) };
                    let button = egui::Button::new(job).fill(fill).corner_radius(6.0).min_size(egui::vec2(156.0, 54.0));
                    let r = ui.add(button);
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(2.0);
                    r
                } else {
                    // Left-aligned, the width of the pane (a justified layout).
                    let text = RichText::new(name).size(15.0).color(if marked { Color32::LIGHT_RED } else if selected { Color32::WHITE } else { Color32::from_gray(185) });
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                        ui.add(egui::Button::new(text).selected(selected).fill(if selected { Color32::from_rgb(0x2C, 0x6B, 0xA3) } else { Color32::TRANSPARENT }).min_size(egui::vec2(156.0, 30.0)))
                    })
                    .inner
                };
                if r.clicked() {
                    self.page = page;
                }
                r.on_hover_text(hint);
            }
            // The way back to the WinUI 3 window, on every page. On a PC without the Windows App Runtime, it says so.
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min).with_cross_justify(true), |ui| {
                ui.add_space(4.0);
                let open = ui
                    .add_enabled(self.winui, egui::Button::new(RichText::new("New settings window").size(14.0)).min_size(egui::vec2(156.0, 32.0)))
                    .on_hover_text("The Windows 11 style window. It changes the same settings")
                    .on_disabled_hover_text("Needs the Windows App Runtime");
                if open.clicked() {
                    self.m.open_new();
                }
            });
        });
        let body = egui::Frame::new().fill(Color32::from_rgb(0x1B, 0x1C, 0x20)).inner_margin(egui::Margin::symmetric(18, 10));
        if self.problem.is_none() && !self.config.issues.is_empty() {
            let issues = self.m.issues();
            let unusable = self.config.issues.iter().any(|e| !pms::config::issue_is_unknown(e));
            egui::Panel::top("ignored").show(ui, |ui| {
                ui.add_space(6.0);
                let (said, color) = if unusable { ("Some settings in your settings file couldn't be used.", Color32::LIGHT_RED) } else { ("Some lines in your settings file aren't settings.", Color32::from_rgb(0xE0, 0xB0, 0x40)) };
                ui.colored_label(color, format!("{said} {}", copy::IGNORED_LINES));
                for (_, on, words) in issues {
                    ui.horizontal(|ui| {
                        if let Some(page) = page_tagged(on)
                            && ui.button("Go to it").clicked()
                        {
                            self.page = page;
                        }
                        ui.label(RichText::new(words).monospace());
                    });
                }
                if ui.button("Open settings file").clicked() {
                    self.open_config();
                }
                ui.add_space(4.0);
            });
        }
        if let Some(p) = self.problem.clone() {
            egui::Panel::top("problem").show(ui, |ui| {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::LIGHT_RED, "There's a problem in your settings file. You're seeing the defaults, and nothing is saved until it's fixed.");
                ui.label(RichText::new(p.words()).monospace());
                ui.horizontal(|ui| {
                    if ui.button("Open settings file").clicked() {
                        self.open_config();
                    }
                    if ui.button("Show in File Explorer").clicked() {
                        self.show_config();
                    }
                    if ui.add_enabled(self.has_last_good(), egui::Button::new("Restore last working settings")).clicked() {
                        self.restore_last_good();
                    }
                });
                ui.add_space(6.0);
            });
        }
        egui::CentralPanel::default().frame(body).show(ui, |ui| {
            if let Some(why) = self.fallback {
                let note = egui::Frame::new().fill(Color32::from_rgb(0x2B, 0x28, 0x1C)).stroke(egui::Stroke::new(1.0f32, AMBER)).corner_radius(6).inner_margin(10);
                note.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(why).color(Color32::from_gray(230)));
                    if why == copy::CLASSIC_NO_RUNTIME {
                        ui.hyperlink_to(copy::RUNTIME_DOWNLOAD.0, copy::RUNTIME_DOWNLOAD.1);
                    }
                });
                ui.add_space(8.0);
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.page {
                Page::General => self.general(ui),
                Page::Widgets => self.widgets(ui),
                Page::Meters => self.meters(ui),
                Page::Counters => self.counter_rows(ui),
                Page::Backdrop => {
                    let mut ask = None;
                    backdrop(ui, &mut self.config);
                    reset_line(ui, &mut ask, Section::Backdrop);
                    start_glow(ui, &mut self.config.start_glow);
                    reset_line(ui, &mut ask, Section::StartGlow);
                    app_glow(ui, &mut self.config.app_glow);
                    reset_line(ui, &mut ask, Section::AppGlow);
                    self.confirm = ask.or(self.confirm);
                }
                Page::Tv => tv(ui, &mut self.m),
                Page::Poom => poom(ui, &mut self.m),
                Page::Twatch => {
                    twatch(ui, &mut self.m);
                    let mut ask = None;
                    reset_line(ui, &mut ask, Section::Twatch);
                    self.confirm = ask.or(self.confirm);
                }
                Page::Hotmic => {
                    hotmic(ui, &mut self.config.hotmic);
                    let mut ask = None;
                    reset_line(ui, &mut ask, Section::Hotmic);
                    self.confirm = ask.or(self.confirm);
                }
                Page::Toasts => {
                    toasts(ui, &mut self.config.toasts);
                    let mut ask = None;
                    reset_line(ui, &mut ask, Section::Toasts);
                    self.confirm = ask.or(self.confirm);
                }
                Page::Network => network(ui, &mut self.m),
                Page::Power => {
                    power(ui, &mut self.m);
                    let mut ask = None;
                    reset_line(ui, &mut ask, Section::Performance);
                    self.confirm = ask.or(self.confirm);
                }
                Page::Logs => self.logs(ui),
            });
        });
        // Asking before a section goes back to its defaults.
        if let Some(section) = self.confirm {
            egui::Window::new(format!("Reset {}?", section.name())).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_max_width(340.0);
                ui.label(self.m.warning(section));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Reset").clicked() {
                        self.m.reset(section);
                        self.confirm = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm = None;
                    }
                });
            });
        }
        if self.confirm_all {
            egui::Window::new("Reset every setting?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_max_width(340.0);
                ui.label(copy::RESET_ALL_ASK);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Reset").clicked() {
                        self.m.reset_all();
                        self.confirm_all = false;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_all = false;
                    }
                });
            });
        }
        self.autosave();
        // Keep ticking while a save or an update is under way, and now and then for the daemon status.
        let busy = self.updating();
        ctx.request_repaint_after(if self.edited_at.is_some() || busy { Duration::from_millis(100) } else { Duration::from_secs(2) });
    }
}

// ---------------------------------------------------------------------------------------------
// Rows: a config name, its control, and ↺ back to the default.

fn heading(ui: &mut egui::Ui, title: &str, help: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).size(20.0).strong().color(Color32::WHITE));
    if !help.is_empty() {
        ui.label(RichText::new(help).color(DIM));
    }
    ui.add_space(6.0);
}

/// A collapsible section of rows, named for its config table.
fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).size(15.0).strong().color(Color32::WHITE)).default_open(true).show(ui, |ui| {
        egui::Grid::new(title).num_columns(3).striped(true).spacing([10.0, 6.0]).min_col_width(KEY_WIDTH).show(ui, body);
    });
    ui.add_space(6.0);
}

/// One setting, by its id (copy.rs): its title (amber when it isn't the default; pointing to it shows what it does and
/// its key in the file), its control, and ↺ to put the default back.
fn row<T: PartialEq + Clone>(ui: &mut egui::Ui, id: &str, value: &mut T, default: T, control: impl FnOnce(&mut egui::Ui, &mut T)) {
    let changed = *value != default;
    let (title, help) = copy::setting(id);
    let text = RichText::new(title).color(if changed { AMBER } else { Color32::from_gray(225) });
    let hover = if help.is_empty() { format!("pms.toml: {}", copy::key(id)) } else { format!("{help}\n\npms.toml: {}", copy::key(id)) };
    ui.add(egui::Label::new(text).sense(egui::Sense::hover())).on_hover_text(hover);
    ui.horizontal_top(|ui| control(ui, value));
    if changed && ui.small_button("↺").on_hover_text("Reset to default").clicked() {
        *value = default;
    } else if !changed {
        ui.label("");
    }
    ui.end_row();
}

/// A setting the file may leave out: it shows what's in effect, the default, until changed; changed back to the
/// default, it's left out again.
fn opt_row<T: PartialEq + Clone>(ui: &mut egui::Ui, id: &str, value: &mut Option<T>, default: T, control: impl FnOnce(&mut egui::Ui, &mut T)) {
    let mut v = value.clone().unwrap_or_else(|| default.clone());
    row(ui, id, &mut v, default.clone(), control);
    if value.as_ref() != Some(&v) && (value.is_some() || v != default) {
        *value = (v != default).then_some(v);
    }
}

/// An optional setting of a meter: as `opt_row`, but changed (and Reset) against what the shipped meter of the same
/// name has (`shipped`); unset, it still means the code's `default`.
fn opt_row_base<T: PartialEq + Clone>(ui: &mut egui::Ui, id: &str, shipped: Option<T>, value: &mut Option<T>, default: T, control: impl FnOnce(&mut egui::Ui, &mut T)) {
    let mut v = value.clone().unwrap_or_else(|| default.clone());
    let before = v.clone();
    row(ui, id, &mut v, shipped.unwrap_or_else(|| default.clone()), control);
    if v != before {
        *value = (v != default).then_some(v);
    }
}

fn parse_color(s: &str) -> Color32 {
    let hex = s.trim_start_matches('#');
    let n = u32::from_str_radix(hex, 16).unwrap_or(0xFFFFFF);
    match hex.len() {
        8 => Color32::from_rgba_unmultiplied((n >> 16) as u8, (n >> 8) as u8, n as u8, (n >> 24) as u8),
        _ => Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8),
    }
}

fn format_color(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{a:02X}{r:02X}{g:02X}{b:02X}") }
}

/// A swatch that opens a picker, and the hex the config keeps (`#RRGGBB` / `#AARRGGBB`).
fn color(ui: &mut egui::Ui, value: &mut String) {
    let mut c = parse_color(value);
    if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::OnlyBlend).changed() {
        *value = format_color(c);
    }
    ui.add(egui::TextEdit::singleline(value).desired_width(92.0).font(egui::TextStyle::Monospace));
}

fn text(ui: &mut egui::Ui, value: &mut String, hint: &str, width: f32) {
    // Sized outright: in a grid a text box would shrink to the column as it was last frame, and never grow.
    ui.add_sized([width, ui.spacing().interact_size.y], egui::TextEdit::singleline(value).hint_text(hint).font(egui::TextStyle::Monospace));
}

fn number<T: egui::emath::Numeric>(ui: &mut egui::Ui, value: &mut T, range: std::ops::RangeInclusive<T>, suffix: &str) {
    ui.add(egui::DragValue::new(value).range(range).suffix(suffix));
}

/// One of `options` (the file's value, what the window calls it: copy.rs).
fn choice(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[(&str, &str)]) {
    let shown = options.iter().find(|(o, _)| *o == value.as_str()).map_or(value.clone(), |(_, label)| label.to_string());
    egui::ComboBox::from_id_salt(id).width(170.0).selected_text(shown).show_ui(ui, |ui| {
        for (o, label) in options {
            ui.selectable_value(value, o.to_string(), *label);
        }
    });
}

/// A list of strings, one per line with ✕, and + to add.
fn string_list(ui: &mut egui::Ui, id: &str, items: &mut Vec<String>, hint: &str) {
    ui.push_id(id, |ui| {
        ui.vertical(|ui| {
            let mut remove = None;
            for (i, item) in items.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    text(ui, item, hint, 280.0);
                    if ui.small_button("✕").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                items.remove(i);
            }
            if ui.small_button("+ Add").clicked() {
                items.push(String::new());
            }
        });
    });
}


/// A color per part a meter draws (`parts`, by name). No add or remove: how many there are is the meter's.
fn color_slots(ui: &mut egui::Ui, parts: &[String], items: &mut Vec<String>) {
    ui.vertical(|ui| {
        for (k, part) in parts.iter().enumerate() {
            // A part without a color yet shows the new one; only a change writes it (and fills in any before it).
            let mut c = items.get(k).cloned().unwrap_or_else(|| "#4FC3F7".into());
            let before = c.clone();
            ui.horizontal(|ui| {
                ui.add_sized([110.0, 18.0], egui::Label::new(part.as_str()).truncate());
                ui.push_id(k, |ui| color(ui, &mut c));
            });
            if c != before {
                if items.len() <= k {
                    items.resize(k + 1, "#4FC3F7".into());
                }
                items[k] = c;
            }
        }
    });
}
/// `max` / `show_above`: none, a number, or an expression.
fn max_editor(ui: &mut egui::Ui, id: &str, value: &mut Option<Max>) {
    ui.push_id(id, |ui| {
        let kind = match value {
            None => 0,
            Some(Max::Number(_)) => 1,
            Some(Max::Expr(_)) => 2,
        };
        let mut new_kind = kind;
        egui::ComboBox::from_id_salt("kind").width(100.0).selected_text(["None", "Number", "Expression"][kind]).show_ui(ui, |ui| {
            ui.selectable_value(&mut new_kind, 0, "None");
            ui.selectable_value(&mut new_kind, 1, "Number");
            ui.selectable_value(&mut new_kind, 2, "Expression");
        });
        if new_kind != kind {
            *value = match new_kind {
                1 => Some(Max::Number(100.0)),
                2 => Some(Max::Expr(String::new())),
                _ => None,
            };
        }
        match value {
            Some(Max::Number(n)) => {
                ui.add(egui::DragValue::new(n).speed(1.0));
            }
            Some(Max::Expr(e)) => text(ui, e, "For example, ram_total", 200.0),
            None => {}
        }
    });
}

/// A table that's in the file or not (`[hotmic]`, say): a switch, and its rows when on.
fn switch<T>(ui: &mut egui::Ui, label: &str, value: &mut Option<T>, make: impl FnOnce() -> T) {
    let mut on = value.is_some();
    if ui.checkbox(&mut on, RichText::new(label).size(14.0)).changed() {
        *value = on.then(make);
    }
    ui.add_space(4.0);
}

// ---------------------------------------------------------------------------------------------
// Pages


impl Gui {
    fn general(&mut self, ui: &mut egui::Ui) {
        heading(ui, copy::GENERAL.0, copy::GENERAL.1);
        section(ui, "PimpMyShell", |ui| {
            ui.label("Status");
            ui.horizontal(|ui| {
                if self.daemon_running {
                    ui.label(RichText::new("● Running").color(OK));
                    if ui.button("Stop").on_hover_text("Stop PimpMyShell and restore the standard taskbar").clicked() {
                        self.stop_daemon();
                    }
                } else {
                    ui.label(RichText::new("● Not running").color(AMBER));
                    if ui.button("Start").clicked() {
                        self.start_daemon();
                    }
                }
            });
            ui.label("");
            ui.end_row();

            ui.label("Settings file");
            ui.horizontal(|ui| {
                // The button first: a long path gives way to it, not the other way around.
                ui.set_max_width(520.0);
                if ui.small_button("Show in File Explorer").clicked() {
                    self.show_config();
                }
                ui.add(egui::Label::new(RichText::new(pms::shown(&self.path)).monospace().small()).truncate());
            });
            ui.label("");
            ui.end_row();

            ui.label("Twatch TV watchlist");
            ui.horizontal(|ui| {
                ui.set_max_width(520.0);
                if ui.small_button("Show in File Explorer").clicked() {
                    self.show_watchlist();
                }
                ui.add(egui::Label::new(RichText::new(pms::shown(&pms::config::watchlist_path(&self.path))).monospace().small()).truncate());
            });
            ui.label("");
            ui.end_row();

            ui.label("Network settings");
            ui.horizontal(|ui| {
                ui.set_max_width(520.0);
                if ui.small_button("Show in File Explorer").clicked() {
                    self.show_network();
                }
                ui.add(egui::Label::new(RichText::new(pms::shown(&pms::config::network_path(&self.path))).monospace().small()).truncate());
            });
            ui.label(copy::PRIVATE_FILES);
            ui.end_row();

            ui.label("Explorer");
            ui.horizontal(|ui| {
                if ui.small_button("Restart now").on_hover_text("Gives back what Explorer's holding. It won't, with a folder window or file copy open").clicked() {
                    self.restart_explorer();
                }
                ui.checkbox(&mut self.config.notify_explorer, copy::setting("notify_explorer").0).on_hover_text(copy::setting("notify_explorer").1);
            });
            ui.label("");
            ui.end_row();

            ui.label(copy::RESET_ALL.0);
            if ui.small_button("Reset...").on_hover_text(copy::RESET_ALL.1).clicked() {
                self.confirm_all = true;
            }
            ui.label("");
            ui.end_row();

            ui.label("Start when I sign in").on_hover_text("Run PimpMyShell automatically when you sign in to Windows");
            let mut on = self.autostarts();
            if ui.checkbox(&mut on, "").changed() {
                self.set_autostart(on);
            }
            ui.label("");
            ui.end_row();
        });

        let c = &mut self.config;
        section(ui, "Taskbar", |ui| {
            row(ui, "theme", &mut c.theme, "auto".into(), |ui, v| choice(ui, "theme", v, copy::THEMES));
            row(ui, "meter_height", &mut c.meter_height, "tall".into(), |ui, v| {
                choice(ui, "height", v, copy::HEIGHTS)
            });
            row(ui, "bar_height", &mut c.bar_height, "full".into(), |ui, v| {
                choice(ui, "bar-height", v, copy::HEIGHTS)
            });
            row(ui, "interval_ms", &mut c.interval_ms, 1000, |ui, v| number(ui, v, 16..=60_000, " ms"));
            row(ui, "software_gpus", &mut c.software_gpus, false, |ui, v| {
                ui.checkbox(v, "");
            });
            row(ui, "value_font", &mut c.value_font, None, |ui, v| {
                let mut s = v.clone().unwrap_or_default();
                text(ui, &mut s, "Cascadia Mono", 200.0);
                *v = Some(s).filter(|s| !s.is_empty());
            });
        });

        let mut ask = None;
        reset_line(ui, &mut ask, Section::Taskbar);
        self.software_update(ui);
        reset_line(ui, &mut ask, Section::Updates);
        self.confirm = ask.or(self.confirm);
    }

    fn software_update(&mut self, ui: &mut egui::Ui) {
        let c = &mut self.config;
        section(ui, "Software update", |ui| {
            // On is the default, set or not: unset only means the first run's question hasn't been answered yet (and
            // until it is, nothing is checked). A change here answers it.
            let asked = c.update_check.is_some();
            let mut on = c.update_check != Some(false);
            row(ui, "update_check", &mut on, true, |ui, v| {
                ui.checkbox(v, if asked { "" } else { "You haven't chosen yet" });
            });
            if on != (c.update_check != Some(false)) {
                c.update_check = Some(on);
            }
            row(ui, "update_channel", &mut c.update_channel, "stable".into(), |ui, v| {
                choice(ui, "update_channel", v, copy::CHANNELS)
            });
        });
        let state = self.update.lock().unwrap().clone();
        let installed = pms::update::installed();
        let latest = self.config.latest_channel();
        let busy = matches!(state, Update::Checking | Update::Downloading(..));
        let not_installed = "Only an installed copy can install a version";
        let reinstall_help = "Download this version, even if it's the same as or older than yours, and choose whether to install it";
        ui.horizontal(|ui| {
            ui.label(format!("Version {}", pms::version::CURRENT));
            if ui.add_enabled(!busy, egui::Button::new("Check for updates")).clicked() {
                self.start_check(pms::update::Ask { install: installed, automatic: false, force: false, latest });
            }
            ui.separator();
            ui.label("Reinstall:");
            for (label, latest) in [("Stable version", false), ("Latest version", true)] {
                let button = ui.add_enabled(!busy && installed, egui::Button::new(label));
                if button.on_hover_text(reinstall_help).on_disabled_hover_text(not_installed).clicked() {
                    self.start_check(pms::update::Ask { install: true, automatic: false, force: true, latest });
                }
            }
        });
        ui.label(RichText::new(pms::update::addresses()).small().color(DIM));
        match state {
            Update::Idle if !installed => {
                ui.label(RichText::new("This is a portable copy. It can check for updates, but you'll need to install them yourself.").color(DIM));
            }
            Update::Idle => {}
            Update::Checking => {
                ui.label("Checking for updates…");
            }
            Update::Downloading(done, total) => {
                let bar = egui::ProgressBar::new(done as f32 / total.max(1) as f32).desired_width(320.0);
                ui.add(bar.text(format!("Downloading… {} of {} KB", done / 1024, total / 1024)));
            }
            Update::Done(Status::UpToDate { stable }) => {
                let which = if latest { "The latest release" } else { "The latest stable version" };
                ui.label(RichText::new(format!("You're up to date. {which} is {stable}.")).color(OK));
            }
            Update::Done(Status::Available { version }) => {
                ui.horizontal(|ui| {
                    ui.label(format!("PimpMyShell {version} is available. This copy can't install updates."));
                    if ui.button("Go to the release page").clicked() {
                        model::open_page(&pms::update::release_page(&version));
                    }
                });
            }
            Update::Done(Status::Stuck { version }) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("PimpMyShell {version} didn't install. We won't install it again automatically.")).color(ERROR));
                    if ui.button("Go to the releases page").clicked() {
                        model::open_page(pms::update::RELEASES_PAGE);
                    }
                });
            }
            Update::Done(Status::Ready { version, path, sha256 }) => {
                let note = if version == pms::version::CURRENT {
                    " It's the version you have now.".to_string()
                } else if pms::version::newer(pms::version::CURRENT, &version) {
                    format!(" It's older than the version you have ({}).", pms::version::CURRENT)
                } else {
                    String::new()
                };
                ui.label(RichText::new(format!("PimpMyShell {version} is ready to install. It's downloaded and verified.{note}")).color(OK));
                ui.horizontal(|ui| {
                    let install = ui.button("Install and restart");
                    if install.on_hover_text("The taskbar returns to normal for a moment while PimpMyShell installs, then PimpMyShell starts again").clicked() {
                        self.install(&path, &version, &sha256);
                    }
                    if ui.button("Not now").clicked() {
                        *self.update.lock().unwrap() = Update::Idle;
                    }
                });
            }
            Update::Failed(e) => {
                ui.label(RichText::new(format!("We couldn't check for updates: {e}")).color(ERROR));
            }
        }
    }

    fn meters(&mut self, ui: &mut egui::Ui) {
        heading(ui, copy::METERS.0, copy::METERS.1);
        let mut ask = None;
        reset_line(ui, &mut ask, Section::Meters);
        self.confirm = ask.or(self.confirm);
        let interval = self.config.interval_ms;
        // What's shipped that you've removed, a click from coming back.
        let missing: Vec<String> = self.m.missing_meters().iter().map(|m| m.label.clone()).collect();
        let mut restore = None;
        // The meters, not the TVs (they're widgets of their own), and which is selected. What's asked of the list is done after it.
        let plain = self.m.plain_meters();
        if !plain.contains(&self.meter) {
            self.meter = plain.first().copied().unwrap_or(0);
        }
        let (mut action, mut add): (Option<(&str, usize)>, bool) = (None, false);
        let meters = &mut self.m.config.meters;
        let baseline = &self.m.baseline;
        ui.horizontal_top(|ui| {
            // The list, in the taskbar's order: a swatch, the label, its style.
            ui.vertical(|ui| {
                ui.set_width(230.0);
                let mut group: Option<&str> = None;
                for (n, &i) in plain.iter().enumerate() {
                    let m = &meters[i];
                    if n > 0 && m.group.as_deref() != group {
                        ui.add_space(4.0);
                    }
                    group = m.group.as_deref();
                    let swatch = m.color.as_ref().or(m.colors.first()).map(|c| parse_color(c)).unwrap_or(Color32::GRAY);
                    let selected = self.meter == i;
                    let fill = if selected { Color32::from_rgb(0x1F, 0x3F, 0x60) } else { Color32::from_rgb(0x23, 0x24, 0x29) };
                    let r = egui::Frame::new()
                        .fill(fill)
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.set_width(214.0);
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(4.0, 18.0), egui::Sense::hover());
                                ui.painter().rect_filled(rect, 0.0, swatch.to_opaque());
                                ui.label(RichText::new(&m.label).strong().color(Color32::WHITE));
                                ui.label(RichText::new(format!("{:?}", m.style).to_lowercase()).small().color(DIM));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("✕").on_hover_text("Remove").clicked() {
                                        action = Some(("remove", i));
                                    }
                                    if ui.add_enabled(n + 1 < plain.len(), egui::Button::new("↓").small()).on_hover_text("Move down").clicked() {
                                        action = Some(("down", i));
                                    }
                                    if ui.add_enabled(n > 0, egui::Button::new("↑").small()).on_hover_text("Move up").clicked() {
                                        action = Some(("up", i));
                                    }
                                });
                            });
                        })
                        .response
                        .interact(egui::Sense::click());
                    if r.clicked() {
                        self.meter = i;
                    }
                }
                ui.add_space(6.0);
                if ui.button("+ Add meter").clicked() {
                    add = true;
                }
                if !missing.is_empty() {
                    ui.add_space(10.0);
                    ui.label(RichText::new("Add back").strong().color(Color32::WHITE));
                    ui.horizontal_wrapped(|ui| {
                        for label in &missing {
                            if ui.button(format!("+ {label}")).clicked() {
                                restore = Some(label.clone());
                            }
                        }
                    });
                }
            });
            ui.add_space(10.0);
            ui.vertical(|ui| match meters.get_mut(self.meter).filter(|m| !model::is_screen(m)) {
                Some(m) => {
                    let base = baseline.meters.iter().find(|b| b.label.eq_ignore_ascii_case(&m.label)).cloned();
                    meter_editor(ui, m, base.as_ref(), interval)
                }
                None => {
                    ui.label("You don't have any meters. Select + Add meter to make one.");
                }
            });
        });
        match action {
            Some(("remove", i)) => {
                self.m.config.meters.remove(i);
            }
            Some((way, i)) => self.meter = self.m.move_meter(i, way == "up"),
            None => {}
        }
        if add {
            let at = self.m.new_meter_at();
            self.m.config.meters.insert(at, toml_meter(model::NEW_METER));
            self.meter = at;
        }
        if let Some(at) = restore.and_then(|label: String| self.m.restore_meter(&label)) {
            self.meter = at;
        }
    }

    /// What's along the taskbar, in its order: the meters, together; each TV and Twatch TV; and the Hot Mic's label at an end.
    fn widgets(&mut self, ui: &mut egui::Ui) {
        heading(ui, copy::WIDGETS.0, copy::WIDGETS.1);
        let widgets = self.m.widgets();
        if widgets.is_empty() {
            ui.label(copy::WIDGETS_NONE);
        }
        let mut moved = None;
        for (at, w) in widgets.iter().enumerate() {
            let meters = &self.m.config.meters;
            let (title, what) = match *w {
                Widget::Meters => ("Meters".to_string(), self.m.plain_meters().iter().map(|&i| meters[i].label.as_str()).collect::<Vec<_>>().join(", ")),
                Widget::Screen(i) if model::is_twatch(&meters[i]) => (meters[i].label.clone(), "Twatch TV".to_string()),
                Widget::Screen(i) => (meters[i].label.clone(), "TV".to_string()),
                Widget::Mic => ("Hot Mic".to_string(), if at == 0 { copy::WIDGET_MIC_FIRST } else { copy::WIDGET_MIC_LAST }.to_string()),
            };
            let (left, right) = (self.m.can_move_widget(at, true), self.m.can_move_widget(at, false));
            egui::Frame::new().fill(Color32::from_rgb(0x23, 0x24, 0x29)).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                ui.set_width(520.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(title).strong().color(Color32::WHITE));
                        ui.label(RichText::new(what).small().color(DIM));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(right, egui::Button::new("↓")).on_hover_text("Further right").clicked() {
                            moved = Some((at, false));
                        }
                        if ui.add_enabled(left, egui::Button::new("↑")).on_hover_text("Further left").clicked() {
                            moved = Some((at, true));
                        }
                    });
                });
            });
            ui.add_space(2.0);
        }
        if let Some((at, left)) = moved {
            self.m.move_widget(at, left);
        }
        ui.add_space(6.0);
        if self.m.config.hotmic.is_none() {
            ui.small(copy::WIDGETS_NO_MIC);
        }
        ui.small(copy::ORDER_NOTE);
    }

    /// The logs: where they are, the last thing written, and what they can say about you.
    fn logs(&mut self, ui: &mut egui::Ui) {
        heading(ui, copy::LOGS.0, copy::LOGS.1);
        egui::Grid::new("logs").num_columns(3).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.label("Logs");
            ui.horizontal(|ui| {
                ui.set_max_width(520.0);
                if ui.small_button("Show in File Explorer").clicked() {
                    self.show_logs();
                }
                ui.add(egui::Label::new(RichText::new(pms::shown(&pms_tap::logfile::dir())).monospace().small()).truncate());
            });
            ui.label("");
            ui.end_row();

            ui.label("Last log entry");
            ui.scope(|ui| {
                ui.set_max_width(520.0);
                ui.add(egui::Label::new(RichText::new(&self.last_log).monospace().small().color(DIM)).truncate());
            });
            ui.label("");
            ui.end_row();

            ui.label("");
            // One under another: a grid's cell would set them side by side, each a letter wide.
            ui.vertical(|ui| {
                ui.set_width(460.0);
                // In amber: worth a look before a log goes anywhere.
                let amber = Color32::from_rgb(0xF2, 0xB1, 0x4B);
                ui.add_space(4.0);
                ui.label(RichText::new(copy::LOG_PRIVACY.0).strong().color(amber));
                ui.add(egui::Label::new(RichText::new(copy::LOG_PRIVACY.1).small().color(amber)).wrap());
                ui.add(egui::Label::new(RichText::new(copy::log_contents()).small().color(amber)).wrap());
            });
            ui.label("");
            ui.end_row();

        });
    }

    fn counter_rows(&mut self, ui: &mut egui::Ui) {
        heading(ui, copy::COUNTERS.0, copy::COUNTERS.1);
        let mut ask = None;
        reset_line(ui, &mut ask, Section::Counters);
        self.confirm = ask.or(self.confirm);
        let mut remove = None;
        egui::Grid::new("counters").num_columns(5).spacing([10.0, 6.0]).striped(true).show(ui, |ui| {
            for id in ["counter.name", "counter.path", "counter.agg", "counter.default", ""] {
                let (title, help) = if id.is_empty() { ("", "") } else { copy::setting(id) };
                ui.add(egui::Label::new(RichText::new(title).color(DIM)).sense(egui::Sense::hover())).on_hover_text(help);
            }
            ui.end_row();
            for (i, r) in self.counters.iter_mut().enumerate() {
                text(ui, &mut r.name, "Name", 110.0);
                text(ui, &mut r.path, "\\Object(*)\\Counter", 380.0);
                let label = |a: Agg| copy::AGGS.iter().find(|(n, _)| *n == format!("{a:?}").to_lowercase()).map_or("", |(_, l)| *l);
                egui::ComboBox::from_id_salt(("agg", i)).width(110.0).selected_text(label(r.agg)).show_ui(ui, |ui| {
                    for a in [Agg::Sum, Agg::Max, Agg::Min, Agg::Avg, Agg::Engine] {
                        ui.selectable_value(&mut r.agg, a, label(a));
                    }
                });
                ui.horizontal(|ui| {
                    let mut on = r.default.is_some();
                    if ui.checkbox(&mut on, "").changed() {
                        r.default = on.then_some(0.0);
                    }
                    if let Some(d) = &mut r.default {
                        ui.add(egui::DragValue::new(d));
                    }
                });
                if ui.small_button("✕").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            self.counters.remove(i);
        }
        if ui.button("+ Add counter").clicked() {
            self.counters.push(model::new_counter());
        }
    }
}

/// The `[[meter]]` table's rows, those that apply to its style. `interval` is the top-level interval_ms.
// (The shipped values are cloned alike, Copy or not: one pattern for every setting.)
#[allow(clippy::clone_on_copy)]
fn meter_editor(ui: &mut egui::Ui, m: &mut MeterSpec, base: Option<&MeterSpec>, interval: u64) {
    let style = m.style;
    let (label, expr, colors) = (m.label.clone(), m.expr.clone(), m.colors.clone());
    section(ui, "Meter", |ui| {
        row(ui, "meter.label", &mut m.label, label, |ui, v| text(ui, v, "", 200.0));
        opt_row_base(ui, "meter.group", base.and_then(|b| b.group.clone()), &mut m.group, String::new(), |ui, v| text(ui, v, "For example, memory", 200.0));
        opt_row_base(ui, "meter.snug", base.and_then(|b| b.snug.clone()), &mut m.snug, false, |ui, v| {
            ui.checkbox(v, "");
        });
        opt_row_base(ui, "meter.caption", base.and_then(|b| b.caption.clone()), &mut m.caption, true, |ui, v| {
            ui.checkbox(v, "");
        });
        opt_row_base(ui, "meter.sideways", base.and_then(|b| b.sideways.clone()), &mut m.sideways, matches!(style, Style::Disks | Style::Nics), |ui, v| {
            ui.checkbox(v, "");
        });
        row(ui, "meter.style", &mut m.style, base.map_or(Style::History, |b| b.style), |ui, v| {
            let label = |s: Style| copy::STYLES.iter().find(|(n, _)| *n == format!("{s:?}").to_lowercase()).map_or("", |(_, l)| *l);
            egui::ComboBox::from_id_salt("style").width(170.0).selected_text(label(*v)).show_ui(ui, |ui| {
                for s in [Style::History, Style::Bar, Style::Pie, Style::Bars, Style::Cores, Style::Engines, Style::Disks, Style::Nics] {
                    ui.selectable_value(v, s, label(s));
                }
            });
        });
        if matches!(style, Style::Disks | Style::Nics) {
            let id = if style == Style::Disks { "meter.names.disks" } else { "meter.names.nics" };
            row(ui, id, &mut m.names, Vec::new(), |ui, v| string_list(ui, "names", v, "Name"));
        }
        if !matches!(style, Style::Cores | Style::Engines | Style::Disks | Style::Nics) {
            let many = matches!(style, Style::Pie | Style::Bars);
            row(ui, "meter.expr", &mut m.expr, base.map_or(expr, |b| b.expr.clone()), |ui, v| {
                let mut list = v.as_ref().map(Exprs::list).unwrap_or_default();
                if many {
                    string_list(ui, "exprs", &mut list, "Expression");
                } else {
                    list.resize(1, String::new());
                    text(ui, &mut list[0], "Expression", 320.0);
                }
                *v = match list.len() {
                    0 => None,
                    1 => Some(Exprs::One(list.remove(0))),
                    _ => Some(Exprs::Many(list)),
                };
            });
            if many {
                row(ui, "meter.names", &mut m.names, base.map_or(Vec::new(), |b| b.names.clone()), |ui, v| string_list(ui, "names", v, "Name"));
            }
        }
        if matches!(style, Style::Cores | Style::Pie | Style::Bars | Style::Disks | Style::Nics) {
            let id = match style {
                Style::Cores => "meter.colors.cores",
                Style::Disks => "meter.colors.disks",
                Style::Nics => "meter.colors.nics",
                _ => "meter.colors",
            };
            // A color per part the meter draws: two for threads, disks and adapters; one per slice or bar.
            let parts: Vec<String> = match style {
                Style::Cores => vec!["User time".into(), "Kernel time".into()],
                Style::Disks => vec!["Write".into(), "Read".into()],
                Style::Nics => vec!["Sent".into(), "Received".into()],
                _ => {
                    let exprs = m.expr.as_ref().map(Exprs::list).unwrap_or_default();
                    exprs.iter().enumerate().map(|(k, e)| m.names.get(k).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| e.clone())).collect()
                }
            };
            let shipped = base.map_or(colors, |b| b.colors.clone());
            row(ui, id, &mut m.colors, shipped, |ui, v| color_slots(ui, &parts, v));
        } else {
            opt_row_base(ui, "meter.color", base.and_then(|b| b.color.clone()), &mut m.color, "#4FC3F7".into(), color);
        }
        if !matches!(style, Style::Cores | Style::Engines) {
            row(ui, "meter.max", &mut m.max, base.and_then(|b| b.max.clone()), |ui, v| max_editor(ui, "max", v));
            row(ui, "meter.unit", &mut m.unit, base.map_or(String::new(), |b| b.unit.clone()), |ui, v| {
                choice(ui, "unit", v, copy::UNITS);
                text(ui, v, "Or another suffix", 110.0);
            });
            let decimals = if m.unit == "%" { 0 } else { 1 };
            opt_row_base(ui, "meter.decimals", base.and_then(|b| b.decimals.clone()), &mut m.decimals, decimals, |ui, v| number(ui, v, 0..=4, ""));
        }
        opt_row_base(ui, "meter.interval_ms", base.and_then(|b| b.interval_ms.clone()), &mut m.interval_ms, interval, |ui, v| number(ui, v, 16..=60_000, " ms"));
        if style != Style::History {
            opt_row_base(ui, "meter.value", base.and_then(|b| b.value.clone()), &mut m.value, true, |ui, v| {
                ui.checkbox(v, "");
            });
        }
        opt_row_base(ui, "meter.tooltip", base.and_then(|b| b.tooltip.clone()), &mut m.tooltip, true, |ui, v| {
            ui.checkbox(v, "");
        });
        if matches!(style, Style::Cores | Style::Engines) {
            opt_row_base(ui, "meter.glow", base.and_then(|b| b.glow.clone()), &mut m.glow, "level".into(), |ui, v| {
                choice(ui, "glow", v, copy::GLOWS)
            });
        }
        if style == Style::Cores {
            opt_row_base(ui, "meter.kernel", base.and_then(|b| b.kernel.clone()), &mut m.kernel, "bottom".into(), |ui, v| {
                choice(ui, "kernel", v, copy::KERNELS)
            });
        }
        if matches!(style, Style::Cores | Style::Engines | Style::Bars | Style::Disks | Style::Nics) {
            row(ui, "meter.bar_width", &mut m.bar_width, base.and_then(|b| b.bar_width), |ui, v| {
                let mut on = v.is_some();
                if ui.checkbox(&mut on, "").changed() {
                    *v = on.then_some(5.0);
                }
                match v {
                    Some(w) => number(ui, w, 1.0..=20.0, ""),
                    None => {
                        ui.label(RichText::new("auto").color(DIM));
                    }
                }
            });
        }
        if style == Style::Engines {
            opt_row_base(ui, "meter.vendors", base.and_then(|b| b.vendors.clone()), &mut m.vendors, true, |ui, v| {
                ui.checkbox(v, "");
            });
            opt_row_base(ui, "meter.vendor_label", base.and_then(|b| b.vendor_label.clone()), &mut m.vendor_label, "short".into(), |ui, v| {
                choice(ui, "vendor_label", v, copy::VENDOR_LABELS)
            });
            opt_row_base(ui, "meter.hold_seconds", base.and_then(|b| b.hold_seconds.clone()), &mut m.hold_seconds, 15, |ui, v| number(ui, v, 0..=3600, " s"));
            opt_row_base(ui, "meter.engines_fall_off", base.and_then(|b| b.engines_fall_off.clone()), &mut m.engines_fall_off, false, |ui, v| {
                ui.checkbox(v, "");
            });
        }
        if matches!(style, Style::Disks | Style::Nics) {
            let id = if style == Style::Disks { "meter.hold_seconds.disks" } else { "meter.hold_seconds.nics" };
            opt_row_base(ui, id, base.and_then(|b| b.hold_seconds.clone()), &mut m.hold_seconds, 15, |ui, v| number(ui, v, 0..=3600, " s"));
        }
        if style == Style::Pie {
            opt_row_base(ui, "meter.per_gpu", base.and_then(|b| b.per_gpu.clone()), &mut m.per_gpu, false, |ui, v| {
                ui.checkbox(v, "");
            });
            opt_row_base(ui, "meter.label_inside", base.and_then(|b| b.label_inside.clone()), &mut m.label_inside, "never".into(), |ui, v| {
                choice(ui, "inside", v, copy::INSIDE)
            });
            opt_row_base(ui, "meter.slice_glow", base.and_then(|b| b.slice_glow.clone()), &mut m.slice_glow, String::new(), |ui, v| {
                ui.text_edit_singleline(v);
            });
            opt_row_base(ui, "meter.slice_glow_extend", base.and_then(|b| b.slice_glow_extend), &mut m.slice_glow_extend, 2.5, |ui, v| {
                ui.add(egui::Slider::new(v, 0.0..=8.0));
            });
            opt_row_base(ui, "meter.slice_glow_radius", base.and_then(|b| b.slice_glow_radius), &mut m.slice_glow_radius, 4.0, |ui, v| {
                ui.add(egui::Slider::new(v, 1.0..=24.0));
            });
            opt_row_base(ui, "meter.slice_glow_strength", base.and_then(|b| b.slice_glow_strength), &mut m.slice_glow_strength, 1.0, |ui, v| {
                ui.add(egui::Slider::new(v, 0.1..=1.0));
            });
            opt_row_base(ui, "meter.slice_glow_color", base.and_then(|b| b.slice_glow_color.clone()), &mut m.slice_glow_color, String::new(), |ui, v| {
                ui.text_edit_singleline(v);
            });
            opt_row_base(ui, "meter.slice_glow_pulse_ms", base.and_then(|b| b.slice_glow_pulse_ms), &mut m.slice_glow_pulse_ms, 0, |ui, v| {
                ui.add(egui::Slider::new(v, 0..=10_000));
            });
            opt_row_base(ui, "meter.pressure", base.and_then(|b| b.pressure.clone()), &mut m.pressure, 0.8, |ui, v| {
                ui.add(egui::Slider::new(v, 0.5..=0.99));
            });
        }
        if style == Style::History {
            opt_row_base(ui, "meter.time_axis", base.and_then(|b| b.time_axis.clone()), &mut m.time_axis, "log".into(), |ui, v| {
                choice(ui, "time", v, copy::TIME_AXES)
            });
            opt_row_base(ui, "meter.history_seconds", base.and_then(|b| b.history_seconds.clone()), &mut m.history_seconds, 300, |ui, v| number(ui, v, 10..=86_400, " s"));
        }
        row(ui, "meter.show_above", &mut m.show_above, base.and_then(|b| b.show_above.clone()), |ui, v| max_editor(ui, "show_above", v));
        row(ui, "meter.show_when", &mut m.show_when, base.and_then(|b| b.show_when.clone()), show_when_editor);
    });
}

/// `show_when`: off, or an expression with the level it shows at and the one it leaves under.
fn show_when_editor(ui: &mut egui::Ui, value: &mut Option<ShowWhen>) {
    let mut on = value.is_some();
    if ui.checkbox(&mut on, "").changed() {
        *value = on.then(|| ShowWhen { expr: "1 - mem_avail / ram_total".into(), at: 0.85, until: Some(0.8) });
    }
    if let Some(w) = value {
        text(ui, &mut w.expr, "For example, 1 - mem_avail / ram_total", 240.0);
        ui.label("Show at");
        ui.add(egui::DragValue::new(&mut w.at).speed(0.01));
        ui.label("Hide below");
        let mut until = w.until.unwrap_or(w.at);
        if ui.add(egui::DragValue::new(&mut until).speed(0.01)).changed() {
            w.until = Some(until);
        }
    }
}

/// tint, grain and gaps, for the dark look (`[backdrop]`) or the light one (`[backdrop.light]`).
fn look_rows(ui: &mut egui::Ui, tint: &mut Option<String>, grain: &mut Option<f32>, gaps: &mut Option<String>, default_tint: &str) {
    opt_row(ui, "look.tint", tint, default_tint.into(), color);
    opt_row(ui, "look.grain", grain, config::DEFAULT_GRAIN, |ui, v| {
        knob(ui, v, 0.2);
    });
    row(ui, "look.gaps", gaps, None, |ui, v| {
        let mut on = v.is_some();
        if ui.checkbox(&mut on, "").changed() {
            *v = on.then(|| "#18FFFFFF".into());
        }
        if let Some(c) = v {
            color(ui, c);
        }
    });
}

fn backdrop(ui: &mut egui::Ui, c: &mut Config) {
    heading(ui, copy::LOOK.0, copy::LOOK.1);
    switch(ui, "Use a custom taskbar background", &mut c.backdrop, || Backdrop { tint: Some(config::DARK_TINT.into()), grain: Some(config::DEFAULT_GRAIN), gaps: None, light: None });
    section(ui, "Theme", |ui| {
        row(ui, "theme", &mut c.theme, "auto".into(), |ui, v| choice(ui, "theme", v, copy::THEMES));
    });
    let Some(b) = &mut c.backdrop else { return };
    section(ui, "Dark mode", |ui| look_rows(ui, &mut b.tint, &mut b.grain, &mut b.gaps, config::DARK_TINT));
    let light = b.light.get_or_insert_with(Look::default);
    section(ui, "Light mode", |ui| look_rows(ui, &mut light.tint, &mut light.grain, &mut light.gaps, config::LIGHT_TINT));
    if *light == Look::default() {
        b.light = None;
    }
}

fn twatch(ui: &mut egui::Ui, m: &mut model::Model) {
    ui.label(copy::TWATCH.1);
    missing_part(ui, pms::parts::TWATCH);
    ui.add_space(8.0);
    // Streamlink first: without it, there's no live video.
    let stuck = m.config.twatch.as_ref().is_some_and(|t| !t.favorites.is_empty() && t.pictures != "always");
    ui.strong("Streamlink");
    let job = m.streamlink_job.lock().unwrap().clone();
    let busy = matches!(job, model::WadJob::Getting(..));
    if stuck && !busy && pms::streamlink::exe().is_none() {
        ui.colored_label(egui::Color32::from_rgb(0xE0, 0xA0, 0x30), format!("{}. {}", copy::TWATCH_NO_STREAMLINK.0, copy::TWATCH_NO_STREAMLINK.1));
    }
    ui.label(format!("Live video needs it, to turn a channel into a stream. About 85 MB: the newest stable build, from {}", pms::streamlink::URL));
    if let Some(version) = pms::streamlink::version() {
        ui.label(format!("Streamlink {version} is installed. {}", m.streamlink_note.lock().unwrap()));
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new("Remove")).clicked() {
                m.remove_streamlink();
            }
            if ui.add_enabled(!busy, egui::Button::new("Get the newest")).clicked() {
                m.get_streamlink();
            }
        });
    } else if ui.add_enabled(!busy, egui::Button::new("Download")).clicked() {
        m.get_streamlink();
    }
    match job {
        model::WadJob::Getting(done, total) => {
            ui.label(format!("Downloading... {} of {} MB", done >> 20, total >> 20));
        }
        model::WadJob::Failed(e) => {
            ui.label(format!("That didn't work: {e}"));
        }
        model::WadJob::Idle => {}
    }
    ui.add_space(8.0);
    let mut on = m.twatch_on();
    if ui.checkbox(&mut on, "Twatch TV on your taskbar").changed() {
        m.set_twatch_on(on);
    }
    ui.add_space(8.0);
    ui.strong("Favorites");
    ui.label(copy::TWATCH_FAVORITES);
    if m.favorites_text.is_some() {
        ui.label(copy::TWATCH_AS_TEXT);
        if let Some(text) = &mut m.favorites_text {
            ui.add(egui::TextEdit::multiline(text).desired_rows(8));
        }
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                m.save_favorites_text();
            }
            if ui.button("Cancel").clicked() {
                m.cancel_favorites_text();
            }
        });
    }
    let as_text = m.favorites_text.is_some();
    let t = m.twatch();
    let mut swap = None;
    let count = if as_text { 0 } else { t.favorites.len() };
    for i in 0..count {
        ui.horizontal(|ui| {
            if ui.add_enabled(i > 0, egui::Button::new("Up")).clicked() {
                swap = Some((i, i - 1));
            }
            if ui.add_enabled(i + 1 < count, egui::Button::new("Down")).clicked() {
                swap = Some((i, i + 1));
            }
            ui.label(format!("{}.", i + 1));
            ui.label(t.favorites[i].as_str());
        });
    }
    if let Some((a, b)) = swap {
        t.favorites.swap(a, b);
    }
    if !as_text {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut m.new_favorite).hint_text("A channel's name, or a link to it"));
            if ui.button("Add").clicked() {
                m.add_favorite();
            }
            if ui.button("Edit as text").clicked() {
                m.favorites_as_text();
            }
        });
    }
    if !m.favorites_note.is_empty() {
        ui.label(m.favorites_note.as_str());
    }
    let t = m.twatch();
    ui.add_space(8.0);
    ui.strong("How it plays");
    ui.horizontal(|ui| {
        ui.label(copy::setting("twatch.volume").0).on_hover_text(copy::setting("twatch.volume").1);
        ui.add(egui::DragValue::new(&mut t.volume).range(0..=100).suffix("%"));
    });
    ui.horizontal(|ui| {
        ui.label(copy::setting("twatch.higher_favorite").0);
        egui::ComboBox::from_id_salt("twatch-higher").selected_text(copy::TWATCH_HIGHER.iter().find(|(v, _)| *v == t.higher_favorite).map_or("", |(_, l)| *l)).show_ui(ui, |ui| {
            for (value, label) in copy::TWATCH_HIGHER {
                ui.selectable_value(&mut t.higher_favorite, value.to_string(), label);
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label(copy::setting("twatch.check_seconds").0);
        egui::ComboBox::from_id_salt("twatch-check").selected_text(copy::TWATCH_CHECKS.iter().find(|(v, _)| v.parse() == Ok(t.check_seconds)).map_or("", |(_, l)| *l)).show_ui(ui, |ui| {
            for (value, label) in copy::TWATCH_CHECKS {
                ui.selectable_value(&mut t.check_seconds, value.parse().unwrap_or(pms::twatch::live::USUAL_CHECK), label);
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label(copy::setting("twatch.check_extra_seconds").0).on_hover_text(copy::setting("twatch.check_extra_seconds").1);
        egui::ComboBox::from_id_salt("twatch-check-extra").selected_text(copy::TWATCH_CHECK_EXTRAS.iter().find(|(v, _)| v.parse() == Ok(t.check_extra_seconds)).map_or("", |(_, l)| *l)).show_ui(ui, |ui| {
            for (value, label) in copy::TWATCH_CHECK_EXTRAS {
                ui.selectable_value(&mut t.check_extra_seconds, value.parse().unwrap_or(pms::twatch::live::USUAL_CHECK_EXTRA), label);
            }
        });
    });
    ui.checkbox(&mut t.notify_live, copy::setting("twatch.notify_live").0);
    ui.horizontal(|ui| {
        ui.label("Highest quality");
        egui::ComboBox::from_id_salt("twatch-quality").selected_text(t.top_quality.clone()).show_ui(ui, |ui| {
            for (value, label) in copy::TWATCH_QUALITIES {
                ui.selectable_value(&mut t.top_quality, value.to_string(), label);
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label(copy::setting("twatch.media_controls").0).on_hover_text(copy::setting("twatch.media_controls").1);
        egui::ComboBox::from_id_salt("twatch-media").selected_text(copy::TWATCH_MEDIA_CONTROLS.iter().find(|(v, _)| *v == t.media_controls).map_or("", |(_, l)| *l)).show_ui(ui, |ui| {
            for (value, label) in copy::TWATCH_MEDIA_CONTROLS {
                ui.selectable_value(&mut t.media_controls, value.to_string(), label);
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label(copy::setting("twatch.more_sets").0);
        ui.add(egui::Slider::new(&mut t.more_sets, 0..=pms::twatch::MOST_MORE_SETS));
    });
    ui.add_space(8.0);
    ui.small(copy::TWATCH_NETWORK);
    ui.small(copy::TWATCH_POWER);
    ui.small(copy::ORDER_HINT);
    ui.small(copy::TWATCH_NOTICE);
}

/// The proxy, and every way PimpMyShell uses the network, each with its switch.
fn network(ui: &mut egui::Ui, m: &mut model::Model) {
    use pms::net::Use;
    ui.label(copy::NETWORK.1);
    ui.add_space(8.0);
    if let Some(e) = pms::config::network_trouble(&m.path) {
        ui.colored_label(egui::Color32::LIGHT_RED, format!("Your network settings didn't load. {}", copy::NETWORK_UNREAD));
        ui.label(RichText::new(pms::problem::Problem::of(&pms::config::network_path(&m.path), &e).words()).monospace());
        if ui.button("Show in File Explorer").clicked() {
            m.show_network();
        }
        ui.add_space(8.0);
    }
    ui.strong(copy::PROXY.0);
    ui.label(copy::PROXY.1);
    let applied = m.config.network.as_ref().map(|n| n.proxy.clone()).filter(|p| p != pms::config::NETWORK_UNREAD).unwrap_or_default();
    let typed = m.proxy_text.trim().to_string();
    let wrong = (!typed.is_empty()).then(|| pms::net::check(&typed).err()).flatten();
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut m.proxy_text).hint_text("socks5h://127.0.0.1:9050").desired_width(260.0));
        if ui.add_enabled(typed != applied && wrong.is_none(), egui::Button::new(if typed.is_empty() { "Remove" } else { "Apply" })).clicked() {
            m.apply_proxy();
        }
        if ui.add_enabled(!applied.is_empty() && typed == applied, egui::Button::new("Test")).clicked() {
            m.test_proxy();
        }
    });
    if let Some(wrong) = wrong {
        ui.colored_label(egui::Color32::from_rgb(0xE0, 0x60, 0x60), wrong);
    }
    if !applied.is_empty() && !m.uses_proxy() {
        ui.colored_label(egui::Color32::from_rgb(0xE0, 0xB0, 0x40), format!("{}. {}", copy::PROXY_UNUSED.0, copy::PROXY_UNUSED.1));
    }
    ui.small(copy::PROXY_LIMITS);
    let said = m.proxy_note.lock().unwrap().clone();
    if !said.is_empty() {
        ui.label(said);
    }
    ui.checkbox(&mut m.network().isolate, copy::PROXY_ISOLATE.0).on_hover_text(copy::PROXY_ISOLATE.1);
    ui.add_space(8.0);
    let all_on = { let n = m.network(); n.live_checks && n.streams && n.updates && n.downloads };
    ui.horizontal(|ui| {
        ui.strong("What PimpMyShell uses the network for");
        if ui.button(if all_on { "Turn off all" } else { "Turn on all" }).clicked() {
            m.all_through_proxy(!all_on);
        }
    });
    let notes = m.use_notes.lock().unwrap().clone();
    let mut last_who = "";
    for (at, (what, title, who, words)) in pms::net::USES.iter().enumerate() {
        if *who != last_who {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(*who).italics());
            last_who = who;
        }
        ui.horizontal(|ui| {
            let n = m.network();
            let on = match what {
                Use::Live => &mut n.live_checks,
                Use::Streams | Use::Video => &mut n.streams,
                Use::Updates => &mut n.updates,
                Use::Downloads => &mut n.downloads,
            };
            ui.checkbox(on, format!("{title}: use proxy")).on_hover_text(*words);
            if ui.add_enabled(notes[at] != "Trying...", egui::Button::new("Test")).clicked() {
                m.try_use(at);
            }
        });
        ui.small(*words);
        if !notes[at].is_empty() {
            ui.label(notes[at].as_str());
        }
    }
    ui.add_space(8.0);
    ui.small(copy::PROXY_STRICT);
    ui.small(copy::NETWORK_ELSE);
}

fn tv(ui: &mut egui::Ui, m: &mut model::Model) {
    ui.label(copy::TV.1);
    ui.add_space(8.0);
    let sets = m.tvs().len();
    ui.horizontal(|ui| {
        ui.strong(format!("TVs on your taskbar: {sets}"));
        if ui.button("Add a TV").clicked() {
            m.add_tv();
        }
        if ui.add_enabled(sets > 0, egui::Button::new("Remove one")).clicked() {
            m.remove_tv();
        }
    });
    if sets == 1 && pms::tv::console_running() {
        ui.label("Removing your last TV quits POOM, and anything you haven't saved is lost.");
    }
    ui.checkbox(&mut m.config.tv_intro, copy::setting("tv_intro").0).on_hover_text(copy::setting("tv_intro").1);
    if m.config.tv_intro {
        let named = |c: u32| copy::TV_CHANNELS.iter().find(|(v, _)| v.parse() == Ok(c)).map_or(String::new(), |(_, l)| (*l).to_string());
        for (id, salt, channel) in [("tv_intro_from", "surf-from", &mut m.config.tv_intro_from), ("tv_intro_to", "surf-to", &mut m.config.tv_intro_to)] {
            ui.horizontal(|ui| {
                ui.label(copy::setting(id).0).on_hover_text(copy::setting(id).1);
                egui::ComboBox::from_id_salt(salt).selected_text(named(*channel)).show_ui(ui, |ui| {
                    for (value, label) in copy::TV_CHANNELS {
                        ui.selectable_value(channel, value.parse().unwrap_or(0), label);
                    }
                });
            });
        }
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(copy::setting("shuffle_seconds").0).on_hover_text(copy::setting("shuffle_seconds").1);
        ui.add(egui::DragValue::new(&mut m.config.shuffle_seconds).range(5..=3600).suffix(" s"));
    });
    ui.checkbox(&mut m.config.shuffle_jitter, copy::setting("shuffle_jitter").0).on_hover_text(copy::setting("shuffle_jitter").1);
    ui.add_space(8.0);
    ui.small(copy::TV_HELP);
    ui.small(copy::ORDER_HINT);
}

/// The console on channel 6: which game is in it, more to get, and whether a TV tuning in switches it on.
/// When PimpMyShell gets out of the way: for a game, for the frame rate, on battery, and Twatch TV's pictures instead of video.
fn power(ui: &mut egui::Ui, m: &mut model::Model) {
    heading(ui, copy::POWER.0, copy::POWER.1);
    let c = &mut m.config;
    section(ui, "When nobody can see it", |ui| {
        row(ui, "rest_when_covered", &mut c.rest_when_covered, true, |ui, v| {
            ui.checkbox(v, "");
        });
    });
    section(ui, "While you play", |ui| {
        row(ui, "rest_for_games", &mut c.rest_for_games, true, |ui, v| {
            ui.checkbox(v, "");
        });
    });
    section(ui, "Animations", |ui| {
        row(ui, "max_fps", &mut c.max_fps, None, |ui, v| {
            let mut on = v.is_some();
            if ui.checkbox(&mut on, "").changed() {
                *v = on.then_some(30);
            }
            match v {
                Some(f) => number(ui, f, 0..=240, " fps"),
                None => {
                    ui.label(RichText::new("auto").color(DIM));
                }
            }
        });
        opt_row(ui, "battery_fps", &mut c.battery_fps, 30, |ui, v| number(ui, v, 0..=240, " fps"));
    });
    // Twatch TV's, read without making it a section of the settings: that's only for changing one.
    let d = pms::config::Twatch::default();
    let mut t = c.twatch.clone().unwrap_or_default();
    let was = (t.pictures.clone(), t.metered_pictures);
    section(ui, "Twatch TV", |ui| {
        row(ui, "twatch.pictures", &mut t.pictures, d.pictures.clone(), |ui, v| choice(ui, "twatch-pictures", v, &copy::TWATCH_PICTURES));
        row(ui, "twatch.metered_pictures", &mut t.metered_pictures, d.metered_pictures, |ui, v| {
            ui.checkbox(v, "");
        });
    });
    if (t.pictures.clone(), t.metered_pictures) != was {
        let mine = m.twatch();
        (mine.pictures, mine.metered_pictures) = (t.pictures, t.metered_pictures);
    }
    ui.add_space(6.0);
    ui.small(copy::POWER_BY_ITSELF);
}

/// A program of PimpMyShell's that isn't beside this one (pms::parts), said at the top of the page of what needs it.
fn missing_part(ui: &mut egui::Ui, file: &'static str) {
    if !pms::parts::here(file) {
        ui.add_space(6.0);
        ui.label(RichText::new(format!("{file} is missing")).strong().color(Color32::LIGHT_RED));
        ui.label(copy::part_missing(file));
    }
}

fn poom(ui: &mut egui::Ui, m: &mut model::Model) {
    use pms::wads::Wad;
    ui.label(copy::POOM.1);
    missing_part(ui, pms::parts::POOM);
    ui.add_space(8.0);
    let wads = pms::wads::list();
    let playing = pms::wads::chosen().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    ui.strong("Which game plays");
    if wads.is_empty() {
        ui.label("No games yet. Get one below, and it goes on channel 6.");
    }
    for file in &wads {
        ui.horizontal(|ui| {
            let is_on = playing.as_deref() == Some(file.as_str());
            if ui.radio(is_on, pms::wads::title(file)).clicked() {
                m.choose_wad(file);
            }
            if ui.small_button("Remove").clicked() {
                m.remove_wad(file);
            }
        });
    }
    ui.add_space(8.0);
    ui.strong("Get more games");
    let job = m.wad_job.lock().unwrap().clone();
    let busy = matches!(job, model::WadJob::Getting(..));
    for (file, wad, title, help, label) in [
        ("freedoom1.wad", Wad::Freedoom, "Freedoom", format!("Free, with its own art and levels, under a BSD license. 24 MB.\nFrom {}", pms::wads::url(Wad::Freedoom)), "Download"),
        ("doom1.wad", Wad::Shareware, "Doom (shareware episode)", format!("The first episode, unmodified. 1.4 MB. You'll see id Software's license first, and nothing is downloaded unless you agree.\nFrom {}", pms::wads::url(Wad::Shareware)), "Download"),
    ] {
        if wads.iter().any(|w| w.eq_ignore_ascii_case(file)) {
            continue;
        }
        ui.label(format!("{title}: {help}"));
        if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
            m.get_wad(wad);
        }
        ui.add_space(6.0);
    }
    ui.label("A game of your own: put an IWAD, like doom2.wad, in this folder, and it appears above.");
    if ui.button("Open folder").clicked() {
        m.show_wads();
    }
    match job {
        model::WadJob::Getting(0, 0) => {
            ui.label("Waiting for you...");
        }
        model::WadJob::Getting(done, total) => {
            ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32));
        }
        model::WadJob::Failed(e) => {
            ui.label(format!("That didn't work: {e}"));
        }
        model::WadJob::Idle => {}
    }
    ui.add_space(8.0);
    ui.checkbox(&mut m.config.console_autostart, copy::setting("console_autostart").0).on_hover_text(copy::setting("console_autostart").1);
    ui.add_space(8.0);
    ui.small(copy::POOM_HELP);
    ui.small(copy::TV_NOTICE);
}

fn hotmic(ui: &mut egui::Ui, h: &mut Option<Hotmic>) {
    heading(ui, copy::HOTMIC.0, copy::HOTMIC.1);
    switch(ui, "Alert me when an app uses my microphone", h, Hotmic::default);
    let Some(h) = h else { return };
    let d = Hotmic::default();
    section(ui, "Alert", |ui| {
        row(ui, "hotmic.color", &mut h.color, d.color.clone(), color);
        row(ui, "hotmic.period_ms", &mut h.period_ms, d.period_ms, |ui, v| number(ui, v, 0..=10_000, " ms"));
        row(ui, "hotmic.low", &mut h.low, d.low, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "hotmic.high", &mut h.high, d.high, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "hotmic.broad_ms", &mut h.broad_ms, d.broad_ms, |ui, v| number(ui, v, 0..=60_000, " ms"));
        row(ui, "hotmic.position", &mut h.position, d.position.clone(), |ui, v| choice(ui, "mic-position", v, copy::MIC_POSITIONS));
        row(ui, "hotmic.ignore", &mut h.ignore, Vec::new(), |ui, v| string_list(ui, "ignore", v, "For example, obs64.exe"));
    });
}

fn toasts(ui: &mut egui::Ui, t: &mut Option<Toasts>) {
    heading(ui, copy::TOASTS.0, copy::TOASTS.1);
    // Off with its settings kept (`enabled = false`, as a notice's *Keep Windows notifications* leaves it), it comes
    // on again with them.
    let mut on = t.as_ref().is_some_and(|t| t.enabled);
    if ui.checkbox(&mut on, RichText::new("Notifications in the taskbar").size(14.0)).changed() {
        match (on, t.as_mut()) {
            (true, Some(t)) => t.enabled = true,
            (true, None) => *t = Some(Toasts::default()),
            (false, _) => *t = None,
        }
    }
    ui.add_space(4.0);
    let Some(t) = t.as_mut().filter(|t| t.enabled) else { return };
    let d = Toasts::default();
    section(ui, "Taskbar notifications", |ui| {
        row(ui, "toasts.duration_ms", &mut t.duration_ms, d.duration_ms, |ui, v| number(ui, v, 1000..=60_000, " ms"));
        row(ui, "toasts.confirm_clicks", &mut t.confirm_clicks, d.confirm_clicks.clone(), |ui, v| choice(ui, "toast-confirm", v, copy::CONFIRMS));
        row(ui, "toasts.position", &mut t.position, d.position.clone(), |ui, v| choice(ui, "toast-position", v, copy::TOAST_POSITIONS));
        row(ui, "toasts.style", &mut t.style, d.style.clone(), |ui, v| choice(ui, "toast-style", v, copy::TOAST_STYLES));
        row(ui, "toasts.ignore", &mut t.ignore, Vec::new(), |ui, v| string_list(ui, "toast-ignore", v, "For example, Teams"));
    });
}

fn start_glow(ui: &mut egui::Ui, g: &mut Option<StartGlow>) {
    ui.add_space(10.0);
    switch(ui, "Glow behind the Start button", g, StartGlow::default);
    let Some(g) = g else { return };
    let d = StartGlow::default();
    section(ui, "Start button", |ui| {
        row(ui, "start_glow.color", &mut g.color, d.color.clone(), color);
        row(ui, "start_glow.hover", &mut g.hover, d.hover, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "start_glow.press", &mut g.press, d.press, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "start_glow.hover_ms", &mut g.hover_ms, d.hover_ms, |ui, v| number(ui, v, 0..=2000, " ms"));
        row(ui, "start_glow.open", &mut g.open, d.open, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "start_glow.press_ms", &mut g.press_ms, d.press_ms, |ui, v| number(ui, v, 0..=2000, " ms"));
        row(ui, "start_glow.hover_radius", &mut g.hover_radius, d.hover_radius, |ui, v| number(ui, v, 0.0..=600.0, " DIP"));
        row(ui, "start_glow.press_radius", &mut g.press_radius, d.press_radius, |ui, v| number(ui, v, 0.0..=600.0, " DIP"));
    });
}

fn app_glow(ui: &mut egui::Ui, g: &mut Option<AppGlow>) {
    ui.add_space(10.0);
    switch(ui, "Glow behind app buttons", g, AppGlow::default);
    let Some(g) = g else { return };
    let d = AppGlow::default();
    section(ui, "App buttons", |ui| {
        row(ui, "app_glow.hover", &mut g.hover, d.hover, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "app_glow.press", &mut g.press, d.press, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "app_glow.hover_ms", &mut g.hover_ms, d.hover_ms, |ui, v| number(ui, v, 0..=2000, " ms"));
        row(ui, "app_glow.press_ms", &mut g.press_ms, d.press_ms, |ui, v| number(ui, v, 0..=2000, " ms"));
        row(ui, "app_glow.hover_radius", &mut g.hover_radius, d.hover_radius, |ui, v| number(ui, v, 0.0..=600.0, " DIP"));
        row(ui, "app_glow.press_radius", &mut g.press_radius, d.press_radius, |ui, v| number(ui, v, 0.0..=600.0, " DIP"));
    });
}

/// A 0..`full` knob (pms-ui).
fn knob(ui: &mut egui::Ui, value: &mut f32, full: f32) {
    pms_ui::knob(ui, value, 0.0, full);
}
