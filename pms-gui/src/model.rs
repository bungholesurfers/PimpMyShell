//! What the settings window edits and does, whichever UI shows it (egui in main.rs, WinUI 3 in winui.rs): the
//! config as edited, saving it once edits settle (only a config the daemon would take), following what the daemon
//! writes into the file meanwhile, the daemon's state, and software update's checks.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pms::config::{self, Agg, Config, CounterSpec, Exprs, Max, MeterSpec};
use pms::update::Status;

/// How long edits settle before they're saved.
pub const SAVE_AFTER: Duration = Duration::from_millis(500);

/// A counter as an editable row (the config keeps them in a map keyed by name).
pub struct CounterRow {
    pub name: String,
    pub path: String,
    pub agg: Agg,
    pub default: Option<f64>,
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

    /// The config file, selected in Explorer.
    pub fn show_config(&self) {
        let _ = std::process::Command::new("explorer.exe").arg("/select,").arg(&self.path).spawn();
    }

    /// Installs a downloaded version (the installer runs silently, then PimpMyShell starts again if it was running):
    /// this window closes for it.
    pub fn install(&mut self, path: &std::path::Path, version: &str, sha256: &str) {
        match pms::update::install(path, version, sha256) {
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
        std::fs::write(&path, config::EXAMPLE).unwrap();
        Model::open(path)
    }

    fn change_outside(m: &Model) -> String {
        let text = config::EXAMPLE.replace("interval_ms = 1000", "interval_ms = 1234");
        assert_ne!(text, config::EXAMPLE);
        std::fs::write(&m.path, &text).unwrap();
        text
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
