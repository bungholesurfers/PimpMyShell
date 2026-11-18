//! Efficiency mode, Windows' EcoQoS, for PimpMyShell's own processes, as the settings say (`[efficiency]`): the slowest clock
//! speed that will do, and on a processor that has them (Intel's since its 12th generation, say), the efficient cores. Windows
//! 11 does this on mains power too; Windows 10 only on battery. None of what's put in it has to be quick, only on time, and
//! that it is with plenty to spare: a meter once a second, a TV's frame every 33 ms that takes well under one, a download.
//!
//! What's put in it, each unless the settings say not:
//!
//! - **The daemon**, as soon as it's read its settings, and again whenever they change.
//! - **The helpers**, each as it's started (pms-spawn sets it before the helper runs any of its own code): pms-fetch,
//!   Streamlink, and the PIMPBOX's games and demos being looked at.
//! - **Twatch TV's players**, as they're started, and again when the settings change.
//! - **The PIMPBOX and its game**, by the console itself, which knows when its window's open: by default, while it's closed.
//!   A game you're playing is Windows' to decide, as it would be any other.
//!
//! Each asks too that the finer timers it, or something in it, might ask for are ignored, so that Windows doesn't wake the
//! processor more often for them; but for the players, where Windows' own media stack may want them.

use std::sync::RwLock;

use pms_spawn::power;

use crate::config::{Config, Efficiency};

/// What's put in efficiency mode by a switch of its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Daemon,
    Twatch,
    Helpers,
}

/// What the settings last said: for what starts helpers away from the settings.
static WANTED: RwLock<Option<Efficiency>> = RwLock::new(None);

/// Takes up what the settings say (net::follow does this, for whatever starts a helper).
pub fn follow(config: &Config) {
    if let Ok(mut w) = WANTED.write() {
        *w = Some(config.efficiency.clone().unwrap_or_default());
    }
}

fn wanted() -> Efficiency {
    WANTED.read().ok().and_then(|w| w.clone()).unwrap_or_default()
}

/// What `part` is started with (pms_spawn's `power`), as the settings last said.
pub fn flags(part: Part) -> u32 {
    flags_of(&wanted(), part)
}

/// The same, as these settings say.
pub fn flags_in(config: &Config, part: Part) -> u32 {
    flags_of(&config.efficiency.clone().unwrap_or_default(), part)
}

fn flags_of(e: &Efficiency, part: Part) -> u32 {
    let (on, timers) = match part {
        Part::Daemon => (e.daemon, power::COARSE_TIMERS),
        Part::Helpers => (e.helpers, power::COARSE_TIMERS),
        // Windows' media stack may ask for finer timers, for a picture on time: those it gets.
        Part::Twatch => (e.twatch, 0),
    };
    if on { power::EFFICIENT | timers } else { power::WINDOWS }
}

/// When the PIMPBOX and its game are in efficiency mode: "closed" (while its window is), "always" or "never".
pub fn pimpbox() -> String {
    let how = wanted().pimpbox;
    if crate::config::PIMPBOX_EFFICIENT.contains(&how.as_str()) { how } else { Efficiency::default().pimpbox }
}

/// Puts the daemon (this process) as the settings say: said in the log when that changes.
pub fn own(config: &Config) {
    static SET: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::MAX);
    follow(config);
    let want = flags(Part::Daemon);
    if SET.swap(want, std::sync::atomic::Ordering::Relaxed) != want {
        let took = pms_spawn::set_own_power(want);
        crate::log(match (want != power::WINDOWS, took) {
            (true, true) => "efficiency mode: on".to_string(),
            (false, true) => "efficiency mode: off, Windows decides".to_string(),
            (_, false) => format!("efficiency mode: Windows wouldn't have it: {}", std::io::Error::last_os_error()),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_is_efficient_until_the_settings_say_not_and_the_players_keep_their_timers() {
        let config = crate::config::parse("").unwrap();
        follow(&config);
        assert_eq!(flags(Part::Helpers), power::EFFICIENT | power::COARSE_TIMERS);
        assert_eq!(flags(Part::Daemon), power::EFFICIENT | power::COARSE_TIMERS);
        assert_eq!(flags(Part::Twatch), power::EFFICIENT);
        assert_eq!(pimpbox(), "closed");
        let config = crate::config::parse("[efficiency]\nhelpers = false\npimpbox = \"always\"\n").unwrap();
        follow(&config);
        assert_eq!(flags(Part::Helpers), power::WINDOWS);
        assert_eq!(flags(Part::Twatch), power::EFFICIENT);
        assert_eq!(pimpbox(), "always");
        follow(&crate::config::parse("").unwrap());
    }
}
