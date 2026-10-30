//! NVIDIA GPUs' performance states (P0 the busiest, up to P12 or so at rest), from NVML - the management library
//! NVIDIA's driver installs (nvml.dll) - loaded if it's there; nothing without it. Other makers' GPUs have no such
//! library worth the name, so they show no state.

use std::collections::HashMap;
use std::ffi::{c_char, c_void};

use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_core::{HSTRING, PCSTR};

type Device = *mut c_void;
type Init = unsafe extern "C" fn() -> i32;
type Count = unsafe extern "C" fn(*mut u32) -> i32;
type ByIndex = unsafe extern "C" fn(u32, *mut Device) -> i32;
type Name = unsafe extern "C" fn(Device, *mut c_char, u32) -> i32;
type PState = unsafe extern "C" fn(Device, *mut u32) -> i32;

pub struct Nvml {
    _lib: HMODULE,
    pstate: PState,
    /// The GPUs, by name.
    devices: Vec<(String, Device)>,
}

// NVML's handles are good on any thread.
unsafe impl Send for Nvml {}

impl Nvml {
    /// NVML, if NVIDIA's driver is installed and it starts.
    pub fn open() -> Option<Nvml> {
        let lib = ["nvml.dll", r"C:\Program Files\NVIDIA Corporation\NVSMI\nvml.dll"]
            .iter()
            .find_map(|p| unsafe { LoadLibraryW(&HSTRING::from(*p)) }.ok())?;
        let get = |name: &str| unsafe { GetProcAddress(lib, PCSTR(format!("{name}\0").as_ptr())) };
        unsafe {
            let init: Init = std::mem::transmute(get("nvmlInit_v2")?);
            let count: Count = std::mem::transmute(get("nvmlDeviceGetCount_v2")?);
            let by_index: ByIndex = std::mem::transmute(get("nvmlDeviceGetHandleByIndex_v2")?);
            let name: Name = std::mem::transmute(get("nvmlDeviceGetName")?);
            let pstate: PState = std::mem::transmute(get("nvmlDeviceGetPerformanceState")?);
            if init() != 0 {
                return None;
            }
            let mut n = 0u32;
            if count(&mut n) != 0 {
                return None;
            }
            let mut devices = Vec::new();
            for i in 0..n {
                let mut d: Device = std::ptr::null_mut();
                if by_index(i, &mut d) != 0 {
                    continue;
                }
                let mut buf = [0 as c_char; 96];
                if name(d, buf.as_mut_ptr(), buf.len() as u32) != 0 {
                    continue;
                }
                let s = std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
                devices.push((s, d));
            }
            Some(Nvml { _lib: lib, pstate, devices })
        }
    }

    /// Each GPU's performance state ("P8"), by adapter name (as DXGI names it: NVML's names are the same). Two
    /// GPUs of one name come in NVML's order.
    pub fn states(&self) -> HashMap<String, Vec<String>> {
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        for (name, d) in &self.devices {
            let mut p = 32u32;
            let state = if unsafe { (self.pstate)(*d, &mut p) } == 0 && p < 32 { format!("P{p}") } else { String::new() };
            out.entry(name.clone()).or_default().push(state);
        }
        out
    }
}
