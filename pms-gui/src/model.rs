//! What the settings window edits and does, whichever UI shows it (egui in main.rs, WinUI 3 in winui.rs): the
//! config as edited, saving it once edits settle (only a config the daemon would take), following what the daemon
//! writes into the file meanwhile, the daemon's state, and software update's checks.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pms::config::{self, Agg, Config, CounterSpec, Exprs, Max, MeterSpec, Toasts};
use pms::update::Status;

use crate::copy;
use pms::lang::settings as s;

/// How long edits settle before they're saved.
pub const SAVE_AFTER: Duration = Duration::from_millis(500);
/// And how long once a favorite's been moved. Taking one a few places up or down is a run of steps, and every save is a new
/// order for the sets to go by: one save for the run, not one for each step.
pub const ORDER_SETTLES: Duration = Duration::from_secs(3);
/// What's said while a new order waits to be saved.
const ORDER_WAITS: s::Id = s::MODEL_ORDER_WAITS;
/// What marks the row of the settings window that's doing the listing.
const HERE: s::Id = s::MODEL_HERE;
/// What the settings window is called, which is how another start of this program finds it.
pub const WINDOW_TITLE: &str = "PimpMyShell Settings";

/// Where a page asked for from outside waits for the window that's open.
fn page_file() -> PathBuf {
    pms_tap::data_dir().join("settings-page.txt")
}

/// Sends the settings window that's open, if there is one, to `page`, and brings it to the front: false if there's none.
pub fn go_to_page(page: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
    if unsafe { FindWindowW(windows_core::PCWSTR::null(), &windows_core::HSTRING::from(WINDOW_TITLE)) }.is_err() {
        return false;
    }
    let _ = std::fs::write(page_file(), page);
    bring_forward()
}

/// Whether this is the only settings window of the user's own settings: true, and it stays the only one for as long as this
/// process runs. False, if there's one already, which is brought to the front instead: once it has its window, should it be
/// starting too (a double-click on the shortcut starts two). A window for another settings file (a scratch copy) isn't one.
/// `taking_over`: started by the window that's open, to be the other kind of window in its place (`open_classic`,
/// `open_new`), so it waits a few seconds for that one to go.
pub fn only_window(taking_over: bool) -> bool {
    use windows::Win32::Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
    let Ok(held) = (unsafe { CreateMutexW(None, false, windows_core::w!("Local\\PimpMyShell.Settings")) }) else { return true };
    // Held, and left open, for as long as this process runs; Windows lets go of it when it ends, however it ends.
    let got = unsafe { WaitForSingleObject(held, if taking_over { 5_000 } else { 0 }) };
    if got == WAIT_OBJECT_0 || got == WAIT_ABANDONED {
        return true;
    }
    let started = Instant::now();
    while !bring_forward() && started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Brings the settings window that's open to the front: false if there's none.
fn bring_forward() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow};
    let Ok(window) = (unsafe { FindWindowW(windows_core::PCWSTR::null(), &windows_core::HSTRING::from(WINDOW_TITLE)) }) else { return false };
    unsafe {
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        let _ = SetForegroundWindow(window);
    }
    true
}

/// The page asked for since this was last asked, if any.
pub fn page_asked() -> Option<String> {
    let file = page_file();
    let page = std::fs::read_to_string(&file).ok()?;
    let _ = std::fs::remove_file(&file);
    Some(page.trim().to_string())
}

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
    Twatch,
    Hotmic,
    Toasts,
}

impl Section {
    pub fn name(self) -> &'static str {
        match self {
            Section::Taskbar => s::SECTION_TASKBAR.t(),
            Section::Performance => s::SECTION_PERFORMANCE.t(),
            Section::Updates => s::SECTION_UPDATES.t(),
            Section::Meters => s::SECTION_METERS.t(),
            Section::Counters => s::SECTION_COUNTERS.t(),
            Section::Backdrop => s::SECTION_BACKDROP.t(),
            Section::StartGlow => s::SECTION_START_GLOW.t(),
            Section::AppGlow => s::SECTION_APP_GLOW.t(),
            Section::Twatch => s::SECTION_TWATCH.t(),
            Section::Hotmic => s::SECTION_HOTMIC.t(),
            Section::Toasts => s::SECTION_TOASTS.t(),
        }
    }
}

/// Fetching a WAD, written by its thread.
#[derive(Clone, Default)]
pub enum WadJob {
    #[default]
    Idle,
    Getting(u64, u64),
    Failed(String),
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

/// A process, as the settings list it.
#[derive(Clone, Debug, PartialEq)]
pub struct Running {
    pub name: String,
    /// How deep under the one that started it.
    pub depth: usize,
    /// Its share of all the processors since the look before: none yet, the first time it's seen.
    pub cpu: Option<f32>,
    /// Its own memory, in bytes.
    pub memory: u64,
    /// How much of the graphics card it's using, which of its engines, and how many bytes of the card's own memory: none
    /// where Windows doesn't say.
    pub gpu: Option<pms::procs::GpuUse>,
    /// What it is, where its name doesn't say enough.
    pub about: &'static str,
    /// The sandbox it's in, as its own token says: None where it can't be asked.
    pub sandbox: Option<pms::sandbox::Sandbox>,
}

pub struct Model {
    pub path: PathBuf,
    pub config: Config,
    /// What came of asking for the newest Streamlink, if there's anything to say.
    pub streamlink_note: Arc<Mutex<String>>,
    /// The favorites as text, while they're being edited that way. Nothing's a favorite until it's saved: a name half typed could
    /// be somebody else's channel.
    pub favorites_text: Option<String>,
    /// A channel being typed, to add.
    pub new_favorite: String,
    /// What was wrong with the last list or name.
    pub favorites_note: String,
    /// The proxy's address as it's being typed, until it's applied; and what came of trying it.
    pub proxy_text: String,
    pub proxy_note: Arc<Mutex<String>>,
    /// What came of trying each use's way out, by its place in `pms::net::USES`.
    pub use_notes: Arc<Mutex<Vec<String>>>,
    /// Which of those tries didn't work.
    pub use_failed: Arc<Mutex<Vec<bool>>>,
    pub wad_job: Arc<Mutex<WadJob>>,
    /// Fetching Streamlink.
    pub streamlink_job: Arc<Mutex<WadJob>>,
    /// What ships (pms.example.toml): what a setting's Reset goes back to, and what counts as "changed".
    pub baseline: Config,
    pub counters: Vec<CounterRow>,
    /// What's on disk, to notice edits; and when the last edit happened.
    pub saved: String,
    pub edited_at: Option<Instant>,
    /// How long edits settle before the next save: `SAVE_AFTER`, or `ORDER_SETTLES` once a favorite's been moved.
    settle: Duration,
    /// The config as it stood at the last look, while it's waiting to be saved: any change starts the wait again, so that it's
    /// after the edits stop, and not during them, that it's saved.
    typed: Option<String>,
    /// The last config that a `Sampler` took.
    checked: Option<Config>,
    /// Last save outcome, or why the config can't be saved yet.
    pub message: (String, bool),
    /// What's wrong with the file. While there is something, nothing here is saved: it's someone's work.
    pub problem: Option<pms::problem::Problem>,
    pub daemon_running: bool,
    pub autostart: String,
    pub last_log: String,
    pub polled: Instant,
    /// PimpMyShell's processes, as they were at the last look: each one's name, how deep under the one that started it, its
    /// share of all the processors since the look before (none yet, the first time it's seen), and its own memory in bytes.
    pub procs: Vec<Running>,
    /// The same for what isn't ours but does our work: the Explorer process the taskbar's in, which our part in the taskbar
    /// is inside; and the Desktop Window Manager, which draws it.
    pub theirs: Vec<Running>,
    /// How much of the graphics card each process is using: looked at each time the processes are.
    gpus: Option<pms::procs::Gpus>,
    /// The processes are on show (the page they're on is): only then are they counted, which isn't free.
    pub watching_procs: bool,
    /// How much dedicated memory the graphics cards have in use in all, in bytes: what's really on them.
    pub cards_in_use: Option<u64>,
    /// How long each had had a processor at that look, and when that was.
    procs_before: (Vec<(u32, u64)>, Instant),
    pub update: Arc<Mutex<Update>>,
    /// The file as we last read or wrote it. Others write it too (you in an editor, the daemon with the answer to its
    /// question or a notice's *Keep Windows notifications*): `follow_disk` notices.
    disk: Option<String>,
}

const SAVES: s::Id = s::MODEL_SAVES;
const NOT_SAVING: s::Id = s::MODEL_NOT_SAVING;

/// What's on disk now, to tell when that changes: the settings, and the favorites and network settings beside them.
fn on_disk(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let beside = |p: std::path::PathBuf| std::fs::read_to_string(p).unwrap_or_default();
    Some(format!("{text}\0{}\0{}", beside(config::watchlist_path(path)), beside(config::network_path(path))))
}

impl Model {
    pub fn open(path: PathBuf) -> Model {
        // A file from before 0.1.4: only what its owner changed, from now on. A daemon does this as it starts, so one
        // that's running with such a file is older than this window, and can't read the new form: it's left as it is.
        if !pms::daemon::running() {
            let _ = config::adopt_layers(&path);
        }
        let path_read = path.clone();
        let (config, message, problem) = match config::load(&path) {
            Ok(c) => (c, (SAVES, false), None),
            Err(_) if !path.exists() => (example(), (s::MODEL_NO_FILE_YET, false), None),
            Err(e) => (example(), (NOT_SAVING, true), Some(pms::problem::Problem::of(&path, &e))),
        };
        // In the language the settings have, from the first thing said.
        pms::speak(&config.language);
        let message = (message.0.t().to_string(), message.1);
        let counters = config.counters.iter().map(|(name, spec)| counter_row(name, spec)).collect();
        let mut gui = Model {
            path,
            config,
            streamlink_note: Arc::default(),
            favorites_text: None,
            new_favorite: String::new(),
            favorites_note: String::new(),
            proxy_text: String::new(),
            proxy_note: Arc::default(),
            use_notes: Arc::new(Mutex::new(vec![String::new(); pms::net::USES.len()])),
            use_failed: Arc::new(Mutex::new(vec![false; pms::net::USES.len()])),
            typed: None,
            checked: None,
            wad_job: Arc::default(),
            streamlink_job: Arc::default(),
            baseline: example(),
            counters,
            saved: String::new(),
            edited_at: None,
            settle: SAVE_AFTER,
            message,
            problem,
            daemon_running: false,
            autostart: String::new(),
            last_log: String::new(),
            polled: Instant::now() - Duration::from_secs(10),
            procs: Vec::new(),
            theirs: Vec::new(),
            gpus: None,
            watching_procs: true,
            cards_in_use: None,
            procs_before: (Vec::new(), Instant::now()),
            update: Arc::default(),
            disk: on_disk(&path_read),
        };
        gui.proxy_text = shown_proxy(&gui.config);
        // Compare against the config as the editor states it, so opening isn't an edit.
        gui.saved = config::comparable(&gui.current()).unwrap_or_default();
        gui.note_watchlist();
        gui.speak();
        gui.poll_daemon();
        gui
    }

    pub fn poll_daemon(&mut self) {
        self.daemon_running = pms::daemon::running();
        self.autostart = pms::daemon::autostart(&["status".into()]).unwrap_or_default();
        let log = pms_tap::logfile::Log::Daemon.path();
        self.last_log = std::fs::read_to_string(log).ok().and_then(|t| t.lines().last().map(str::to_string)).unwrap_or_default();
        self.polled = Instant::now();
        if !self.watching_procs {
            self.follow_disk();
            return;
        }
        if self.gpus.is_none() {
            self.gpus = pms::procs::Gpus::new();
        }
        let (now, before) = (pms::procs::ours(), &self.procs_before);
        let apart = before.1.elapsed();
        let graphics = self.gpus.as_mut().map(pms::procs::Gpus::look);
        self.cards_in_use = self.gpus.as_ref().and_then(pms::procs::Gpus::in_use);
        let running = |p: &pms::procs::Proc, about: &'static str| Running {
            name: if p.pid == std::process::id() { format!("{} {}", p.name, HERE.t()) } else { p.name.clone() },
            depth: p.depth,
            cpu: before.0.iter().find(|(pid, _)| *pid == p.pid).map(|(_, busy)| pms::procs::share(*busy, p.busy, apart)),
            memory: p.private,
            // One that isn't using a card at all is using none of it: which is known, where the counters are.
            gpu: graphics.as_ref().map(|g| g.get(&p.pid).cloned().unwrap_or_default()),
            about,
            sandbox: pms::sandbox::of_process(p.pid),
        };
        let theirs: Vec<(pms::procs::Proc, &'static str)> = [pms::procs::shell().map(|p| (p, "")), pms::procs::dwm().map(|p| (p, pms::procs::DWM_IS.t()))].into_iter().flatten().collect();
        self.procs = now.iter().map(|p| running(p, "")).collect();
        self.theirs = theirs.iter().map(|(p, about)| running(p, about)).collect();
        self.procs_before = (now.iter().chain(theirs.iter().map(|(p, _)| p)).map(|p| (p.pid, p.busy)).collect(), Instant::now());
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
        if self.problem.is_some() && !self.follow_disk() {
            return;
        }
        let config = self.current();
        let Ok(text) = config::comparable(&config) else { return };
        if text == self.saved {
            (self.edited_at, self.typed, self.settle) = (None, None, SAVE_AFTER);
            if self.message.0 == ORDER_WAITS.t() {
                self.message = (SAVES.t().to_string(), false);
            }
            return;
        }
        if self.typed.as_ref().is_some_and(|typed| *typed != text) {
            self.edited_at = Some(Instant::now());
        }
        self.typed = Some(text.clone());
        let at = *self.edited_at.get_or_insert_with(Instant::now);
        if at.elapsed() < self.settle {
            return;
        }
        (self.edited_at, self.typed, self.settle) = (None, None, SAVE_AFTER);
        // Only what a sampler goes by can be wrong for one, and making one takes a moment.
        if !self.checked.as_ref().is_some_and(|c| pms::meters::same_sampler(c, &config)) {
            if let Err(e) = pms::meters::Sampler::new(&config) {
                self.message = (s::MODEL_NOT_SAVED.with(&[("why", &e)]), true);
                self.saved = text; // don't retry until the next edit
                return;
            }
            self.checked = Some(config.clone());
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
                self.disk = on_disk(&self.path);
                // What wasn't a setting isn't in the file any more.
                self.config.issues.clear();
                let saved = if self.daemon_running { s::MODEL_SAVED.t() } else { s::MODEL_SAVED_START.t() };
                self.message = (saved.to_string(), false);
            }
            Err(e) => self.message = (s::MODEL_COULDNT_SAVE.with(&[("why", &e)]), true),
        }
    }

    /// Saves now, without waiting for edits to settle.
    pub fn save_now(&mut self) {
        (self.edited_at, self.settle) = (Some(Instant::now() - SAVE_AFTER), SAVE_AFTER);
        self.autosave();
    }

    /// Starts a check (and, for the installed copy, the download) on a thread of its own.
    pub fn start_check(&mut self, ask: pms::update::Ask) {
        pms::net::follow(&self.config);
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

    /// Fetches a WAD on a thread of its own: the daemon notices it, and the screen changes over. The shareware's licence
    /// comes first.
    pub fn get_wad(&self, wad: pms::wads::Wad) {
        pms::net::follow(&self.config);
        let shared = self.wad_job.clone();
        *shared.lock().unwrap() = WadJob::Getting(0, 0);
        std::thread::spawn(move || {
            if wad == pms::wads::Wad::Shareware && !pms::notices::accepts_shareware_license() {
                *shared.lock().unwrap() = WadJob::Idle;
                return;
            }
            let progress = |done, total| *shared.lock().unwrap() = WadJob::Getting(done, total);
            *shared.lock().unwrap() = match pms::wads::get(wad, &progress) {
                Ok(_) => WadJob::Idle,
                Err(e) => WadJob::Failed(e),
            };
        });
    }

    pub fn remove_wad(&self, name: &str) {
        if let Err(e) = pms::wads::remove(name) {
            *self.wad_job.lock().unwrap() = WadJob::Failed(e);
        }
    }

    /// Switches the console off and deletes what it's kept compiled: it compiles it again the next time it comes on.
    pub fn clear_shaders(&mut self) {
        let sets: Vec<String> = self.config.meters.iter().filter(|m| m.style == pms::config::Style::Screen && m.set.as_deref() != Some("twatch")).map(|m| m.label.clone()).collect();
        // Nothing of it is held open: it's read when the console comes on, and that's all.
        pms::tv::switch_console_off(&sets);
        // Its own files in its own folder, by name: nothing else that might be there, and no folder with anything in it.
        let kept = pms::wads::shaders();
        self.message = match pms::wads::clear_shaders_in(&kept) {
            Ok(0) => (s::MODEL_NO_SHADERS.t().to_string(), false),
            Ok(_) => (s::MODEL_SHADERS_CLEARED.t().to_string(), false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (s::MODEL_NO_SHADERS.t().to_string(), false),
            Err(e) => (s::MODEL_COULDNT_CLEAR.with(&[("what", &pms::shown(&kept)), ("why", &e)]), true),
        };
    }

    /// Puts `name` in the console: the daemon notices.
    pub fn choose_wad(&self, name: &str) {
        if let Err(e) = pms::wads::choose(name) {
            *self.wad_job.lock().unwrap() = WadJob::Failed(e);
        }
    }

    /// The TVs: the labels of the `screen` meters.
    pub fn tvs(&self) -> Vec<String> {
        self.config.meters.iter().filter(|m| is_tv(m)).map(|m| m.label.clone()).collect()
    }

    /// Whether there's a Twatch TV.
    pub fn twatch_on(&self) -> bool {
        self.config.meters.iter().any(is_twatch)
    }

    /// Puts a Twatch TV on the taskbar, at the end of the strip, or takes it off.
    pub fn set_twatch_on(&mut self, on: bool) {
        self.config.meters.retain(|m| !is_twatch(m));
        if on {
            self.config.meters.push(toml_meter("label = \"Twatch TV\"\nstyle = \"screen\"\nset = \"twatch\""));
            self.twatch();
        }
    }

    /// Its settings.
    pub fn sandboxes(&mut self) -> &mut pms::config::Sandboxes {
        self.config.sandbox.get_or_insert_with(Default::default)
    }

    /// Which processes are in efficiency mode.
    pub fn efficiency(&mut self) -> &mut pms::config::Efficiency {
        self.config.efficiency.get_or_insert_with(Default::default)
    }

    pub fn demos(&mut self) -> &mut pms::config::Demos {
        self.config.demos.get_or_insert_with(Default::default)
    }

    /// Number One's settings, on the TV page.
    pub fn iss(&mut self) -> &mut pms::config::Iss {
        self.config.iss.get_or_insert_with(Default::default)
    }

    pub fn twatch(&mut self) -> &mut pms::config::Twatch {
        self.config.twatch.get_or_insert_with(Default::default)
    }

    /// Starts editing the favorites as text.
    pub fn favorites_as_text(&mut self) {
        let favorites = self.config.twatch.as_ref().map_or(Vec::new(), |t| t.favorites.clone());
        self.favorites_text = Some(favorites.join("\n"));
        self.favorites_note.clear();
    }

    /// Makes the text the favorites, if every line of it can be one. If not, nothing changes, and the text stays to be put right.
    pub fn save_favorites_text(&mut self) {
        let Some(text) = &self.favorites_text else { return };
        let (good, bad) = pms::twatch::live::parse_list(text);
        if !bad.is_empty() {
            self.favorites_note = s::MODEL_BAD_NAMES.with(&[("names", &bad.join(", "))]);
            return;
        }
        self.twatch().favorites = good;
        self.favorites_text = None;
        self.favorites_note.clear();
    }

    pub fn cancel_favorites_text(&mut self) {
        self.favorites_text = None;
        self.favorites_note.clear();
    }

    /// From now on this window says things in the language the settings have.
    pub fn speak(&self) {
        pms::speak(&self.config.language);
    }

    /// Two favorites change places: a step up or down the list.
    pub fn swap_favorites(&mut self, a: usize, b: usize) {
        let favorites = &mut self.twatch().favorites;
        if a != b && a < favorites.len() && b < favorites.len() {
            favorites.swap(a, b);
            self.order_changed();
        }
    }

    /// The favorites in the order they were dragged into: taken only if it's the same channels, each as often.
    pub fn reorder_favorites(&mut self, order: Vec<String>) {
        let favorites = &mut self.twatch().favorites;
        let sorted = |list: &[String]| {
            let mut list = list.to_vec();
            list.sort();
            list
        };
        if *favorites != order && sorted(favorites) == sorted(&order) {
            *favorites = order;
            self.order_changed();
        }
    }

    /// A favorite's been moved: there may be more moves to come, so saving waits longer than it does for anything else.
    fn order_changed(&mut self) {
        self.settle = ORDER_SETTLES;
        if self.problem.is_none() {
            self.message = (ORDER_WAITS.t().to_string(), false);
        }
    }

    /// Adds what's been typed to the end of the favorites, if it can be a channel.
    pub fn add_favorite(&mut self) {
        let (good, bad) = pms::twatch::live::parse_list(&self.new_favorite);
        if !bad.is_empty() {
            self.favorites_note = s::MODEL_BAD_NAME.with(&[("names", &bad.join(", "))]);
            return;
        }
        let favorites = &mut self.twatch().favorites;
        for handle in good {
            if !favorites.iter().any(|f| f.eq_ignore_ascii_case(&handle)) {
                favorites.push(handle);
            }
        }
        self.new_favorite.clear();
        self.favorites_note.clear();
    }

    /// Fetches Streamlink, on a thread of its own.
    pub fn get_streamlink(&self) {
        pms::net::follow(&self.config);
        let (shared, note) = (self.streamlink_job.clone(), self.streamlink_note.clone());
        *shared.lock().unwrap() = WadJob::Getting(0, 0);
        note.lock().unwrap().clear();
        std::thread::spawn(move || {
            let progress = |done, total| *shared.lock().unwrap() = WadJob::Getting(done, total);
            *shared.lock().unwrap() = match pms::streamlink::install(&progress) {
                Ok(fetched) => {
                    if !fetched {
                        *note.lock().unwrap() = s::MODEL_NEWEST.t().into();
                    }
                    WadJob::Idle
                }
                Err(e) => WadJob::Failed(e),
            };
        });
    }

    pub fn remove_streamlink(&self) {
        if let Err(e) = pms::streamlink::remove() {
            *self.streamlink_job.lock().unwrap() = WadJob::Failed(e);
        }
    }

    /// One more TV.
    pub fn add_tv(&mut self) {
        pms::tv::add_tv(&mut self.config);
    }

    /// One TV fewer: the last one added.
    pub fn remove_tv(&mut self) {
        if let Some(i) = self.config.meters.iter().rposition(is_tv) {
            self.config.meters.remove(i);
        }
    }

    /// The network settings, made if they aren't there.
    pub fn network(&mut self) -> &mut pms::config::Network {
        self.config.network.get_or_insert_with(Default::default)
    }

    /// The proxy as typed becomes the proxy, if it can be one, or none, if it's empty.
    pub fn apply_proxy(&mut self) {
        let typed = self.proxy_text.trim().to_string();
        if let Err(e) = (!typed.is_empty()).then(|| pms::net::check(&typed)).transpose() {
            *self.proxy_note.lock().unwrap() = s::MODEL_NOT_APPLIED.with(&[("why", &e)]);
            return;
        }
        // A proxy that nothing goes through protects nothing: one applied with every switch off switches them all on.
        let none_on = !self.uses_proxy();
        self.network().proxy = typed.clone();
        let said = if !typed.is_empty() && none_on {
            self.all_through_proxy(true);
            s::MODEL_ALL_THROUGH.t()
        } else {
            ""
        };
        self.proxy_text = typed;
        *self.proxy_note.lock().unwrap() = said.into();
    }

    /// Whether anything is switched on to go through the proxy.
    pub fn uses_proxy(&self) -> bool {
        self.config.network.as_ref().is_some_and(|n| n.live_checks || n.iss || n.streams || n.updates || n.downloads)
    }

    /// Tries the proxy that's applied, on a thread of its own: one small request to GitHub, which every update check makes too.
    pub fn test_proxy(&self) {
        let note = self.proxy_note.clone();
        let Some(proxy) = self.config.network.as_ref().map(|n| n.proxy.clone()).filter(|p| !p.is_empty()) else {
            *note.lock().unwrap() = s::MODEL_NO_PROXY.t().into();
            return;
        };
        *note.lock().unwrap() = s::MODEL_TRYING.t().into();
        // Which helpers are sandboxed goes along: following this mustn't take the settings' own pms-fetch out of its sandbox.
        let sandbox = self.config.sandbox.clone();
        std::thread::spawn(move || {
            let config = pms::config::parse(&format!("[network]\nproxy = \"{}\"\n", proxy.replace(['"', '\\'], ""))).ok().map(|c| pms::config::Config { sandbox, ..c });
            let said = match config {
                Some(config) => {
                    let began = Instant::now();
                    pms::net::follow(&config);
                    // By way of pms-fetch.exe, as the real thing goes: in its sandbox, if it has one.
                    match pms::fetch::fetch(pms::net::Use::Updates, pms::fetch::Kind::Head, pms::update::STABLE_URL, Duration::from_secs(20), 0, 0) {
                        Ok(got) if (200..300).contains(&got.status) => s::MODEL_PROXY_WORKS.with(&[("ms", &began.elapsed().as_millis())]),
                        Ok(got) => s::MODEL_PROXY_ANSWERED.with(&[("status", &got.status)]),
                        Err(e) => s::MODEL_DIDNT_WORK.with(&[("why", &e)]),
                    }
                }
                None => s::MODEL_BAD_ADDRESS.t().into(),
            };
            *note.lock().unwrap() = said;
        });
    }

    /// Every use through the proxy, or none.
    pub fn all_through_proxy(&mut self, on: bool) {
        let n = self.network();
        (n.live_checks, n.iss, n.streams, n.updates, n.downloads) = (on, on, on, on, on);
    }

    /// Tries the way use number `at` goes out, as the settings now have it, on a thread of its own.
    pub fn try_use(&self, at: usize) {
        let Some((what, ..)) = pms::net::USES.get(at).copied() else { return };
        pms::net::follow(&self.config);
        let (notes, failed) = (self.use_notes.clone(), self.use_failed.clone());
        notes.lock().unwrap()[at] = s::MODEL_TRYING.t().into();
        failed.lock().unwrap()[at] = false;
        std::thread::spawn(move || {
            let tried = pms::net::try_out(what);
            failed.lock().unwrap()[at] = tried.is_err();
            notes.lock().unwrap()[at] = tried.unwrap_or_else(|e| e);
        });
    }

    /// No TVs at all.
    pub fn remove_tvs(&mut self) {
        self.config.meters.retain(|m| !is_tv(m));
    }

    /// The folder WADs go in, in Explorer.
    pub fn show_wads(&self) {
        let dir = pms::wads::dir();
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::process::Command::new("explorer.exe").arg(dir).spawn();
    }

    /// Whether a check or a download is under way.
    pub fn updating(&self) -> bool {
        matches!(*self.update.lock().unwrap(), Update::Checking | Update::Downloading(..))
    }

    /// Starts the daemon (pmsd.exe beside this exe) with this config.
    pub fn start_daemon(&mut self) {
        let exe = std::env::current_exe().map(|p| p.with_file_name("pmsd.exe"));
        self.message = match exe.and_then(|exe| std::process::Command::new(exe).arg(&self.path).spawn()) {
            Ok(_) => (s::MODEL_STARTING.t().into(), false),
            Err(e) => (s::MODEL_COULDNT_START.with(&[("why", &e)]), true),
        };
    }

    /// Asks the daemon to stop (it puts the stock taskbar back).
    pub fn stop_daemon(&mut self) {
        self.message = match pms::daemon::stop() {
            Ok(_) => (s::MODEL_STOPPING.t().into(), false),
            Err(e) => (e, true),
        };
    }

    /// Start at sign-in, on or off.
    pub fn set_autostart(&mut self, on: bool) {
        let args = if on { vec!["on".to_string(), self.path.display().to_string()] } else { vec!["off".to_string()] };
        self.message = match pms::daemon::autostart(&args) {
            Ok(_) if on => (s::MODEL_WILL_START.t().into(), false),
            Ok(_) => (s::MODEL_WONT_START.t().into(), false),
            Err(e) => (e, true),
        };
        self.poll_daemon();
    }

    pub fn autostarts(&self) -> bool {
        self.autostart.starts_with("at logon")
    }

    /// The classic (egui) window in this one's place, on the same config: once edits are saved.
    pub fn open_classic(&mut self) {
        self.save_now();
        match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("--egui").arg(&self.path).spawn()) {
            Ok(_) => std::process::exit(0),
            Err(e) => self.message = (s::MODEL_NO_CLASSIC.with(&[("why", &e)]), true),
        }
    }

    /// The new (WinUI 3) window in this one's place, on the same config: once edits are saved.
    pub fn open_new(&mut self) {
        self.save_now();
        match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("--winui").arg(&self.path).spawn()) {
            Ok(_) => std::process::exit(0),
            Err(e) => self.message = (s::MODEL_NO_NEW.with(&[("why", &e)]), true),
        }
    }

    /// The welcome, again, in a window of its own.
    pub fn open_welcome(&mut self) {
        if let Err(e) = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("--welcome").arg(&self.path).spawn()) {
            self.message = (s::MODEL_NO_WELCOME.with(&[("why", &e)]), true);
        }
    }

    /// The config file, selected in Explorer.
    pub fn show_config(&self) {
        pms::notices::show_file(&self.path);
    }

    pub fn show_watchlist(&self) {
        // There's nothing to pick out until there are favorites: then it's the folder.
        let list = config::watchlist_path(&self.path);
        pms::notices::show_file(if list.is_file() { &list } else { &self.path });
    }

    /// The network settings' file, or the folder it would be in.
    pub fn show_network(&self) {
        let file = config::network_path(&self.path);
        pms::notices::show_file(if file.is_file() { &file } else { &self.path });
    }

    pub fn open_config(&self) {
        let _ = pms::notices::open_file(&self.path.display().to_string());
    }

    pub fn has_last_good(&self) -> bool {
        config::last_good(&self.path).is_some()
    }

    /// Goes back to the last settings that loaded. What was there is kept beside them.
    pub fn restore_last_good(&mut self) {
        match config::restore_last_good(&self.path) {
            Ok(()) => {
                self.follow_disk();
            }
            Err(e) => self.message = (s::MODEL_DIDNT_WORK.with(&[("why", &e)]), true),
        }
    }

    /// Every setting back to its default, but for the favorites, the network settings, the games and the answer about updates.
    pub fn reset_all(&mut self) {
        match config::reset(&self.path) {
            Ok(()) => {
                self.follow_disk();
                self.message = (s::MODEL_RESET_ALL_DONE.t().to_string(), false);
            }
            Err(e) => self.message = (s::MODEL_DIDNT_WORK.with(&[("why", &e)]), true),
        }
    }

    pub fn restart_explorer(&mut self) {
        self.message = match pms::explorer::ask() {
            Ok(()) => (s::MODEL_RESTARTING_EXPLORER.t().to_string(), false),
            Err(e) => (e, true),
        };
    }

    /// Says which lines of the watchlist were skipped, unless the favorites are being edited.
    fn note_watchlist(&mut self) {
        if self.favorites_text.is_none() {
            let bad: Vec<String> = config::watchlist_trouble(&self.path).into_iter().map(|(n, text)| s::MODEL_WATCHLIST_LINE.with(&[("line", &n), ("text", &text)])).collect();
            self.favorites_note = if bad.is_empty() { String::new() } else { s::MODEL_SKIPPED.with(&[("lines", &bad.join(", "))]) };
        }
    }

    /// The logs, with the daemon's selected in Explorer.
    pub fn show_logs(&self) {
        let log = pms_tap::logfile::Log::Daemon.path();
        let _ = std::process::Command::new("explorer.exe").arg("/select,").arg(log).spawn();
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

pub fn is_twatch(m: &MeterSpec) -> bool {
    m.style == pms::config::Style::Screen && m.set.as_deref() == Some("twatch")
}

fn is_tv(m: &MeterSpec) -> bool {
    m.style == pms::config::Style::Screen && !is_twatch(m)
}

/// A TV or a Twatch TV, which are widgets of their own rather than meters.
pub fn is_screen(m: &MeterSpec) -> bool {
    m.style == pms::config::Style::Screen
}

/// Something along the taskbar, for the Widgets page.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Widget {
    /// All the meters, together.
    Meters,
    /// A TV or a Twatch TV: where it is in the config's list of meters.
    Screen(usize),
    /// The Hot Mic's label, at one end.
    Mic,
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
            Section::Taskbar => {
                (c.theme, c.meter_height, c.bar_height, c.interval_ms, c.value_font, c.software_gpus) = (b.theme, b.meter_height, b.bar_height, b.interval_ms, b.value_font, b.software_gpus)
            }
            // Twatch TV's pictures are on this page too, without making a Twatch TV section where there isn't one.
            Section::Performance => {
                (c.max_fps, c.battery_fps, c.rest_for_games, c.rest_when_covered) = (b.max_fps, b.battery_fps, b.rest_for_games, b.rest_when_covered);
                c.efficiency = b.efficiency;
                if let Some(t) = c.twatch.as_mut() {
                    let d = b.twatch.unwrap_or_default();
                    (t.pictures, t.metered_pictures) = (d.pictures, d.metered_pictures);
                }
            }
            // Whether to check at all is consent, not a setting: only its owner changes that.
            Section::Updates => (c.update_channel, c.update_warn) = (b.update_channel, b.update_warn),
            Section::Meters => c.meters = b.meters,
            Section::Counters => *counters = self.baseline_rows(),
            Section::Backdrop => c.backdrop = b.backdrop,
            Section::StartGlow => c.start_glow = b.start_glow,
            Section::AppGlow => c.app_glow = b.app_glow,
            // Your favorites are yours.
            // Its pictures are under Power & performance, which resets them.
            Section::Twatch => {
                let mine = c.twatch.take();
                let favorites = mine.as_ref().map(|t| t.favorites.clone()).unwrap_or_default();
                c.twatch = if favorites.is_empty() { b.twatch } else { Some(pms::config::Twatch { favorites, ..b.twatch.unwrap_or_default() }) };
                if let (Some(t), Some(mine)) = (c.twatch.as_mut(), mine) {
                    (t.pictures, t.metered_pictures) = (mine.pictures, mine.metered_pictures);
                }
            }
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
            Section::Twatch if c.twatch.as_ref().is_some_and(|t| !t.favorites.is_empty()) => kept.push(s::MODEL_YOUR_FAVORITES.t()),
            Section::Hotmic if c.hotmic.as_ref().is_some_and(|h| !h.ignore.is_empty()) => kept.push(copy::setting("hotmic.ignore").0),
            Section::Toasts => match &c.toasts {
                Some(t) => {
                    let d = self.baseline.toasts.clone().unwrap_or_default();
                    if !t.enabled {
                        kept.push(s::MODEL_TOASTS_OFF.t());
                    }
                    if t.confirm_clicks != d.confirm_clicks {
                        kept.push(copy::setting("toasts.confirm_clicks").0);
                    }
                    if !t.ignore.is_empty() {
                        kept.push(copy::setting("toasts.ignore").0);
                    }
                }
                None => kept.push(s::MODEL_TOASTS_OFF.t()),
            },
            _ => {}
        }
        kept
    }

    /// What the confirmation says: what goes, and what stays.
    pub fn warning(&self, section: Section) -> String {
        let kept = self.kept(section);
        let all = if kept.is_empty() { s::MODEL_RESET_EVERYTHING } else { s::MODEL_RESET_EVERYTHING_ELSE }.with(&[("section", &section.name())]);
        let all = match section {
            Section::Meters => format!("{all} {}", s::MODEL_RESET_METERS.t()),
            Section::Counters => format!("{all} {}", s::MODEL_RESET_COUNTERS.t()),
            _ => all,
        };
        if kept.is_empty() { all } else { s::MODEL_RESET_KEEPS.with(&[("kept", &kept.join(", ")), ("rest", &all)]) }
    }

    /// Puts `section` back as it ships, saves that, and says what it left alone.
    pub fn reset(&mut self, section: Section) {
        let kept = self.kept(section);
        let (mut config, mut counters) = (self.config.clone(), std::mem::take(&mut self.counters));
        self.apply_reset(section, &mut config, &mut counters);
        (self.config, self.counters) = (config, counters);
        self.save_now();
        if !self.message.1 {
            let done = s::MODEL_SECTION_RESET.with(&[("section", &section.name())]);
            self.message = (if kept.is_empty() { done } else { s::MODEL_NOT_CHANGED.with(&[("done", &done), ("kept", &kept.join(", "))]) }, false);
        }
    }

    /// Takes in what changed in the file since we last read or wrote it, so that a save here never undoes it: the file
    /// wins over a change here that hasn't been saved yet. Returns whether the file is ours to write now. It isn't
    /// after taking something in, nor while what's there doesn't load: that's someone's work, not to be saved over.
    fn follow_disk(&mut self) -> bool {
        let Ok(text) = std::fs::read_to_string(&self.path) else { return true };
        let seen = on_disk(&self.path);
        if self.disk == seen {
            return self.problem.is_none();
        }
        self.disk = seen;
        let config = match config::parse_at(&self.path, &text) {
            Ok(c) => c,
            Err(e) => {
                self.problem = Some(pms::problem::Problem::of(&self.path, &e));
                self.message = (NOT_SAVING.t().to_string(), true);
                return false;
            }
        };
        if self.problem.take().is_some() {
            self.message = (SAVES.t().to_string(), false);
        }
        self.note_watchlist();
        // Only comments or spacing changed: nothing to take in.
        if config::comparable(&config).is_ok_and(|t| t == self.saved) {
            return true;
        }
        let unsaved = config::comparable(&self.current()).is_ok_and(|t| t != self.saved);
        self.counters = config.counters.iter().map(|(name, spec)| counter_row(name, spec)).collect();
        self.config = config;
        self.proxy_text = shown_proxy(&self.config);
        self.saved = config::comparable(&self.current()).unwrap_or_default();
        self.edited_at = None;
        self.message = if unsaved {
            (s::MODEL_CHANGED_OUTSIDE.t().to_string(), true)
        } else {
            (s::MODEL_CHANGED.t().to_string(), false)
        };
        false
    }

    /// The shipped meter this one is (by its name), for what its settings go back to; None for one of your own.
    pub fn baseline_meter(&self, label: &str) -> Option<&MeterSpec> {
        self.baseline.meters.iter().find(|b| b.label.eq_ignore_ascii_case(label))
    }

    /// The shipped meters you don't have (going by their names), for Add back. Not TVs: the TV page has those.
    pub fn missing_meters(&self) -> Vec<&MeterSpec> {
        let have = |label: &str| self.config.meters.iter().any(|m| m.label.eq_ignore_ascii_case(label));
        self.baseline.meters.iter().filter(|b| !is_screen(b) && !have(&b.label)).collect()
    }

    /// The meters, not the TVs or Twatch TVs: where each is in the config's list, in order.
    pub fn plain_meters(&self) -> Vec<usize> {
        self.config.meters.iter().enumerate().filter(|(_, m)| !is_screen(m)).map(|(i, _)| i).collect()
    }

    /// What's along the taskbar, in its order: the meters together, where the first of them is; each TV and Twatch TV; and the
    /// Hot Mic's label at whichever end it goes, while it's on.
    pub fn widgets(&self) -> Vec<Widget> {
        let mic = self.config.hotmic.is_some().then(|| self.config.mic_position() == "start");
        let mut out = Vec::new();
        if mic == Some(true) {
            out.push(Widget::Mic);
        }
        for (i, m) in self.config.meters.iter().enumerate() {
            if is_screen(m) {
                out.push(Widget::Screen(i));
            } else if !out.contains(&Widget::Meters) {
                out.push(Widget::Meters);
            }
        }
        if mic == Some(false) {
            out.push(Widget::Mic);
        }
        out
    }

    /// Whether widget `at` (in `widgets()`) can move a step up (`up`) or down: the Hot Mic's label always can, to the other end;
    /// the rest move among themselves, between its ends.
    pub fn can_move_widget(&self, at: usize, up: bool) -> bool {
        let widgets = self.widgets();
        match widgets.get(at) {
            Some(Widget::Mic) => (at == 0) != up,
            Some(w) => {
                let middle: Vec<&Widget> = widgets.iter().filter(|w| **w != Widget::Mic).collect();
                let k = middle.iter().position(|m| *m == w).unwrap_or(0);
                if up { k > 0 } else { k + 1 < middle.len() }
            }
            None => false,
        }
    }

    /// Moves widget `at` (in `widgets()`) a step up (`up`) or down. The Hot Mic's label goes to the other end. Moving anything
    /// writes the list with the meters together, where the widgets say.
    pub fn move_widget(&mut self, at: usize, up: bool) {
        if !self.can_move_widget(at, up) {
            return;
        }
        let mut widgets = self.widgets();
        let moving = widgets[at];
        if moving == Widget::Mic {
            if let Some(h) = self.config.hotmic.as_mut() {
                h.position = if up { "start" } else { "tray" }.into();
            }
            return;
        }
        widgets.retain(|w| *w != Widget::Mic);
        let k = widgets.iter().position(|w| *w == moving).unwrap_or(0);
        widgets.swap(k, if up { k - 1 } else { k + 1 });
        let plain = self.plain_meters();
        self.arrange(&widgets, &plain);
    }

    /// Moves meter `i` (in the config's list) a step up or down among the meters, past any TV between them. Returns where it is
    /// now.
    pub fn move_meter(&mut self, i: usize, up: bool) -> usize {
        let mut plain = self.plain_meters();
        let Some(k) = plain.iter().position(|&p| p == i) else { return i };
        let other = if up { k.checked_sub(1) } else { (k + 1 < plain.len()).then_some(k + 1) };
        let Some(other) = other else { return i };
        plain.swap(k, other);
        let widgets: Vec<Widget> = self.widgets().into_iter().filter(|w| *w != Widget::Mic).collect();
        self.arrange(&widgets, &plain)[i]
    }

    /// Where a new meter goes: after the last meter, so before any TV that follows them.
    pub fn new_meter_at(&self) -> usize {
        self.plain_meters().last().map_or(0, |i| i + 1)
    }

    /// Writes the config's list in `widgets`' order, the meters together in `plain`'s. Returns where each one went.
    fn arrange(&mut self, widgets: &[Widget], plain: &[usize]) -> Vec<usize> {
        let old = std::mem::take(&mut self.config.meters);
        let mut went = vec![0; old.len()];
        let mut order = Vec::with_capacity(old.len());
        for w in widgets {
            match w {
                Widget::Meters => order.extend_from_slice(plain),
                Widget::Screen(i) => order.push(*i),
                Widget::Mic => {}
            }
        }
        for (new, &i) in order.iter().enumerate() {
            went[i] = new;
        }
        let mut old: Vec<Option<MeterSpec>> = old.into_iter().map(Some).collect();
        self.config.meters = order.iter().filter_map(|&i| old[i].take()).collect();
        went
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

impl Model {
    /// What's wrong with each setting that couldn't be used: the setting, the page it's on, and what to say, for the settings to
    /// mark it in red.
    pub fn issues(&self) -> Vec<(String, &'static str, String)> {
        self.config
            .issues
            .iter()
            .map(|e| {
                let id = config::issue_setting(e);
                let then = if config::issue_is_unknown(e) { s::MODEL_LEFT_OUT.t() } else { s::MODEL_BACK_TO_DEFAULT.t() };
                let words = format!("{}\n{then}", pms::problem::Problem::of(&self.path, e).words());
                (id.clone(), config::settings_page(&id), words)
            })
            .collect()
    }
}

/// The proxy as the settings show it: none, while the network settings' file won't load (the banner says why).
fn shown_proxy(config: &Config) -> String {
    config.network.as_ref().map(|n| n.proxy.clone()).filter(|p| p != pms::config::NETWORK_UNREAD).unwrap_or_default()
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
    fn widgets_move_with_the_meters_together_and_the_mic_at_an_end() {
        let mut m = stripped();
        let mut tv2 = m.config.meters[1].clone();
        tv2.label = "TV 2".into();
        let mut ram = m.config.meters[0].clone();
        ram.label = "RAM".into();
        // CPU, TV, RAM, TV 2: the meters show as one, where the first is.
        m.config.meters = vec![m.config.meters[0].clone(), m.config.meters[1].clone(), ram, tv2];
        m.config.hotmic = Some(Default::default());
        let labels = |m: &Model| m.config.meters.iter().map(|x| x.label.clone()).collect::<Vec<_>>();
        assert_eq!(m.widgets(), [Widget::Meters, Widget::Screen(1), Widget::Screen(3), Widget::Mic]);
        // Nothing passes the mic, which goes to the other end.
        assert!(!m.can_move_widget(2, false) && !m.can_move_widget(0, true));
        m.move_widget(3, true);
        assert_eq!((m.config.mic_position(), m.widgets()[0]), ("start", Widget::Mic));
        // TV up past the meters: the meters come together after it.
        m.move_widget(2, true);
        assert_eq!(labels(&m), ["TV", "CPU", "RAM", "TV 2"]);
        assert_eq!(m.widgets(), [Widget::Mic, Widget::Screen(0), Widget::Meters, Widget::Screen(3)]);
        // A meter moves among the meters, and says where it went; past the first, it stays.
        assert_eq!(m.move_meter(2, true), 1);
        assert_eq!(labels(&m), ["TV", "RAM", "CPU", "TV 2"]);
        assert_eq!(m.move_meter(1, true), 1);
        assert_eq!(m.new_meter_at(), 3);
        // The TV isn't a meter to add back.
        m.config.meters.retain(|x| !is_screen(x));
        assert!(m.missing_meters().iter().all(|b| !is_screen(b)));
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
        // A folder each: what's beside a settings file goes with it.
        let dir = std::env::temp_dir().join(format!("pms-gui-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pms.toml");
        let _ = std::fs::remove_file(config::watchlist_path(&path));
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
            Section::Twatch,
            Section::Hotmic,
            Section::Toasts,
        ];
        let mut m = on_file("sections");
        assert!(sections.iter().all(|s| m.at_default(*s)), "a new config is all defaults");
        let c = &mut m.config;
        c.theme = "dark".into();
        c.software_gpus = true;
        c.rest_for_games = false;
        c.update_channel = "latest".into();
        c.meters.remove(1);
        c.backdrop = None;
        c.start_glow = None;
        c.app_glow = None;
        c.hotmic = None;
        c.twatch = Some(pms::config::Twatch { more_sets: 1, ..Default::default() });
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
    fn saving_waits_for_the_edits_to_stop() {
        let mut m = on_file("settle");
        m.config.theme = "dark".into();
        m.autosave();
        std::thread::sleep(SAVE_AFTER / 2);
        m.config.theme = "light".into();
        m.autosave();
        std::thread::sleep(SAVE_AFTER * 3 / 4);
        m.autosave();
        assert!(!std::fs::read_to_string(&m.path).unwrap().contains("theme = \"light\""), "still typing: it's only been a moment since the last edit");
        std::thread::sleep(SAVE_AFTER / 2);
        m.autosave();
        assert!(std::fs::read_to_string(&m.path).unwrap().contains("theme = \"light\""));
        std::fs::remove_file(&m.path).unwrap();
    }

    #[test]
    fn a_favorite_moved_is_saved_once_the_moving_stops_or_the_window_goes() {
        let mut m = on_file("order");
        let saved = |m: &Model| std::fs::read_to_string(config::watchlist_path(&m.path)).map(|text| config::read_watchlist(&text).0).unwrap_or_default();
        m.twatch().favorites = vec!["crab".into(), "lobster".into(), "shrimp".into()];
        m.save_now();
        assert_eq!(saved(&m), ["crab", "lobster", "shrimp"]);
        // A step down, and another: long enough apart for anything else to have been saved between them, and it isn't.
        m.swap_favorites(0, 1);
        m.autosave();
        std::thread::sleep(SAVE_AFTER * 3 / 2);
        m.autosave();
        assert_eq!(saved(&m), ["crab", "lobster", "shrimp"], "there may be more moves to come");
        m.swap_favorites(1, 2);
        m.autosave();
        assert_eq!(m.twatch().favorites, ["lobster", "shrimp", "crab"]);
        // The window closing doesn't lose it.
        m.save_now();
        assert_eq!(saved(&m), ["lobster", "shrimp", "crab"]);
        // What's dragged into place is taken if it's the same channels, and only then; and anything after a save waits no
        // longer than it ever did.
        m.reorder_favorites(vec!["crab".into(), "lobster".into()]);
        m.reorder_favorites(vec!["crab".into(), "lobster".into(), "prawn".into()]);
        assert_eq!(m.twatch().favorites, ["lobster", "shrimp", "crab"]);
        m.reorder_favorites(vec!["crab".into(), "lobster".into(), "shrimp".into()]);
        assert_eq!(m.twatch().favorites, ["crab", "lobster", "shrimp"]);
        m.save_now();
        m.twatch().favorites.pop();
        m.autosave();
        std::thread::sleep(SAVE_AFTER * 3 / 2);
        m.autosave();
        assert_eq!(saved(&m), ["crab", "lobster"]);
        let _ = std::fs::remove_file(config::watchlist_path(&m.path));
        std::fs::remove_file(&m.path).unwrap();
    }

    #[test]
    fn a_list_of_favorites_only_changes_when_its_saved_and_right() {
        let mut m = on_file("favorites-text");
        m.twatch().favorites = vec!["crab".into()];
        m.favorites_as_text();
        assert_eq!(m.favorites_text.as_deref(), Some("crab"));
        m.favorites_text = Some("lobs".into());
        assert_eq!(m.twatch().favorites, ["crab"], "a half-typed name isn't a favorite");
        m.favorites_text = Some("lobster\nnot a-name\n".into());
        m.save_favorites_text();
        assert_eq!(m.twatch().favorites, ["crab"]);
        assert!(m.favorites_note.contains("a-name") && m.favorites_text.is_some(), "it says what's wrong and keeps your text");
        m.favorites_text = Some("lobster\nhttps://www.twitch.tv/Shrimp\n".into());
        m.save_favorites_text();
        assert_eq!(m.twatch().favorites, ["lobster", "shrimp"]);
        assert!(m.favorites_text.is_none() && m.favorites_note.is_empty());
        m.new_favorite = "@prawn".into();
        m.add_favorite();
        m.new_favorite = "PRAWN".into();
        m.add_favorite();
        assert_eq!(m.twatch().favorites, ["lobster", "shrimp", "prawn"]);
        assert!(m.new_favorite.is_empty());
        let _ = std::fs::remove_file(&m.path);
    }

    #[test]
    fn a_reset_of_how_it_plays_leaves_your_favorites() {
        let mut m = on_file("twatch-reset");
        m.config.twatch = Some(pms::config::Twatch { favorites: vec!["crab".into()], top_quality: "1080p60".into(), ..Default::default() });
        assert!(!m.at_default(Section::Twatch));
        assert_eq!(m.kept(Section::Twatch), ["Your favorites"]);
        m.reset(Section::Twatch);
        let t = m.config.twatch.clone().unwrap();
        assert_eq!((t.favorites, t.top_quality.as_str()), (vec!["crab".to_string()], "480p"));
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
    fn a_file_broken_before_the_window_opened_is_left_alone_too() {
        let path = std::env::temp_dir().join(format!("pms-gui-test-opened-broken-{}.toml", std::process::id()));
        std::fs::write(&path, "config_version = 2\n[twatch\n").unwrap();
        let mut m = Model::open(path.clone());
        assert_eq!(m.problem.as_ref().and_then(|p| p.line), Some(2));
        m.config.interval_ms = 2000;
        m.edited_at = Some(Instant::now() - SAVE_AFTER);
        m.autosave();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "config_version = 2\n[twatch\n", "a change here doesn't overwrite it");
        std::fs::write(&path, "config_version = 2\ninterval_ms = 777\n").unwrap();
        m.autosave();
        assert!(m.problem.is_none(), "fixing the file is noticed");
        assert_eq!(m.config.interval_ms, 777);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn favorites_are_saved_to_their_own_file_and_followed_from_it() {
        let mut m = on_file("watchlist");
        let list = config::watchlist_path(&m.path);
        let _ = std::fs::remove_file(&list);
        m.twatch().favorites = vec!["crab".into()];
        m.edited_at = Some(Instant::now() - SAVE_AFTER);
        m.autosave();
        m.autosave();
        assert!(std::fs::read_to_string(&list).is_ok_and(|t| t.contains("crab\n")), "in the watchlist");
        assert!(!std::fs::read_to_string(&m.path).unwrap().contains("crab"), "and not in the settings");
        std::fs::write(&list, "lobster\n").unwrap();
        m.poll_daemon();
        assert_eq!(m.config.twatch.as_ref().unwrap().favorites, ["lobster"]);
        std::fs::remove_file(&list).unwrap();
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
