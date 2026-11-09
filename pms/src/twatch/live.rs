//! Who's live, how often to look, and which set shows whom. All of it is decided here, without touching the net or a screen.
//!
//! Looking is one small request for a channel's preview picture: Twitch's CDN has one while a channel is live and redirects to a
//! placeholder while it isn't. That needs no account, no client ID and no Streamlink. Every favorite is looked at about as often as
//! the settings say, whatever its place in the list, in no order: the next is one at random of those due, looks are spread out
//! rather than coming together, and nobody's looked at again before their own wait is up.

use std::time::{Duration, Instant};

/// How often, in seconds, each favorite is looked at, unless you say otherwise.
pub const USUAL_CHECK: u32 = 600;
/// And at most how much longer each one waits, picked afresh after each look, so that looks don't come like clockwork.
pub const USUAL_CHECK_EXTRA: u32 = 300;
/// How much longer it all takes on battery.
const ON_BATTERY: u32 = 2;
/// The least time between two looks at favorites that have never been looked at, as at the start: soon enough to know who's live
/// within moments, without asking all at once.
const FIRST_GAP: Duration = Duration::from_secs(3);
/// The least time between two looks after that, at either end: an interval's looks are spread over half of it, at least.
const LEAST_GAP: Duration = Duration::from_secs(5);
const MOST_GAP: Duration = Duration::from_secs(60);
/// Each failure in a row doubles the wait, this many times at most.
const MOST_DOUBLINGS: u32 = 3;

/// Whether `handle` can be a Twitch channel's name, which also makes it safe to put in an address or on a command line.
pub fn valid(handle: &str) -> bool {
    (1..=25).contains(&handle.len()) && handle.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Favorites written out by hand, however that was: one to a line or not, as names, @names or links to the channel. What comes
/// back is what can be a channel, each once, in the order given, and what can't, as it was written.
pub fn parse_list(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut good, mut bad) = (Vec::new(), Vec::new());
    for word in text.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()) {
        let link = word.split(['?', '#']).next().unwrap_or(word).trim_end_matches('/');
        let name = link.rsplit_once("twitch.tv/").map_or(link, |(_, name)| name);
        let handle = name.trim_start_matches('@').to_ascii_lowercase();
        if !valid(&handle) {
            bad.push(word.to_string());
        } else if !good.contains(&handle) {
            good.push(handle);
        }
    }
    (good, bad)
}

/// Favorites as they can be used: lower case, each once, in the order given, without what can't be a channel.
pub fn tidy(favorites: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for handle in favorites.iter().map(|h| h.trim().trim_start_matches('@').to_ascii_lowercase()) {
        if valid(&handle) && !out.contains(&handle) {
            out.push(handle);
        }
    }
    out
}

/// What's known about a favorite.
#[derive(Clone, Debug)]
pub struct Watch {
    pub handle: String,
    pub live: bool,
    pub looked: Option<Instant>,
    /// A look is under way.
    pub asked: bool,
    pub failures: u32,
    /// It's been seen, one way or the other, so its going live would be news.
    pub known: bool,
    /// When you were last told it went live.
    pub told: Option<Instant>,
    /// How much longer than usual it waits before the next look: picked afresh after each one.
    pub extra: Duration,
}

impl Watch {
    pub fn new(handle: &str) -> Watch {
        Watch { handle: handle.to_string(), live: false, looked: None, asked: false, failures: 0, known: false, told: None, extra: Duration::ZERO }
    }
}

/// How long favorite `w` waits between looks: `check`, and its own random extra, twice as long on battery, and longer after each
/// failure in a row.
pub fn wait(w: &Watch, battery: bool, check: Duration) -> Duration {
    (check + w.extra) * if battery { ON_BATTERY } else { 1 } * (1 << w.failures.min(MOST_DOUBLINGS))
}

/// A number nobody could guess, different each time.
pub fn random() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEED: AtomicU64 = AtomicU64::new(0);
    let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    let mut x = SEED.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed) ^ time;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x
}

/// Some time from none up to `most`, different each time.
pub fn extra(most: Duration) -> Duration {
    if most.is_zero() {
        return Duration::ZERO;
    }
    Duration::from_millis(random() % (most.as_millis() as u64 + 1))
}

/// The favorite to look at now, if any: one at random of those due, those never looked at before the rest. Nothing while the last
/// look, at `last`, was too recent. `roll` is the chance that picks.
pub fn next(watches: &[Watch], battery: bool, check: Duration, last: Option<Instant>, now: Instant, roll: u64) -> Option<usize> {
    let due: Vec<usize> = watches.iter().enumerate().filter(|(_, w)| !w.asked && w.looked.is_none_or(|t| now.duration_since(t) >= wait(w, battery, check))).map(|(i, _)| i).collect();
    let first: Vec<usize> = due.iter().copied().filter(|i| watches[*i].looked.is_none()).collect();
    let pick = if first.is_empty() { &due } else { &first };
    let gap = if first.is_empty() { (check / (watches.len().max(1) as u32 * 2)).clamp(LEAST_GAP, MOST_GAP) } else { FIRST_GAP };
    if pick.is_empty() || last.is_some_and(|t| now.duration_since(t) < gap) {
        return None;
    }
    Some(pick[(roll % pick.len() as u64) as usize])
}

/// A set, as far as who it shows goes.
#[derive(Clone, Debug, Default)]
pub struct Slot {
    pub on: bool,
    pub showing: Option<String>,
    /// Picked from the remote: it stays until that channel ends.
    pub pinned: Option<String>,
    /// The one channel this set shows, and it shows nothing else.
    pub only: Option<String>,
}

/// Who each set should show, given who's live, best first: the best ones that fit, and a set that's already showing one of those
/// carries on with it, rather than two sets swapping streams.
pub fn assign(slots: &[Slot], live: &[String]) -> Vec<Option<String>> {
    // A set that's kept to a channel is out of the running, and so is its channel.
    let kept: Vec<&String> = slots.iter().filter(|s| s.on).filter_map(|s| s.only.as_ref()).collect();
    let free: Vec<Slot> = slots.iter().map(|s| Slot { on: s.on && s.only.is_none(), ..s.clone() }).collect();
    let open: Vec<String> = live.iter().filter(|h| !kept.contains(h)).cloned().collect();
    let mut out = share_out(&free, &open);
    for (i, slot) in slots.iter().enumerate().filter(|(_, s)| s.on) {
        if let Some(only) = &slot.only {
            out[i] = Some(only.clone()).filter(|h| live.contains(h));
        }
    }
    out
}

fn share_out(slots: &[Slot], live: &[String]) -> Vec<Option<String>> {
    let room = slots.iter().filter(|s| s.on).count();
    let is_live = |h: &String| live.contains(h);
    let mut wanted: Vec<&String> = slots.iter().filter(|s| s.on).filter_map(|s| s.pinned.as_ref()).filter(|h| is_live(h)).collect();
    for handle in live {
        if wanted.len() < room && !wanted.contains(&handle) {
            wanted.push(handle);
        }
    }
    let mut out: Vec<Option<String>> = vec![None; slots.len()];
    let mut placed: Vec<&String> = Vec::new();
    for pass in 0..2 {
        for (i, slot) in slots.iter().enumerate().filter(|(_, s)| s.on) {
            let mine = if pass == 0 { slot.pinned.as_ref() } else { slot.showing.as_ref() };
            if out[i].is_none()
                && let Some(handle) = mine.filter(|h| wanted.contains(h) && !placed.contains(h))
            {
                out[i] = Some(handle.clone());
                placed.push(handle);
            }
        }
    }
    let mut rest = wanted.into_iter().filter(|h| !placed.contains(h));
    for (i, _) in slots.iter().enumerate().filter(|(_, s)| s.on) {
        if out[i].is_none() {
            out[i] = rest.next().cloned();
        }
    }
    out
}

/// Where a set stands, for saying what it'll do.
pub struct Standing<'a> {
    /// Every favorite, best first, and the ones that are live.
    pub favorites: &'a [String],
    pub live: &'a [String],
    pub showing: Option<&'a str>,
    /// It stays where it is, whatever else goes live.
    pub held: bool,
    /// You picked this channel, and it's for as long as it lasts.
    pub picked: bool,
    /// It's only ever this channel.
    pub only: Option<&'a str>,
    /// What the other sets are showing.
    pub elsewhere: &'a [&'a str],
    /// A set with nothing on would take the next favorite, and this one would be left alone.
    pub spare_set: bool,
    /// Of the sets that follow your favorites, this is showing the lowest: it's the one that gives way.
    pub gives_way: bool,
}

/// What a set says when nothing's going to move it.
const STAYS: &str = "Stays on until this stream ends";

/// What a set will do next, in a line, for its remote: the question it answers is "will this change by itself?".
pub fn plan(s: &Standing) -> String {
    if let Some(only) = s.only {
        return match s.showing {
            Some(_) => format!("Only ever shows {only}"),
            None => format!("Only ever shows {only}: no signal until they're live"),
        };
    }
    let Some(showing) = s.showing else { return "Comes on when a favorite goes live".into() };
    let at = s.favorites.iter().position(|f| f == showing).unwrap_or(0);
    let rivals: Vec<&String> = s.favorites[..at].iter().filter(|f| !s.elsewhere.contains(&f.as_str())).collect();
    if s.held || s.picked {
        return match rivals.iter().find(|f| s.live.contains(f)) {
            Some(higher) => format!("Staying here, though {higher} ranks higher and is live"),
            None if s.held => "Stays on this channel until it ends".into(),
            None => "You picked this. It stays until it ends".into(),
        };
    }
    match rivals.as_slice() {
        [] => STAYS.into(),
        _ if s.spare_set || !s.gives_way => STAYS.into(),
        [one] => format!("Switches if {one} goes live"),
        [one, two] => format!("Switches if {one} or {two} goes live"),
        _ => "Switches if a higher favorite goes live".into(),
    }
}

/// The live favorite after (`by` 1) or before (`by` -1) `now`, going round.
pub fn step(live: &[String], now: Option<&str>, by: isize) -> Option<String> {
    let n = live.len() as isize;
    if n == 0 {
        return None;
    }
    let at = now.and_then(|h| live.iter().position(|l| l == h)).map_or(if by > 0 { -1 } else { 0 }, |i| i as isize);
    Some(live[(at + by).rem_euclid(n) as usize].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn slot(showing: Option<&str>) -> Slot {
        Slot { on: true, showing: showing.map(str::to_string), pinned: None, only: None }
    }

    #[test]
    fn only_what_can_be_a_channel_is_a_favorite() {
        assert!(valid("crab_99") && !valid("") && !valid("Crab") && !valid("a b") && !valid("a&calc") && !valid("--help") && !valid(&"a".repeat(26)));
        assert_eq!(tidy(&names(&[" @Crab ", "crab", "bad name", "lobster"])), names(&["crab", "lobster"]));
    }

    #[test]
    fn a_list_written_by_hand_is_read_however_it_was_written() {
        let (good, bad) = parse_list("Crab\r\n@lobster, https://www.twitch.tv/Shrimp/ twitch.tv/prawn?sr=a\n\n crab \n not-a-name");
        assert_eq!(good, names(&["crab", "lobster", "shrimp", "prawn"]));
        assert_eq!(bad, names(&["not-a-name"]));
        assert_eq!(parse_list(""), (Vec::new(), Vec::new()));
    }

    fn standing<'a>(favorites: &'a [String], live: &'a [String], showing: Option<&'a str>) -> Standing<'a> {
        Standing { favorites, live, showing, held: false, picked: false, only: None, elsewhere: &[], spare_set: false, gives_way: true }
    }

    #[test]
    fn a_set_says_whether_it_will_change_by_itself() {
        let (favorites, live) = (names(&["a", "b", "c", "d"]), names(&["c"]));
        assert_eq!(plan(&standing(&favorites, &live, None)), "Comes on when a favorite goes live");
        assert_eq!(plan(&standing(&favorites, &live, Some("a"))), STAYS, "it's the best");
        assert_eq!(plan(&standing(&favorites, &live, Some("b"))), "Switches if a goes live");
        assert_eq!(plan(&standing(&favorites, &live, Some("c"))), "Switches if a or b goes live");
        assert_eq!(plan(&standing(&favorites, &live, Some("d"))), "Switches if a higher favorite goes live");
    }

    #[test]
    fn a_set_only_warns_of_what_would_really_move_it() {
        let (favorites, live) = (names(&["a", "b", "c"]), names(&["a", "c"]));
        let elsewhere = ["a"];
        assert_eq!(plan(&Standing { elsewhere: &elsewhere, ..standing(&favorites, &live, Some("c")) }), "Switches if b goes live", "a's on another set");
        assert_eq!(plan(&Standing { spare_set: true, ..standing(&favorites, &live, Some("c")) }), STAYS, "a spare set would take it");
        assert_eq!(plan(&Standing { gives_way: false, ..standing(&favorites, &live, Some("c")) }), STAYS, "another set gives way first");
    }

    #[test]
    fn a_set_that_stays_says_so_and_says_what_it_is_missing() {
        let (favorites, live) = (names(&["a", "b"]), names(&["a", "b"]));
        let none: Vec<String> = Vec::new();
        assert_eq!(plan(&Standing { held: true, ..standing(&favorites, &none, Some("b")) }), "Stays on this channel until it ends");
        assert_eq!(plan(&Standing { picked: true, ..standing(&favorites, &none, Some("b")) }), "You picked this. It stays until it ends");
        assert_eq!(plan(&Standing { held: true, ..standing(&favorites, &live, Some("b")) }), "Staying here, though a ranks higher and is live");
    }

    #[test]
    fn every_favorite_waits_the_same_whatever_its_place() {
        let check = Duration::from_secs(600);
        let mut w = Watch::new("a");
        assert_eq!(wait(&w, false, check), check);
        w.extra = Duration::from_secs(100);
        assert_eq!(wait(&w, false, check), Duration::from_secs(700), "its own extra on top");
        assert_eq!(wait(&w, true, check), Duration::from_secs(1400), "twice that on battery");
        w.failures = 1;
        assert_eq!(wait(&w, false, check), Duration::from_secs(1400));
        w.failures = 9;
        assert_eq!(wait(&w, false, check), Duration::from_secs(5600), "but not forever");
    }

    #[test]
    fn the_next_look_is_one_at_random_of_those_due_and_never_too_soon() {
        let (now, check) = (Instant::now(), Duration::from_secs(600));
        let mut watches = vec![Watch::new("a"), Watch::new("b"), Watch::new("c")];
        let picked: std::collections::HashSet<usize> = (0..30).filter_map(|roll| next(&watches, false, check, None, now, roll)).collect();
        assert_eq!(picked.len(), 3, "any of them, not the first in the list");
        assert_eq!(next(&watches, false, check, Some(now), now + Duration::from_secs(1), 0), None, "not right after another");
        assert!(next(&watches, false, check, Some(now), now + FIRST_GAP, 0).is_some(), "but soon, while none has been looked at");
        watches[0].looked = Some(now);
        watches[1].asked = true;
        assert_eq!(next(&watches, false, check, None, now, 0), Some(2), "not one being looked at, nor one just looked at");
        watches[2].looked = Some(now);
        watches[1] = Watch { looked: Some(now), ..Watch::new("b") };
        assert_eq!(next(&watches, false, check, None, now + check - Duration::from_secs(1), 0), None, "nobody's wait is up");
        assert!(next(&watches, false, check, None, now + check, 0).is_some());
        let gap = (check / 6).min(MOST_GAP);
        assert_eq!(next(&watches, false, check, Some(now + check), now + check + gap - Duration::from_secs(1), 0), None, "looks are spread out");
    }

    #[test]
    fn over_hours_everybody_is_looked_at_in_no_order_and_never_twice_too_soon() {
        let check = Duration::from_secs(600);
        let most_extra = Duration::from_secs(300);
        let start = Instant::now();
        let mut watches: Vec<Watch> = ["a", "b", "c", "d", "e", "f"].iter().map(|h| Watch::new(h)).collect();
        let (mut last, mut looks): (Option<Instant>, Vec<(u64, usize)>) = (None, Vec::new());
        let mut seed = 12345u64;
        for s in 0..4 * 3600u64 {
            let now = start + Duration::from_secs(s);
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            if let Some(i) = next(&watches, false, check, last, now, seed >> 16) {
                let first = watches[i].looked.is_none();
                watches[i].looked = Some(now);
                watches[i].extra = Duration::from_secs((seed >> 20) % if first { check.as_secs() } else { most_extra.as_secs() + 1 });
                last = Some(now);
                looks.push((s, i));
            }
        }
        for i in 0..watches.len() {
            let times: Vec<u64> = looks.iter().filter(|(_, w)| *w == i).map(|(s, _)| *s).collect();
            assert!(times[0] < 20, "everybody's first look comes within moments: {}", times[0]);
            let longest = times.windows(2).map(|p| p[1] - p[0]).max().unwrap();
            let shortest = times.windows(2).map(|p| p[1] - p[0]).min().unwrap();
            assert!(shortest >= check.as_secs(), "nobody twice too soon: {shortest} s");
            assert!(longest <= check.as_secs() * 2 + most_extra.as_secs(), "and nobody left out long: {longest} s");
        }
        let gaps: Vec<u64> = looks.windows(2).skip(6).map(|p| p[1].0 - p[0].0).collect();
        assert!(gaps.iter().all(|g| *g >= LEAST_GAP.as_secs()), "no two looks together");
        let orders: std::collections::HashSet<Vec<usize>> = looks.chunks(6).skip(1).take(8).map(|c| c.iter().map(|(_, i)| *i).collect()).collect();
        assert!(orders.len() > 4, "in no fixed order");
    }

    #[test]
    fn the_best_live_favorite_goes_on_the_one_set() {
        assert_eq!(assign(&[slot(None)], &names(&["a", "b"])), [Some("a".to_string())]);
        assert_eq!(assign(&[slot(Some("b"))], &names(&["a", "b"])), [Some("a".to_string())], "and it takes over when it goes live");
        assert_eq!(assign(&[slot(Some("b"))], &[]), [None]);
    }

    #[test]
    fn a_set_kept_to_a_channel_shows_that_or_nothing() {
        let live = names(&["a", "b"]);
        let only = |h: &str| Slot { only: Some(h.into()), ..slot(None) };
        assert_eq!(assign(&[only("b"), slot(None)], &live), [Some("b".to_string()), Some("a".to_string())], "b is taken, and the other set gets a");
        assert_eq!(assign(&[only("c"), slot(None)], &live), [None, Some("a".to_string())], "c isn't live: it waits, and nothing else goes on it");
        assert_eq!(assign(&[only("a"), slot(Some("a"))], &names(&["a"])), [Some("a".to_string()), None], "nobody else shows its channel");
        let off = Slot { on: false, ..only("a") };
        assert_eq!(assign(&[off, slot(None)], &live), [None, Some("a".to_string())], "turned off, it keeps nothing from the others");
    }

    #[test]
    fn a_set_kept_to_a_channel_says_so() {
        let (favorites, live) = (names(&["a", "b"]), names(&["a"]));
        let waiting = Standing { only: Some("b"), ..standing(&favorites, &live, None) };
        assert_eq!(plan(&waiting), "Only ever shows b: no signal until they're live");
        let showing = Standing { only: Some("a"), ..standing(&favorites, &live, Some("a")) };
        assert_eq!(plan(&showing), "Only ever shows a");
    }

    #[test]
    fn more_sets_show_the_next_best_without_swapping() {
        let got = assign(&[slot(Some("b")), slot(None), slot(None)], &names(&["a", "b", "c", "d"]));
        assert_eq!(got, [Some("b".to_string()), Some("a".to_string()), Some("c".to_string())]);
    }

    #[test]
    fn a_set_thats_off_shows_nobody() {
        let off = Slot { on: false, ..slot(Some("a")) };
        assert_eq!(assign(&[off, slot(None)], &names(&["a", "b"])), [None, Some("a".to_string())]);
    }

    #[test]
    fn a_pick_stays_until_it_ends() {
        let picked = Slot { pinned: Some("b".into()), ..slot(Some("b")) };
        assert_eq!(assign(std::slice::from_ref(&picked), &names(&["a", "b"])), [Some("b".to_string())]);
        assert_eq!(assign(&[picked], &names(&["a"])), [Some("a".to_string())], "it ended");
    }

    #[test]
    fn stepping_goes_round_the_live_favorites() {
        let live = names(&["a", "b", "c"]);
        assert_eq!(step(&live, Some("c"), 1).as_deref(), Some("a"));
        assert_eq!(step(&live, Some("a"), -1).as_deref(), Some("c"));
        assert_eq!(step(&live, None, 1).as_deref(), Some("a"));
        assert_eq!(step(&[], Some("a"), 1), None);
    }
}
