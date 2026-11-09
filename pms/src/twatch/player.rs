//! The player process (pms-twatch), and how high it's allowed to go.
//!
//! A stream comes in renditions. The player gets the master playlist and Windows' adaptive source picks among them by itself as the
//! network allows, under a limit set here: the best the settings allow, one step lower for as long as the machine is busy or the
//! stream keeps stalling.

use std::io::{BufRead, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use pms_screen::Screen;

use super::Event;

/// Stalls this close together mean the network isn't keeping up.
const STALLS: usize = 2;
const STALL_WINDOW: Duration = Duration::from_secs(60);
/// How long a step down lasts before another can be taken, and how long things must stay quiet to go back up one.
const SETTLE: Duration = Duration::from_secs(30);
const RECOVER: Duration = Duration::from_secs(300);

/// A quality setting, like 720p60.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Quality {
    pub height: u32,
    pub fps: u32,
}

impl Quality {
    pub const USUAL: Quality = Quality { height: 480, fps: 30 };

    /// "720p60", or "480p", which means 30 frames a second.
    pub fn parse(text: &str) -> Option<Quality> {
        let (height, fps) = text.split_once('p')?;
        Some(Quality { height: height.parse().ok()?, fps: if fps.is_empty() { 30 } else { fps.parse().ok()? } })
    }
}

/// One rendition of a stream.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Tier {
    pub bandwidth: u32,
    pub height: u32,
    pub fps: u32,
}

impl Tier {
    pub fn name(&self) -> String {
        format!("{}p{}", self.height, self.fps)
    }
}

/// The video renditions in a master playlist, lowest first.
pub fn tiers(master: &str) -> Vec<Tier> {
    let mut out: Vec<Tier> = master
        .lines()
        .filter_map(|line| line.strip_prefix("#EXT-X-STREAM-INF:"))
        .filter_map(|attributes| {
            let get = |key: &str| attributes.split(',').find_map(|a| a.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v.trim_matches('"')));
            let height = get("RESOLUTION")?.split_once('x')?.1.parse().ok()?;
            let fps = get("FRAME-RATE").and_then(|f| f.parse::<f32>().ok()).unwrap_or(30.0).round() as u32;
            Some(Tier { bandwidth: get("BANDWIDTH")?.parse().ok()?, height, fps })
        })
        .collect();
    out.sort_by_key(|t| t.bandwidth);
    out.dedup();
    out
}

/// Why it's lower than it could be.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Why {
    Busy,
    Stalling,
}

/// How high a stream may go, right now.
pub struct Pace {
    /// The renditions, and the highest it may ever use and may use now, as indexes into them.
    pub tiers: Vec<Tier>,
    top: usize,
    pub at: usize,
    pub why: Option<Why>,
    moved: Instant,
    began: Instant,
    stalls: Vec<Instant>,
}

impl Pace {
    pub fn new(tiers: Vec<Tier>, best: Quality, now: Instant) -> Pace {
        let top = tiers.iter().rposition(|t| t.height <= best.height && t.fps <= best.fps).unwrap_or(0);
        Pace { tiers, top, at: top, why: None, moved: now, began: now, stalls: Vec::new() }
    }

    /// The most it may ever use has changed.
    pub fn aim_for(&mut self, best: Quality, now: Instant) {
        let tiers = std::mem::take(&mut self.tiers);
        *self = Pace::new(tiers, best, now);
    }

    /// The stream stalled.
    pub fn stalled(&mut self, now: Instant) {
        self.stalls.push(now);
    }

    /// Checks on things: the new limit, if it should change.
    pub fn check(&mut self, busy: bool, now: Instant) -> Option<u32> {
        self.stalls.retain(|t| now.duration_since(*t) < STALL_WINDOW);
        let trouble = if busy {
            Some(Why::Busy)
        } else if self.stalls.len() >= STALLS {
            Some(Why::Stalling)
        } else {
            None
        };
        let waited = now.duration_since(self.moved);
        if trouble.is_some() {
            if self.at == 0 || waited < SETTLE {
                return None;
            }
            self.at -= 1;
            self.why = trouble;
            self.stalls.clear();
        } else {
            if self.at >= self.top || waited < RECOVER {
                return None;
            }
            self.at += 1;
            if self.at == self.top {
                self.why = None;
            }
        }
        self.moved = now;
        Some(self.limit())
    }

    /// What the player is told, in bits a second: nothing if the stream has no renditions to choose among.
    pub fn limit(&self) -> u32 {
        self.tiers.get(self.at).map_or(0, |t| t.bandwidth)
    }

    /// The lowest with a picture.
    pub fn floor(&self) -> u32 {
        self.tiers.first().map_or(0, |t| t.bandwidth)
    }

    pub fn lowered(&self) -> bool {
        self.at < self.top
    }

    /// The rendition playing at `rate` bits a second.
    pub fn playing(&self, rate: u32) -> Option<&Tier> {
        self.tiers.iter().find(|t| t.bandwidth == rate)
    }

    /// Whether `rate` is below what it's allowed, which at first only means it hasn't got going.
    pub fn below(&self, rate: u32, now: Instant) -> bool {
        now.duration_since(self.began) > SETTLE && self.tiers.get(self.top).is_some_and(|t| rate != 0 && rate < t.bandwidth)
    }
}

/// What the player said.
#[derive(Clone, PartialEq, Debug)]
pub enum Said {
    Playing,
    Buffering,
    Rate(u32),
    Failed(String),
    Ended,
    Note(String),
}

fn read(line: &str) -> Option<Said> {
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    Some(match word {
        "playing" => Said::Playing,
        "buffering" => Said::Buffering,
        "rate" => Said::Rate(rest.parse().ok()?),
        "failed" => Said::Failed(rest.to_string()),
        "ended" => Said::Ended,
        "note" => Said::Note(rest.to_string()),
        _ => return None,
    })
}

/// A running player, drawing on a screen of its own.
pub struct Player {
    child: Child,
    stdin: ChildStdin,
    name: String,
    pub screen: Option<Screen>,
}

impl Player {
    /// Starts one, on the screen called `name`, within `pace`. What it says goes to `events`, tagged with `set` and `id`.
    pub fn start(url: &str, name: &str, pace: &Pace, set: usize, id: u64, events: Sender<Event>) -> Result<Player, String> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("pms-twatch.exe");
        let mut command = std::process::Command::new(exe);
        // Through the proxy, if that's where the video goes: then it fetches everything itself, not Windows.
        match crate::net::route_for(crate::net::Use::Video) {
            Some(proxy) => command.env("PMS_TWATCH_PROXY", proxy),
            None => command.env_remove("PMS_TWATCH_PROXY"),
        };
        let mut child = command
            .args([url, name, &pace.limit().to_string(), &pace.floor().to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("pms-twatch: {e}"))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else { return Err("pms-twatch: no pipes".into()) };
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(said) = read(&line) {
                    let _ = events.send(Event::Player { set, id, said });
                }
            }
            let _ = events.send(Event::Player { set, id, said: Said::Ended });
        });
        Ok(Player { child, stdin, name: name.to_string(), screen: None })
    }

    pub fn tell(&mut self, line: &str) {
        let _ = writeln!(self.stdin, "{line}");
        let _ = self.stdin.flush();
    }

    /// Its screen, once it has made it.
    pub fn screen(&mut self) -> Option<&Screen> {
        if self.screen.is_none() {
            self.screen = Screen::open(&self.name).ok();
        }
        self.screen.as_ref()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HD: Quality = Quality { height: 720, fps: 60 };

    const MASTER: &str = "#EXTM3U\n\
        #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"1080p60 (source)\"\n\
        #EXT-X-STREAM-INF:BANDWIDTH=6500000,RESOLUTION=1920x1080,CODECS=\"avc1.64002A,mp4a.40.2\",VIDEO=\"chunked\",FRAME-RATE=60.000\nhttps://a/1080.m3u8\n\
        #EXT-X-STREAM-INF:BANDWIDTH=3400000,AVERAGE-BANDWIDTH=9,RESOLUTION=1280x720,VIDEO=\"720p60\",FRAME-RATE=60.000\nhttps://a/720p60.m3u8\n\
        #EXT-X-STREAM-INF:BANDWIDTH=1400000,RESOLUTION=852x480,VIDEO=\"480p30\",FRAME-RATE=30.000\nhttps://a/480.m3u8\n\
        #EXT-X-STREAM-INF:BANDWIDTH=160000,CODECS=\"mp4a.40.2\",VIDEO=\"audio_only\"\nhttps://a/audio.m3u8\n";

    fn pace(best: Quality, now: Instant) -> Pace {
        Pace::new(tiers(MASTER), best, now)
    }

    #[test]
    fn a_master_playlist_gives_its_video_renditions_lowest_first() {
        let got = tiers(MASTER);
        assert_eq!(got.iter().map(Tier::name).collect::<Vec<_>>(), ["480p30", "720p60", "1080p60"]);
        assert_eq!(got[1].bandwidth, 3_400_000, "BANDWIDTH, not AVERAGE-BANDWIDTH");
        assert!(tiers("nonsense").is_empty());
    }

    #[test]
    fn qualities_read_as_written() {
        assert_eq!(Quality::parse("720p60"), Some(HD));
        assert_eq!(Quality::parse("480p"), Some(Quality { height: 480, fps: 30 }));
        assert_eq!(Quality::parse("best"), None);
    }

    #[test]
    fn it_goes_no_higher_than_the_setting_allows() {
        let now = Instant::now();
        assert_eq!(pace(HD, now).limit(), 3_400_000);
        assert_eq!(pace(Quality { height: 1080, fps: 60 }, now).limit(), 6_500_000);
        assert_eq!(pace(Quality { height: 720, fps: 30 }, now).limit(), 1_400_000, "720p60 is too much for 720p30");
        assert_eq!(pace(HD, now).floor(), 1_400_000, "the audio-only one isn't a picture");
    }

    #[test]
    fn a_busy_machine_steps_it_down_and_quiet_brings_it_back() {
        let t0 = Instant::now();
        let mut p = pace(HD, t0);
        assert_eq!(p.check(true, t0 + Duration::from_secs(5)), None, "it's only just started");
        assert_eq!(p.check(true, t0 + SETTLE), Some(1_400_000));
        assert!(p.lowered());
        assert_eq!(p.why, Some(Why::Busy));
        assert_eq!(p.check(true, t0 + SETTLE * 3), None, "there's nowhere lower");
        assert_eq!(p.check(false, t0 + SETTLE + Duration::from_secs(10)), None);
        assert_eq!(p.check(false, t0 + SETTLE + RECOVER), Some(3_400_000));
        assert!(!p.lowered());
        assert_eq!(p.why, None);
        assert_eq!(p.check(false, t0 + SETTLE + RECOVER * 3), None, "no higher than it may go");
    }

    #[test]
    fn stalls_close_together_step_it_down_and_stray_ones_dont() {
        let t0 = Instant::now();
        let mut p = pace(HD, t0);
        p.stalled(t0);
        assert_eq!(p.check(false, t0 + SETTLE), None, "one's nothing");
        p.stalled(t0 + STALL_WINDOW * 2);
        assert_eq!(p.check(false, t0 + STALL_WINDOW * 2), None, "nor two far apart");
        p.stalled(t0 + STALL_WINDOW * 2 + Duration::from_secs(1));
        assert_eq!(p.check(false, t0 + STALL_WINDOW * 2 + Duration::from_secs(2)), Some(1_400_000));
        assert_eq!(p.why, Some(Why::Stalling));
    }

    #[test]
    fn a_stream_getting_going_is_not_a_stream_held_back() {
        let t0 = Instant::now();
        let p = pace(HD, t0);
        assert!(!p.below(1_400_000, t0 + Duration::from_secs(5)));
        assert!(p.below(1_400_000, t0 + SETTLE * 2));
        assert!(!p.below(3_400_000, t0 + SETTLE * 2));
        assert!(!p.below(0, t0 + SETTLE * 2), "nothing's known yet");
    }

    #[test]
    fn a_stream_with_no_renditions_has_no_limit() {
        let p = Pace::new(Vec::new(), HD, Instant::now());
        assert_eq!((p.limit(), p.floor()), (0, 0));
    }

    #[test]
    fn what_the_player_says_is_understood() {
        assert_eq!(read("playing 1280 720"), Some(Said::Playing));
        assert_eq!(read("rate 3400000"), Some(Said::Rate(3_400_000)));
        assert_eq!(read("failed it broke badly"), Some(Said::Failed("it broke badly".into())));
        assert_eq!(read("what"), None);
    }
}
