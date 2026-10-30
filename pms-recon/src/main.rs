//! Step 0: walk the HWND tree of every taskbar and print what's hosting what.
//!
//! The interesting bit is the class name of the island window(s):
//!   - `Windows.UI.Composition.DesktopWindowContentBridge` -> system XAML (Windows.UI.Xaml, UWP)
//!   - `Microsoft.UI.Content.DesktopChildSiteBridge`       -> WinUI 3 (Microsoft.UI.Xaml, WinAppSDK)

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::BOOL;

fn class_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n as usize])
}

fn text_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n as usize])
}

fn dump(hwnd: HWND, depth: usize) {
    let mut rc = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rc).ok() };
    let mut pid = 0u32;
    let tid = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
    let exstyle = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();

    println!(
        "{:indent$}{:?} [{}] \"{}\"  ({},{})-({},{}) {}x{}  pid={} tid={} style={:08x} ex={:08x}{}",
        "",
        hwnd.0,
        class_of(hwnd),
        text_of(hwnd),
        rc.left,
        rc.top,
        rc.right,
        rc.bottom,
        rc.right - rc.left,
        rc.bottom - rc.top,
        pid,
        tid,
        style,
        exstyle,
        if visible { "" } else { "  (hidden)" },
        indent = depth * 2
    );

    let mut child = unsafe { GetWindow(hwnd, GW_CHILD) }.unwrap_or_default();
    while !child.is_invalid() {
        dump(child, depth + 1);
        child = unsafe { GetWindow(child, GW_HWNDNEXT) }.unwrap_or_default();
    }
}

unsafe extern "system" fn enum_top(hwnd: HWND, _: LPARAM) -> BOOL {
    let class = class_of(hwnd);
    if class == "Shell_TrayWnd" || class == "Shell_SecondaryTrayWnd" {
        dump(hwnd, 0);
        println!();
    }
    true.into()
}

fn main() {
    unsafe { EnumWindows(Some(enum_top), LPARAM(0)).ok() };
}
