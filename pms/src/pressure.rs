//! Whether the machine is short of memory or video memory, for what can get out of the way when it is. A thread of its own takes a
//! reading now and then: it isn't short until it has stayed short, or fine until it has stayed fine, so nothing comes and goes.
//!
//! Memory is short when either what's in use or what's committed nears what there is. Video memory goes by the adapters big enough
//! to matter, and an adapter that's gone is simply no longer read: a GPU coming out isn't a shortage, and nobody's told it is.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1};
use windows::Win32::System::Performance::{PDH_HCOUNTER, PDH_HQUERY, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhOpenQueryW};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_core::{PCWSTR, w};

/// Percent of memory, in use or committed, that's short, and that's fine again.
const RAM_SHORT: u32 = 90;
const RAM_FINE: u32 = 80;
/// Percent of an adapter's dedicated memory.
const VRAM_SHORT: u32 = 69;
const VRAM_FINE: u32 = 60;
const STAYS_SHORT: Duration = Duration::from_secs(10);
const STAYS_FINE: Duration = Duration::from_secs(60);
const READ_EVERY: Duration = Duration::from_secs(5);
/// How often to see which adapters there are.
const ADAPTERS_EVERY: Duration = Duration::from_secs(60);
/// An adapter with less than this isn't what anything here would be using: it's the sliver an integrated GPU sets aside.
const SMALLEST_ADAPTER: u64 = 1 << 30;

/// What's short right now.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Pressure {
    pub ram: bool,
    pub vram: bool,
}

impl Pressure {
    /// Why a Twatch TV's player has to go, which takes memory of both kinds.
    pub fn twatch(self) -> Option<&'static str> {
        if self.vram {
            Some("VRAM PRESSURE")
        } else if self.ram {
            Some("RAM PRESSURE")
        } else {
            None
        }
    }

    /// Why the console has to go: it draws in software, so video memory is nothing to it.
    pub fn console(self) -> Option<&'static str> {
        self.ram.then_some("RAM PRESSURE")
    }
}

const RAM: u8 = 1;
const VRAM: u8 = 2;
static NOW: AtomicU8 = AtomicU8::new(0);

/// What's short right now. `PMS_PRESSURE` says so instead, as "ram", "vram" or both, or a file saying that, for trying it on a PC
/// with plenty.
pub fn now() -> Pressure {
    if let Ok(asked) = std::env::var("PMS_PRESSURE") {
        let asked = std::fs::read_to_string(&asked).unwrap_or(asked);
        return Pressure { ram: asked.replace("vram", "").contains("ram"), vram: asked.contains("vram") };
    }
    let bits = NOW.load(Ordering::Relaxed);
    Pressure { ram: bits & RAM != 0, vram: bits & VRAM != 0 }
}

/// One thing that can be short.
#[derive(Default)]
struct Gate {
    on: bool,
    /// Since when it's been on the other side of the line.
    since: Option<Instant>,
}

impl Gate {
    /// A reading, in percent, if there is one. No reading is no shortage, straight away.
    fn step(&mut self, level: Option<u32>, short: u32, fine: u32, now: Instant) -> bool {
        let Some(level) = level else {
            (self.on, self.since) = (false, None);
            return false;
        };
        let leaning = if self.on { level < fine } else { level >= short };
        if !leaning {
            self.since = None;
            return self.on;
        }
        let since = *self.since.get_or_insert(now);
        if now.duration_since(since) >= if self.on { STAYS_FINE } else { STAYS_SHORT } {
            self.on = !self.on;
            self.since = None;
        }
        self.on
    }
}

/// The more used of memory in use and memory committed, in percent.
fn ram_level() -> Option<u32> {
    let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut m) }.ok()?;
    let committed = (m.ullTotalPageFile > 0).then(|| 100 - (m.ullAvailPageFile.saturating_mul(100) / m.ullTotalPageFile) as u32);
    Some(m.dwMemoryLoad.max(committed.unwrap_or(0)))
}

/// Every adapter big enough to matter: how `GPU Adapter Memory` names it, and its dedicated memory.
fn adapters() -> HashMap<String, u64> {
    let mut out = HashMap::new();
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return out };
    for i in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else { break };
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else { continue };
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0 && desc.DedicatedVideoMemory as u64 >= SMALLEST_ADAPTER {
            out.insert(luid_name(desc.AdapterLuid.HighPart as u32, desc.AdapterLuid.LowPart), desc.DedicatedVideoMemory as u64);
        }
    }
    out
}

fn luid_name(high: u32, low: u32) -> String {
    format!("luid_0x{high:08x}_0x{low:08x}")
}

/// The fullest of `adapters`, in percent, from what `GPU Adapter Memory` says is in use. None with nothing to go by.
fn fullest(adapters: &HashMap<String, u64>, used: &[(String, f64)]) -> Option<u32> {
    let mut by_adapter: HashMap<&str, f64> = HashMap::new();
    for (name, bytes) in used {
        if let Some(at) = name.find("_phys_") {
            *by_adapter.entry(&name[..at]).or_default() += bytes;
        }
    }
    let levels = adapters.iter().filter_map(|(name, total)| Some(by_adapter.iter().find(|(n, _)| n.eq_ignore_ascii_case(name))?.1 * 100.0 / *total as f64));
    // More than it has is a reading to ignore, of a card that's halfway through going.
    levels.filter(|l| l.is_finite() && *l <= 110.0).map(|l| l as u32).max()
}

/// The counter of video memory in use, while it's open.
struct Vram {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
}

impl Vram {
    fn open() -> Option<Vram> {
        let mut query = PDH_HQUERY::default();
        if unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) } != 0 {
            return None;
        }
        let mut counter = PDH_HCOUNTER::default();
        if unsafe { PdhAddEnglishCounterW(query, w!(r"\GPU Adapter Memory(*)\Dedicated Usage"), 0, &mut counter) } != 0 {
            unsafe { PdhCloseQuery(query) };
            return None;
        }
        Some(Vram { query, counter })
    }

    fn read(&self) -> Option<Vec<(String, f64)>> {
        if unsafe { PdhCollectQueryData(self.query) } != 0 {
            return None;
        }
        crate::meters::instances(self.counter).ok()
    }
}

impl Drop for Vram {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

/// Starts keeping an eye on things, for as long as the process lasts.
pub fn start() {
    let _ = std::thread::Builder::new().name("pressure".into()).spawn(|| {
        let (mut ram, mut vram) = (Gate::default(), Gate::default());
        let mut found: Option<(Instant, HashMap<String, u64>, Option<Vram>)> = None;
        let mut was = 0;
        loop {
            let now = Instant::now();
            // Adapters come and go, and a counter only knows the ones there were when it was opened.
            if found.as_ref().is_none_or(|(at, ..)| now.duration_since(*at) >= ADAPTERS_EVERY) {
                let list = adapters();
                let counter = if list.is_empty() { None } else { Vram::open() };
                found = Some((now, list, counter));
            }
            let vram_level = found.as_ref().and_then(|(_, list, counter)| fullest(list, &counter.as_ref()?.read()?));
            let ram_level = ram_level();
            let (ram_short, vram_short) = (ram.step(ram_level, RAM_SHORT, RAM_FINE, now), vram.step(vram_level, VRAM_SHORT, VRAM_FINE, now));
            let bits = if ram_short { RAM } else { 0 } + if vram_short { VRAM } else { 0 };
            NOW.store(bits, Ordering::Relaxed);
            if bits != was {
                was = bits;
                let level = |l: Option<u32>| l.map_or_else(|| "no reading".to_string(), |l| format!("{l}% in use"));
                crate::log(format!("memory: RAM {} ({}), VRAM {} ({})", if bits & RAM != 0 { "low" } else { "OK" }, level(ram_level), if bits & VRAM != 0 { "low" } else { "OK" }, level(vram_level)));
            }
            std::thread::sleep(READ_EVERY);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_takes_staying_short_to_be_short_and_staying_fine_to_be_fine() {
        let (mut g, t) = (Gate::default(), Instant::now());
        assert!(!g.step(Some(95), 90, 80, t));
        assert!(!g.step(Some(95), 90, 80, t + Duration::from_secs(9)));
        assert!(g.step(Some(95), 90, 80, t + Duration::from_secs(10)));
        assert!(g.step(Some(85), 90, 80, t + Duration::from_secs(100)), "between the lines it stays as it is");
        assert!(g.step(Some(70), 90, 80, t + Duration::from_secs(110)));
        assert!(g.step(Some(70), 90, 80, t + Duration::from_secs(160)));
        assert!(!g.step(Some(70), 90, 80, t + Duration::from_secs(170)));
    }

    #[test]
    fn a_moment_over_the_line_is_nothing() {
        let (mut g, t) = (Gate::default(), Instant::now());
        g.step(Some(95), 90, 80, t);
        g.step(Some(50), 90, 80, t + Duration::from_secs(5));
        assert!(!g.step(Some(95), 90, 80, t + Duration::from_secs(11)));
    }

    #[test]
    fn a_gpu_going_away_is_no_shortage_and_no_fuss() {
        let (mut g, t) = (Gate::default(), Instant::now());
        g.step(Some(80), 69, 60, t);
        assert!(g.step(Some(80), 69, 60, t + Duration::from_secs(10)));
        assert!(!g.step(None, 69, 60, t + Duration::from_secs(11)), "straight away");
        let adapters = HashMap::from([(luid_name(0, 0x8d2c), 8 << 30)]);
        assert_eq!(fullest(&adapters, &[]), None, "nothing to go by");
        let other = [("luid_0x00000000_0x0000ffff_phys_0".to_string(), 1e9)];
        assert_eq!(fullest(&adapters, &other), None, "not one of ours");
        let wild = [("luid_0x00000000_0x00008D2C_phys_0".to_string(), 1e12)];
        assert_eq!(fullest(&adapters, &wild), None, "more than it has");
    }

    #[test]
    fn the_fullest_adapter_counts_whatever_case_it_is_named_in() {
        let adapters = HashMap::from([(luid_name(0, 0x8d2c), 8 << 30), (luid_name(0, 0xb660), 4 << 30)]);
        let used = [("luid_0x00000000_0x00008D2C_phys_0".to_string(), (2u64 << 30) as f64), ("luid_0x00000000_0x0000B660_phys_0".to_string(), (3u64 << 30) as f64)];
        assert_eq!(fullest(&adapters, &used), Some(75));
    }

    /// `cargo test -p pms this_machine -- --ignored --nocapture`: what it reads here, to compare with Task Manager.
    #[test]
    #[ignore = "reads this machine"]
    fn this_machine() {
        let list = adapters();
        let used = Vram::open().and_then(|v| v.read());
        println!("RAM level {:?}, adapters {list:?}, in use {used:?}", ram_level());
        println!("VRAM level {:?}", used.as_ref().and_then(|u| fullest(&list, u)));
    }

    #[test]
    fn what_has_to_go_depends_on_what_it_uses() {
        let vram = Pressure { vram: true, ram: false };
        let ram = Pressure { vram: false, ram: true };
        assert_eq!(vram.twatch(), Some("VRAM PRESSURE"));
        assert_eq!(ram.twatch(), Some("RAM PRESSURE"));
        assert_eq!(vram.console(), None, "the console doesn't use the GPU");
        assert_eq!(ram.console(), Some("RAM PRESSURE"));
        assert_eq!(Pressure::default().twatch(), None);
    }
}
