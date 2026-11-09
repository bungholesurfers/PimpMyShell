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

/// The folder of the config the daemon runs with, once it's said which: the TVs' saved state lives beside it, as the sets are named
/// there. A dev run on a config of its own keeps its own.
static RUNNING_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Says which config the daemon runs with, and brings the TVs' state over from where it was kept before (`pms_tap::data_dir()`),
/// once, for the default config.
pub fn running_with(path: &Path) {
    let dir = path.parent().map_or_else(config_dir, Path::to_path_buf);
    if is_default(path) {
        for name in ["tv.txt", "twatch.txt"] {
            let (old, new) = (pms_tap::data_dir().join(name), dir.join(name));
            if old.is_file() && !new.exists() {
                match std::fs::rename(&old, &new) {
                    Ok(()) => crate::log(format!("moved {name} beside the config")),
                    Err(e) => crate::log(format!("couldn't move {name} beside the config: {e}")),
                }
            }
        }
    }
    let _ = RUNNING_DIR.set(dir);
}

/// Where a file that goes with the running config's sets is kept: beside it.
pub fn beside_config(name: &str) -> PathBuf {
    RUNNING_DIR.get().cloned().unwrap_or_else(config_dir).join(name)
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
        crate::log(format!("couldn't create {} for the config: {e}", dir.display()));
        return;
    }
    for (from, to) in [(old.to_path_buf(), path.to_path_buf()), (old.with_extension("toml.bak"), path.with_extension("toml.bak"))] {
        if !from.is_file() || to.exists() {
            continue;
        }
        match std::fs::rename(&from, &to) {
            Ok(()) => crate::log(format!("moved {} to {}", from.display(), to.display())),
            Err(e) => match std::fs::copy(&from, &to) {
                Ok(_) => crate::log(format!("copied {} to {}, keeping the old one, as it couldn't be moved: {e}", from.display(), to.display())),
                Err(e) => crate::log(format!("couldn't move {} to {}: {e}", from.display(), to.display())),
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
    /// A TV landing on the console's channel switches the console on, instead of showing no signal. On unless you say otherwise.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub console_autostart: bool,
    /// A TV starting up runs through its channels, from `tv_intro_from` to `tv_intro_to`, game or no game. Off, it comes on where
    /// it was left.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub tv_intro: bool,
    /// The channel the run starts on (Captain Midnight's, 1295), and the one it ends on and stays (the console's, 6).
    #[serde(default = "intro_from", skip_serializing_if = "is_intro_from")]
    pub tv_intro_from: u32,
    #[serde(default = "intro_to", skip_serializing_if = "is_intro_to")]
    pub tv_intro_to: u32,
    /// How long every shuffling TV stays on a channel, in seconds.
    #[serde(default = "shuffle_seconds", skip_serializing_if = "is_usual_shuffle")]
    pub shuffle_seconds: u32,
    /// Shuffling TVs each go up to a quarter of that late. Off, they all go together.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub shuffle_jitter: bool,
    /// A notification when Explorer is holding a lot of ours (explorer.rs). Explorer is only ever restarted when you ask.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub notify_explorer: bool,
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
    /// What a Twatch TV (a `screen` meter with `set = "twatch"`) does. Absent: its defaults, with no favorites.
    pub twatch: Option<Twatch>,
    /// A proxy, and what goes through it (net.rs). It lives in a file of its own (`network_path`), as the favorites do, so that the
    /// settings file can be shared without it: read here from a file from before that, and not written here again. Absent: none.
    #[serde(default, skip_serializing)]
    pub network: Option<Network>,
    /// What the file says that can't be used, each as the error it would have been: a setting nobody knows, or a value that won't
    /// do. Each is left out, so that the rest still loads, and is for the settings to show (problem.rs reads them).
    #[serde(skip)]
    pub issues: Vec<String>,
}

/// A SOCKS5 proxy, and what goes through it: nothing, unless you switch it on. Whatever's to go through it and can't reach it
/// fails, rather than go without it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields, default)]
pub struct Network {
    /// "socks5://host:port", or "socks5h://host:port" for names to be looked up at the proxy too, with "user:password@" before the
    /// host if it wants them. Empty: no proxy.
    pub proxy: String,
    /// Twatch TV's looks at who's live, and its pictures.
    pub live_checks: bool,
    /// Twitch's streams: Streamlink finding them, and the video itself. They go together: the address Streamlink gets names the
    /// IP address that asked for it, so the video going another way would tie the two together.
    pub streams: bool,
    /// PimpMyShell's updates.
    pub updates: bool,
    /// Games, and Streamlink itself.
    pub downloads: bool,
    /// Each of those goes as somebody else: a user name and password of its own, made up at random each time PimpMyShell starts, in
    /// place of any in `proxy`. Tor keeps what comes with different ones on different circuits.
    pub isolate: bool,
    /// The video's own switch, from before it went with `streams`. Read, and forgotten.
    #[serde(default, skip_serializing)]
    pub video: Option<bool>,
}

/// The network settings' own file, beside the settings: a proxy's address, and the password it may have, aren't for sharing.
pub fn network_path(path: &Path) -> PathBuf {
    path.with_file_name("pms-network.toml")
}

/// The proxy, while the network settings' file won't load: nowhere, for everything, until it's fixed. Not knowing whether a
/// proxy was wanted, nothing goes without one.
pub const NETWORK_UNREAD: &str = "(pms-network.toml didn't load: fix it, and nothing goes out until then)";

const NETWORK_HEADER: &str = "# Your PimpMyShell network settings: a proxy, and what goes through it. They're kept out of pms.toml so that it can be
# shared without them. Settings > Network changes them for you; pms.example.toml says what each one is.

";

/// The network settings come from their own file. One that isn't there yet is made from what the settings used to hold.
fn take_in_network(path: &Path, config: &mut Config) {
    let file = network_path(path);
    match std::fs::read_to_string(&file) {
        Ok(text) => match toml::from_str::<Network>(&text) {
            Ok(n) => config.network = Some(n),
            // A file that won't read could have been a proxy: everything goes nowhere until it's fixed, rather than without one.
            Err(e) => {
                crate::log(format!("{}: {e}; nothing goes out over the network until it's fixed", file.display()));
                config.network = Some(Network { proxy: NETWORK_UNREAD.into(), live_checks: true, streams: true, updates: true, downloads: true, ..Default::default() });
            }
        },
        Err(_) => {
            if let Some(n) = &config.network {
                let _ = save_network(path, n);
            }
        }
    }
}

/// What's wrong with the network settings' file beside `path`, if it's there and won't load.
pub fn network_trouble(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(network_path(path)).ok()?;
    toml::from_str::<Network>(&text).err().map(|e| e.to_string())
}

/// Writes `network` to its own file, only what isn't the default: unless that's what's there already, or there's nothing to say
/// and no file yet.
pub fn save_network(path: &Path, network: &Network) -> Result<(), String> {
    let file = network_path(path);
    let was = std::fs::read_to_string(&file).ok();
    // What stands in for a file that won't load is never written over it: that's somebody's to fix.
    if network.proxy == NETWORK_UNREAD || was.as_deref().and_then(|t| toml::from_str::<Network>(t).ok()).as_ref() == Some(network) || (was.is_none() && *network == Network::default()) {
        return Ok(());
    }
    let mut mine = toml::Table::try_from(network).map_err(|e| e.to_string())?;
    let usual = toml::Table::try_from(Network::default()).map_err(|e| e.to_string())?;
    mine.retain(|key, value| usual.get(key) != Some(value));
    let text = format!("{NETWORK_HEADER}{}", toml::to_string(&mine).map_err(|e| e.to_string())?);
    let tmp = file.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &file).map_err(|e| format!("{}: {e}", file.display()))
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

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

fn intro_from() -> u32 {
    crate::tv::INTRO_FROM
}

fn is_intro_from(channel: &u32) -> bool {
    *channel == crate::tv::INTRO_FROM
}

fn intro_to() -> u32 {
    crate::tv::INTRO_TO
}

fn is_intro_to(channel: &u32) -> bool {
    *channel == crate::tv::INTRO_TO
}

fn shuffle_seconds() -> u32 {
    crate::tv::USUAL_SHUFFLE
}

fn is_usual_shuffle(seconds: &u32) -> bool {
    *seconds == crate::tv::USUAL_SHUFFLE
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
    // A new one goes with the others, under the header and before the first table.
    let after = lines[..tables].iter().rposition(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')).map_or(0, |i| i + 1);
    match existing {
        Some(i) => lines[i] = line,
        None => lines.insert(after, line),
    }
    let new = lines.join("\n") + "\n";
    strictly(&new, key).map_err(|e| format!("setting {key}: {e}"))?;
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
    strictly(&new, &format!("{table}.{key}")).map_err(|e| format!("setting {table}.{key}: {e}"))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, new).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load(path: &Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_at(path, &text)
}

/// `text`, the settings file at `path`, with what's kept beside it.
pub fn parse_at(path: &Path, text: &str) -> Result<Config, String> {
    let mut config = parse(text).map_err(|e| format!("{}: {e}", path.display()))?;
    take_in_watchlist(path, &mut config);
    take_in_network(path, &mut config);
    Ok(config)
}

/// Where the last settings that loaded are kept, for when these don't.
fn last_good_path(path: &Path) -> PathBuf {
    path.with_extension("toml.last-good")
}

/// The settings at `path` have just loaded: keeps a copy, for the day they don't.
pub fn remember_good(path: &Path) {
    let keep = last_good_path(path);
    let Ok(text) = std::fs::read(path) else { return };
    if std::fs::read(&keep).ok().as_ref() != Some(&text) {
        let _ = std::fs::write(keep, text);
    }
}

/// The last settings that loaded, if there are any.
pub fn last_good(path: &Path) -> Option<PathBuf> {
    Some(last_good_path(path)).filter(|p| p.is_file())
}

/// Puts the last settings that loaded back. What's there now stays beside them as `.toml.broken`, as it's someone's work.
pub fn restore_last_good(path: &Path) -> Result<(), String> {
    let good = last_good(path).ok_or("there are no earlier settings to go back to")?;
    if path.is_file() {
        std::fs::copy(path, path.with_extension("toml.broken")).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    std::fs::copy(&good, path).map(drop).map_err(|e| format!("{}: {e}", path.display()))
}

/// What a reset keeps: the answer about updates, which is yours more than a setting is. The favorites and the network settings have
/// files of their own (`watchlist_path`, `network_path`), and the games and what the TVs were showing are kept elsewhere, so those
/// stay too.
const KEPT_BY_RESET: [&str; 2] = ["update_check", "update_channel"];

/// Every setting at `path` back to its default, but for `KEPT_BY_RESET`. What was there stays beside it as
/// `.toml.before-reset`, even if it didn't load.
pub fn reset(path: &Path) -> Result<(), String> {
    let was = std::fs::read_to_string(path).unwrap_or_default();
    if !was.trim().is_empty() {
        let keep = path.with_extension("toml.before-reset");
        std::fs::write(&keep, &was).map_err(|e| format!("{}: {e}", keep.display()))?;
    }
    // Settings from before the favorites had their own file still hold them: reading them moves them there first.
    let _ = parse_at(path, &was);
    let old: toml::Table = toml::from_str(&was).unwrap_or_default();
    let kept: toml::Table = KEPT_BY_RESET.iter().filter_map(|key| Some((key.to_string(), old.get(*key)?.clone()))).collect();
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, file_text(&kept)?).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The favorites' own file, beside the settings: a line of it gone wrong can't stop the rest from loading.
pub fn watchlist_path(path: &Path) -> PathBuf {
    path.with_file_name("pms-twatch-watchlist.txt")
}

const WATCHLIST_HEADER: &str = "# Your Twatch TV favorites, best first: one channel per line. Names, @names and twitch.tv links all work.\n# Lines that start with # are left alone.\n";

/// What a watchlist says: the favorites, and the lines that can't be, with their numbers.
pub fn read_watchlist(text: &str) -> (Vec<String>, Vec<(usize, String)>) {
    let (mut good, mut bad) = (Vec::new(), Vec::new());
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        let line = line.split_once(" #").map_or(line, |(l, _)| l).trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (names, wrong) = crate::twatch::live::parse_list(line);
        good.extend(names);
        if !wrong.is_empty() {
            bad.push((i + 1, line.to_string()));
        }
    }
    let mut seen = Vec::new();
    good.retain(|h: &String| {
        let new = !seen.contains(h);
        seen.push(h.clone());
        new
    });
    (good, bad)
}

/// The lines of the watchlist beside `path` that can't be favorites.
pub fn watchlist_trouble(path: &Path) -> Vec<(usize, String)> {
    std::fs::read_to_string(watchlist_path(path)).map_or(Vec::new(), |text| read_watchlist(&text).1)
}

/// Favorites come from their own file. One that isn't there yet is made from the ones the settings used to hold.
fn take_in_watchlist(path: &Path, config: &mut Config) {
    match std::fs::read_to_string(watchlist_path(path)) {
        Ok(text) => {
            let (favorites, _) = read_watchlist(&text);
            if config.twatch.is_some() || !favorites.is_empty() {
                config.twatch.get_or_insert_with(Default::default).favorites = favorites;
            }
        }
        Err(_) => {
            if let Some(t) = config.twatch.as_ref().filter(|t| !t.favorites.is_empty()) {
                let _ = save_watchlist(path, &t.favorites);
            }
        }
    }
}

/// Writes `favorites`, unless that's what's there already, with the comments at the top of what's there.
pub fn save_watchlist(path: &Path, favorites: &[String]) -> Result<(), String> {
    let file = watchlist_path(path);
    let was = std::fs::read_to_string(&file).ok();
    if was.as_deref().map_or(favorites.is_empty(), |text| read_watchlist(text).0 == crate::twatch::live::tidy(favorites)) {
        return Ok(());
    }
    let top: String = was.iter().flat_map(|t| t.lines()).take_while(|l| l.trim_start().starts_with('#')).map(|l| format!("{l}\n")).collect();
    let top = if top.is_empty() { WATCHLIST_HEADER.to_string() } else { top };
    let text = format!("{top}{}", favorites.iter().map(|f| format!("{f}\n")).collect::<String>());
    let tmp = file.with_extension("txt.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &file).map_err(|e| format!("{}: {e}", file.display()))
}

/// `config` in a form to compare: the settings file, and the favorites and network settings, which aren't in it.
pub fn comparable(config: &Config) -> Result<String, String> {
    let favorites = config.twatch.as_ref().map_or(String::new(), |t| t.favorites.join("\n"));
    let network = config.network.as_ref().map_or(Ok(String::new()), toml::to_string).map_err(|e| e.to_string())?;
    Ok(format!("{}\n{favorites}\n{network}", to_toml(config)?))
}

/// A config from its text: what's in it, on top of the defaults if it's a file of changes. A setting that can't be used is left
/// out, and so goes back to its default, with what was wrong in `issues`: see `leniently`.
pub fn parse(text: &str) -> Result<Config, String> {
    let mut over: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let layered = over.remove(LAYERED).is_some();
    if !layered && let Ok(config) = toml::from_str::<Config>(text) {
        let config = brought_up_to_date(config);
        if not_usable(&config).is_none() {
            return Ok(config);
        }
    }
    leniently(over, layered)
}

/// `text` as settings, with `setting` usable: what's written into a file has to be, as opposed to what's found there.
fn strictly(text: &str, setting: &str) -> Result<Config, String> {
    let config = parse(text)?;
    match config.issues.iter().find(|e| issue_setting(e) == setting) {
        Some(e) => Err(e.clone()),
        None => Ok(config),
    }
}

/// The most settings that can't be used that a file can have before it's just not a settings file.
const MOST_ISSUES: usize = 64;

/// Your settings, `over` the defaults if `layered`, leaving out each one that can't be used, which so goes back to its default,
/// and keeping what was wrong with it in `issues`. That's a setting nobody knows any more, or one misspelled; a value of the wrong
/// kind; or one that isn't among the choices, is out of range, or isn't a color. One of those, which an update can bring about as
/// easily as a slip, shouldn't cost all the rest. What's left out is taken from your settings, not from what they make with the
/// defaults, so a meter whose style can't be used keeps the one it comes with. Anything else wrong is still an error.
fn leniently(mut over: toml::Table, layered: bool) -> Result<Config, String> {
    let mut issues = Vec::new();
    loop {
        let table = if layered { crate::layers::merge(defaults(), &over) } else { over.clone() };
        let said = match toml::Value::Table(table).try_into::<Config>() {
            Ok(config) => {
                let config = brought_up_to_date(config);
                match not_usable(&config) {
                    None => return Ok(Config { issues, ..config }),
                    Some(said) => said,
                }
            }
            Err(e) => e.to_string(),
        };
        let gone = match (unknown_field(&said), bad_value(&said)) {
            (Some((path, key)), _) => remove_at(&mut over, &path, &key, None),
            (None, Some((path, key, value))) => remove_at(&mut over, &path, &key, Some(&value)),
            _ => false,
        };
        if !gone || issues.len() >= MOST_ISSUES {
            return Err(said);
        }
        issues.push(said);
    }
}

/// The table and key an "unknown field" error is about: `("network", "video")`, or `("", "theme_x")` at the top.
fn unknown_field(error: &str) -> Option<(String, String)> {
    let rest = &error[error.find("unknown field `")? + "unknown field `".len()..];
    let key = rest.split('`').next()?.to_string();
    let path = error.rsplit_once("\nin `").and_then(|(_, p)| p.split('`').next()).unwrap_or("").to_string();
    Some((path, key))
}

/// The table, key and value an error about a value is about: `("toasts", "duration_ms", "long")`, `("", "theme", "5")`.
fn bad_value(error: &str) -> Option<(String, String, String)> {
    let path = error.rsplit_once("\nin `").and_then(|(_, p)| p.split('`').next())?;
    let (table, key) = path.rsplit_once('.').unwrap_or(("", path));
    let first = error.lines().next().unwrap_or(error);
    let value = match first.split_once("string \"") {
        Some((_, rest)) => rest.split('"').next()?,
        None => first.split('`').nth(1)?,
    };
    Some((table.to_string(), key.to_string(), value.to_string()))
}

/// Whether `value` is what an error calls `said`: itself, written out, or in it, for a list.
fn says(value: &toml::Value, said: &str) -> bool {
    match value {
        toml::Value::String(s) => s == said,
        toml::Value::Array(items) => items.iter().any(|v| says(v, said)),
        toml::Value::Table(_) => false,
        other => other.to_string() == said,
    }
}

/// Takes `key` out of the table at `path` ("toasts", "meter[2]", or "meter": every one of them that has it), if it's there, and
/// only where it's `value`, if that's given.
fn remove_at(table: &mut toml::Table, path: &str, key: &str, value: Option<&str>) -> bool {
    let Some((part, rest)) = path.split_once('.').or(Some((path, ""))).filter(|(p, _)| !p.is_empty()) else {
        let here = table.get(key).is_some_and(|v| value.is_none_or(|said| says(v, said)));
        return here && table.remove(key).is_some();
    };
    let (name, index) = match part.split_once('[') {
        Some((name, i)) => (name, i.trim_end_matches(']').parse::<usize>().ok()),
        None => (part, None),
    };
    match (table.get_mut(name), index) {
        (Some(toml::Value::Table(t)), None) => remove_at(t, rest, key, value),
        (Some(toml::Value::Array(a)), Some(i)) => match a.get_mut(i) {
            Some(toml::Value::Table(t)) => remove_at(t, rest, key, value),
            _ => false,
        },
        // A list, with no saying which: out of each that has it.
        (Some(toml::Value::Array(a)), None) => a.iter_mut().filter_map(|v| if let toml::Value::Table(t) = v { Some(remove_at(t, rest, key, value)) } else { None }).fold(false, |any, r| any | r),
        _ => false,
    }
}

/// The setting an issue is about, as the settings window knows it: `toasts.position`, `tv_intro_from`, `meter.style`.
pub fn issue_setting(error: &str) -> String {
    let (path, key) = unknown_field(error).or_else(|| bad_value(error).map(|(path, key, _)| (path, key))).unwrap_or_default();
    if path.is_empty() { key } else { format!("{path}.{key}") }
}

/// The settings window's page (its tag) that setting `id` is on: `toasts.position` is on the notifications page, `tv_intro_from`
/// on the TV's.
pub fn settings_page(id: &str) -> &'static str {
    let (table, key) = id.split_once('.').unwrap_or(("", id));
    match (table, key) {
        ("twatch", _) => "twatch",
        ("toasts", _) => "toasts",
        ("hotmic", _) => "hotmic",
        ("meter", _) => "meters",
        ("counters", _) => "counters",
        ("backdrop" | "start_glow" | "app_glow", _) => "look",
        ("network", _) => "network",
        ("", "console_autostart") => "poom",
        ("", k) if k.starts_with("tv_intro") || k.starts_with("shuffle_") => "tv",
        ("", "meter" | "meter_height" | "bar_height" | "software_gpus") => "meters",
        ("", "counters") => "counters",
        ("", "backdrop" | "start_glow" | "app_glow") => "look",
        ("", "twatch") => "twatch",
        ("", "toasts") => "toasts",
        ("", "hotmic") => "hotmic",
        _ => "general",
    }
}

/// Whether an issue is a setting nobody knows, as opposed to a value that can't be used.
pub fn issue_is_unknown(error: &str) -> bool {
    unknown_field(error).is_some()
}

/// The choices some settings have.
const THEMES: [&str; 3] = ["auto", "dark", "light"];
const HEIGHTS: [&str; 3] = ["compact", "tall", "full"];
const UPDATE_CHANNELS: [&str; 2] = ["stable", "latest"];
const PICTURES: [&str; 4] = ["never", "initial", "low_memory", "always"];
const HIGHER_FAVORITE: [&str; 2] = ["switch", "stay"];
const MEDIA_CONTROLS: [&str; 3] = ["never", "unmuted", "playing"];
const CARD_POSITIONS: [&str; 4] = ["clock", "tray", "tv", "mic"];
const CARD_STYLES: [&str; 2] = ["standard", "pimped"];
const CONFIRMS: [&str; 3] = ["card", "window", "off"];
const MIC_POSITIONS: [&str; 2] = ["tray", "start"];

/// Whether `s` is a color as the taskbar takes one: #RRGGBB, or #AARRGGBB.
fn is_color(s: &str) -> bool {
    let hex = s.strip_prefix('#').unwrap_or(s);
    matches!(hex.len(), 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The first setting in `c` that's of the right kind but can't be used, as reading it would have said, had it known: one that isn't
/// among the choices, is out of range, or isn't a color.
fn not_usable(c: &Config) -> Option<String> {
    fn choice(path: &str, v: &str, ok: &[&str]) -> Option<String> {
        (!ok.contains(&v)).then(|| format!("unknown variant `{v}`, expected one of {}\nin `{path}`", ok.iter().map(|o| format!("`{o}`")).collect::<Vec<_>>().join(", ")))
    }
    fn range(path: &str, v: u32, lo: u32, hi: u32) -> Option<String> {
        (v < lo || v > hi).then(|| format!("out of range: `{v}`, expected {lo} to {hi}\nin `{path}`"))
    }
    // Empty is no color, where a color is optional.
    fn color(path: &str, v: &str) -> Option<String> {
        (!v.is_empty() && !is_color(v)).then(|| format!("bad color `{v}`, expected #RRGGBB or #AARRGGBB\nin `{path}`"))
    }
    let channels: Vec<String> = crate::tv::CHANNELS.iter().map(u32::to_string).collect();
    let channels: Vec<&str> = channels.iter().map(String::as_str).collect();
    let mut found = vec![
        choice("theme", &c.theme, &THEMES),
        choice("meter_height", &c.meter_height, &HEIGHTS),
        choice("bar_height", &c.bar_height, &HEIGHTS),
        choice("update_channel", &c.update_channel, &UPDATE_CHANNELS),
        choice("tv_intro_from", &c.tv_intro_from.to_string(), &channels),
        choice("tv_intro_to", &c.tv_intro_to.to_string(), &channels),
        range("shuffle_seconds", c.shuffle_seconds, 5, 3600),
    ];
    if let Some(t) = &c.twatch {
        found.extend([
            choice("twatch.pictures", &t.pictures, &PICTURES),
            choice("twatch.top_quality", &t.top_quality, &crate::twatch::QUALITIES),
            choice("twatch.higher_favorite", &t.higher_favorite, &HIGHER_FAVORITE),
            choice("twatch.media_controls", &t.media_controls, &MEDIA_CONTROLS),
            range("twatch.more_sets", t.more_sets, 0, crate::twatch::MOST_MORE_SETS),
            range("twatch.check_seconds", t.check_seconds, crate::twatch::FASTEST_CHECK, 3600),
            range("twatch.check_extra_seconds", t.check_extra_seconds, 0, 3600),
            range("twatch.volume", t.volume, 0, 100),
        ]);
    }
    if let Some(t) = &c.toasts {
        found.extend([choice("toasts.position", &t.position, &CARD_POSITIONS), choice("toasts.style", &t.style, &CARD_STYLES), choice("toasts.confirm_clicks", &t.confirm_clicks, &CONFIRMS)]);
    }
    if let Some(h) = &c.hotmic {
        found.extend([color("hotmic.color", &h.color), choice("hotmic.position", &h.position, &MIC_POSITIONS)]);
    }
    if let Some(g) = &c.start_glow {
        found.push(color("start_glow.color", &g.color));
    }
    if let Some(b) = &c.backdrop {
        found.extend([color("backdrop.tint", b.tint.as_deref().unwrap_or_default()), color("backdrop.gaps", b.gaps.as_deref().unwrap_or_default())]);
        if let Some(l) = &b.light {
            found.extend([color("backdrop.light.tint", l.tint.as_deref().unwrap_or_default()), color("backdrop.light.gaps", l.gaps.as_deref().unwrap_or_default())]);
        }
    }
    for m in &c.meters {
        found.push(color("meter.color", m.color.as_deref().unwrap_or_default()));
        found.extend(m.colors.iter().map(|v| color("meter.colors", v)));
        found.push(color("meter.slice_glow_color", m.slice_glow_color.as_deref().unwrap_or_default()));
    }
    found.into_iter().flatten().next()
}

/// What older files said in ways that have changed.
fn brought_up_to_date(mut config: Config) -> Config {
    if let Some(n) = &mut config.network {
        n.video = None;
    }
    if let Some(t) = &mut config.twatch
        && t.thumbnails.take() == Some(true)
    {
        t.pictures = "always".into();
    }
    config
}

/// `config`'s every setting, as `to_toml` compares them: a counter spelled out with nothing but the usual sum is the
/// same as its bare path, however it was written. A Twatch TV's settings at their defaults aren't there, as with no
/// `[twatch]` at all: the shipped settings have none for them to be left out against, so written down, they'd stay as
/// they are when a default improves.
fn as_table(config: &Config) -> Result<toml::Table, String> {
    let mut plain = config.clone();
    for spec in plain.counters.values_mut() {
        if let CounterSpec::Full { path, agg: Agg::Sum, default: None } = spec {
            *spec = CounterSpec::Path(std::mem::take(path));
        }
    }
    let mut table = toml::Table::try_from(&plain).map_err(|e| e.to_string())?;
    // Twatch TV's table has a default for every value: only what isn't that is yours.
    let usual = toml::Table::try_from(Twatch::default()).map_err(|e| e.to_string())?;
    if let Some(toml::Value::Table(mine)) = table.get_mut("twatch") {
        mine.retain(|key, value| usual.get(key) != Some(value));
        if mine.is_empty() {
            table.remove("twatch");
        }
    }
    Ok(table)
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
    // First: the settings stop holding the favorites as soon as they're written, and they mustn't go missing.
    if let Some(t) = &config.twatch {
        save_watchlist(path, &t.favorites)?;
    }
    if let Some(n) = &config.network {
        save_network(path, n)?;
    }
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
    /// A live picture, drawn by another program (pms-screen): the screen named by the meter's label.
    Screen,
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

impl MeterSpec {
    /// Whether its label is on its side: as `sideways` says, or by default for disks and network interfaces.
    pub fn label_sideways(&self) -> bool {
        self.sideways.unwrap_or(matches!(self.style, Style::Disks | Style::Nics))
    }
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
    /// Screen: which set it is: "tv" (default, 4:3) or "twatch" (16:9).
    pub set: Option<String>,
    /// Engines: label each GPU by its maker (see `vendor_label`) in place of the caption, and light its bars
    /// in the maker's colors with a glow of them around (default true).
    pub vendors: Option<bool>,
    /// false: no label beside the meter (it stays in the tooltip).
    pub caption: Option<bool>,
    /// true: the label on its side, reading down, so that it takes little room across; a device's or GPU's label too. Unset:
    /// on for disks and nics, off for the rest.
    pub sideways: Option<bool>,
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
    /// Pie: a slice that glows, by its name in `names`. The glow follows the slice as it grows and shrinks.
    pub slice_glow: Option<String>,
    /// ... how far the slice can stand out past the pie's edge, in DIPs (default 2.5). It's the pie's own size at rest, and stands
    /// out only while its amount is changing, the more the faster; its share stays true.
    pub slice_glow_extend: Option<f64>,
    /// ... how far the glow reaches, in DIPs (default 4), and how strong it is, 0 to 1 (default 1).
    pub slice_glow_radius: Option<f64>,
    pub slice_glow_strength: Option<f64>,
    /// ... its color (default: the slice's), and how long a slow pulse takes, in milliseconds (default 0: it holds steady).
    pub slice_glow_color: Option<String>,
    pub slice_glow_pulse_ms: Option<u64>,
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
    /// Where the label naming the apps goes along the strip: "tray", after the meters, beside the system tray (default), or
    /// "start", before them.
    pub position: String,
}

impl Default for Hotmic {
    fn default() -> Self {
        Hotmic { color: "#E53935".into(), period_ms: 1600, low: 0.25, high: 0.7, broad_ms: 5000, ignore: Vec::new(), position: "tray".into() }
    }
}

impl Config {
    /// Where the microphone label goes along the strip: "tray" or "start".
    pub fn mic_position(&self) -> &str {
        self.hotmic.as_ref().map_or("tray", |h| h.position.as_str())
    }
}

#[derive(Deserialize, Serialize, Clone, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Twatch {
    /// Twitch channels, best first: the best one that's live goes on. They live in `watchlist_path`: read here from a file from
    /// before that, and not written here again.
    #[serde(skip_serializing)]
    pub favorites: Vec<String>,
    /// How many more sets may come out while more favorites are live, up to 2, and go away afterwards.
    pub more_sets: u32,
    /// When a still picture now and then takes the place of live video: "never", "initial" (until a set is clicked), "low_memory"
    /// (default: while RAM or VRAM is short), or "always".
    pub pictures: String,
    /// What `pictures` used to be. Read, turned into that, and not written again.
    #[serde(skip_serializing)]
    pub thumbnails: Option<bool>,
    /// Say so when a favorite goes live.
    pub notify_live: bool,
    /// The most it plays: "1080p60", "720p60", "720p", "480p" (default) or "360p". It goes lower by itself when it has to.
    pub top_quality: String,
    /// What a set does when a favorite above the one it's showing goes live: "switch" to it (default), or "stay" until this one ends.
    pub higher_favorite: String,
    /// How often, in seconds, each favorite is looked at (default 600): in no order, spread out, whatever its place in the list.
    pub check_seconds: u32,
    /// At most how many seconds more each wait is, picked afresh each time (default 300).
    pub check_extra_seconds: u32,
    /// How loud every set is, 0 to 100 (default 100): what Windows' volume mixer would show for it.
    pub volume: u32,
    /// Pictures in place of video while Windows says the connection is metered (default true).
    pub metered_pictures: bool,
    /// When a set shows in Windows' media controls (the volume flyout, the lock screen, media keys): "never" (default), "unmuted"
    /// (while its sound is on) or "playing" (whenever it's playing video).
    pub media_controls: String,
}

impl Default for Twatch {
    fn default() -> Self {
        Twatch { favorites: Vec::new(), more_sets: 0, pictures: "low_memory".into(), thumbnails: None, notify_live: true, top_quality: "480p".into(), higher_favorite: "switch".into(), check_seconds: crate::twatch::live::USUAL_CHECK, check_extra_seconds: crate::twatch::live::USUAL_CHECK_EXTRA, volume: 100, metered_pictures: true, media_controls: "never".into() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
    /// Where cards stop: left of "clock" (over the tray), "tray", "tv" (the first screen) or "mic" (its label, while it shows).
    /// What isn't showing counts as the tray.
    #[serde(skip_serializing_if = "is_clock")]
    pub position: String,
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

fn is_clock(s: &str) -> bool {
    s == "clock"
}

fn default_position() -> String {
    "clock".to_string()
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
        Toasts { enabled: true, duration_ms: 7000, ignore: Vec::new(), position: default_position(), style: "standard".into(), confirm_clicks: "card".into() }
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
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# mine\ninterval_ms = 500\nupdate_check = true\n\n[hotmic]\n# update_check = nope\n", "with the others");
        set_top_level(&path, "update_check", "false").unwrap();
        assert!(load(&path).unwrap().update_check == Some(false));
        assert!(set_top_level(&path, "update_check", "\"maybe\"").is_err());
        assert!(std::fs::read_to_string(&path).unwrap().starts_with("# mine\ninterval_ms = 500\nupdate_check = false\n"));
        // Under the header of a file of changes, not above it.
        std::fs::write(&path, HEADER).unwrap();
        set_top_level(&path, "update_check", "true").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{HEADER}update_check = true\n"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn twatch_keeps_only_what_you_changed() {
        let mut config = parse(HEADER).unwrap();
        config.twatch = Some(Twatch::default());
        assert!(untouched(&config), "its defaults aren't written down");
        config.twatch = Some(Twatch { volume: 40, ..Twatch::default() });
        assert_eq!(to_toml(&config).unwrap(), format!("{HEADER}\n[twatch]\nvolume = 40\n"));
        // A file holding every value, as 0.1.5's first builds wrote, is the same as one holding the change alone.
        let full = format!("{HEADER}\n[twatch]\ncheck_seconds = 600\ncheck_extra_seconds = 300\nhigher_favorite = \"switch\"\nmetered_pictures = true\nmore_sets = 0\nnotify_live = true\npictures = \"low_memory\"\ntop_quality = \"480p\"\nvolume = 40\n");
        assert_eq!(to_toml(&parse(&full).unwrap()).unwrap(), format!("{HEADER}\n[twatch]\nvolume = 40\n"));
    }

    #[test]
    fn the_network_settings_live_in_a_file_of_their_own() {
        let dir = std::env::temp_dir().join(format!("pms-network-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("pms.toml");
        // From before they had their own file: reading them moves them there, and saving leaves them out of the settings.
        std::fs::write(&path, format!("{HEADER}\n[network]\nproxy = \"socks5h://127.0.0.1:9050\"\nstreams = true\n")).unwrap();
        let config = load(&path).unwrap();
        assert_eq!(config.network.as_ref().map(|n| (n.proxy.as_str(), n.streams, n.updates)), Some(("socks5h://127.0.0.1:9050", true, false)));
        save(&path, &config).unwrap();
        let settings = std::fs::read_to_string(&path).unwrap();
        assert!(!settings.contains("network") && !settings.contains("socks5h"), "nothing of it in the settings: {settings}");
        let own = std::fs::read_to_string(network_path(&path)).unwrap();
        assert!(own.contains("proxy = \"socks5h://127.0.0.1:9050\"") && own.contains("streams = true") && !own.contains("updates"), "only what's yours: {own}");
        assert_eq!(load(&path).unwrap().network, config.network, "and it comes back from there");
        // A file that won't load: everything goes nowhere, and saving doesn't write over it.
        std::fs::write(network_path(&path), "proxy = [nonsense").unwrap();
        let broken = load(&path).unwrap();
        assert_eq!(broken.network.as_ref().map(|n| (n.proxy.as_str(), n.updates)), Some((NETWORK_UNREAD, true)));
        save(&path, &broken).unwrap();
        assert_eq!(std::fs::read_to_string(network_path(&path)).unwrap(), "proxy = [nonsense", "left for you to fix");
        // And what's wrong is there to say, with its line, in a notice that opens the file.
        let e = network_trouble(&path).expect("it's trouble");
        let notice = crate::notices::network_settings(&network_path(&path), &e);
        assert!(notice.detail.contains("Line 1") && notice.fix.is_some_and(|(_, f)| f.ends_with("pms-network.toml")), "{}", notice.detail);
        std::fs::write(network_path(&path), "proxy = \"socks5h://127.0.0.1:9050\"
prxy = 1
").unwrap();
        assert!(network_trouble(&path).is_some(), "a name that isn't a setting is trouble here: it could be the proxy's, misspelled");
        std::fs::write(network_path(&path), "proxy = \"socks5h://127.0.0.1:9050\"
").unwrap();
        assert!(network_trouble(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_isnt_a_setting_is_left_out_and_the_rest_loads() {
        let c = parse(&format!("{HEADER}theme = \"dark\"\nthme = \"light\"\n\n[toasts]\nduration_ms = 3000\nduraton_ms = 9\n")).unwrap();
        assert_eq!(c.theme, "dark");
        assert_eq!(c.toasts.as_ref().unwrap().duration_ms, 3000);
        assert_eq!(c.issues.len(), 2, "{:?}", c.issues);
        assert!(c.issues.iter().any(|e| e.contains("`thme`")) && c.issues.iter().any(|e| e.contains("`duraton_ms`")));
        // Each is the problem it would have been, with its line.
        let text = format!("{HEADER}theme = \"dark\"\nthme = \"light\"\n");
        let c = parse(&text).unwrap();
        let p = crate::problem::Problem::read(&text, &c.issues[0]);
        assert_eq!(p.line, Some(text.lines().position(|l| l.starts_with("thme")).unwrap() + 1));
        // A meter's own, too.
        let c = parse(&format!("{HEADER}[[meter]]\nlabel = \"CPU\"\ncolour = \"red\"\n")).unwrap();
        assert_eq!(c.issues.len(), 1, "{:?}", c.issues);
        // A value that can't be used is left out too, and its setting goes back to its default.
        let c = parse(&format!("{HEADER}theme = 5\ninterval_ms = 500\n")).unwrap();
        assert_eq!((c.theme.as_str(), c.interval_ms, c.issues.len()), ("auto", 500, 1));
        assert_eq!(issue_setting(&c.issues[0]), "theme");
        let c = parse(&format!("{HEADER}[toasts]\nduration_ms = \"long\"\nstyle = \"pimped\"\n")).unwrap();
        assert_eq!(c.toasts.as_ref().map(|t| (t.duration_ms, t.style.as_str())), Some((Toasts::default().duration_ms, "pimped")));
        assert_eq!(issue_setting(&c.issues[0]), "toasts.duration_ms");
        // But a file that isn't TOML is still no settings at all.
        assert!(parse(&format!("{HEADER}theme = \n")).is_err());
    }

    #[test]
    fn a_value_that_cant_be_used_goes_back_to_its_default_and_says_where() {
        // Of the right kind, but not one of the choices, out of range, or not a color: each on its own, with the rest kept.
        let text = format!("{HEADER}tv_intro_from = 99\ntheme = \"purple\"\nmeter_height = \"huge\"\nshuffle_seconds = 1\nshuffle_jitter = false\n\n[twatch]\ntop_quality = \"8k\"\nvolume = 40\n\n[toasts]\nposition = \"moon\"\nstyle = \"pimped\"\n\n[hotmic]\ncolor = \"notacolor\"\nposition = \"middle\"\n");
        let c = parse(&text).unwrap();
        let mut settings: Vec<String> = c.issues.iter().map(|e| issue_setting(e)).collect();
        settings.sort();
        assert_eq!(settings, ["hotmic.color", "hotmic.position", "meter_height", "shuffle_seconds", "theme", "toasts.position", "tv_intro_from", "twatch.top_quality"]);
        assert_eq!((c.tv_intro_from, c.theme.as_str(), c.meter_height.as_str(), c.shuffle_seconds), (crate::tv::INTRO_FROM, "auto", "tall", crate::tv::USUAL_SHUFFLE));
        assert!(!c.shuffle_jitter, "what's fine stays");
        assert_eq!(c.mic_position(), "tray");
        let start = parse(&format!("{HEADER}[hotmic]\nposition = \"start\"\n")).unwrap();
        assert_eq!((start.mic_position(), start.issues.len()), ("start", 0));
        assert_eq!(c.twatch.as_ref().map(|t| (t.top_quality.as_str(), t.volume)), Some(("480p", 40)));
        assert_eq!(c.toasts.as_ref().map(|t| (t.position.as_str(), t.style.as_str())), Some(("clock", "pimped")));
        // Each is found in the file, and said plainly.
        let p = crate::problem::Problem::read(&text, c.issues.iter().find(|e| issue_setting(e) == "tv_intro_from").unwrap());
        assert_eq!(p.source.as_deref(), Some("tv_intro_from = 99"));
        assert!(p.what.contains("`99` isn't one of the choices"), "{}", p.what);
        let p = crate::problem::Problem::read(&text, c.issues.iter().find(|e| issue_setting(e) == "shuffle_seconds").unwrap());
        assert_eq!(p.source.as_deref(), Some("shuffle_seconds = 1"));
        // A meter's style that's gone: that meter keeps the style it comes with, and the others theirs.
        let c = parse(&format!("{HEADER}[[meter]]\nlabel = \"CPU\"\nstyle = \"sparkles\"\n")).unwrap();
        let usual = parse(HEADER).unwrap();
        assert_eq!(c.meters.iter().map(|m| m.style).collect::<Vec<_>>(), usual.meters.iter().map(|m| m.style).collect::<Vec<_>>());
        assert_eq!(c.issues.iter().map(|e| issue_setting(e)).collect::<Vec<_>>(), ["meter.style"]);
        // And what ships has nothing wrong with it.
        assert!(usual.issues.is_empty(), "{:?}", usual.issues);
        // Each issue's setting is on a page of the settings, and the notice opens the first one's.
        assert_eq!(["tv_intro_from", "toasts.position", "meter.style", "theme", "twatch.volume", "backdrop.tint"].map(settings_page), ["tv", "toasts", "meters", "general", "twatch", "look"]);
        assert_eq!(settings_page("console_autostart"), "poom");
        let dir = std::env::temp_dir().join(format!("pms-issues-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("pms.toml");
        std::fs::write(&path, format!("{HEADER}tv_intro_from = 99
")).unwrap();
        let notice = crate::notices::settings_issues(&path, &load(&path).unwrap().issues);
        assert_eq!(notice.fix.map(|(_, to)| to).as_deref(), Some("pms:settings:tv"));
        assert!(notice.detail.contains("tv_intro_from = 99"), "{}", notice.detail);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_videos_old_switch_is_read_and_forgotten() {
        let c = parse(&format!("{HEADER}[network]\nstreams = true\nvideo = false\n")).unwrap();
        assert!(c.issues.is_empty(), "it was a setting, so it's no problem: {:?}", c.issues);
        assert_eq!(c.network.as_ref().map(|n| (n.streams, n.video)), Some((true, None)));
    }

    #[test]
    fn reset_keeps_yours() {
        let dir = std::env::temp_dir().join(format!("pms-config-reset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pms.toml");
        let mine = format!("{HEADER}theme = \"dark\"\nupdate_check = false\nshuffle_seconds = 7\n");
        std::fs::write(&path, &mine).unwrap();
        save_watchlist(&path, &["somebody".to_string()]).unwrap();
        reset(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{HEADER}\nupdate_check = false\n"));
        assert_eq!(std::fs::read_to_string(path.with_extension("toml.before-reset")).unwrap(), mine, "what was there is kept");
        assert_eq!(load(&path).unwrap().twatch.map(|t| t.favorites), Some(vec!["somebody".to_string()]), "the favorites stay");
        // Settings that don't load are reset too.
        std::fs::write(&path, "theme = [").unwrap();
        reset(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), HEADER);
        std::fs::remove_dir_all(&dir).unwrap();
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
