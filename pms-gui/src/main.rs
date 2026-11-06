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
mod winui;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText};
use pms::config::{self, Agg, AppGlow, Backdrop, Config, Exprs, Hotmic, Look, Max, MeterSpec, ShowWhen, StartGlow, Style, Toasts};
use pms::update::Status;

use model::{Model, Update, toml_meter};


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
    if !flag("--egui") && (flag("--winui") || winui::available()) {
        match winui::run(path.clone()) {
            Ok(()) => return Ok(()),
            // It can't start (only once a process: Application::Start), so the egui one does.
            Err(e) => eprintln!("WinUI 3: {e}; the egui window instead"),
        }
    }
    egui_main(path)
}

fn egui_main(path: PathBuf) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: {
            let v = egui::ViewportBuilder::default().with_title("PimpMyShell").with_inner_size([960.0, 700.0]).with_min_inner_size([760.0, 460.0]);
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
            Ok(Box::new(Gui::open(path)))
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
    ctx.style_mut(|s| {
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
    Meters,
    Counters,
    Backdrop,
    Hotmic,
    Toasts,
}

const PAGES: [(Page, &str, &str); 6] = [
    (Page::General, "General", "Status, startup, and updates"),
    (Page::Meters, "Meters", "The meters next to the system tray"),
    (Page::Counters, "Performance counters", "What your meters read"),
    (Page::Backdrop, "Taskbar look", "Background and glows"),
    (Page::Hotmic, "Microphone alert", "Know when an app uses your microphone"),
    (Page::Toasts, "Notifications", "Notifications in the taskbar"),
];

/// The egui window: the model (model.rs, which the WinUI 3 window shares), and which page and meter it shows.
struct Gui {
    m: Model,
    page: Page,
    meter: usize,
    /// This PC can show the new (WinUI 3) window: the General page offers it.
    winui: bool,
}

impl Gui {
    fn open(path: PathBuf) -> Gui {
        Gui { m: Model::open(path), page: Page::General, meter: 0, winui: winui::available() }
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.polled.elapsed() > Duration::from_secs(2) {
            self.poll_daemon();
        }
        let status_bar = egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 6)).fill(Color32::from_rgb(0x15, 0x16, 0x19));
        egui::TopBottomPanel::bottom("status").frame(status_bar).show(ctx, |ui| {
            let (text, error) = &self.message;
            ui.label(RichText::new(text).color(if *error { ERROR } else { DIM }));
        });
        let sidebar = egui::Frame::new().fill(Color32::from_rgb(0x16, 0x17, 0x1A)).inner_margin(12);
        egui::SidePanel::left("pages").resizable(false).exact_width(180.0).frame(sidebar).show(ctx, |ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("PimpMyShell").size(22.0).strong().color(Color32::WHITE));
            ui.label(RichText::new(format!("Version {}", pms::version::CURRENT)).small().color(DIM));
            ui.add_space(14.0);
            for (page, name, hint) in PAGES {
                let selected = self.page == page;
                let r = if page == Page::General {
                    // The first stop, larger, with the one thing to know at a glance: whether the daemon runs.
                    let (state, color) = if self.daemon_running { ("● Running", OK) } else { ("● Not running", AMBER) };
                    let mut job = egui::text::LayoutJob::default();
                    let (style, align) = (ui.style().clone(), egui::Align::LEFT);
                    RichText::new(name).size(17.0).strong().color(Color32::WHITE).append_to(&mut job, &style, egui::FontSelection::Default, align);
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
                    let text = RichText::new(name).size(15.0).color(if selected { Color32::WHITE } else { Color32::from_gray(185) });
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
        egui::CentralPanel::default().frame(body).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.page {
                Page::General => self.general(ui),
                Page::Meters => self.meters(ui),
                Page::Counters => self.counter_rows(ui),
                Page::Backdrop => {
                    backdrop(ui, &mut self.config);
                    start_glow(ui, &mut self.config.start_glow);
                    app_glow(ui, &mut self.config.app_glow);
                }
                Page::Hotmic => hotmic(ui, &mut self.config.hotmic),
                Page::Toasts => toasts(ui, &mut self.config.toasts),
            });
        });
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

            ui.label("Last log entry");
            ui.add(egui::Label::new(RichText::new(&self.last_log).monospace().small().color(DIM)).truncate());
            ui.label("");
            ui.end_row();

            ui.label("");
            ui.scope(|ui| {
                ui.set_max_width(460.0);
                ui.add(egui::Label::new(RichText::new(copy::LOG_PRIVACY).small().color(DIM)).wrap());
            });
            ui.label("");
            ui.end_row();

            ui.label("Start when I sign in").on_hover_text("Run PimpMyShell automatically when you sign in to Windows");
            let mut on = self.autostarts();
            if ui.checkbox(&mut on, "").changed() {
                self.set_autostart(on);
            }
            ui.label("");
            ui.end_row();

            ui.label("Settings file");
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.path.display().to_string()).monospace().small());
                if ui.small_button("Show in File Explorer").clicked() {
                    self.show_config();
                }
            });
            ui.label("");
            ui.end_row();
        });

        let c = &mut self.config;
        section(ui, "Taskbar", |ui| {
            row(ui, "theme", &mut c.theme, "auto".into(), |ui, v| choice(ui, "theme", v, copy::THEMES));
            row(ui, "meter_height", &mut c.meter_height, "tall".into(), |ui, v| {
                choice(ui, "height", v, copy::HEIGHTS)
            });
            row(ui, "interval_ms", &mut c.interval_ms, 1000, |ui, v| number(ui, v, 16..=60_000, " ms"));
            row(ui, "software_gpus", &mut c.software_gpus, false, |ui, v| {
                ui.checkbox(v, "");
            });
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
            row(ui, "value_font", &mut c.value_font, None, |ui, v| {
                let mut s = v.clone().unwrap_or_default();
                text(ui, &mut s, "Cascadia Mono", 200.0);
                *v = Some(s).filter(|s| !s.is_empty());
            });
        });

        self.software_update(ui);
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
        ui.label(RichText::new(format!("Checks start at {}", pms::update::first_url(latest))).small().color(DIM));
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
        let interval = self.config.interval_ms;
        // What's shipped that you've removed, a click from coming back.
        let missing: Vec<String> = self.m.missing_meters().iter().map(|m| m.label.clone()).collect();
        let mut restore = None;
        let meters = &mut self.m.config.meters;
        let baseline = &self.m.baseline;
        self.meter = self.meter.min(meters.len().saturating_sub(1));
        ui.horizontal_top(|ui| {
            // The list: a swatch, the label, its style.
            ui.vertical(|ui| {
                ui.set_width(230.0);
                let mut action = None;
                let mut group: Option<&str> = None;
                for (i, m) in meters.iter().enumerate() {
                    if i > 0 && m.group.as_deref() != group {
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
                                    if ui.small_button("↓").on_hover_text("Move down").clicked() {
                                        action = Some(("down", i));
                                    }
                                    if ui.small_button("↑").on_hover_text("Move up").clicked() {
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
                match action {
                    Some(("remove", i)) => {
                        meters.remove(i);
                    }
                    Some(("up", i)) if i > 0 => {
                        meters.swap(i, i - 1);
                        self.meter = i - 1;
                    }
                    Some(("down", i)) if i + 1 < meters.len() => {
                        meters.swap(i, i + 1);
                        self.meter = i + 1;
                    }
                    _ => {}
                }
                ui.add_space(6.0);
                if ui.button("+ Add meter").clicked() {
                    meters.push(toml_meter(model::NEW_METER));
                    self.meter = meters.len() - 1;
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
            ui.vertical(|ui| match meters.get_mut(self.meter) {
                Some(m) => {
                    let base = baseline.meters.iter().find(|b| b.label.eq_ignore_ascii_case(&m.label)).cloned();
                    meter_editor(ui, m, base.as_ref(), interval)
                }
                None => {
                    ui.label("You don't have any meters. Select + Add meter to make one.");
                }
            });
        });
        if let Some(at) = restore.and_then(|label: String| self.m.restore_meter(&label)) {
            self.meter = at;
        }
    }

    fn counter_rows(&mut self, ui: &mut egui::Ui) {
        heading(ui, copy::COUNTERS.0, copy::COUNTERS.1);
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
        row(ui, "meter.style", &mut m.style, Style::History, |ui, v| {
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
            row(ui, "meter.expr", &mut m.expr, expr, |ui, v| {
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
                row(ui, "meter.names", &mut m.names, Vec::new(), |ui, v| string_list(ui, "names", v, "Name"));
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
                Style::Nics => vec!["Upload".into(), "Download".into()],
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
            row(ui, "meter.max", &mut m.max, None, |ui, v| max_editor(ui, "max", v));
            row(ui, "meter.unit", &mut m.unit, String::new(), |ui, v| {
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
            row(ui, "meter.bar_width", &mut m.bar_width, None, |ui, v| {
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
        row(ui, "meter.show_above", &mut m.show_above, None, |ui, v| max_editor(ui, "show_above", v));
        row(ui, "meter.show_when", &mut m.show_when, None, show_when_editor);
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
