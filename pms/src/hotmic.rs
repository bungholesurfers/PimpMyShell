//! Who is using the microphone, from the same place the tray's mic indicator gets it: the
//! capability consent store. Every app that ever used the mic has a key under
//!
//!   HKCU\...\CapabilityAccessManager\ConsentStore\microphone\<PackageFamilyName>
//!   HKCU\...\CapabilityAccessManager\ConsentStore\microphone\NonPackaged\C:#path#to#app.exe
//!
//! with `LastUsedTimeStart`/`LastUsedTimeStop` FILETIMEs; while the app holds the mic open,
//! `LastUsedTimeStop` is 0. A registry change notification wakes the daemon on every change.

use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE};
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::CreateEventW;
use windows_core::{PCWSTR, PWSTR, w};

const MIC_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone");

pub struct MicWatch {
    key: HKEY,
    event: HANDLE,
}

impl Drop for MicWatch {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.key);
            let _ = CloseHandle(self.event);
        }
    }
}

impl MicWatch {
    pub fn new() -> Result<MicWatch, String> {
        let event = unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.map_err(|e| e.to_string())?;
        let mut key = HKEY::default();
        let err = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, MIC_KEY, None, KEY_READ | KEY_NOTIFY, &mut key) };
        if err != ERROR_SUCCESS {
            unsafe { CloseHandle(event) }.ok();
            return Err(format!("open microphone consent store: {}", err.to_hresult().message()));
        }
        Ok(MicWatch { key, event })
    }

    /// Signalled when anything under the consent store changes (after a `poll`).
    pub fn event(&self) -> HANDLE {
        self.event
    }

    /// Consent-store names of the apps holding the mic right now. Re-arms the change
    /// notification first, so a change during the scan still signals `event`.
    pub fn poll(&self) -> Vec<String> {
        let filter = REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET;
        let err = unsafe { RegNotifyChangeKeyValue(self.key, true, filter, Some(self.event), true) };
        if err != ERROR_SUCCESS {
            crate::log(format!("mic watch: RegNotifyChangeKeyValue: {}", err.to_hresult().message()));
        }
        let mut out = Vec::new();
        scan(self.key, &mut out, 0);
        out
    }
}

fn qword(key: HKEY, value: PCWSTR) -> Option<u64> {
    let mut v = 0u64;
    let mut size = size_of::<u64>() as u32;
    let err = unsafe { RegGetValueW(key, PCWSTR::null(), value, RRF_RT_REG_QWORD, None, Some(&mut v as *mut u64 as _), Some(&mut size)) };
    (err == ERROR_SUCCESS).then_some(v)
}

fn scan(key: HKEY, out: &mut Vec<String>, depth: u32) {
    let mut buf = [0u16; 512];
    for i in 0.. {
        let mut len = buf.len() as u32;
        let err = unsafe { RegEnumKeyExW(key, i, Some(PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) };
        if err != ERROR_SUCCESS {
            break;
        }
        let name = String::from_utf16_lossy(&buf[..len as usize]);
        let mut sub = HKEY::default();
        if unsafe { RegOpenKeyExW(key, PCWSTR(buf.as_ptr()), None, KEY_READ, &mut sub) } != ERROR_SUCCESS {
            continue;
        }
        match (qword(sub, w!("LastUsedTimeStart")), qword(sub, w!("LastUsedTimeStop"))) {
            (Some(start), Some(0)) if start != 0 => out.push(name),
            (Some(_), _) => {}
            // No timestamps: a container like NonPackaged.
            (None, _) if depth == 0 => scan(sub, out, depth + 1),
            (None, _) => {}
        }
        unsafe {
            let _ = RegCloseKey(sub);
        }
    }
}

/// `C:#Program Files#obs-studio#bin#64bit#obs64.exe` -> `obs64.exe`,
/// `Microsoft.WindowsSoundRecorder_8wekyb3d8bbwe` -> `Microsoft.WindowsSoundRecorder`.
pub fn friendly(name: &str) -> &str {
    if name.contains('#') { name.rsplit('#').next().unwrap_or(name) } else { name.split('_').next().unwrap_or(name) }
}

/// What to call the app in the "microphone in use by" label: the exe's FileDescription
/// ("OBS Studio") or the package's display name, falling back to the exe or package name.
pub fn display_name(key: &str) -> String {
    let fallback = friendly(key).trim_end_matches(".exe").to_string();
    let found = if key.contains('#') { file_description(&key.replace('#', "\\")) } else { package_name(key) };
    found.filter(|n| !n.trim().is_empty()).unwrap_or(fallback)
}

fn file_description(path: &str) -> Option<String> {
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    let path = windows_core::HSTRING::from(path);
    let size = unsafe { GetFileVersionInfoSizeW(&path, None) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    unsafe { GetFileVersionInfoW(&path, None, size, data.as_mut_ptr() as _) }.ok()?;
    let query = |sub: &str| -> Option<(*const u16, u32)> {
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        unsafe { VerQueryValueW(data.as_ptr() as _, &windows_core::HSTRING::from(sub), &mut ptr, &mut len) }.as_bool().then_some((ptr as *const u16, len))
    };
    // The first language/codepage pair names the string table to read.
    let (tr, len) = query(r"\VarFileInfo\Translation")?;
    if len < 4 {
        return None;
    }
    let (lang, cp) = unsafe { (*tr, *tr.add(1)) };
    let (s, n) = query(&format!(r"\StringFileInfo\{lang:04x}{cp:04x}\FileDescription"))?;
    let text = unsafe { std::slice::from_raw_parts(s, n as usize) };
    Some(String::from_utf16_lossy(text).trim_end_matches('\0').to_string())
}

fn package_name(family: &str) -> Option<String> {
    let pm = windows::Management::Deployment::PackageManager::new().ok()?;
    let packages = pm.FindPackagesByUserSecurityIdPackageFamilyName(&windows_core::HSTRING::new(), &windows_core::HSTRING::from(family)).ok()?;
    let name = packages.into_iter().next()?.DisplayName().ok()?.to_string();
    // Unresolved resource names come back as "ms-resource:..."; not worth showing.
    (!name.starts_with("ms-resource:")).then_some(name)
}

/// How loud the microphone is right now, and whether Windows has it muted.
///
/// The level is each active capture endpoint's peak meter (the one the Sound settings bars
/// show), taken after the endpoint's volume and mute: a muted mic reads 0, a quieter volume
/// reads quieter. Apps that mute themselves (Discord, Teams) still receive audio, so they read
/// as live; Windows can't see an app-level mute.
pub struct MicLevel {
    meters: Vec<windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation>,
    volume: Option<windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume>,
}

impl MicLevel {
    pub fn open() -> Result<MicLevel, String> {
        use windows::Win32::Media::Audio::*;
        use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
        let e = |err: windows_core::Error| err.to_string();
        let devices: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(e)?;
        let all = unsafe { devices.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) }.map_err(e)?;
        let mut meters = Vec::new();
        for i in 0..unsafe { all.GetCount() }.map_err(e)? {
            if let Ok(meter) = unsafe { all.Item(i).and_then(|d| d.Activate(CLSCTX_ALL, None)) } {
                meters.push(meter);
            }
        }
        let volume = unsafe { devices.GetDefaultAudioEndpoint(eCapture, eConsole).and_then(|d| d.Activate(CLSCTX_ALL, None)) }.ok();
        Ok(MicLevel { meters, volume })
    }

    /// Peak level 0..1 over all capture endpoints.
    pub fn level(&self) -> f32 {
        self.meters.iter().filter_map(|m| unsafe { m.GetPeakValue() }.ok()).fold(0.0, f32::max)
    }

    /// The default capture device is muted.
    pub fn muted(&self) -> bool {
        self.volume.as_ref().and_then(|v| unsafe { v.GetMute() }.ok()).is_some_and(|b| b.as_bool())
    }
}
