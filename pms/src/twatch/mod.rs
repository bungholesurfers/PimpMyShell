//! Twatch TV: a `screen` meter with `set = "twatch"`, a flat-screen set that keeps an eye on your favorite Twitch channels and
//! plays the best one that's live, as live video, or as a picture now and then if that's what you'd rather have. With `more_sets`
//! it can also bring out one or two more sets while more favorites are live, which go away again afterwards, and a button says
//! when there's a favorite no set can show. Off the mains, or short of memory, it shows pictures in place of video, and goes back
//! when that's over.
//!
//! The pieces, each of which knows as little as it can about the others:
//!
//!   live.rs       who's live, how often to look, who goes on which set: decisions only
//!   look.rs       what a set draws itself
//!   player.rs     the player process, and how high a stream may go
//!   streamlink.rs (beside this) turning a channel into a stream
//!
//! This file runs them. One thread owns every set, and anything slow (a look, asking Streamlink, a picture) happens on a thread of
//! its own and comes back as an `Event`, so that nothing that's drawing ever waits for the net.

pub mod live;
mod look;
mod player;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pms_screen::{BUTTONS, Button, Screen, Status};

use crate::config::{Config, MeterSpec, Style};
use crate::pressure::Pressure;
use crate::streamlink::{self, Asked as Answer, Stream};
use crate::tv::Canvas;
use live::{Slot, Watch};
use player::{Pace, Player, Quality, Said, Tier, Why};

/// The most sets that can come and go, beside the one you put there.
pub const MOST_MORE_SETS: u32 = 2;
/// The most sets of all, past which nothing suggests another.
const MOST_SETS: usize = 6;
/// The buttons beside the sets, which are screens with no meter of their own in the file.
const MORE_CELL: &str = "Twatch More";
const AUTO_CELL: &str = "Twatch Auto";
pub const CELL_SET: &str = "twatch cell";
/// How long the button for another set stays wide enough to say why it's there.
const ANNOUNCED: Duration = Duration::from_secs(8);
/// How long before a favorite that went live is news again, if it drops out and comes back.
const TOLD_AGAIN: Duration = Duration::from_secs(30 * 60);
/// The highest qualities there are to choose from, lowest first.
pub const QUALITIES: [&str; 5] = ["360p", "480p", "720p", "720p60", "1080p60"];
/// What a set looks at when it has nothing moving.
const QUIET_FPS: u32 = 10;
/// How long a set with nobody to show, and nobody touching it, stays on, and for how long before that it says so.
const SLEEPS_AFTER: Duration = Duration::from_secs(5 * 60);
const WARNS_FOR: Duration = Duration::from_secs(60);

/// `SLEEPS_AFTER` and `WARNS_FOR`, or, for trying it, as many seconds as PMS_TWATCH_SLEEPS says, half of them warned.
fn sleep_times() -> (Duration, Duration) {
    match std::env::var("PMS_TWATCH_SLEEPS").ok().and_then(|s| s.parse::<u64>().ok()) {
        Some(secs) => (Duration::from_secs(secs), Duration::from_secs(secs / 2)),
        None => (SLEEPS_AFTER, WARNS_FOR),
    }
}
/// How long the plus beside the sets takes to fade to an outline once it's put away, and in how many steps.
const PLUS_FADE: Duration = Duration::from_millis(700);
const PLUS_FADE_STEPS: f32 = 20.0;
/// How long the channel's panel stays up.
const NUMBER: i64 = 2500;
/// How often a still picture is fetched.
const STILL_EVERY: Duration = Duration::from_secs(60);
/// A stream that's sent no frames for this long has a problem.
const NO_FRAMES: Duration = Duration::from_secs(20);
/// How long to wait before trying again, after each failure in a row.
const RETRIES: [Duration; 3] = [Duration::from_secs(10), Duration::from_secs(30), Duration::from_secs(60)];
/// How often to see whether Streamlink has been fetched, while it's missing.
const CHECK_STREAMLINK: Duration = Duration::from_secs(5);
/// How often to ask whether there's a newer Streamlink, while it keeps failing.
const NEWER_STREAMLINK: Duration = Duration::from_secs(6 * 60 * 60);
/// How long a set shows its logo on starting, in milliseconds.
const BOOT: i64 = 1000;
/// The shortest wait between looks that a config can ask for, in seconds.
pub const FASTEST_CHECK: u32 = 30;

/// Twitch's preview pictures, which only exist for a channel that's live, and the channel's page.
pub const PREVIEWS: &str = "https://static-cdn.jtvnw.net/previews-ttv/";
const CHANNEL_PAGE: &str = "https://www.twitch.tv/";


/// Sets that shouldn't show at the moment, by their meters' labels.
static HIDDEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The buttons beside the sets that should show at the moment.
static CELLS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
/// A newer Streamlink that's been found and not said yet.
static NEWER: Mutex<Option<String>> = Mutex::new(None);
/// A favorite is live and a set is waiting to play it, but there's no Streamlink: not said yet, and whether that's been looked into
/// since the daemon started.
static NO_STREAMLINK: Mutex<(bool, bool)> = Mutex::new((false, false));
/// You've been told there's no Streamlink. Once is enough: the set says so whenever you look at it. It goes once Streamlink is here,
/// so that losing it again is news.
const TOLD_NO_STREAMLINK: &str = "streamlink-told.txt";
/// What's been asked for that only the daemon can do.
static ASKED: Mutex<Asked> = Mutex::new(Asked { add_set: false, removed: Vec::new(), quality: 0, volume: 0, went_live: Vec::new() });

/// What the sets have asked of the daemon.
#[derive(Default)]
pub struct Asked {
    /// One more set, for good.
    pub add_set: bool,
    /// Sets to take off the taskbar, by name, and whether each is one that's always there (as opposed to coming and going).
    pub removed: Vec<(String, bool)>,
    /// How many steps to move the highest quality, up or down.
    pub quality: i32,
    /// And the volume, by tens.
    pub volume: i32,
    /// Favorites that have just gone live, for telling you about.
    pub went_live: Vec<String>,
}

/// What's been asked for since this was last called.
pub fn take_asked() -> Asked {
    ASKED.lock().map(|mut a| std::mem::take(&mut *a)).unwrap_or_default()
}

fn ask(what: impl FnOnce(&mut Asked)) {
    if let Ok(mut asked) = ASKED.lock() {
        what(&mut asked);
    }
}

/// The newer Streamlink there is, the first time this is asked after finding it, for telling you about. It's for you to fetch.
pub fn newer_streamlink() -> Option<String> {
    NEWER.lock().ok()?.take()
}

/// Whether to tell you that a favorite is live and there's no Streamlink to play it: the first time it happens, and not again until
/// Streamlink has been here and gone.
pub fn no_streamlink() -> bool {
    NO_STREAMLINK.lock().is_ok_and(|mut n| std::mem::take(&mut n.0))
}

fn found_no_streamlink() {
    if let Ok(mut n) = NO_STREAMLINK.lock()
        && !n.1
    {
        let told = pms_tap::data_dir().join(TOLD_NO_STREAMLINK);
        *n = (!told.exists(), true);
        if n.0 {
            let _ = std::fs::write(told, "");
        }
    }
}

/// Streamlink is here: the next time it's missing, that's worth saying.
fn streamlink_is_here() {
    let _ = std::fs::remove_file(pms_tap::data_dir().join(TOLD_NO_STREAMLINK));
    if let Ok(mut n) = NO_STREAMLINK.lock() {
        *n = (false, false);
    }
}

/// Whether the meter labelled `label` is a set with nothing to show that should stay out of the way, or a button with nothing to say.
pub fn hidden(label: &str) -> bool {
    if label == MORE_CELL || label == AUTO_CELL {
        return !CELLS.lock().is_ok_and(|c| c.contains(&label));
    }
    HIDDEN.lock().is_ok_and(|h| h.iter().any(|l| l == label))
}

fn is_twatch(m: &MeterSpec) -> bool {
    m.style == Style::Screen && m.set.as_deref() == Some("twatch")
}

/// Takes the set called `name` off the taskbar: its meter, for one that's always there; for one that comes and goes, one fewer of
/// those. Its favorites and settings stay, for when there's a set again.
pub fn remove_set(config: &mut Config, name: &str, fixed: bool) {
    if fixed {
        config.meters.retain(|m| !(is_twatch(m) && m.label == name));
    } else if let Some(t) = config.twatch.as_mut() {
        t.more_sets = t.more_sets.min(MOST_MORE_SETS).saturating_sub(1);
    }
}

fn more_sets(config: &Config) -> u32 {
    config.twatch.as_ref().map_or(0, |t| t.more_sets.min(MOST_MORE_SETS))
}

/// The names sets can have that the ones in the file don't, in order.
fn free_names(config: &Config) -> impl Iterator<Item = String> {
    let taken: Vec<String> = config.meters.iter().filter(|m| is_twatch(m)).map(|m| m.label.clone()).collect();
    let first = taken.first().cloned().unwrap_or_default();
    (2..).map(move |n| format!("{first} {n}")).filter(move |l| !taken.contains(l))
}

/// `config` with what goes after the last set that's there, for the meters: the sets that can come and go, and the buttons. None
/// with no set to go after.
pub fn with_more_sets(config: &Config) -> Option<Config> {
    let at = config.meters.iter().rposition(is_twatch)?;
    let mut out = config.clone();
    let like_it = |label: &str, set: &str| {
        let mut m = config.meters[at].clone();
        m.label = label.to_string();
        m.set = Some(set.to_string());
        m
    };
    let more: Vec<String> = free_names(config).take(more_sets(config) as usize).collect();
    let cells = [like_it(MORE_CELL, CELL_SET), like_it(AUTO_CELL, CELL_SET)];
    out.meters.splice(at + 1..at + 1, more.iter().map(|l| like_it(l, "twatch")).chain(cells));
    Some(out)
}

/// One more set for good, after the last one there is.
pub fn add_set(config: &mut Config) {
    let Some(at) = config.meters.iter().rposition(is_twatch) else { return };
    let Some(label) = free_names(config).next() else { return };
    let mut set = config.meters[at].clone();
    set.label = label;
    config.meters.insert(at + 1, set);
}

/// Moves the highest quality `by` steps, stopping at either end.
pub fn step_quality(config: &mut Config, by: i32) {
    if by != 0 {
        let t = config.twatch.get_or_insert_with(Default::default);
        let at = QUALITIES.iter().position(|q| *q == t.top_quality).unwrap_or(1) as i32;
        t.top_quality = QUALITIES[(at + by).clamp(0, QUALITIES.len() as i32 - 1) as usize].to_string();
    }
}

/// Steps every set's volume `by` tens, from 0 to 100.
pub fn step_volume(config: &mut Config, by: i32) {
    if by != 0 {
        let t = config.twatch.get_or_insert_with(Default::default);
        let now = t.volume.min(100) as i32;
        // Onto the tens first, from wherever a hand-written value put it.
        let to = if by > 0 { (now / 10 + by) * 10 } else { ((now + 9) / 10 + by) * 10 };
        t.volume = to.clamp(0, 100) as u32;
    }
}

/// Whether Windows says the internet connection is metered: a data cap, paying by the byte, roaming, or over the limit. Asked at
/// most every 30 seconds. PMS_METERED: as if it were, for trying it.
fn metered() -> bool {
    static SEEN: std::sync::Mutex<Option<(Instant, bool)>> = std::sync::Mutex::new(None);
    if std::env::var_os("PMS_METERED").is_some() {
        return true;
    }
    let Ok(mut seen) = SEEN.lock() else { return false };
    if let Some((at, said)) = *seen
        && at.elapsed() < Duration::from_secs(30)
    {
        return said;
    }
    let said = (|| -> windows::core::Result<bool> {
        use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
        let cost = NetworkInformation::GetInternetConnectionProfile()?.GetConnectionCost()?;
        Ok(matches!(cost.NetworkCostType()?, NetworkCostType::Fixed | NetworkCostType::Variable) || cost.Roaming()? || cost.OverDataLimit()?)
    })()
    .unwrap_or(false);
    *seen = Some((Instant::now(), said));
    said
}

/// What the group of sets is to be, as the daemon sees it.
#[derive(Clone, PartialEq, Debug)]
struct Setup {
    /// The sets, and whether each is one that's always there.
    sets: Vec<(String, bool)>,
    favorites: Vec<String>,
    pictures: Pictures,
    best: Quality,
    top: String,
    notify: bool,
    /// How many more sets come by themselves.
    more: u32,
    /// Sets stay on what they're showing when a higher favorite goes live, until you say otherwise.
    stay: bool,
    /// How often each favorite is looked at, and at most how much longer each wait can be.
    check: Duration,
    check_extra: Duration,
    battery: bool,
    /// The connection is metered, and that's to mean pictures.
    metered: bool,
    /// How loud the sets are, 0 to 100.
    volume: u32,
    /// When a set shows in Windows' media controls.
    media: MediaControls,
    busy: bool,
    resting: bool,
    /// Windows has turned the display off.
    dark: bool,
    /// What the machine is short of.
    pressure: Pressure,
}

/// When pictures take the place of video.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Pictures {
    Never,
    /// Until you click a set.
    Initial,
    /// While the machine is short of memory.
    LowMemory,
    Always,
}

impl Pictures {
    fn parse(text: &str) -> Pictures {
        match text {
            "never" => Pictures::Never,
            "initial" => Pictures::Initial,
            "always" => Pictures::Always,
            _ => Pictures::LowMemory,
        }
    }
}

/// When a set shows in Windows' media controls, and so takes the media keys.
#[derive(Clone, Copy, PartialEq, Debug)]
enum MediaControls {
    Never,
    /// While its sound is on.
    Unmuted,
    /// Whenever it's playing.
    Playing,
}

impl MediaControls {
    fn parse(text: &str) -> MediaControls {
        match text {
            "unmuted" => MediaControls::Unmuted,
            "playing" => MediaControls::Playing,
            _ => MediaControls::Never,
        }
    }

    fn shows(self, muted: bool) -> bool {
        match self {
            MediaControls::Never => false,
            MediaControls::Unmuted => !muted,
            MediaControls::Playing => true,
        }
    }
}

/// What a set shows.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Shows {
    Video,
    Pictures,
    Nothing,
}

impl Setup {
    /// What a set shows, for one that `wants_video`, having been clicked. Video would run down a battery, and with memory short
    /// the player has to go, so pictures take over, or if you said never, nothing does.
    fn shows(&self, wants_video: bool) -> Shows {
        if self.battery || self.metered || self.pictures == Pictures::Always {
            Shows::Pictures
        } else if self.pressure.twatch().is_some() {
            if self.pictures == Pictures::Never { Shows::Nothing } else { Shows::Pictures }
        } else if self.pictures == Pictures::Initial && !wants_video {
            Shows::Pictures
        } else {
            Shows::Video
        }
    }

    /// Whether a set with this mute stops for now: nobody can see it, and it isn't being listened to.
    fn dozes(&self, muted: bool, wants_video: bool) -> bool {
        self.dark && (muted || self.shows(wants_video) != Shows::Video)
    }
}

fn setup(config: &Config, resting: bool, battery: bool, busy: bool, dark: bool) -> Setup {
    let mine = config.twatch.clone().unwrap_or_default();
    let mut sets: Vec<(String, bool)> = config.meters.iter().filter(|m| is_twatch(m)).map(|m| (m.label.clone(), true)).collect();
    if !sets.is_empty() {
        sets.extend(free_names(config).take(more_sets(config) as usize).map(|l| (l, false)));
    }
    // Nothing in the file can make it hammer Twitch.
    let check = Duration::from_secs(u64::from(mine.check_seconds.clamp(FASTEST_CHECK, 3600)));
    let check_extra = Duration::from_secs(u64::from(mine.check_extra_seconds.min(3600)));
    let (pictures, more) = (Pictures::parse(&mine.pictures), more_sets(config));
    let best = Quality::parse(&mine.top_quality).unwrap_or(Quality::USUAL);
    Setup { sets, favorites: live::tidy(&mine.favorites), pictures, best, top: mine.top_quality, notify: mine.notify_live, more, stay: mine.higher_favorite == "stay", check, check_extra, battery, metered: false, volume: mine.volume.min(100), media: MediaControls::parse(&mine.media_controls), busy, resting, dark, pressure: Pressure::default() }
}

/// What comes back from the slow work.
pub enum Event {
    Looked { handle: String, live: Result<bool, String> },
    Found { set: usize, id: u64, asked: Answer, tiers: Vec<Tier> },
    Still { set: usize, id: u64, picture: Result<Option<Picture>, String> },
    Player { set: usize, id: u64, said: Said },
    /// How looking for a newer Streamlink went: which one there is, if there is.
    Newer(Result<Option<String>, String>),
}

/// A still picture: RGB, width, height.
pub type Picture = (Vec<u8>, usize, usize);

/// Where a set is getting its picture from.
enum Feed {
    /// Nowhere.
    Nothing,
    /// Streamlink's being asked.
    Asking,
    Playing(Box<Player>),
    Stills,
    /// There's something wrong.
    Trouble,
}

/// What's on the screen, when it isn't a picture.
#[derive(Clone, PartialEq)]
enum Scene {
    Off,
    /// Off and put away; on standby (it went off by itself, and comes back on by itself), or not.
    PutAway(bool),
    NoSignal,
    Standby(&'static str),
    /// Why it's been evicted, if it has.
    Still(Option<&'static str>),
    /// It's kept to a channel that isn't live.
    Waiting,
    /// It's been evicted, with nothing to show for it.
    Evicted(&'static str),
    /// Somebody's live for it, and video is wanted, but there's no Streamlink to play them.
    NoStreamlink,
    /// The same, with no player to play them: part of PimpMyShell is missing.
    NoPlayer,
}

struct Set {
    name: String,
    screen: Screen,
    /// Always there, as opposed to coming and going.
    fixed: bool,
    /// Out of sight, for one that comes and goes.
    hidden: bool,
    feed_name: String,
    on: bool,
    muted: bool,
    /// It stays on what it's showing, whatever else goes live, until that ends.
    held: bool,
    /// The one channel it shows.
    only: Option<String>,
    /// You clicked it, where pictures come until you do.
    wants_video: bool,
    /// Why its player has gone, while memory is short.
    evicted: Option<&'static str>,
    /// It stopped for the dark with a picture up, which stays up until there's a new one.
    dozing: bool,
    /// Its player has been told not to bother with the picture.
    video_off: bool,
    /// How loud its player's been told to be.
    volume_told: Option<u32>,
    /// Whether its player's been told to be in Windows' media controls.
    media_told: Option<bool>,
    /// It has somebody to play, as video, and there's no Streamlink to do it.
    needs_streamlink: bool,
    /// The same, with no player (pms-twatch.exe) to do it.
    needs_player: bool,
    /// With nobody to show, its little logo going round, and whether that's what it's doing: it wants frames for it.
    bounce: look::Bounce,
    bouncing: bool,
    /// Since when it's had nobody to show, with nobody touching it: after `SLEEPS_AFTER` it goes to standby, as a TV does.
    quiet_since: Option<Instant>,
    /// It's on standby: it went off by itself, and comes on again for a favorite who's live, or at the next start.
    slept: bool,
    /// It was muted because it changed channel by itself, and it's still muted.
    auto_muted: bool,
    /// The last channel it showed: a new one it comes to by itself is somebody else.
    was_showing: Option<String>,
    /// How much longer it shows its logo.
    boot: i64,
    stays: i64,
    small: bool,
    clicks: u32,
    pressed: [u32; BUTTONS.len()],
    showing: Option<String>,
    pinned: Option<String>,
    feed: Feed,
    /// Which try this is: what comes back from an earlier one is out of date.
    id: u64,
    tries: usize,
    retry_at: Instant,
    stream: Option<Stream>,
    pace: Option<Pace>,
    rate: u32,
    number: i64,
    shown: u64,
    frame_at: Instant,
    still: Option<Picture>,
    still_at: Option<Instant>,
    drawn: Option<(Scene, String, u32, u32)>,
    px: Vec<u8>,
}

fn state_file() -> PathBuf {
    crate::config::beside_config("twatch.txt")
}

fn kept_text(sets: &[Set]) -> String {
    // A set on standby is kept as on: it's off only until there's something to see.
    sets.iter().map(|s| format!("{}\t{}\t{}\t{}\t{}\n", u8::from(s.on || s.slept), u8::from(s.muted), u8::from(s.held), s.only.as_deref().unwrap_or(""), s.name)).collect()
}

/// A set's power, sound, whether it was held and the channel it's kept to, as they were left: on and silent, to begin with, and held
/// as `stay` says.
fn kept(text: &str, name: &str, stay: bool) -> (bool, bool, bool, Option<String>) {
    text.lines().find_map(|l| match l.split('\t').collect::<Vec<_>>().as_slice() {
        [on, muted, held, only, n] if *n == name => Some((*on != "0", *muted != "0", *held != "0", Some(only.to_string()).filter(|o| live::valid(o)))),
        [on, muted, held, n] if *n == name => Some((*on != "0", *muted != "0", *held != "0", None)),
        [on, muted, n] if *n == name => Some((*on != "0", *muted != "0", stay, None)),
        _ => None,
    }).unwrap_or((true, true, stay, None))
}

impl Set {
    fn new(name: &str, fixed: bool, index: usize, stay: bool) -> Result<Set, String> {
        let screen = Screen::create(name)?;
        let (on, muted, held, only) = kept(&std::fs::read_to_string(state_file()).unwrap_or_default(), name, stay);
        let now = Instant::now();
        Ok(Set {
            name: name.to_string(),
            clicks: screen.clicks(),
            pressed: BUTTONS.map(|b| screen.presses(b)),
            screen,
            fixed,
            hidden: !fixed,
            feed_name: format!("twatch-{}-{index}", std::process::id()),
            on: on || !fixed,
            muted,
            held,
            only,
            wants_video: false,
            evicted: None,
            volume_told: None,
            media_told: None,
            needs_streamlink: false,
            needs_player: false,
            bounce: look::Bounce::default(),
            bouncing: false,
            quiet_since: None,
            slept: false,
            auto_muted: false,
            was_showing: None,
            boot: if on && fixed { BOOT } else { 0 },
            stays: 0,
            small: !on && fixed,
            showing: None,
            pinned: None,
            feed: Feed::Nothing,
            id: 0,
            tries: 0,
            retry_at: now,
            stream: None,
            pace: None,
            rate: 0,
            number: 0,
            shown: 0,
            frame_at: now,
            dozing: false,
            video_off: false,
            still: None,
            still_at: None,
            drawn: None,
            px: Vec::new(),
        })
    }

    /// Lets go of whatever it's getting its picture from.
    fn stop(&mut self) {
        self.id += 1;
        self.feed = Feed::Nothing;
        self.stream = None;
        self.pace = None;
        self.rate = 0;
        self.shown = 0;
        self.video_off = false;
        self.volume_told = None;
        self.media_told = None;
        self.still = None;
        self.still_at = None;
        self.drawn = None;
    }

    /// On to `who`.
    fn tune(&mut self, who: Option<String>) {
        self.stop();
        self.dozing = false;
        self.wants_video = false;
        self.tries = 0;
        self.retry_at = Instant::now();
        self.number = if who.is_some() { NUMBER } else { 0 };
        self.showing = who;
    }

    fn trouble(&mut self, why: &str) {
        crate::log(format!("{}: {why}", self.name));
        let wait = RETRIES[self.tries.min(RETRIES.len() - 1)];
        self.stop();
        self.dozing = false;
        self.feed = Feed::Trouble;
        self.tries += 1;
        self.retry_at = Instant::now() + wait;
    }

    /// Lets go of the stream until the display's back.
    fn doze(&mut self) {
        if !matches!(self.feed, Feed::Nothing) {
            self.dozing |= self.has_picture();
            self.stop();
        }
    }

    /// Keeps its player's picture going, or not, as the display is. Having none to show while it's dark isn't a stream in trouble,
    /// and neither are the few seconds it takes to get going again.
    fn follow_display(&mut self, dark: bool, now: Instant) {
        let Feed::Playing(player) = &mut self.feed else { return };
        let changed = self.video_off != dark;
        if changed {
            player.tell(&format!("video {}", u8::from(!dark)));
            self.video_off = dark;
        }
        if changed || dark {
            self.frame_at = now;
        }
    }

    fn power(&mut self) {
        self.on = !self.on;
        self.small = false;
        self.stays = crate::tv::STAYS_OFF;
        self.drawn = None;
        if self.on {
            self.boot = BOOT;
        } else {
            self.boot = 0;
            self.pinned = None;
            self.tune(None);
        }
    }

    fn scene(&self, idle: &'static str) -> Scene {
        match &self.feed {
            _ if !self.on && self.small => Scene::PutAway(self.slept),
            _ if !self.on => Scene::Off,
            Feed::Stills if self.still.is_some() => Scene::Still(self.evicted),
            _ if self.evicted.is_some() => Scene::Evicted(self.evicted.unwrap_or_default()),
            Feed::Nothing | Feed::Trouble if self.needs_player => Scene::NoPlayer,
            Feed::Nothing | Feed::Trouble if self.needs_streamlink => Scene::NoStreamlink,
            Feed::Nothing if self.only.is_some() && self.showing.is_none() => Scene::Waiting,
            Feed::Trouble => Scene::NoSignal,
            Feed::Asking => Scene::Standby("TUNING"),
            Feed::Playing(_) => Scene::Standby("TUNING"),
            Feed::Stills => Scene::Standby("TUNING"),
            Feed::Nothing => Scene::Standby(if self.showing.is_some() { "TUNING" } else { idle }),
        }
    }

    /// Whether a picture of the stream is up.
    fn has_picture(&self) -> bool {
        match self.feed {
            Feed::Playing(_) => self.shown != 0,
            Feed::Stills => self.still.is_some(),
            _ => false,
        }
    }

    /// Whether it goes by your favorites, as opposed to staying where it is.
    fn follows(&self) -> bool {
        !self.held && self.pinned.is_none() && self.only.is_none()
    }

    fn lowered(&self) -> bool {
        self.pace.as_ref().is_some_and(|p| p.lowered() || p.below(self.rate, Instant::now()))
    }

    /// Draws the next frame at `size`, if there's anything new. `seen`: something's showing it.
    fn draw(&mut self, dt: i64, size: (u32, u32), idle: &'static str, seen: bool) {
        let (w, h) = size;
        // Its second of fame only counts once somebody can see it.
        let booting = self.on && self.boot > 0;
        if booting && seen {
            self.boot = (self.boot - dt).max(0);
        }
        if !self.on && !self.small {
            self.stays -= dt;
            if self.stays <= 0 {
                self.small = true;
                self.drawn = None;
            }
        }
        let was_up = self.number > 0;
        if !booting {
            self.number -= dt;
        }
        self.bouncing = false;
        let mut px = std::mem::take(&mut self.px);
        px.resize((w * h * 4) as usize, 0);
        let mut fresh = false;
        if let Feed::Playing(player) = &mut self.feed
            && let Some(screen) = player.screen()
        {
            screen.want(w, h);
            if let Some(n) = screen.latest(self.shown, w, h, &mut px) {
                // The channel's panel goes up with the first picture, not while there's nothing to see.
                if self.shown == 0 {
                    self.number = NUMBER;
                }
                self.shown = n;
                self.frame_at = Instant::now();
                fresh = true;
            }
        }
        if self.dozing {
            if !self.has_picture() {
                self.px = px;
                return;
            }
            self.dozing = false;
        }
        let handle = self.showing.clone().unwrap_or_default();
        let mut c = Canvas { w: w as usize, h: h as usize, px: &mut px };
        let send = if booting {
            look::boot(&mut c, self.boot, BOOT);
            self.drawn = None;
            true
        } else if matches!(self.feed, Feed::Playing(_)) && self.shown != 0 {
            if fresh {
                self.dress(&mut c, &handle);
            }
            fresh
        } else {
            let scene = self.scene(idle);
            // Idle, with nobody to show (not on its way to someone): its little logo goes round, so every frame is new. Left that way,
            // nobody touching it, it says it's about to go to standby, then does.
            self.bouncing = matches!(scene, Scene::Standby(word) if word != "TUNING");
            let quiet = if self.bouncing { self.quiet_since.get_or_insert_with(Instant::now).elapsed() } else { Duration::ZERO };
            if !self.bouncing {
                self.quiet_since = None;
            }
            let (sleeps_after, warns_for) = sleep_times();
            let warn = quiet + warns_for >= sleeps_after;
            if quiet >= sleeps_after {
                crate::log(format!("{}: nobody to show for {} s, so it's gone to standby", self.name, sleeps_after.as_secs()));
                self.slept = true;
                self.quiet_since = None;
                self.power();
                self.bouncing = false;
            }
            // What's up changes only when one of these does.
            let key = (scene.clone(), if self.number > 0 { handle.clone() } else { String::new() }, w, h);
            if self.drawn.as_ref() == Some(&key) && !(was_up && self.number <= 0) && !self.bouncing {
                false
            } else {
                self.drawn = Some(key);
                match scene {
                    Scene::Off => c.fill([0, 0, 0]),
                    Scene::PutAway(standby) => crate::tv::put_away(&mut c, true, standby),
                    Scene::NoSignal => look::no_signal(&mut c),
                    Scene::NoStreamlink => look::no_streamlink(&mut c),
                    Scene::NoPlayer => look::no_player(&mut c),
                    Scene::Standby(word) if self.bouncing => look::idle(&mut c, word, &mut self.bounce, dt, warn),
                    Scene::Standby(word) => look::standby(&mut c, word),
                    Scene::Waiting => look::waiting(&mut c, self.only.as_deref().unwrap_or_default()),
                    Scene::Evicted(why) => {
                        c.fill([0, 0, 0]);
                        crate::tv::evicted(&mut c, why);
                    }
                    Scene::Still(why) => {
                        if let Some((rgb, width, height)) = &self.still {
                            look::still(&mut c, rgb, *width, *height);
                        }
                        self.dress(&mut c, &handle);
                        if let Some(why) = why {
                            crate::tv::evicted(&mut c, why);
                        }
                    }
                }
                true
            }
        };
        if send {
            self.screen.publish(w, h, &px);
        }
        self.px = px;
    }

    /// What goes over a picture.
    fn dress(&self, c: &mut Canvas, handle: &str) {
        if self.number > 0 {
            look::channel(c, handle);
        }
        if self.lowered() {
            look::lowered(c);
        }
    }

    /// The quality line for the remote.
    fn quality(&self, setup: &Setup) -> String {
        if setup.battery {
            return "On battery, so pictures only".into();
        }
        if setup.metered {
            return "The connection is metered, so pictures only".into();
        }
        if let Some(why) = self.evicted {
            let short = if why.starts_with('V') { "VRAM" } else { "RAM" };
            let then = if setup.pictures == Pictures::Always { "It'll stay on pictures" } else { "Video comes back when there's room" };
            return format!("Evicted, as {short} is low. {then}");
        }
        if setup.pictures == Pictures::Always {
            return String::new();
        }
        if setup.shows(self.wants_video) == Shows::Pictures {
            return "Click the TwaTV for video".into();
        }
        let Some(pace) = &self.pace else { return String::new() };
        let now = pace.playing(self.rate).map_or_else(|| "Live".to_string(), Tier::name);
        match pace.why {
            Some(Why::Busy) => format!("{now}, lowered: this PC is busy"),
            Some(Why::Stalling) => format!("{now}, lowered: the stream keeps stalling"),
            None if pace.below(self.rate, Instant::now()) => format!("{now}, lowered: the network can't keep up"),
            None => now,
        }
    }
}

/// Whether Twitch has a preview of `handle` right now, which is whether it's live.
fn look(handle: &str) -> Result<bool, String> {
    if let Ok(list) = std::env::var("PMS_TWATCH_LIVE") {
        return Ok(list.split(',').any(|h| h == handle));
    }
    let url = format!("{PREVIEWS}live_user_{handle}-320x180.jpg");
    let response = crate::wads::agent(crate::net::Use::Live, Duration::from_secs(10), 0).head(&url).call().map_err(|e| e.to_string())?;
    match response.status().as_u16() {
        200 => Ok(true),
        301 | 302 | 404 => Ok(false),
        other => Err(format!("{url}: {other}")),
    }
}

/// Fetches `handle`'s preview: None if it's not live.
fn still(handle: &str) -> Result<Option<Picture>, String> {
    let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let url = format!("{PREVIEWS}live_user_{handle}-320x180.jpg?t={time}");
    let response = crate::wads::agent(crate::net::Use::Live, Duration::from_secs(20), 0).get(&url).call().map_err(|e| e.to_string())?;
    if response.status().as_u16() != 200 {
        return Ok(None);
    }
    let bytes = response.into_body().read_to_vec().map_err(|e| e.to_string())?;
    let mut decoder = jpeg_decoder::Decoder::new(bytes.as_slice());
    let pixels = decoder.decode().map_err(|e| e.to_string())?;
    let info = decoder.info().ok_or("no size")?;
    let rgb = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels,
        jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|v| [*v; 3]).collect(),
        other => return Err(format!("a picture in {other:?}")),
    };
    Ok(Some((rgb, info.width as usize, info.height as usize)))
}

/// Asks about `handle`, and reads its renditions.
fn find(handle: &str) -> (Answer, Vec<Tier>) {
    let asked = match std::env::var("PMS_TWATCH_SOURCE") {
        Ok(master) => Answer::Live(Stream { master, title: format!("{handle}, from a test stream"), category: "Testing".into() }),
        Err(_) => streamlink::ask(handle),
    };
    let tiers = match &asked {
        Answer::Live(stream) => crate::wads::agent(crate::net::Use::Video, Duration::from_secs(15), 3).get(&stream.master).call().ok().and_then(|r| r.into_body().read_to_string().ok()).map_or_else(Vec::new, |text| player::tiers(&text)),
        _ => Vec::new(),
    };
    (asked, tiers)
}

/// Opens `url` in the browser. Only ever a page of ours or a valid channel's.
fn open(url: &str) {
    let _ = std::process::Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", url]).spawn();
}

struct Running {
    sets: Vec<Set>,
    watches: Vec<Watch>,
    events: Sender<Event>,
    looks: Sender<String>,
    /// Whether Streamlink is here, which version, and when that was last looked at.
    streamlink: Option<(bool, Instant)>,
    /// Whether the player, pms-twatch.exe, is here, as of that look.
    player_here: bool,
    /// The proxy the video goes through, as players were last started with it.
    video_proxy: Option<String>,
    /// When the last look at who's live went out.
    last_look: Option<Instant>,
    streamlink_version: Option<String>,
    /// What sets do by default, as it last was: changing it applies to all of them.
    stay: Option<bool>,
    /// When a newer Streamlink was last asked about, and whether that's still going on.
    newer: Option<Instant>,
    looking_for_newer: bool,
    /// The most a stream may go up to, as it last was: changing it applies to what's playing.
    best: Option<Quality>,
    /// The buttons beside the sets.
    cells: Vec<Cell>,
    /// How many favorites are live with no set to show them, and until when the button says so.
    left_out: usize,
    announced_until: Option<Instant>,
}

/// A button beside the sets.
struct Cell {
    name: &'static str,
    screen: Screen,
    clicks: u32,
    /// How many it said, whether it said so in words, the size, and how far it had faded.
    drawn: Option<(usize, bool, u32, u32, u8)>,
    px: Vec<u8>,
    /// When it was last put away, for fading.
    small_since: Option<Instant>,
}

impl Running {
    /// Makes the sets and favorites what they should be.
    fn sync(&mut self, setup: &Setup) {
        let names: Vec<&String> = setup.sets.iter().map(|s| &s.0).collect();
        self.sets.retain(|s| names.contains(&&s.name));
        if self.stay.replace(setup.stay).is_some_and(|was| was != setup.stay) {
            for set in &mut self.sets {
                (set.held, set.pinned) = (setup.stay, None);
            }
            self.save();
        }
        for (index, (name, fixed)) in setup.sets.iter().enumerate() {
            if !self.sets.iter().any(|s| s.name == *name) {
                match Set::new(name, *fixed, index, setup.stay) {
                    Ok(set) => self.sets.insert(index.min(self.sets.len()), set),
                    Err(e) => crate::log(format!("Twatch TV `{name}`: {e}")),
                }
            }
        }
        if self.watches.iter().map(|w| &w.handle).ne(setup.favorites.iter()) {
            let old = std::mem::take(&mut self.watches);
            self.watches = setup.favorites.iter().map(|h| old.iter().find(|w| w.handle == *h).cloned().unwrap_or_else(|| Watch::new(h))).collect();
            for set in &mut self.sets {
                set.pinned = set.pinned.take().filter(|p| setup.favorites.contains(p));
                set.only = set.only.take().filter(|p| setup.favorites.contains(p));
            }
        }
        if self.best.replace(setup.best).is_some_and(|was| was != setup.best) {
            let now = Instant::now();
            for set in &mut self.sets {
                if let (Some(pace), Feed::Playing(player)) = (&mut set.pace, &mut set.feed) {
                    pace.aim_for(setup.best, now);
                    player.tell(&format!("cap {}", pace.limit()));
                }
            }
        }
        if self.cells.is_empty() {
            for name in [MORE_CELL, AUTO_CELL] {
                match Screen::create(name) {
                    Ok(screen) => self.cells.push(Cell { name, clicks: screen.clicks(), screen, drawn: None, px: Vec::new(), small_since: None }),
                    Err(e) => crate::log(format!("Twatch TV `{name}`: {e}")),
                }
            }
        }
    }

    fn live(&self) -> Vec<String> {
        self.watches.iter().filter(|w| w.live).map(|w| w.handle.clone()).collect()
    }

    fn save(&self) {
        let _ = std::fs::write(state_file(), kept_text(&self.sets));
    }

    /// Streamlink couldn't open a channel that Twitch says is live: most likely Twitch changed something, and the fix is a newer
    /// Streamlink, if there's one yet. This only finds out: it's new code, so getting it is up to you.
    fn look_for_newer_streamlink(&mut self) {
        let now = Instant::now();
        if self.looking_for_newer || self.newer.is_some_and(|at| now.duration_since(at) < NEWER_STREAMLINK) {
            return;
        }
        (self.looking_for_newer, self.newer) = (true, Some(now));
        let events = self.events.clone();
        std::thread::spawn(move || {
            let _ = events.send(Event::Newer(streamlink::newer()));
        });
    }

    fn event(&mut self, event: Event, setup: &Setup) {
        match event {
            Event::Looked { handle, live } => {
                if let Some(watch) = self.watches.iter_mut().find(|w| w.handle == handle) {
                    watch.asked = false;
                    // After its first look, each starts somewhere in an interval of its own, so that looks spread out from the start.
                    watch.extra = live::extra(if watch.looked.is_none() { setup.check } else { setup.check_extra });
                    watch.looked = Some(Instant::now());
                    match live {
                        Ok(live) => {
                            let news = live && !watch.live && watch.known && watch.told.is_none_or(|t| t.elapsed() >= TOLD_AGAIN);
                            if news {
                                watch.told = Some(Instant::now());
                                if setup.notify {
                                    let handle = watch.handle.clone();
                                    ask(|a| a.went_live.push(handle));
                                }
                            }
                            watch.live = live;
                            watch.known = true;
                            watch.failures = 0;
                        }
                        Err(e) => {
                            crate::log(format!("Twatch TV: couldn't tell if {handle} is live: {e}"));
                            watch.failures += 1;
                        }
                    }
                }
            }
            Event::Found { set, id, asked, tiers } => {
                let events = self.events.clone();
                let Some(s) = self.sets.get_mut(set).filter(|s| s.id == id) else { return };
                match asked {
                    Answer::Live(stream) => {
                        let pace = Pace::new(tiers, setup.best, Instant::now());
                        match Player::start(&stream.master, &s.feed_name, &pace, set, id, events) {
                            Ok(mut player) => {
                                player.tell(&format!("mute {}", u8::from(s.muted)));
                                player.tell(&format!("volume {}", setup.volume));
                                let who = s.showing.clone().unwrap_or_default();
                                let what = if stream.title.is_empty() { who } else { format!("{who}: {}", stream.title) };
                                player.tell(&format!("title {}", what.replace(['\r', '\n'], " ")));
                                s.volume_told = Some(setup.volume);
                                s.pace = Some(pace);
                                s.stream = Some(stream);
                                s.feed = Feed::Playing(Box::new(player));
                                s.frame_at = Instant::now();
                            }
                            Err(e) => s.trouble(&e),
                        }
                    }
                    Answer::Offline => {
                        let gone = s.showing.take();
                        s.stop();
                        if let Some(watch) = self.watches.iter_mut().find(|w| Some(&w.handle) == gone.as_ref()) {
                            watch.live = false;
                            watch.looked = Some(Instant::now());
                        }
                        self.look_for_newer_streamlink();
                    }
                    Answer::Failed(why) => {
                        s.trouble(&why);
                        self.look_for_newer_streamlink();
                    }
                }
            }
            Event::Newer(found) => {
                self.looking_for_newer = false;
                match found {
                    Ok(Some(version)) => {
                        crate::log(format!("Twatch TV: Streamlink {version} is out, and it's yours to fetch"));
                        if let Ok(mut newer) = NEWER.lock() {
                            *newer = Some(version);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => crate::log(format!("Twatch TV: a newer Streamlink: {e}")),
                }
            }
            Event::Still { set, id, picture } => {
                let Some(s) = self.sets.get_mut(set).filter(|s| s.id == id) else { return };
                match picture {
                    Ok(Some(picture)) => {
                        s.still = Some(picture);
                        s.tries = 0;
                        s.drawn = None;
                    }
                    Ok(None) => {
                        let gone = s.showing.take();
                        s.stop();
                        if let Some(watch) = self.watches.iter_mut().find(|w| Some(&w.handle) == gone.as_ref()) {
                            watch.live = false;
                        }
                    }
                    Err(e) => s.trouble(&e),
                }
            }
            Event::Player { set, id, said } => {
                let Some(s) = self.sets.get_mut(set).filter(|s| s.id == id) else { return };
                match said {
                    Said::Playing => s.tries = 0,
                    Said::Buffering if s.shown != 0 => {
                        if let Some(pace) = &mut s.pace {
                            pace.stalled(Instant::now());
                        }
                    }
                    Said::Buffering => {}
                    Said::Rate(rate) => s.rate = rate,
                    Said::Note(note) => crate::log(format!("{}: {note}", s.name)),
                    Said::Failed(why) => s.trouble(&format!("the player: {why}")),
                    Said::Ended => {
                        let who = s.showing.clone();
                        s.trouble("the stream ended");
                        // Most likely the channel did: find out now.
                        if let Some(watch) = self.watches.iter_mut().find(|w| Some(&w.handle) == who.as_ref()) {
                            watch.looked = None;
                        }
                    }
                }
            }
        }
    }

    /// What the remotes and the picture have asked for.
    fn input(&mut self, setup: &Setup) {
        let live = self.live();
        let mut changed = false;
        let mut sound_on = None;
        for (i, s) in self.sets.iter_mut().enumerate() {
            for (n, button) in BUTTONS.iter().enumerate() {
                let now = s.screen.presses(*button);
                if now == s.pressed[n] {
                    continue;
                }
                s.pressed[n] = now;
                changed = true;
                // Somebody's here: it isn't going anywhere for now.
                (s.quiet_since, s.slept) = (None, false);
                match button {
                    Button::TvPower => s.power(),
                    Button::ChannelUp | Button::ChannelDown if s.on && s.only.is_none() => {
                        let by = if *button == Button::ChannelUp { 1 } else { -1 };
                        if let Some(next) = live::step(&live, s.showing.as_deref(), by).filter(|n| Some(n) != s.showing.as_ref()) {
                            s.pinned = Some(next.clone());
                            s.tune(Some(next));
                        }
                    }
                    // Each of these is this set's own, and a pick goes with it.
                    Button::Follow => (s.held, s.pinned, s.only) = (false, None, None),
                    Button::Stay => (s.held, s.pinned, s.only) = (true, None, None),
                    // With nothing on, it starts with the best favorite there is, which its list can then change.
                    Button::Only => {
                        let first = self.watches.first().map(|w| w.handle.clone());
                        (s.held, s.pinned, s.only) = (false, None, s.only.clone().or_else(|| s.showing.clone()).or(first));
                    }
                    Button::QualityUp => ask(|a| a.quality += 1),
                    Button::QualityDown => ask(|a| a.quality -= 1),
                    Button::VolumeUp => ask(|a| a.volume += 1),
                    Button::VolumeDown => ask(|a| a.volume -= 1),
                    Button::AddFavorite => crate::tray::open_settings_on("twatch"),
                    Button::NukeTv => {
                        let (name, fixed) = (s.name.clone(), s.fixed);
                        ask(|a| a.removed.push((name, fixed)));
                    }
                    Button::Mute => {
                        s.muted = !s.muted;
                        s.auto_muted = false;
                        if let Feed::Playing(player) = &mut s.feed {
                            player.tell(&format!("mute {}", u8::from(s.muted)));
                        }
                        if !s.muted {
                            sound_on = Some(i);
                            // A picture has no sound, so that's asking for video.
                            s.wants_video = true;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(pick) = s.screen.take_tune() {
                changed = true;
                (s.quiet_since, s.slept) = (None, false);
                let handle = self.watches.get(pick as usize - 1).map(|w| w.handle.clone());
                if s.only.is_some() {
                    // Kept to a channel, picking changes which one, live or not.
                    s.only = handle.or(s.only.take());
                } else if let Some(handle) = handle.filter(|_| s.on) {
                    if !live.contains(&handle) {
                        // Somebody who isn't live: the set waits for them, and is theirs once they are.
                        (s.held, s.pinned, s.only) = (false, None, Some(handle));
                    } else if Some(&handle) != s.showing.as_ref() {
                        s.pinned = Some(handle.clone());
                        s.tune(Some(handle));
                    }
                }
            }
            if s.screen.clicks() != s.clicks {
                s.clicks = s.screen.clicks();
                changed = true;
                (s.quiet_since, s.slept) = (None, false);
                if !s.on {
                    s.power();
                } else if s.needs_streamlink || s.needs_player {
                    crate::tray::open_settings_on("twatch");
                } else if s.showing.is_some() && !s.wants_video && setup.pictures == Pictures::Initial && setup.shows(false) == Shows::Pictures && setup.shows(true) == Shows::Video {
                    s.wants_video = true;
                } else if let Some(handle) = &s.showing {
                    open(&format!("{CHANNEL_PAGE}{handle}"));
                }
            }
        }
        for cell in &mut self.cells {
            if cell.screen.clicks() != cell.clicks {
                cell.clicks = cell.screen.clicks();
                if cell.name == MORE_CELL {
                    ask(|a| a.add_set = true);
                } else {
                    crate::tray::open_settings_on("twatch");
                }
            }
        }
        // One sound at a time.
        if let Some(on) = sound_on {
            for (_, s) in self.sets.iter_mut().enumerate().filter(|(i, s)| *i != on && !s.muted) {
                s.muted = true;
                if let Feed::Playing(player) = &mut s.feed {
                    player.tell("mute 1");
                }
            }
        }
        if changed {
            self.save();
        }
    }

    /// Whether Streamlink is here: looked at every few seconds, as the settings can fetch it or remove it at any time. The player
    /// is looked for along with it, as a reinstall puts it back while we run.
    fn has_streamlink(&mut self, now: Instant) -> bool {
        match self.streamlink {
            Some((have, at)) if now.duration_since(at) < CHECK_STREAMLINK => have,
            _ => {
                self.player_here = crate::parts::here(crate::parts::TWATCH);
                let testing = std::env::var_os("PMS_TWATCH_SOURCE").is_some();
                self.streamlink_version = if testing { Some("(a test stream)".into()) } else { streamlink::version() };
                let have = testing || streamlink::exe().is_some();
                if have && !testing && self.streamlink.is_none_or(|(had, _)| !had) {
                    streamlink_is_here();
                }
                self.streamlink = Some((have, now));
                have
            }
        }
    }

    /// What a set with nobody to show says.
    fn idle(&mut self, setup: &Setup, now: Instant) -> &'static str {
        if setup.favorites.is_empty() {
            "NO FAVORITES"
        } else if setup.shows(true) == Shows::Video && !self.has_streamlink(now) {
            "NO STREAMLINK"
        } else {
            "NOBODY LIVE"
        }
    }

    /// Once a second: who to look at, who goes where, and how each stream is doing.
    fn think(&mut self, setup: &Setup, now: Instant) {
        // The video's to go another way: what's playing starts again, that way.
        let proxy = crate::net::proxy_for(crate::net::Use::Video);
        if proxy != self.video_proxy {
            crate::log(format!("Twatch TV: the video goes {} now", if proxy.is_some() { "through the proxy" } else { "without a proxy" }));
            self.video_proxy = proxy;
            for s in &mut self.sets {
                if matches!(s.feed, Feed::Playing(_) | Feed::Asking) {
                    s.dozing |= s.has_picture();
                    s.stop();
                }
            }
        }
        if setup.resting {
            for s in &mut self.sets {
                if !matches!(s.feed, Feed::Nothing) {
                    s.stop();
                }
            }
            self.left_out = 0;
        } else {
            self.look_around(setup, now);
            let live = self.live();
            // A set on standby counts as on: it's to come on again for somebody.
            let slots: Vec<Slot> = self.sets.iter().map(|s| Slot { on: s.on || s.slept, showing: s.showing.clone(), pinned: s.pinned.clone().or_else(|| s.showing.clone().filter(|_| s.held)), only: s.only.clone() }).collect();
            let who = live::assign(&slots, &live);
            // A set you've turned off is room already, and it's for you to turn it on again.
            let room_for_more = self.sets.len() < MOST_SETS && self.sets.iter().all(|s| s.on);
            let left_out = if room_for_more { live.iter().filter(|h| !who.iter().flatten().any(|w| w == *h)).count() } else { 0 };
            if left_out > self.left_out {
                self.announced_until = Some(now + ANNOUNCED);
            }
            self.left_out = left_out;
            for (i, (s, who)) in self.sets.iter_mut().zip(who).enumerate() {
                s.pinned = s.pinned.take().filter(|p| live.contains(p));
                if s.slept && !s.on && who.is_some() {
                    crate::log(format!("{}: a favorite's live, so it's back from standby", s.name));
                    s.slept = false;
                    s.power();
                }
                // Somebody new, that you didn't ask for, comes in quietly: a stranger shouldn't start talking out of nowhere. Coming
                // back to what it was showing, starting up, or a set kept to one channel finding it live isn't that.
                let somebody_else = who.is_some() && who != s.showing && s.was_showing.is_some() && who != s.was_showing && s.only.is_none();
                if somebody_else && !s.muted {
                    s.muted = true;
                    s.auto_muted = true;
                    if let Feed::Playing(player) = &mut s.feed {
                        player.tell("mute 1");
                    }
                }
                if s.showing != who {
                    s.tune(who);
                }
                if s.showing.is_some() {
                    s.was_showing = s.showing.clone();
                }
                // A set that comes and goes, turned off, is only off until it's no longer called for.
                if !s.fixed && !s.on && s.small && live.len() <= i {
                    s.on = true;
                }
            }
            let has_streamlink = self.has_streamlink(now);
            let has_player = self.player_here;
            for (i, s) in self.sets.iter_mut().enumerate() {
                // Nobody to come back to: its old picture shouldn't stay up.
                s.dozing &= s.showing.is_some();
                s.evicted = setup.pressure.twatch().filter(|_| s.showing.is_some());
                let shows = setup.shows(s.wants_video);
                // Video wanted, and what it takes missing: the player first (it's ours, and a reinstall away), then Streamlink.
                let wants = s.on && s.showing.is_some() && shows == Shows::Video;
                let (needs_player, needs) = (wants && !has_player, wants && has_player && !has_streamlink);
                if needs_player && !s.needs_player {
                    crate::log(format!("{}: {} is live, but {} isn't there to play them", s.name, s.showing.as_deref().unwrap_or_default(), crate::parts::TWATCH));
                    crate::parts::found_missing(crate::parts::TWATCH);
                }
                if needs && !s.needs_streamlink {
                    crate::log(format!("{}: {} is live, but there's no Streamlink to play them", s.name, s.showing.as_deref().unwrap_or_default()));
                    found_no_streamlink();
                }
                if needs != s.needs_streamlink || needs_player != s.needs_player {
                    s.drawn = None;
                }
                (s.needs_streamlink, s.needs_player) = (needs, needs_player);
                if shows == Shows::Nothing {
                    if !matches!(s.feed, Feed::Nothing) {
                        s.stop();
                    }
                    s.dozing = false;
                } else if setup.dozes(s.muted, s.wants_video) {
                    s.doze();
                } else {
                    feed(s, i, setup, shows == Shows::Pictures, has_streamlink && has_player, &self.events, now);
                    s.follow_display(setup.dark, now);
                    if let Feed::Playing(player) = &mut s.feed {
                        if s.volume_told != Some(setup.volume) {
                            player.tell(&format!("volume {}", setup.volume));
                            s.volume_told = Some(setup.volume);
                        }
                        let media = setup.media.shows(s.muted);
                        if s.media_told != Some(media) {
                            player.tell(&format!("media {}", u8::from(media)));
                            s.media_told = Some(media);
                        }
                    }
                }
            }
        }
        self.tell(setup);
        self.tell_cells(setup, now);
    }

    /// Says which buttons there are and what they do.
    fn tell_cells(&mut self, setup: &Setup, now: Instant) {
        let shown = [(self.left_out > 0).then_some(MORE_CELL), (setup.more > 0 && !setup.resting).then_some(AUTO_CELL)];
        if let Ok(mut cells) = CELLS.lock() {
            *cells = shown.into_iter().flatten().collect();
        }
        let announcing = self.announcing(now);
        let left_out: Vec<String> = {
            let on_a_set: Vec<&String> = self.sets.iter().filter_map(|s| s.showing.as_ref()).collect();
            self.live().into_iter().filter(|h| !on_a_set.contains(&h)).collect()
        };
        let left_out = left_out.join(", ");
        for cell in &self.cells {
            let (small, what) = if cell.name == MORE_CELL {
                let them = if self.left_out == 1 { "more favorite is" } else { "more favorites are" };
                (!announcing, format!("{} {them} live: {left_out}. Select to add a TwaTV for good", self.left_out))
            } else {
                let them = if setup.more == 1 { "one more TwaTV comes" } else { "more TwaTVs come" };
                (true, format!("When more favorites go live, {them} by themselves, up to {}. Select to change that", setup.more))
            };
            cell.screen.set_info(&format!("how\t{what}\n"));
            cell.screen.set_status(Status { tv_on: true, collapsed: small, modern: true, note: true, ..Status::default() });
        }
    }

    /// Whether a button's plus is fading: it wants frames for that, however quiet the sets are.
    fn fading(&self, now: Instant) -> bool {
        self.cells.iter().any(|c| c.small_since.is_some_and(|since| now.duration_since(since) < PLUS_FADE))
    }

    fn announcing(&self, now: Instant) -> bool {
        self.announced_until.is_some_and(|until| now < until)
    }

    /// Draws the buttons, when there's something new to draw.
    fn draw_cells(&mut self, now: Instant) {
        let (announcing, left_out) = (self.announcing(now), self.left_out);
        for cell in &mut self.cells {
            let Some((w, h)) = cell.screen.wanted() else { continue };
            let more = cell.name == MORE_CELL;
            // Put away (narrower than it's tall, once the taskbar has made it so), the plus fades to an outline.
            let small = more && !announcing && w < h;
            let since = if small { *cell.small_since.get_or_insert(now) } else { now };
            if !small {
                cell.small_since = None;
            }
            let step = (now.duration_since(since).as_secs_f32() / PLUS_FADE.as_secs_f32() * PLUS_FADE_STEPS).min(PLUS_FADE_STEPS) as u8;
            let key = (if more { left_out } else { 0 }, more && announcing, w, h, step);
            if cell.drawn == Some(key) {
                continue;
            }
            cell.drawn = Some(key);
            cell.px.resize((w * h * 4) as usize, 0);
            let mut c = Canvas { w: w as usize, h: h as usize, px: &mut cell.px };
            if more {
                look::more(&mut c, announcing.then_some(left_out), f32::from(step) / PLUS_FADE_STEPS);
            } else {
                look::automatic(&mut c);
            }
            cell.screen.publish(w, h, &cell.px);
        }
    }

    /// One look, if one's due and it's been long enough since the last (live.rs says which, and when).
    fn look_around(&mut self, setup: &Setup, now: Instant) {
        if let Some(i) = live::next(&self.watches, setup.battery, setup.check, self.last_look, now, live::random()) {
            self.watches[i].asked = true;
            self.last_look = Some(now);
            let _ = self.looks.send(self.watches[i].handle.clone());
        }
    }

    /// Says how things stand, to the remotes and the meters.
    fn tell(&mut self, setup: &Setup) {
        let list: String = self.watches.iter().map(|w| format!("fav\t{}\t{}\n", w.handle, u8::from(w.live))).collect();
        let (favorites, live) = (setup.favorites.as_slice(), self.live());
        let checked = self.watches.iter().filter_map(|w| w.looked).max().map(|t| t.elapsed().as_secs());
        let rank = |handle: &str| favorites.iter().position(|f| f == handle);
        let spare_set = self.sets.iter().any(|s| s.on && s.showing.is_none());
        let lowest = self.sets.iter().filter(|s| s.on && s.follows()).filter_map(|s| s.showing.as_deref().and_then(rank)).max();
        let shown: Vec<String> = self.sets.iter().filter_map(|s| s.showing.clone()).collect();
        let no_streamlink = self.streamlink.is_some_and(|(have, _)| !have);
        for s in &mut self.sets {
            let elsewhere: Vec<&str> = shown.iter().map(String::as_str).filter(|h| Some(*h) != s.showing.as_deref()).collect();
            let standing = live::Standing {
                favorites,
                live: &live,
                showing: s.showing.as_deref(),
                held: s.held,
                picked: s.pinned.is_some(),
                only: s.only.as_deref(),
                elsewhere: &elsewhere,
                spare_set,
                gives_way: s.showing.as_deref().and_then(rank) == lowest,
            };
            let plan = live::plan(&standing);
            let plan = if s.auto_muted && s.muted && s.showing.is_some() { format!("{plan}. Muted, as it changed channel by itself") } else { plan };
            let (title, category) = s.stream.as_ref().map_or((String::new(), String::new()), |t| (t.title.clone(), t.category.clone()));
            let waiting = s.only.as_ref().filter(|_| s.on && s.showing.is_none()).map(|only| format!("Waiting for {only}"));
            let state = match &s.feed {
                _ if waiting.is_some() => waiting.as_deref().unwrap_or_default(),
                _ if !s.on && s.slept => "Standby: it comes back on when a favorite goes live",
                _ if !s.on => "Off",
                // What's missing is said below, with the way to get it.
                _ if s.needs_streamlink || s.needs_player => "Live, not playing",
                Feed::Trouble => "No signal",
                Feed::Asking => "Tuning: asking Streamlink for the stream",
                Feed::Playing(_) | Feed::Stills => {
                    if s.has_picture() {
                        "Live"
                    } else {
                        "Tuning"
                    }
                }
                Feed::Nothing if setup.favorites.is_empty() => "No favorites yet",
                Feed::Nothing if s.showing.is_some() => "Tuning",
                Feed::Nothing => "Nobody's live",
            };
            // Where the picture comes from, while there's one, and what's missing, while video is wanted and can't be had.
            let via = match &s.feed {
                Feed::Playing(_) if s.has_picture() => format!("Live video through Streamlink {}", self.streamlink_version.as_deref().unwrap_or_default()),
                Feed::Stills if s.has_picture() => "A new picture every minute, from Twitch's previews".to_string(),
                _ => String::new(),
            };
            let warn = if s.needs_player {
                format!("{} is live, but the player, {}, is missing: installing PimpMyShell again puts it back", s.showing.as_deref().unwrap_or_default(), crate::parts::TWATCH)
            } else if !no_streamlink || setup.favorites.is_empty() || setup.shows(true) != Shows::Video {
                String::new()
            } else if s.needs_streamlink {
                format!("{} is live, but live video needs Streamlink, which isn't installed", s.showing.as_deref().unwrap_or_default())
            } else {
                "Live video needs Streamlink, which isn't installed".to_string()
            };
            let clean = |t: &str| t.replace(['\t', '\n', '\r'], " ");
            let now = format!("now\t{}\t{}\t{}\t{}\t{}\n", s.showing.as_deref().or(s.only.as_deref()).unwrap_or(""), clean(&title), clean(&category), s.quality(setup), state);
            let seen = checked.map_or(String::new(), |ago| format!("seen\t{ago}\n"));
            s.screen.set_info(&format!("{now}how\t{plan}\nvia\t{via}\nwarn\t{warn}\ntop\t{}\nvol\t{}%\n{seen}{list}", setup.top, setup.volume));
            // A set's shuffle light is its hold: the one says it'll wander off, and the other that it won't.
            s.screen.set_status(Status { tv_on: s.on, collapsed: s.small && !s.on, modern: true, muted: s.muted, shuffle: !s.follows(), only: s.only.is_some(), ..Status::default() });
        }
        for s in &mut self.sets {
            let hide = !s.fixed && !s.has_picture() && !s.dozing && s.only.is_none() && s.evicted.is_none() && (s.on || s.small);
            // One that comes and goes starts up when it comes.
            if s.hidden && !hide && s.on {
                s.boot = BOOT;
            }
            s.hidden = hide;
        }
        let hidden: Vec<String> = self.sets.iter().filter(|s| s.hidden).map(|s| s.name.clone()).collect();
        if let Ok(mut h) = HIDDEN.lock() {
            *h = hidden;
        }
    }

    /// Draws every set.
    fn draw(&mut self, dt: i64, idle: &'static str) {
        let usual = self.sets.iter().find_map(|s| s.screen.wanted());
        for s in &mut self.sets {
            // A set still out of sight has to have its picture ready for when it's shown.
            let seen = s.screen.wanted().is_some() && !s.hidden;
            if let Some(size) = s.screen.wanted().or(usual) {
                s.draw(dt, size, idle, seen);
            }
            if matches!(s.feed, Feed::Playing(_)) && s.frame_at.elapsed() > NO_FRAMES {
                s.trouble("no frames for a while");
            }
        }
    }
}

/// Starts, or keeps up, what set `s` needs to show who it's showing, in `pictures` or video.
fn feed(s: &mut Set, index: usize, setup: &Setup, pictures: bool, has_streamlink: bool, events: &Sender<Event>, now: Instant) {
    let Some(handle) = s.showing.clone() else { return };
    let ready = now >= s.retry_at && (pictures || has_streamlink);
    let events = events.clone();
    // The power came or went, or you changed your mind: on to the other way of showing it, straight away, with what's up staying up
    // until the other has something to show.
    if matches!((&s.feed, pictures), (Feed::Playing(_) | Feed::Asking, true) | (Feed::Stills, false)) {
        s.dozing |= s.has_picture();
        s.stop();
    }
    match s.feed {
        Feed::Nothing | Feed::Trouble if ready && pictures => {
            s.stop();
            s.feed = Feed::Stills;
        }
        Feed::Nothing | Feed::Trouble if ready => {
            s.stop();
            s.feed = Feed::Asking;
            let id = s.id;
            std::thread::spawn(move || {
                let (asked, tiers) = find(&handle);
                let _ = events.send(Event::Found { set: index, id, asked, tiers });
            });
            return;
        }
        Feed::Playing(ref mut player) => {
            if let Some(pace) = &mut s.pace
                && let Some(limit) = pace.check(setup.busy, now)
            {
                player.tell(&format!("cap {limit}"));
            }
            return;
        }
        _ => {}
    }
    if matches!(s.feed, Feed::Stills) && s.still_at.is_none_or(|t| now.duration_since(t) >= STILL_EVERY) {
        s.still_at = Some(now);
        let id = s.id;
        let handle = s.showing.clone().unwrap_or_default();
        std::thread::spawn(move || {
            let _ = events.send(Event::Still { set: index, id, picture: still(&handle) });
        });
    }
}

fn run(stop: &AtomicBool, setup: &Mutex<Option<Setup>>, events: Sender<Event>, incoming: Receiver<Event>) {
    // One look at a time.
    let (looks, wanted) = channel::<String>();
    let answers = events.clone();
    std::thread::spawn(move || {
        for handle in wanted {
            let live = look(&handle);
            let _ = answers.send(Event::Looked { handle, live });
        }
    });
    let now = Instant::now();
    let mut group = Running { sets: Vec::new(), watches: Vec::new(), events, looks, streamlink: None, player_here: true, last_look: None, video_proxy: crate::net::proxy_for(crate::net::Use::Video), streamlink_version: None, stay: None, newer: None, looking_for_newer: false, best: None, cells: Vec::new(), left_out: 0, announced_until: None };
    let (mut last, mut thought) = (now, now - Duration::from_secs(5));
    while !stop.load(Ordering::Relaxed) {
        let began = Instant::now();
        let Some(setup) = setup.lock().ok().and_then(|s| s.clone()) else { break };
        group.sync(&setup);
        while let Ok(event) = incoming.try_recv() {
            group.event(event, &setup);
        }
        group.input(&setup);
        if began.duration_since(thought) >= Duration::from_secs(1) {
            thought = began;
            group.think(&setup, began);
        }
        let dt = last.elapsed().as_millis().min(250) as i64;
        last = began;
        let idle = group.idle(&setup, began);
        if !setup.resting {
            group.draw(dt, idle);
            group.draw_cells(began);
        }
        let moving = group.sets.iter().any(|s| matches!(s.feed, Feed::Playing(_)) || (s.on && s.boot > 0)) || group.fading(began);
        // A set going round with its logo, and nothing playing: frames enough for that to glide, and no more.
        let bouncing = group.sets.iter().any(|s| s.bouncing);
        let rate = match (moving, setup.busy) {
            (false, _) if bouncing && !setup.battery => crate::tv::BUSY_FPS,
            (false, _) => QUIET_FPS,
            (true, true) => crate::tv::BUSY_FPS,
            (true, false) => crate::tv::FPS,
        };
        std::thread::sleep(Duration::from_secs(1).div_f64(f64::from(rate)).saturating_sub(began.elapsed()));
    }
    if let Ok(mut c) = CELLS.lock() {
        c.clear();
    }
    if let Ok(mut h) = HIDDEN.lock() {
        h.clear();
    }
    group.save();
}

/// The sets, and the thread that looks after them.
#[derive(Default)]
pub struct Group {
    thread: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    setup: Arc<Mutex<Option<Setup>>>,
}

impl Group {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.stop.store(false, Ordering::Relaxed);
    }

    /// Keeps the sets as they should be. Every second or so.
    pub fn update(&mut self, config: &Config, active: bool, battery: bool, busy: bool, dark: bool, pressure: Pressure) {
        let wanted = setup(config, !active, battery, busy, dark);
        let metered = wanted.metered || config.twatch.as_ref().is_none_or(|t| t.metered_pictures) && !wanted.sets.is_empty() && metered();
        let wanted = Setup { pressure, metered, ..wanted };
        if wanted.sets.is_empty() {
            self.stop();
            return;
        }
        if let Ok(mut setup) = self.setup.lock() {
            *setup = Some(wanted);
        }
        if self.thread.is_none() {
            let (stop, setup) = (self.stop.clone(), self.setup.clone());
            let (events, incoming) = channel();
            self.thread = Some(std::thread::spawn(move || run(&stop, &setup, events, incoming)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> Config {
        crate::config::parse(text).unwrap()
    }

    #[test]
    fn in_the_dark_only_a_set_with_something_to_hear_keeps_going() {
        let c = config(SET);
        assert!(setup(&c, false, false, false, true).dozes(true, false));
        assert!(!setup(&c, false, false, false, true).dozes(false, false));
        assert!(!setup(&c, false, false, false, false).dozes(true, false));
        // Pictures have no sound.
        assert!(setup(&c, false, true, false, true).dozes(false, false));
    }

    #[test]
    fn a_set_back_from_the_dark_keeps_its_picture_until_there_is_a_new_one() {
        let mut s = Set::new(&format!("twatch-dozing-{}", std::process::id()), true, 0, false).unwrap();
        s.showing = Some("crab".into());
        s.feed = Feed::Stills;
        s.still = Some((vec![0; 3], 1, 1));
        s.doze();
        assert!(matches!(s.feed, Feed::Nothing) && s.dozing);
        s.draw(16, (2, 2), "NOBODY LIVE", true);
        assert!(s.dozing && s.drawn.is_none(), "nothing's drawn over it");
        s.feed = Feed::Stills;
        s.still = Some((vec![0; 3], 1, 1));
        s.draw(16, (2, 2), "NOBODY LIVE", true);
        assert!(!s.dozing);
        s.dozing = true;
        s.tune(Some("lobster".into()));
        assert!(!s.dozing, "another channel isn't that picture");
    }

    #[test]
    fn a_set_with_nothing_up_has_no_picture_to_keep() {
        let mut s = Set::new(&format!("twatch-no-picture-{}", std::process::id()), true, 0, false).unwrap();
        s.feed = Feed::Asking;
        s.doze();
        assert!(matches!(s.feed, Feed::Nothing) && !s.dozing);
    }

    const SET: &str = "[[meter]]\nlabel = \"RAM\"\nexpr = \"1\"\nmax = 1\n[[meter]]\nlabel = \"Twatch TV\"\nstyle = \"screen\"\nset = \"twatch\"\n[[meter]]\nlabel = \"TV\"\nstyle = \"screen\"\n";

    #[test]
    fn more_sets_go_right_after_the_one_thats_there() {
        let labels = |c: Config| c.meters.iter().map(|m| m.label.clone()).collect::<Vec<_>>();
        assert_eq!(labels(with_more_sets(&config(SET)).unwrap()), ["RAM", "Twatch TV", MORE_CELL, AUTO_CELL, "TV"], "the buttons are always ready");
        let c = with_more_sets(&config(&format!("{SET}[twatch]\nmore_sets = 2\n"))).unwrap();
        assert_eq!(labels(c), ["RAM", "Twatch TV", "Twatch TV 2", "Twatch TV 3", MORE_CELL, AUTO_CELL, "TV"]);
    }

    #[test]
    fn a_set_added_for_good_takes_the_next_name_and_the_ones_that_come_and_go_move_on() {
        let mut c = config(&format!("{SET}[twatch]\nmore_sets = 1\n"));
        add_set(&mut c);
        let labels: Vec<&str> = c.meters.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, ["RAM", "Twatch TV", "Twatch TV 2", "TV"]);
        assert_eq!(setup(&c, false, false, false, false).sets, [("Twatch TV".to_string(), true), ("Twatch TV 2".to_string(), true), ("Twatch TV 3".to_string(), false)]);
    }

    #[test]
    fn the_highest_quality_steps_and_stops_at_the_ends() {
        let mut c = config(SET);
        step_quality(&mut c, 1);
        assert_eq!(c.twatch.as_ref().unwrap().top_quality, "720p");
        step_quality(&mut c, 9);
        assert_eq!(c.twatch.as_ref().unwrap().top_quality, "1080p60");
        step_quality(&mut c, -9);
        assert_eq!(c.twatch.as_ref().unwrap().top_quality, "360p");
    }

    #[test]
    fn the_buttons_stay_away_until_they_have_something_to_say() {
        assert!(hidden(MORE_CELL) && hidden(AUTO_CELL));
    }

    #[test]
    fn there_are_never_more_than_three() {
        let c = config(&format!("{SET}[twatch]\nmore_sets = 9\n"));
        assert_eq!(setup(&c, false, false, false, false).sets.len(), 1, "from the file, too many is none: the usual");
        let mut c = config(SET);
        c.twatch.get_or_insert_with(Default::default).more_sets = 9;
        assert_eq!(setup(&c, false, false, false, false).sets.len(), 3);
        assert_eq!(setup(&c, false, false, false, false).sets[2], ("Twatch TV 3".to_string(), false));
    }

    #[test]
    fn without_one_in_the_meters_there_are_none() {
        let c = config("[twatch]\nmore_sets = 2\n");
        assert!(setup(&c, false, false, false, false).sets.is_empty());
        assert!(with_more_sets(&c).is_none());
    }

    fn pressed(c: &Config, pressure: Pressure) -> Setup {
        Setup { pressure, ..setup(c, false, false, false, false) }
    }

    #[test]
    fn what_a_set_shows_goes_by_the_setting_and_the_machine() {
        let with = |value: &str| config(&format!("{SET}[twatch]\npictures = \"{value}\"\n"));
        let short = Pressure { vram: true, ..Default::default() };
        let fine = Pressure::default();
        assert_eq!(pressed(&config(SET), fine).shows(false), Shows::Video, "the usual: video, with plenty of memory");
        assert_eq!(pressed(&config(SET), short).shows(false), Shows::Pictures, "and pictures without");
        assert_eq!(pressed(&with("never"), short).shows(false), Shows::Nothing, "the player goes either way");
        assert_eq!(pressed(&with("initial"), fine).shows(false), Shows::Pictures, "pictures until you click");
        assert_eq!(pressed(&with("initial"), fine).shows(true), Shows::Video);
        assert_eq!(pressed(&with("initial"), short).shows(true), Shows::Pictures, "a click doesn't bring it back while memory's short");
        assert_eq!(pressed(&with("always"), fine).shows(true), Shows::Pictures);
    }

    #[test]
    fn removing_a_set_takes_only_that_one() {
        let mut c = config(&format!("{SET}[[meter]]\nlabel = \"Twatch TV 2\"\nstyle = \"screen\"\nset = \"twatch\"\n\n[twatch]\nmore_sets = 2\n"));
        remove_set(&mut c, "Twatch TV 2", true);
        assert_eq!(c.meters.iter().filter(|m| is_twatch(m)).map(|m| m.label.as_str()).collect::<Vec<_>>(), ["Twatch TV"]);
        remove_set(&mut c, "Twatch TV 3", false);
        assert_eq!(c.twatch.as_ref().unwrap().more_sets, 1, "one that came by itself: one fewer of those");
        remove_set(&mut c, "Twatch TV", true);
        assert!(!c.meters.iter().any(is_twatch));
        assert_eq!(c.twatch.as_ref().unwrap().more_sets, 1, "its settings stay");
    }

    #[test]
    fn media_controls_are_never_unless_you_say() {
        let with = |value: &str| setup(&config(&format!("{SET}[twatch]\nmedia_controls = \"{value}\"\n")), false, false, false, false).media;
        let usual = setup(&config(SET), false, false, false, false).media;
        assert_eq!(usual, MediaControls::Never);
        assert!(!usual.shows(false) && !usual.shows(true), "not even with its sound on");
        assert!(with("unmuted").shows(false) && !with("unmuted").shows(true), "only while it can be heard");
        assert!(with("playing").shows(true), "muted or not");
        assert_eq!(with("nonsense"), MediaControls::Never);
    }

    #[test]
    fn the_old_setting_comes_through() {
        assert_eq!(config(&format!("{SET}[twatch]\nthumbnails = true\n")).twatch.unwrap().pictures, "always");
        assert_eq!(config(&format!("{SET}[twatch]\nthumbnails = false\n")).twatch.unwrap().pictures, "low_memory");
    }

    #[test]
    fn a_set_keeps_its_one_channel_between_runs() {
        assert_eq!(kept("1\t0\t0\tcrab\tTwatch TV\n", "Twatch TV", false), (true, false, false, Some("crab".to_string())));
        assert_eq!(kept("1\t0\t0\t\tTwatch TV\n", "Twatch TV", false), (true, false, false, None));
    }

    #[test]
    fn off_the_mains_it_is_pictures_whatever_you_asked_for() {
        let c = config(SET);
        assert_eq!(setup(&c, false, false, false, false).shows(true), Shows::Video);
        assert_eq!(setup(&c, false, true, false, false).shows(true), Shows::Pictures);
    }

    #[test]
    fn nothing_in_the_file_can_make_it_look_too_often() {
        // From the file, a value out of range is no value: the usual, and the settings say what was wrong.
        let c = config(&format!("{SET}[twatch]\ncheck_seconds = 1\n"));
        assert_eq!(setup(&c, false, false, false, false).check, Duration::from_secs(u64::from(live::USUAL_CHECK)));
        assert_eq!(c.issues.len(), 1);
        // However it comes, it's no quicker than the quickest.
        let mut c = config(SET);
        c.twatch.get_or_insert_with(Default::default).check_seconds = 1;
        assert_eq!(setup(&c, false, false, false, false).check, Duration::from_secs(u64::from(FASTEST_CHECK)));
        let usual = setup(&config(SET), false, false, false, false);
        assert_eq!((usual.check, usual.check_extra), (Duration::from_secs(600), Duration::from_secs(300)), "ten minutes, and up to five more");
        let extras: Vec<Duration> = (0..50).map(|_| live::extra(usual.check_extra)).collect();
        assert!(extras.iter().all(|e| *e <= usual.check_extra) && extras.iter().any(|e| *e != extras[0]), "up to five more, never the same for long");
        assert_eq!(live::extra(Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn it_plays_480p_unless_you_say_otherwise() {
        assert!(setup(&config(SET), false, false, false, false).best == Quality::parse("480p").unwrap());
    }

    #[test]
    fn a_set_comes_back_as_it_was_left() {
        assert_eq!(kept("", "Twatch TV", false), (true, true, false, None), "on and silent");
        assert_eq!(kept("", "Twatch TV", true), (true, true, true, None), "and as the setting says");
        assert_eq!(kept("0\t0\t1\tTwatch TV\n", "Twatch TV", false), (false, false, true, None));
        assert_eq!(kept("0\t0\tTwatch TV\n", "Twatch TV", false), (false, false, false, None), "as it was written before sets could be held");
        assert_eq!(kept("0\t0\t1\tOther\n", "Twatch TV", false), (true, true, false, None));
    }
}
