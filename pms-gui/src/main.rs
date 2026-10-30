//! PimpMyShell's settings: edits `pms.toml` while you watch. Every change is validated and saved half a second after
//! you stop, and the running daemon applies it within a second, so the taskbar follows along.
//!
//! Like Nagare's panel, settings go by their config names: what you see here is what the file says, and a name can be
//! looked up in pms.example.toml for the whole story. Hover a name for a hint; a name in amber differs from its
//! default, and ↺ puts the default back.

#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use pms::config::{self, Agg, AppGlow, Backdrop, Config, CounterSpec, Exprs, Hotmic, Look, Max, MeterSpec, StartGlow, Style, Toasts};
use pms::update::Status;

const SAVE_AFTER: Duration = Duration::from_millis(500);
const AMBER: Color32 = Color32::from_rgb(0xFF, 0xC1, 0x5A);
const ERROR: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x80);
const OK: Color32 = Color32::from_rgb(0x81, 0xC7, 0x84);
const DIM: Color32 = Color32::from_gray(150);
/// The config-name column.
const KEY_WIDTH: f32 = 170.0;

fn main() -> eframe::Result {
    pms::daemon::migrate();
    let path = std::env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(config::default_path);
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
    (Page::General, "General", "the daemon, starting at sign-in, updates"),
    (Page::Meters, "Meters", "the strip by the tray"),
    (Page::Counters, "Counters", "what the meters read"),
    (Page::Backdrop, "Backdrop", "the taskbar's look, dark and light"),
    (Page::Hotmic, "Hot mic", "the microphone warning"),
    (Page::Toasts, "Toasts", "notifications in the taskbar"),
];

/// A counter as an editable row (the config keeps them in a map keyed by name).
struct CounterRow {
    name: String,
    path: String,
    agg: Agg,
    default: Option<f64>,
}

/// The updater's state, written by its thread.
#[derive(Clone, Default)]
enum Update {
    #[default]
    Idle,
    Checking,
    Downloading(u64, u64),
    Done(Status),
    Failed(String),
}

struct Gui {
    path: PathBuf,
    config: Config,
    counters: Vec<CounterRow>,
    page: Page,
    meter: usize,
    /// What's on disk, to notice edits; and when the last edit happened.
    saved: String,
    edited_at: Option<Instant>,
    /// Last save outcome, or why the config can't be saved yet.
    message: (String, bool),
    daemon_running: bool,
    autostart: String,
    last_log: String,
    polled: Instant,
    update: Arc<Mutex<Update>>,
    /// `update_check` as the file has it: the daemon writes it too (the answer to its question).
    disk_update_check: Option<bool>,
}

impl Gui {
    fn open(path: PathBuf) -> Gui {
        let (config, message) = match config::load(&path) {
            Ok(c) => (c, (format!("Editing {}", path.display()), false)),
            Err(_) if !path.exists() => (example(), (format!("No config yet: starting from the example; your first change saves it to {}", path.display()), false)),
            Err(e) => (example(), (format!("Couldn't read the config ({e}); showing the example. Nothing is saved until you edit."), true)),
        };
        let counters = config
            .counters
            .iter()
            .map(|(name, spec)| match spec {
                CounterSpec::Path(p) => CounterRow { name: name.clone(), path: p.clone(), agg: Agg::Sum, default: None },
                CounterSpec::Full { path, agg, default } => CounterRow { name: name.clone(), path: path.clone(), agg: *agg, default: *default },
            })
            .collect();
        let mut gui = Gui {
            path,
            config,
            counters,
            page: Page::Meters,
            meter: 0,
            saved: String::new(),
            edited_at: None,
            message,
            daemon_running: false,
            autostart: String::new(),
            last_log: String::new(),
            polled: Instant::now() - Duration::from_secs(10),
            update: Arc::default(),
            disk_update_check: None,
        };
        gui.disk_update_check = gui.config.update_check;
        // Compare against the config as the editor states it, so opening isn't an edit.
        gui.saved = config::to_toml(&gui.current()).unwrap_or_default();
        gui.poll_daemon();
        gui
    }

    fn poll_daemon(&mut self) {
        self.daemon_running = pms::daemon::running();
        self.autostart = pms::daemon::autostart(&["status".into()]).unwrap_or_default();
        let log = config::default_path().with_file_name("daemon.log");
        self.last_log = std::fs::read_to_string(log).ok().and_then(|t| t.lines().last().map(str::to_string)).unwrap_or_default();
        self.polled = Instant::now();
        // The daemon keeps the answer to its question in the file: follow it here (unless it was changed here
        // meanwhile), so that the next save doesn't put the old value back.
        if let Ok(disk) = config::load(&self.path)
            && disk.update_check != self.disk_update_check
        {
            if self.config.update_check == self.disk_update_check {
                let clean = config::to_toml(&self.current()).is_ok_and(|t| t == self.saved);
                self.config.update_check = disk.update_check;
                if clean {
                    self.saved = config::to_toml(&self.current()).unwrap_or_default();
                }
            }
            self.disk_update_check = disk.update_check;
        }
    }

    /// The config as edited, with the counter rows folded back in.
    fn current(&self) -> Config {
        let mut c = self.config.clone();
        c.counters = self
            .counters
            .iter()
            .filter(|r| !r.name.trim().is_empty())
            .map(|r| {
                let spec = if r.agg == Agg::Sum && r.default.is_none() {
                    CounterSpec::Path(r.path.clone())
                } else {
                    CounterSpec::Full { path: r.path.clone(), agg: r.agg, default: r.default }
                };
                (r.name.trim().to_string(), spec)
            })
            .collect();
        c
    }

    /// Save once edits have settled, if the config is valid (the same check the daemon makes).
    fn autosave(&mut self) {
        let config = self.current();
        let Ok(text) = config::to_toml(&config) else { return };
        if text == self.saved {
            self.edited_at = None;
            return;
        }
        let at = *self.edited_at.get_or_insert_with(Instant::now);
        if at.elapsed() < SAVE_AFTER {
            return;
        }
        self.edited_at = None;
        if let Err(e) = pms::meters::Sampler::new(&config) {
            self.message = (format!("Not saved: {e}"), true);
            self.saved = text; // don't retry until the next edit
            return;
        }
        // Keep the hand-written original (with its comments) the first time we overwrite it.
        let backup = self.path.with_extension("toml.bak");
        if self.path.exists() && !backup.exists() {
            let _ = std::fs::copy(&self.path, &backup);
        }
        match config::save(&self.path, &config) {
            Ok(()) => {
                self.saved = text;
                self.disk_update_check = config.update_check;
                let live = if self.daemon_running { " — the taskbar follows in a moment" } else { " — start the daemon to see it" };
                self.message = (format!("Saved{live}"), false);
            }
            Err(e) => self.message = (format!("Couldn't save: {e}"), true),
        }
    }
}

fn example() -> Config {
    toml_from(config::EXAMPLE).expect("the bundled example parses")
}

fn toml_from(s: &str) -> Result<Config, String> {
    // config::load wants a file; parse the same way.
    let tmp = std::env::temp_dir().join(format!("pms-gui-{}.toml", std::process::id()));
    std::fs::write(&tmp, s).map_err(|e| e.to_string())?;
    let c = config::load(&tmp);
    let _ = std::fs::remove_file(&tmp);
    c
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
            ui.label(RichText::new(format!("P.M.S. · v{}", pms::version::CURRENT)).small().color(DIM));
            ui.add_space(14.0);
            for (page, name, hint) in PAGES {
                let selected = self.page == page;
                let text = RichText::new(name).size(15.0).color(if selected { Color32::WHITE } else { Color32::from_gray(185) });
                let r = ui.add_sized([156.0, 28.0], egui::SelectableLabel::new(selected, text));
                if r.clicked() {
                    self.page = page;
                }
                r.on_hover_text(hint);
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.add_space(4.0);
                let (text, color) = if self.daemon_running { ("● daemon running", OK) } else { ("● daemon stopped", Color32::GRAY) };
                ui.label(RichText::new(text).color(color).small());
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
        let busy = matches!(*self.update.lock().unwrap(), Update::Checking | Update::Downloading(..));
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
    egui::CollapsingHeader::new(RichText::new(title).monospace().size(14.0).color(Color32::WHITE)).default_open(true).show(ui, |ui| {
        egui::Grid::new(title).num_columns(3).striped(true).spacing([10.0, 6.0]).min_col_width(KEY_WIDTH).show(ui, body);
    });
    ui.add_space(6.0);
}

/// One setting: its config name (amber when it isn't the default), its control, and ↺ to put the default back.
fn row<T: PartialEq + Clone>(ui: &mut egui::Ui, key: &str, help: &str, value: &mut T, default: T, control: impl FnOnce(&mut egui::Ui, &mut T)) {
    let changed = *value != default;
    let text = RichText::new(key).monospace().color(if changed { AMBER } else { Color32::from_gray(215) });
    ui.add(egui::Label::new(text).sense(egui::Sense::hover())).on_hover_text(help);
    ui.horizontal_top(|ui| control(ui, value));
    if changed && ui.small_button("↺").on_hover_text("back to the default").clicked() {
        *value = default;
    } else if !changed {
        ui.label("");
    }
    ui.end_row();
}

/// A setting the file may leave out: it shows what's in effect, the default, until changed; changed back to the
/// default, it's left out again.
fn opt_row<T: PartialEq + Clone>(ui: &mut egui::Ui, key: &str, help: &str, value: &mut Option<T>, default: T, control: impl FnOnce(&mut egui::Ui, &mut T)) {
    let mut v = value.clone().unwrap_or_else(|| default.clone());
    row(ui, key, help, &mut v, default.clone(), control);
    if value.as_ref() != Some(&v) && (value.is_some() || v != default) {
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

fn choice(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[(&str, &str)]) {
    egui::ComboBox::from_id_salt(id).width(120.0).selected_text(value.clone()).show_ui(ui, |ui| {
        for (o, hint) in options {
            ui.selectable_value(value, o.to_string(), *o).on_hover_text(*hint);
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
            if ui.small_button("+ add").clicked() {
                items.push(String::new());
            }
        });
    });
}

fn color_list(ui: &mut egui::Ui, items: &mut Vec<String>) {
    ui.vertical(|ui| {
        let mut remove = None;
        for (i, c) in items.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.push_id(i, |ui| color(ui, c));
                if ui.small_button("✕").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            items.remove(i);
        }
        if ui.small_button("+ add").clicked() {
            items.push("#4FC3F7".into());
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
        egui::ComboBox::from_id_salt("kind").width(100.0).selected_text(["none", "number", "expression"][kind]).show_ui(ui, |ui| {
            ui.selectable_value(&mut new_kind, 0, "none");
            ui.selectable_value(&mut new_kind, 1, "number");
            ui.selectable_value(&mut new_kind, 2, "expression");
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
            Some(Max::Expr(e)) => text(ui, e, "e.g. ram_total", 200.0),
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

const THEMES: &[(&str, &str)] = &[("auto", "follow Windows' mode for apps and the taskbar"), ("dark", "always the dark look"), ("light", "always the light look")];

impl Gui {
    fn general(&mut self, ui: &mut egui::Ui) {
        heading(ui, "General", "");
        section(ui, "daemon", |ui| {
            ui.label(RichText::new("state").monospace());
            ui.horizontal(|ui| {
                if self.daemon_running {
                    ui.label(RichText::new("running").color(OK));
                    if ui.button("Stop").on_hover_text("puts the stock taskbar back").clicked() {
                        self.message = match pms::daemon::stop() {
                            Ok(m) => (m, false),
                            Err(e) => (e, true),
                        };
                    }
                } else {
                    ui.label("stopped");
                    if ui.button("Start").clicked() {
                        let exe = std::env::current_exe().map(|p| p.with_file_name("pmsd.exe"));
                        self.message = match exe.and_then(|exe| std::process::Command::new(exe).arg(&self.path).spawn()) {
                            Ok(_) => ("Daemon starting".into(), false),
                            Err(e) => (format!("Couldn't start pmsd.exe: {e}"), true),
                        };
                    }
                }
            });
            ui.label("");
            ui.end_row();

            ui.label(RichText::new("last said").monospace());
            ui.add(egui::Label::new(RichText::new(&self.last_log).monospace().small().color(DIM)).truncate());
            ui.label("");
            ui.end_row();

            ui.label(RichText::new("start at sign-in").monospace()).on_hover_text("HKCU\\…\\Run: PimpMyShell");
            let mut on = self.autostart.starts_with("at logon");
            if ui.checkbox(&mut on, "").changed() {
                let args = if on { vec!["on".to_string(), self.path.display().to_string()] } else { vec!["off".to_string()] };
                self.message = match pms::daemon::autostart(&args) {
                    Ok(m) => (m, false),
                    Err(e) => (e, true),
                };
                self.poll_daemon();
            }
            ui.label("");
            ui.end_row();

            ui.label(RichText::new("config file").monospace());
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.path.display().to_string()).monospace().small());
                if ui.small_button("Show").clicked() {
                    let _ = std::process::Command::new("explorer.exe").arg("/select,").arg(&self.path).spawn();
                }
            });
            ui.label("");
            ui.end_row();
        });

        let c = &mut self.config;
        section(ui, "(top level)", |ui| {
            row(ui, "theme", "which look: auto follows Windows' light/dark mode; or always dark / light. It picks the backdrop's look and our text color.", &mut c.theme, "auto".into(), |ui, v| choice(ui, "theme", v, THEMES));
            row(ui, "meter_height", "how tall meters are: compact (3/4 of the taskbar), tall (halfway to full), full", &mut c.meter_height, "tall".into(), |ui, v| {
                choice(ui, "height", v, &[("compact", "3/4 of the taskbar"), ("tall", "halfway between compact and full"), ("full", "the taskbar's height")])
            });
            row(ui, "interval_ms", "how often meters update, unless a meter sets its own", &mut c.interval_ms, 1000, |ui, v| number(ui, v, 16..=60_000, " ms"));
            row(ui, "software_gpus", "count software adapters (Microsoft Basic Render Driver, Windows' CPU stand-in for a GPU) in GPU meters and engine counters; off: they're left out", &mut c.software_gpus, false, |ui, v| {
                ui.checkbox(v, "");
            });
            row(ui, "max_fps", "frame-rate cap for the long-running animations (the hot-mic alert), always. Unset: none, the display's full rate", &mut c.max_fps, None, |ui, v| {
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
            opt_row(ui, "battery_fps", "the cap while the PC runs on its battery (0: none); the lower of it and max_fps wins", &mut c.battery_fps, 30, |ui, v| number(ui, v, 0..=240, " fps"));
            row(ui, "value_font", "the meter values' font; fixed-width keeps digits still. Empty: Cascadia Mono, else Consolas.", &mut c.value_font, None, |ui, v| {
                let mut s = v.clone().unwrap_or_default();
                text(ui, &mut s, "Cascadia Mono", 200.0);
                *v = Some(s).filter(|s| !s.is_empty());
            });
        });

        self.software_update(ui);
    }

    fn software_update(&mut self, ui: &mut egui::Ui) {
        let c = &mut self.config;
        section(ui, "software update", |ui| {
            row(ui, "update_check", "whether the daemon checks GitHub for a newer version by itself, every day or two, and downloads it. It always asks before installing. Unset: the daemon asks you, once.", &mut c.update_check, None, |ui, v| {
                let mut on = *v == Some(true);
                if ui.checkbox(&mut on, if v.is_none() { "not asked yet" } else { "" }).changed() {
                    *v = Some(on);
                }
            });
            row(ui, "update_channel", "which version software update offers: stable, the one the repository's update/stable.txt names (a release that has proven itself); or latest, the newest release published, as soon as it's out", &mut c.update_channel, "stable".into(), |ui, v| {
                choice(ui, "update_channel", v, &[("stable", "the version update/stable.txt names"), ("latest", "the newest release published")])
            });
        });
        let state = self.update.lock().unwrap().clone();
        let installed = pms::update::installed();
        let latest = self.config.latest_channel();
        let busy = matches!(state, Update::Checking | Update::Downloading(..));
        let not_installed = "only the installed copy (from the installer) can install a version";
        ui.horizontal(|ui| {
            ui.label(format!("PimpMyShell {}", pms::version::CURRENT));
            if ui.add_enabled(!busy, egui::Button::new("Check for updates")).on_hover_text(if latest { "the newest release published" } else { "the stable version" }).clicked() {
                self.start_check(pms::update::Ask { install: installed, automatic: false, force: false, latest });
            }
            ui.separator();
            for (label, latest, help) in [
                ("Reinstall stable", false, "downloads the stable version, whatever this one is - the same, or older - and offers to install it"),
                ("Reinstall latest release", true, "downloads the newest release published, whatever this one is, and offers to install it"),
            ] {
                let button = ui.add_enabled(!busy && installed, egui::Button::new(label));
                if button.on_hover_text(help).on_disabled_hover_text(not_installed).clicked() {
                    self.start_check(pms::update::Ask { install: true, automatic: false, force: true, latest });
                }
            }
        });
        match state {
            Update::Idle if !installed => {
                ui.label(RichText::new("This copy isn't the installed one (a development build): it can check for a release, not install it.").color(DIM));
            }
            Update::Idle => {}
            Update::Checking => {
                ui.label("Checking…");
            }
            Update::Downloading(done, total) => {
                let bar = egui::ProgressBar::new(done as f32 / total.max(1) as f32).desired_width(320.0);
                ui.add(bar.text(format!("downloading: {} of {} KB", done / 1024, total / 1024)));
            }
            Update::Done(Status::UpToDate { stable }) => {
                let which = if latest { "latest release" } else { "stable version" };
                ui.label(RichText::new(format!("Up to date (the {which} is {stable}).")).color(OK));
            }
            Update::Done(Status::Available { version }) => {
                ui.horizontal(|ui| {
                    ui.label(format!("PimpMyShell {version} is out."));
                    if ui.button("Its page").clicked() {
                        let _ = std::process::Command::new("explorer.exe").arg(pms::update::release_page(&version)).spawn();
                    }
                });
            }
            Update::Done(Status::Stuck { version }) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("PimpMyShell {version} was installed, but didn't take: it isn't installed again by itself.")).color(ERROR));
                    if ui.button("Releases page").clicked() {
                        let _ = std::process::Command::new("explorer.exe").arg(pms::update::RELEASES_PAGE).spawn();
                    }
                });
            }
            Update::Done(Status::Ready { version, path, sha256 }) => {
                let how = if version == pms::version::CURRENT {
                    " (the version this is)".to_string()
                } else if pms::version::newer(pms::version::CURRENT, &version) {
                    format!(" (older than this one, {})", pms::version::CURRENT)
                } else {
                    String::new()
                };
                ui.label(RichText::new(format!("PimpMyShell {version}{how} is downloaded, and its signature checks out.")).color(OK));
                ui.horizontal(|ui| {
                    let install = ui.button(format!("Install {version} and restart"));
                    if install.on_hover_text("the installer runs silently: the taskbar goes back to stock for a moment, then PimpMyShell starts again").clicked() {
                        match pms::update::install(&path, &version, &sha256) {
                            Ok(()) => std::process::exit(0),
                            Err(e) => self.message = (e, true),
                        }
                    }
                    if ui.button("Not now").clicked() {
                        *self.update.lock().unwrap() = Update::Idle;
                    }
                });
            }
            Update::Failed(e) => {
                ui.label(RichText::new(format!("Couldn't check: {e}")).color(ERROR));
            }
        }
    }

    /// Starts a check (and, for the installed copy, the download) on a thread of its own.
    fn start_check(&mut self, ask: pms::update::Ask) {
        let shared = self.update.clone();
        *shared.lock().unwrap() = Update::Checking;
        std::thread::spawn(move || {
            let progress = |done, total| *shared.lock().unwrap() = Update::Downloading(done, total);
            let result = pms::update::check(ask, &progress);
            *shared.lock().unwrap() = match result {
                Ok(s) => Update::Done(s),
                Err(e) => Update::Failed(e),
            };
        });
    }

    fn meters(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Meters", "Left to right on the taskbar, as [[meter]] tables in the file. When space runs short, the last meters hide first.");
        let interval = self.config.interval_ms;
        let meters = &mut self.config.meters;
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
                                    if ui.small_button("✕").on_hover_text("remove").clicked() {
                                        action = Some(("remove", i));
                                    }
                                    if ui.small_button("↓").on_hover_text("later").clicked() {
                                        action = Some(("down", i));
                                    }
                                    if ui.small_button("↑").on_hover_text("sooner").clicked() {
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
                    meters.push(toml_meter("label = \"NEW\"\nexpr = \"0\"\nunit = \"%\"\nmax = 100\ncolor = \"#4FC3F7\""));
                    self.meter = meters.len() - 1;
                }
            });
            ui.add_space(10.0);
            ui.vertical(|ui| match meters.get_mut(self.meter) {
                Some(m) => meter_editor(ui, m, interval),
                None => {
                    ui.label("No meters.");
                }
            });
        });
    }

    fn counter_rows(&mut self, ui: &mut egui::Ui) {
        heading(
            ui,
            "Counters",
            "[counters]: Windows performance counters by English path, under names the meters' expressions use (as well as ram_total, vram_total, swap_total, swap_used and cpu_count). Browse paths with typeperf -q.",
        );
        let mut remove = None;
        egui::Grid::new("counters").num_columns(5).spacing([10.0, 6.0]).striped(true).show(ui, |ui| {
            for (h, hint) in [
                ("name", "what expressions call it"),
                ("path", "\\Object(instance)\\Counter; * for every instance"),
                ("agg", "how instances (*) combine; engine = Task Manager's GPU % (the busiest engine)"),
                ("default", "a stand-in for when the counter can't be read"),
                ("", ""),
            ] {
                ui.add(egui::Label::new(RichText::new(h).monospace().color(DIM)).sense(egui::Sense::hover())).on_hover_text(hint);
            }
            ui.end_row();
            for (i, r) in self.counters.iter_mut().enumerate() {
                text(ui, &mut r.name, "name", 110.0);
                text(ui, &mut r.path, "\\Object(*)\\Counter", 380.0);
                let agg = format!("{:?}", r.agg).to_lowercase();
                egui::ComboBox::from_id_salt(("agg", i)).width(80.0).selected_text(agg).show_ui(ui, |ui| {
                    for (a, name) in [(Agg::Sum, "sum"), (Agg::Max, "max"), (Agg::Min, "min"), (Agg::Avg, "avg"), (Agg::Engine, "engine")] {
                        ui.selectable_value(&mut r.agg, a, name);
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
            self.counters.push(CounterRow { name: String::new(), path: r"\Processor Information(_Total)\% Processor Utility".into(), agg: Agg::Sum, default: None });
        }
    }
}

fn toml_meter(s: &str) -> MeterSpec {
    toml_from(&format!("[[meter]]\n{s}")).ok().and_then(|c| c.meters.into_iter().next()).expect("valid meter template")
}

/// The `[[meter]]` table's rows, those that apply to its style. `interval` is the top-level interval_ms.
fn meter_editor(ui: &mut egui::Ui, m: &mut MeterSpec, interval: u64) {
    let style = m.style;
    let (label, expr, colors) = (m.label.clone(), m.expr.clone(), m.colors.clone());
    section(ui, "[[meter]]", |ui| {
        row(ui, "label", "the caption", &mut m.label, label, |ui, v| text(ui, v, "", 200.0));
        opt_row(ui, "group", "meters in different groups get a separator between them", &mut m.group, String::new(), |ui, v| text(ui, v, "e.g. memory", 200.0));
        opt_row(ui, "snug", "right against the meter before it: no separator, no spacing between", &mut m.snug, false, |ui, v| {
            ui.checkbox(v, "");
        });
        opt_row(ui, "caption", "show the label beside the meter on the taskbar (the tooltip has it either way)", &mut m.caption, true, |ui, v| {
            ui.checkbox(v, "");
        });
        row(ui, "style", "how the meter draws", &mut m.style, Style::History, |ui, v| {
            egui::ComboBox::from_id_salt("style").width(120.0).selected_text(format!("{v:?}").to_lowercase()).show_ui(ui, |ui| {
                for (s, name, help) in [
                    (Style::History, "history", "a graph of recent values"),
                    (Style::Bar, "bar", "a horizontal gauge"),
                    (Style::Pie, "pie", "slices of a whole, one per expr"),
                    (Style::Bars, "bars", "a vertical bar per expr"),
                    (Style::Cores, "cores", "a bar per CPU thread, paired by core"),
                    (Style::Engines, "engines", "a bar per GPU engine, grouped by GPU"),
                ] {
                    ui.selectable_value(v, s, name).on_hover_text(help);
                }
            });
        });
        if !matches!(style, Style::Cores | Style::Engines) {
            let many = matches!(style, Style::Pie | Style::Bars);
            let help = "+ - * / ( ) min max abs, over counter names and the builtins; a pie or bars takes one per slice or bar";
            row(ui, "expr", help, &mut m.expr, expr, |ui, v| {
                let mut list = v.as_ref().map(Exprs::list).unwrap_or_default();
                if many {
                    string_list(ui, "exprs", &mut list, "expression");
                } else {
                    list.resize(1, String::new());
                    text(ui, &mut list[0], "expression", 320.0);
                }
                *v = match list.len() {
                    0 => None,
                    1 => Some(Exprs::One(list.remove(0))),
                    _ => Some(Exprs::Many(list)),
                };
            });
            if many {
                row(ui, "names", "one per slice or bar, for the tooltip", &mut m.names, Vec::new(), |ui, v| string_list(ui, "names", v, "name"));
            }
        }
        if matches!(style, Style::Cores | Style::Pie | Style::Bars) {
            let help = if style == Style::Cores { "user time, then kernel time" } else { "one per slice or bar" };
            row(ui, "colors", help, &mut m.colors, colors, color_list);
        } else {
            opt_row(ui, "color", "the meter's color", &mut m.color, "#4FC3F7".into(), color);
        }
        if !matches!(style, Style::Cores | Style::Engines) {
            row(ui, "max", "full scale: a number or an expression; none scales to fit (a pie: its slices' sum)", &mut m.max, None, |ui, v| max_editor(ui, "max", v));
            row(ui, "unit", "%, bytes, bytes/s, bits/s (sizes and rates scale to K, M, G), or any suffix", &mut m.unit, String::new(), |ui, v| {
                egui::ComboBox::from_id_salt("unit").width(90.0).selected_text(if v.is_empty() { "(none)" } else { v.as_str() }).show_ui(ui, |ui| {
                    for u in ["", "%", "bytes", "bytes/s", "bits/s"] {
                        ui.selectable_value(v, u.to_string(), if u.is_empty() { "(none)" } else { u });
                    }
                });
                text(ui, v, "or any suffix", 80.0);
            });
            let decimals = if m.unit == "%" { 0 } else { 1 };
            opt_row(ui, "decimals", "digits after the point", &mut m.decimals, decimals, |ui, v| number(ui, v, 0..=4, ""));
        }
        opt_row(ui, "interval_ms", "this meter's own update rate (otherwise the top-level interval_ms)", &mut m.interval_ms, interval, |ui, v| number(ui, v, 16..=60_000, " ms"));
        if style != Style::History {
            opt_row(ui, "value", "show the value as text (the tooltip has it either way)", &mut m.value, true, |ui, v| {
                ui.checkbox(v, "");
            });
        }
        opt_row(ui, "tooltip", "a tooltip on hover", &mut m.tooltip, true, |ui, v| {
            ui.checkbox(v, "");
        });
        if matches!(style, Style::Cores | Style::Engines) {
            let help = "level: glow by how busy the bars are now. heat: each bar heats up as it works (full in ~8 s at 100%) and cools when it stops (~30 s) - the hotter, the wider and brighter its glow and the brighter its fill. off";
            opt_row(ui, "glow", help, &mut m.glow, "level".into(), |ui, v| {
                choice(ui, "glow", v, &[("level", "by how busy now"), ("heat", "by how hot they've got"), ("off", "no glow")])
            });
        }
        if matches!(style, Style::Cores | Style::Engines | Style::Bars) {
            row(ui, "bar_width", "DIPs per bar before squeezing; unset fits the count", &mut m.bar_width, None, |ui, v| {
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
            let help = "label each GPU by its maker (NVDA, AMD, INTC, QCOM, MSFT) in place of the caption, its bars lit in the maker's colors with a glow of them around";
            opt_row(ui, "vendors", help, &mut m.vendors, true, |ui, v| {
                ui.checkbox(v, "");
            });
            opt_row(ui, "vendor_label", "ticker: NVDA, AMD, INTC, QCOM, MSFT; short: NV, RAD (the rest as tickers)", &mut m.vendor_label, "ticker".into(), |ui, v| {
                choice(ui, "vendor_label", v, &[("ticker", "stock tickers"), ("short", "NV, RAD")])
            });
            opt_row(ui, "engine_hold_seconds", "an idle GPU's bars fall off after this long (the GPU that worked last stays); 0 = never", &mut m.engine_hold_seconds, 15, |ui, v| number(ui, v, 0..=3600, " s"));
        }
        if style == Style::Pie {
            opt_row(ui, "per_gpu", "one of this pie per graphics adapter with memory of its own (dedicated memory above 0), each reading that adapter's memory against its size, in its maker's color: expr, max and color are filled in", &mut m.per_gpu, false, |ui, v| {
                ui.checkbox(v, "");
            });
            opt_row(ui, "label_inside", "the label inside the pie: never, auto (when space runs short), always", &mut m.label_inside, "never".into(), |ui, v| {
                choice(ui, "inside", v, &[("never", "beside the pie"), ("auto", "inside when space runs short"), ("always", "inside the pie")])
            });
            let help = "how full the first slice gets before the resource is tight: past it the rim's glow swells (it fades in between a third and two thirds full)";
            opt_row(ui, "pressure", help, &mut m.pressure, 0.8, |ui, v| {
                ui.add(egui::Slider::new(v, 0.5..=0.99));
            });
        }
        if style == Style::History {
            opt_row(ui, "time_axis", "log: the newest samples get the most room, older ones squeeze up; linear: one sample per step", &mut m.time_axis, "log".into(), |ui, v| {
                choice(ui, "time", v, &[("log", "newest samples widest, the past compressed"), ("linear", "one sample per step")])
            });
            opt_row(ui, "history_seconds", "how far back the graph reaches", &mut m.history_seconds, 300, |ui, v| number(ui, v, 10..=86_400, " s"));
        }
        row(ui, "show_above", "hidden until its value reaches this (a number or an expression)", &mut m.show_above, None, |ui, v| max_editor(ui, "show_above", v));
    });
}

/// tint, grain and gaps, for the dark look (`[backdrop]`) or the light one (`[backdrop.light]`).
fn look_rows(ui: &mut egui::Ui, tint: &mut Option<String>, grain: &mut Option<f32>, gaps: &mut Option<String>, default_tint: &str) {
    opt_row(ui, "tint", "laid over the blurred desktop; its alpha is the opacity", tint, default_tint.into(), color);
    opt_row(ui, "grain", "noise over the tint; 0 = none", grain, config::DEFAULT_GRAIN, |ui, v| {
        knob(ui, v, 0.2);
    });
    row(ui, "gaps", "paint the free space between buttons", gaps, None, |ui, v| {
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
    heading(ui, "Backdrop", "PimpMyShell's own taskbar fill in place of the stock acrylic: a look for dark mode and one for light, picked by theme.");
    switch(ui, "Use PimpMyShell's backdrop", &mut c.backdrop, || Backdrop { tint: Some(config::DARK_TINT.into()), grain: Some(config::DEFAULT_GRAIN), gaps: None, light: None });
    section(ui, "(top level) ", |ui| {
        row(ui, "theme", "which look: auto follows Windows' light/dark mode; or always dark / light", &mut c.theme, "auto".into(), |ui, v| choice(ui, "theme", v, THEMES));
    });
    let Some(b) = &mut c.backdrop else { return };
    section(ui, "[backdrop]  · the dark look", |ui| look_rows(ui, &mut b.tint, &mut b.grain, &mut b.gaps, config::DARK_TINT));
    let light = b.light.get_or_insert_with(Look::default);
    section(ui, "[backdrop.light]", |ui| look_rows(ui, &mut light.tint, &mut light.grain, &mut light.gaps, config::LIGHT_TINT));
    if *light == Look::default() {
        b.light = None;
    }
}

fn hotmic(ui: &mut egui::Ui, h: &mut Option<Hotmic>) {
    heading(ui, "Hot mic", "While an app has the microphone open: the whole taskbar warns, then settles into a label by the tray naming the apps. An app that lets go stays listed for 10 seconds, (stopped N sec. ago); a new app starting to listen brings back the whole-taskbar warning.");
    switch(ui, "Warn while the microphone is in use", h, Hotmic::default);
    let Some(h) = h else { return };
    let d = Hotmic::default();
    section(ui, "[hotmic]", |ui| {
        row(ui, "color", "the warning's color", &mut h.color, d.color.clone(), color);
        row(ui, "period_ms", "one breath of the wash; 0 = steady", &mut h.period_ms, d.period_ms, |ui, v| number(ui, v, 0..=10_000, " ms"));
        row(ui, "low", "the breath's lowest opacity", &mut h.low, d.low, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "high", "the breath's highest opacity", &mut h.high, d.high, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "broad_ms", "how long the whole taskbar warns before settling around the label (it settles at once when an app closes)", &mut h.broad_ms, d.broad_ms, |ui, v| number(ui, v, 0..=60_000, " ms"));
        row(ui, "ignore", "apps never to warn about: part of their name or path", &mut h.ignore, Vec::new(), |ui, v| string_list(ui, "ignore", v, "e.g. obs64.exe"));
    });
}

fn toasts(ui: &mut egui::Ui, t: &mut Option<Toasts>) {
    heading(ui, "Toasts", "Notifications shown inside the taskbar instead of as popups; their buttons work, and they stay in the notification center.");
    switch(ui, "Notifications in the taskbar", t, Toasts::default);
    let Some(t) = t else { return };
    let d = Toasts::default();
    section(ui, "[toasts]", |ui| {
        row(ui, "duration_ms", "how long each toast stays (less while more are waiting)", &mut t.duration_ms, d.duration_ms, |ui, v| number(ui, v, 1000..=60_000, " ms"));
        row(ui, "ignore", "apps that keep their stock popups: name or AUMID", &mut t.ignore, Vec::new(), |ui, v| string_list(ui, "toast-ignore", v, "e.g. Teams"));
    });
}

fn start_glow(ui: &mut egui::Ui, g: &mut Option<StartGlow>) {
    ui.add_space(10.0);
    switch(ui, "Glow behind the Start button", g, StartGlow::default);
    let Some(g) = g else { return };
    let d = StartGlow::default();
    section(ui, "[start_glow]", |ui| {
        row(ui, "color", "the glow's color", &mut g.color, d.color.clone(), color);
        row(ui, "hover", "how bright while the pointer is over the button (none at rest)", &mut g.hover, d.hover, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "press", "how bright while it's pressed", &mut g.press, d.press, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "hover_ms", "how long it takes to fade in and out with the pointer", &mut g.hover_ms, d.hover_ms, |ui, v| number(ui, v, 0..=2000, " ms"));
        row(ui, "open", "how bright while the Start menu is open (the Windows key opens it too)", &mut g.open, d.open, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "press_ms", "how long into and out of a press (quicker)", &mut g.press_ms, d.press_ms, |ui, v| number(ui, v, 0..=2000, " ms"));
        row(ui, "hover_radius", "how far the glow reaches with the pointer over the button", &mut g.hover_radius, d.hover_radius, |ui, v| number(ui, v, 0.0..=600.0, " DIP"));
        row(ui, "press_radius", "how far it reaches while pressed", &mut g.press_radius, d.press_radius, |ui, v| number(ui, v, 0.0..=600.0, " DIP"));
    });
}

fn app_glow(ui: &mut egui::Ui, g: &mut Option<AppGlow>) {
    ui.add_space(10.0);
    switch(ui, "Glows behind the app buttons, in their icons' colors", g, AppGlow::default);
    let Some(g) = g else { return };
    let d = AppGlow::default();
    section(ui, "[app_glow]", |ui| {
        row(ui, "hover", "how bright while the pointer is over a button (none at rest)", &mut g.hover, d.hover, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "press", "how bright while it's pressed", &mut g.press, d.press, |ui, v| {
            knob(ui, v, 1.0);
        });
        row(ui, "hover_ms", "how long it takes to fade in and out with the pointer", &mut g.hover_ms, d.hover_ms, |ui, v| number(ui, v, 0..=2000, " ms"));
        row(ui, "press_ms", "how long into and out of a press (quicker)", &mut g.press_ms, d.press_ms, |ui, v| number(ui, v, 0..=2000, " ms"));
        row(ui, "hover_radius", "how far the glow reaches with the pointer over a button", &mut g.hover_radius, d.hover_radius, |ui, v| number(ui, v, 0.0..=600.0, " DIP"));
        row(ui, "press_radius", "how far it reaches while pressed", &mut g.press_radius, d.press_radius, |ui, v| number(ui, v, 0.0..=600.0, " DIP"));
    });
}

/// A 0..ull knob (pms-ui).
fn knob(ui: &mut egui::Ui, value: &mut f32, full: f32) {
    pms_ui::knob(ui, value, 0.0, full);
}
