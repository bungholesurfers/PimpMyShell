//! `pms.toml`: everything the daemon keeps applied. Every section is optional; the meters
//! keys (`interval_ms`, `[counters]`, `[[meter]]`) stay top-level so `pms meters <file>`
//! reads the same file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub fn default_path() -> PathBuf {
    pms_tap::data_dir().join("pms.toml")
}

/// The example config, which a first run starts from.
pub const EXAMPLE: &str = include_str!("../../pms.example.toml");

/// A first run: no config yet at `path`, so start from the example.
pub fn ensure(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, EXAMPLE).map_err(|e| format!("{}: {e}", path.display()))
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_interval")]
    pub interval_ms: u64,
    /// Meter cell height: "compact" (75% of the taskbar), "tall" (87.5%, default) or "full".
    #[serde(default = "default_height")]
    pub meter_height: String,
    /// Font for meter values; default "Cascadia Mono, Consolas" (fixed-width digits).
    pub value_font: Option<String>,
    /// "auto" (default: follow Windows' light/dark mode for the taskbar), "dark" or "light". Picks
    /// the backdrop's look and the color of our text.
    #[serde(default = "default_theme", skip_serializing_if = "is_auto")]
    pub theme: String,
    /// Frame-rate cap for the long-running animations (the hot-mic alert), always; unset or 0: none, the
    /// display's full rate.
    pub max_fps: Option<u32>,
    /// The cap while the PC runs on its battery (unset: 30; 0: none). The lower of it and max_fps wins.
    pub battery_fps: Option<u32>,
    /// Count software adapters - Microsoft Basic Render Driver (WARP), Windows' CPU stand-in for a GPU - in GPU
    /// meters and `agg = "engine"` counters. Default false: they're left out.
    #[serde(default, skip_serializing_if = "is_false")]
    pub software_gpus: bool,
    /// Software update: whether the daemon checks GitHub for a newer version by itself, every day or two, and
    /// downloads it - installing only once asked. Unset: not asked yet (the daemon asks, once).
    pub update_check: Option<bool>,
    /// Which version software update offers: "stable" (default), the one the repository's update/stable.txt names,
    /// or "latest", the newest release published.
    #[serde(default = "default_channel", skip_serializing_if = "is_stable")]
    pub update_channel: String,
    #[serde(default)]
    pub counters: BTreeMap<String, CounterSpec>,
    #[serde(default, rename = "meter")]
    pub meters: Vec<MeterSpec>,
    /// Absent: keep the stock acrylic.
    pub backdrop: Option<Backdrop>,
    /// Absent: don't watch the microphone.
    pub hotmic: Option<Hotmic>,
    /// Absent: native toasts as usual.
    pub toasts: Option<Toasts>,
    /// Absent: the Start button stays as it is.
    pub start_glow: Option<StartGlow>,
    /// Absent: the app buttons stay as they are.
    pub app_glow: Option<AppGlow>,
}

fn is_history(s: &Style) -> bool {
    *s == Style::History
}

fn default_interval() -> u64 {
    1000
}

fn default_height() -> String {
    "tall".into()
}

fn default_theme() -> String {
    "auto".into()
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_auto(s: &str) -> bool {
    s == "auto"
}

fn default_channel() -> String {
    "stable".into()
}

fn is_stable(s: &str) -> bool {
    s == "stable"
}

impl Config {
    /// Software update's channel is the newest release, not the stable tag.
    pub fn latest_channel(&self) -> bool {
        self.update_channel == "latest"
    }
}

/// Sets the top-level `key = value` (TOML) in the file at `path`, keeping everything else in it as it is, comments
/// too: an existing line `key = ...` before the first table is replaced, else the line goes first. The result must
/// load, or the file is left alone; it is written via a temporary file.
pub fn set_top_level(path: &Path, key: &str, value: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let line = format!("{key} = {value}");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let tables = lines.iter().position(|l| l.trim_start().starts_with('[')).unwrap_or(lines.len());
    let existing = lines[..tables].iter().position(|l| l.split('=').next().is_some_and(|k| k.trim() == key));
    match existing {
        Some(i) => lines[i] = line,
        None => lines.insert(0, line),
    }
    let new = lines.join("\n") + "\n";
    toml::from_str::<Config>(&new).map_err(|e| format!("setting {key}: {e}"))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, new).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load(path: &Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Write `config` to `path` (via a temporary file, so the daemon never reloads half a file).
/// Comments in the old file are not kept.
pub fn save(path: &Path, config: &Config) -> Result<(), String> {
    let text = to_toml(config)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn to_toml(config: &Config) -> Result<String, String> {
    let body = toml::to_string_pretty(config).map_err(|e| e.to_string())?;
    Ok(format!("# PimpMyShell config, written by pms-gui. See pms.example.toml for every option.

{body}"))
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum CounterSpec {
    Path(String),
    Full {
        path: String,
        #[serde(default)]
        agg: Agg,
        /// Used when the counter can't be read (e.g. no pagefile -> no Paging File instances).
        default: Option<f64>,
    },
}

#[derive(Deserialize, Serialize, Default, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Agg {
    #[default]
    Sum,
    Max,
    Min,
    Avg,
    /// GPU engines, the Task Manager way: sum over processes per engine, then the busiest engine.
    Engine,
}

#[derive(Deserialize, Serialize, Clone, Copy, Default, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    #[default]
    History,
    Bar,
    Pie,
    Cores,
    /// A bar per GPU engine, shown only once it has done some work.
    Engines,
    /// A vertical bar per expression (e.g. NET up and down).
    Bars,
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum Exprs {
    One(String),
    Many(Vec<String>),
}

impl Exprs {
    pub fn list(&self) -> Vec<String> {
        match self {
            Exprs::One(e) => vec![e.clone()],
            Exprs::Many(v) => v.clone(),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum Max {
    Number(f64),
    Expr(String),
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MeterSpec {
    pub label: String,
    #[serde(default, skip_serializing_if = "is_history")]
    pub style: Style,
    /// One expression, or (pie) one per slice. Unused by `cores`.
    pub expr: Option<Exprs>,
    pub color: Option<String>,
    /// Pie slices; `cores`: [user, kernel].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<String>,
    /// Full scale: a number or an expression (`"vram_total"`). Omitted: history and bar
    /// auto-scale to recent values, a pie divides its slices by their sum.
    pub max: Option<Max>,
    /// This meter's own sample interval; default: the top-level `interval_ms`.
    pub interval_ms: Option<u64>,
    /// Meters in different groups get a separator between them.
    pub group: Option<String>,
    /// Show the value text (bar, pie, cores, engines; history always shows it). Default true.
    pub value: Option<bool>,
    /// Hover tooltip. Default true.
    pub tooltip: Option<bool>,
    /// cores / engines: bar width in DIPs (before any squeezing).
    pub bar_width: Option<f64>,
    /// Only show the meter while its value is at or above this (number or expression).
    pub show_above: Option<Max>,
    /// Pie: a name per slice, for the tooltip ("in use", "standby", ...).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    /// Pie: label inside the pie: "always", "never", or "auto" (default: only when space is short).
    pub label_inside: Option<String>,
    /// Engines: label each GPU by its maker (NVDA, AMD, INTC, QCOM, MSFT) in place of the caption, and light its bars
    /// in the maker's colors with a glow of them around (default true).
    pub vendors: Option<bool>,
    /// false: no label beside the meter (it stays in the tooltip).
    pub caption: Option<bool>,
    /// true: right against the meter before it, no separator or spacing between (whatever the groups).
    pub snug: Option<bool>,
    /// Engines: "ticker" (default: NVDA, AMD, INTC, QCOM, MSFT) or "short" (NV, RAD; the rest as tickers).
    pub vendor_label: Option<String>,
    /// Cores / engines: how the bars glow. "level" (default): by how busy they are now. "heat": each bar heats up as
    /// it works (to full in ~8 s at 100%) and cools when it stops (~30 s): the hotter, the wider and brighter its
    /// glow, and the brighter its fill; a CPU thread's kernel part glows in kernel's color. "off".
    pub glow: Option<String>,
    /// One of this meter for every graphics adapter with memory of its own (dedicated memory above 0), each
    /// reading that adapter's dedicated memory against its size, in its maker's color: `expr`, `max` and `color` are
    /// filled in. Software adapters only with software_gpus.
    pub per_gpu: Option<bool>,
    /// per_gpu: an adapter needs this much dedicated memory (MB) for a meter of its own (default 1024). Graphics
    /// built into the CPU borrow RAM, and often report a small reserved slice as dedicated: it would sit near full.
    pub min_memory_mb: Option<u64>,
    /// Pie: the fill (of the first slice, e.g. RAM's "in use") past which the resource is under pressure; the rim's
    /// glow swells from there (default 0.8). Below a third full there is no glow; it fades in by two thirds.
    pub pressure: Option<f64>,
    /// History: "log" (default: recent samples full width, older ones ever more compressed) or
    /// "linear" (one sample per bar).
    pub time_axis: Option<String>,
    /// History: how far back the graph reaches, in seconds (default 300).
    pub history_seconds: Option<u64>,
    /// Engines: how long an idle GPU stays before its bars fall off (default 15; 0 = for good).
    pub engine_hold_seconds: Option<u64>,
    /// "%", "bytes", "bytes/s", "bits/s", or any suffix to print after the number.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    pub decimals: Option<usize>,
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Backdrop {
    /// The dark look (and the only one, without `[backdrop.light]`): the tint over the blurred
    /// desktop, alpha its opacity (default #7F060606)...
    pub tint: Option<String>,
    /// ... grain opacity 0..1 (default 0.01) ...
    pub grain: Option<f32>,
    /// ... and paint for the free space between buttons.
    pub gaps: Option<String>,
    /// The look while Windows is in light mode (default: tint #C8F3F3F3, grain 0.01).
    pub light: Option<Look>,
}

#[derive(Deserialize, Serialize, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Look {
    pub tint: Option<String>,
    pub grain: Option<f32>,
    pub gaps: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Hotmic {
    pub color: String,
    /// One breath, low -> high -> low. 0 = steady at `high`.
    pub period_ms: u32,
    pub low: f32,
    pub high: f32,
    /// How long the whole taskbar shows the warning before it settles around the label.
    pub broad_ms: u32,
    /// Apps to ignore, matched case-insensitively against the consent-store key, e.g.
    /// "obs64.exe" or "Microsoft.WindowsSoundRecorder".
    pub ignore: Vec<String>,
}

impl Default for Hotmic {
    fn default() -> Self {
        Hotmic { color: "#E53935".into(), period_ms: 1600, low: 0.25, high: 0.7, broad_ms: 5000, ignore: Vec::new() }
    }
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Toasts {
    /// How long each toast stays (shorter while more are queued).
    pub duration_ms: u32,
    /// Apps that keep their native banners, matched case-insensitively against the app name or
    /// AUMID, e.g. "Teams" for incoming calls.
    pub ignore: Vec<String>,
}

/// A glow behind the Start button: none at rest, `hover` (opacity) while the pointer is over it, `press` while it's
/// pressed, `open` while its menu is open (opened from the keyboard too); fading over `hover_ms`, or the quicker
/// `press_ms` into and out of a press; reaching `hover_radius`, or `press_radius` pressed (DIPs).
#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct StartGlow {
    pub color: String,
    pub hover: f32,
    pub press: f32,
    pub open: f32,
    pub hover_ms: u32,
    pub press_ms: u32,
    pub hover_radius: f32,
    pub press_radius: f32,
}

impl Default for StartGlow {
    fn default() -> Self {
        StartGlow { color: "#4CC2FF".into(), hover: 0.45, press: 0.9, open: 0.6, hover_ms: 220, press_ms: 70, hover_radius: 120.0, press_radius: 170.0 }
    }
}

impl StartGlow {
    pub fn command(&self) -> String {
        let s = self;
        format!("buttonglow start {} {} {} {} {} {} {} {}", s.color, s.hover, s.press, s.open, s.hover_ms, s.press_ms, s.hover_radius, s.press_radius)
    }
}

/// Glows behind the apps' taskbar buttons, each in its icon's most prominent color: as `[start_glow]`, without a
/// color or an open state of its own.
#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct AppGlow {
    pub hover: f32,
    pub press: f32,
    pub hover_ms: u32,
    pub press_ms: u32,
    pub hover_radius: f32,
    pub press_radius: f32,
}

impl Default for AppGlow {
    fn default() -> Self {
        AppGlow { hover: 0.4, press: 0.8, hover_ms: 220, press_ms: 70, hover_radius: 90.0, press_radius: 130.0 }
    }
}

impl AppGlow {
    pub fn command(&self) -> String {
        let s = self;
        format!("buttonglow apps {} {} {} {} {} {}", s.hover, s.press, s.hover_ms, s.press_ms, s.hover_radius, s.press_radius)
    }
}

impl Default for Toasts {
    fn default() -> Self {
        Toasts { duration_ms: 7000, ignore: Vec::new() }
    }
}

impl Toasts {
    pub fn ignores(&self, app: &str, aumid: &str) -> bool {
        let (app, aumid) = (app.to_lowercase(), aumid.to_lowercase());
        self.ignore.iter().map(|i| i.to_lowercase()).any(|i| app.contains(&i) || aumid.contains(&i))
    }
}

impl Hotmic {
    pub fn command(&self) -> String {
        format!("alert broad {} {} {} {}", pms_tap::ipc::quote(&self.color), self.period_ms, self.low, self.high)
    }
}

impl Backdrop {
    pub fn commands(&self, light: bool) -> Vec<String> {
        std::iter::once("backdrop on".to_string()).chain(self.look(light)).collect()
    }

    /// Tint, grain and gaps for the dark or light look.
    pub fn look(&self, light: bool) -> Vec<String> {
        let q = pms_tap::ipc::quote;
        let (tint, grain, gaps) = match (light, &self.light) {
            (true, Some(l)) => (l.tint.clone().unwrap_or(LIGHT_TINT.into()), l.grain.unwrap_or(DEFAULT_GRAIN), l.gaps.clone()),
            (true, None) => (LIGHT_TINT.into(), self.grain.unwrap_or(DEFAULT_GRAIN), self.gaps.clone()),
            (false, _) => (self.tint.clone().unwrap_or(DARK_TINT.into()), self.grain.unwrap_or(DEFAULT_GRAIN), self.gaps.clone()),
        };
        let mut out = vec![format!("backdrop tint {}", q(&tint)), format!("backdrop grain {grain}")];
        out.push(match gaps {
            Some(g) => format!("backdrop gaps {}", q(&g)),
            None => "backdrop gaps off".into(),
        });
        out
    }
}

pub const DARK_TINT: &str = "#7F060606";
pub const DEFAULT_GRAIN: f32 = 0.01;
pub const LIGHT_TINT: &str = "#C8F3F3F3";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_round_trips() {
        let example = include_str!("../../pms.example.toml");
        let config: Config = toml::from_str(example).unwrap();
        let written = to_toml(&config).unwrap();
        let again: Config = toml::from_str(&written).unwrap_or_else(|e| panic!("{e}\n{written}"));
        assert_eq!(to_toml(&again).unwrap(), written);
        assert_eq!(again.meters.len(), config.meters.len());
        assert_eq!(again.counters.len(), config.counters.len());
    }

    #[test]
    fn top_level() {
        let path = std::env::temp_dir().join(format!("pms-config-test-{}.toml", std::process::id()));
        std::fs::write(&path, "# mine\ninterval_ms = 500\n\n[hotmic]\n# update_check = nope\n").unwrap();
        set_top_level(&path, "update_check", "true").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "update_check = true\n# mine\ninterval_ms = 500\n\n[hotmic]\n# update_check = nope\n");
        set_top_level(&path, "update_check", "false").unwrap();
        assert!(load(&path).unwrap().update_check == Some(false));
        assert!(set_top_level(&path, "update_check", "\"maybe\"").is_err());
        assert!(std::fs::read_to_string(&path).unwrap().starts_with("update_check = false\n# mine"));
        std::fs::remove_file(&path).unwrap();
    }
}
