//! The pictures: the design (what the customizer's knobs set, kept in art/design.json), the SVG each picture is
//! drawn as from it, and their rendering (resvg) into the files the build and the installer use.

use std::path::Path;

use base64::Engine as _;
use resvg::{tiny_skia, usvg};
use serde::{Deserialize, Serialize};

use crate::flag;

pub const ICON_SIZES: [u32; 10] = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];
/// The installer's pictures, at the size Inno Setup draws each at 100, 125, 150, 175, 200, 225 and 250% (its help has
/// them; they aren't quite in proportion, as they follow the font). It picks the nearest and stretches that to fit, so any
/// other size is resampled on the way to the screen.
pub const WIZARD_SIZES: [(u32, u32); 7] = [(202, 386), (269, 515), (336, 643), (403, 772), (430, 824), (498, 953), (534, 1022)];
pub const BACKDROP_SIZES: [(u32, u32); 7] = [(596, 432), (796, 576), (994, 720), (1193, 864), (1272, 922), (1471, 1066), (1630, 1148)];
pub const CORNER_SIZES: [u32; 7] = [58, 77, 97, 116, 124, 143, 159];

/// Which bill a picture has: the original, or the flag with its digital trail (flag.rs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bill {
    #[default]
    Classic,
    Flag,
}

/// Everything the pictures are made of that the customizer can change. Coordinates are in the logo's 256 units.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Design {
    /// The crab: its width (of 256) and how far down its top is.
    pub crab_size: f32,
    pub crab_y: f32,
    /// The bill in each picture: the main icon, the box's logo, the installer's background and the settings' (which the
    /// settings read from this design when they're built).
    pub icon_bill: Bill,
    pub box_bill: Bill,
    pub backdrop_bill: Bill,
    pub settings_bill: Bill,
    /// Which lies on top of the other: the crab, or the bill.
    pub crab_on_top: bool,
    /// The bill: its tilt (degrees), its width (of 256), the height of its middle, and its color.
    pub bill_tilt: f32,
    pub bill_size: f32,
    pub bill_y: f32,
    pub bill_color: String,
    /// The box: the tint of its glass, the words on its left face (a line each; `\n`, typed, is a new line too) and
    /// their color.
    pub glass: String,
    pub face_text: String,
    pub face_color: String,
    /// The reflection across the glass: its strength (0..1) and how far it reaches either side of its middle (of a face).
    pub gloss: f32,
    pub gloss_width: f32,
    /// The strip of tape along the box's top: its strength (0..1), its width (of the top) and its color.
    pub tape: f32,
    pub tape_width: f32,
    pub tape_color: String,
    /// The installer's background: its color, and the bills tiled over it - their color, strength (0..1), size (22 is
    /// as in the settings) and tilt.
    pub wizard_bg: String,
    pub dollar_color: String,
    pub dollar_strength: f32,
    pub dollar_size: f32,
    pub dollar_tilt: f32,
}

impl Default for Design {
    fn default() -> Self {
        Design {
            crab_size: 244.0,
            crab_y: 4.0,
            icon_bill: Bill::Classic,
            box_bill: Bill::Classic,
            backdrop_bill: Bill::Flag,
            settings_bill: Bill::Flag,
            crab_on_top: false,
            bill_tilt: -9.0,
            bill_size: 200.0,
            bill_y: 185.0,
            bill_color: "#85BB65".into(),
            glass: "#A9D2F5".into(),
            face_text: "\u{2605}PMS".into(),
            face_color: "#FFFFFF".into(),
            gloss: 0.3,
            gloss_width: 0.1,
            tape: 0.35,
            tape_width: 0.22,
            tape_color: "#BFE6FF".into(),
            wizard_bg: "#0F1411".into(),
            dollar_color: "#85C76B".into(),
            dollar_strength: 0.36,
            dollar_size: 22.0,
            dollar_tilt: 0.0,
        }
    }
}

impl Design {
    pub fn load(path: &Path) -> Design {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json + "\n").map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// `#RRGGBB` scaled toward black (`k` < 1) or white (`k` > 1), for a color's lighter and darker shades.
fn shade(hex: &str, k: f32) -> String {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x85BB65);
    let c = |v: u32| {
        let v = v as f32;
        let out = if k <= 1.0 { v * k } else { v + (255.0 - v) * (k - 1.0) };
        out.round().clamp(0.0, 255.0) as u8
    };
    format!("#{:02X}{:02X}{:02X}", c((n >> 16) & 0xFF), c((n >> 8) & 0xFF), c(n & 0xFF))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The logo, 256 units square: the crab, its claws raised; in front of it a dollar bill, askew, with its shadow.
pub fn logo_svg(d: &Design, crab: &str, style: Bill) -> Result<String, String> {
    // The crab's own <svg> goes in as a nested one, placed and sized here (its viewBox keeps its proportions).
    let crab = crab.find("<svg").map_or(crab, |at| &crab[at..]);
    let height = d.crab_size * 511.0 / 755.0;
    let crab = crab.replacen("<svg", &format!(r#"<svg x="{}" y="{}" width="{}" height="{height}""#, (256.0 - d.crab_size) / 2.0, d.crab_y, d.crab_size), 1);
    // The bill: 200 by 90 at full size, around its middle.
    let k = d.bill_size / 200.0;
    let (light, base, dark, line, ink) = (shade(&d.bill_color, 1.2), shade(&d.bill_color, 0.85), shade(&d.bill_color, 0.6), shade(&d.bill_color, 0.4), shade(&d.bill_color, 1.85));
    // The bill itself, flat, at 200 by 90 around (128, 185).
    let paper_bill = format!(
        r##"    <rect x="30" y="142" width="200" height="90" rx="7" fill="#000" opacity="0.35" filter="url(#soft)" transform="translate(3 5)"/>
    <rect x="28" y="140" width="200" height="90" rx="7" fill="url(#paper)" stroke="{line}" stroke-width="3"/>
    <rect x="37" y="149" width="182" height="72" rx="4" fill="none" stroke="{ink}" stroke-width="2.5" opacity="0.9"/>
    <ellipse cx="128" cy="185" rx="27" ry="28" fill="{dark}" stroke="{ink}" stroke-width="2.5"/>
    <path d="M140 170 C 136 164, 120 163, 117 171 C 113 181, 141 182, 139 194 C 137 204, 118 205, 114 197 M128 160 L128 210"
          fill="none" stroke="{ink}" stroke-width="7" stroke-linecap="round" stroke-linejoin="round"/>
    <g fill="{ink}" font-family="Segoe UI, Arial" font-weight="700" font-size="17">
      <text x="44" y="168">1</text><text x="203" y="168">1</text><text x="44" y="215">1</text><text x="203" y="215">1</text>
    </g>"##
    );
    let bill = format!(r#"<g transform="translate(128 {y}) rotate({tilt}) scale({k}) translate(-128 -185)">{paper_bill}</g>"#, y = d.bill_y, tilt = d.bill_tilt);
    let bill = match style {
        Bill::Classic => bill,
        Bill::Flag => flag_in_logo(d, &paper_bill)?,
    };
    let (under, over) = if d.crab_on_top { (&bill, &crab) } else { (&crab, &bill) };
    Ok(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 256 256" width="256" height="256">
  <defs>
    <filter id="soft" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="4"/></filter>
    <linearGradient id="paper" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{light}"/><stop offset="1" stop-color="{base}"/></linearGradient>
  </defs>
  {under}
  {over}
</svg>"##
    ))
}

/// The flag, bent at rest: what `flat` (an SVG of the flat picture, in its units) looks like, with `flag::PAD` above and
/// below it, as a PNG of `k` pixels a unit.
fn bent(flat: &str, k: f32, phase: f32) -> Result<Vec<u8>, String> {
    let (w, h) = ((flag::FLAT_W * k).round() as u32, (flag::FLAT_H * k).round() as u32);
    let source = pixmap(flat, w, h)?;
    let out_h = h + (2.0 * flag::PAD * k).round() as u32;
    let mut out = tiny_skia::Pixmap::new(w, out_h).ok_or("no pixmap")?;
    let pixels = source.pixels();
    let row = |x: u32, y: i32| -> [f32; 4] {
        if y < 0 || y >= h as i32 {
            return [0.0; 4];
        }
        let p = pixels[y as usize * w as usize + x as usize];
        [p.red() as f32, p.green() as f32, p.blue() as f32, p.alpha() as f32]
    };
    for x in 0..w {
        let b = flag::bend((x as f32 + 0.5) / w as f32, phase);
        for y in 0..out_h {
            // Where this pixel comes from in the flat picture: its height around the middle, undone.
            let from = ((y as f32 + 0.5 - out_h as f32 / 2.0) / k - b.lift) / b.stretch + flag::FLAT_H / 2.0;
            let at = from * k - 0.5;
            let (top, part) = (at.floor(), at - at.floor());
            let (a, c) = (row(x, top as i32), row(x, top as i32 + 1));
            let mix = |i: usize| ((a[i] + (c[i] - a[i]) * part) * b.light).round().clamp(0.0, 255.0) as u8;
            if let Some(p) = tiny_skia::PremultipliedColorU8::from_rgba(mix(0), mix(1), mix(2), mix(3)) {
                out.pixels_mut()[(y * w + x) as usize] = p;
            }
        }
    }
    out.encode_png().map_err(|e| e.to_string())
}

/// The trail, as SVG in the flat picture's units.
fn trail(color: &str, k: f32) -> String {
    flag::blocks().iter().map(|(x, y, w, h, opacity)| format!(r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{color}" opacity="{opacity}"/>"#, x * k, y * k, w * k, h * k)).collect()
}

/// The flat flag for the tiles, as dollars.rs draws it.
fn flat_flag(color: &str) -> String {
    let (x, y, w, h) = (flag::BILL_X, flag::BILL_Y, flag::BILL_W, flag::BILL_H);
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let corners: String = [(-25.0, -8.5), (25.0, -8.5), (-25.0, 8.5), (25.0, 8.5)].iter().map(|(dx, dy): &(f32, f32)| format!(r#"<circle cx="{}" cy="{}" r="1.4" fill="{color}"/>"#, cx + dx, cy + dy)).collect();
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {fw} {fh}" width="{fw}" height="{fh}">{trail}<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="5" fill="{color}" opacity="{paper}"/><g fill="none" stroke="{color}"><rect x="{x1}" y="{y1}" width="{w1}" height="{h1}" rx="5" stroke-width="1.8"/><rect x="{x2}" y="{y2}" width="{w2}" height="{h2}" rx="2.5" stroke-width="0.8"/><circle cx="{cx}" cy="{cy}" r="10.5" stroke-width="1.2"/></g>{corners}<text x="{cx}" y="{sign}" font-family="Segoe UI Semibold, Segoe UI, Arial" font-weight="600" font-size="17" text-anchor="middle" fill="{color}">$</text></svg>"#,
        fw = flag::FLAT_W,
        fh = flag::FLAT_H,
        trail = trail(color, 1.0),
        paper = flag::PAPER,
        x1 = x + 0.9,
        y1 = y + 0.9,
        w1 = w - 1.8,
        h1 = h - 1.8,
        x2 = x + 4.0,
        y2 = y + 4.0,
        w2 = w - 8.0,
        h2 = h - 8.0,
        sign = cy + 5.5,
    )
}

/// The logo's own bill as a flag: `bill` is its markup, flat, at 200 by 90 around (128, 185). The trail takes room, so the
/// whole flag is fitted to the width the bill had, give or take.
fn flag_in_logo(d: &Design, bill: &str) -> Result<String, String> {
    let k = 200.0 / flag::BILL_W;
    let (w, h) = (flag::FLAT_W * k, flag::FLAT_H * k);
    let color = shade(&d.bill_color, 0.85);
    let flat = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}">{trail}<g transform="translate({x} {y})">{bill}</g></svg>"##,
        trail = trail(&color, k),
        x = (flag::BILL_X + flag::BILL_W / 2.0) * k - 128.0,
        y = h / 2.0 - 185.0,
    );
    let png = bent(&flat, k * 2.0, flag::REST)?;
    let tall = h + 2.0 * flag::PAD * k;
    let fit = (d.bill_size * 1.3).min(250.0) / w;
    Ok(format!(
        r#"<image transform="translate(128 {y}) rotate({tilt}) scale({fit}) translate({left} {top})" width="{w}" height="{tall}" preserveAspectRatio="none" xlink:href="data:image/png;base64,{png}"/>"#,
        y = d.bill_y,
        tilt = d.bill_tilt,
        left = -w / 2.0,
        top = -tall / 2.0,
        png = b64(&png),
    ))
}

/// The box's faces as transforms from a unit square, so that what's drawn on one lies on it: across (u), then down (v).
const LEFT_FACE: &str = "matrix(98 48 0 122 30 66)";
const RIGHT_FACE: &str = "matrix(98 -48 0 122 128 114)";
const TOP_FACE: &str = "matrix(98 -48 98 48 30 66)";

/// A band of light across a face, as a gradient on its unit square: strongest where `a.0 * u + a.1 * v + a.2` is
/// `middle`, gone `width` either side.
fn band(id: &str, a: (f32, f32, f32), middle: f32, width: f32, strength: f32) -> String {
    let width = width.max(0.01);
    let squared = a.0 * a.0 + a.1 * a.1;
    let at = |t: f32| ((t - a.2) * a.0 / squared, (t - a.2) * a.1 / squared);
    let ((x1, y1), (x2, y2)) = (at(middle - width), at(middle + width));
    // The curve of a smoothstep, either side of the middle.
    let stops: String = [(0.0, 0.0), (0.125, 0.156), (0.25, 0.5), (0.375, 0.844), (0.5, 1.0), (0.625, 0.844), (0.75, 0.5), (0.875, 0.156), (1.0, 0.0)]
        .iter()
        .map(|(offset, k): &(f32, f32)| format!(r##"<stop offset="{offset}" stop-color="#FFFFFF" stop-opacity="{}"/>"##, k * strength))
        .collect();
    format!(r#"<linearGradient id="{id}" gradientUnits="userSpaceOnUse" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}">{stops}</linearGradient>"#)
}

/// The installer's box, 256 units square: a glassy cube seen from above a corner - its top with a strip of tape, its left
/// face with the design's words, its right face showing the logo, and a reflection on each, in the face's own
/// perspective: slanting down to the right, across the left face's top corner and the right face's foot, clear of what's
/// printed on them.
pub fn box_svg(d: &Design, logo: &str) -> String {
    let (top_a, top_b) = (shade(&d.glass, 1.6), shade(&d.glass, 1.0));
    let (left_a, left_b) = (shade(&d.glass, 1.0), shade(&d.glass, 0.72));
    let (right_a, right_b) = (shade(&d.glass, 0.9), shade(&d.glass, 0.62));
    let (text, size) = face_lines(&d.face_text);
    // The logo stays vectors: nested, on a unit square, so that whatever renders the box renders it at its own size.
    let logo = nested(logo, r#"x="0" y="0" width="1" height="1""#);
    let gloss = d.gloss.clamp(0.0, 1.0);
    let reflections = [
        band("gloss-left", (0.75, 0.45, 0.0), 0.16, d.gloss_width, gloss),
        band("gloss-right", (0.75, 0.45, 0.0), 1.08, d.gloss_width, gloss),
        band("gloss-top", (0.7, -0.7, 0.5), 0.5, d.gloss_width, gloss * 0.6),
    ]
    .concat();
    let tape_y = 0.5 - d.tape_width / 2.0;
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 256 256" width="256" height="256">
  <defs>
    <linearGradient id="top" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{top_a}" stop-opacity="0.95"/><stop offset="1" stop-color="{top_b}" stop-opacity="0.9"/></linearGradient>
    <linearGradient id="left" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{left_a}" stop-opacity="0.9"/><stop offset="1" stop-color="{left_b}" stop-opacity="0.85"/></linearGradient>
    <linearGradient id="right" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{right_a}" stop-opacity="0.9"/><stop offset="1" stop-color="{right_b}" stop-opacity="0.88"/></linearGradient>
    {reflections}
    <filter id="floor" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="6"/></filter>
  </defs>
  <ellipse cx="128" cy="240" rx="92" ry="10" fill="#000" opacity="0.28" filter="url(#floor)"/>
  <path d="M128 18 L226 66 L128 114 L30 66 Z" fill="url(#top)"/>
  <path d="M30 66 L128 114 L128 236 L30 188 Z" fill="url(#left)"/>
  <path d="M128 114 L226 66 L226 188 L128 236 Z" fill="url(#right)"/>
  <rect transform="{TOP_FACE}" y="{tape_y}" width="1" height="{tape_h}" fill="{tape_color}" opacity="{tape}"/>
  <!-- The faces' pictures, each laid onto its face. -->
  <g transform="matrix(0.98 0.48 0 1.22 30 66)">
    <g text-anchor="middle" font-family="Segoe UI, Segoe UI Symbol, Arial" font-weight="700" font-size="{size}" fill="{face}" opacity="0.92">{text}</g>
  </g>
  <g transform="matrix(88 -43 0 102 133 128)">{logo}</g>
  <!-- Glass: its reflections, then bright edges. -->
  <rect transform="{LEFT_FACE}" width="1" height="1" fill="url(#gloss-left)"/>
  <rect transform="{RIGHT_FACE}" width="1" height="1" fill="url(#gloss-right)"/>
  <rect transform="{TOP_FACE}" width="1" height="1" fill="url(#gloss-top)"/>
  <path d="M128 114 L128 236 M30 66 L128 114 L226 66" fill="none" stroke="#FFFFFF" stroke-width="2.5" stroke-opacity="0.85" stroke-linejoin="round"/>
  <path d="M128 18 L226 66 L226 188 L128 236 L30 188 L30 66 Z" fill="none" stroke="#FFFFFF" stroke-width="2" stroke-opacity="0.6" stroke-linejoin="round"/>
</svg>"##,
        face = d.face_color,
        tape_h = d.tape_width,
        tape_color = d.tape_color,
        tape = d.tape.clamp(0.0, 1.0),
    )
}

/// The words as lines, centered on the face as a block, and their size: as big as one line has it, until they'd
/// stop fitting the face.
fn face_lines(words: &str) -> (String, f32) {
    let words = words.replace(r"\n", "\n");
    let lines: Vec<&str> = words.lines().collect();
    let count = lines.len().max(1) as f32;
    let size = (90.0 / count).min(30.0);
    let baseline = |i: usize| ((62.0 + (i as f32 - (count - 1.0) / 2.0) * size * 1.1) * 10.0).round() / 10.0;
    let text = lines.iter().enumerate().map(|(i, line)| format!(r#"<text x="50" y="{}">{}</text>"#, baseline(i), escape(line))).collect();
    (text, size)
}

/// `svg`, a whole 256 by 256 picture, placed inside another as `at` says.
fn nested(svg: &str, at: &str) -> String {
    svg.replacen(r#"width="256" height="256""#, at, 1)
}

/// The wizard's tall picture, 164 by 314 units: the box low down, on nothing, with the backdrop showing through.
pub fn wizard_svg(boxed: &str) -> String {
    let boxed = nested(boxed, r#"x="17" y="160" width="130" height="130""#);
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 164 314" width="164" height="314" preserveAspectRatio="none">
  {boxed}
</svg>"##
    )
}

/// What the settings tile behind everything (pms-gui's dollars.rs, `draw_tile`, and see `copy::LEGAL` there), as SVG: two
/// bills, each tilted its own way, with the slogan between them, on a tile of 288 by 184.
fn bills() -> String {
    let bill = |x: f32, y: f32, tilt: f32| {
        let corners: String = [(-25.0, -8.5), (25.0, -8.5), (-25.0, 8.5), (25.0, 8.5)].iter().map(|(dx, dy): &(f32, f32)| format!(r#"<circle cx="{}" cy="{}" r="1.4" stroke="none"/>"#, x + dx, y + dy)).collect();
        format!(
            r#"<g transform="rotate({tilt} {x} {y})" fill="none"><rect x="{x0}" y="{y0}" width="72" height="38" rx="5" stroke-width="1.8"/><rect x="{x1}" y="{y1}" width="64" height="30" rx="2.5" stroke-width="0.8"/><circle cx="{x}" cy="{y}" r="10.5" stroke-width="1.2"/><g fill="currentColor">{corners}<text x="{x}" y="{sign}" font-size="17" stroke="none">$</text></g></g>"#,
            x0 = x - 36.0,
            y0 = y - 19.0,
            x1 = x - 32.0,
            y1 = y - 15.0,
            sign = y + 5.5,
        )
    };
    let say = |x: f32, y: f32| format!(r#"<text x="{x}" y="{}" font-size="21" fill="currentColor" stroke="none">BUY $MSFT</text>"#, y + 7.0);
    format!("{}{}{}{}", bill(50.0, 48.0, -14.0), bill(200.0, 140.0, 11.0), say(206.0, 48.0), say(74.0, 140.0))
}

/// The tile with flags, still: as dollars.rs has them at rest, and the slogans between them.
fn flags(color: &str) -> Result<String, String> {
    let flat = flat_flag(color);
    let mut tile = String::new();
    for (x, middle, ahead) in flag::FLAGS {
        let png = b64(&bent(&flat, 4.0, flag::REST + ahead)?);
        tile += &format!(
            r#"<image x="{x}" y="{y}" width="{w}" height="{h}" preserveAspectRatio="none" xlink:href="data:image/png;base64,{png}"/>"#,
            y = middle - flag::FLAT_H / 2.0 - flag::PAD,
            w = flag::FLAT_W,
            h = flag::FLAT_H + 2.0 * flag::PAD,
        );
    }
    for (x, y) in flag::SLOGANS {
        tile += &format!(r#"<text x="{x}" y="{}" font-size="21" fill="currentColor" stroke="none">BUY $MSFT</text>"#, y + 7.0);
    }
    Ok(tile)
}

/// The wizard's background, 596 by 432 units, as the settings have it: the tile crisp where nobody reads and blurred where
/// they do, over `wizard_bg`.
pub fn backdrop_svg(d: &Design) -> Result<String, String> {
    let scale = d.dollar_size.max(6.0) / 22.0;
    let tile = match d.backdrop_bill {
        Bill::Classic => bills(),
        Bill::Flag => flags(&d.dollar_color)?,
    };
    Ok(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 596 432" width="596" height="432" preserveAspectRatio="none">
  <defs>
    <pattern id="tile" width="288" height="184" patternUnits="userSpaceOnUse" patternTransform="rotate({tilt}) scale({scale})">
      <g color="{color}" stroke="{color}" font-family="Segoe UI Semibold, Segoe UI, Arial" font-weight="600" text-anchor="middle">{tile}</g>
    </pattern>
    <filter id="soft" x="-5%" y="-5%" width="110%" height="110%"><feGaussianBlur stdDeviation="4.5"/></filter>
    <linearGradient id="fade" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0.42" stop-color="#fff" stop-opacity="0"/><stop offset="0.8" stop-color="#fff" stop-opacity="1"/>
    </linearGradient>
    <mask id="crisp" maskUnits="userSpaceOnUse" x="0" y="0" width="596" height="432"><rect width="596" height="432" fill="url(#fade)"/></mask>
    <radialGradient id="light" cx="0.15" cy="0" r="0.9"><stop offset="0" stop-color="{color}" stop-opacity="0.13"/><stop offset="1" stop-color="{color}" stop-opacity="0"/></radialGradient>
    <linearGradient id="foot" x1="0" y1="0" x2="0" y2="1"><stop offset="0.8" stop-color="#000" stop-opacity="0"/><stop offset="0.87" stop-color="#000" stop-opacity="0.6"/></linearGradient>
  </defs>
  <rect width="596" height="432" fill="{bg}"/>
  <rect x="-20" y="-20" width="636" height="472" fill="url(#tile)" opacity="{soft}" filter="url(#soft)"/>
  <rect width="596" height="432" fill="url(#tile)" opacity="{crisp}" mask="url(#crisp)"/>
  <rect width="596" height="432" fill="url(#light)"/>
  <rect width="596" height="432" fill="url(#foot)"/>
</svg>"##,
        tilt = d.dollar_tilt,
        color = d.dollar_color,
        crisp = d.dollar_strength.clamp(0.0, 1.0),
        soft = (d.dollar_strength * 0.75).clamp(0.0, 1.0),
        bg = d.wizard_bg,
    ))
}

/// An SVG at `w` by `h` pixels: RGBA (unpremultiplied), for the customizer's previews.
pub fn rgba(svg: &str, w: u32, h: u32) -> Result<Vec<u8>, String> {
    let pixmap = pixmap(svg, w, h)?;
    Ok(pixmap.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }).collect())
}

/// An SVG at `w` by `h` pixels, as a PNG.
pub fn png(svg: &str, w: u32, h: u32) -> Result<Vec<u8>, String> {
    pixmap(svg, w, h)?.encode_png().map_err(|e| e.to_string())
}

/// An SVG at `w` by `h` pixels, as a PNG, drawn bigger and averaged down: edges come out smoother than the renderer's own,
/// and a picture inside is shrunk evenly, which the renderer doesn't do well. The smaller the picture, the more it gains.
fn smooth_png(svg: &str, w: u32, h: u32) -> Result<Vec<u8>, String> {
    let ss = match w.max(h) {
        0..=32 => 8,
        33..=1000 => 4,
        _ => 2,
    };
    let big = pixmap(svg, w * ss, h * ss)?;
    let mut out = tiny_skia::Pixmap::new(w, h).ok_or("no pixmap")?;
    let (pixels, wide, n) = (big.pixels(), (w * ss) as usize, ss * ss);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            for dy in 0..ss {
                for dx in 0..ss {
                    let p = pixels[(y * ss + dy) as usize * wide + (x * ss + dx) as usize];
                    for (s, v) in sum.iter_mut().zip([p.red(), p.green(), p.blue(), p.alpha()]) {
                        *s += v as u32;
                    }
                }
            }
            let [r, g, b, a] = sum.map(|s| ((s + n / 2) / n) as u8);
            // Rounding can leave a color a step above its alpha.
            if let Some(p) = tiny_skia::PremultipliedColorU8::from_rgba(r.min(a), g.min(a), b.min(a), a) {
                out.pixels_mut()[(y * w + x) as usize] = p;
            }
        }
    }
    out.encode_png().map_err(|e| e.to_string())
}

fn pixmap(svg: &str, w: u32, h: u32) -> Result<tiny_skia::Pixmap, String> {
    static FONTS: std::sync::OnceLock<std::sync::Arc<usvg::fontdb::Database>> = std::sync::OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        std::sync::Arc::new(db)
    });
    let options = usvg::Options { fontdb: fonts.clone(), ..Default::default() };
    let tree = usvg::Tree::from_str(svg, &options).map_err(|e| format!("SVG: {e}"))?;
    let size = tree.size();
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or("no pixmap")?;
    let transform = tiny_skia::Transform::from_scale(w as f32 / size.width(), h as f32 / size.height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

pub fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// An .ico holding each size as a PNG (which Windows reads at every size since Vista).
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, png) in images {
        let side = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        out.extend_from_slice(png);
    }
    out
}

/// The crab, as Openclipart has it.
pub fn crab(root: &Path) -> Result<String, String> {
    std::fs::read_to_string(root.join("art/openclipart-192996/red-crab.svg")).map_err(|e| format!("the crab: {e}"))
}

/// Every picture, written where the build and the installer take them from: assets/icon (pms.ico, a PNG per size,
/// pms.svg) and installer/art (setup.ico, box.svg, the wizard and corner pictures). Says what it wrote.
pub fn write_all(root: &Path, d: &Design) -> Result<String, String> {
    // Windows keeps some files mapped while it shows them (an .ico, for its icon or thumbnail, even after the window
    // closes), and a mapped file can't be written over - but it can be renamed: the old one moves aside, into
    // %TEMP%, and the new one takes its place.
    let write = |path: std::path::PathBuf, bytes: &[u8]| {
        let first = std::fs::write(&path, bytes);
        match first {
            Err(e) if e.raw_os_error() == Some(1224) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                let aside = std::env::temp_dir().join(format!("pms-art-{stamp}-{name}"));
                std::fs::rename(&path, &aside).map_err(|e| format!("{} is held open by another program and can't be moved aside: {e}", path.display()))?;
                std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))
            }
            other => other.map_err(|e| format!("{}: {e}", path.display())),
        }
    };
    let crab = crab(root)?;
    let logo = logo_svg(d, &crab, d.icon_bill)?;
    let icons = root.join("assets/icon");
    std::fs::create_dir_all(&icons).map_err(|e| e.to_string())?;
    write(icons.join("pms.svg"), logo.as_bytes())?;
    let mut pngs = Vec::new();
    for size in ICON_SIZES {
        let png = png(&logo, size, size)?;
        write(icons.join(format!("pms-{size}.png")), &png)?;
        pngs.push((size, png));
    }
    write(icons.join("pms.ico"), &ico(&pngs))?;

    let boxed = box_svg(d, &logo_svg(d, &crab, d.box_bill)?);
    let art = root.join("installer/art");
    std::fs::create_dir_all(&art).map_err(|e| e.to_string())?;
    write(art.join("box.svg"), boxed.as_bytes())?;
    let mut box_pngs = Vec::new();
    for size in ICON_SIZES {
        box_pngs.push((size, smooth_png(&boxed, size, size)?));
    }
    write(art.join("setup.ico"), &ico(&box_pngs))?;
    for (w, h) in WIZARD_SIZES {
        write(art.join(format!("wizard-{w}x{h}.png")), &smooth_png(&wizard_svg(&boxed), w, h)?)?;
    }
    for (w, h) in BACKDROP_SIZES {
        write(art.join(format!("backdrop-{w}x{h}.png")), &smooth_png(&backdrop_svg(d)?, w, h)?)?;
    }
    for size in CORNER_SIZES {
        write(art.join(format!("corner-{size}.png")), &smooth_png(&boxed, size, size)?)?;
    }
    Ok(format!("{} (pms.ico, {} sizes); {} (setup.ico, {} wizard, {} backdrop and {} corner pictures)", icons.display(), ICON_SIZES.len(), art.display(), WIZARD_SIZES.len(), BACKDROP_SIZES.len(), CORNER_SIZES.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_band_runs_across_its_face() {
        // Strongest at the middle, so the gradient's ends are `width` to either side of it, along the slant.
        let g = band("g", (1.0, 0.0, 0.0), 0.5, 0.1, 0.3);
        assert!(g.contains(r#"x1="0.4" y1="0" x2="0.6" y2="0""#), "{g}");
        assert!(g.contains(r##"offset="0.5" stop-color="#FFFFFF" stop-opacity="0.3""##), "{g}");
    }

    #[test]
    fn the_box_is_vectors_all_the_way_down() {
        let d = Design::default();
        let logo = logo_svg(&d, "<svg></svg>", Bill::Classic).unwrap();
        let wizard = wizard_svg(&box_svg(&d, &logo));
        assert!(!wizard.contains("base64"), "a picture inside is a picture resampled");
        assert!(pixmap(&wizard, 202, 386).unwrap().pixels().iter().any(|p| p.alpha() > 0));
    }

    #[test]
    fn smoothing_keeps_a_picture_where_it_was() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" width="10" height="10"><rect width="5" height="10" fill="#fff"/></svg>"##;
        let png = smooth_png(svg, 10, 10).unwrap();
        let picture = tiny_skia::Pixmap::decode_png(&png).unwrap();
        assert_eq!((picture.width(), picture.height()), (10, 10));
        assert_eq!(picture.pixels()[4].alpha(), 255);
        assert_eq!(picture.pixels()[5].alpha(), 0);
    }

    #[test]
    fn each_picture_has_its_own_bill() {
        let mut d = Design::default();
        assert_eq!((d.icon_bill, d.box_bill, d.backdrop_bill, d.settings_bill), (Bill::Classic, Bill::Classic, Bill::Flag, Bill::Flag));
        let classic = logo_svg(&d, "<svg></svg>", Bill::Classic).unwrap();
        let flagged = logo_svg(&d, "<svg></svg>", Bill::Flag).unwrap();
        assert!(!classic.contains("<image") && flagged.contains("<image"));
        assert!(backdrop_svg(&d).unwrap().contains("<image"));
        d.backdrop_bill = Bill::Classic;
        assert!(!backdrop_svg(&d).unwrap().contains("<image"));
    }

    #[test]
    fn a_design_from_before_there_was_a_choice_still_loads() {
        let d: Design = serde_json::from_str(r#"{"face_text": "BUY"}"#).unwrap();
        assert_eq!(d.icon_bill, Bill::Classic);
        assert_eq!(d.settings_bill, Bill::Flag);
        assert!(serde_json::to_string_pretty(&Design { settings_bill: Bill::Classic, ..d }).unwrap().contains(r#""settings_bill": "classic""#), "dollars.rs looks for this");
    }

    #[test]
    fn what_lies_on_top_comes_last() {
        let crab = r#"<svg id="crab"></svg>"#;
        let mut d = Design::default();
        let cash_on_top = logo_svg(&d, crab, Bill::Classic).unwrap();
        assert!(cash_on_top.find("crab").unwrap() < cash_on_top.find("translate(128").unwrap());
        d.crab_on_top = true;
        let crab_on_top = logo_svg(&d, crab, Bill::Classic).unwrap();
        assert!(crab_on_top.find("translate(128").unwrap() < crab_on_top.find("crab").unwrap());
    }

    #[test]
    fn one_line_stays_where_it_was() {
        assert_eq!(face_lines("$MSFT"), (r#"<text x="50" y="62">$MSFT</text>"#.to_string(), 30.0));
    }

    #[test]
    fn lines_center_as_a_block() {
        let (text, size) = face_lines("BUY\n$MSFT");
        assert_eq!(size, 30.0);
        assert_eq!(text, r#"<text x="50" y="45.5">BUY</text><text x="50" y="78.5">$MSFT</text>"#);
        assert_eq!(face_lines(r"BUY\n$MSFT").0, text, "typed as a backslash and an n");
    }

    #[test]
    fn many_lines_shrink_to_fit_and_stay_escaped() {
        assert_eq!(face_lines("a\nb\nc\nd\ne").1, 18.0);
        assert!(face_lines("<&>\nx").0.contains("&lt;&amp;&gt;"));
        assert_eq!(face_lines("").0, "");
    }
}
