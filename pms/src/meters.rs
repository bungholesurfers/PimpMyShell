//! Meters: sample performance counters, evaluate each meter's expressions, and push the results
//! to the TAP, each meter at its own interval.
//!
//! Counters are added with `PdhAddEnglishCounterW`, so configs use English paths regardless of
//! the display language. Wildcard paths (`\Network Interface(*)\...`) are aggregated across
//! instances (sum by default), skipping `_Total` so it isn't double counted.
//!
//! Meters sharing an interval share a PDH query holding just the counters they use; a query is
//! collected only when its interval comes round. The `cores` style reads per-CPU user and
//! privileged time itself and lays the bars out by the CPU topology Windows reports.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Performance::*;
use windows::Win32::System::SystemInformation::*;
use windows_core::{HSTRING, PCWSTR, s, w};

use crate::config::{self, Agg, Config, CounterSpec, Exprs, Max, MeterSpec, Style};
use crate::expr::{self, Expr};

const PDH_FMT_NOCAP100: u32 = 0x8000;
/// Below this an interval is pointless: about one frame at 60 Hz.
const FRAME_MS: u64 = 16;
/// Samples a `bar` without `max` looks back over to scale itself.
const AUTO_WINDOW: usize = 60;
/// A new sampler's first sample, at the latest.
const FIRST_SAMPLE: Duration = Duration::from_millis(150);
const CORE_USER: &str = r"\Processor Information(*)\% User Time";
const CORE_KERNEL: &str = r"\Processor Information(*)\% Privileged Time";
const GPU_ENGINES: &str = r"\GPU Engine(*)\Utilization Percentage";
/// How often the list of GPUs present is refreshed (an engine stays while its GPU is there).
const GPU_REFRESH: Duration = Duration::from_secs(30);

fn pdh(status: u32, what: &str) -> Result<(), String> {
    if status == 0 { Ok(()) } else { Err(format!("{what}: PDH status {status:#010x}")) }
}

/// Meters with `per_gpu = true`, one per graphics adapter with dedicated memory: each gets a counter of its
/// adapter's dedicated memory in use (`vram_gpuN`), a builtin of its size (`vram_total_gpuN`), its maker's color,
/// and the adapter's name after its label (for the tooltip). None if no meter asks for it.
fn per_gpu(config: &Config, builtins: &mut HashMap<String, f64>) -> Option<Config> {
    if !config.meters.iter().any(|m| m.per_gpu == Some(true)) {
        return None;
    }
    // The least memory an adapter needs for its own meter: the strictest any per_gpu meter asks.
    let least = config.meters.iter().filter(|m| m.per_gpu == Some(true)).map(|m| m.min_memory_mb.unwrap_or(1024)).max().unwrap_or(1024) as usize * 1024 * 1024;
    let mut adapters = Vec::new();
    if let Ok(factory) = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() } {
        for i in 0.. {
            let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else { break };
            let Ok(d) = (unsafe { adapter.GetDesc1() }) else { continue };
            let software = d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0;
            if d.DedicatedVideoMemory == 0 || d.DedicatedVideoMemory < least || (software && !config.software_gpus) {
                continue;
            }
            let name = String::from_utf16_lossy(&d.Description[..d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len())]);
            adapters.push((name, d.AdapterLuid.HighPart as u32, d.AdapterLuid.LowPart, d.DedicatedVideoMemory as f64));
        }
    }
    let mut out = config.clone();
    out.meters.clear();
    for spec in &config.meters {
        if spec.per_gpu != Some(true) {
            out.meters.push(spec.clone());
            continue;
        }
        for (k, (name, high, low, size)) in adapters.iter().enumerate() {
            let (counter, total) = (format!("vram_gpu{k}"), format!("vram_total_gpu{k}"));
            // The adapter's memory counter instances are `luid_0x..._0x..._phys_N`.
            let path = format!(r"\GPU Adapter Memory(luid_0x{high:08X}_0x{low:08X}_phys_*)\Dedicated Usage");
            out.counters.insert(counter.clone(), CounterSpec::Full { path, agg: Agg::Sum, default: Some(0.0) });
            builtins.insert(total.clone(), *size);
            let mut m = spec.clone();
            m.per_gpu = None;
            m.expr = Some(Exprs::One(counter));
            m.max = Some(Max::Expr(total));
            if let Some(v) = pms_tap::vendors::vendor(name) {
                m.color = Some(v.bar_hex());
                m.colors = vec![v.bar_hex()];
            }
            m.label = format!("{} · {name}", spec.label);
            out.meters.push(m);
        }
    }
    Some(out)
}

/// Dedicated memory of every hardware adapter, which is what the `GPU Adapter Memory`
/// `Dedicated Usage` counters add up against.
fn vram_total() -> Option<f64> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.ok()?;
    let mut total = 0u64;
    for i in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else { break };
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else { continue };
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0 {
            total += desc.DedicatedVideoMemory as u64;
        }
    }
    (total > 0).then_some(total as f64)
}

/// Pagefile (swap) space in bytes: (allocated, in use), summed over all pagefiles. Read live;
/// pagefiles grow and shrink. (0, 0) without a pagefile.
fn swap() -> (f64, f64) {
    type Query = unsafe extern "system" fn(u32, *mut std::ffi::c_void, u32, *mut u32) -> i32;
    const SYSTEM_PAGEFILE_INFORMATION: u32 = 18;
    let mut si = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut si) };
    let page = si.dwPageSize as f64;
    unsafe {
        let Ok(ntdll) = GetModuleHandleW(w!("ntdll.dll")) else { return (0.0, 0.0) };
        let Some(f) = GetProcAddress(ntdll, s!("NtQuerySystemInformation")) else { return (0.0, 0.0) };
        let f: Query = std::mem::transmute(f);
        let mut buf = vec![0u64; 512];
        let mut len = 0u32;
        if f(SYSTEM_PAGEFILE_INFORMATION, buf.as_mut_ptr() as _, (buf.len() * 8) as u32, &mut len) != 0 {
            return (0.0, 0.0);
        }
        // SYSTEM_PAGEFILE_INFORMATION: NextEntryOffset, TotalSize, TotalInUse, PeakUsage (pages), name.
        let base = buf.as_ptr() as *const u8;
        let (mut total, mut used, mut off) = (0.0, 0.0, 0usize);
        while len > 0 && off + 16 <= (buf.len() * 8) {
            let entry = base.add(off) as *const u32;
            total += *entry.add(1) as f64 * page;
            used += *entry.add(2) as f64 * page;
            match *entry {
                0 => break,
                next => off += next as usize,
            }
        }
        (total, used)
    }
}

/// Values available to expressions without a counter.
fn builtins() -> HashMap<String, f64> {
    let mut env = HashMap::new();
    let (swap_total, swap_used) = swap();
    env.insert("swap_total".into(), swap_total);
    env.insert("swap_used".into(), swap_used);
    let mut mem = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut mem) }.is_ok() {
        env.insert("ram_total".into(), mem.ullTotalPhys as f64);
    }
    let mut si = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut si) };
    env.insert("cpu_count".into(), si.dwNumberOfProcessors as f64);
    if let Some(v) = vram_total() {
        env.insert("vram_total".into(), v);
    }
    env
}

/// The system timer's current resolution in ms (15.6 by default; lower while some app has
/// raised it). Waits can't be finer than this.
fn timer_resolution_ms() -> f64 {
    type Query = unsafe extern "system" fn(*mut u32, *mut u32, *mut u32) -> i32;
    unsafe {
        let Ok(ntdll) = GetModuleHandleW(w!("ntdll.dll")) else { return 15.625 };
        let Some(f) = GetProcAddress(ntdll, s!("NtQueryTimerResolution")) else { return 15.625 };
        let f: Query = std::mem::transmute(f);
        let (mut min, mut max, mut cur) = (0, 0, 0);
        if f(&mut min, &mut max, &mut cur) == 0 { cur as f64 / 10_000.0 } else { 15.625 }
    }
}

/// Shortest interval a meter may ask for right now.
pub fn min_interval_ms() -> u64 {
    (timer_resolution_ms().ceil() as u64).max(FRAME_MS)
}

// ---------------------------------------------------------------------------------------------
// CPU topology

/// Logical processors in display order as (group, number), and the `cores` topology string
/// for the TAP (see pms-tap/src/meters.rs).
pub struct Topology {
    pub cpus: Vec<(u16, u32)>,
    pub layout: String,
}

fn bits(group: u16, mask: usize) -> impl Iterator<Item = (u16, u32)> {
    (0..usize::BITS).filter(move |b| mask & (1 << b) != 0).map(move |b| (group, b))
}

#[allow(non_upper_case_globals)] // RelationProcessorCore etc. are Windows constants
pub fn topology() -> Result<Topology, String> {
    let mut len = 0u32;
    let _ = unsafe { GetLogicalProcessorInformationEx(RelationAll, None, &mut len) };
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    unsafe { GetLogicalProcessorInformationEx(RelationAll, Some(buf.as_mut_ptr() as _), &mut len) }.map_err(|e| e.to_string())?;

    // (group, mask, efficiency class) per core; (group, mask) per L3 and per NUMA node.
    let (mut cores, mut l3s, mut nodes) = (Vec::new(), Vec::new(), Vec::new());
    let base = buf.as_ptr() as *const u8;
    let mut off = 0usize;
    while off < len as usize {
        let info = unsafe { &*(base.add(off) as *const SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX) };
        unsafe {
            match info.Relationship {
                RelationProcessorCore => {
                    let p = &info.Anonymous.Processor;
                    let m = p.GroupMask[0];
                    cores.push((m.Group, m.Mask, p.EfficiencyClass));
                }
                RelationCache if info.Anonymous.Cache.Level == 3 => {
                    let m = info.Anonymous.Cache.Anonymous.GroupMask;
                    l3s.push((m.Group, m.Mask));
                }
                RelationNumaNode => {
                    let m = info.Anonymous.NumaNode.Anonymous.GroupMask;
                    nodes.push((m.Group, m.Mask));
                }
                _ => {}
            }
        }
        off += info.Size as usize;
    }
    cores.sort_by_key(|&(g, m, _)| (g, m.trailing_zeros()));
    let top_class = cores.iter().map(|c| c.2).max().unwrap_or(0);
    let hybrid = cores.iter().any(|c| c.2 != top_class);
    let find = |sets: &[(u16, usize)], (g, m): (u16, usize)| sets.iter().position(|&(sg, sm)| sg == g && sm & m != 0);

    let mut cpus = Vec::new();
    let mut layout = String::new();
    let mut prev: Option<(Option<usize>, Option<usize>)> = None;
    for &(g, m, class) in &cores {
        let here = (find(&nodes, (g, m)), find(&l3s, (g, m)));
        match prev {
            None => {}
            Some((node, _)) if node != here.0 => layout.push('|'),
            Some((_, l3)) if l3 != here.1 => layout.push('/'),
            Some(_) => layout.push(','),
        }
        prev = Some(here);
        if hybrid && class < top_class {
            layout.push('e');
        }
        layout += &m.count_ones().to_string();
        cpus.extend(bits(g, m));
    }
    Ok(Topology { cpus, layout })
}

// ---------------------------------------------------------------------------------------------
// Sampling

struct Counter {
    name: String,
    path: String,
    handle: PDH_HCOUNTER,
    agg: Agg,
    default: Option<f64>,
}

/// One PDH query per distinct interval.
struct Group {
    interval: Duration,
    next: Instant,
    query: PDH_HQUERY,
    counters: Vec<Counter>,
    /// Per-CPU (user, privileged) counters, if a `cores` meter is in this group: they only
    /// split busy time into user and kernel; how busy comes from idle cycles (see Cycles).
    cores: Option<(PDH_HCOUNTER, PDH_HCOUNTER)>,
    cycles: Option<Cycles>,
    /// The GPU Engine utilization counter, if an `engines` meter is in this group.
    engines: Option<PDH_HCOUNTER>,
    meters: Vec<usize>,
}

/// Per-CPU busy fraction from idle cycle counts against the cycle counter. User/privileged time
/// is only charged once per clock tick (15.6 ms), so over short intervals it's a few lumpy
/// ticks or nothing; idle cycles are exact. The ticks still tell us the user/kernel split,
/// which we smooth.
struct Cycles {
    idle: Vec<u64>,
    tsc: u64,
    /// Smoothed user share of busy time, per CPU.
    user_share: Vec<f64>,
}

impl Group {
    /// Whether the group's query has counters to collect.
    fn reads(&self) -> bool {
        !self.counters.is_empty() || self.cores.is_some() || self.engines.is_some()
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

enum Scale {
    Auto,
    Fixed(f64),
    Expr(Expr),
}

struct Meter {
    spec: MeterSpec,
    exprs: Vec<Expr>,
    scale: Scale,
    /// Recent values, for auto-scaled bars.
    recent: VecDeque<f64>,
    /// Whether the TAP shows it (None: not decided yet), and since when it has had nothing to show.
    shown: Option<bool>,
    absent_since: Option<Instant>,
    threshold: Option<Scale>,
    /// Engines: every engine seen working (and when), and the ones on screen.
    engines: HashMap<EngineKey, Instant>,
    engines_shown: Vec<EngineKey>,
    /// Engines: each shown GPU's power state, as last sent ("P8"; empty where unknown).
    states_sent: Vec<String>,
    /// Last `meters tip` sent.
    tip: String,
}

/// Default colors for meters that don't set their own.
const PALETTE: &[&str] = &["#4FC3F7", "#BA68C8", "#AED581", "#FFD54F", "#FF8A65", "#4DB6AC", "#F06292", "#9575CD"];
const CORE_COLORS: [&str; 2] = ["#66BB6A", "#E040FB"];

impl Meter {
    fn colors(&self, i: usize) -> Vec<String> {
        let fallback = || PALETTE[i % PALETTE.len()].to_string();
        match self.spec.style {
            Style::Cores => {
                let c = &self.spec.colors;
                vec![c.first().cloned().unwrap_or(CORE_COLORS[0].into()), c.get(1).cloned().unwrap_or(CORE_COLORS[1].into())]
            }
            Style::Pie if !self.spec.colors.is_empty() => self.spec.colors.clone(),
            _ => vec![self.spec.color.clone().or(self.spec.colors.first().cloned()).unwrap_or_else(fallback)],
        }
    }
}

pub struct Sampler {
    groups: Vec<Group>,
    value_font: Option<String>,
    meters: Vec<Meter>,
    builtins: HashMap<String, f64>,
    topology: Option<Topology>,
    /// Adapter LUID (as GPU Engine instance names spell it) -> adapter name.
    gpus: HashMap<String, String>,
    gpus_checked: Instant,
    /// Show software adapters (Microsoft Basic Render Driver) too.
    software_gpus: bool,
    /// NVIDIA's management library, for its GPUs' power states.
    nvml: Option<crate::nvml::Nvml>,
    warned: HashSet<String>,
}

impl Sampler {
    pub fn new(config: &Config) -> Result<Sampler, String> {
        if !["compact", "tall", "full"].contains(&config.meter_height.as_str()) {
            return Err(format!("meter_height = \"{}\": use \"compact\", \"tall\" or \"full\"", config.meter_height));
        }
        let mut builtins = builtins();
        let expanded = per_gpu(config, &mut builtins);
        let config = expanded.as_ref().unwrap_or(config);
        let known: Vec<String> = config.counters.keys().cloned().chain(builtins.keys().cloned()).collect();
        let parse = |label: &str, src: &str| -> Result<Expr, String> {
            let e = expr::parse(src).map_err(|e| format!("meter `{label}`: {e}"))?;
            let mut vars = Vec::new();
            e.vars(&mut vars);
            match vars.iter().find(|v| !known.contains(v)) {
                Some(v) => Err(format!("meter `{label}` uses `{v}`, which is not a counter or builtin ({})", known.join(", "))),
                None => Ok(e),
            }
        };

        let mut meters = Vec::new();
        for spec in &config.meters {
            if let Some(v) = spec.time_axis.as_deref().filter(|v| !["log", "linear"].contains(v)) {
                return Err(format!("meter `{}`: time_axis = \"{v}\": use \"log\" or \"linear\"", spec.label));
            }
            if let Some(v) = spec.glow.as_deref().filter(|v| !["level", "heat", "off"].contains(v)) {
                return Err(format!("meter `{}`: glow = \"{v}\": use \"level\", \"heat\" or \"off\"", spec.label));
            }
            if let Some(v) = spec.vendor_label.as_deref().filter(|v| !["ticker", "short"].contains(v)) {
                return Err(format!("meter `{}`: vendor_label = \"{v}\": use \"ticker\" or \"short\"", spec.label));
            }
            if let Some(v) = spec.label_inside.as_deref().filter(|v| !["always", "never", "auto"].contains(v)) {
                return Err(format!("meter `{}`: label_inside = \"{v}\": use \"always\", \"never\" or \"auto\"", spec.label));
            }
            let exprs = match (&spec.expr, spec.style) {
                (_, Style::Cores | Style::Engines) => Vec::new(),
                (Some(e), _) => e.list().iter().map(|s| parse(&spec.label, s)).collect::<Result<_, _>>()?,
                (None, _) => return Err(format!("meter `{}` needs an `expr`", spec.label)),
            };
            let scale = match &spec.max {
                None => Scale::Auto,
                Some(Max::Number(n)) => Scale::Fixed(*n),
                Some(Max::Expr(s)) => Scale::Expr(parse(&spec.label, s)?),
            };
            let threshold = match &spec.show_above {
                None => None,
                Some(Max::Number(n)) => Some(Scale::Fixed(*n)),
                Some(Max::Expr(s)) => Some(Scale::Expr(parse(&spec.label, s)?)),
            };
            meters.push(Meter {
                spec: spec.clone(),
                exprs,
                scale,
                recent: VecDeque::new(),
                shown: None,
                absent_since: None,
                threshold,
                engines: HashMap::new(),
                engines_shown: Vec::new(),
                states_sent: Vec::new(),
                tip: String::new(),
            });
        }
        let topology = if meters.iter().any(|m| m.spec.style == Style::Cores) { Some(topology()?) } else { None };

        // Group meters by interval; each group's query gets only the counters its meters use.
        let floor = min_interval_ms();
        let mut by_interval: Vec<(u64, Vec<usize>)> = Vec::new();
        for (i, m) in meters.iter().enumerate() {
            let ms = m.spec.interval_ms.unwrap_or(config.interval_ms).max(floor);
            match by_interval.iter_mut().find(|(g, _)| *g == ms) {
                Some((_, v)) => v.push(i),
                None => by_interval.push((ms, vec![i])),
            }
        }
        let now = Instant::now();
        let mut groups = Vec::new();
        for (ms, members) in by_interval {
            let mut query = PDH_HQUERY::default();
            pdh(unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) }, "PdhOpenQuery")?;
            // From here on Drop closes the query, including on the error paths below.
            let mut group =
                Group { interval: Duration::from_millis(ms), next: now, query, counters: Vec::new(), cores: None, cycles: None, engines: None, meters: members };
            let mut wanted = Vec::new();
            for &i in &group.meters {
                let m = &meters[i];
                let max_expr = match &m.scale {
                    Scale::Expr(e) => Some(e),
                    _ => None,
                };
                for e in m.exprs.iter().chain(max_expr) {
                    e.vars(&mut wanted);
                }
                if m.spec.style == Style::Engines && group.engines.is_none() {
                    let mut h = PDH_HCOUNTER::default();
                    pdh(unsafe { PdhAddEnglishCounterW(query, &HSTRING::from(GPU_ENGINES), 0, &mut h) }, GPU_ENGINES)?;
                    group.engines = Some(h);
                }
                if m.spec.style == Style::Cores && group.cores.is_none() {
                    let add = |path: &str| -> Result<PDH_HCOUNTER, String> {
                        let mut h = PDH_HCOUNTER::default();
                        pdh(unsafe { PdhAddEnglishCounterW(query, &HSTRING::from(path), 0, &mut h) }, path)?;
                        Ok(h)
                    };
                    group.cores = Some((add(CORE_USER)?, add(CORE_KERNEL)?));
                    let cpus = topology.as_ref().map_or(&[][..], |t| &t.cpus[..]);
                    group.cycles = Some(Cycles { idle: idle_cycles(cpus), tsc: tsc(), user_share: vec![0.6; cpus.len()] });
                }
            }
            for (name, spec) in &config.counters {
                if !wanted.contains(name) {
                    continue;
                }
                let (path, agg, default) = match spec {
                    CounterSpec::Path(p) => (p.clone(), Agg::default(), None),
                    CounterSpec::Full { path, agg, default } => (path.clone(), *agg, *default),
                };
                let mut handle = PDH_HCOUNTER::default();
                pdh(unsafe { PdhAddEnglishCounterW(query, &HSTRING::from(&path), 0, &mut handle) }, &format!("counter `{name}` ({path})"))?;
                group.counters.push(Counter { name: name.clone(), path, handle, agg, default });
            }
            // Rate counters need two samples before they mean anything; this is the first. (A group of
            // constant expressions has no counters: PDH says "no data" for an empty query.)
            if group.reads() {
                pdh(unsafe { PdhCollectQueryData(query) }, "PdhCollectQueryData")?;
            }
            // The first sample comes soon (rates over a short first window are fine), so a fresh
            // strip isn't blank for a whole interval.
            group.next = now + group.interval.min(FIRST_SAMPLE);
            groups.push(group);
        }
        // Always: it also notes the software adapters for `agg = "engine"` counters to leave out.
        let gpus = gpu_names(config.software_gpus);
        Ok(Sampler {
            groups,
            value_font: config.value_font.clone(),
            meters,
            builtins,
            topology,
            gpus,
            gpus_checked: Instant::now(),
            software_gpus: config.software_gpus,
            nvml: if config.meters.iter().any(|m| m.style == Style::Engines) { crate::nvml::Nvml::open() } else { None },
            warned: HashSet::new(),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.meters.is_empty()
    }

    pub fn describe(&self) -> String {
        let intervals: Vec<String> = self.groups.iter().map(|g| format!("{}ms x{}", g.interval.as_millis(), g.meters.len())).collect();
        format!("{} meter(s) [{}]", self.meters.len(), intervals.join(", "))
    }

    /// TAP commands that (re)create the strip: height, then each meter, with a separator wherever
    /// the group changes.
    pub fn define(&mut self, height: &str) -> Vec<String> {
        let q = crate::quote;
        // A fresh strip knows nothing: presence has to be decided (and said) again.
        for m in &mut self.meters {
            (m.shown, m.absent_since) = (None, None);
            m.states_sent.clear();
        }
        let mut out = vec!["meters clear".to_string(), format!("meters height {height}")];
        let mut group: Option<&str> = None;
        for (i, m) in self.meters.iter_mut().enumerate() {
            let this = m.spec.group.as_deref();
            // A snug meter sits right against the one before it, whatever its group.
            if i > 0 && this != group && m.spec.snug != Some(true) {
                out.push("meters sep".into());
            }
            group = this;
            let colors = m.colors(i);
            let mut fields = vec![format!("label={}", m.spec.label), format!("wide={}", widest(&m.spec))];
            match m.spec.style {
                Style::Engines => {}
                Style::Bars => {
                    let n = m.exprs.len().max(1);
                    let colors: Vec<String> =
                        (0..n).map(|j| m.spec.colors.get(j).cloned().unwrap_or_else(|| colors[j % colors.len()].clone())).collect();
                    fields.push("style=bars".into());
                    fields.push(format!("colors={}", colors.join(",")));
                }
                Style::History => {
                    fields.push("style=history".into());
                    let interval = self.groups.iter().find(|g| g.meters.contains(&i)).map_or(1000, |g| g.interval.as_millis() as u64).max(1);
                    fields.push(format!("span={}", m.spec.history_seconds.unwrap_or(300) * 1000 / interval));
                    if m.spec.time_axis.as_deref() == Some("linear") {
                        fields.push("time=linear".into());
                    }
                    fields.push(format!("color={}", colors[0]));
                    if let Scale::Fixed(n) = m.scale {
                        fields.push(format!("max={n}"));
                    }
                }
                Style::Bar => {
                    fields.push("style=bar".into());
                    fields.push(format!("color={}", colors[0]));
                }
                Style::Pie => {
                    let n = m.exprs.len().max(1);
                    let colors: Vec<String> =
                        (0..n).map(|j| colors.get(j).cloned().unwrap_or_else(|| PALETTE[(i + j) % PALETTE.len()].into())).collect();
                    fields.push("style=pie".into());
                    fields.push(format!("colors={}", colors.join(",")));
                }
                Style::Cores => {
                    fields.push("style=cores".into());
                    fields.push(format!("colors={},{}", colors[0], colors[1]));
                    fields.push(format!("topo={}", self.topology.as_ref().map_or("1".into(), |t| t.layout.clone())));
                }
            }
            if matches!(m.spec.style, Style::Engines) {
                fields.push("style=engines".into());
                fields.push(format!("color={}", colors[0]));
            }
            if m.spec.value == Some(false) {
                fields.push("novalue=1".into());
            }
            if m.spec.vendors == Some(false) {
                fields.push("novendors=1".into());
            }
            if m.spec.caption == Some(false) {
                fields.push("nocaption=1".into());
            }
            if m.spec.snug == Some(true) {
                fields.push("snug=1".into());
            }
            if let Some(v) = &m.spec.vendor_label {
                fields.push(format!("vendorlabel={v}"));
            }
            if let Some(g) = &m.spec.glow {
                fields.push(format!("glow={g}"));
            }
            if m.spec.tooltip == Some(false) {
                fields.push("notip=1".into());
            }
            if let Some(bw) = m.spec.bar_width {
                fields.push(format!("barw={bw}"));
            }
            if let Some(p) = m.spec.pressure {
                fields.push(format!("pressure={p}"));
            }
            if let Some(inside) = &m.spec.label_inside {
                fields.push(format!("inside={inside}"));
            }
            if let Some(font) = &self.value_font {
                fields.push(format!("font={font}"));
            }
            // The TAP needs to hear about bars and tooltips again.
            m.engines_shown.clear();
            m.tip.clear();
            out.push(std::iter::once("meters add".to_string()).chain(fields.iter().map(|f| q(f))).collect::<Vec<_>>().join(" "));
            if m.spec.style == Style::Bars {
                let names: Vec<String> = (0..m.exprs.len()).map(|j| q(m.spec.names.get(j).map_or(&(j + 1).to_string(), |n| n))).collect();
                out.push(format!("meters bars {i} {}", names.join(" ")));
            }
        }
        out
    }

    /// When the next group is due.
    pub fn next_due(&self) -> Option<Instant> {
        self.groups.iter().map(|g| g.next).min()
    }

    /// Collect every group that is due and return the TAP commands for its meters.
    pub fn sample_due(&mut self, now: Instant) -> Vec<String> {
        let mut out = Vec::new();
        for gi in 0..self.groups.len() {
            if self.groups[gi].next > now {
                continue;
            }
            let g = &mut self.groups[gi];
            g.next += g.interval;
            if g.next <= now {
                g.next = now + g.interval;
            }
            let t0 = Instant::now();
            let collected = if g.reads() { pdh(unsafe { PdhCollectQueryData(g.query) }, "PdhCollectQueryData") } else { Ok(()) };
            if crate::trace() {
                crate::log(format!("trace: group {}ms collected in {:.1}ms", g.interval.as_millis(), t0.elapsed().as_secs_f64() * 1000.0));
            }
            if let Err(e) = collected {
                crate::log(e);
                continue;
            }
            // A failing counter leaves its name unset (meters using it show "–") unless it
            // has a default. Say so once, not every tick.
            let mut env = self.builtins.clone();
            let (swap_total, swap_used) = swap();
            env.insert("swap_total".into(), swap_total);
            env.insert("swap_used".into(), swap_used);
            for c in &self.groups[gi].counters {
                match read(c) {
                    Ok(v) => {
                        env.insert(c.name.clone(), v);
                        self.warned.remove(&c.name);
                    }
                    Err(e) => {
                        if let Some(d) = c.default {
                            env.insert(c.name.clone(), d);
                        } else if self.warned.insert(c.name.clone()) {
                            crate::log(format!("{e} (warning once; meters using `{}` show –)", c.name));
                        }
                    }
                }
            }
            let cpus = self.topology.as_ref().map_or(&[][..], |t| &t.cpus[..]);
            let g = &mut self.groups[gi];
            let cores = match (g.cores, g.cycles.as_mut()) {
                (Some((u, k)), Some(cycles)) => Some(core_values(cpus, u, k, cycles)),
                _ => None,
            };
            let engines = self.groups[gi].engines.map(engine_utilization);
            if engines.is_some() && self.gpus_checked.elapsed() >= GPU_REFRESH {
                self.gpus = gpu_names(self.software_gpus);
                self.gpus_checked = Instant::now();
            }
            for mi in self.groups[gi].meters.clone() {
                out.extend(self.push_lines(mi, &env, cores.as_deref(), engines.as_ref(), now));
            }
        }
        out
    }

    /// `meters push` for meter `i`, preceded by a `meters presence` change if its data came or
    /// went. A meter with nothing to show (no swap, a counter that doesn't exist) starts hidden;
    /// one that loses its data keeps saying so for a while before it leaves.
    fn push_lines(
        &mut self,
        i: usize,
        env: &HashMap<String, f64>,
        cores: Option<&[(f64, f64)]>,
        engines: Option<&HashMap<EngineKey, f64>>,
        now: Instant,
    ) -> Vec<String> {
        let q = crate::quote;
        let gpus = &self.gpus;
        let nvml = self.nvml.as_ref();
        let m = &mut self.meters[i];
        let mut pre = Vec::new();
        let values: Vec<f64> = m.exprs.iter().map(|e| e.eval(env).unwrap_or(f64::NAN)).collect();
        let v0 = values.first().copied().unwrap_or(f64::NAN);
        let max = match &m.scale {
            Scale::Auto => None,
            Scale::Fixed(n) => Some(*n),
            Scale::Expr(e) => e.eval(env).ok(),
        };
        let mut absent = !v0.is_finite();
        let (text, numbers): (String, Vec<f64>) = match m.spec.style {
            Style::History => (format_value(v0, &m.spec), max.map_or(vec![v0], |mx| vec![v0, mx])),
            Style::Bar => {
                if v0.is_finite() {
                    if m.recent.len() == AUTO_WINDOW {
                        m.recent.pop_front();
                    }
                    m.recent.push_back(v0);
                }
                let scale = max.unwrap_or_else(|| m.recent.iter().copied().fold(0.0, f64::max) * 1.1);
                (format_value(v0, &m.spec), vec![if scale > 0.0 { v0 / scale } else { 0.0 }])
            }
            Style::Pie => {
                let sum: f64 = values.iter().filter(|v| v.is_finite()).sum();
                let whole = max.unwrap_or(sum);
                absent |= whole <= 0.0;
                let text = match (whole > 0.0, values.len()) {
                    (false, _) => "none".to_string(),
                    (true, 1) => format_value(v0, &m.spec),
                    (true, _) => format_value(sum, &m.spec),
                };
                (text, values.iter().map(|v| if whole > 0.0 && v.is_finite() { v / whole } else { 0.0 }).collect())
            }
            Style::Cores => {
                let c = cores.unwrap_or(&[]);
                absent = c.is_empty();
                let busy = if c.is_empty() { f64::NAN } else { c.iter().map(|(u, k)| u + k).sum::<f64>() / c.len() as f64 * 100.0 };
                let mut spec = m.spec.clone();
                if spec.unit.is_empty() {
                    spec.unit = "%".into();
                }
                (format_value(busy, &spec), c.iter().flat_map(|&(u, k)| [u, k]).collect())
            }
            Style::Bars => {
                if values.iter().any(|v| v.is_finite()) {
                    if m.recent.len() == AUTO_WINDOW {
                        m.recent.pop_front();
                    }
                    m.recent.push_back(values.iter().copied().filter(|v| v.is_finite()).fold(0.0, f64::max));
                }
                absent = !values.iter().any(|v| v.is_finite());
                let scale = max.unwrap_or_else(|| m.recent.iter().copied().fold(0.0, f64::max) * 1.1);
                // Each bar's own reading, for its tooltip.
                let tips: Vec<String> = values.iter().map(|v| q(&format_value(*v, &m.spec))).collect();
                let tip = tips.join(" ");
                if tip != m.tip {
                    pre.push(format!("meters bartips {i} {tip}"));
                    m.tip = tip;
                }
                (String::new(), values.iter().map(|v| if scale > 0.0 && v.is_finite() { v / scale } else { 0.0 }).collect())
            }
            Style::Engines => {
                let util = engines.cloned().unwrap_or_default();
                for (k, v) in &util {
                    if *v > 0.0 {
                        m.engines.insert(k.clone(), now);
                    }
                }
                // Whole adapters come and go, never single engines: an adapter shows while any of
                // its engines worked within `engine_hold_seconds` (0 = for good), and the most
                // recently active adapter always stays. A shown adapter keeps a bar for every engine
                // that ever worked (drivers list many that never do). Adapters must also still be
                // present (the adapter list, not the counter's instances, which idle engines can
                // drop out of).
                let hold = m.spec.engine_hold_seconds.unwrap_or(15);
                let mut adapters: HashMap<&str, Instant> = HashMap::new();
                for (k, t) in &m.engines {
                    let last = adapters.entry(k.0.as_str()).or_insert(*t);
                    *last = (*last).max(*t);
                }
                let latest = adapters.iter().max_by_key(|(_, t)| **t).map(|(a, _)| *a);
                let shown = |a: &str| {
                    let recent = adapters.get(a).is_some_and(|t| hold == 0 || now.duration_since(*t) < Duration::from_secs(hold));
                    (recent || Some(a) == latest) && (gpus.is_empty() || gpus.contains_key(a))
                };
                let mut on: Vec<EngineKey> = m.engines.keys().filter(|k| shown(&k.0)).cloned().collect();
                // Adapter, then engine type (same-type engines sit together), then index.
                on.sort_by(|a, b| (&a.0, a.1, &a.3, a.2).cmp(&(&b.0, b.1, &b.3, b.2)));
                if on != m.engines_shown {
                    let labels: Vec<String> = on.iter().map(|k| q(&engine_label(k, gpus))).collect();
                    pre.push(format!("meters bars {i} {}", labels.join(" ")));
                    m.engines_shown = on.clone();
                }
                // Each shown GPU's power state, in the order the strip shows them (NVIDIA's only, from NVML).
                if let Some(nvml) = nvml {
                    let all = nvml.states();
                    let mut seen: HashMap<&str, usize> = HashMap::new();
                    let mut states = Vec::new();
                    let mut last: Option<&str> = None;
                    for k in &on {
                        if last == Some(k.0.as_str()) {
                            continue;
                        }
                        last = Some(k.0.as_str());
                        let name = gpus.get(&k.0).map_or("", String::as_str);
                        let nth = seen.entry(name).or_default();
                        states.push(all.get(name).and_then(|s| s.get(*nth)).cloned().unwrap_or_default());
                        *nth += 1;
                    }
                    if states != m.states_sent {
                        let words: Vec<String> = states.iter().map(|s| if s.is_empty() { "-".to_string() } else { s.clone() }).collect();
                        pre.push(format!("meters gpustate {i} {}", words.join(" ")));
                        m.states_sent = states;
                    }
                }
                absent = on.is_empty();
                let vals: Vec<f64> = on.iter().map(|k| util.get(k).copied().unwrap_or(0.0) / 100.0).collect();
                let busiest = vals.iter().copied().fold(0.0, f64::max) * 100.0;
                let mut spec = m.spec.clone();
                if spec.unit.is_empty() {
                    spec.unit = "%".into();
                }
                (format_value(busiest, &spec), vals)
            }
        };

        // Threshold meters show only while their value is up there.
        let level = match m.spec.style {
            Style::Cores | Style::Engines => text.trim_end_matches('%').parse::<f64>().unwrap_or(f64::NAN),
            _ => v0,
        };
        let threshold = match &m.threshold {
            Some(Scale::Fixed(n)) => Some(*n),
            Some(Scale::Expr(e)) => e.eval(env).ok(),
            _ => None,
        };
        if let Some(t) = threshold {
            absent |= level.is_nan() || level < t;
        }

        // Pies explain themselves in the tooltip: named slices, and what they are out of.
        if m.spec.style == Style::Pie && (!m.spec.names.is_empty() || max.is_some()) {
            let mut parts: Vec<String> = if m.spec.names.is_empty() {
                vec![format_value(v0, &m.spec)]
            } else {
                m.spec.names.iter().zip(&values).map(|(n, v)| format!("{n} {}", format_value(*v, &m.spec))).collect()
            };
            if let Some(mx) = max {
                parts.push(format!("of {}", format_value(mx, &m.spec)));
            }
            let tip = parts.join(" · ");
            if tip != m.tip {
                pre.push(format!("meters tip {i} {}", q(&tip)));
                m.tip = tip;
            }
        }

        let mut out = pre;
        let show = match (m.shown, absent) {
            (None, absent) => !absent,
            (Some(false), absent) => !absent,
            (Some(true), false) => true,
            (Some(true), true) => m.absent_since.get_or_insert(now).elapsed() < ABSENT_GRACE,
        };
        if !absent {
            m.absent_since = None;
        }
        if m.shown != Some(show) {
            // Before the first value only "off" needs saying: the TAP shows a meter on its first push.
            if m.shown.is_some() || !show {
                out.push(format!("meters presence {i} {}", if show { "on" } else { "off" }));
            }
            m.shown = Some(show);
        }
        let mut line = format!("meters push {i} {}", q(&text));
        for n in numbers {
            line += &format!(" {}", if n.is_finite() { format!("{n:.4}") } else { "nan".into() });
        }
        out.push(line);
        out
    }
}

/// A GPU engine: (adapter LUID as spelled in instance names, physical adapter, engine index,
/// engine type). Sorts by adapter, then engine.
type EngineKey = (String, u32, u32, String);

/// `pid_1234_luid_0x00000000_0x0000D1E4_phys_0_eng_3_engtype_VideoDecode`.
fn engine_key(name: &str) -> Option<EngineKey> {
    let rest = &name[name.find("luid_")? + 5..];
    let (luid, rest) = rest.split_once("_phys_")?;
    let (phys, rest) = rest.split_once("_eng_")?;
    let (eng, kind) = rest.split_once("_engtype_").unwrap_or((rest, ""));
    Some((luid.to_ascii_lowercase(), phys.parse().ok()?, eng.parse().ok()?, kind.to_string()))
}

/// Utilization % per engine, summed over the processes using it.
fn engine_utilization(handle: PDH_HCOUNTER) -> HashMap<EngineKey, f64> {
    let mut out: HashMap<EngineKey, f64> = HashMap::new();
    for (name, v) in instances(handle).unwrap_or_default() {
        if let Some(k) = engine_key(&name).filter(|k| !skipped_gpu(&k.0)) {
            *out.entry(k).or_default() += v;
        }
    }
    out
}

/// "NVIDIA GeForce RTX 3080 — 3D (engine 0)".
fn engine_label(k: &EngineKey, gpus: &HashMap<String, String>) -> String {
    let gpu = gpus.get(&k.0).map_or("GPU", String::as_str);
    let kind = k.3.replace('_', " ");
    if kind.is_empty() { format!("{gpu} — engine {}", k.2) } else { format!("{gpu} — {kind} (engine {})", k.2) }
}

/// Software adapters - Microsoft Basic Render Driver (WARP), which Windows keeps around and which does CPU work, not
/// GPU work - by LUID: GPU meters and `agg = "engine"` counters leave them out, unless `software_gpus = true`.
static SKIPPED_GPUS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Whether an adapter's LUID (as GPU Engine instance names spell it, in any case) is one to leave out.
fn skipped_gpu(luid: &str) -> bool {
    let luid = luid.to_ascii_lowercase();
    SKIPPED_GPUS.lock().unwrap().iter().any(|s| luid.contains(s))
}

/// Adapter names by LUID, spelled like GPU Engine instance names (`0x00000000_0x0000d1e4`); and, unless
/// `software_gpus`, the software adapters noted to be left out.
fn gpu_names(software_gpus: bool) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut software = Vec::new();
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return out };
    for i in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else { break };
        let Ok(d) = (unsafe { adapter.GetDesc1() }) else { continue };
        let name = String::from_utf16_lossy(&d.Description[..d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len())]);
        let luid = format!("0x{:08x}_0x{:08x}", d.AdapterLuid.HighPart as u32, d.AdapterLuid.LowPart);
        if !software_gpus && d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            software.push(luid.clone());
        }
        out.insert(luid, name);
    }
    *SKIPPED_GPUS.lock().unwrap() = software;
    out
}

/// Every instance of a counter as (name, value), valid data only.
fn instances(handle: PDH_HCOUNTER) -> Result<Vec<(String, f64)>, u32> {
    let fmt = PDH_FMT(PDH_FMT_DOUBLE.0 | PDH_FMT_NOCAP100);
    let (mut size, mut count) = (0u32, 0u32);
    let status = unsafe { PdhGetFormattedCounterArrayW(handle, fmt, &mut size, &mut count, None) };
    if status != PDH_MORE_DATA {
        return if status == 0 { Ok(Vec::new()) } else { Err(status) };
    }
    // The buffer holds the item array followed by the instance name strings.
    let mut buf = vec![0u64; (size as usize).div_ceil(8)];
    let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
    let status = unsafe { PdhGetFormattedCounterArrayW(handle, fmt, &mut size, &mut count, Some(items)) };
    if status != 0 {
        return Err(status);
    }
    let items = unsafe { std::slice::from_raw_parts(items, count as usize) };
    Ok(items
        .iter()
        .filter(|it| matches!(it.FmtValue.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA))
        .map(|it| (unsafe { it.szName.to_string() }.unwrap_or_default(), unsafe { it.FmtValue.Anonymous.doubleValue }))
        .collect())
}

fn read(counter: &Counter) -> Result<f64, String> {
    let all = instances(counter.handle).map_err(|s| format!("{}: PDH status {s:#010x}", counter.path))?;
    let wildcard = counter.path.contains('*');
    let values: Vec<(String, f64)> = all.into_iter().filter(|(n, _)| !wildcard || n != "_Total").collect();
    if values.is_empty() {
        return Ok(0.0);
    }
    let nums = || values.iter().map(|(_, v)| *v);
    Ok(match counter.agg {
        Agg::Sum => nums().sum(),
        Agg::Max => nums().fold(f64::MIN, f64::max),
        Agg::Min => nums().fold(f64::MAX, f64::min),
        Agg::Avg => nums().sum::<f64>() / values.len() as f64,
        Agg::Engine => {
            // pid_1234_luid_0x0_0xC1E2_phys_0_eng_3_engtype_Copy -> luid_0x0_0xC1E2_phys_0_eng_3
            let mut engines: HashMap<&str, f64> = HashMap::new();
            for (name, v) in &values {
                let key = name.find("luid_").map_or(name.as_str(), |at| &name[at..]);
                let key = key.find("_engtype").map_or(key, |end| &key[..end]);
                if skipped_gpu(key) {
                    continue;
                }
                *engines.entry(key).or_default() += v;
            }
            engines.values().copied().fold(0.0, f64::max)
        }
    })
}

/// How long a meter that lost its data keeps showing "none" before it leaves.
const ABSENT_GRACE: Duration = Duration::from_secs(5);

/// The widest text a meter's value is likely to need, so its cell doesn't jitter as values change.
fn widest(spec: &MeterSpec) -> String {
    match spec.unit.as_str() {
        "%" => "100%".into(),
        "bytes" => "999.9 MB".into(),
        "bytes/s" => "999.9 KB/s".into(),
        "bits/s" => "999.9 Mb/s".into(),
        "" if spec.style == Style::Cores => "100%".into(),
        "" => "999.9".into(),
        suffix => format!("999.9{suffix}"),
    }
}

/// Idle cycles per CPU, in topology order (`QueryIdleProcessorCycleTimeEx`, per processor group).
fn idle_cycles(cpus: &[(u16, u32)]) -> Vec<u64> {
    use windows::Win32::System::WindowsProgramming::QueryIdleProcessorCycleTimeEx;
    let mut by_group: HashMap<u16, Vec<u64>> = HashMap::new();
    for &(g, _) in cpus {
        by_group.entry(g).or_insert_with(|| {
            let mut size = 0u32;
            let _ = unsafe { QueryIdleProcessorCycleTimeEx(g, &mut size, None) };
            let mut v = vec![0u64; size as usize / 8];
            let _ = unsafe { QueryIdleProcessorCycleTimeEx(g, &mut size, Some(v.as_mut_ptr())) };
            v
        });
    }
    cpus.iter().map(|(g, n)| by_group[g].get(*n as usize).copied().unwrap_or(0)).collect()
}

/// The cycle counter idle cycles are measured in.
fn tsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { core::arch::x86_64::_rdtsc() }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

/// Per-CPU (user, kernel) busy fractions since the last call, in topology order: how busy from
/// idle cycles, split by the (tick-granular, so smoothed) user/privileged time counters.
fn core_values(cpus: &[(u16, u32)], user: PDH_HCOUNTER, kernel: PDH_HCOUNTER, c: &mut Cycles) -> Vec<(f64, f64)> {
    let by_cpu = |h| -> HashMap<(u16, u32), f64> {
        instances(h)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(name, v)| {
                let (g, n) = name.split_once(',')?;
                Some(((g.parse().ok()?, n.parse().ok()?), v / 100.0))
            })
            .collect()
    };
    let (u, k) = (by_cpu(user), by_cpu(kernel));
    let (idle, now) = (idle_cycles(cpus), tsc());
    let elapsed = now.wrapping_sub(c.tsc) as f64;
    let out = cpus
        .iter()
        .enumerate()
        .map(|(i, cpu)| {
            let (ut, kt) = (u.get(cpu).copied().unwrap_or(0.0), k.get(cpu).copied().unwrap_or(0.0));
            if ut + kt > 0.0 {
                c.user_share[i] = 0.7 * c.user_share[i] + 0.3 * ut / (ut + kt);
            }
            // No cycle counter (not x86-64): fall back to the ticks.
            if elapsed <= 0.0 || now == 0 {
                return (ut, kt);
            }
            let busy = (1.0 - idle[i].wrapping_sub(c.idle[i]) as f64 / elapsed).clamp(0.0, 1.0);
            (busy * c.user_share[i], busy * (1.0 - c.user_share[i]))
        })
        .collect();
    (c.idle, c.tsc) = (idle, now);
    out
}

fn human(v: f64, base: f64, units: &[&str], decimals: usize) -> String {
    let mut v = v;
    let mut i = 0;
    while v.abs() >= base && i + 1 < units.len() {
        v /= base;
        i += 1;
    }
    let d = if i == 0 { 0 } else { decimals };
    format!("{v:.d$} {}", units[i])
}

fn format_value(v: f64, spec: &MeterSpec) -> String {
    if !v.is_finite() {
        return "–".into();
    }
    match spec.unit.as_str() {
        "%" => format!("{v:.*}%", spec.decimals.unwrap_or(0)),
        "bytes" => human(v, 1024.0, &["B", "KB", "MB", "GB", "TB"], spec.decimals.unwrap_or(1)),
        "bytes/s" => human(v, 1024.0, &["B/s", "KB/s", "MB/s", "GB/s"], spec.decimals.unwrap_or(1)),
        "bits/s" => human(v * 8.0, 1000.0, &["b/s", "Kb/s", "Mb/s", "Gb/s"], spec.decimals.unwrap_or(1)),
        "" => format!("{v:.*}", spec.decimals.unwrap_or(1)),
        suffix => format!("{v:.*}{suffix}", spec.decimals.unwrap_or(1)),
    }
}

/// `pms meters <file>`: just the sampler, in the foreground.
pub fn run(path: &str) -> Result<(), String> {
    let config = config::load(path.as_ref())?;
    let mut sampler = Sampler::new(&config)?;
    let mut defined = false;
    println!("sampling {}; Ctrl+C to stop", sampler.describe());
    loop {
        let next = sampler.next_due().unwrap_or_else(|| Instant::now() + Duration::from_secs(1));
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
        let lines = sampler.sample_due(Instant::now());

        // The TAP may have been re-injected (meters gone) or be absent; keep trying.
        if !defined {
            match sampler.define(&config.meter_height).iter().try_for_each(|l| crate::send(l).map(drop)) {
                Ok(()) => defined = true,
                Err(e) => {
                    eprintln!("waiting for TAP: {e}");
                    continue;
                }
            }
        }
        for line in lines {
            match crate::send(&line) {
                Ok(resp) if resp.starts_with("error") => {
                    eprintln!("TAP: {resp}; re-defining meters");
                    defined = false;
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("lost TAP: {e}");
                    defined = false;
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(unit: &str) -> MeterSpec {
        toml::from_str(&format!("label = \"x\"\nexpr = \"1\"\nunit = \"{unit}\"")).unwrap()
    }

    #[test]
    fn formatting() {
        assert_eq!(format_value(53.4, &spec("%")), "53%");
        assert_eq!(format_value(512.0, &spec("bytes/s")), "512 B/s");
        assert_eq!(format_value(3.5 * 1024.0 * 1024.0, &spec("bytes/s")), "3.5 MB/s");
        assert_eq!(format_value(125_000.0, &spec("bits/s")), "1.0 Mb/s");
        assert_eq!(format_value(42.25, &spec("°C")), "42.2°C");
    }

    #[test]
    fn topology_covers_every_cpu() {
        let t = topology().unwrap();
        let mut si = SYSTEM_INFO::default();
        unsafe { GetSystemInfo(&mut si) };
        assert_eq!(t.cpus.len(), si.dwNumberOfProcessors as usize, "layout {}", t.layout);
        let threads: usize = t.layout.split(['|', '/', ',']).map(|c| c.trim_start_matches('e').parse::<usize>().unwrap()).sum();
        assert_eq!(threads, t.cpus.len());
    }

    /// Every machine has Microsoft Basic Render Driver among its adapters: it's a software one, left out of GPU meters
    /// unless software_gpus, and its engines are recognised by their LUID in any case.
    #[test]
    fn software_gpus_are_skipped() {
        let names = gpu_names(false);
        let warp = names.iter().find(|(_, n)| n.contains("Basic Render")).map(|(l, _)| l.clone()).expect("WARP is listed");
        assert!(skipped_gpu(&warp.to_ascii_uppercase()));
        gpu_names(true);
        assert!(!skipped_gpu(&warp));
    }
}
