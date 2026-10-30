//! The pictures: the design (what the customizer's knobs set, kept in art/design.json), the SVG each picture is
//! drawn as from it, and their rendering (resvg) into the files the build and the installer use.

use std::path::Path;

use base64::Engine as _;
use resvg::{tiny_skia, usvg};
use serde::{Deserialize, Serialize};

pub const ICON_SIZES: [u32; 10] = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];
/// The wizard's tall picture at 100, 150 and 200% (Inno Setup picks the one that fits the display).
pub const WIZARD_SIZES: [(u32, u32); 3] = [(164, 314), (246, 471), (328, 628)];
pub const CORNER_SIZES: [u32; 3] = [55, 83, 110];

/// Everything the pictures are made of that the customizer can change. Coordinates are in the logo's 256 units.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Design {
    /// The crab: its width (of 256) and how far down its top is.
    pub crab_size: f32,
    pub crab_y: f32,
    /// The bill: its tilt (degrees), its width (of 256), the height of its middle, and its color.
    pub bill_tilt: f32,
    pub bill_size: f32,
    pub bill_y: f32,
    pub bill_color: String,
    /// The box: the tint of its glass, the words on its left face and their color.
    pub glass: String,
    pub face_text: String,
    pub face_color: String,
    /// The installer's tall picture: its background, and the dollar signs tiled over it - their color, strength
    /// (0..1), size (units of the picture's 164-wide frame) and tilt.
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
            bill_tilt: -9.0,
            bill_size: 200.0,
            bill_y: 185.0,
            bill_color: "#85BB65".into(),
            glass: "#A9D2F5".into(),
            face_text: "\u{2605}PMS".into(),
            face_color: "#FFFFFF".into(),
            wizard_bg: "#082641".into(),
            dollar_color: "#85BB65".into(),
            dollar_strength: 0.16,
            dollar_size: 22.0,
            dollar_tilt: -15.0,
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
pub fn logo_svg(d: &Design, crab: &str) -> String {
    // The crab's own <svg> goes in as a nested one, placed and sized here (its viewBox keeps its proportions).
    let crab = crab.find("<svg").map_or(crab, |at| &crab[at..]);
    let height = d.crab_size * 511.0 / 755.0;
    let crab = crab.replacen("<svg", &format!(r#"<svg x="{}" y="{}" width="{}" height="{height}""#, (256.0 - d.crab_size) / 2.0, d.crab_y, d.crab_size), 1);
    // The bill: 200 by 90 at full size, around its middle.
    let k = d.bill_size / 200.0;
    let (light, base, dark, line, ink) = (shade(&d.bill_color, 1.2), shade(&d.bill_color, 0.85), shade(&d.bill_color, 0.6), shade(&d.bill_color, 0.4), shade(&d.bill_color, 1.85));
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 256 256" width="256" height="256">
  <defs>
    <filter id="soft" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="4"/></filter>
    <linearGradient id="paper" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{light}"/><stop offset="1" stop-color="{base}"/></linearGradient>
  </defs>
  {crab}
  <g transform="translate(128 {y}) rotate({tilt}) scale({k}) translate(-128 -185)">
    <rect x="30" y="142" width="200" height="90" rx="7" fill="#000" opacity="0.35" filter="url(#soft)" transform="translate(3 5)"/>
    <rect x="28" y="140" width="200" height="90" rx="7" fill="url(#paper)" stroke="{line}" stroke-width="3"/>
    <rect x="37" y="149" width="182" height="72" rx="4" fill="none" stroke="{ink}" stroke-width="2.5" opacity="0.9"/>
    <ellipse cx="128" cy="185" rx="27" ry="28" fill="{dark}" stroke="{ink}" stroke-width="2.5"/>
    <path d="M140 170 C 136 164, 120 163, 117 171 C 113 181, 141 182, 139 194 C 137 204, 118 205, 114 197 M128 160 L128 210"
          fill="none" stroke="{ink}" stroke-width="7" stroke-linecap="round" stroke-linejoin="round"/>
    <g fill="{ink}" font-family="Segoe UI, Arial" font-weight="700" font-size="17">
      <text x="44" y="168">1</text><text x="203" y="168">1</text><text x="44" y="215">1</text><text x="203" y="215">1</text>
    </g>
  </g>
</svg>"##,
        y = d.bill_y,
        tilt = d.bill_tilt,
    )
}

/// The installer's box, 256 units square: a glassy cube seen from above a corner - its top, its left face with the
/// design's words, its right face showing the logo.
pub fn box_svg(d: &Design, logo_png: &str) -> String {
    let (top_a, top_b) = (shade(&d.glass, 1.6), shade(&d.glass, 1.0));
    let (left_a, left_b) = (shade(&d.glass, 1.0), shade(&d.glass, 0.72));
    let (right_a, right_b) = (shade(&d.glass, 0.9), shade(&d.glass, 0.62));
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 256 256" width="256" height="256">
  <defs>
    <linearGradient id="top" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{top_a}" stop-opacity="0.95"/><stop offset="1" stop-color="{top_b}" stop-opacity="0.9"/></linearGradient>
    <linearGradient id="left" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{left_a}" stop-opacity="0.9"/><stop offset="1" stop-color="{left_b}" stop-opacity="0.85"/></linearGradient>
    <linearGradient id="right" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{right_a}" stop-opacity="0.9"/><stop offset="1" stop-color="{right_b}" stop-opacity="0.88"/></linearGradient>
    <linearGradient id="glare" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#FFFFFF" stop-opacity="0"/><stop offset="0.5" stop-color="#FFFFFF" stop-opacity="0.45"/><stop offset="1" stop-color="#FFFFFF" stop-opacity="0"/></linearGradient>
    <filter id="soft" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="6"/></filter>
    <clipPath id="front"><path d="M30 66 L128 114 L128 236 L30 188 Z"/></clipPath>
  </defs>
  <ellipse cx="128" cy="240" rx="92" ry="10" fill="#000" opacity="0.28" filter="url(#soft)"/>
  <path d="M128 18 L226 66 L128 114 L30 66 Z" fill="url(#top)"/>
  <path d="M30 66 L128 114 L128 236 L30 188 Z" fill="url(#left)"/>
  <path d="M128 114 L226 66 L226 188 L128 236 Z" fill="url(#right)"/>
  <!-- The faces' pictures, each laid onto its face. -->
  <g transform="matrix(0.98 0.48 0 1.22 30 66)">
    <text x="50" y="62" text-anchor="middle" font-family="Segoe UI, Segoe UI Symbol, Arial" font-weight="700" font-size="30" fill="{face}" opacity="0.92">{text}</text>
  </g>
  <image x="0" y="0" width="1" height="1" preserveAspectRatio="xMidYMid meet" transform="matrix(88 -43 0 102 133 128)"
         xlink:href="data:image/png;base64,{logo_png}"/>
  <!-- Glass: bright edges, and a glare across the front face (that face only: let past its edges, it reads as a
       stray strip laid over the box). -->
  <path d="M128 114 L128 236 M30 66 L128 114 L226 66" fill="none" stroke="#FFFFFF" stroke-width="2.5" stroke-opacity="0.85" stroke-linejoin="round"/>
  <path d="M128 18 L226 66 L226 188 L128 236 L30 188 L30 66 Z" fill="none" stroke="#FFFFFF" stroke-width="2" stroke-opacity="0.6" stroke-linejoin="round"/>
  <path d="M30 150 L128 104 L128 128 L30 174 Z" fill="url(#glare)" opacity="0.35" clip-path="url(#front)"/>
</svg>"##,
        face = d.face_color,
        text = escape(&d.face_text),
    )
}

/// The wizard's tall picture, 164 by 314 units: dollar signs tiled over the background, light falling from above,
/// the box low down.
pub fn wizard_svg(d: &Design, box_png: &str) -> String {
    let s = d.dollar_size.max(6.0);
    // A tile holds two signs, the second offset half a tile, so the rows stagger like a pattern on paper money.
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 164 314" width="164" height="314" preserveAspectRatio="none">
  <defs>
    <pattern id="dollars" width="{s}" height="{s}" patternUnits="userSpaceOnUse" patternTransform="rotate({tilt})">
      <g font-family="Segoe UI, Arial" font-weight="700" font-size="{f}" fill="{color}" fill-opacity="{a}" text-anchor="middle">
        <text x="{q}" y="{base1}">$</text><text x="{q3}" y="{base2}">$</text>
      </g>
    </pattern>
    <radialGradient id="light" cx="0.5" cy="0" r="0.85"><stop offset="0" stop-color="#FFFFFF" stop-opacity="0.16"/><stop offset="1" stop-color="#FFFFFF" stop-opacity="0"/></radialGradient>
    <linearGradient id="foot" x1="0" y1="0" x2="0" y2="1"><stop offset="0.45" stop-color="#000" stop-opacity="0"/><stop offset="1" stop-color="#000" stop-opacity="0.45"/></linearGradient>
  </defs>
  <rect width="164" height="314" fill="{bg}"/>
  <rect width="164" height="314" fill="url(#dollars)"/>
  <rect width="164" height="314" fill="url(#light)"/>
  <rect width="164" height="314" fill="url(#foot)"/>
  <image x="17" y="160" width="130" height="130" xlink:href="data:image/png;base64,{box_png}"/>
</svg>"##,
        tilt = d.dollar_tilt,
        f = s * 0.55,
        color = d.dollar_color,
        a = d.dollar_strength.clamp(0.0, 1.0),
        q = s * 0.25,
        q3 = s * 0.75,
        base1 = s * 0.42,
        base2 = s * 0.92,
        bg = d.wizard_bg,
    )
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
    let logo = logo_svg(d, &crab(root)?);
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

    let boxed = box_svg(d, &b64(&png(&logo, 256, 256)?));
    let art = root.join("installer/art");
    std::fs::create_dir_all(&art).map_err(|e| e.to_string())?;
    write(art.join("box.svg"), boxed.as_bytes())?;
    let mut box_pngs = Vec::new();
    for size in ICON_SIZES {
        box_pngs.push((size, png(&boxed, size, size)?));
    }
    write(art.join("setup.ico"), &ico(&box_pngs))?;
    let box_png = b64(&png(&boxed, 512, 512)?);
    for (w, h) in WIZARD_SIZES {
        write(art.join(format!("wizard-{w}x{h}.png")), &png(&wizard_svg(d, &box_png), w, h)?)?;
    }
    for size in CORNER_SIZES {
        write(art.join(format!("corner-{size}.png")), &png(&boxed, size, size)?)?;
    }
    Ok(format!("{} (pms.ico, {} sizes); {} (setup.ico, {} wizard and {} corner pictures)", icons.display(), ICON_SIZES.len(), art.display(), WIZARD_SIZES.len(), CORNER_SIZES.len()))
}
