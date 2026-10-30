//! `pms-art`: PimpMyShell's pictures, and a window to customize them, in the manner of Nagare's customizer.
//!
//! - The logo: a crab (Scout's "Red Crab", public domain, from Openclipart: art/openclipart-192996) with a dollar
//!   bill in front of it. Into assets/icon: pms.ico (16 to 256 pixels) and a PNG of each size; the build puts the
//!   .ico into every exe (pms-res), the tray and the notifications show it.
//! - The installer's box: a glassy cube, words on one face and the logo on the other. And the installer's pictures:
//!   the box on dollar signs tiled over the dark, and in the pages' corner. Into installer/art.
//!
//! The window shows them all as they'll be, with knobs (they go to eleven) for the crab, the bill, the box and the
//! wallpaper of dollars; Save keeps the design in art/design.json and renders every file.
//!
//!   pms-art                      the window
//!   pms-art --render [workspace] render every file from art/design.json, no window (cargo xtask art)

#![windows_subsystem = "windows"]

mod art;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use art::Design;
use eframe::egui::{self, Color32, RichText, TextureHandle, TextureOptions};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("in the workspace").to_path_buf()
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--render") {
        let root = args.get(1).map(PathBuf::from).unwrap_or_else(workspace);
        let path = root.join("art/design.json");
        let design = Design::load(&path);
        // The design lives with the source: a first render writes it out.
        if !path.exists() && let Err(e) = design.save(&path) {
            eprintln!("pms-art: {e}");
        }
        match art::write_all(&root, &design) {
            Ok(said) => println!("rendered {said}"),
            Err(e) => {
                eprintln!("pms-art: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let root = workspace();
    let options = eframe::NativeOptions {
        viewport: {
            let v = egui::ViewportBuilder::default().with_title("PimpMyShell art").with_inner_size([1100.0, 760.0]).with_min_inner_size([900.0, 600.0]);
            match std::fs::read(root.join("assets/icon/pms-64.png")).ok().and_then(|b| eframe::icon_data::from_png_bytes(&b).ok()) {
                Some(icon) => v.with_icon(icon),
                None => v,
            }
        },
        ..Default::default()
    };
    eframe::run_native("PimpMyShell art", options, Box::new(move |cc| Ok(Box::new(Customizer::open(root, &cc.egui_ctx)))))
}

/// The previews, rendered from one design.
struct Previews {
    logo: TextureHandle,
    small: Vec<(u32, TextureHandle)>,
    boxed: TextureHandle,
    wizard: TextureHandle,
}

struct Customizer {
    root: PathBuf,
    crab: String,
    design: Design,
    /// As saved in art/design.json.
    saved: Design,
    /// The design the previews show, and when the design last changed (they catch up a moment after).
    shown: Option<Design>,
    changed: Instant,
    previews: Option<Previews>,
    message: (String, bool),
}

impl Customizer {
    fn open(root: PathBuf, ctx: &egui::Context) -> Customizer {
        let design = Design::load(&root.join("art/design.json"));
        let (crab, message) = match art::crab(&root) {
            Ok(c) => (c, ("Drag a knob, pick a color: the previews follow. Save keeps the design and renders every file.".to_string(), false)),
            Err(e) => (String::new(), (e, true)),
        };
        style(ctx);
        Customizer { root, crab, saved: design.clone(), design, shown: None, changed: Instant::now(), previews: None, message }
    }

    /// Render the previews from the design: the logo, big and at the small sizes as they'll really look, the box,
    /// and the wizard's picture.
    fn render(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let d = &self.design;
        let tex = |name: &str, svg: &str, w: u32, h: u32, crisp: bool| -> Result<TextureHandle, String> {
            let rgba = art::rgba(svg, w, h)?;
            let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            Ok(ctx.load_texture(name, image, if crisp { TextureOptions::NEAREST } else { TextureOptions::LINEAR }))
        };
        let logo = art::logo_svg(d, &self.crab);
        let boxed = art::box_svg(d, &art::b64(&art::png(&logo, 256, 256)?));
        let wizard = art::wizard_svg(d, &art::b64(&art::png(&boxed, 512, 512)?));
        self.previews = Some(Previews {
            logo: tex("logo", &logo, 256, 256, false)?,
            small: [16, 24, 32, 48].iter().map(|&s| tex(&format!("logo{s}"), &logo, s, s, true).map(|t| (s, t))).collect::<Result<_, _>>()?,
            boxed: tex("box", &boxed, 256, 256, false)?,
            wizard: tex("wizard", &wizard, 328, 628, false)?,
        });
        self.shown = Some(self.design.clone());
        Ok(())
    }

    fn save(&mut self) {
        let result = self.design.save(&self.root.join("art/design.json")).and_then(|()| art::write_all(&self.root, &self.design));
        self.message = match result {
            Ok(said) => {
                self.saved = self.design.clone();
                (format!("Saved art/design.json and rendered {said}. The next build puts the icon in the exes."), false)
            }
            Err(e) => (format!("Couldn't save: {e}"), true),
        };
    }
}

/// Flat and dark, like the settings.
fn style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        let v = &mut s.visuals;
        v.panel_fill = Color32::from_rgb(0x1B, 0x1C, 0x20);
        let w = &mut v.widgets;
        for w in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open, &mut w.noninteractive] {
            w.corner_radius = egui::CornerRadius::ZERO;
        }
        w.inactive.weak_bg_fill = Color32::from_rgb(0x2A, 0x2B, 0x31);
        w.hovered.weak_bg_fill = Color32::from_rgb(0x37, 0x38, 0x3F);
    });
}

/// A `#RRGGBB` color: a swatch with its picker, and the hex.
fn color(ui: &mut egui::Ui, hex: &mut String) {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0xFFFFFF);
    let mut c = Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8);
    if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).changed() {
        *hex = format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b());
    }
    ui.add(egui::TextEdit::singleline(hex).desired_width(76.0).font(egui::TextStyle::Monospace));
}

/// A section of rows: a name, its control.
fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).size(15.0).strong().color(Color32::WHITE));
    egui::Grid::new(title).num_columns(2).spacing([12.0, 8.0]).min_col_width(110.0).show(ui, body);
}

fn knob_row(ui: &mut egui::Ui, name: &str, help: &str, value: &mut f32, min: f32, max: f32) {
    ui.label(name).on_hover_text(help);
    ui.horizontal(|ui| pms_ui::knob(ui, value, min, max));
    ui.end_row();
}

fn color_row(ui: &mut egui::Ui, name: &str, value: &mut String) {
    ui.label(name);
    ui.horizontal(|ui| color(ui, value));
    ui.end_row();
}

impl eframe::App for Customizer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Previews catch up 80 ms after the last change, so turning a knob stays smooth.
        let stale = self.shown.as_ref() != Some(&self.design);
        if stale
            && self.changed.elapsed() >= Duration::from_millis(80)
            && !self.crab.is_empty()
            && let Err(e) = self.render(ctx)
        {
            self.message = (e, true);
            self.shown = Some(self.design.clone());
        }
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            let (text, error) = &self.message;
            ui.label(RichText::new(text).color(if *error { Color32::from_rgb(0xFF, 0x8A, 0x80) } else { Color32::from_gray(150) }));
        });
        let before = self.design.clone();
        egui::SidePanel::left("knobs").resizable(false).exact_width(330.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("PimpMyShell art").size(20.0).strong().color(Color32::WHITE));
                let d = &mut self.design;
                section(ui, "Crab", |ui| {
                    knob_row(ui, "size", "how wide the crab is", &mut d.crab_size, 120.0, 256.0);
                    knob_row(ui, "height", "how far down the crab sits", &mut d.crab_y, -40.0, 90.0);
                });
                section(ui, "Bill", |ui| {
                    knob_row(ui, "tilt", "how askew the bill lies", &mut d.bill_tilt, -35.0, 35.0);
                    knob_row(ui, "size", "how wide the bill is", &mut d.bill_size, 120.0, 250.0);
                    knob_row(ui, "height", "where the bill's middle is: up over the crab, or down below it", &mut d.bill_y, 120.0, 230.0);
                    color_row(ui, "color", &mut d.bill_color);
                });
                section(ui, "Box", |ui| {
                    color_row(ui, "glass", &mut d.glass);
                    ui.label("words");
                    ui.add(egui::TextEdit::singleline(&mut d.face_text).desired_width(150.0));
                    ui.end_row();
                    color_row(ui, "their color", &mut d.face_color);
                });
                section(ui, "Installer picture", |ui| {
                    color_row(ui, "background", &mut d.wizard_bg);
                    color_row(ui, "dollar signs", &mut d.dollar_color);
                    knob_row(ui, "strength", "how strongly the dollar signs show", &mut d.dollar_strength, 0.0, 0.6);
                    knob_row(ui, "size", "how big each dollar sign's tile is", &mut d.dollar_size, 10.0, 60.0);
                    knob_row(ui, "tilt", "how the rows of dollar signs slant", &mut d.dollar_tilt, -45.0, 45.0);
                });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let unsaved = self.design != self.saved;
                    if ui.add_enabled(unsaved, egui::Button::new(RichText::new("Save and render").strong())).clicked() {
                        self.save();
                    }
                    if ui.add_enabled(unsaved, egui::Button::new("Revert")).on_hover_text("back to what's saved").clicked() {
                        self.design = self.saved.clone();
                    }
                    if ui.button("Defaults").clicked() {
                        self.design = Design::default();
                    }
                });
                if self.design != self.saved {
                    ui.label(RichText::new("unsaved changes").small().color(Color32::from_rgb(0xFF, 0xC1, 0x5A)));
                }
            });
        });
        if self.design != before {
            self.changed = Instant::now();
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            let Some(p) = &self.previews else {
                ui.label("Rendering…");
                return;
            };
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Logo").strong());
                    ui.image((p.logo.id(), egui::vec2(256.0, 256.0)));
                    ui.add_space(6.0);
                    ui.label(RichText::new("as the taskbar and Explorer show it").small().color(Color32::from_gray(150)));
                    ui.horizontal(|ui| {
                        for (s, t) in &p.small {
                            ui.vertical(|ui| {
                                ui.image((t.id(), egui::vec2(*s as f32, *s as f32)));
                                ui.label(RichText::new(format!("{s}")).small().color(Color32::from_gray(130)));
                            });
                        }
                    });
                    ui.add_space(12.0);
                    ui.label(RichText::new("Installer box").strong());
                    ui.image((p.boxed.id(), egui::vec2(200.0, 200.0)));
                });
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("Installer picture").strong());
                    ui.image((p.wizard.id(), egui::vec2(328.0, 628.0)));
                });
            });
        });
        let waiting = self.shown.as_ref() != Some(&self.design);
        if waiting {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
    }
}
