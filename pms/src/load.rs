//! Whether the machine is busy, for what can do less when it is. All of its processors, from Windows' own totals, asked now and
//! then: it isn't a busy machine until it has stayed busy, or a quiet one until it has stayed quiet, so nothing flaps.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::GetSystemTimes;

const BUSY: f32 = 0.8;
const QUIET: f32 = 0.6;
const STAYS_BUSY: Duration = Duration::from_secs(5);
const STAYS_QUIET: Duration = Duration::from_secs(15);

#[derive(Default)]
pub struct Load {
    last: Option<(u64, u64)>,
    busy: bool,
    /// Since when it's been on the other side of the line.
    since: Option<Instant>,
}

fn ticks(t: FILETIME) -> u64 {
    u64::from(t.dwHighDateTime) << 32 | u64::from(t.dwLowDateTime)
}

impl Load {
    /// Takes a reading, and says whether the machine is busy.
    pub fn busy(&mut self, now: Instant) -> bool {
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        if unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.is_err() {
            return self.busy;
        }
        // Kernel time includes idle time.
        let (idle, total) = (ticks(idle), ticks(kernel) + ticks(user));
        let last = self.last.replace((idle, total));
        let Some((last_idle, last_total)) = last else { return self.busy };
        let spent = total.saturating_sub(last_total);
        if spent == 0 {
            return self.busy;
        }
        let used = 1.0 - idle.saturating_sub(last_idle) as f32 / spent as f32;
        self.step(used, now)
    }

    fn step(&mut self, used: f32, now: Instant) -> bool {
        let leaning = if self.busy { used < QUIET } else { used > BUSY };
        if !leaning {
            self.since = None;
            return self.busy;
        }
        let since = *self.since.get_or_insert(now);
        if now.duration_since(since) >= if self.busy { STAYS_QUIET } else { STAYS_BUSY } {
            self.busy = !self.busy;
            self.since = None;
        }
        self.busy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_takes_staying_busy_to_be_busy_and_staying_quiet_to_be_quiet() {
        let (mut load, start) = (Load::default(), Instant::now());
        let at = |s: u64| start + Duration::from_secs(s);
        assert!(!load.step(0.95, at(0)));
        assert!(!load.step(0.95, at(4)));
        assert!(load.step(0.95, at(5)));
        assert!(load.step(0.7, at(6)), "between the lines, it stays as it was");
        assert!(load.step(0.3, at(7)));
        assert!(load.step(0.3, at(21)));
        assert!(!load.step(0.3, at(22)));
    }

    #[test]
    fn a_spike_is_only_a_spike() {
        let (mut load, start) = (Load::default(), Instant::now());
        let at = |s: u64| start + Duration::from_secs(s);
        load.step(0.95, at(0));
        load.step(0.2, at(3));
        assert!(!load.step(0.95, at(6)), "the clock started again");
    }

    #[test]
    fn it_reads_this_machine() {
        let mut load = Load::default();
        load.busy(Instant::now());
        std::thread::sleep(Duration::from_millis(20));
        load.busy(Instant::now());
        assert!(load.last.is_some());
    }
}
