//! Whether somebody's playing a game, for what can rest meanwhile (pressure.rs): the TVs, the Twatch TVs, and the console.
//!
//! A game, not just anything busy: the window in front fills its screen (borderless or exclusive), its own process keeps the
//! graphics card's 3D engine busy, and somebody's played lately: the keyboard, the mouse, or a controller, which Windows' own
//! last-input time leaves out: XInput's (Xbox controllers, and others through a remapper), and the rest as Windows.Gaming.Input
//! sees them (a PlayStation controller as it is, a wheel, a stick). Video playing full screen decodes on another engine, and nobody plays a video;
//! a busy compile has no window filling the screen. Exclusive full screen, which Windows says outright, is a game by that alone
//! (with the playing). Once a game counts, it goes on counting without the playing, for as long as it fills the screen and keeps
//! the card busy: a cutscene, or a controller Windows doesn't know, isn't the game being over.
//!
//! It takes a game's being on for a while to count, and its being over for longer to stop counting (`Flip`), so that
//! Alt+Tab, a loading screen or a pause menu changes nothing.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::System::Performance::{PDH_HCOUNTER, PDH_HQUERY, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhOpenQueryW};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Input::XboxController::{XINPUT_STATE, XInputGetState};
use windows::Win32::UI::Shell::{QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId};
use windows_core::{HSTRING, PCWSTR};

/// How busy the 3D engine has to be kept, in percent, by the process in front.
const BUSY_3D: f64 = 20.0;
/// How lately somebody has to have touched the keyboard or mouse.
const PLAYED_WITHIN: Duration = Duration::from_secs(60);
/// How long a game has to be on to count, and over to stop counting.
const COUNTS_AFTER: Duration = Duration::from_secs(10);
const OVER_AFTER: Duration = Duration::from_secs(20);

/// What's on the desktop rather than in front of it: never a game, however it fills the screen.
const SHELL: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

/// The window in front, its process, and whether it fills the screen it's on.
fn in_front() -> Option<(u32, bool)> {
    let window = unsafe { GetForegroundWindow() };
    if window.is_invalid() {
        return None;
    }
    let mut class = [0u16; 64];
    let n = unsafe { GetClassNameW(window, &mut class) } as usize;
    if SHELL.contains(&String::from_utf16_lossy(&class[..n]).as_str()) {
        return None;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    Some((pid, fills(window)))
}

/// Whether the window in front fills the screen the taskbar's on, which hides the taskbar: nothing on it can be seen. Not a game
/// necessarily (a video, a presentation): it's for what's drawn only to be seen, which can stop meanwhile. A few calls, cheap
/// enough to ask every second.
pub fn taskbar_covered() -> bool {
    let window = unsafe { GetForegroundWindow() };
    if window.is_invalid() || in_front().is_none_or(|(_, fills)| !fills) {
        return false;
    }
    let Ok(taskbar) = (unsafe { FindWindowW(windows_core::w!("Shell_TrayWnd"), None) }) else { return false };
    unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) == MonitorFromWindow(taskbar, MONITOR_DEFAULTTONEAREST) }
}

/// Whether `window` covers all of the screen it's on.
fn fills(window: HWND) -> bool {
    let mut r = RECT::default();
    if unsafe { GetWindowRect(window, &mut r) }.is_err() {
        return false;
    }
    let mut m = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    if !unsafe { GetMonitorInfoW(monitor, &mut m) }.as_bool() {
        return false;
    }
    let s = m.rcMonitor;
    r.left <= s.left && r.top <= s.top && r.right >= s.right && r.bottom >= s.bottom
}

/// Whether somebody's touched the keyboard or mouse within `within`.
fn touched_within(within: Duration) -> bool {
    let mut info = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return false;
    }
    u64::from(unsafe { GetTickCount() }.wrapping_sub(info.dwTime)) <= within.as_millis() as u64
}

/// Controllers: what each of XInput's four said last (its packet number, which changes with anything done with it), and when one
/// was last played with.
#[derive(Default)]
struct Pads {
    packets: [Option<u32>; 4],
    played: Option<Instant>,
}

impl Pads {
    /// Whether one's been played with within `within`. A packet number that's moved on since the last look is somebody playing.
    fn played_within(&mut self, within: Duration, now: Instant) -> bool {
        let packets: [Option<u32>; 4] = std::array::from_fn(|i| {
            let mut state = XINPUT_STATE::default();
            (unsafe { XInputGetState(i as u32, &mut state) } == 0).then_some(state.dwPacketNumber)
        });
        self.look(packets, within, now)
    }

    /// `played_within`, given what the controllers say now (None: none in that slot).
    fn look(&mut self, packets: [Option<u32>; 4], within: Duration, now: Instant) -> bool {
        for (last, packet) in self.packets.iter_mut().zip(packets) {
            if packet.is_some() && last.is_some() && packet != *last {
                self.played = Some(now);
            }
            *last = packet;
        }
        self.played.is_some_and(|t| now.duration_since(t) <= within)
    }
}

/// What a controller says at a moment: its buttons, its switches (the D-pad, a hat), and its axes (sticks, triggers, a wheel).
#[derive(Clone, PartialEq, Debug, Default)]
struct Reading {
    buttons: Vec<bool>,
    switches: Vec<i32>,
    axes: Vec<f64>,
}

/// How far an axis has to move (of its whole travel, 0 to 1) to be somebody moving it, not a stick drifting.
const AXIS_MOVED: f64 = 0.08;

impl Reading {
    /// Whether somebody did something between `self` and `now`. A controller that sends its state all the time, idle or not, says
    /// nothing new until something on it changes.
    fn moved(&self, now: &Reading) -> bool {
        self.buttons != now.buttons || self.switches != now.switches || self.axes.len() != now.axes.len() || self.axes.iter().zip(&now.axes).any(|(a, b)| (a - b).abs() > AXIS_MOVED)
    }
}

/// The controllers XInput doesn't see, as Windows.Gaming.Input does: what each said last, and when one was last played with.
#[derive(Default)]
struct Raw {
    last: HashMap<String, Reading>,
    played: Option<Instant>,
    /// Windows Runtime's been made ready on this thread.
    ready: bool,
}

impl Raw {
    fn played_within(&mut self, within: Duration, now: Instant) -> bool {
        if !std::mem::replace(&mut self.ready, true) {
            let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED) };
        }
        // The list fills in a moment after it's first asked for, and keeps up as controllers come and go.
        let mut seen = HashMap::new();
        if let Ok(all) = windows::Gaming::Input::RawGameController::RawGameControllers() {
            for i in 0..all.Size().unwrap_or(0) {
                let Ok(pad) = all.GetAt(i) else { continue };
                let (Ok(id), Some(reading)) = (pad.NonRoamableId(), read(&pad)) else { continue };
                seen.insert(id.to_string(), reading);
            }
        }
        if self.look(seen) {
            self.played = Some(now);
        }
        self.played.is_some_and(|t| now.duration_since(t) <= within)
    }

    /// Whether any controller seen before has been played with since; and remembers what each says now.
    fn look(&mut self, seen: HashMap<String, Reading>) -> bool {
        let moved = seen.iter().any(|(id, now)| self.last.get(id).is_some_and(|before| before.moved(now)));
        self.last = seen;
        moved
    }
}

/// What `pad` says now.
fn read(pad: &windows::Gaming::Input::RawGameController) -> Option<Reading> {
    use windows::Gaming::Input::GameControllerSwitchPosition;
    let count = |n: windows_core::Result<i32>| n.ok().and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let mut r = Reading { buttons: vec![false; count(pad.ButtonCount())], switches: Vec::new(), axes: vec![0.0; count(pad.AxisCount())] };
    let mut switches = vec![GameControllerSwitchPosition::Center; count(pad.SwitchCount())];
    pad.GetCurrentReading(&mut r.buttons, &mut switches, &mut r.axes).ok()?;
    r.switches = switches.iter().map(|s| s.0).collect();
    Some(r)
}

/// Exclusive full screen, which Windows says outright.
fn exclusive() -> bool {
    unsafe { SHQueryUserNotificationState() }.is_ok_and(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN)
}

/// How busy one process keeps the 3D engines: a counter of its own, opened again when the process in front changes. A rate needs
/// two readings, so the first after opening says nothing.
struct Engine {
    pid: u32,
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
    primed: bool,
}

impl Engine {
    fn open(pid: u32) -> Option<Engine> {
        let mut query = PDH_HQUERY::default();
        if unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) } != 0 {
            return None;
        }
        let mut counter = PDH_HCOUNTER::default();
        let path = HSTRING::from(format!(r"\GPU Engine(pid_{pid}_*engtype_3D)\Utilization Percentage"));
        if unsafe { PdhAddEnglishCounterW(query, &path, 0, &mut counter) } != 0 {
            unsafe { PdhCloseQuery(query) };
            return None;
        }
        Some(Engine { pid, query, counter, primed: false })
    }

    /// Percent, all its 3D engines together; None until there are two readings.
    fn read(&mut self) -> Option<f64> {
        if unsafe { PdhCollectQueryData(self.query) } != 0 {
            return None;
        }
        if !std::mem::replace(&mut self.primed, true) {
            return None;
        }
        crate::meters::instances(self.counter).ok().map(|all| all.iter().map(|(_, v)| v).sum())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

/// On only after `COUNTS_AFTER` of yes, and off only after `OVER_AFTER` of no.
#[derive(Default)]
struct Flip {
    on: bool,
    since: Option<Instant>,
}

impl Flip {
    fn step(&mut self, yes: bool, now: Instant) -> bool {
        if yes == self.on {
            self.since = None;
            return self.on;
        }
        let since = *self.since.get_or_insert(now);
        if now.duration_since(since) >= if self.on { OVER_AFTER } else { COUNTS_AFTER } {
            (self.on, self.since) = (yes, None);
        }
        self.on
    }
}

/// Keeps an eye out for a game: read now and then (`step`), it says whether one is on.
#[derive(Default)]
pub struct Watch {
    engine: Option<Engine>,
    pads: Pads,
    raw: Raw,
    flip: Flip,
}

impl Watch {
    pub fn step(&mut self, now: Instant) -> bool {
        let yes = self.looks_like_one(now);
        self.flip.step(yes, now)
    }

    fn looks_like_one(&mut self, now: Instant) -> bool {
        // Looked at every time, so that a packet number to compare with is always fresh.
        let pad = self.pads.played_within(PLAYED_WITHIN, now) | self.raw.played_within(PLAYED_WITHIN, now);
        let Some((pid, fills)) = in_front() else {
            self.engine = None;
            return false;
        };
        // To begin counting, somebody has to be playing; once it counts, filling the screen and the card being busy will do.
        if !fills || !(self.flip.on || pad || touched_within(PLAYED_WITHIN)) {
            return false;
        }
        if exclusive() {
            return true;
        }
        if self.engine.as_ref().is_none_or(|e| e.pid != pid) {
            self.engine = Engine::open(pid);
        }
        self.engine.as_mut().and_then(Engine::read).is_some_and(|busy| busy >= BUSY_3D)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_controller_played_with_counts_as_playing() {
        let (mut pads, t, minute) = (Pads::default(), Instant::now(), Duration::from_secs(60));
        assert!(!pads.look([Some(7), None, None, None], minute, t), "first sight of one is nothing yet");
        assert!(!pads.look([Some(7), None, None, None], minute, t + Duration::from_secs(5)), "left alone");
        assert!(pads.look([Some(9), None, None, None], minute, t + Duration::from_secs(10)), "a button, a stick");
        assert!(pads.look([Some(9), None, None, None], minute, t + Duration::from_secs(60)), "still within the minute");
        assert!(!pads.look([Some(9), None, None, None], minute, t + Duration::from_secs(80)), "a minute on, not");
        assert!(!pads.look([None, Some(3), None, None], minute, t + Duration::from_secs(85)), "one plugged in isn't one played");
    }

    #[test]
    fn any_controller_counts_when_something_on_it_changes() {
        let pad = |buttons: [bool; 2], switch: i32, axes: [f64; 2]| Reading { buttons: buttons.to_vec(), switches: vec![switch], axes: axes.to_vec() };
        let idle = pad([false, false], 0, [0.5, 0.0]);
        assert!(!idle.moved(&pad([false, false], 0, [0.53, 0.02])), "a stick drifting, or nothing new");
        assert!(idle.moved(&pad([true, false], 0, [0.5, 0.0])), "a button");
        assert!(idle.moved(&pad([false, false], 1, [0.5, 0.0])), "the D-pad");
        assert!(idle.moved(&pad([false, false], 0, [0.8, 0.0])), "a stick, pushed");
        let mut raw = Raw::default();
        let seen = |r: &Reading| HashMap::from([("DualSense".to_string(), r.clone())]);
        assert!(!raw.look(seen(&idle)), "first sight of one is nothing yet");
        assert!(!raw.look(seen(&idle)), "left alone");
        assert!(raw.look(seen(&pad([true, false], 0, [0.5, 0.0]))), "played with");
    }

    #[test]
    fn a_game_counts_after_a_while_and_stops_counting_after_longer() {
        let (mut f, t) = (Flip::default(), Instant::now());
        assert!(!f.step(true, t));
        assert!(!f.step(true, t + Duration::from_secs(5)));
        assert!(f.step(true, t + Duration::from_secs(10)), "on for ten seconds: it counts");
        assert!(f.step(false, t + Duration::from_secs(15)), "Alt+Tab, or a loading screen");
        assert!(f.step(true, t + Duration::from_secs(20)), "and back: nothing changed");
        assert!(f.step(false, t + Duration::from_secs(25)));
        assert!(f.step(false, t + Duration::from_secs(40)));
        assert!(!f.step(false, t + Duration::from_secs(45)), "over for twenty seconds: it's over");
    }
}
