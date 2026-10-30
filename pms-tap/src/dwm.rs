//! DWM window attributes for the taskbar windows. DWM only honours these from the process
//! that owns the window, which is why this lives in the TAP rather than the CLI.

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmSetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, GetWindowThreadProcessId};
use windows_core::BOOL;

use crate::commands::R;

/// Top-level taskbar windows (primary and secondary monitors) owned by this explorer.
pub fn taskbar_windows() -> Vec<HWND> {
    unsafe extern "system" fn collect(hwnd: HWND, out: LPARAM) -> BOOL {
        let mut buf = [0u16; 64];
        let n = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
        let class = String::from_utf16_lossy(&buf[..n]);
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == std::process::id() && (class == "Shell_TrayWnd" || class == "Shell_SecondaryTrayWnd") {
            unsafe { (*(out.0 as *mut Vec<HWND>)).push(hwnd) };
        }
        true.into()
    }
    let mut out: Vec<HWND> = Vec::new();
    let _ = unsafe { EnumWindows(Some(collect), LPARAM(&mut out as *mut _ as isize)) };
    out
}

pub fn set_host_backdrop(on: bool) -> R<String> {
    let value = BOOL::from(on);
    let mut out = String::new();
    for hwnd in taskbar_windows() {
        let r = unsafe {
            DwmSetWindowAttribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, &value as *const BOOL as _, size_of::<BOOL>() as u32)
        };
        out += &format!("{:?}: USE_HOSTBACKDROPBRUSH={on} -> {r:?}\n", hwnd.0);
    }
    Ok(out)
}

pub fn command(args: &[String]) -> R<String> {
    match (args.first().map(String::as_str), args.get(1).map(String::as_str)) {
        (Some("hostbackdrop"), Some(v @ ("0" | "1" | "on" | "off"))) => set_host_backdrop(matches!(v, "1" | "on")),
        _ => Err("usage: dwm hostbackdrop on|off".into()),
    }
}
