//! When Number One looks at the station next, and when the tank's been emptied: the schedule follows the tank.
//!
//! The tank (as the readings of 15-17 November 2026 show it, every change of NODE3000005) fills for most of a day, from about 5%
//! to 50-70%, a jump of a percent or a few each time somebody goes, half an hour to two hours apart, and nothing at night. Then the
//! urine processor runs (NODE3000004 at 16, NORMAL) and drains it, a percent every five or six minutes, give or take one (46, 47,
//! 46...), for two to eight hours, to about 5%, where it stops. Now and then a big jump in the middle of a drain (34% to 65%: a
//! transfer, it seems).
//!
//! So: filling, nothing can happen that matters, and a look every 15 minutes is plenty. Draining, a look a third of the time
//! it'll take to empty, as the rate of the last readings says (or 5.5 minutes a percent), from a minute to 15. Close to empty,
//! live (TLCP long polling: each change within a second), until it's emptied or stops draining. Away from the PC or on battery,
//! twice as long between looks, and never live.
//!
//! **Emptied** is a drain that's reached its bottom: at 10% or under, after being at 30% or over in the same drain, the level
//! gets to 5% (where the processor stops), or rises two over the lowest it got to, or the processor stops running, or nothing's
//! fallen for 15 minutes at 8% or under. Then not again until the tank's been at 30% again, so that 5, 6, 5 is one event.
//!
//! While the station's signal is lost its values are frozen: those readings say nothing about the tank, and the time without a
//! signal doesn't count as time without a fall.

use std::collections::VecDeque;

/// The processor's state while it's draining the tank.
const NORMAL: u32 = 16;
/// How a drain usually goes, in seconds a percent, when the readings can't say.
const USUAL_RATE: f32 = 5.5 * 60.0;
/// Where a drain stops.
const BOTTOM: f32 = 5.0;
/// A drain has to have been this full to empty.
const ARMED: f32 = 30.0;
/// Low enough to be at the bottom, and to be watched live.
const LOW: f32 = 10.0;
const LIVE_UNDER: f32 = 9.0;
const STILL_UNDER: f32 = 8.0;
/// How long without a new low, at the bottom, means it's stopped.
const STILL_FOR: i64 = 15 * 60;
/// How long the readings are kept, and how far back a fall's looked for, and the rate's fitted.
const KEPT: i64 = 3 * 3600;
const FALL_WITHIN: i64 = 40 * 60;
const FIT_WITHIN: i64 = 60 * 60;
/// Live when it'll be empty in less than this.
const LIVE_ETA: f32 = 10.0 * 60.0;
/// Between looks: filling, and the least and most while draining.
const FILLING: i64 = 15 * 60;
const SOONEST: i64 = 60;

/// How the next look goes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    /// A look this many seconds after the last.
    Every(i64),
    /// Live: each change as it comes.
    Live,
}

/// What the tank's doing, as far as the readings say.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Doing {
    /// Filling, or nothing yet known.
    Filling,
    /// Draining, and how long it'll be to empty, in seconds.
    Draining(f32),
}

/// The schedule as it stands: what it's doing, and how it looks next.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Step {
    pub doing: Doing,
    pub mode: Mode,
}

/// The readings, and where the drain's got to.
#[derive(Default, Clone, Debug)]
pub struct Plan {
    /// Each reading with a signal: when (seconds since 1970) and the level.
    seen: VecDeque<(i64, f32)>,
    /// The tank's been at 30% or over since it last emptied.
    armed: bool,
    /// The processor was running, at the last reading; and has been, since the tank was last at 30%.
    running: bool,
    ran: bool,
    /// The lowest it's got to, low down, and when it last got lower.
    low: Option<f32>,
    fell_at: i64,
    /// Since when the signal's been lost, if it is.
    lost_since: Option<i64>,
}

impl Plan {
    pub const fn new() -> Plan {
        Plan { seen: VecDeque::new(), armed: false, running: false, ran: false, low: None, fell_at: 0, lost_since: None }
    }

    /// A reading, at `at`: the tank's level, the processor's state, and whether the station has a signal. True if it's just been
    /// emptied.
    pub fn take(&mut self, at: i64, level: f32, processor: Option<u32>, signal: bool) -> bool {
        if !signal {
            self.lost_since.get_or_insert(at);
            return false;
        }
        // Time without a signal isn't time without a fall.
        if let Some(lost) = self.lost_since.take() {
            self.fell_at += at - lost;
        }
        self.seen.push_back((at, level));
        while self.seen.front().is_some_and(|(t, _)| at - t > KEPT) {
            self.seen.pop_front();
        }
        let was_running = self.running;
        self.running = processor == Some(NORMAL);
        if level >= ARMED || (self.running && level >= LOW + 5.0) {
            self.armed = true;
        }
        self.ran |= self.running && self.armed;
        if !self.armed || level > LOW {
            self.low = None;
            return false;
        }
        let draining = self.draining(at);
        let (low, new_low) = match self.low {
            Some(l) if level >= l => (l, false),
            _ => (level, true),
        };
        if new_low {
            self.low = Some(level);
            self.fell_at = at;
        }
        let emptied = (level <= BOTTOM && (draining || new_low))
            || level >= low + 2.0
            || (self.ran && was_running && !self.running)
            || (level <= STILL_UNDER && at - self.fell_at >= STILL_FOR);
        if emptied {
            self.armed = false;
            self.ran = false;
            self.low = None;
        }
        emptied
    }

    /// Whether it's draining, as of `at`: the processor's running, or the level's fallen two or more within the last 40 minutes.
    fn draining(&self, at: i64) -> bool {
        let Some((_, now)) = self.seen.back() else { return false };
        let high = self.seen.iter().filter(|(t, _)| at - t <= FALL_WITHIN).map(|(_, l)| *l).fold(f32::MIN, f32::max);
        self.running || high - now >= 2.0
    }

    /// How fast it's draining, in seconds a percent: fitted to the readings of the last hour since the last jump up, or the usual.
    fn rate(&self, at: i64) -> f32 {
        let recent: Vec<(i64, f32)> = self.seen.iter().filter(|(t, _)| at - t <= FIT_WITHIN).copied().collect();
        let since = recent.windows(2).rposition(|w| w[1].1 - w[0].1 >= 3.0).map_or(0, |i| i + 1);
        let points = &recent[since..];
        if points.len() < 3 || points[points.len() - 1].0 - points[0].0 < 10 * 60 {
            return USUAL_RATE;
        }
        let n = points.len() as f32;
        let t0 = points[0].0;
        let (mx, my) = (points.iter().map(|(t, _)| (t - t0) as f32).sum::<f32>() / n, points.iter().map(|(_, l)| *l).sum::<f32>() / n);
        let (mut sxy, mut sxx) = (0.0, 0.0);
        for (t, l) in points {
            let x = (t - t0) as f32 - mx;
            sxy += x * (l - my);
            sxx += x * x;
        }
        let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
        // Percent a second, falling: as fast as a drain goes (a percent in 30 s to 30 min), or the usual.
        if slope < -1.0 / 1800.0 && slope > -1.0 / 30.0 { -1.0 / slope } else { USUAL_RATE }
    }

    /// The schedule, as of `at`: `away` (nobody at the PC, or on battery) is twice as long between looks, and never live.
    pub fn next(&self, at: i64, away: bool) -> Step {
        let Some((_, level)) = self.seen.back().copied() else { return Step { doing: Doing::Filling, mode: Mode::Every(if away { 2 * FILLING } else { FILLING }) } };
        let slow = if away { 2 } else { 1 };
        // Not draining, or draining what can't be emptied (it hasn't been at 30% since it last was): nothing to watch for.
        if !self.draining(at) || !self.armed {
            return Step { doing: Doing::Filling, mode: Mode::Every(FILLING * slow) };
        }
        let eta = (level - BOTTOM).max(0.0) * self.rate(at);
        let mode = if !away && (eta < LIVE_ETA || level <= LIVE_UNDER) { Mode::Live } else { Mode::Every(((eta / 3.0) as i64).clamp(SOONEST, FILLING) * slow) };
        Step { doing: Doing::Draining(eta), mode }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every change of the urine tank's level, 15 to 17 November 2026 (GMT), as the Bluesky bot iss-piss-tracker posted them:
    /// a day, then its changes, each `HH:MM:SS level`.
    const DAYS: [(i64, &str); 3] = [
        (15, "09:21:25 57 09:41:13 58 11:21:32 60 12:17:44 61 12:33:30 64 13:39:26 65 14:23:59 68 15:46:25 70 15:50:18 69 15:51:09 68 15:53:52 67 15:54:36 68 15:55:18 67 16:00:18 66 16:12:49 65 16:13:42 64 16:19:30 63 16:25:14 62 16:29:44 66 16:35:28 65 16:40:56 64 16:46:28 63 16:47:01 64 16:47:55 63 16:52:05 62 16:58:14 61 17:03:34 60 17:08:49 59 17:30:11 55 17:35:21 54 17:40:15 53 17:45:48 52 17:51:15 51 17:56:13 50 18:01:50 49 18:06:36 48 18:11:59 47 18:17:13 46 18:22:19 45 18:28:02 44 19:05:30 42 19:08:46 41 19:14:15 40 19:19:43 39 19:25:04 38 19:31:17 37 19:35:36 36 19:39:02 38 19:45:02 37 19:50:53 36 19:56:44 35 20:00:47 34 20:05:30 33 20:11:24 32 20:17:09 31 20:21:23 30 20:22:00 31 20:22:46 30 20:26:53 29 20:32:52 28 20:38:17 27 20:42:49 26 20:50:07 25 20:55:29 24 20:57:50 27 21:16:28 29 21:20:49 28 21:26:11 27 21:30:17 26 21:31:40 27 21:33:57 26 21:39:41 25 21:44:38 24 21:49:41 23 21:54:16 22 21:55:37 23 21:57:48 22 22:03:01 21 22:07:39 20 22:13:33 19 22:18:06 18 22:18:38 19 22:19:14 18 22:22:51 17 22:28:50 16 22:33:03 15 22:38:54 14 22:43:13 13 22:57:15 11 22:58:54 10 23:03:31 9 23:09:08 8 23:13:45 7 23:19:23 6 23:24:39 5 23:27:29 6 23:28:03 7 23:29:18 8 23:31:45 9 23:39:05 11 23:57:56 13"),
        (16, "02:05:25 12 03:44:00 13 03:48:56 12 06:37:59 14 06:47:45 17 06:48:30 18 07:29:33 19 08:08:11 22 09:56:44 23 11:17:52 28 11:58:23 29 13:27:24 30 13:34:25 32 13:59:00 36 15:22:52 38 15:56:35 40 16:17:26 44 18:19:55 49 20:47:21 50 20:57:13 52 22:26:41 53 22:46:46 57 23:52:34 58"),
        (17, "01:05:35 57 01:07:11 56 01:11:33 55 01:17:40 54 01:32:04 52 01:36:20 51 01:42:19 50 01:47:28 49 01:53:03 48 01:58:41 47 02:04:37 46 02:10:17 45 02:15:05 44 02:20:19 43 02:25:42 42 02:31:27 41 02:36:32 40 02:41:40 39 02:44:15 40 02:46:53 39 02:52:24 38 03:00:25 37 03:02:27 36 03:07:45 35 03:12:57 34 03:20:26 65 03:27:28 64 03:33:06 63 03:38:37 62 03:44:18 61 03:50:01 60 03:55:46 59 04:01:30 58 04:06:04 57 04:06:38 58 04:07:23 57 04:11:31 56 04:12:14 57 04:12:54 56 04:17:19 55 04:22:48 54 04:28:34 53 04:32:44 52 04:38:26 51 04:43:55 50 04:48:04 49 04:53:50 48 04:59:09 47 05:03:24 46 05:03:58 47 05:04:42 46 05:08:57 45 05:14:19 44 05:19:44 43 05:24:20 42 05:29:39 41 05:33:40 40 05:34:20 41 05:35:03 40 05:39:15 39 05:44:44 38 05:50:01 37 05:53:58 36 05:59:40 35 06:03:39 34 06:04:18 35 06:04:59 34 06:06:29 36 06:21:08 34 06:22:10 33 06:27:55 32 06:32:04 31 06:32:41 32 06:33:20 31 06:37:25 30 06:42:55 29 06:47:00 28 06:47:37 29 06:48:21 28 06:52:28 27 06:57:47 30 07:01:39 29 07:05:47 28 07:06:25 29 07:07:08 28 07:11:25 27 07:16:45 26 07:20:47 25 07:21:27 26 07:22:07 25 07:26:20 24 07:31:36 23 07:37:12 22 07:41:25 21 07:42:13 22 07:42:46 21 07:46:43 20 08:04:36 17 08:07:37 16 08:08:12 17 08:08:50 16 08:12:52 15 08:18:15 14 08:22:26 13 08:27:39 12 08:35:15 11 08:40:34 10 08:44:33 9 08:45:03 10 08:45:52 9 08:49:56 8 08:55:12 7 08:59:15 6 08:59:48 7 09:01:02 6 09:01:35 7 09:09:35 6 09:10:22 5 09:12:50 6 09:13:26 7 09:14:35 8 09:53:28 13 10:18:41 15 11:33:32 16 11:37:27 19 13:00:06 21 13:06:02 23 14:05:10 25 14:07:37 29 16:01:24 32 17:03:48 36 17:45:07 38 20:17:18 40 20:33:03 45 21:03:21 46 21:14:57 49"),
    ];

    /// Seconds since 15 November 2026, 00:00 GMT.
    fn changes() -> Vec<(i64, f32)> {
        let mut out = Vec::new();
        for (day, text) in DAYS {
            let words: Vec<&str> = text.split(' ').collect();
            for pair in words.chunks(2) {
                let hms: Vec<i64> = pair[0].split(':').map(|n| n.parse().unwrap()).collect();
                out.push(((day - 15) * 86_400 + hms[0] * 3600 + hms[1] * 60 + hms[2], pair[1].parse().unwrap()));
            }
        }
        out
    }

    fn at(day: i64, hms: &str) -> i64 {
        let n: Vec<i64> = hms.split(':').map(|n| n.parse().unwrap()).collect();
        (day - 15) * 86_400 + n[0] * 3600 + n[1] * 60 + n[2]
    }

    /// What a run of the schedule over the readings comes to: each emptying, when it was declared; and the requests, a day at a
    /// time, as each is made: a look is one request (a session made, with its snapshot, and left: it ends by itself), going
    /// live is two (a session, and the clock's subscription), each long poll one (answered at a change, or after 30 s with
    /// nothing), and leaving live one (the session ended). `processor`: the processor's state at a time, if it's known.
    fn replay(series: &[(i64, f32)], processor: &dyn Fn(i64) -> Option<u32>) -> (Vec<i64>, Vec<usize>) {
        let level_at = |t: i64| series.iter().take_while(|(c, _)| *c <= t).last().map_or(series[0].1, |(_, l)| *l);
        let (start, end) = (series[0].0, series[series.len() - 1].0);
        let mut plan = Plan::default();
        let (mut events, mut requests) = (Vec::new(), vec![0usize; 3]);
        let count = |t: i64, n: usize, r: &mut Vec<usize>| r[(t / 86_400).clamp(0, 2) as usize] += n;
        let mut t = start;
        while t < end {
            count(t, 1, &mut requests);
            if plan.take(t, level_at(t), processor(t), true) {
                events.push(t);
            }
            match plan.next(t, false).mode {
                Mode::Every(s) => t += s,
                Mode::Live => {
                    let began = t;
                    count(t, 1, &mut requests);
                    loop {
                        let next = series.iter().map(|(c, _)| *c).find(|c| *c > t);
                        t = match next {
                            Some(c) if c - t <= 30 => c,
                            _ => t + 30,
                        };
                        count(t, 1, &mut requests);
                        if plan.take(t, level_at(t), processor(t), true) {
                            events.push(t);
                        }
                        if plan.next(t, false).mode != Mode::Live || t >= end {
                            count(t, 1, &mut requests);
                            let hm = |t: i64| format!("{} {:02}:{:02}:{:02}", 15 + t / 86_400, t % 86_400 / 3600, t % 3600 / 60, t % 60);
                            println!("live from {} to {}", hm(began), hm(t));
                            break;
                        }
                    }
                }
            }
        }
        (events, requests)
    }

    #[test]
    fn each_drain_empties_once_and_is_declared_at_its_bottom() {
        let series = changes();
        let (events, requests) = replay(&series, &|_| None);
        // 15 November: down to 5% at 23:24:39. 17 November: down to 5% at 09:10:22 (after 6, 7, 6, 7, 6 at the bottom).
        assert_eq!(events, [at(15, "23:24:39"), at(17, "09:10:22")], "{:?}", events.iter().map(|e| (e / 86_400 + 15, e % 86_400 / 3600, e % 3600 / 60, e % 60)).collect::<Vec<_>>());
        // A whole day: well under the 288 looks every 5 minutes made (each with its session ended, 576 requests).
        println!("requests a day, 15-17 November: {requests:?}");
        assert!(requests[1] <= 110, "16 November, all filling: {}", requests[1]);
        assert!(requests.iter().all(|r| *r < 288), "{requests:?}");
    }

    #[test]
    fn the_processor_stopping_at_the_bottom_is_the_end_of_it() {
        // 17 November, as the processor had it: NORMAL from the drain's start until 09:11:50, then STANDBY. A drain that stops
        // short of 5% is declared when the processor stops.
        let series: Vec<(i64, f32)> = changes().into_iter().filter(|(t, l)| *t >= at(17, "00:00:00") && *t <= at(17, "09:10:00") && *l > 0.0).collect();
        let mut stopped_short = series.clone();
        stopped_short.retain(|(_, l)| *l >= 6.0);
        stopped_short.push((at(17, "09:20:00"), 7.0));
        let processor = |t: i64| Some(if (at(17, "01:00:00")..at(17, "09:11:50")).contains(&t) { NORMAL } else { 32 });
        let (events, _) = replay(&stopped_short, &processor);
        assert_eq!(events.len(), 1, "{events:?}");
        assert!(events[0] >= at(17, "09:11:50") && events[0] <= at(17, "09:12:50"), "within a minute of its stopping: {}", events[0] - at(17, "09:11:50"));
    }

    #[test]
    fn jitter_at_the_bottom_is_one_event() {
        let mut plan = Plan::default();
        let mut events = 0;
        for (i, level) in [60.0, 40.0, 20.0, 10.0, 9.0, 8.0, 7.0, 6.0, 5.0, 6.0, 5.0, 6.0, 5.0, 7.0, 8.0, 6.0].into_iter().enumerate() {
            events += usize::from(plan.take(i as i64 * 300, level, None, true));
        }
        assert_eq!(events, 1);
        // Filling, with a fall of one now and then: nothing.
        let mut filling = Plan::default();
        assert!(![12.0, 13.0, 12.0, 14.0, 17.0, 18.0].into_iter().enumerate().any(|(i, l)| filling.take(i as i64 * 900, l, None, true)));
        assert_eq!(filling.next(5 * 900, false).mode, Mode::Every(15 * 60));
    }

    #[test]
    fn a_drain_that_stops_low_is_emptied_after_a_while() {
        let mut plan = Plan::default();
        let mut t = 0;
        let mut events = vec![];
        for level in [50.0, 40.0, 30.0, 20.0, 12.0, 10.0, 9.0, 8.0, 7.0] {
            t += 330;
            if plan.take(t, level, None, true) {
                events.push(t);
            }
        }
        for _ in 0..20 {
            t += 60;
            if plan.take(t, 7.0, None, true) {
                events.push(t);
            }
        }
        assert_eq!(events, [330 * 9 + 15 * 60]);
    }

    #[test]
    fn the_schedule_follows_the_tank() {
        let mut plan = Plan::default();
        assert_eq!(plan.next(0, false).mode, Mode::Every(15 * 60), "nothing known: 15 minutes");
        plan.take(0, 40.0, None, true);
        assert_eq!(plan.next(0, false), Step { doing: Doing::Filling, mode: Mode::Every(15 * 60) });
        assert_eq!(plan.next(0, true).mode, Mode::Every(30 * 60), "away: twice as long");
        // Draining a percent every 5.5 minutes from 40%: hours to go, so every 15 minutes still.
        let mut t = 0;
        for level in (20..40).rev() {
            t += 330;
            plan.take(t, level as f32, Some(NORMAL), true);
        }
        let Step { doing: Doing::Draining(eta), mode } = plan.next(t, false) else { panic!() };
        assert!((eta - 15.0 * 330.0).abs() < 200.0, "{eta}");
        assert_eq!(mode, Mode::Every(((eta / 3.0) as i64).clamp(60, 900)));
        for level in (8..20).rev() {
            t += 330;
            plan.take(t, level as f32, Some(NORMAL), true);
        }
        assert_eq!(plan.next(t, false).mode, Mode::Live, "nearly empty: live");
        assert!(matches!(plan.next(t, true).mode, Mode::Every(_)), "but never live while nobody's there");
        // The signal lost for 20 minutes: its frozen readings say nothing, and that time isn't time without a fall.
        let mut lost = Plan::default();
        for (i, level) in [40.0, 30.0, 20.0, 10.0, 8.0].into_iter().enumerate() {
            lost.take(i as i64 * 330, level, None, true);
        }
        assert!(!lost.take(5 * 330, 8.0, None, false));
        assert!(!lost.take(5 * 330 + 1200, 8.0, None, false));
        assert!(!lost.take(5 * 330 + 1260, 8.0, None, true), "back: 20 minutes with no fall, but 20 of them without a signal");
    }
}
