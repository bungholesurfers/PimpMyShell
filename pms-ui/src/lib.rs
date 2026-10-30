//! Widgets PimpMyShell's windows share: the settings (pms-gui) and the art customizer (pms-art).

use eframe::egui::{self, Color32, RichText};

/// A knob, as on an amp: these go to eleven. It turns through 270 degrees, 0 to 11 standing for `min` to `max`,
/// dragged up or down (or with the wheel); a double click puts it back to 0. Its number shows beside it.
pub fn knob(ui: &mut egui::Ui, value: &mut f32, min: f32, max: f32) -> egui::Response {
    let size = 30.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click_and_drag());
    let span = (max - min).max(f32::EPSILON);
    let before = ((*value - min) / span).clamp(0.0, 1.0);
    let mut t = before;
    if response.dragged() {
        t = (t - response.drag_delta().y / 160.0 + response.drag_delta().x / 320.0).clamp(0.0, 1.0);
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            t = (t + scroll.signum() / 44.0).clamp(0.0, 1.0);
        }
    }
    if response.double_clicked() {
        t = 0.0;
    }
    if t != before {
        *value = min + t * span;
    }
    let p = ui.painter();
    let c = rect.center();
    let r = size / 2.0 - 2.0;
    let angle = |t: f32| std::f32::consts::PI * (0.75 + 1.5 * t);
    let at = |t: f32, k: f32| c + egui::vec2(angle(t).cos(), angle(t).sin()) * r * k;
    // The track, and the arc it's turned through so far.
    let arc = |from: f32, to: f32, color: Color32, width: f32| {
        let n = 32;
        let points: Vec<egui::Pos2> = (0..=n).map(|i| at(from + (to - from) * i as f32 / n as f32, 1.0)).collect();
        p.add(egui::Shape::line(points, egui::Stroke::new(width, color)));
    };
    arc(0.0, 1.0, Color32::from_rgb(0x34, 0x35, 0x3C), 3.0);
    if t > 0.0 {
        arc(0.0, t, Color32::from_rgb(0x3A, 0x8E, 0xD8), 3.0);
    }
    let face = if response.hovered() || response.dragged() { Color32::from_rgb(0x3A, 0x3B, 0x43) } else { Color32::from_rgb(0x2C, 0x2D, 0x33) };
    p.circle_filled(c, r - 4.0, face);
    p.line_segment([at(t, 0.15), at(t, 0.62)], egui::Stroke::new(2.0_f32, Color32::WHITE));
    ui.label(RichText::new(format!("{:.1}", t * 11.0)).monospace());
    response.on_hover_text("drag up or down, or scroll; double click for 0 - and yes, it goes to eleven")
}