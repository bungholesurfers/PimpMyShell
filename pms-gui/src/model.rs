//! What the settings window edits and does, whichever UI shows it (egui in main.rs, WinUI 3 in winui.rs): the
//! config as edited, saving it once edits settle (only a config the daemon would take), following what the daemon
//! writes into the file meanwhile, the daemon's state, and software update's checks.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pms::config::{self, Agg, Config, CounterSpec, Exprs, Max, MeterSpec, Toasts};
use pms::update::Status;

use crate::copy;

/// How long edits settle before they're saved.
pub const SAVE_AFTER: Duration = Duration::from_millis(500);

/// A counter as an editable row (the config keeps them in a map keyed by name).
pub struct CounterRow {
    pub name: String,
    pub path: String,
    pub agg: Agg,
    pub default: Option<f64>,
}

/// A part of the settings that can go back to its defaults in one go.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Section {
    Taskbar,
    Performance,
    Updates,
    Meters,
    Counters,
    Backdrop,
    StartGlow,
    AppGlow,
    Hotmic,
    Toasts,
}

impl Section {
    pub fn name(self) -> &'static str {
        match self {
            Section::Taskbar => "Taskbar",
            Section::Performance => "Performance",
            Section::Updates => "Software update",
            Section::Meters => "Meters",
            Section::Counters => "Performance counters",
            Section::Backdrop => "Background",
            Section::StartGlow => "Start button",
            Section::AppGlow => "App buttons",
            Section::Hotmic => "Microphone alert",
            Section::Toasts => "Notifications",
        }
    }
}

/// The updater's state, written by its thread.
#[derive(Clone, Default)]
pub enum Update {
    #[default]
    Idle,
    Checking,
    Downloading(u64, u64),
    Done(Status),
    Failed(String),
}

pub struct Model {
    pub path: PathBuf,
    pub config: Config,
    /// What ships (pms.example.toml): what a setting's Reset goes back to, and what counts as "changed".
    pub baseline: Config,
    pub counters: Vec<CounterRow>,
    /// What's on disk, to notice edits; and when the last edit happened.
    pub saved: String,
    pub edited_at: Option<Instant>,
    /// Last save outcome, or why the config can't be saved yet.
    pub message: (String, bool),
    pub daemon_running: bool,
    pub autostart: String,
    pub last_log: String,
    pub polled: Instant,
    pub update: Arc<Mutex<Update>>,
    /// The file as we last read or wrote it. Others write it too (you in an editor, the daemon with the answer to its
    /// question or a notice's *Keep Windows notifications*): `follow_disk` notices.
    disk: Option<String>,
}

impl Model {
    pub fn open(path: PathBuf) -> Model {
        // A file from before 0.1.4: only what its owner changed, from now on. A daemon does this as it starts, so one
        // that's running with such a file is older than this window, and can't read the new form: it's left as it is.
        if !pms::daemon::running() {
            let _ = config::adopt_layers(&path);
        }
        let path_read = path.clone();
        let (config, message) = match config::load(&path) {
            Ok(c) => (c, ("Changes save automatically".to_string(), false)),
            Err(_) if !path.exists() => (example(), ("You don't have a settings file yet. Your first change creates it".to_string(), false)),
            Err(e) => (example(), (format!("We couldn't read your settings file, so you're seeing the defaults. Nothing is saved until you change something. ({e})"), true)),
        };
        let counters = config.counters.iter().map(|(name, spec)| counter_row(name, spec)).collect();
        let mut gui = Model {
            path,
            config,
            baseline: example(),
            counters,
            saved: String::new(),
            edited_at: None,
            message,
            daemon_running: false,
            autostart: String::new(),
            last_log: String::new(),
            polled: Instant::now() - Duration::from_secs(10),
            update: Arc::default(),
            disk: std::fs::read_to_string(&path_read).ok(),
        };
        // Compare against the config as the editor states it, so opening isn't an edit.
        gui.saved = config::to_toml(&gui.current()).unwrap_or_default();
        gui.poll_daemon();
        gui
    }

    pub fn poll_daemon(&mut self) {
        self.daemon_running = pms::daemon::running();
        self.autostart = pms::daemon::autostart(&["status".into()]).unwrap_or_default();
        let log = pms_tap::logfile::Log::Daemon.path();
        self.last_log = std::fs::read_to_string(log).ok().and_then(|t| t.lines().last().map(str::to_string)).unwrap_or_default();
        self.polled = Instant::now();
        self.follow_disk();
    }

    /// The config as edited, with the counter rows folded back in.
    pub fn current(&self) -> Config {
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
    pub fn autosave(&mut self) {
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
            self.message = (format!("Your change wasn't saved: {e}"), true);
            self.saved = text; // don't retry until the next edit
            return;
        }
        if !self.follow_disk() {
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
                self.disk = std::fs::read_to_string(&self.path).ok();
                let saved = if self.daemon_running { "Saved" } else { "Saved. Start PimpMyShell to see your changes" };
                self.message = (saved.to_string(), false);
            }
            Err(e) => self.message = (format!("We couldn't save your settings: {e}"), true),
        }
    }

    /// Saves now, without waiting for edits to settle.
    pub fn save_now(&mut self) {
        self.edited_at = Some(Instant::now() - SAVE_AFTER);
        self.autosave();
    }

    /// Starts a check (and, for the installed copy, the download) on a thread of its own.
    pub fn start_check(&mut self, ask: pms::update::Ask) {
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

    /// Whether a check or a download is under way.
    pub fn updating(&self) -> bool {
        matches!(*self.update.lock().unwrap(), Update::Checking | Update::Downloading(..))
    }

    /// Starts the daemon (pmsd.exe beside this exe) with this config.
    pub fn start_daemon(&mut self) {
        let exe = std::env::current_exe().map(|p| p.with_file_name("pmsd.exe"));
        self.message = match exe.and_then(|exe| std::process::Command::new(exe).arg(&self.path).spawn()) {
            Ok(_) => ("Starting PimpMyShell…".into(), false),
            Err(e) => (format!("We couldn't start PimpMyShell: {e}"), true),
        };
    }

    /// Asks the daemon to stop (it puts the stock taskbar back).
    pub fn stop_daemon(&mut self) {
        self.message = match pms::daemon::stop() {
            Ok(_) => ("Stopping PimpMyShell…".into(), false),
            Err(e) => (e, true),
        };
    }

    /// Start at sign-in, on or off.
    pub fn set_autostart(&mut self, on: bool) {
        let args = if on { vec!["on".to_string(), self.path.display().to_string()] } else { vec!["off".to_string()] };
        self.message = match pms::daemon::autostart(&args) {
            Ok(_) if on => ("PimpMyShell will start when you sign in".into(), false),
            Ok(_) => ("PimpMyShell won't start when you sign in".into(), false),
            Err(e) => (e, true),
        };
        self.poll_daemon();
    }

    pub fn autostarts(&self) -> bool {
        self.autostart.starts_with("at logon")
    }

    /// The classic (egui) window in this one's place, on the same config: once edits are saved.
    pub fn open_classic(&mut self) {
        self.edited_at = Some(std::time::Instant::now() - SAVE_AFTER);
        self.autosave();
        match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("--egui").arg(&self.path).spawn()) {
            Ok(_) => std::process::exit(0),
            Err(e) => self.message = (format!("We couldn't open the classic window: {e}"), true),
        }
    }

    /// The new (WinUI 3) window in this one's place, on the same config: once edits are saved.
    pub fn open_new(&mut self) {
        self.edited_at = Some(std::time::Instant::now() - SAVE_AFTER);
        self.autosave();
        match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("--winui").arg(&self.path).spawn()) {
            Ok(_) => std::process::exit(0),
            Err(e) => self.message = (format!("We couldn't open the new settings window: {e}"), true),
        }
    }

    /// The welcome, again, in a window of its own.
    pub fn open_welcome(&mut self) {
        if let Err(e) = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("--welcome").arg(&self.path).spawn()) {
            self.message = (format!("We couldn't open the welcome window: {e}"), true);
        }
    }

    /// The config file, selected in Explorer.
    pub fn show_config(&self) {
        let _ = std::process::Command::new("explorer.exe").arg("/select,").arg(&self.path).spawn();
    }

    /// Installs a downloaded version (the installer runs silently, then PimpMyShell starts again if it was running):
    /// this window closes for it, and the installer opens it again.
    pub fn install(&mut self, path: &std::path::Path, version: &str, sha256: &str) {
        match pms::update::install(path, version, sha256, true) {
            Ok(()) => std::process::exit(0),
            Err(e) => self.message = (e, true),
        }
    }
}

/// Opens a page in the browser.
pub fn open_page(url: &str) {
    let _ = std::process::Command::new("explorer.exe").arg(url).spawn();
}

pub fn example() -> Config {
    config::parse(config::EXAMPLE).expect("the bundled example parses")
}

pub fn toml_meter(s: &str) -> MeterSpec {
    config::parse(&format!("[[meter]]\n{s}")).ok().and_then(|c| c.meters.into_iter().next()).expect("valid meter template")
}

/// A new meter's table, for + Add meter.
pub const NEW_METER: &str = "label = \"NEW\"\nexpr = \"0\"\nunit = \"%\"\nmax = 100\ncolor = \"#4FC3F7\"";

fn counter_row(name: &str, spec: &CounterSpec) -> CounterRow {
    match spec {
        CounterSpec::Path(p) => CounterRow { name: name.to_string(), path: p.clone(), agg: Agg::Sum, default: None },
        CounterSpec::Full { path, agg, default } => CounterRow { name: name.to_string(), path: path.clone(), agg: *agg, default: *default },
    }
}

/// A new counter's row, for + Add counter.
pub fn new_counter() -> CounterRow {
    CounterRow { name: String::new(), path: r"\Processor Information(_Total)\% Processor Utility".into(), agg: Agg::Sum, default: None }
}

impl Model {
    fn baseline_rows(&self) -> Vec<CounterRow> {
        self.baseline.counters.iter().map(|(name, spec)| counter_row(name, spec)).collect()
    }

    /// Puts `section` back as it ships, in `config` and `counters`: all but what's kept (`kept`).
    fn apply_reset(&self, section: Section, c: &mut Config, counters: &mut Vec<CounterRow>) {
        let b = self.baseline.clone();
        match section {
            Section::Taskbar => (c.theme, c.meter_height, c.bar_height, c.interval_ms, c.value_font) = (b.theme, b.meter_height, b.bar_height, b.interval_ms, b.value_font),
            Section::Performance => (c.max_fps, c.battery_fps, c.software_gpus) = (b.max_fps, b.battery_fps, b.software_gpus),
            // Whether to check at all is consent, not a setting: only its owner changes that.
            Section::Updates => c.update_channel = b.update_channel,
            Section::Meters => c.meters = b.meters,
            Section::Counters => *counters = self.baseline_rows(),
            Section::Backdrop => c.backdrop = b.backdrop,
            Section::StartGlow => c.start_glow = b.start_glow,
            Section::AppGlow => c.app_glow = b.app_glow,
            Section::Hotmic => {
                let ignore = c.hotmic.take().map(|h| h.ignore);
                c.hotmic = b.hotmic;
                if let (Some(h), Some(ignore)) = (c.hotmic.as_mut(), ignore) {
                    h.ignore = ignore;
                }
            }
            // Off stays off (it decides whether notifications get read at all), and so do the choices that were made once
            // and for all: *Shut up, computer*, and the apps kept out.
            Section::Toasts => {
                if let (Some(mine), Some(shipped)) = (c.toasts.as_mut(), b.toasts) {
                    *mine = Toasts { enabled: mine.enabled, confirm_clicks: std::mem::take(&mut mine.confirm_clicks), ignore: std::mem::take(&mut mine.ignore), ..shipped };
                }
            }
        }
    }

    /// Whether a reset of `section` would change nothing.
    pub fn at_default(&self, section: Section) -> bool {
        let key = |r: &CounterRow| (r.name.clone(), r.path.clone(), format!("{:?}", r.agg), r.default.map(f64::to_bits));
        let sorted = |rows: &[CounterRow]| {
            let mut all: Vec<_> = rows.iter().map(key).collect();
            all.sort();
            all
        };
        let (mut config, mut counters) = (self.config.clone(), self.counters.iter().map(|r| CounterRow { name: r.name.clone(), path: r.path.clone(), agg: r.agg, default: r.default }).collect());
        self.apply_reset(section, &mut config, &mut counters);
        config::to_toml(&config).ok() == config::to_toml(&self.config).ok() && sorted(&counters) == sorted(&self.counters)
    }

    /// What a reset of `section` leaves as you set it: the ones that aren't at their defaults, in the window's words.
    pub fn kept(&self, section: Section) -> Vec<&'static str> {
        let c = &self.config;
        let mut kept = Vec::new();
        match section {
            Section::Updates if c.update_check.is_some() => kept.push(copy::setting("update_check").0),
            Section::Hotmic if c.hotmic.as_ref().is_some_and(|h| !h.ignore.is_empty()) => kept.push(copy::setting("hotmic.ignore").0),
            Section::Toasts => match &c.toasts {
                Some(t) => {
                    let d = self.baseline.toasts.clone().unwrap_or_default();
                    if !t.enabled {
                        kept.push("Notifications in the taskbar being off");
                    }
                    if t.confirm_clicks != d.confirm_clicks {
                        kept.push(copy::setting("toasts.confirm_clicks").0);
                    }
                    if !t.ignore.is_empty() {
                        kept.push(copy::setting("toasts.ignore").0);
                    }
                }
                None => kept.push("Notifications in the taskbar being off"),
            },
            _ => {}
        }
        kept
    }

    /// What the confirmation says: what goes, and what stays.
    pub fn warning(&self, section: Section) -> String {
        let all = format!("Everything else in {} goes back to its default.", section.name());
        let all = match section {
            Section::Meters => format!("{all} Meters you added are removed, and meters you removed come back."),
            Section::Counters => format!("{all} Counters you added are removed, and meters that read them may stop working."),
            _ => all,
        };
        match self.kept(section).as_slice() {
            [] => all.replace("Everything else", "Everything"),
            kept => format!("This won't change: {}.\n\n{all}", kept.join(", ")),
        }
    }

    /// Puts `section` back as it ships, saves that, and says what it left alone.
    pub fn reset(&mut self, section: Section) {
        let kept = self.kept(section);
        let (mut config, mut counters) = (self.config.clone(), std::mem::take(&mut self.counters));
        self.apply_reset(section, &mut config, &mut counters);
        (self.config, self.counters) = (config, counters);
        self.save_now();
        if !self.message.1 {
            let done = format!("{} reset", section.name());
            self.message = (if kept.is_empty() { done } else { format!("{done}. Not changed: {}", kept.join(", ")) }, false);
        }
    }

    /// Takes in what changed in the file since we last read or wrote it, so that a save here never undoes it: the file
    /// wins over a change here that hasn't been saved yet. Returns whether the file is ours to write now. It isn't
    /// after taking something in, nor while what's there doesn't load: that's someone's work, not to be saved over.
    fn follow_disk(&mut self) -> bool {
        let Ok(text) = std::fs::read_to_string(&self.path) else { return true };
        if self.disk.as_ref() == Some(&text) {
            return true;
        }
        let config = match config::parse(&text) {
            Ok(c) => c,
            Err(e) => {
                self.message = (format!("Your settings file has an error, so changes here aren't being saved. Fix the file to continue. ({e})"), true);
                return false;
            }
        };
        self.disk = Some(text);
        // Only comments or spacing changed: nothing to take in.
        if config::to_toml(&config).is_ok_and(|t| t == self.saved) {
            return true;
        }
        let unsaved = config::to_toml(&self.current()).is_ok_and(|t| t != self.saved);
        self.counters = config.counters.iter().map(|(name, spec)| counter_row(name, spec)).collect();
        self.config = config;
        self.saved = config::to_toml(&self.current()).unwrap_or_default();
        self.edited_at = None;
        self.message = if unsaved {
            ("Your settings file changed outside this window, so we loaded it. Make your last change again".to_string(), true)
        } else {
            ("Your settings file changed, so we loaded it".to_string(), false)
        };
        false
    }

    /// The shipped meter this one is (by its name), for what its settings go back to; None for one of your own.
    pub fn baseline_meter(&self, label: &str) -> Option<&MeterSpec> {
        self.baseline.meters.iter().find(|b| b.label.eq_ignore_ascii_case(label))
    }

    /// The shipped meters you don't have (going by their names), for Add back.
    pub fn missing_meters(&self) -> Vec<&MeterSpec> {
        let have = |label: &str| self.config.meters.iter().any(|m| m.label.eq_ignore_ascii_case(label));
        self.baseline.meters.iter().filter(|b| !have(&b.label)).collect()
    }

    /// Puts shipped meter `label` back, after the shipped meter before it that you still have (with the counters it
    /// reads, should they be gone too). Returns where it went.
    pub fn restore_meter(&mut self, label: &str) -> Option<usize> {
        let shipped = self.baseline.meters.iter().position(|b| b.label.eq_ignore_ascii_case(label))?;
        let meter = self.baseline.meters[shipped].clone();
        let at = self.baseline.meters[..shipped]
            .iter()
            .rev()
            .find_map(|b| self.config.meters.iter().position(|m| m.label.eq_ignore_ascii_case(&b.label)))
            .map_or(0, |i| i + 1);
        let max = |m: &Option<Max>| match m {
            Some(Max::Expr(e)) => e.clone(),
            _ => String::new(),
        };
        let mut reads = meter.expr.as_ref().map(Exprs::list).unwrap_or_default();
        reads.extend([max(&meter.max), max(&meter.show_above)]);
        reads.extend(meter.show_when.iter().map(|w| w.expr.clone()));
        let names: Vec<&str> = reads.iter().flat_map(|e| e.split(|c: char| !c.is_alphanumeric() && c != '_')).collect();
        for (name, spec) in &self.baseline.counters {
            if names.contains(&name.as_str()) && !self.counters.iter().any(|r| r.name == *name) {
                self.counters.push(counter_row(name, spec));
            }
        }
        self.config.meters.insert(at, meter);
        Some(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A model with only the shipped config's first and last meters, and no counters.
    fn stripped() -> Model {
        let path = std::env::temp_dir().join("pms-gui-test-that-does-not-exist.toml");
        let mut m = Model::open(path);
        let last = m.config.meters.len() - 1;
        m.config.meters = vec![m.config.meters[0].clone(), m.config.meters[last].clone()];
        m.counters.clear();
        m
    }

    #[test]
    fn a_shipped_meter_goes_back_where_it_was() {
        let mut m = stripped();
        let shipped: Vec<String> = m.baseline.meters.iter().map(|b| b.label.clone()).collect();
        assert_eq!(m.missing_meters().len(), shipped.len() - 2);
        for label in shipped[1..shipped.len() - 1].iter().rev() {
            m.restore_meter(label).unwrap();
        }
        assert!(m.missing_meters().is_empty());
        assert_eq!(m.config.meters.iter().map(|b| b.label.clone()).collect::<Vec<_>>(), shipped);
    }

    #[test]
    fn a_meter_comes_back_with_its_counters() {
        let mut m = stripped();
        m.restore_meter("SWAP").unwrap();
        assert!(m.counters.iter().any(|c| c.name == "mem_avail"), "what its show_when reads");
        assert!(m.restore_meter("nothing we ship").is_none());
    }

    /// A model of a file of its own holding the shipped config.
    fn on_file(name: &str) -> Model {
        let path = std::env::temp_dir().join(format!("pms-gui-test-{name}-{}.toml", std::process::id()));
        std::fs::write(&path, "config_version = 2\n").unwrap();
        Model::open(path)
    }

    fn change_outside(m: &Model) -> String {
        let text = config::EXAMPLE.replace("interval_ms = 1000", "interval_ms = 1234");
        assert_ne!(text, config::EXAMPLE);
        let text = format!("config_version = 2\n{text}");
        std::fs::write(&m.path, &text).unwrap();
        text
    }

    #[test]
    fn every_section_goes_back_to_its_defaults() {
        let sections = [
            Section::Taskbar,
            Section::Performance,
            Section::Updates,
            Section::Meters,
            Section::Counters,
            Section::Backdrop,
            Section::StartGlow,
            Section::AppGlow,
            Section::Hotmic,
            Section::Toasts,
        ];
        let mut m = on_file("sections");
        assert!(sections.iter().all(|s| m.at_default(*s)), "a new config is all defaults");
        let c = &mut m.config;
        c.theme = "dark".into();
        c.software_gpus = true;
        c.update_channel = "latest".into();
        c.meters.remove(1);
        c.backdrop = None;
        c.start_glow = None;
        c.app_glow = None;
        c.hotmic = None;
        c.toasts.as_mut().unwrap().style = "pimped".into();
        m.counters.pop();
        for s in sections {
            assert!(!m.at_default(s), "{s:?} changed");
            m.reset(s);
            assert!(m.at_default(s), "{s:?} reset");
        }
        assert_eq!(config::to_toml(&m.current()), config::to_toml(&m.baseline), "back to writing nothing");
        std::fs::remove_file(&m.path).unwrap();
    }

    #[test]
    fn a_reset_leaves_your_choices_alone_and_says_so() {
        let mut m = on_file("kept");
        m.config.update_check = Some(false);
        m.config.update_channel = "latest".into();
        assert!(m.warning(Section::Updates).contains("won't change"), "{}", m.warning(Section::Updates));
        m.reset(Section::Updates);
        assert_eq!(m.config.update_check, Some(false), "consent stays");
        assert_eq!(m.config.update_channel, m.baseline.update_channel);
        assert!(m.message.0.contains("Not changed"), "{}", m.message.0);

        let t = m.config.toasts.as_mut().unwrap();
        t.style = "pimped".into();
        t.confirm_clicks = "off".into();
        t.ignore = vec!["Teams".into()];
        t.enabled = false;
        assert_eq!(m.kept(Section::Toasts).len(), 3);
        m.reset(Section::Toasts);
        let t = m.config.toasts.as_ref().unwrap();
        assert_eq!(t.style, "standard");
        assert!(t.confirm_clicks == "off" && t.ignore == ["Teams"] && !t.enabled);
        assert!(m.at_default(Section::Toasts), "what's kept doesn't leave Reset lit");

        m.config.toasts = None;
        m.reset(Section::Toasts);
        assert!(m.config.toasts.is_none(), "off stays off");
        std::fs::remove_file(&m.path).unwrap();
    }

    #[test]
    fn a_reset_with_nothing_kept_says_nothing_of_it() {
        let m = on_file("nothing-kept");
        assert!(!m.warning(Section::Taskbar).contains("won't change"));
        assert!(m.warning(Section::Taskbar).starts_with("Everything in"));
        std::fs::remove_file(&m.path).unwrap();
    }

    #[test]
    fn a_change_outside_comes_in() {
        let mut m = on_file("outside");
        change_outside(&m);
        assert!(!m.follow_disk());
        assert_eq!(m.config.interval_ms, 1234);
        assert!(m.follow_disk(), "taken in, it's ours to write again");
        std::fs::remove_file(&m.path).unwrap();
    }

    #[test]
    fn a_save_never_undoes_a_change_outside() {
        let mut m = on_file("pending");
        m.config.interval_ms = 2000;
        let text = change_outside(&m);
        m.edited_at = Some(Instant::now() - SAVE_AFTER);
        m.autosave();
        assert_eq!(std::fs::read_to_string(&m.path).unwrap(), text, "the file is as it was changed");
        assert_eq!(m.config.interval_ms, 1234);
        assert!(m.message.1, "and it says the change here was dropped");
        std::fs::remove_file(&m.path).unwrap();
    }

    #[test]
    fn a_file_that_does_not_load_is_left_alone() {
        let mut m = on_file("broken");
        m.config.interval_ms = 2000;
        std::fs::write(&m.path, "interval_ms = [half typed").unwrap();
        m.edited_at = Some(Instant::now() - SAVE_AFTER);
        m.autosave();
        assert_eq!(std::fs::read_to_string(&m.path).unwrap(), "interval_ms = [half typed");
        assert_eq!(m.config.interval_ms, 2000, "what you changed here stays, for when the file is fixed");
        std::fs::remove_file(&m.path).unwrap();
    }
}
