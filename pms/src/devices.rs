//! Disks and network interfaces for the `disks` and `nics` meters: which ones there are, what to call them, which
//! one matters most (the disk Windows is on; the interface the route to the internet goes through), and which have
//! been busy lately.
//!
//! Counters come per instance: `\PhysicalDisk(0 C: D:)\...` (disk number, then its drive letters) and
//! `\Network Interface(Intel[R] Wi-Fi 6E AX211 160MHz)\...` (the adapter's description, with `(` `)` `#` `/` `\`
//! spelled `[` `]` `_` `_` `_`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, GENERIC_ACCESS_RIGHTS};
use windows::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceLuidToNameW, FreeMibTable, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, GetBestInterfaceEx, GetIfTable2, IP_ADAPTER_ADDRESSES_LH,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, IN_ADDR, IN_ADDR_0, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDiskFreeSpaceExW, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    IOCTL_STORAGE_QUERY_PROPERTY, PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY, StorageDeviceProperty,
};
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows_core::HSTRING;

/// Below this (bytes/s, all of a device's bars together) a device counts as idle: background trickles (a log line,
/// a broadcast) don't keep it on the strip.
pub const IDLE_BELOW: f64 = 2048.0;

// ---------------------------------------------------------------------------------------------
// Activity: which devices show, and in what order

/// Which devices of a meter show, in what order. The pinned one (if it's there) always shows, first. The rest show
/// while they've been busy within the hold time, ordered by how busy they've been lately; the order is only worked
/// out again when a device comes or goes, so bars don't swap places under the pointer.
#[derive(Default)]
pub struct Tracker {
    /// Per device: when it was last busy (None: not since we've known it), and its smoothed rate.
    seen: HashMap<String, (Option<Instant>, f64)>,
    order: Vec<String>,
}

impl Tracker {
    /// Take this sample's readings (device, its bars' rates added up), and return the devices to show, in order.
    /// `hold`: how long an idle device stays (zero: for good).
    pub fn update(&mut self, now: Instant, readings: &[(String, f64)], pinned: Option<&str>, hold: Duration) -> &[String] {
        self.seen.retain(|k, _| readings.iter().any(|(r, _)| r == k));
        for (k, rate) in readings {
            let e = self.seen.entry(k.clone()).or_insert((None, *rate));
            e.1 = 0.7 * e.1 + 0.3 * rate;
            if *rate >= IDLE_BELOW {
                e.0 = Some(now);
            }
        }
        let pinned = pinned.filter(|p| self.seen.contains_key(*p));
        let recent = |t: &Option<Instant>| t.is_some_and(|t| hold.is_zero() || now.duration_since(t) < hold);
        let mut shown: Vec<&String> = self.seen.iter().filter(|(k, (t, _))| Some(k.as_str()) == pinned || recent(t)).map(|(k, _)| k).collect();
        if shown.is_empty() {
            // Nothing pinned, nothing busy: the one busy last (or else the first there is) keeps the meter up.
            let last = self.seen.iter().filter(|(_, (t, _))| t.is_some()).max_by_key(|(_, (t, _))| *t).map(|(k, _)| k);
            shown.extend(last.or_else(|| readings.first().map(|(k, _)| k)).and_then(|k| self.seen.get_key_value(k)).map(|(k, _)| k));
        }
        let same = shown.len() == self.order.len() && shown.iter().all(|k| self.order.contains(k));
        if !same {
            let mut order: Vec<String> = shown.into_iter().cloned().collect();
            let rate = |k: &String| self.seen.get(k).map_or(0.0, |s| s.1);
            order.sort_by(|a, b| {
                let first = |k: &String| Some(k.as_str()) != pinned;
                (first(a), -rate(a), a).partial_cmp(&(first(b), -rate(b), b)).unwrap_or(std::cmp::Ordering::Equal)
            });
            self.order = order;
        }
        &self.order
    }
}

// ---------------------------------------------------------------------------------------------
// Disks

pub struct Disk {
    /// The counter instance (`0 C: D:`).
    pub instance: String,
    pub label: String,
    /// "Samsung SSD 990 PRO 2TB · NVMe".
    pub model: String,
    /// Its drive letters, with free and total space.
    pub volumes: Vec<(String, u64, u64)>,
    /// What Windows calls the disk: `\\.\PhysicalDrive0`.
    pub id: String,
    /// Windows is on it.
    pub system: bool,
}

/// The disks behind `\PhysicalDisk(*)` instances (`_Total` aside).
pub fn disks(instances: &[String]) -> Vec<Disk> {
    let windows = windows_drive();
    instances
        .iter()
        .filter(|i| *i != "_Total")
        .map(|instance| {
            let mut words = instance.split_whitespace();
            let number = words.next().and_then(|n| n.parse::<u32>().ok());
            let letters: Vec<String> = words.map(str::to_string).collect();
            // `\\.\PhysicalDrive0`, is `disk0` on the taskbar: the number diskpart and Disk Management call it by.
            let label = number.map_or_else(|| letters.first().cloned().unwrap_or("disk".into()), |n| format!("disk{n}"));
            Disk {
                instance: instance.clone(),
                label,
                id: number.map_or_else(|| instance.clone(), |n| format!(r"\\.\PhysicalDrive{n}")),
                model: number.and_then(disk_model).unwrap_or_default(),
                volumes: letters.iter().map(|l| (l.clone(), free_space(l).0, free_space(l).1)).collect(),
                system: windows.as_ref().is_some_and(|w| letters.iter().any(|l| l.eq_ignore_ascii_case(w))),
            }
        })
        .collect()
}

/// "C:", where Windows is.
fn windows_drive() -> Option<String> {
    let mut buf = [0u16; 260];
    let n = unsafe { GetWindowsDirectoryW(Some(&mut buf)) } as usize;
    let dir = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
    dir.get(..2).filter(|d| d.ends_with(':')).map(str::to_string)
}

/// (free, total) bytes on drive "C:".
fn free_space(letter: &str) -> (u64, u64) {
    let (mut free, mut total) = (0u64, 0u64);
    let root = HSTRING::from(format!("{letter}\\"));
    let _ = unsafe { GetDiskFreeSpaceExW(&root, Some(&mut free), Some(&mut total), None) };
    (free, total)
}

/// What the disk says it is, and how it's attached: "Samsung SSD 990 PRO 2TB · NVMe". Asking needs no rights to
/// the disk itself (no read access is requested).
fn disk_model(number: u32) -> Option<String> {
    let path = HSTRING::from(format!(r"\\.\PhysicalDrive{number}"));
    let h = unsafe {
        CreateFileW(&path, GENERIC_ACCESS_RIGHTS(0).0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None)
    }
    .ok()?;
    let query = STORAGE_PROPERTY_QUERY { PropertyId: StorageDeviceProperty, QueryType: PropertyStandardQuery, AdditionalParameters: [0] };
    let mut buf = vec![0u8; 1024];
    let mut got = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const _),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(buf.as_mut_ptr() as *mut _),
            buf.len() as u32,
            Some(&mut got),
            None,
        )
    };
    let _ = unsafe { CloseHandle(h) };
    ok.ok()?;
    let d = unsafe { &*(buf.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };
    let text = |off: u32| -> String {
        let off = off as usize;
        if off == 0 || off >= buf.len() {
            return String::new();
        }
        let end = buf[off..].iter().position(|&b| b == 0).map_or(buf.len(), |e| off + e);
        String::from_utf8_lossy(&buf[off..end]).trim().to_string()
    };
    let (vendor, product) = (text(d.VendorIdOffset), text(d.ProductIdOffset));
    let name = if vendor.is_empty() || product.starts_with(&vendor) { product } else { format!("{vendor} {product}") };
    let bus = match d.BusType.0 {
        1 => "SCSI",
        3 => "ATA",
        7 => "USB",
        8 => "RAID",
        9 => "iSCSI",
        10 => "SAS",
        11 => "SATA",
        12 => "SD card",
        13 => "MMC",
        14 | 15 => "virtual",
        16 => "Storage Spaces",
        17 => "NVMe",
        _ => "",
    };
    Some(match (name.is_empty(), bus.is_empty()) {
        (true, _) => bus.to_string(),
        (false, true) => name,
        (false, false) => format!("{name} · {bus}"),
    })
}

// ---------------------------------------------------------------------------------------------
// Network interfaces

pub struct Nic {
    /// The counter instance (the description, spelled the counters' way).
    pub instance: String,
    pub label: String,
    /// What Windows calls the interface inside: its LUID's name, `ethernet_32773`.
    pub id: String,
    /// The interfaces of its type with a lower LUID index and no counters to read, so no bar: why it's `eth5` on a PC
    /// with one adapter.
    pub earlier: Vec<String>,
    /// "Wi-Fi", as Windows' settings call it, and what it is ("Intel(R) Wi-Fi 6E AX211 160MHz").
    pub name: String,
    pub description: String,
    /// Link speed, bits/s (receive).
    pub speed: u64,
    pub addresses: Vec<String>,
    /// The route to the internet goes through it.
    pub internet: bool,
}

/// Whether counter instance `instance` is adapter `description` spelled some other way than `instance_name` expects: the same
/// letters and digits (other punctuation swapped, or dropped), or cut short, as long names can be.
fn loosely(description: &str, instance: &str) -> bool {
    let plain = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    let (d, i) = (plain(description), plain(instance));
    !i.is_empty() && (d == i || (i.chars().count() >= 24 && d.starts_with(&i)))
}

/// An interface with counters and no adapter to go with it is labelled by its kind (ETH), with no name, no interfaces before it,
/// and no route to the internet: said once in the log, so a report shows why. Not by name: an adapter's says what it is (a VPN, a
/// virtual machine's), and reports get posted. What's said is how near the nearest adapter came: how long each is, how much they
/// share from the start, and where they part, by the characters there if those are punctuation.
fn unmatched(instance: &str, adapters: &[Adapter]) {
    static SAID: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    if let Ok(mut said) = SAID.lock()
        && !said.iter().any(|s| s == instance)
    {
        said.push(instance.to_string());
        crate::log(format!("network interface counters match none of the {} adapter(s): {}", adapters.len(), nearest(instance, adapters)));
    }
}

/// How near the adapter nearest to `instance` came, without either's name.
fn nearest(instance: &str, adapters: &[Adapter]) -> String {
    let lower = |s: &str| s.chars().flat_map(char::to_lowercase).collect::<Vec<char>>();
    let i = lower(instance);
    let Some(d) = adapters.iter().map(|a| lower(&instance_name(&a.description))).max_by_key(|d| d.iter().zip(&i).take_while(|(a, b)| a == b).count()) else {
        return "there are none".into();
    };
    let same = d.iter().zip(&i).take_while(|(a, b)| a == b).count();
    let say = |c: Option<&char>| match c {
        None => "its end".to_string(),
        Some(c) if c.is_alphanumeric() => "a letter or digit".to_string(),
        Some(c) => format!("{c:?}"),
    };
    format!("the nearest has {} characters to its {}, the same for the first {same}, then {} where the counters have {}", d.len(), i.len(), say(d.get(same)), say(i.get(same)))
}

/// How `\Network Interface(*)` spells an adapter's description.
fn instance_name(description: &str) -> String {
    description.chars().map(|c| match c {
        '(' => '[',
        ')' => ']',
        '#' | '/' | '\\' => '_',
        c => c,
    }).collect()
}

/// The interfaces behind `\Network Interface(*)` instances, with what Windows knows of each.
pub fn nics(instances: &[String]) -> Vec<Nic> {
    let internet = internet_interface();
    let adapters = adapters();
    let all = interfaces();
    let mut taken: HashMap<String, usize> = HashMap::new();
    instances
        .iter()
        .filter(|i| *i != "_Total")
        .map(|instance| {
            let a = adapters
                .iter()
                .find(|a| instance_name(&a.description).eq_ignore_ascii_case(instance))
                .or_else(|| adapters.iter().find(|a| loosely(&a.description, instance)));
            if a.is_none() {
                unmatched(instance, &adapters);
            }
            let kind = a.map_or_else(|| short_kind(0, instance, instance), |a| short_kind(a.if_type, &a.name, &a.description));
            // eth5, wlan0; ETH, ETH2, ... should an adapter have no LUID name to go by.
            let n = taken.entry(kind.to_string()).or_default();
            *n += 1;
            let fallback = if *n == 1 { kind.to_string() } else { format!("{kind}{n}") };
            let id = a.map_or_else(String::new, |a| a.luid_name.clone());
            Nic {
                instance: instance.clone(),
                label: short_interface(&id).unwrap_or(fallback),
                earlier: earlier_interfaces(&id, &all, instances),
                id,
                name: a.map_or_else(|| instance.clone(), |a| a.name.clone()),
                description: a.map_or_else(String::new, |a| a.description.clone()),
                speed: a.map_or(0, |a| a.speed),
                addresses: a.map_or_else(Vec::new, |a| a.addresses.clone()),
                internet: a.is_some_and(|a| internet.is_some_and(|i| a.indexes.contains(&i))),
            }
        })
        .collect()
}

/// Every interface Windows has numbered, hidden ones included (`GetAdaptersAddresses` leaves many out), as its LUID's
/// name and description. Filter drivers, stacked on real adapters, aren't interfaces in their own right.
fn interfaces() -> Vec<(String, String)> {
    const FILTER_INTERFACE: u8 = 1 << 1;
    let text = |wide: &[u16]| String::from_utf16_lossy(&wide[..wide.iter().position(|&c| c == 0).unwrap_or(wide.len())]);
    let mut table = std::ptr::null_mut();
    if unsafe { GetIfTable2(&mut table) }.is_err() || table.is_null() {
        return Vec::new();
    }
    let rows = unsafe { std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize) };
    let mut out = Vec::new();
    for row in rows.iter().filter(|r| r.InterfaceAndOperStatusFlags._bitfield & FILTER_INTERFACE == 0) {
        let mut name = [0u16; 257];
        if unsafe { ConvertInterfaceLuidToNameW(&row.InterfaceLuid, &mut name) }.is_ok() {
            out.push((text(&name), text(&row.Description)));
        }
    }
    unsafe { FreeMibTable(table as *const _) };
    out
}

/// The interfaces of `id`'s type with a lower LUID index, in order, as the lines of a table for a fixed-width font: its
/// headings, then `eth1  ethernet_32769  Microsoft Kernel Debug Network Adapter`. One with a `\Network Interface(*)`
/// instance isn't among them: it gets a bar of its own when it's active.
fn earlier_interfaces(id: &str, all: &[(String, String)], instances: &[String]) -> Vec<String> {
    let parts = |name: &str| name.rsplit_once('_').and_then(|(kind, n)| Some((kind.to_string(), n.parse::<u32>().ok()?)));
    let Some((kind, number)) = parts(id) else { return Vec::new() };
    let counted = |description: &str| instances.iter().any(|i| i.eq_ignore_ascii_case(&instance_name(description)));
    let mut before: Vec<(u32, &str, &str)> = all
        .iter()
        .filter(|(_, description)| !counted(description))
        .filter_map(|(name, description)| parts(name).filter(|(k, n)| *k == kind && *n < number).map(|(_, n)| (n, name.as_str(), description.as_str())))
        .collect();
    before.sort();
    before.dedup_by_key(|b| b.0);
    if before.is_empty() {
        return Vec::new();
    }
    let head = ["PMS shorthand", "LUID name", "Description"].map(String::from);
    let rows: Vec<[String; 3]> = before.iter().map(|(_, name, description)| [short_interface(name).unwrap_or_else(|| name.to_string()), name.to_string(), description.to_string()]).collect();
    let width = |column: usize| std::iter::once(&head).chain(&rows).map(|r| r[column].chars().count()).max().unwrap_or(0);
    let (first, second) = (width(0), width(1));
    std::iter::once(&head).chain(&rows).map(|[a, b, c]| format!("{a:<first$}  {b:<second$}  {c}")).collect()
}

/// An interface's LUID name, short enough for the taskbar: `ethernet_32773` is `eth5`, `wireless_32768` is `wlan0`.
/// (Windows numbers the interfaces it assigns from 32768, so that's the zero.)
fn short_interface(luid_name: &str) -> Option<String> {
    let (kind, number) = luid_name.rsplit_once('_')?;
    let number: u32 = number.parse().ok()?;
    let kind = match kind {
        "ethernet" => "eth",
        "wireless" => "wlan",
        "tunnel" => "tun",
        "loopback" => "lo",
        other => other,
    };
    Some(format!("{kind}{}", number.checked_sub(32768).unwrap_or(number)))
}

/// A short label by the kind of interface: WiFi, ETH, vETH (Hyper-V's virtual switches), CELL, VPN, BT, NET.
fn short_kind(if_type: u32, name: &str, description: &str) -> &'static str {
    let says = |w: &str| name.to_ascii_lowercase().contains(w) || description.to_ascii_lowercase().contains(w);
    match if_type {
        _ if says("bluetooth") => "BT",
        71 => "WiFi",
        243 | 244 => "CELL",
        23 | 53 | 131 => "VPN",
        6 if says("vethernet") || says("hyper-v") => "vETH",
        6 => "ETH",
        _ if says("wi-fi") || says("wireless") || says("wlan") => "WiFi",
        _ if says("vpn") || says("wireguard") || says("tap-") || says("tunnel") => "VPN",
        _ if says("ethernet") => "ETH",
        _ => "NET",
    }
}

#[derive(Default)]
struct Adapter {
    /// IPv4 and IPv6 interface indexes.
    indexes: [u32; 2],
    /// Its interface LUID's name: `ethernet_32773`, `wireless_32768`.
    luid_name: String,
    if_type: u32,
    name: String,
    description: String,
    speed: u64,
    addresses: Vec<String>,
}

fn adapters() -> Vec<Adapter> {
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_INCLUDE_GATEWAYS;
    let mut size = 32 * 1024u32;
    let mut buf: Vec<u64>;
    loop {
        buf = vec![0u64; size as usize / 8 + 1];
        let r = unsafe { GetAdaptersAddresses(AF_UNSPEC.0 as u32, flags, None, Some(buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH), &mut size) };
        match r {
            0 => break,
            111 => continue, // ERROR_BUFFER_OVERFLOW: size now says how much
            _ => return Vec::new(),
        }
    }
    let mut out = Vec::new();
    let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    while let Some(a) = unsafe { p.as_ref() } {
        let mut addresses = Vec::new();
        let mut u = a.FirstUnicastAddress;
        while let Some(addr) = unsafe { u.as_ref() } {
            if let Some(s) = unsafe { sockaddr_text(addr.Address.lpSockaddr) } {
                addresses.push(s);
            }
            u = addr.Next;
        }
        let mut luid_name = [0u16; 257];
        let named = unsafe { ConvertInterfaceLuidToNameW(&a.Luid, &mut luid_name) }.is_ok();
        let luid_name = if named { String::from_utf16_lossy(&luid_name[..luid_name.iter().position(|&c| c == 0).unwrap_or(0)]) } else { String::new() };
        out.push(Adapter {
            indexes: [unsafe { a.Anonymous1.Anonymous.IfIndex }, a.Ipv6IfIndex],
            luid_name,
            if_type: a.IfType,
            name: unsafe { a.FriendlyName.to_string() }.unwrap_or_default(),
            description: unsafe { a.Description.to_string() }.unwrap_or_default(),
            speed: a.ReceiveLinkSpeed,
            addresses,
        });
        p = a.Next;
    }
    out
}

/// An address as text, IPv4 as usual; IPv6 unless it's link-local (fe80::/10), compressed loosely.
unsafe fn sockaddr_text(sa: *const SOCKADDR) -> Option<String> {
    let sa = unsafe { sa.as_ref()? };
    if sa.sa_family == AF_INET {
        let a = unsafe { &*(sa as *const SOCKADDR as *const SOCKADDR_IN) };
        let b = unsafe { a.sin_addr.S_un.S_un_b };
        return Some(format!("{}.{}.{}.{}", b.s_b1, b.s_b2, b.s_b3, b.s_b4));
    }
    if sa.sa_family == AF_INET6 {
        let a = unsafe { &*(sa as *const SOCKADDR as *const SOCKADDR_IN6) };
        let w = unsafe { a.sin6_addr.u.Word };
        let w: Vec<u16> = w.iter().map(|x| u16::from_be(*x)).collect();
        // Link-local (fe80::/10) says nothing useful.
        if w[0] & 0xffc0 == 0xfe80 {
            return None;
        }
        return Some(ipv6_text(&w));
    }
    None
}

/// The usual short form: the longest run of two or more zero groups becomes `::`.
fn ipv6_text(w: &[u16]) -> String {
    let (mut best, mut run) = ((0, 0), (0, 0));
    for (i, x) in w.iter().enumerate() {
        if *x == 0 {
            run = if run.1 > 0 && run.0 + run.1 == i { (run.0, run.1 + 1) } else { (i, 1) };
            if run.1 > best.1 {
                best = run;
            }
        }
    }
    let hex = |s: &[u16]| s.iter().map(|x| format!("{x:x}")).collect::<Vec<_>>().join(":");
    if best.1 < 2 {
        return hex(w);
    }
    format!("{}::{}", hex(&w[..best.0]), hex(&w[best.0 + best.1..]))
}

/// The interface Windows would send a packet to the internet through (no packet is sent: this asks the routing
/// table): toward 1.1.1.1, else (IPv6 only) 2606:4700:4700::1111.
fn internet_interface() -> Option<u32> {
    let mut index = 0u32;
    let v4 = SOCKADDR_IN { sin_family: AF_INET, sin_port: 0, sin_addr: IN_ADDR { S_un: IN_ADDR_0 { S_addr: u32::from_ne_bytes([1, 1, 1, 1]) } }, sin_zero: [0; 8] };
    if unsafe { GetBestInterfaceEx(&v4 as *const _ as *const SOCKADDR, &mut index) } == 0 {
        return Some(index);
    }
    let mut v6 = SOCKADDR_IN6 { sin6_family: AF_INET6, ..Default::default() };
    let words: [u16; 8] = [0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111];
    v6.sin6_addr.u.Word = words.map(u16::to_be);
    (unsafe { GetBestInterfaceEx(&v6 as *const _ as *const SOCKADDR, &mut index) } == 0).then_some(index)
}

/// "1.2 Gb/s".
pub fn link_speed(bits: u64) -> String {
    match bits {
        0 => String::new(),
        b if b >= 1_000_000_000 => format!("{:.1} Gb/s", b as f64 / 1e9).replace(".0 ", " "),
        b if b >= 1_000_000 => format!("{} Mb/s", b / 1_000_000),
        b => format!("{} kb/s", b / 1000),
    }
}

/// "120 GB free of 476 GB".
pub fn space(free: u64, total: u64) -> String {
    let gb = |b: u64| {
        let g = b as f64 / (1u64 << 30) as f64;
        if g >= 1024.0 { format!("{:.1} TB", g / 1024.0) } else { format!("{g:.0} GB") }
    };
    format!("{} free of {}", gb(free), gb(total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(list: &[(&str, f64)]) -> Vec<(String, f64)> {
        list.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn pinned_first_busy_ones_join_and_fall_off() {
        let mut t = Tracker::default();
        let t0 = Instant::now();
        let hold = Duration::from_secs(15);
        // Idle all round: just the pinned one.
        assert_eq!(t.update(t0, &r(&[("0 C:", 0.0), ("1 D:", 0.0), ("2 E:", 0.0)]), Some("0 C:"), hold), ["0 C:"]);
        // E gets busy: it joins, after C.
        let t1 = t0 + Duration::from_secs(1);
        assert_eq!(t.update(t1, &r(&[("0 C:", 0.0), ("1 D:", 0.0), ("2 E:", 1e6)]), Some("0 C:"), hold), ["0 C:", "2 E:"]);
        // D busier than E: D joins, and goes before E (C stays first, idle or not).
        let t2 = t1 + Duration::from_secs(1);
        for _ in 0..5 {
            t.update(t2, &r(&[("0 C:", 0.0), ("1 D:", 5e7), ("2 E:", 1e6)]), Some("0 C:"), hold);
        }
        assert_eq!(t.update(t2, &r(&[("0 C:", 0.0), ("1 D:", 5e7), ("2 E:", 1e6)]), Some("0 C:"), hold), ["0 C:", "1 D:", "2 E:"]);
        // Nothing changes membership while the rates shift: the order holds still.
        let t3 = t2 + Duration::from_secs(1);
        for _ in 0..10 {
            t.update(t3, &r(&[("0 C:", 0.0), ("1 D:", 1e4), ("2 E:", 9e7)]), Some("0 C:"), hold);
        }
        assert_eq!(t.update(t3, &r(&[("0 C:", 0.0), ("1 D:", 1e4), ("2 E:", 9e7)]), Some("0 C:"), hold), ["0 C:", "1 D:", "2 E:"]);
        // D goes idle; past the hold it falls off, and E stays.
        let t4 = t3 + Duration::from_secs(16);
        assert_eq!(t.update(t4, &r(&[("0 C:", 0.0), ("1 D:", 0.0), ("2 E:", 9e7)]), Some("0 C:"), hold), ["0 C:", "2 E:"]);
        // A trickle doesn't count as busy.
        let t5 = t4 + Duration::from_secs(16);
        assert_eq!(t.update(t5, &r(&[("0 C:", 0.0), ("1 D:", 100.0), ("2 E:", 0.0)]), Some("0 C:"), hold), ["0 C:"]);
    }

    #[test]
    fn without_a_pinned_one_the_last_busy_stays() {
        let mut t = Tracker::default();
        let t0 = Instant::now();
        let hold = Duration::from_secs(5);
        // Nothing busy yet, nothing pinned: the first one, so the meter has something.
        assert_eq!(t.update(t0, &r(&[("ETH", 0.0), ("WiFi", 0.0)]), None, hold), ["ETH"]);
        t.update(t0, &r(&[("ETH", 0.0), ("WiFi", 1e5)]), None, hold);
        let later = t0 + Duration::from_secs(60);
        assert_eq!(t.update(later, &r(&[("ETH", 0.0), ("WiFi", 0.0)]), None, hold), ["WiFi"]);
        // A device that's gone is forgotten.
        assert_eq!(t.update(later, &r(&[("ETH", 0.0)]), None, hold), ["ETH"]);
    }

    #[test]
    fn counter_spelling() {
        assert_eq!(instance_name("Intel(R) Ethernet Connection #2 I219-V"), "Intel[R] Ethernet Connection _2 I219-V");
        // Spelled some other way, or cut short, it's still found; a different adapter, or a short stub, isn't.
        assert!(loosely("Realtek PCIe 2.5GbE Family Controller", "Realtek PCIe 2_5GbE Family Controller"));
        assert!(loosely("Intel(R) Ethernet Controller I225-V with a very long name", "Intel[R] Ethernet Controller I225-V with"));
        assert!(!loosely("Intel(R) Ethernet Connection #2 I219-V", "Intel[R] Ethernet Connection I219-V"));
        assert!(!loosely("Intel(R) Ethernet Connection I219-V", "Intel"));
        // Said without names: where the spellings part, by punctuation only.
        let mullvad = Adapter { description: "Mullvad (WireGuard)".into(), ..Adapter::default() };
        let said = nearest("Mullvad <WireGuard>", &[mullvad]);
        assert!(!said.to_lowercase().contains("mullvad") && !said.to_lowercase().contains("wireguard"), "{said}");
        assert!(said.contains("first 8") && said.contains("'['") && said.contains("'<'"), "{said}");
        assert_eq!(short_kind(71, "Wi-Fi", "Intel(R) Wi-Fi 6E AX211"), "WiFi");
        let all = [("ethernet_32773", "Mine"), ("ethernet_32770", "WAN Miniport (IP)"), ("ethernet_32769", "Kernel Debug"), ("wireless_32768", "Wi-Fi"), ("ethernet_32774", "Later")];
        let all: Vec<(String, String)> = all.iter().map(|(n, d)| (n.to_string(), d.to_string())).collect();
        assert_eq!(
            earlier_interfaces("ethernet_32773", &all, &[]),
            [
                "PMS shorthand  LUID name       Description",
                "eth1           ethernet_32769  Kernel Debug",
                "eth2           ethernet_32770  WAN Miniport (IP)"
            ]
        );
        assert!(earlier_interfaces("wireless_32768", &all, &[]).is_empty());
        // One with counters gets a bar of its own, spelled the counters' way.
        assert_eq!(earlier_interfaces("ethernet_32773", &all, &["WAN Miniport [IP]".to_string()])[1..], ["eth1           ethernet_32769  Kernel Debug"]);
        assert_eq!(short_interface("ethernet_32773").as_deref(), Some("eth5"));
        assert_eq!(short_interface("wireless_32768").as_deref(), Some("wlan0"));
        assert_eq!(short_interface("tunnel_32512").as_deref(), Some("tun32512"));
        assert_eq!(short_interface(""), None);
        assert_eq!(short_kind(6, "Bluetooth Network Connection", "Bluetooth Device (Personal Area Network)"), "BT");
        assert_eq!(short_kind(6, "vEthernet (Default Switch)", "Hyper-V Virtual Ethernet Adapter"), "vETH");
        assert_eq!(link_speed(1_200_000_000), "1.2 Gb/s");
        assert_eq!(link_speed(10_000_000_000), "10 Gb/s");
        assert_eq!(ipv6_text(&[0xfd69, 0x6969, 0x6969, 0x6969, 0, 0, 0, 0xc2d]), "fd69:6969:6969:6969::c2d");
        assert_eq!(ipv6_text(&[0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111]), "2606:4700:4700::1111");
        assert_eq!(ipv6_text(&[1, 0, 2, 0, 3, 0, 4, 5]), "1:0:2:0:3:0:4:5");
    }
}

#[cfg(test)]
mod here {
    /// What this machine has: `cargo test -p pms --lib devices::here -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print() {
        for d in super::disks(&["0 C:".into(), "_Total".into()]) {
            println!("disk {} label={} model={:?} system={} volumes={:?}", d.instance, d.label, d.model, d.system, d.volumes);
        }
        for n in super::nics(&["Red Hat VirtIO Ethernet Adapter".into()]) {
            println!("nic {} label={} name={:?} desc={:?} speed={} addrs={:?} internet={}", n.instance, n.label, n.name, n.description, n.speed, n.addresses, n.internet);
        }
    }
}
