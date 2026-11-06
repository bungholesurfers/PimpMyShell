//! `pms.toml`: everything the daemon keeps applied. Every section is optional; the meters
//! keys (`interval_ms`, `[counters]`, `[[meter]]`) stay top-level so `pms meters <file>`
//! reads the same file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where the user's settings live: `%APPDATA%\PimpMyShell`, the roaming profile, so they follow the user to another
/// PC (as VS Code's settings.json does). The machine's own files - logs, the TAP's copies, icons, the banners it
/// switched off, update state - stay in `pms_tap::data_dir()`, `%LOCALAPPDATA%\PimpMyShell`.
pub fn config_dir() -> PathBuf {
    match std::env::var_os("APPDATA") {
        Some(base) => PathBuf::from(base).join("PimpMyShell"),
        None => pms_tap::data_dir(),
    }
}

/// The config: `pms.toml` in `config_dir()`. One in the place it had before 0.1.3 (`legacy_path`) moves over the
/// first time this is asked, with the settings' backup of it.
pub fn default_path() -> PathBuf {
    static MOVED: std::sync::Once = std::sync::Once::new();
    let path = config_dir().join("pms.toml");
    MOVED.call_once(|| move_config(&legacy_path(), &path));
    path
}

/// Where the config was before 0.1.3: `%LOCALAPPDATA%\PimpMyShell\pms.toml`.
pub fn legacy_path() -> PathBuf {
    pms_tap::data_dir().join("pms.toml")
}

/// The config a program was given (its first argument), or the default one. The old default (a start-at-sign-in
/// entry written before 0.1.3 names it) means the default: the config has moved from there.
pub fn given(arg: Option<std::ffi::OsString>) -> PathBuf {
    match arg.map(PathBuf::from) {
        Some(p) if !same_file_name(&p, &legacy_path()) => p,
        _ => default_path(),
    }
}

/// Whether `path` is the default config (`default_path`).
pub fn is_default(path: &Path) -> bool {
    same_file_name(path, &default_path())
}

fn same_file_name(a: &Path, b: &Path) -> bool {
    let full = |p: &Path| std::path::absolute(p).map(|p| p.display().to_string().to_lowercase()).unwrap_or_default();
    full(a) == full(b)
}

/// Moves the config at `old` (and its `.toml.bak`) to `path`, unless there's one at `path` already: then nothing is
/// touched. A move it can't make (a roaming profile on another drive) copies instead; the old file then stays.
fn move_config(old: &Path, path: &Path) {
    if path.exists() || !old.is_file() {
        return;
    }
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        crate::log(format!("moving the config: {}: {e}", dir.display()));
        return;
    }
    for (from, to) in [(old.to_path_buf(), path.to_path_buf()), (old.with_extension("toml.bak"), path.with_extension("toml.bak"))] {
        if !from.is_file() || to.exists() {
            continue;
        }
        match std::fs::rename(&from, &to) {
            Ok(()) => crate::log(format!("moved {} to {}", from.display(), to.display())),
            Err(e) => match std::fs::copy(&from, &to) {
                Ok(_) => crate::log(format!("copied {} to {} (couldn't move it: {e}; the old one stays)", from.display(), to.display())),
                Err(e) => crate::log(format!("moving {} to {}: {e}", from.display(), to.display())),
            },
        }
    }
}

/// The example config, which a first run starts from.
pub const EXAMPLE: &str = include_str!("../../pms.example.toml");

/// What the defaults were in past releases, to tell a file's leftovers from its owner's choices (`adopt_layers`).
const PAST_DEFAULTS: [&str; 4] =
    [include_str!("../shipped/0.1.0.toml"), include_str!("../shipped/0.1.1.toml"), include_str!("../shipped/0.1.2.toml"), include_str!("../shipped/0.1.3.toml")];

/// Marks a file holding only what its owner changed, on top of `EXAMPLE` (layers.rs). A file without it holds every
/// setting, as files did before 0.1.4 (and an empty one means a stock taskbar).
const LAYERED: &str = "config_version";

const HEADER: &str = "# Your PimpMyShell settings: only what you've changed. Everything else comes from the defaults, which can
# improve with updates. pms.example.toml has every option, and the settings window changes them for you.

config_version = 2
";

/// A first run: no config yet at `path`, so start from the example.
pub fn ensure(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, HEADER).map_err(|e| format!("{}: {e}", path.display()))
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_interval")]
    pub interval_ms: u64,
    /// Meter cell height, but for bars: "compact" (75% of the taskbar), "tall" (87.5%, default) or "full" (100%).
    #[serde(default = "default_height")]
    pub meter_height: String,
    /// The same for meters of bars (threads, GPU engines, disks, adapters): default "full", all but a pixel of the taskbar.
    #[serde(default = "default_bar_height")]
    pub bar_height: String,
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
    /// Software update: whether the daemon checks GitHub for a newer version by itself (a while after each start,
    /// then every day or two) and downloads it - installing only once asked. On by default; unset means the first
    /// run's question isn't answered yet, and until it is (OK or closing it: on; no: off), nothing is checked.
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
    /// Absent (or `enabled = false`): native toasts as usual. `toasts()` is it while they're on.
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

fn default_bar_height() -> String {
    "full".into()
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

fn is_true(b: &bool) -> bool {
    *b
}

impl Config {
    /// Software update's channel is the newest release, not the stable tag.
    pub fn latest_channel(&self) -> bool {
        self.update_channel == "latest"
    }

    /// The toasts' settings while they're on: a `[toasts]` there, and not `enabled = false`.
    pub fn toasts(&self) -> Option<&Toasts> {
        self.toasts.as_ref().filter(|t| t.enabled)
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
    parse(&new).map_err(|e| format!("setting {key}: {e}"))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, new).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Sets `key = value` (TOML) in the table `[table]` of the file at `path`, keeping the rest as `set_top_level` does:
/// an existing line `key = ...` in the table is replaced, else the line goes right under its header. In a file of
/// changes, a table that isn't there yet comes at the end (it means the defaults, not "off", as in a file holding every
/// setting: there the table must be there). And the result must load, or the file is left alone.
pub fn set_in_table(path: &Path, table: &str, key: &str, value: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let header = format!("[{table}]");
    let found = lines.iter().position(|l| l.split('#').next().is_some_and(|h| h.trim() == header));
    let layered = lines.iter().any(|l| l.split('=').next().is_some_and(|k| k.trim() == LAYERED));
    let start = match found {
        Some(start) => start,
        None if layered => {
            lines.extend([String::new(), header.clone()]);
            lines.len() - 1
        }
        None => return Err(format!("no {header} in {}", path.display())),
    };
    let end = lines[start + 1..].iter().position(|l| l.trim_start().starts_with('[')).map_or(lines.len(), |i| start + 1 + i);
    let line = format!("{key} = {value}");
    match lines[start + 1..end].iter().position(|l| l.split('=').next().is_some_and(|k| k.trim() == key)) {
        Some(i) => lines[start + 1 + i] = line,
        None => lines.insert(start + 1, line),
    }
    let new = lines.join("\n") + "\n";
    parse(&new).map_err(|e| format!("setting {table}.{key}: {e}"))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, new).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load(path: &Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// A config from its text: what's in it, on top of the defaults if it's a file of changes.
pub fn parse(text: &str) -> Result<Config, String> {
    let mut over: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    if over.remove(LAYERED).is_none() {
        return toml::from_str(text).map_err(|e| e.to_string());
    }
    toml::Value::Table(crate::layers::merge(defaults(), &over)).try_into().map_err(|e: toml::de::Error| e.to_string())
}

/// `config`'s every setting, as `to_toml` compares them: a counter spelled out with nothing but the usual sum is the
/// same as its bare path, however it was written.
fn as_table(config: &Config) -> Result<toml::Table, String> {
    let mut plain = config.clone();
    for spec in plain.counters.values_mut() {
        if let CounterSpec::Full { path, agg: Agg::Sum, default: None } = spec {
            *spec = CounterSpec::Path(std::mem::take(path));
        }
    }
    toml::Table::try_from(&plain).map_err(|e| e.to_string())
}

/// The defaults, in the same form.
fn defaults() -> &'static toml::Table {
    static DEFAULTS: std::sync::OnceLock<toml::Table> = std::sync::OnceLock::new();
    DEFAULTS.get_or_init(|| {
        let example: Config = toml::from_str(EXAMPLE).expect("the bundled example parses");
        as_table(&example).expect("the bundled example has a table")
    })
}

fn file_text(over: &toml::Table) -> Result<String, String> {
    let body = toml::to_string_pretty(over).map_err(|e| e.to_string())?;
    Ok(if body.is_empty() { HEADER.to_string() } else { format!("{HEADER}\n{body}") })
}

/// A file holding every setting (from before 0.1.4) becomes one holding what its owner changed: what still matches a
/// past default goes, so that today's applies. Their file stays beside it as `.toml.full.bak`. Returns whether it did.
pub fn adopt_layers(path: &Path) -> Result<bool, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| format!("{}: {e}", path.display()))?;
    if table.contains_key(LAYERED) || text.trim().is_empty() {
        return Ok(false);
    }
    let full = as_table(&parse(&text)?)?;
    let past: Vec<toml::Table> = PAST_DEFAULTS.iter().filter_map(|t| parse(t).and_then(|c| as_table(&c)).ok()).collect();
    let adopted = crate::layers::adopt(defaults(), &full, &past);
    let backup = path.with_extension("toml.full.bak");
    if !backup.exists() {
        std::fs::write(&backup, &text).map_err(|e| format!("{}: {e}", backup.display()))?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, file_text(&crate::layers::diff(defaults(), &adopted))?).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

/// Write `config` to `path` (via a temporary file, so the daemon never reloads half a file): only what differs from
/// the defaults. Comments in the old file are not kept.
pub fn save(path: &Path, config: &Config) -> Result<(), String> {
    let text = to_toml(config)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Whether `config` is all defaults.
pub fn untouched(config: &Config) -> bool {
    to_toml(config).is_ok_and(|text| text == HEADER)
}

/// The file for `config`: what differs from the defaults, and nothing else.
pub fn to_toml(config: &Config) -> Result<String, String> {
    file_text(&crate::layers::diff(defaults(), &as_table(config)?))
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
    /// A pair of bars (write, read) per physical disk: the one Windows is on always, first; the others while busy.
    Disks,
    /// A pair of bars (up, down) per network interface: the one the internet route takes always, first; the others
    /// while busy.
    Nics,
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

/// A meter's `show_when`: it appears once `expr` reaches `at`, and leaves once `expr` is back under `until`
/// (default: `at`), so that a level hovering at `at` doesn't make it blink. E.g. the pagefile, only once RAM runs
/// short: `{ expr = "1 - mem_avail / ram_total", at = 0.85, until = 0.8 }`.
#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ShowWhen {
    pub expr: String,
    pub at: f64,
    pub until: Option<f64>,
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MeterSpec {
    pub label: String,
    #[serde(default, skip_serializing_if = "is_history")]
    pub style: Style,
    /// One expression, or (pie, bars) one per slice or bar. Unused by cores, engines, disks and nics.
    pub expr: Option<Exprs>,
    pub color: Option<String>,
    /// Pie slices; bars: one per bar; cores: [user, kernel]; disks: [write, read]; nics: [up, down]. Others: the
    /// first stands in for `color`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<String>,
    /// Full scale: a number or an expression (`"vram_total"`). Omitted: history, bar, bars,
    /// disks and nics auto-scale to recent values; a pie divides its slices by their sum.
    pub max: Option<Max>,
    /// This meter's own sample interval; default: the top-level `interval_ms`.
    pub interval_ms: Option<u64>,
    /// Meters in different groups get a separator between them.
    pub group: Option<String>,
    /// Show the value text (bar, pie, cores, engines, bars, disks, nics; history always shows it). Default true.
    pub value: Option<bool>,
    /// Hover tooltip. Default true.
    pub tooltip: Option<bool>,
    /// cores / engines / bars / disks / nics: bar width in DIPs (before any squeezing).
    pub bar_width: Option<f64>,
    /// Only show the meter while its value is at or above this (number or expression).
    pub show_above: Option<Max>,
    /// Only show the meter while something else says it matters (see ShowWhen).
    pub show_when: Option<ShowWhen>,
    /// Pie / bars: a name per slice or bar, for the tooltip ("in use", "standby", ...); disks / nics: each device's
    /// two bars (default write/read, up/down).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    /// Pie: label inside the pie: "always", "auto" (only when space is short) or "never" (default).
    pub label_inside: Option<String>,
    /// Engines: label each GPU by its maker (see `vendor_label`) in place of the caption, and light its bars
    /// in the maker's colors with a glow of them around (default true).
    pub vendors: Option<bool>,
    /// false: no label beside the meter (it stays in the tooltip).
    pub caption: Option<bool>,
    /// true: right against the meter before it, no separator or spacing between (whatever the groups).
    pub snug: Option<bool>,
    /// Engines: "short" (default: NV, RAD; the rest as tickers) or "ticker" (NVDA, AMD, INTC, QCOM, MSFT).
    pub vendor_label: Option<String>,
    /// Cores / engines: how the bars glow. "level" (default): by how busy they are now. "heat": each bar heats up as
    /// it works (to full in ~8 s at 100%) and cools when it stops (~30 s): the hotter, the wider and brighter its
    /// glow, and the brighter its fill; a CPU thread's kernel part glows in kernel's color. "off".
    pub glow: Option<String>,
    /// Cores: where each thread's kernel time sits in its bar: "bottom" (default; user time stacked above it) or
    /// "top" (above user time).
    pub kernel: Option<String>,
    /// One of this meter for every graphics adapter with at least `min_memory_mb` of dedicated memory (default 1024),
    /// each reading that adapter's dedicated memory against its size, in its maker's color (if known): `expr`, `max` and `color` are
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
    /// Engines, disks, nics: how long an idle GPU (or engine, with engines_fall_off), disk or network interface stays
    /// before its bars fall off (default 15; 0 = for good).
    #[serde(alias = "engine_hold_seconds")]
    pub hold_seconds: Option<u64>,
    /// Engines: idle engines fall off too, each on its own, not just whole GPUs.
    pub engines_fall_off: Option<bool>,
    /// "%", "bytes", "bytes/s", "bits/s", or any suffix to print after the number.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    pub decimals: Option<usize>,
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Backdrop {
    /// The dark look: the tint over the blurred desktop, alpha its opacity (default #7F060606)...
    pub tint: Option<String>,
    /// ... grain opacity 0..1 (default 0.01) ...
    pub grain: Option<f32>,
    /// ... and paint for the free space between buttons.
    pub gaps: Option<String>,
    /// The look while Windows is in light mode. Without it: tint #C8F3F3F3 with the grain and gaps above; with it,
    /// its unset keys are tint #C8F3F3F3, grain 0.01, no gaps.
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
    /// false: off, as if there were no `[toasts]`, with its settings kept for when they're on again (the
    /// reminder that notification access is off offers to turn them off so: notices.rs).
    #[serde(skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// How long each toast stays in front (a card that comes forward again with more behind it: at most 4 s). One
    /// that asks for longer gets at least 25 s, and reminders, alarms and calls stay until they're answered.
    pub duration_ms: u32,
    /// Apps that keep their native banners, matched case-insensitively against the app name or
    /// AUMID, e.g. "Teams" for incoming calls.
    pub ignore: Vec<String>,
    /// How cards look: "standard", or "pimped" (gold, with a sheen that sweeps across).
    #[serde(skip_serializing_if = "is_standard_card")]
    pub style: String,
    /// Before a notification opens something, show what it is and ask: "card", in the notification itself; "window", in
    /// a window of its own with room for all of it; "off". Answering *Shut up, computer* makes it "off".
    #[serde(skip_serializing_if = "is_card")]
    pub confirm_clicks: String,
}

fn is_card(how: &String) -> bool {
    how == "card"
}

fn is_standard_card(style: &String) -> bool {
    style == "standard"
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
        Toasts { enabled: true, duration_ms: 7000, ignore: Vec::new(), style: "standard".into(), confirm_clicks: "card".into() }
    }
}

impl Toasts {
    pub fn pimped(&self) -> bool {
        self.style == "pimped"
    }

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
        let again = parse(&written).unwrap_or_else(|e| panic!("{e}\n{written}"));
        assert_eq!(to_toml(&again).unwrap(), written);
        assert_eq!(again.meters.len(), config.meters.len());
        assert_eq!(again.counters.len(), config.counters.len());
    }

    /// A config survives being written and read, and writing it again changes nothing.
    fn round_trip(config: &Config) -> String {
        let text = to_toml(config).unwrap();
        let back = parse(&text).unwrap();
        assert_eq!(as_table(&back).unwrap(), as_table(config).unwrap());
        assert_eq!(to_toml(&back).unwrap(), text);
        text
    }

    #[test]
    fn the_defaults_write_nothing() {
        let shipped = parse(EXAMPLE).unwrap();
        assert_eq!(round_trip(&shipped), HEADER);
    }

    #[test]
    fn changes_write_only_themselves() {
        let mut c = parse(EXAMPLE).unwrap();
        c.interval_ms = 500;
        c.meters[0].group = Some("mine".into());
        c.toasts.as_mut().unwrap().duration_ms = 9000;
        c.backdrop = None;
        c.meters.remove(3);
        let text = round_trip(&c);
        for wanted in ["interval_ms = 500", "group = \"mine\"", "duration_ms = 9000", "unset = [\"backdrop\"]", "removed_meters"] {
            assert!(text.contains(wanted), "{wanted} in\n{text}");
        }
        assert!(!text.contains("colors") && !text.contains("hotmic"), "and nothing else:\n{text}");
    }

    #[test]
    fn a_counter_spelled_out_is_no_change() {
        let mut c = parse(EXAMPLE).unwrap();
        for spec in c.counters.values_mut() {
            if let CounterSpec::Full { path, agg: Agg::Sum, default: None } = spec {
                *spec = CounterSpec::Path(path.clone());
            }
        }
        assert_eq!(to_toml(&c).unwrap(), HEADER);
    }

    #[test]
    fn the_daemon_can_write_into_a_file_of_changes() {
        let path = std::env::temp_dir().join(format!("pms-config-layered-{}.toml", std::process::id()));
        std::fs::write(&path, HEADER).unwrap();
        // What it writes: the update question's answer, *Shut up, computer*, *Keep Windows notifications*.
        set_top_level(&path, "update_check", "true").unwrap();
        set_in_table(&path, "toasts", "confirm_clicks", "\"off\"").unwrap();
        set_in_table(&path, "toasts", "enabled", "false").unwrap();
        let c = load(&path).unwrap();
        assert_eq!(c.update_check, Some(true));
        let toasts = c.toasts.as_ref().unwrap();
        assert!(toasts.confirm_clicks == "off" && !toasts.enabled);
        assert_eq!(toasts.duration_ms, parse(EXAMPLE).unwrap().toasts.unwrap().duration_ms, "the rest is still the default");
        assert_eq!(c.meters.len(), parse(EXAMPLE).unwrap().meters.len());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn the_first_file_is_all_defaults() {
        let dir = std::env::temp_dir().join(format!("pms-config-ensure-{}", std::process::id()));
        let path = dir.join("pms.toml");
        ensure(&path).unwrap();
        assert_eq!(to_toml(&load(&path).unwrap()).unwrap(), HEADER);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn past_defaults_all_load() {
        for text in PAST_DEFAULTS {
            parse(text).unwrap();
        }
    }

    #[test]
    fn an_old_file_keeps_only_choices() {
        let path = std::env::temp_dir().join(format!("pms-config-adopt-{}.toml", std::process::id()));
        // 0.1.0's defaults, with one thing of the owner's.
        let text = format!("update_check = true\n{}", PAST_DEFAULTS[0]);
        std::fs::write(&path, &text).unwrap();
        assert!(adopt_layers(&path).unwrap());
        let adopted = std::fs::read_to_string(&path).unwrap();
        assert_eq!(adopted, format!("{HEADER}\nupdate_check = true\n"));
        let backup = path.with_extension("toml.full.bak");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), text);
        assert!(!adopt_layers(&path).unwrap(), "once");
        assert_eq!(load(&path).unwrap().meters.len(), parse(EXAMPLE).unwrap().meters.len());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(&backup).unwrap();
    }

    /// The daemon's stand-in for a config that doesn't load: empty, a stock taskbar.
    #[test]
    fn empty_is_stock() {
        assert!(parse("").unwrap().meters.is_empty());
        let c: Config = toml::from_str("").unwrap();
        assert!(c.toasts.is_none() && c.backdrop.is_none() && c.hotmic.is_none() && c.meters.is_empty() && c.update_check.is_none());
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

    #[test]
    fn moves_to_roaming() {
        let root = std::env::temp_dir().join(format!("pms-config-move-test-{}", std::process::id()));
        let (local, roaming) = (root.join("Local"), root.join("Roaming").join("PimpMyShell"));
        std::fs::create_dir_all(&local).unwrap();
        let (old, new) = (local.join("pms.toml"), roaming.join("pms.toml"));
        std::fs::write(&old, "# mine\n").unwrap();
        std::fs::write(old.with_extension("toml.bak"), "# my backup\n").unwrap();
        // Moved, with its backup, into a folder made for it.
        move_config(&old, &new);
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "# mine\n");
        assert_eq!(std::fs::read_to_string(new.with_extension("toml.bak")).unwrap(), "# my backup\n");
        assert!(!old.exists() && !old.with_extension("toml.bak").exists());
        // One there already: nothing is touched, either side.
        std::fs::write(&old, "# another\n").unwrap();
        move_config(&old, &new);
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "# mine\n");
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "# another\n");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn in_table() {
        let path = std::env::temp_dir().join(format!("pms-config-table-test-{}.toml", std::process::id()));
        let text = "[toasts] # mine\nduration_ms = 5000\n\n[start_glow]\nhover = 0.5\n";
        std::fs::write(&path, text).unwrap();
        assert!(load(&path).unwrap().toasts().is_some());
        // Under its header, the rest as it was; again, the same line replaced.
        set_in_table(&path, "toasts", "enabled", "false").unwrap();
        let off = "[toasts] # mine\nenabled = false\nduration_ms = 5000\n\n[start_glow]\nhover = 0.5\n";
        assert!(std::fs::read_to_string(&path).unwrap() == off);
        set_in_table(&path, "toasts", "enabled", "false").unwrap();
        assert!(std::fs::read_to_string(&path).unwrap() == off);
        let c = load(&path).unwrap();
        assert!(c.toasts().is_none() && c.toasts.as_ref().is_some_and(|t| t.duration_ms == 5000));
        // No such table, or a result that doesn't load: the file as it was.
        assert!(set_in_table(&path, "app_glow", "hover", "1").is_err());
        assert!(set_in_table(&path, "toasts", "enabled", "\"nah\"").is_err());
        assert!(std::fs::read_to_string(&path).unwrap() == off);
        std::fs::remove_file(&path).unwrap();
    }
}
