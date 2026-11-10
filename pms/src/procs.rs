//! PimpMyShell's own processes, for the settings to list: the daemon and everything it started (the console, the game in
//! it, Twatch TV's players and whatever those started), and the settings and the command line themselves. For each, how
//! much of the processor it's had so far, to tell how busy it is between two looks, and how much memory is its alone.
//!
//! Its memory is its private working set: what's in RAM just now that no other process shares. Not its commit, which counts
//! what it's only reserved; and not its whole working set, which counts every DLL's pages again for each process using them.
//! It's what Task Manager's "Memory" column shows.
//!
//! How long it's had a processor is counted in processor cycles, which Windows keeps exactly, to the switch from one
//! thread to the next. The times it also keeps (what `GetProcessTimes` gives) are only added to at each tick of its clock,
//! some 64 times a second, by whichever thread happens to be running then: between two looks two seconds apart on a
//! machine of a dozen processors, one tick is a twentieth of a percent, and a process that works in bursts shorter than a
//! tick can do a good deal and be charged nothing. Cycles don't miss it.
//!
//! And how much of the graphics card each is using, and how much of its memory (`Gpus`): from the counters Windows keeps
//! for every process and every card, which are what Task Manager's GPU columns show. The card Windows makes up out of the
//! processor, for when there's none (the Microsoft Basic Render Driver, WARP), isn't counted: work on that is processor
//! work, and is in the processor's column already.

use windows::Win32::Foundation::{CloseHandle, FILETIME};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2};
use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::System::WindowsProgramming::QueryProcessCycleTime;

/// The programs that are PimpMyShell's, whoever started them.
const OURS: [&str; 5] = ["pmsd.exe", "pms-gui.exe", "pms.exe", crate::parts::PIMPBOX, "pms-twatch.exe"];

/// One of them, as it is now.
#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: u32,
    /// What it's called: its program, and what that is where the name doesn't say.
    pub name: String,
    /// How many processes are between it and the one it descends from: for showing who started what.
    pub depth: usize,
    /// How long it's had a processor in all, in 100-nanosecond steps: user and kernel.
    pub busy: u64,
    /// Its private working set, in bytes.
    pub private: u64,
}

/// How many processors there are: what a process's share of them is out of.
pub fn processors() -> u32 {
    std::thread::available_parallelism().map_or(1, |n| n.get() as u32)
}

/// Every process there is: which, started by which, running what.
fn all() -> Vec<(u32, u32, String)> {
    let mut found = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return found };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let name = &entry.szExeFile[..entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len())];
            found.push((entry.th32ProcessID, entry.th32ParentProcessID, String::from_utf16_lossy(name)));
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    found
}

/// How many cycles a processor does in a second, as Windows has it (the rate its cycle counter runs at, whatever speed the
/// processor's really going at just then): for turning cycles into time. None if it doesn't say.
fn cycles_a_second() -> Option<f64> {
    use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW};
    static RATE: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
    *RATE.get_or_init(|| {
        let (mut megahertz, mut size) = (0u32, 4u32);
        let read = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, windows_core::w!(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"), windows_core::w!("~MHz"), RRF_RT_REG_DWORD, None, Some((&raw mut megahertz).cast()), Some(&mut size)) };
        (read.is_ok() && megahertz > 0).then(|| f64::from(megahertz) * 1e6)
    })
}

/// So many cycles, as the time they took, in 100-nanosecond steps. None if there's no telling.
fn time_of(cycles: u64) -> Option<u64> {
    cycles_a_second().filter(|_| cycles > 0).map(|rate| (cycles as f64 / rate * 1e7) as u64)
}

/// How long a process has had a processor, and how much memory is its alone. None if it can't be asked (it's gone).
fn measured(pid: u32) -> Option<(u64, u64)> {
    unsafe {
        // One that's another user's can't be opened to be asked: the system's own list of processes says the same of it.
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return listed(pid) };
        let (mut made, mut left, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let timed = GetProcessTimes(process, &mut made, &mut left, &mut kernel, &mut user).is_ok();
        let since = |t: FILETIME| u64::from(t.dwHighDateTime) << 32 | u64::from(t.dwLowDateTime);
        // Its cycles, which are exact, where they can be had; its times, which are by the tick, where they can't.
        let mut cycles = 0u64;
        let exact = QueryProcessCycleTime(process, &mut cycles).ok().and_then(|()| time_of(cycles));
        // The newer counters have the private working set. Where Windows hasn't those, what it's committed is the nearest.
        let mut memory = PROCESS_MEMORY_COUNTERS_EX2::default();
        let asked = K32GetProcessMemoryInfo(process, (&raw mut memory).cast::<PROCESS_MEMORY_COUNTERS>(), size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32).as_bool();
        let _ = CloseHandle(process);
        let private = if asked && memory.PrivateWorkingSetSize > 0 { memory.PrivateWorkingSetSize } else { memory.PrivateUsage };
        timed.then(|| (exact.unwrap_or(since(kernel) + since(user)), private as u64))
    }
}

/// PimpMyShell's processes: each of its own programs, with everything it started under it, in that order.
pub fn ours() -> Vec<Proc> {
    let everything = all();
    let is_ours = |name: &str| OURS.iter().any(|o| o.eq_ignore_ascii_case(name));
    let mut found: Vec<Proc> = Vec::new();
    // From each of ours that none of ours started, down through whatever it started.
    let mut todo: Vec<(u32, usize)> = everything
        .iter()
        .filter(|(_, parent, name)| is_ours(name) && !everything.iter().any(|(pid, _, other)| pid == parent && is_ours(other)))
        .map(|(pid, ..)| (*pid, 0))
        .rev()
        .collect();
    while let Some((pid, depth)) = todo.pop() {
        if found.iter().any(|p| p.pid == pid) || found.len() >= 64 {
            continue;
        }
        let Some((_, parent, name)) = everything.iter().find(|(p, ..)| *p == pid) else { continue };
        let Some((busy, private)) = measured(pid) else { continue };
        // The game in the console is the console's own program again: said, since the name doesn't.
        let game = name.eq_ignore_ascii_case(crate::parts::PIMPBOX) && everything.iter().any(|(p, _, n)| p == parent && n.eq_ignore_ascii_case(crate::parts::PIMPBOX));
        found.push(Proc { pid, name: if game { format!("{name} (POOM)") } else { name.clone() }, depth, busy, private });
        todo.extend(everything.iter().filter(|(child, parent, _)| *parent == pid && *child != pid).map(|(child, ..)| (*child, depth + 1)).rev());
    }
    found
}

/// The Explorer process the taskbar's in, which is the one PimpMyShell's part in the taskbar lives inside: not one of
/// PimpMyShell's own, and all of it is counted, Explorer's own work and memory with ours, as there's no telling them apart
/// from outside. None if there's no taskbar.
pub fn shell() -> Option<Proc> {
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId};
    let mut pid = 0;
    unsafe {
        let taskbar = FindWindowW(windows_core::w!("Shell_TrayWnd"), windows_core::PCWSTR::null()).ok()?;
        GetWindowThreadProcessId(taskbar, Some(&mut pid));
    }
    let (busy, private) = measured(pid)?;
    Some(Proc { pid, name: "explorer.exe (the taskbar, with PimpMyShell's part in it)".into(), depth: 0, busy, private })
}

/// How long a process has had a processor, and how much memory is its alone, from the list the system keeps of every
/// process (what Task Manager reads): for one this user isn't let open. Its layout is the 64-bit one, which is the only
/// Windows this runs on.
fn listed(pid: u32) -> Option<(u64, u64)> {
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQuerySystemInformation(class: i32, to: *mut core::ffi::c_void, room: u32, needed: *mut u32) -> i32;
    }
    const PROCESSES: i32 = 5;
    let mut room = vec![0u64; 1 << 17];
    for _ in 0..4 {
        let mut needed = 0u32;
        let status = unsafe { NtQuerySystemInformation(PROCESSES, room.as_mut_ptr().cast(), (room.len() * 8) as u32, &mut needed) };
        if status == 0 {
            break;
        }
        // Not enough room: as much as it says, and some over, as the list grows while it's asked for.
        room = vec![0u64; (needed as usize).div_ceil(8) + 4096];
    }
    let bytes = unsafe { std::slice::from_raw_parts(room.as_ptr().cast::<u8>(), room.len() * 8) };
    let number = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default());
    let mut at = 0;
    loop {
        // Each entry: how far to the next, at its start; its private working set at 8; its cycles at 24; its times in user
        // and kernel at 40 and 48; and which process it is at 80.
        if at + 88 > bytes.len() {
            return None;
        }
        if number(at + 80) == u64::from(pid) {
            return Some((time_of(number(at + 24)).unwrap_or(number(at + 40) + number(at + 48)), number(at + 8)));
        }
        let next = (number(at) & 0xFFFF_FFFF) as usize;
        if next == 0 {
            return None;
        }
        at += next;
    }
}

/// The Desktop Window Manager of this session, `dwm.exe`: not PimpMyShell's, but where a good part of what it puts on the
/// taskbar is actually drawn (see `DWM_IS`). None if it can't be found.
pub fn dwm() -> Option<Proc> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn ProcessIdToSessionId(process: u32, session: *mut u32) -> i32;
    }
    let session = |pid: u32| {
        let mut session = u32::MAX;
        (unsafe { ProcessIdToSessionId(pid, &mut session) } != 0).then_some(session)
    };
    let ours = session(std::process::id());
    let everything = all();
    let (pid, ..) = everything.iter().find(|(pid, _, name)| name.eq_ignore_ascii_case("dwm.exe") && session(*pid) == ours).or_else(|| everything.iter().find(|(_, _, name)| name.eq_ignore_ascii_case("dwm.exe")))?;
    let (busy, private) = measured(*pid)?;
    Some(Proc { pid: *pid, name: "dwm.exe".into(), depth: 0, busy, private })
}

/// What the Desktop Window Manager has to do with PimpMyShell, for whoever's reading its row.
pub const DWM_IS: &str = "Desktop Window Manager: the system compositor. The taskbar is a XAML island whose visual tree, with PimpMyShell's elements in it, is a tree of Windows.UI.Composition visuals that DWM composes every frame. So the meters' and TVs' surfaces, the acrylic and other effect brushes, and every composition animation (glows, ripples, toasts, the remote's beam) are rendered here, on DWM's own GPU time and video memory, not in Explorer.";

/// What the graphics memory figures are, and aren't, for whoever's reading them.
pub const GPU_MEMORY_IS: &str = "GPU figures are Windows' own per-process counters, the ones Task Manager shows. The memory column is dedicated video memory committed to each process, which isn't what it has on the card: memory the card has paged out to system RAM still counts, and a surface shared between processes counts in each of them (most of all in dwm.exe, which holds every window's). So a row can overstate what a process really occupies, and the rows can add up to more than the card has in use.";

/// How much of the graphics card each process is using, and how much of its memory: looked at as often as it's asked.
/// The first look says nothing of how busy anything is, as that's measured between two.
pub struct Gpus {
    query: windows::Win32::System::Performance::PDH_HQUERY,
    busy: windows::Win32::System::Performance::PDH_HCOUNTER,
    memory: windows::Win32::System::Performance::PDH_HCOUNTER,
    /// Each card's dedicated memory in use, all told.
    whole: windows::Win32::System::Performance::PDH_HCOUNTER,
    /// The cards that aren't cards (WARP), as the counters' names spell them.
    made_up: Vec<String>,
}

// It's handles to a query of its own, used by one thread at a time.
unsafe impl Send for Gpus {}

impl Gpus {
    /// None if Windows hasn't those counters.
    pub fn new() -> Option<Gpus> {
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1};
        use windows::Win32::System::Performance::{PDH_HCOUNTER, PDH_HQUERY, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhOpenQueryW};
        let mut made_up = Vec::new();
        if let Ok(factory) = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() } {
            for i in 0.. {
                let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else { break };
                let Ok(d) = (unsafe { adapter.GetDesc1() }) else { continue };
                // Software, by its flag; and by who it says made it, which is Microsoft's own basic renderer.
                if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 || (d.VendorId, d.DeviceId) == (0x1414, 0x8c) {
                    made_up.push(format!("0x{:08x}_0x{:08x}", d.AdapterLuid.HighPart as u32, d.AdapterLuid.LowPart));
                }
            }
        }
        unsafe {
            let mut query = PDH_HQUERY::default();
            if PdhOpenQueryW(None, 0, &mut query) != 0 {
                return None;
            }
            let (mut busy, mut memory, mut whole) = (PDH_HCOUNTER::default(), PDH_HCOUNTER::default(), PDH_HCOUNTER::default());
            let added = PdhAddEnglishCounterW(query, windows_core::w!(r"\GPU Engine(*)\Utilization Percentage"), 0, &mut busy) == 0
                && PdhAddEnglishCounterW(query, windows_core::w!(r"\GPU Process Memory(*)\Dedicated Usage"), 0, &mut memory) == 0;
            if !added {
                PdhCloseQuery(query);
                return None;
            }
            // Not every Windows has this one: without it there's only no total to give.
            PdhAddEnglishCounterW(query, windows_core::w!(r"\GPU Adapter Memory(*)\Dedicated Usage"), 0, &mut whole);
            PdhCollectQueryData(query);
            Some(Gpus { query, busy, memory, whole, made_up })
        }
    }

    /// For each process that's using a card: how much of it (a percentage, the busiest of the card's engines, as Task
    /// Manager has it), and how many bytes of its dedicated memory are committed to it. Committed, which isn't the same as
    /// on the card: see `GPU_MEMORY_IS`.
    pub fn look(&mut self) -> std::collections::HashMap<u32, (f32, u64)> {
        use std::collections::HashMap;
        unsafe { windows::Win32::System::Performance::PdhCollectQueryData(self.query) };
        // A counter's instance is named for its process, its card, and for an engine which of the card's: pid_1234_luid_0x…_0x…_phys_0_eng_3_engtype_3D.
        let named = |name: &str| -> Option<(u32, String)> {
            let rest = name.strip_prefix("pid_")?;
            let (pid, rest) = rest.split_once('_')?;
            let card = rest.strip_prefix("luid_")?.to_ascii_lowercase();
            (!self.made_up.iter().any(|made_up| card.starts_with(made_up.as_str()))).then_some((pid.parse().ok()?, card))
        };
        let mut engines: HashMap<(u32, String), f64> = HashMap::new();
        for (name, value) in crate::meters::instances(self.busy).unwrap_or_default() {
            if let Some(which) = named(&name) {
                *engines.entry(which).or_default() += value;
            }
        }
        let mut found: HashMap<u32, (f32, u64)> = HashMap::new();
        for ((pid, _), busy) in engines {
            let so_far = found.entry(pid).or_default();
            so_far.0 = so_far.0.max(busy.clamp(0.0, 100.0) as f32);
        }
        for (name, value) in crate::meters::instances(self.memory).unwrap_or_default() {
            if let Some((pid, _)) = named(&name) {
                found.entry(pid).or_default().1 += value.max(0.0) as u64;
            }
        }
        found
    }
}

impl Gpus {
    /// How much dedicated memory the cards have in use in all, as of the last look: what's actually on them, which the
    /// processes' own figures needn't add up to. None if Windows doesn't say.
    pub fn in_use(&self) -> Option<u64> {
        let cards = crate::meters::instances(self.whole).ok()?;
        let real: Vec<f64> = cards.iter().filter(|(name, _)| name.strip_prefix("luid_").is_some_and(|card| !self.made_up.iter().any(|made_up| card.to_ascii_lowercase().starts_with(made_up.as_str())))).map(|(_, bytes)| *bytes).collect();
        (!real.is_empty()).then(|| real.iter().sum::<f64>().max(0.0) as u64)
    }
}

impl Drop for Gpus {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Performance::PdhCloseQuery(self.query) };
    }
}

/// How busy a process has been between two looks, as Windows says it: a percentage of all the processors there are.
/// `before` and `now` are its `busy` at each look, `apart` how long between them.
pub fn share(before: u64, now: u64, apart: std::time::Duration) -> f32 {
    let steps = apart.as_nanos() as f64 / 100.0 * f64::from(processors());
    if steps <= 0.0 { 0.0 } else { (now.saturating_sub(before) as f64 / steps * 100.0).clamp(0.0, 100.0) as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_can_be_measured() {
        let (busy, private) = measured(std::process::id()).expect("ourselves");
        assert!(private > 100_000, "{private} bytes of our own");
        let _ = busy;
        assert!(all().iter().any(|(pid, ..)| *pid == std::process::id()));
        // The system's own list says much the same of us as we do when asked: and it's all there is for another user's.
        let (_, listed_private) = listed(std::process::id()).expect("ourselves, in the system's list");
        assert!(listed_private > 100_000 && listed_private < private * 4 + 50_000_000, "{listed_private} against {private}");
        let dwm = dwm().expect("a desktop window manager");
        assert!(dwm.private > 1_000_000, "{}", dwm.private);
        // The graphics counters can be asked, whatever they say.
        if let Some(mut gpus) = Gpus::new() {
            let _ = gpus.look();
        }
    }

    #[test]
    fn a_share_is_out_of_every_processor() {
        // One processor flat out for a second, of however many there are.
        let one = share(0, 10_000_000, std::time::Duration::from_secs(1));
        assert!((one - 100.0 / processors() as f32).abs() < 0.01, "{one}");
        assert_eq!(share(5, 5, std::time::Duration::from_secs(1)), 0.0);
        assert_eq!(share(0, 10, std::time::Duration::ZERO), 0.0);
    }
}
