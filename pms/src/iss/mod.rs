//! Channel 37, Number One: the International Space Station's urine tank, on the TV. NASA's ISSLive! telemetry, by way of
//! Lightstreamer, inspired by pISSStream (by Jaennaet), whose idea it is; none of its code. On the screen: an astronaut, his flag
//! planted, and the flag's the tank (look.rs).
//!
//! - **Only when it's on.** It goes online (`Use::Iss`, by pms-fetch.exe, through the proxy when the network settings say) only
//!   once the settings say so (`[iss] online`, off unless you switch it on); then in the background, whatever the TVs are
//!   showing, so that a flush is told and played when it happens. A TV on the channel costs no more requests: it shows the last.
//! - **How often follows the tank** (plan.rs): every 15 minutes while it fills; while it drains, a third of the time it'll take to
//!   empty (a minute to 15); live (TLCP long polling, tlcp.rs) when it's nearly empty, until it has. Away from the PC or on
//!   battery, twice as long between looks, and never live.
//! - **A look** is one GET, a snapshot of all five items, in a session that ends by itself (made to be kept no time at all). A
//!   live session's ended when it's done with, as the server would otherwise keep it a while.
//! - **What moves:** nothing, but the flag rippling for 2 s when a new value comes, and the flush's whirlpool for 3 s: each new
//!   picture costs Explorer a frame for the whole taskbar.
//! - **A flush**, a drain from 30% or more reaching its bottom (plan.rs): FLUSH! on the flag while it's still low, for up to a
//!   quarter of an hour, a homage on a toilet, and a notification unless you've said not (`[iss] notify_flush`).
//! - **A click** takes the flag round the station's other water, 5 s each: the waste water, then the drinking water, then back.
//!   Switched off, it opens the settings where it's switched on.
//! - `PMS_ISS_SOURCE`, for trying it: a file whose text is taken as the server's answer, read again every few seconds and never
//!   sent anywhere, the channel on whatever the settings say.
//!
//! - **PTV's bumps** (bumps.rs, drawn by solid.rs) take the picture over for a few seconds: an ident on the quarter hour and a
//!   homage on the hour (local time), an ident when somebody's back after ten minutes away, a homage on a toilet when the station
//!   flushes, and the showroom the first time the channel's watched in a run; each kind dealt round without repeats, by a hash of
//!   the day and the turn. Only while somebody'd see one (switched on, not on battery, somebody at the PC, the set awake and in
//!   sight), each in its first minute or not at all, and those that come of something happening not on top of another.
//!   `[iss] bumps` says how often ("often", "hourly" for the homages on the hour only, or "off"); `pms bump` and the key on the
//!   remote's card play one now, whatever it says.

pub mod bumps;
pub mod look;
pub mod plan;
mod solid;
pub mod tlcp;

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::lang::iss as t;
use crate::tv::Canvas;
use look::{Fill, Scene};

/// Its channel.
pub const CHANNEL: u32 = pms_screen::ISS_CHANNEL;
/// How long the server's asked to hold a live poll for something to say (it has a limit of its own, 30 s).
const LONG_POLL_MS: u32 = 30_000;
/// The flushed look stays this long after the flush, in seconds.
const FLUSH_SHOWN: i64 = 15 * 60;
/// Under this, it's still just been flushed.
const FLUSHED: f32 = 10.0;
/// This full, and the amber light's on.
const NEARLY_FULL: f32 = 90.0;
/// The urine processor's state while it's distilling.
const DISTILLING: u32 = 16;
/// How long the flag ripples, and the whirlpool turns, in ms; and how often a frame of either is drawn.
const RIPPLE_MS: i64 = 2000;
const WHIRL_MS: i64 = 3000;
const ANIMATED_MS: i64 = 66;
/// How long the tour shows each of the other tanks.
const TOUR_MS: i64 = 5000;

/// What's known of the station's water, as the last answer left it.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Reading {
    /// The urine tank, in percent, and when it last changed (seconds since 1970).
    pub urine: Option<f32>,
    pub urine_at: Option<i64>,
    /// The urine processor's state (`tlcp::PROCESSOR`), and since when.
    pub processor: Option<u32>,
    pub processor_at: Option<i64>,
    pub waste: Option<f32>,
    pub drinking: Option<f32>,
    /// Whether the ground has the station's signal.
    pub signal: Option<bool>,
}

fn reading(known: &tlcp::Known, now: i64) -> Reading {
    let level = |item| known.number(item, "Value").map(|v| v as f32);
    let at = |item| known.number(item, "TimeStamp").and_then(|s| tlcp::when(s, now));
    Reading {
        urine: level(tlcp::URINE),
        urine_at: at(tlcp::URINE),
        processor: known.number(tlcp::PROCESSOR, "Value").map(|v| v as u32),
        processor_at: at(tlcp::PROCESSOR),
        waste: level(tlcp::WASTE),
        drinking: level(tlcp::DRINKING),
        signal: known.get(tlcp::CLOCK, "Status.Class").map(|c| c == tlcp::SIGNAL),
    }
}

/// How it is now, for the sets to show.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Now {
    /// It's switched on.
    pub online: bool,
    pub reading: Option<Reading>,
    /// When the last answer came, in seconds since 1970.
    pub checked: Option<i64>,
    /// Checks in a row that got nothing.
    pub failures: u32,
    /// Counts the tank's new values: one more, and the flag ripples.
    pub seq: u64,
    /// When it was last emptied, as seen here.
    pub flushed: Option<i64>,
}

struct Feed {
    now: Now,
    /// The readings, and where the drain's got to: when to look next, and whether it's emptied.
    plan: plan::Plan,
    /// The schedule, as it was last worked out, and whether that was with nobody at the PC (or on battery).
    step: Option<(plan::Step, bool)>,
    /// A flush that's still to be told, for a notification.
    news: bool,
}

static FEED: Mutex<Feed> = Mutex::new(Feed { now: Now { online: false, reading: None, checked: None, failures: 0, seq: 0, flushed: None }, plan: plan::Plan::new(), step: None, news: false });

/// What the settings say, and whether it's on battery.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Wanted {
    /// How often the bumps come: which of `BUMPS`.
    bumps: usize,
    online: bool,
    notify: bool,
    battery: bool,
}

/// Before the settings are known: off, and quiet.
const QUIET: Wanted = Wanted { bumps: 2, online: false, notify: false, battery: false };
static WANTED: Mutex<Wanted> = Mutex::new(QUIET);
/// Whether the checker's running.
static RUNNING: AtomicBool = AtomicBool::new(false);
/// When the last check was begun, for a checker started again not to check again too soon.
static LAST_TRY: Mutex<Option<Instant>> = Mutex::new(None);

fn unix() -> i64 {
    (pms_screen::now_ms() / 1000) as i64
}

/// The file `PMS_ISS_SOURCE` names, if it names one: taken as the server's answer.
fn source() -> Option<std::path::PathBuf> {
    std::env::var_os("PMS_ISS_SOURCE").map(Into::into)
}

/// Takes up what the settings say, and whether the PC's on battery: every second or so.
pub fn follow(config: &crate::config::Config, battery: bool) {
    let iss = config.iss.clone().unwrap_or_default();
    if let Ok(mut w) = WANTED.lock() {
        *w = Wanted { bumps: BUMPS.iter().position(|b| *b == iss.bumps).unwrap_or(0), online: iss.online, notify: iss.notify_flush, battery };
    }
    presence();
    // Switched on, it looks in the background, whatever the TVs show.
    if switched_on() && !RUNNING.swap(true, Ordering::AcqRel) {
        std::thread::spawn(checker);
    }
}

/// Whether it's switched on: shuffling only lands on it if it is.
pub fn switched_on() -> bool {
    source().is_some() || WANTED.lock().is_ok_and(|w| w.online)
}

/// How it is now.
pub fn now() -> Now {
    let mut now = FEED.lock().map(|f| f.now.clone()).unwrap_or_default();
    now.online = switched_on();
    now
}

/// A flush that's still to be told, if the settings want it told: once.
pub fn take_flushed() -> Option<f32> {
    let mut feed = FEED.lock().ok()?;
    if !std::mem::take(&mut feed.news) {
        return None;
    }
    feed.now.reading.as_ref()?.urine
}

/// An answer's come: what's new in it, a flush if it's one.
fn took(feed: &mut Feed, r: Reading, now: i64, notify: bool) {
    let before = feed.now.reading.as_ref().and_then(|r| r.urine);
    if let Some(level) = r.urine {
        if before != Some(level) {
            feed.now.seq += 1;
        }
        if feed.plan.take(now, level, r.processor, r.signal != Some(false)) {
            feed.now.flushed = Some(now);
            feed.news |= notify;
            crate::log(format!("Number One: the ISS emptied its urine tank, down to {level}%"));
        }
    }
    feed.now.reading = Some(r);
    feed.now.checked = Some(now);
    feed.now.failures = 0;
}

fn failed(why: &str) {
    if let Ok(mut feed) = FEED.lock() {
        feed.now.failures += 1;
        if feed.now.failures == 1 || feed.now.failures == 3 {
            crate::log(format!("Number One: no telemetry ({} in a row): {why}", feed.now.failures));
        }
    }
}

fn answered(reading: Reading) {
    let notify = WANTED.lock().is_ok_and(|w| w.notify);
    if let Ok(mut feed) = FEED.lock() {
        took(&mut feed, reading, unix(), notify);
    }
}

/// Fetches `url`, as text: or, trying it out, the file that stands in for the server.
fn ask(url: &str, patience: Duration) -> Result<String, String> {
    if let Some(file) = source() {
        return std::fs::read_to_string(&file).map_err(|e| format!("PMS_ISS_SOURCE: {e}"));
    }
    let got = crate::fetch::fetch(crate::net::Use::Iss, crate::fetch::Kind::Get, url, patience, 0, 256 << 10)?;
    if got.status != 200 {
        return Err(format!("HTTP {}", got.status));
    }
    Ok(String::from_utf8_lossy(&got.body).into_owned())
}

/// What a session's answer says: its id, if it's a new one, and what it knows now. An error if the session's over or refused.
fn read(answer: &str, subs: &[(u32, &'static [&'static str], &'static [&'static str])], known: &mut tlcp::Known) -> Result<(Option<String>, bool), String> {
    let (mut id, mut any) = (None, false);
    for line in tlcp::parse(answer) {
        match line {
            tlcp::Line::Session(s) => id = Some(s),
            tlcp::Line::Update { sub, item, fields } => {
                known.take(subs, sub, item, &fields);
                any = true;
            }
            tlcp::Line::End(why) => return Err(format!("the session ended: {why}")),
            tlcp::Line::Refused(why) => return Err(format!("refused: {why}")),
            tlcp::Line::Loop | tlcp::Line::Other => {}
        }
    }
    Ok((id, any))
}

/// One check: a snapshot of everything, then the session's ended.
fn check() {
    let subs = tlcp::subscriptions(false);
    let mut known = tlcp::Known::default();
    match ask(&tlcp::snapshot(), Duration::from_secs(20)).and_then(|a| read(&a, &subs, &mut known)) {
        Ok(_) if !known.is_empty() => answered(reading(&known, unix())),
        Ok(_) => failed("an answer with nothing in it"),
        Err(e) => failed(&e),
    }
}

/// Ends a session, rather than leave it to time out. What comes of it doesn't matter.
fn end(session: &str) {
    if source().is_none() {
        let _ = ask(&tlcp::destroy(session), Duration::from_secs(10));
    }
}

/// A live session: its id, and what it knows.
struct Live {
    id: String,
    known: tlcp::Known,
}

/// Live: a session made, or the next long poll of the one there is. Each takes up to half a minute.
fn live(session: &mut Option<Live>) {
    let subs = tlcp::subscriptions(true);
    match session {
        None => {
            let mut known = tlcp::Known::default();
            match ask(&tlcp::live(), Duration::from_secs(20)).and_then(|a| read(&a, &subs, &mut known)) {
                Ok((Some(id), _)) => {
                    // The clock, for the signal: a subscription of its own.
                    if let Err(e) = ask(&tlcp::add_signal(&id), Duration::from_secs(20)).and_then(|a| read(&a, &subs, &mut known)) {
                        crate::log(format!("Number One: live, but without the signal: {e}"));
                    }
                    answered(reading(&known, unix()));
                    *session = Some(Live { id, known });
                }
                Ok((None, _)) => {
                    failed("no session");
                    std::thread::sleep(Duration::from_secs(60));
                }
                Err(e) => {
                    failed(&e);
                    std::thread::sleep(Duration::from_secs(60));
                }
            }
        }
        // Each answer, changes or none, is a reading: the time that passes without a fall counts too.
        Some(s) => match ask(&tlcp::poll(&s.id, LONG_POLL_MS), Duration::from_secs(60)).and_then(|a| read(&a, &subs, &mut s.known)) {
            Ok(_) => answered(reading(&s.known, unix())),
            Err(e) => {
                crate::log(format!("Number One: the live session's over ({e}): a new one"));
                *session = None;
                std::thread::sleep(Duration::from_secs(5));
            }
        },
    }
}

/// Looks at the station, as often as the tank wants (plan.rs), for as long as it's switched on.
fn checker() {
    let mut session: Option<Live> = None;
    loop {
        let w = WANTED.lock().map(|w| *w).unwrap_or(QUIET);
        let trying = source().is_some();
        if !(w.online || trying) {
            if let Some(s) = session.take() {
                end(&s.id);
            }
            RUNNING.store(false, Ordering::Release);
            // Switched on again just then, and found it running.
            if switched_on() && !RUNNING.swap(true, Ordering::AcqRel) {
                continue;
            }
            return;
        }
        let away = AWAY.load(Ordering::Relaxed) || w.battery;
        let step = FEED.lock().map(|mut f| {
            let step = f.plan.next(unix(), away);
            f.step = Some((step, away));
            step
        });
        let Ok(step) = step else { return };
        let failures = FEED.lock().map(|f| f.now.failures).unwrap_or(0);
        let every = match step.mode {
            // Live only while the server answers: one that's failing is given time, below.
            plan::Mode::Live if !trying && failures < 2 => {
                live(&mut session);
                continue;
            }
            // Trying it out, every few seconds: the file's there to be changed by hand.
            _ if trying => Duration::from_secs(3),
            plan::Mode::Live => Duration::from_secs(60),
            plan::Mode::Every(s) => Duration::from_secs(s.max(1) as u64),
        };
        let every = if trying { every } else { every.max(backoff(failures)) };
        if let Some(s) = session.take() {
            end(&s.id);
        }
        let due = LAST_TRY.lock().map(|l| l.is_none_or(|at| at.elapsed() >= every)).unwrap_or(true);
        if due {
            if let Ok(mut l) = LAST_TRY.lock() {
                *l = Some(Instant::now());
            }
            check();
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// After `n` failed looks in a row, at least this long before the next: 2, 4, 8, 16, 32 minutes, then an hour. The server's a
/// public one that nobody pays for (Lightstreamer's push.lightstreamer.com, relaying NASA's feed): one that's struggling, or
/// saying no, isn't to be asked again and again by everybody's PimpMyShell at once.
fn backoff(n: u32) -> Duration {
    if n == 0 { Duration::ZERO } else { Duration::from_secs(60 * (1u64 << n.min(6)).min(60)) }
}

/// Whether it's just been flushed, as of `at`: for a quarter of an hour, while the tank's still nearly empty.
fn flushing(now: &Now, at: i64) -> bool {
    let low = now.reading.as_ref().and_then(|r| r.urine).is_some_and(|v| v < FLUSHED);
    low && now.flushed.is_some_and(|f| at - f < FLUSH_SHOWN)
}

/// How full the tank is, 0 to 1, for the one he carries in the homages: as it was last heard, or as it was in the design.
pub fn tank_of(now: &Now) -> f32 {
    now.reading.as_ref().and_then(|r| r.urine).map_or(0.29, |v| (v / 100.0).clamp(0.0, 1.0))
}

/// A level, as the flag says it.
fn percent(level: Option<f32>) -> String {
    level.map_or_else(|| "??".to_string(), |v| format!("{}%", v.round() as i64))
}

/// Which of the other tanks the tour's on.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tank {
    Waste,
    Water,
}

/// The picture, as `now` has it: on the tour, if it's on; `phase` and `ripple` for the flag (ripple: how much of it is left),
/// `turn` for a flush's whirlpool.
pub fn scene(now: &Now, tour: Option<Tank>, ripple: Option<(f32, f32)>, turn: f32, at: i64) -> Scene {
    let plain = Scene { level: 0.0, fill: Fill::Urine, words: String::new(), small: None, lost: false, bubbles: false, full: false, swirl: None, ripple, bar: None };
    if !now.online {
        return Scene { words: "OFF".into(), bar: Some("SEE SETTINGS"), ripple: None, ..plain };
    }
    let Some(r) = now.reading.as_ref().filter(|_| now.failures < 3) else {
        // Nothing yet, or nothing for three checks running.
        let last = now.reading.as_ref().and_then(|r| r.urine);
        return Scene { words: if now.failures < 3 { "...".into() } else { "??".into() }, small: last.map(|v| percent(Some(v))), ripple: None, ..plain };
    };
    match tour {
        Some(Tank::Waste) => return Scene { level: r.waste.unwrap_or(0.0), fill: Fill::Waste, words: percent(r.waste), small: Some("WASTE".into()), ..plain },
        Some(Tank::Water) => return Scene { level: r.drinking.unwrap_or(0.0), fill: Fill::Water, words: percent(r.drinking), small: Some("DRINK".into()), ..plain },
        None => {}
    }
    let level = r.urine.unwrap_or(0.0);
    if r.signal == Some(false) {
        return Scene { level, words: "LOS".into(), small: Some(percent(r.urine)), lost: true, ..plain };
    }
    let flushing = flushing(now, at);
    Scene {
        level,
        words: percent(r.urine),
        small: flushing.then(|| "FLUSH!".into()),
        swirl: flushing.then_some(turn),
        bubbles: r.processor == Some(DISTILLING),
        full: level >= NEARLY_FULL,
        ..plain
    }
}

/// A few seconds in which something else has the channel's picture: PTV's idents, and its homages (bumps.rs). Each frame, `ms`
/// after the last, it draws the whole picture if there's a new one (Some(true)), or has nothing new (Some(false)); or it's over
/// (None), and the tank's back.
pub(crate) trait Bump: Send {
    fn frame(&mut self, ms: i64, c: &mut Canvas) -> Option<bool>;
}

/// One of PTV's bumps, playing: 15 frames a second, each drawn when it's due, at whatever size the screen is then.
struct Playing {
    kind: bumps::Kind,
    ms: i64,
    /// The frame last drawn, and at what size.
    drawn: Option<(usize, usize, usize)>,
    /// How full the tank he carries is.
    tank: f32,
}

impl Bump for Playing {
    fn frame(&mut self, ms: i64, c: &mut Canvas) -> Option<bool> {
        self.ms += ms;
        let f = (self.ms.max(0) * bumps::FPS as i64 / 1000) as usize;
        if f >= self.kind.frames() {
            return None;
        }
        if self.drawn == Some((f, c.w, c.h)) {
            return Some(false);
        }
        self.drawn = Some((f, c.w, c.h));
        let picture = bumps::picture(self.kind, f, c.w, c.h, self.tank);
        for (px, rgb) in c.px.as_chunks_mut::<4>().0.iter_mut().zip(picture) {
            px.copy_from_slice(&[rgb[2], rgb[1], rgb[0], 255]);
        }
        Some(true)
    }
}

/// How often the bumps come, as the settings say: "often" (the usual), "hourly" or "off".
pub const BUMPS: [&str; 3] = ["often", "hourly", "off"];

/// A hash of `k`, for dealing the bumps: the same every time.
fn hash(k: &[i64]) -> u32 {
    let mut h: u32 = 2_166_136_261;
    for v in k {
        for b in v.to_le_bytes() {
            h = (h ^ u32::from(b)).wrapping_mul(16_777_619);
        }
    }
    h ^= h >> 15;
    h.wrapping_mul(0x2C1B_3C6D) ^ (h >> 12)
}

/// The `turn`th of `kinds` on `day`: each round of them in an order of its own, the day's and the round's, so that none comes
/// again until all have been. `deck` keeps one kind of turn from another.
pub fn deal(kinds: &[bumps::Kind], day: i64, turn: i64, deck: i64) -> bumps::Kind {
    let n = kinds.len().max(1) as i64;
    let (round, at) = (turn.div_euclid(n), turn.rem_euclid(n));
    let mut order: Vec<(u32, usize)> = (0..kinds.len()).map(|i| (hash(&[day, round, deck, i as i64]), i)).collect();
    order.sort_unstable();
    kinds[order[at as usize].1]
}

/// When it is, for the bumps: today (days since 1970, here), which quarter of an hour it is and how many seconds into it, how
/// often the settings say (which of `BUMPS`), whether anybody'd see one (switched on, not on battery, somebody at the PC), and
/// the last time somebody came back to it after a while away (how many times, and when, in ms since 1970).
#[derive(Clone, Copy, Debug)]
pub struct When {
    pub day: i64,
    pub quarter: i64,
    pub into: i64,
    pub often: usize,
    pub seen: bool,
    pub back: (u64, u64),
    pub now_ms: u64,
}

/// How far into a quarter of an hour its bump may start: one that's missed is skipped, not saved up.
const SLOT_GRACE: i64 = 60;
/// How soon after somebody's back, and after a flush, theirs may start.
const BACK_GRACE: u64 = 30_000;
const FLUSH_GRACE: i64 = 60;
/// How long after a bump one that comes of something happening (somebody back, a flush, the showroom) may start: not one on top
/// of another. The quarter hours' aren't held back.
const BETWEEN: u64 = 120_000;
/// Somebody who hasn't touched the PC for this long isn't there; for this long, is away, and coming back is worth a bump.
const IDLE_MS: u64 = 60_000;
const AWAY_MS: u64 = 10 * 60_000;

/// Whether the PC's had nobody at it for a while, whether they're away, how many times somebody's come back after being away, and
/// when.
static IDLE: AtomicBool = AtomicBool::new(false);
static AWAY: AtomicBool = AtomicBool::new(false);
static BACK: AtomicU64 = AtomicU64::new(0);
static BACK_AT: AtomicU64 = AtomicU64::new(0);
/// The showroom's been shown, on first tuning in, this run.
static SHOWROOM: AtomicBool = AtomicBool::new(false);
/// Bumps asked for by hand, in order.
static ASKED: AtomicU64 = AtomicU64::new(0);

/// How long since anybody touched the keyboard or mouse, in ms.
fn idle_ms() -> u64 {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return 0;
    }
    u64::from(unsafe { GetTickCount() }.wrapping_sub(info.dwTime))
}

/// Whether anybody's at the PC, and whether they've just come back: every second or so.
fn presence() {
    let idle = idle_ms();
    IDLE.store(idle >= IDLE_MS, Ordering::Relaxed);
    if idle >= AWAY_MS {
        AWAY.store(true, Ordering::Relaxed);
    } else if idle < 3000 && AWAY.swap(false, Ordering::Relaxed) {
        BACK_AT.store(pms_screen::now_ms(), Ordering::Relaxed);
        BACK.fetch_add(1, Ordering::Relaxed);
    }
}

impl When {
    /// Now, here.
    pub fn now() -> When {
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        let day = tlcp::days(i64::from(t.wYear), i64::from(t.wMonth), i64::from(t.wDay));
        let (minute, second) = (i64::from(t.wMinute), i64::from(t.wSecond));
        let w = WANTED.lock().map(|w| *w).unwrap_or(QUIET);
        When {
            day,
            quarter: i64::from(t.wHour) * 4 + minute / 15,
            into: (minute % 15) * 60 + second,
            often: w.bumps,
            seen: switched_on() && !w.battery && !IDLE.load(Ordering::Relaxed),
            back: (BACK.load(Ordering::Relaxed), BACK_AT.load(Ordering::Relaxed)),
            now_ms: pms_screen::now_ms(),
        }
    }
}

/// `pms bump [<name>|next] [<tv>]`: one of the bumps (or the next of them all), now, on the TV called `tv`, or on every TV the
/// settings have. Each goes to Number One's channel for it, and stays there after.
pub fn bump_now(which: Option<&str>, tv: Option<&str>) -> Result<String, String> {
    let code = match which {
        None | Some("next") => pms_screen::BUMP_ANY,
        Some(name) => {
            let kind = bumps::Kind::from_name(name).ok_or_else(|| format!("no bump called {name}: {}", bumps::Kind::ALL.map(|k| k.name()).join(", ")))?;
            pms_screen::BUMP + kind.index() as u32
        }
    };
    let sets: Vec<String> = match tv {
        Some(tv) => vec![tv.to_string()],
        None => crate::config::load(&crate::config::default_path())
            .map(|c| c.meters.into_iter().filter(|m| m.style == crate::config::Style::Screen && m.set.as_deref() != Some("twatch")).map(|m| m.label).collect())
            .unwrap_or_else(|_| vec!["TV".to_string()]),
    };
    let mut told = Vec::new();
    for set in &sets {
        if let Ok(screen) = pms_screen::Screen::open(set) {
            screen.tune(code);
            told.push(set.as_str());
        }
    }
    if told.is_empty() {
        return Err(format!("no TV to show it on (looked for {}): is the daemon running?", sets.join(", ")));
    }
    Ok(format!("PTV's on {}", told.join(", ")))
}

/// The bump to play when one's asked for by hand (a remote's key, `pms bump`): `kind`, or, None, the next of every kind.
pub fn asked_for(kind: Option<bumps::Kind>) -> bumps::Kind {
    kind.unwrap_or_else(|| {
        let day = (pms_screen::now_ms() / 86_400_000) as i64;
        deal(&bumps::Kind::ALL, day, ASKED.fetch_add(1, Ordering::Relaxed) as i64, 9)
    })
}

/// The channel on one set: what it last showed, and what's moving on it.
#[derive(Default)]
pub struct NumberOne {
    /// The last picture sent, at its size, and whether the channel's number was over it.
    shown: Option<(Scene, usize, usize, bool)>,
    /// The tank's new values, as far as it's shown them.
    seen: Option<u64>,
    /// How long the flag's been rippling, and the whirlpool turning.
    ripple: Option<i64>,
    whirl: Option<i64>,
    /// The flush it's shown.
    flush_seen: Option<i64>,
    /// The tour: which tank, and how much longer.
    tour: Option<(Tank, i64)>,
    /// Since the last frame of what's moving.
    since: i64,
    bump: Option<Box<dyn Bump>>,
    /// The quarter of an hour whose bump it's had its chance at (day, quarter), the last coming back it's seen, and the flush it's
    /// bumped.
    slot: Option<(i64, i64)>,
    back: Option<u64>,
    flush_bumped: Option<i64>,
    /// When its last bump ended, in ms since 1970: those that come of something happening wait a while after another.
    ended: Option<u64>,
}

impl NumberOne {
    /// Tuned in.
    pub fn restart(&mut self) {
        *self = NumberOne::default();
    }

    /// A click: the next of the station's other tanks, and back.
    pub fn tour(&mut self) {
        self.tour = match self.tour {
            None => Some((Tank::Waste, TOUR_MS)),
            Some((Tank::Waste, _)) => Some((Tank::Water, TOUR_MS)),
            Some((Tank::Water, _)) => None,
        };
    }

    pub fn touring(&self) -> Option<Tank> {
        self.tour.map(|t| t.0)
    }

    /// Plays `kind` now, whatever was on, with the tank he carries `tank` full (0 to 1).
    pub fn play(&mut self, kind: bumps::Kind, tank: f32) {
        self.bump = Some(Box::new(Playing { kind, ms: 0, drawn: None, tank }));
    }

    /// Whether a bump's playing.
    pub fn bumping(&self) -> bool {
        self.bump.is_some()
    }

    /// The bump that's due, as of `when`, if any: an ident on the quarter hour and a homage on the hour; an ident when somebody's
    /// back after ten minutes or more away; a homage on a toilet when the station flushes; and the showroom when the channel's
    /// first tuned to, each run. Only if somebody'd see it, and as often as the settings say (hourly: only the homages on the
    /// hour). Each in its first minute or not at all.
    fn bump_due(&mut self, now: &Now, when: &When, at: i64) -> Option<bumps::Kind> {
        use bumps::Kind;
        // What's happened while it couldn't be shown isn't saved up for when it can.
        let back = self.back.replace(when.back.0);
        let slot = (when.day, when.quarter);
        let new_slot = self.slot != Some(slot);
        self.slot = Some(slot);
        let flushed = now.flushed.filter(|f| at - f < FLUSH_GRACE && self.flush_bumped != Some(*f));
        if let Some(f) = flushed {
            self.flush_bumped = Some(f);
        }
        if !when.seen || when.often >= 2 {
            return None;
        }
        let often = when.often == 0;
        if new_slot && when.into < SLOT_GRACE {
            let (hour, quarter) = (when.quarter / 4, when.quarter % 4);
            if quarter == 0 {
                return Some(deal(&Kind::HOMAGES, when.day, hour, 1));
            }
            if often {
                return Some(deal(&Kind::IDENTS, when.day, hour * 3 + quarter - 1, 2));
            }
        }
        if !often || self.ended.is_some_and(|e| when.now_ms.saturating_sub(e) < BETWEEN) {
            return None;
        }
        if let Some(f) = flushed {
            return Some(deal(&Kind::TOILETS, when.day, f, 4));
        }
        if back.is_some_and(|b| b != when.back.0) && when.now_ms.saturating_sub(when.back.1) < BACK_GRACE {
            return Some(deal(&Kind::IDENTS, when.day, when.back.0 as i64, 3));
        }
        if !SHOWROOM.swap(true, Ordering::Relaxed) {
            return Some(Kind::Showroom);
        }
        None
    }

    /// The next frame into `c`, if there's anything new, `dt` ms after the last. `watched`: past the run through the channels,
    /// where a bump may start.
    pub(crate) fn frame(&mut self, dt: i64, c: &mut Canvas, number_up: bool, watched: bool) -> bool {
        let when = watched.then(When::now);
        self.frame_as(dt, c, number_up, &now(), unix(), when.as_ref())
    }

    fn frame_as(&mut self, dt: i64, c: &mut Canvas, number_up: bool, now: &Now, at: i64, when: Option<&When>) -> bool {
        if self.bump.is_none()
            && let Some(when) = when
            && let Some(kind) = self.bump_due(now, when, at)
        {
            crate::log(format!("Number One: PTV plays {}", kind.name()));
            self.play(kind, tank_of(now));
        }
        if let Some(bump) = &mut self.bump {
            match bump.frame(dt, c) {
                Some(drew) => {
                    self.shown = None;
                    return drew;
                }
                None => {
                    self.bump = None;
                    self.ended = Some(pms_screen::now_ms());
                }
            }
        }
        // A new value: the flag's planted again. Not the one there was on tuning in.
        if self.seen != Some(now.seq) {
            if self.seen.is_some() {
                self.ripple = Some(0);
            }
            self.seen = Some(now.seq);
        }
        let flushed = now.flushed.filter(|_| flushing(now, at));
        if flushed.is_some() && self.flush_seen != flushed {
            self.flush_seen = flushed;
            self.whirl = Some(0);
        }
        let step = |t: Option<i64>, end: i64| t.map(|t| t + dt).filter(|t| *t < end);
        let moving = self.ripple.is_some() || self.whirl.is_some();
        self.ripple = step(self.ripple, RIPPLE_MS);
        self.whirl = step(self.whirl, WHIRL_MS);
        if let Some((tank, left)) = self.tour {
            self.tour = (left > dt).then_some((tank, left - dt)).or_else(|| (tank == Tank::Waste).then_some((Tank::Water, TOUR_MS)));
        }
        self.since += dt;
        if moving && self.since < ANIMATED_MS && self.shown.is_some() {
            return false;
        }
        let ripple = self.ripple.map(|t| (t as f32 * 0.0055, 1.0 - t as f32 / RIPPLE_MS as f32));
        let turn = self.whirl.map_or(0.0, |t| -(t as f32) * 0.004);
        let scene = scene(now, self.touring(), ripple, turn, at);
        let key = (scene, c.w, c.h, number_up);
        if self.shown.as_ref() == Some(&key) {
            return false;
        }
        self.since = 0;
        look::draw(c, &key.0);
        self.shown = Some(key);
        true
    }
}

/// How long ago `then` was, or for how long, in words: minutes, or hours.
fn ago(seconds: i64) -> String {
    let minutes = (seconds.max(0) + 30) / 60;
    if minutes < 90 { t::MINUTES.with(&[("n", &minutes.max(1))]) } else { t::HOURS.with(&[("n", &((minutes + 30) / 60))]) }
}

/// What the TV's remote says of it, a line each: for its card beside the remote, in the screen's `info`, each line begun `iss\t`.
pub fn remote_lines(touring: Option<Tank>) -> String {
    let now = now();
    let at = unix();
    let mut lines: Vec<String> = Vec::new();
    if !now.online {
        lines.push(t::OFF.t().into());
    } else {
        match now.reading.as_ref() {
            None if now.failures == 0 => lines.push(t::TUNING.t().into()),
            None => {}
            Some(r) => {
                let level = r.urine.map(|v| v.round() as i64);
                lines.push(match level {
                    Some(v) if r.urine.unwrap_or(0.0) >= NEARLY_FULL => t::TANK_FULL.with(&[("level", &v)]),
                    Some(v) => t::TANK.with(&[("level", &v)]),
                    None => t::TANK_UNKNOWN.t().into(),
                });
                if flushing(&now, at) {
                    lines.push(t::FLUSHED.t().into());
                }
                let state = match r.processor {
                    Some(DISTILLING) => Some(t::DISTILLING),
                    Some(2) => Some(t::STOPPED),
                    Some(4) => Some(t::SHUT_DOWN),
                    Some(8) => Some(t::MAINTENANCE),
                    Some(32) => Some(t::STANDBY),
                    Some(64) => Some(t::IDLE),
                    Some(128) => Some(t::STARTING),
                    _ => None,
                };
                if let Some(state) = state {
                    lines.push(state.with(&[("time", &r.processor_at.map_or_else(|| "?".to_string(), |since| ago(at - since)))]));
                }
                if r.signal == Some(false) {
                    lines.push(t::SIGNAL_LOST.t().into());
                }
                lines.push(t::WATER.with(&[("waste", &percent(r.waste)), ("drinking", &percent(r.drinking))]));
            }
        }
        if now.failures >= 3 {
            lines.push(t::NO_ANSWER.with(&[("n", &now.failures)]));
        }
        if let Some(checked) = now.checked {
            lines.push(if at - checked < 45 { t::CHECKED_NOW.t().into() } else { t::CHECKED.with(&[("time", &ago(at - checked))]) });
        }
        // What the schedule's doing now.
        if let Some((step, away)) = FEED.lock().ok().and_then(|f| f.step) {
            let minutes = |s: i64| ((s + 30) / 60).max(1);
            lines.push(match (step.doing, step.mode) {
                (_, plan::Mode::Live) => t::SCHED_LIVE.t().into(),
                (plan::Doing::Draining(eta), plan::Mode::Every(s)) => t::SCHED_DRAINING.with(&[("time", &ago(eta as i64)), ("n", &minutes(s))]),
                (plan::Doing::Filling, plan::Mode::Every(s)) => t::SCHED_FILLING.with(&[("n", &minutes(s))]),
            });
            if away {
                lines.push(t::SCHED_AWAY.t().into());
            }
        }
        lines.push(match touring {
            Some(Tank::Waste) => t::TOUR_WASTE.t().into(),
            Some(Tank::Water) => t::TOUR_WATER.t().into(),
            None => t::TOUR.t().into(),
        });
    }
    lines.push(t::CREDIT.t().into());
    lines.iter().map(|l| format!("iss\t{}\n", l.replace(['\t', '\n'], " "))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Reading {
        let subs = tlcp::subscriptions(false);
        let mut known = tlcp::Known::default();
        read(tlcp::FIXTURE, &subs, &mut known).unwrap();
        reading(&known, 1_795_000_000)
    }

    fn feed() -> Feed {
        Feed { now: Now { online: true, ..Now::default() }, plan: plan::Plan::new(), step: None, news: false }
    }

    #[test]
    fn a_failing_server_is_given_longer_and_longer_up_to_an_hour() {
        let mins = |n| backoff(n).as_secs() / 60;
        assert_eq!(backoff(0), Duration::ZERO);
        assert_eq!((1..=7).map(mins).collect::<Vec<_>>(), [2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(mins(1000), 60);
    }

    #[test]
    fn the_fixture_reads_as_the_station_was() {
        let r = fixture();
        assert_eq!(r.urine, Some(29.0));
        assert_eq!(r.processor, Some(32), "standby");
        assert_eq!((percent(r.waste), percent(r.drinking)), ("81%".to_string(), "67%".to_string()));
        assert_eq!(r.signal, Some(true));
        // 7718.118 is day 321 (the 17th of November), 14:07 GMT.
        let at = r.urine_at.unwrap();
        assert_eq!((at.rem_euclid(86_400) / 3600, at.rem_euclid(3600) / 60), (14, 7));
    }

    #[test]
    fn a_drain_to_its_bottom_is_a_flush_and_only_once() {
        let mut f = feed();
        let at = |level: f32| Reading { urine: Some(level), signal: Some(true), ..Reading::default() };
        let mut t = 0;
        for level in [20.0, 25.0, 22.0, 18.0, 12.0, 9.0, 8.0] {
            t += 330;
            took(&mut f, at(level), t, true);
        }
        assert_eq!(f.now.flushed, None, "never 30% or more: no flush");
        for level in [45.0, 40.0, 30.0, 20.0, 12.0, 10.0, 8.0, 6.0, 5.0] {
            t += 330;
            took(&mut f, at(level), t, true);
        }
        assert_eq!(f.now.flushed, Some(t), "down to the bottom");
        assert!(f.news);
        let flushed = t;
        for level in [6.0, 5.0, 6.0, 9.0] {
            t += 330;
            took(&mut f, at(level), t, true);
        }
        assert_eq!(f.now.flushed, Some(flushed), "once for each drain");
        let mut quiet = feed();
        for (i, level) in [70.0, 50.0, 30.0, 10.0, 5.0].into_iter().enumerate() {
            took(&mut quiet, at(level), i as i64 * 330, false);
        }
        assert!(quiet.now.flushed.is_some() && !quiet.news, "seen, but nobody asked to be told");
    }

    #[test]
    fn a_new_value_counts_and_the_same_one_does_not() {
        let mut f = feed();
        let at = |level: f32| Reading { urine: Some(level), ..Reading::default() };
        took(&mut f, at(29.0), 1, false);
        took(&mut f, at(29.0), 2, false);
        assert_eq!(f.now.seq, 1);
        took(&mut f, at(30.0), 3, false);
        assert_eq!((f.now.seq, f.now.checked, f.now.failures), (2, Some(3), 0));
    }

    #[test]
    fn each_state_has_its_look() {
        let live = Now { online: true, reading: Some(fixture()), checked: Some(10), ..Now::default() };
        let s = scene(&live, None, None, 0.0, 10);
        assert_eq!((s.words.as_str(), s.lost, s.bubbles, s.full, s.small.clone()), ("29%", false, false, false, None));
        let mut distilling = live.clone();
        distilling.reading.as_mut().unwrap().processor = Some(16);
        assert!(scene(&distilling, None, None, 0.0, 10).bubbles);
        let mut full = live.clone();
        full.reading.as_mut().unwrap().urine = Some(94.0);
        assert!(scene(&full, None, None, 0.0, 10).full);
        let mut lost = live.clone();
        lost.reading.as_mut().unwrap().signal = Some(false);
        let s = scene(&lost, None, None, 0.0, 10);
        assert_eq!((s.words.as_str(), s.small.as_deref(), s.lost), ("LOS", Some("29%"), true));
        let mut flushed = Now { flushed: Some(5), ..live.clone() };
        assert_eq!(scene(&flushed, None, None, 0.3, 10).small, None, "back over a tenth, it's over");
        flushed.reading.as_mut().unwrap().urine = Some(4.0);
        let s = scene(&flushed, None, None, 0.3, 10);
        assert_eq!((s.small.as_deref(), s.swirl), (Some("FLUSH!"), Some(0.3)));
        assert_eq!(scene(&flushed, None, None, 0.3, 5 + FLUSH_SHOWN).small, None, "and then back to live");
        let none = Now { online: true, failures: 3, reading: Some(fixture()), ..Now::default() };
        let s = scene(&none, None, None, 0.0, 10);
        assert_eq!((s.words.as_str(), s.small.as_deref()), ("??", Some("29%")));
        assert_eq!(scene(&Now { online: true, ..Now::default() }, None, None, 0.0, 0).words, "...", "nothing yet");
        let off = scene(&Now::default(), None, None, 0.0, 0);
        assert_eq!((off.words.as_str(), off.bar), ("OFF", Some("SEE SETTINGS")));
        let waste = scene(&live, Some(Tank::Waste), None, 0.0, 10);
        assert_eq!((waste.words.as_str(), waste.fill), ("81%", Fill::Waste));
        let water = scene(&live, Some(Tank::Water), None, 0.0, 10);
        assert_eq!((water.words.as_str(), water.fill, water.small.as_deref()), ("67%", Fill::Water, Some("DRINK")));
    }

    #[test]
    fn it_holds_still_but_for_a_new_value_and_a_flush() {
        let (w, h) = (64, 48);
        let mut px = vec![0; w * h * 4];
        let mut one = NumberOne::default();
        let mut now = Now { online: true, reading: Some(fixture()), checked: Some(10), seq: 1, ..Now::default() };
        let mut frame = |one: &mut NumberOne, now: &Now, at: i64| one.frame_as(33, &mut Canvas { w, h, px: &mut px }, false, now, at, None);
        assert!(frame(&mut one, &now, 10), "the first picture");
        assert!(!frame(&mut one, &now, 10), "and nothing new after it");
        // A new value: it ripples for two seconds, a frame every 66 ms or so, then holds still again.
        now.seq = 2;
        let mut frames = 0;
        for _ in 0..120 {
            frames += usize::from(frame(&mut one, &now, 10));
        }
        assert!((20..=40).contains(&frames), "{frames}");
        assert!(!frame(&mut one, &now, 10));
        // A flush: the whirlpool, then still.
        now.flushed = Some(10);
        now.reading.as_mut().unwrap().urine = Some(4.0);
        now.seq = 3;
        assert!(frame(&mut one, &now, 11));
        let mut frames = 0;
        for _ in 0..200 {
            frames += usize::from(frame(&mut one, &now, 11));
        }
        assert!((30..=60).contains(&frames), "{frames}");
        // Tuned in again during the flush: it turns again; then, a quarter of an hour on, it's plain.
        one.restart();
        assert!(frame(&mut one, &now, 12));
        assert!((0..5).any(|_| frame(&mut one, &now, 12 + FLUSH_SHOWN)), "back to live");
        assert_eq!(scene(&now, None, None, 0.0, 12 + FLUSH_SHOWN).small, None);
    }

    #[test]
    fn a_click_tours_the_water_and_comes_back() {
        let (w, h) = (64, 48);
        let mut px = vec![0; w * h * 4];
        let now = Now { online: true, reading: Some(fixture()), checked: Some(10), seq: 1, ..Now::default() };
        let mut one = NumberOne::default();
        one.frame_as(33, &mut Canvas { w, h, px: &mut px }, false, &now, 10, None);
        one.tour();
        assert_eq!(one.touring(), Some(Tank::Waste));
        let mut seen = vec![];
        for _ in 0..400 {
            one.frame_as(33, &mut Canvas { w, h, px: &mut px }, false, &now, 10, None);
            if seen.last() != Some(&one.touring()) {
                seen.push(one.touring());
            }
        }
        assert_eq!(seen, [Some(Tank::Waste), Some(Tank::Water), None], "five seconds each, then the tank");
        one.tour();
        one.tour();
        one.tour();
        assert_eq!(one.touring(), None, "a click each skips on");
    }

    #[test]
    fn the_remote_says_where_it_comes_from() {
        let lines = remote_lines(None);
        assert!(lines.lines().all(|l| l.starts_with("iss\t")));
        assert!(lines.contains("pISSStream") && lines.contains("Jaennaet") && lines.contains("Lightstreamer"));
        assert_eq!(ago(30), t::MINUTES.with(&[("n", &1)]));
        assert_eq!(ago(3 * 3600), t::HOURS.with(&[("n", &3)]));
    }

    #[test]
    fn each_round_of_bumps_has_each_once() {
        use bumps::Kind;
        for day in [20_000, 20_001] {
            for round in 0..3 {
                let mut seen: Vec<Kind> = (0..7).map(|i| deal(&Kind::IDENTS, day, round * 7 + i, 2)).collect();
                seen.sort_by_key(|k| k.index());
                assert_eq!(seen, Kind::IDENTS, "day {day}, round {round}");
            }
        }
        let a: Vec<Kind> = (0..7).map(|i| deal(&Kind::IDENTS, 20_000, i, 2)).collect();
        let b: Vec<Kind> = (0..7).map(|i| deal(&Kind::IDENTS, 20_000, 7 + i, 2)).collect();
        let c: Vec<Kind> = (0..7).map(|i| deal(&Kind::IDENTS, 20_001, i, 2)).collect();
        assert!(a != b || a != c, "each round, and each day, has an order of its own");
        assert_eq!(deal(&Kind::HOMAGES, 20_000, 5, 1), deal(&Kind::HOMAGES, 20_000, 5, 1), "and it's the same every time");
    }

    #[test]
    fn bumps_come_when_they_are_due_and_only_then() {
        use bumps::Kind;
        SHOWROOM.store(true, Ordering::Relaxed);
        let now = Now { online: true, ..Now::default() };
        let at = |quarter: i64, into: i64| When { day: 20_000, quarter, into, often: 0, seen: true, back: (0, 0), now_ms: 1_000_000 };
        let mut one = NumberOne::default();
        assert!(one.bump_due(&now, &at(4, 10), 0).is_some_and(Kind::is_homage), "on the hour, a homage");
        assert_eq!(one.bump_due(&now, &at(4, 20), 0), None, "once");
        assert!(one.bump_due(&now, &at(5, 5), 0).is_some_and(|k| !k.is_homage()), "a quarter past, an ident");
        assert_eq!(one.bump_due(&now, &at(6, 90), 0), None, "a minute late: missed, and not saved up");
        assert_eq!(one.bump_due(&now, &at(6, 91), 0), None);
        assert_eq!(one.bump_due(&now, &When { seen: false, ..at(7, 5) }, 0), None, "nobody to see it");
        assert_eq!(one.bump_due(&now, &at(7, 6), 0), None, "and it's not kept for later");
        let mut hourly = NumberOne::default();
        assert_eq!(hourly.bump_due(&now, &When { often: 1, ..at(9, 5) }, 0), None, "hourly: no idents");
        assert!(hourly.bump_due(&now, &When { often: 1, ..at(12, 5) }, 0).is_some_and(Kind::is_homage));
        let mut off = NumberOne::default();
        assert_eq!(off.bump_due(&now, &When { often: 2, ..at(12, 5) }, 0), None, "off");
        // Somebody back after a while away: an ident; but not one who came back before the set was looking.
        let mut gap = NumberOne { ended: Some(1_000_000 - BETWEEN / 2), back: Some(0), ..NumberOne::default() };
        assert_eq!(gap.bump_due(&now, &When { back: (1, 999_000), ..at(10, 300) }, 0), None, "not on top of another");
        assert!(gap.bump_due(&now, &at(12, 3), 0).is_some(), "but the hour's is the hour's");
        let mut back = NumberOne::default();
        assert_eq!(back.bump_due(&now, &When { back: (3, 999_000), ..at(10, 300) }, 0), None);
        assert!(back.bump_due(&now, &When { back: (4, 999_000), ..at(10, 301) }, 0).is_some_and(|k| !k.is_homage()));
        assert_eq!(back.bump_due(&now, &When { back: (5, 900_000), ..at(10, 302) }, 0), None, "too long ago");
        // A flush: a homage on a toilet, once.
        let mut flush = NumberOne::default();
        let flushed = Now { flushed: Some(1000), ..now.clone() };
        assert!(flush.bump_due(&flushed, &at(11, 300), 1010).is_some_and(|k| Kind::TOILETS.contains(&k)));
        assert_eq!(flush.bump_due(&flushed, &at(11, 301), 1011), None);
        assert_eq!(NumberOne::default().bump_due(&flushed, &at(11, 300), 1000 + FLUSH_GRACE), None, "a flush that's old news");
    }

    #[test]
    fn a_bump_takes_the_picture_then_hands_it_back() {
        let (w, h) = (64, 48);
        let mut px = vec![0; w * h * 4];
        let now = Now { online: true, reading: Some(fixture()), checked: Some(10), seq: 1, ..Now::default() };
        let mut one = NumberOne::default();
        one.frame_as(33, &mut Canvas { w, h, px: &mut px }, false, &now, 10, None);
        let still = px.clone();
        one.play(bumps::Kind::Tumble, 0.3);
        let (mut drawn, mut ms, mut other) = (0, 0, false);
        while one.bumping() {
            drawn += usize::from(one.frame_as(16, &mut Canvas { w, h, px: &mut px }, false, &now, 10, None));
            other |= one.bumping() && px != still;
            ms += 16;
            assert!(ms < 10_000);
        }
        assert!(other, "the bump's picture, not the tank's");
        assert!((60..=71).contains(&drawn), "15 frames a second, not 60: {drawn}");
        assert!((4500..4800).contains(&ms), "{ms}");
        assert_eq!(px, still, "and then the tank's back");
        assert!(!one.frame_as(16, &mut Canvas { w, h, px: &mut px }, false, &now, 10, None), "holding still");
    }
}

