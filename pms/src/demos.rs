//! Speed demos for the console's title loop (`console_demos` in the settings): real runs from the Doom Speed Demo Archive
//! (https://dsdarchive.com), recorded on the game the console plays (for the shareware episode, on the full game, whose maps
//! it shares), each played here first, unseen, and kept only if it plays out here as it did there: leaving its level at the
//! very tic the archive says it did. One that doesn't (the map's a little different here, or it leans on another engine's
//! ways) has gone out of step, and is never shown.
//!
//! How much the archive's asked of, and when, is the settings' (`[demos]`: whenever PimpMyShell runs or only while the console's
//! on; hours between looks; demos per look; seconds between downloads; demos per game in all; days between asking for the
//! list), all through pms-fetch (as a download: through the proxy, if downloads go through it). Each demo's zip is fetched
//! once, and unzipped there; it's checked by `pms-poom --check`, in the game's sandbox if it has one, which takes about a
//! tenth of a second, and can't take longer than CHECK_PATIENCE whatever's in it (a demo's finite, and capped in size, and the
//! engine stops where its data does). What came of it is kept in `wads\demos\demos.toml`, for each game (by its SHA-256), so
//! that nothing's fetched or checked twice; the demos that play are kept beside it, as `<the archive's number for it>.lmp`,
//! and listed in `play.txt` for the console, which picks a few each time it starts its game.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The archive's list of demos, and where its files are.
const API: &str = "https://dsdarchive.com/api/demos";
const FILES: &str = "https://dsdarchive.com/files/demos/";
/// The categories, the ones most worth watching first: what's fetched first, when not all of them are (`Demos::most`).
const CATEGORIES: [&str; 12] = ["UV Speed", "UV Max", "NM Speed", "Pacifist", "Tyson", "NoMo", "UV Fast", "UV Respawn", "NM 100S", "Stroller", "NoMo 100S", "Collector"];
/// The most a demo may be, zipped or not: an hour of play is half a megabyte.
const MOST: usize = 4 << 20;
/// How long a check may take before it's taken as not playing: they take a tenth of a second.
const CHECK_PATIENCE: Duration = Duration::from_secs(20);
/// A vanilla Doom 1.9 demo's first byte, which is all POOM plays.
const VANILLA: u8 = 109;

/// One of the archive's demos, and what came of it here.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Demo {
    /// The archive's number for it.
    pub id: u64,
    /// The map (E1M2, MAP07).
    pub level: String,
    /// UV Speed, Pacifist, and so on.
    pub category: String,
    /// The time it took, as the archive says it: 0:21.94.
    pub time: String,
    /// Who played it.
    pub players: Vec<String>,
    /// What it was recorded with.
    #[serde(default)]
    pub engine: String,
    /// Where its zip is.
    pub file: String,
    /// Whether it's been fetched and checked yet; and if it has, whether it plays here as it did there.
    #[serde(default)]
    pub checked: bool,
    #[serde(default)]
    pub plays: bool,
    /// When it doesn't play, why.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// How many times the player fires a weapon in it (a held trigger, each time it fires again), as playing it here counted.
    #[serde(default)]
    pub fired: u32,
    /// Which ANALYSIS what's known of it was found by: one found by an older one is played here again (it's kept, so that's no
    /// download), for what the newer one finds out.
    #[serde(default)]
    pub analysis: u32,
    /// The archive's name for the game it was recorded on (doom, doom2, tnt), of the names this game might go by
    /// (`archive_names`). Empty in what an older version kept: the one name there was then.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub wad: String,
}

/// What checking a demo finds out, as of this version: 1, whether it plays here as it did there; 2, and how many times the player
/// fires. A newer version, finding out more, has every demo that plays checked again.
pub const ANALYSIS: u32 = 2;

impl Demo {
    /// Its leaderboard on the archive: the map and category, with this run among them.
    pub fn page(&self) -> String {
        let plus = |s: &str| s.replace(' ', "+");
        format!("https://dsdarchive.com/wads/{}/leaderboard?category={}&level={}", self.wad, plus(&self.category), plus(&self.level))
    }

    fn lmp(&self) -> PathBuf {
        folder().join(format!("{}.lmp", self.id))
    }
}

/// What's known for one game.
#[derive(Serialize, Deserialize, Default)]
struct Game {
    sha256: String,
    /// When the archive was last asked (seconds since 1970).
    #[serde(default)]
    listed: u64,
    #[serde(default)]
    demo: Vec<Demo>,
    /// Its maps, as its own directory lists them (`pms-poom --maps`): which of the archive's runs it could play.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    maps: Vec<String>,
    /// The archive's name for it, once a demo from there has played here; until then, the names it might go by are tried in
    /// turn, and those given up on are `tried`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    wad: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tried: Vec<String>,
}

/// How many of a name's demos are checked, while it's only a name the game might go by, before it's given up on if none plays:
/// a game that isn't that one plays none of them (each leaves its level at another tic, or never does), and one that is plays
/// nearly all of them.
const TRY: usize = 5;

#[derive(Serialize, Deserialize, Default)]
struct Record {
    #[serde(default)]
    game: Vec<Game>,
}

/// Where demos are kept: among the games, where the game's sandbox may read them.
pub fn folder() -> PathBuf {
    crate::wads::dir().join("demos")
}

fn record_file() -> PathBuf {
    folder().join("demos.toml")
}

fn load() -> Record {
    std::fs::read_to_string(record_file()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
}

fn save(record: &Record) -> Result<(), String> {
    let text = toml::to_string(record).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(folder()).map_err(|e| e.to_string())?;
    let writing = folder().join("demos.toml-writing");
    std::fs::write(&writing, text).and_then(|()| std::fs::rename(&writing, record_file())).map_err(|e| e.to_string())
}

/// Freedoom's games: the archive's runs on them are all for an older version, whose maps have changed (0.11, where this has 0.13).
const FREE: [&str; 3] = ["freedoom1.wad", "freedoom2.wad", "freedm.wad"];

fn is_free(iwad: &Path) -> bool {
    iwad.file_name().and_then(|n| n.to_str()).is_some_and(|n| FREE.contains(&n.to_ascii_lowercase().as_str()))
}

/// The names the archive might have the game in `iwad` under, the likeliest first: by its file's name where that's one of id's
/// games, else by the maps it has (`maps`, from its own directory), whatever it's called: episodes are Doom's, MAPxx one of
/// Doom II's three (Doom II, Plutonia, TNT). It's the demos that say which it is: they play only on the game they were
/// recorded on. None for Freedoom, or a game with no maps.
pub fn archive_names(iwad: &Path, maps: &[String]) -> Vec<&'static str> {
    let name = iwad.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
    match name.as_str() {
        "doom1.wad" | "doom.wad" => vec!["doom"],
        "doom2.wad" => vec!["doom2"],
        "tnt.wad" => vec!["tnt"],
        "plutonia.wad" => vec!["plutonia"],
        _ if is_free(iwad) => Vec::new(),
        _ if maps.iter().any(|m| m.starts_with('E')) => vec!["doom"],
        _ if maps.iter().any(|m| m.starts_with("MAP")) => vec!["doom2", "plutonia", "tnt"],
        _ => Vec::new(),
    }
}

/// Which of `names` the demos are to come from now: the one the game's been found to be, once a demo from there has played
/// here; until then, the likeliest not given up on; None once each has been.
fn settle_on(game: &mut Game, names: &[&'static str]) -> Option<&'static str> {
    // What an older version kept came from the one name there was then.
    if let Some(first) = names.first() {
        for d in game.demo.iter_mut().filter(|d| d.wad.is_empty()) {
            d.wad = first.to_string();
        }
    }
    if game.wad.is_empty()
        && let Some(d) = game.demo.iter().find(|d| d.plays)
    {
        game.wad = d.wad.clone();
    }
    if !game.wad.is_empty() {
        return names.iter().find(|n| **n == game.wad).copied();
    }
    names.iter().find(|n| !game.tried.iter().any(|t| t == **n)).copied()
}

/// Whether the archive might yet have demos that play on the game in `iwad`: not for Freedoom, nor once every name it might go
/// by has been tried and none had any. For the settings to say so.
pub fn might_have(iwad: &Path) -> bool {
    let free = is_free(iwad);
    let Some(sha) = sha256(iwad) else { return !free };
    match load().game.into_iter().find(|g| g.sha256 == sha) {
        Some(g) if !g.maps.is_empty() => g.demo.iter().any(|d| d.plays) || archive_names(iwad, &g.maps).iter().any(|n| !g.tried.iter().any(|t| t == n)),
        // Not looked into yet.
        _ => !free,
    }
}

/// A file's SHA-256, kept for as long as the file stays the same size and age: the settings ask often.
fn sha256(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    type Known = Vec<(PathBuf, u64, std::time::SystemTime, String)>;
    static KNOWN: Mutex<Known> = Mutex::new(Vec::new());
    let meta = std::fs::metadata(path).ok()?;
    let (size, modified) = (meta.len(), meta.modified().ok()?);
    let mut known = KNOWN.lock().ok()?;
    if let Some((.., sha)) = known.iter().find(|(p, s, m, _)| p == path && *s == size && *m == modified) {
        return Some(sha.clone());
    }
    let sha: String = Sha256::digest(std::fs::read(path).ok()?).iter().map(|b| format!("{b:02x}")).collect();
    known.retain(|(p, ..)| p != path);
    known.push((path.to_path_buf(), size, modified, sha.clone()));
    Some(sha)
}

/// What's known of the demos for the game in `iwad`: all of them, checked or not.
pub fn known(iwad: &Path) -> Vec<Demo> {
    let Some(sha) = sha256(iwad) else { return Vec::new() };
    let Some(mut game) = load().game.into_iter().find(|g| g.sha256 == sha) else { return Vec::new() };
    // What an older version kept, before the next go says where it came from: the one name there was then.
    let names = archive_names(iwad, &game.maps);
    settle_on(&mut game, &names);
    game.demo
}

/// The list the console reads each time it starts its game (`play.txt`, beside the demos): for each game in the games' folder,
/// each demo that's been checked and plays on it, as `<the game's file>\t<the demo's file>`. It picks a few for its title loop
/// from those for the game it's starting. Gone while demos aren't wanted, so that none are played.
pub fn play_list() -> PathBuf {
    folder().join("play.txt")
}

/// What the PIMPBOX card says about the demo in `file` (one of ours, `<id>.lmp`), as the TV's info lines: `demo` (map, category
/// and time), `runner` and `demo_about` (what it was recorded with, and how often it fires). Nothing for a file that isn't one.
pub fn showing(file: &Path) -> String {
    let Some(id) = file.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u64>().ok()) else { return String::new() };
    let Some(d) = load().game.into_iter().flat_map(|g| g.demo).find(|d| d.id == id) else { return String::new() };
    let one_line = |s: &str| s.replace(['\t', '\r', '\n'], " ");
    format!(
        "demo\t{} {} {}\nrunner\t{}\ndemo_about\t{}{}{}\n",
        one_line(&d.level),
        one_line(&d.category),
        one_line(&d.time),
        one_line(&d.players.join(", ")),
        one_line(&d.engine),
        if d.engine.is_empty() { "" } else { " · " },
        one_line(&crate::lang::pimpbox::FIRES.with(&[("n", &d.fired)]))
    )
}

/// Whether `demo` is one to show, by the settings' `fired_at_least`: it plays here, and the player fires at least that often in it.
pub fn shown(demo: &Demo, fired_at_least: u32) -> bool {
    demo.plays && demo.fired >= fired_at_least
}

/// Writes `play_list` as things are now: what plays and is to be shown, if demos are `on`; nothing, if they aren't.
fn publish(on: bool) {
    if !on {
        let _ = std::fs::remove_file(play_list());
        return;
    }
    let fired_at_least = FIRED_AT_LEAST.load(Ordering::Relaxed);
    let record = load();
    let mut lines = String::new();
    for name in crate::wads::list() {
        let iwad = crate::wads::dir().join(&name);
        let Some(sha) = sha256(&iwad) else { continue };
        let Some(game) = record.game.iter().find(|g| g.sha256 == sha) else { continue };
        for demo in game.demo.iter().filter(|d| shown(d, fired_at_least)).map(Demo::lmp).filter(|p| p.is_file()) {
            lines.push_str(&format!("{}\t{}\n", iwad.display(), demo.display()));
        }
    }
    if std::fs::create_dir_all(folder()).is_ok() {
        let writing = folder().join("play.txt-writing");
        if std::fs::write(&writing, lines).and_then(|()| std::fs::rename(&writing, play_list())).is_err() {
            crate::log("demos: the console's list couldn't be written");
        }
    }
}

/// Whether the archive's time (`0:21.94`, or in whole seconds, `0:46`) is the level's clock at `tics`: to the hundredth (the
/// archive's rounded or cut short; a tic is nearly three), or to the second where that's all it says.
pub fn same_time(archive: &str, tics: u32) -> bool {
    let (whole, hundredths) = match archive.trim().split_once('.') {
        Some((w, h)) => (w, Some(h)),
        None => (archive.trim(), None),
    };
    let Some(seconds) = whole.split(':').try_fold(0u64, |sum, part| part.parse::<u64>().ok().map(|n| sum * 60 + n)) else { return false };
    let tics = u64::from(tics);
    match hundredths.map(str::parse::<u64>) {
        Some(Ok(h)) if h < 100 => {
            let said = seconds * 100 + h;
            tics * 100 / 35 == said || (tics * 100 + 17) / 35 == said
        }
        Some(_) => false,
        None => tics / 35 == seconds,
    }
}

/// Whether a go is under way, and when the last one began (and for which game); and whether demos are wanted, as the settings
/// last said.
static BUSY: AtomicBool = AtomicBool::new(false);
static LAST: Mutex<Option<(Instant, PathBuf)>> = Mutex::new(None);
static WANTED: AtomicBool = AtomicBool::new(false);
/// How often the player has to fire in a demo for it to be shown, as the settings last said.
static FIRED_AT_LEAST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Keeps the demos up to date while the settings want them (`console_demos`): a go now and then, on a thread of its own, to ask
/// the archive what's new and fetch and check what's not been yet, as often and as much as the settings say (`[demos]`), and
/// only while the console's on (`console_on`) if that's what they say. Costs nothing when it isn't time.
pub fn keep_up(config: &crate::config::Config, console_on: bool) {
    let on = config.console_demos;
    let how = config.demos();
    WANTED.store(on, Ordering::Relaxed);
    // Switched on or off, or how much shooting it takes changed (or this is the first look): the console's list says so at once.
    static WAS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
    let now_is = u64::from(on) << 32 | u64::from(how.fired_at_least);
    FIRED_AT_LEAST.store(how.fired_at_least, Ordering::Relaxed);
    if WAS.swap(now_is, Ordering::Relaxed) != now_is && !BUSY.load(Ordering::Acquire) {
        publish(on);
    }
    if !on || BUSY.load(Ordering::Acquire) || (how.when == "console" && !console_on) {
        return;
    }
    let Some(iwad) = crate::wads::chosen() else { return };
    let Ok(mut last) = LAST.lock() else { return };
    let every = Duration::from_secs(u64::from(how.every_hours.clamp(1, 168)) * 3600);
    if last.as_ref().is_some_and(|(at, game)| *game == iwad && at.elapsed() < every) {
        return;
    }
    *last = Some((Instant::now(), iwad.clone()));
    BUSY.store(true, Ordering::Release);
    let spawned = std::thread::Builder::new().name("demos".into()).spawn(move || {
        if let Err(e) = go(&iwad, &how) {
            crate::log(format!("demos: {e}"));
        }
        BUSY.store(false, Ordering::Release);
    });
    if spawned.is_err() {
        BUSY.store(false, Ordering::Release);
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// One go, for the game in `iwad`, as much as `how` says.
fn go(iwad: &Path, how: &crate::config::Demos) -> Result<(), String> {
    let gap = Duration::from_secs(u64::from(how.gap_seconds.clamp(1, 60)));
    let sha = sha256(iwad).ok_or("the game couldn't be read")?;
    let mut record = load();
    let at = match record.game.iter().position(|g| g.sha256 == sha) {
        Some(at) => at,
        None => {
            record.game.push(Game { sha256: sha.clone(), ..Default::default() });
            record.game.len() - 1
        }
    };
    // What game it is, whatever it's called: its maps, read once (in the game's sandbox), then the names it might go by.
    if record.game[at].maps.is_empty() && !is_free(iwad) {
        record.game[at].maps = maps_in(iwad)?;
        save(&record)?;
    }
    let names = archive_names(iwad, &record.game[at].maps);
    let Some(wad) = settle_on(&mut record.game[at], &names) else { return Ok(()) };
    let trying = record.game[at].wad.is_empty();
    let maps = record.game[at].maps.clone();
    let most = how.most.clamp(1, 500) as usize;
    // Only so many of a name that's being tried: most of them are no use if it's the wrong one.
    let wanted = if trying { TRY } else { most };
    let of_it = |g: &Game| g.demo.iter().filter(|d| d.wad == wad).count();
    let list_every = u64::from(how.list_days.clamp(1, 90)) * 24 * 3600;
    if of_it(&record.game[at]) < wanted && now().saturating_sub(record.game[at].listed) >= list_every {
        let listed = listed(wad, &maps, gap)?;
        let game = &mut record.game[at];
        let new: Vec<Demo> = listed.into_iter().filter(|d| !game.demo.iter().any(|k| k.id == d.id)).collect();
        let found = new.len();
        // As many as are still wanted, of the ones most worth it.
        let room = wanted - of_it(game);
        game.demo.extend(worth_most(new, room));
        crate::log(format!(
            "demos: the archive has {found} new for {wad} that this game could play{}; {} to be checked",
            if trying { ", if it's that one" } else { "" },
            game.demo.iter().filter(|d| !d.checked).count()
        ));
        game.listed = now();
        if trying && of_it(game) == 0 {
            give_up(game, wad);
        }
        save(&record)?;
    }
    // Fewer wanted than are waiting (the settings changed): the ones least worth it go. What's been checked stays, so that it's
    // never fetched again.
    let game = &mut record.game[at];
    if of_it(game) > wanted {
        let (kept, waiting): (Vec<Demo>, Vec<Demo>) = std::mem::take(&mut game.demo).into_iter().partition(|d| d.checked || d.wad != wad);
        let room = wanted.saturating_sub(kept.iter().filter(|d| d.wad == wad).count());
        game.demo = kept;
        game.demo.extend(worth_most(waiting, room));
        save(&record)?;
    }
    // What an older analysis found out about isn't all there is to know now: the demos that play are played again, here, from
    // what's kept (no download), for what the newer one finds out. One whose file's gone is fetched again, in its turn.
    let mut again = 0;
    for i in 0..record.game[at].demo.len() {
        let d = &record.game[at].demo[i];
        if !d.checked || d.analysis >= ANALYSIS {
            continue;
        }
        if !d.plays {
            record.game[at].demo[i].analysis = ANALYSIS;
            continue;
        }
        if !d.lmp().is_file() {
            record.game[at].demo[i].checked = false;
            continue;
        }
        let demo = d.clone();
        if let Ok(found) = checked(iwad, &demo) {
            settle(&mut record.game[at].demo[i], found);
            again += 1;
        }
    }
    if again > 0 {
        crate::log(format!("demos: checked {again} again, for what's found out about them now (analysis {ANALYSIS})"));
        save(&record)?;
    }
    let at_once = how.at_once.clamp(1, 50) as usize;
    let waiting: Vec<usize> = (0..record.game[at].demo.len()).filter(|i| !record.game[at].demo[*i].checked && record.game[at].demo[*i].wad == wad).take(at_once).collect();
    let (mut plays, mut doesnt) = (0, 0);
    for i in waiting {
        let demo = record.game[at].demo[i].clone();
        let came = fetched_and_checked(iwad, &demo);
        let d = &mut record.game[at].demo[i];
        match came {
            Ok(found) => {
                if found.0.is_none() {
                    plays += 1;
                } else {
                    doesnt += 1;
                }
                settle(d, found);
            }
            // Not this time (the archive, or the network): it's tried again on another go.
            Err(e) => {
                crate::log(format!("demos: {} {} ({}) couldn't be fetched or checked this time: {e}", demo.level, demo.category, demo.id));
                let _ = std::fs::remove_file(demo.lmp());
                break;
            }
        }
        save(&record)?;
        // One at a time, and not in a rush: it's somebody else's archive.
        std::thread::sleep(gap);
    }
    if plays + doesnt > 0 {
        crate::log(format!("demos: checked {}: {plays} play here, {doesnt} don't", plays + doesnt));
    }
    // A name that was being tried: one of its demos plays, so that's the game; or as many as are tried have been checked and
    // none does, so it isn't, and the next name's listed on the next go.
    let game = &mut record.game[at];
    if trying {
        let of_wad = || game.demo.iter().filter(|d| d.wad == wad);
        if of_wad().any(|d| d.plays) {
            game.wad = wad.to_string();
            crate::log(format!("demos: this game is the archive's {wad}"));
            save(&record)?;
        } else if of_wad().all(|d| d.checked) && of_wad().count() > 0 {
            give_up(game, wad);
            save(&record)?;
        }
    }
    // What plays now, for the console's next game: unless they've been switched off meanwhile.
    publish(WANTED.load(Ordering::Relaxed));
    Ok(())
}

/// `wad` isn't what `game` is: none of its demos play here. What's waiting from there goes; the next name's listed on the next go.
fn give_up(game: &mut Game, wad: &str) {
    crate::log(format!("demos: this game isn't the archive's {wad}"));
    game.demo.retain(|d| d.checked || d.wad != wad);
    game.tried.push(wad.to_string());
    game.listed = 0;
}

/// The maps in the game in `iwad`, as its directory lists them (pms-poom.exe --maps, in the game's sandbox if it has one: it's
/// a file from anywhere).
fn maps_in(iwad: &Path) -> Result<Vec<String>, String> {
    let said = poom(&["--maps".as_ref(), iwad.as_os_str()])?.ok_or("its maps couldn't be read")?;
    let line = said.lines().find_map(|l| l.strip_prefix("maps")).ok_or("its maps couldn't be read")?;
    Ok(line.split_whitespace().map(str::to_string).collect())
}

/// Puts what checking `demo` found into what's known of it; one that doesn't play isn't kept.
fn settle(demo: &mut Demo, (why, fired): Found) {
    demo.checked = true;
    demo.plays = why.is_none();
    demo.why = why.unwrap_or_default();
    demo.fired = fired;
    demo.analysis = ANALYSIS;
    if !demo.plays {
        let _ = std::fs::remove_file(demo.lmp());
    }
}

/// `n` of `demos`, the ones most worth watching: by category (CATEGORIES' order, any other after), each category across every
/// map before the next, so that a few are a spread and not one map's.
fn worth_most(mut demos: Vec<Demo>, n: usize) -> Vec<Demo> {
    let rank = |d: &Demo| CATEGORIES.iter().position(|c| c.eq_ignore_ascii_case(&d.category)).unwrap_or(CATEGORIES.len());
    demos.sort_by(|a, b| (rank(a), &a.level, a.id).cmp(&(rank(b), &b.level, b.id)));
    demos.truncate(n);
    demos
}

/// What the archive lists for `wad`, one player's, not tool-assisted, not under suspicion, of a map this game has, and nothing
/// else: none of them checked yet.
fn listed(wad: &str, maps: &[String], gap: Duration) -> Result<Vec<Demo>, String> {
    let has = |level: &str| maps.iter().any(|m| m == level);
    #[derive(Deserialize)]
    struct Page {
        demos: Vec<Listed>,
        #[serde(default)]
        total_pages: u32,
    }
    #[derive(Deserialize)]
    struct Listed {
        id: u64,
        #[serde(default)]
        players: Vec<String>,
        time: String,
        category: String,
        level: String,
        #[serde(default)]
        engine: Option<String>,
        #[serde(default)]
        tas: bool,
        #[serde(default)]
        guys: u32,
        #[serde(default)]
        suspect: bool,
        #[serde(default)]
        cheated: bool,
        #[serde(default)]
        file: Option<String>,
    }
    let mut found = Vec::new();
    let mut page = 1;
    loop {
        let url = format!("{API}?wad={wad}&only_records=true&per=200&page={page}");
        let got = crate::fetch::fetch(crate::net::Use::Downloads, crate::fetch::Kind::Get, &url, Duration::from_secs(60), 3, MOST)?;
        if got.status != 200 {
            return Err(format!("the archive answered {}", got.status));
        }
        let listing: Page = serde_json::from_slice(&got.body).map_err(|e| format!("the archive's list: {e}"))?;
        for d in listing.demos {
            let file = d.file.unwrap_or_default();
            if d.tas || d.guys != 1 || d.suspect || d.cheated || !has(&d.level) || !file.starts_with(FILES) || file.contains(char::is_whitespace) {
                continue;
            }
            found.push(Demo { id: d.id, level: d.level, category: d.category, time: d.time, players: d.players, engine: d.engine.unwrap_or_default(), file, checked: false, plays: false, why: String::new(), fired: 0, analysis: 0, wad: wad.to_string() });
        }
        // Not without end, whatever it says.
        if page >= listing.total_pages.min(20) {
            break;
        }
        page += 1;
        std::thread::sleep(gap);
    }
    Ok(found)
}

/// What checking a demo found: None if it plays here as it did there, or why it doesn't; and how many times the player fired.
type Found = (Option<String>, u32);

/// Fetches `demo` and checks it on the game in `iwad`; an error if that couldn't be found out this time.
fn fetched_and_checked(iwad: &Path, demo: &Demo) -> Result<Found, String> {
    let doesnt = |why: String| Ok((Some(why), 0));
    let got = match crate::fetch::fetch(crate::net::Use::Downloads, crate::fetch::Kind::Lmp, &demo.file, Duration::from_secs(120), 3, MOST) {
        Ok(got) => got,
        // What came is no use, and would be the same next time: it doesn't play here.
        Err(e) if e.starts_with(crate::fetch::UNUSABLE) => return doesnt(e[crate::fetch::UNUSABLE.len()..].to_string()),
        Err(e) => return Err(e),
    };
    match got.status {
        200 => {}
        404 | 410 => return doesnt("the archive doesn't have it any more".into()),
        other => return Err(format!("the archive answered {other}")),
    }
    match got.body.first() {
        Some(&VANILLA) => {}
        Some(other) => return doesnt(format!("it was recorded for another engine (version {other})")),
        None => return doesnt("it's empty".into()),
    }
    std::fs::create_dir_all(folder()).map_err(|e| e.to_string())?;
    std::fs::write(demo.lmp(), &got.body).map_err(|e| e.to_string())?;
    checked(iwad, demo)
}

/// Checks `demo`, which is here already, on the game in `iwad`.
fn checked(iwad: &Path, demo: &Demo) -> Result<Found, String> {
    Ok(match left_at(iwad, &demo.lmp())? {
        Some(left) if same_time(&demo.time, left.tics) => (None, left.fired),
        Some(left) => (Some(format!("it left {} at {} tics here, not at {}", demo.level, left.tics, demo.time)), left.fired),
        None => (Some(format!("it never left {} here", demo.level)), 0),
    })
}

/// When a demo left its level, by the level's clock in tics, and how many times the player had fired a weapon by then.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Left {
    tics: u32,
    fired: u32,
}

/// What `pms-poom --check` says when the level's been left: `left 869`, and `fired 42` (a line each, so that an older daemon,
/// which knows only the first, still reads it; an older pms-poom says only the first).
fn left_said(said: &str) -> Option<Left> {
    let tics = said.lines().find_map(|l| l.strip_prefix("left ")?.trim().parse().ok())?;
    let fired = said.lines().find_map(|l| l.strip_prefix("fired ")?.trim().parse().ok()).unwrap_or(0);
    Some(Left { tics, fired })
}

/// Plays `demo` on the game in `iwad`, unseen (pms-poom.exe --check, in the game's sandbox if it has one): when it left its
/// level and how many times the player fired, or None if it never did.
fn left_at(iwad: &Path, demo: &Path) -> Result<Option<Left>, String> {
    Ok(poom(&["--check".as_ref(), iwad.as_os_str(), demo.as_os_str()])?.and_then(|said| left_said(&said)))
}

/// What pms-poom.exe says, given `args`, run unseen in the game's sandbox if it has one; None if it took longer than
/// CHECK_PATIENCE (it's stopped then).
fn poom(args: &[&std::ffi::OsStr]) -> Result<Option<String>, String> {
    let exe = crate::parts::path(crate::parts::POOM).filter(|p| p.is_file()).ok_or("pms-poom.exe isn't there")?;
    let saves = crate::wads::saves();
    let command = args.iter().fold(pms_spawn::Command::new(exe), |c, a| c.arg(a)).current_dir(&saves).stdout(pms_spawn::Stdio::Piped).power(crate::efficiency::flags(crate::efficiency::Part::Helpers));
    let command = if crate::sandbox::wanted(crate::sandbox::Helper::Poom) {
        let c = crate::sandbox::container(crate::sandbox::Helper::Poom, &[(&crate::wads::dir(), false), (&saves, true)], false)?;
        command.container(c).mitigations(pms_spawn::mitigate::USUAL | pms_spawn::mitigate::NO_WIN32K)
    } else {
        command
    };
    let mut child = command.spawn()?;
    let Some(mut out) = child.stdout.take() else { return Err("no output".into()) };
    let reading = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut out, &mut text);
        text
    });
    let began = Instant::now();
    while child.try_wait().is_none() {
        if began.elapsed() > CHECK_PATIENCE {
            child.kill();
            child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(Some(reading.join().unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_is_the_archives_to_the_tic() {
        // E1M2 UV Speed, 0:21.94, left at 768 tics (21.942...).
        assert!(same_time("0:21.94", 768));
        // Cut short or rounded: 869 tics is 24.828..., which the archive says as 24.83, and some as 24.82.
        assert!(same_time("0:24.83", 869) && same_time("0:24.82", 869));
        assert!(!same_time("0:24.83", 870) && !same_time("0:24.83", 868), "a tic either way is another time");
        assert!(same_time("2:43.17", 5711));
        assert!(same_time("0:46", 46 * 35 + 34) && !same_time("0:46", 47 * 35), "whole seconds, where that's all it says");
        assert!(same_time("1:00:00.00", 3600 * 35));
        for bad in ["", "soon", "0:21.9x", "0:21.940", ":"] {
            assert!(!same_time(bad, 768), "{bad}");
        }
    }

    #[test]
    fn a_game_is_known_by_its_name_or_else_by_its_maps() {
        let maps = |m: &[&str]| m.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let episodes = maps(&["E1M1", "E1M2"]);
        let numbered = maps(&["MAP01", "MAP32"]);
        assert_eq!(archive_names(Path::new(r"C:\games\DOOM1.WAD"), &episodes), ["doom"]);
        assert_eq!(archive_names(Path::new("tnt.wad"), &[]), ["tnt"], "id's names say which, before its maps are read");
        assert_eq!(archive_names(Path::new("mine.wad"), &episodes), ["doom"]);
        assert_eq!(archive_names(Path::new("mine.wad"), &numbered), ["doom2", "plutonia", "tnt"], "the demos say which of the three");
        assert!(archive_names(Path::new("mine.wad"), &[]).is_empty() && archive_names(Path::new("Freedoom2.wad"), &numbered).is_empty());
    }

    fn demo(id: u64, wad: &str, checked: bool, plays: bool) -> Demo {
        Demo { id, level: "MAP01".into(), category: "UV Speed".into(), time: "0:10.00".into(), players: vec![], engine: String::new(), file: String::new(), checked, plays, why: String::new(), fired: 0, analysis: ANALYSIS, wad: wad.into() }
    }

    #[test]
    fn the_names_a_game_might_go_by_are_tried_until_one_plays() {
        let names = ["doom2", "plutonia", "tnt"];
        let mut game = Game::default();
        assert_eq!(settle_on(&mut game, &names), Some("doom2"));
        game.demo = vec![demo(1, "doom2", true, false), demo(2, "doom2", false, false)];
        give_up(&mut game, "doom2");
        assert_eq!(game.demo.len(), 1, "what was waiting from there goes; what was checked stays, never to be fetched again");
        assert_eq!(settle_on(&mut game, &names), Some("plutonia"));
        game.demo.push(demo(3, "plutonia", true, true));
        assert_eq!(settle_on(&mut game, &names), Some("plutonia"));
        assert_eq!(game.wad, "plutonia", "a demo of it played: that's the game");
        game.tried.push("tnt".into());
        assert_eq!(settle_on(&mut game, &names), Some("plutonia"));
        let mut hopeless = Game { tried: names.iter().map(|n| n.to_string()).collect(), ..Default::default() };
        assert_eq!(settle_on(&mut hopeless, &names), None);
    }

    #[test]
    fn what_an_older_version_kept_is_from_the_one_name_there_was() {
        let mut game = Game { demo: vec![demo(1, "", true, true)], ..Default::default() };
        assert_eq!(settle_on(&mut game, &["doom"]), Some("doom"));
        assert_eq!((game.wad.as_str(), game.demo[0].wad.as_str()), ("doom", "doom"));
    }

    #[test]
    fn a_demos_page_is_its_leaderboard() {
        let d = Demo { id: 1, level: "E1M2".into(), category: "UV Speed".into(), time: "0:21.94".into(), players: vec![], engine: String::new(), file: String::new(), checked: true, plays: true, why: String::new(), fired: 0, analysis: ANALYSIS, wad: "doom".into() };
        assert_eq!(d.page(), "https://dsdarchive.com/wads/doom/leaderboard?category=UV+Speed&level=E1M2");
    }

    #[test]
    fn what_a_check_says_is_read() {
        assert_eq!(left_said("Playing demo x.\nleft 869\nfired 42\n"), Some(Left { tics: 869, fired: 42 }));
        assert_eq!(left_said("left 768\n"), Some(Left { tics: 768, fired: 0 }), "an older pms-poom");
        assert_eq!(left_said("timed 5711 gametics\n"), None, "never left");
        assert_eq!(left_said("left soon\nfired 3"), None);
    }

    #[test]
    fn a_demo_is_shown_if_it_plays_and_there_was_shooting_enough() {
        let mut d = Demo { fired: 12, ..demo(1, "doom", true, true) };
        assert!(shown(&d, 0) && shown(&d, 12) && !shown(&d, 13));
        d.plays = false;
        assert!(!shown(&d, 0));
    }
}
