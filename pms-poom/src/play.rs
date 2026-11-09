//! The window you play in, opened by a click on the taskbar's screen. Input is simply this window's: there are no hooks, and
//! raw mouse input only comes while it has the focus. So nothing can outlive it: Alt+Tab, a click elsewhere or closing it
//! lets go of the pointer, because losing the focus does. Closing it pauses a game in progress, for the taskbar to keep.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;

use room::doom::d_event::{D_PostEvent, event_t};
use room::doom::doomgeneric::{DOOMGENERIC_RESX, DOOMGENERIC_RESY};
use room::doom::doomkeys::*;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLACK_BRUSH, ClientToScreen, COLORONCOLOR, DIB_RGB_COLORS, GetDC, GetStockObject, HBRUSH, ReleaseDC, SRCCOPY, SetStretchBltMode, StretchDIBits,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Input::{GetRawInputData, HRAWINPUT, MOUSE_MOVE_RELATIVE, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT, RIM_TYPEMOUSE, RegisterRawInputDevices, RAWINPUTDEVICE_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::w;

const EV_MOUSE: i32 = 2;
/// How wide the picture is, in DIPs: it's 4:3, as Doom was on a screen of its day.
const WIDTH: i32 = 960;
const HEIGHT: i32 = 720;

thread_local! {
    static WINDOW: Cell<isize> = const { Cell::new(0) };
    static GRABBED: Cell<bool> = const { Cell::new(false) };
    static KEYS: RefCell<VecDeque<(bool, u8)>> = const { RefCell::new(VecDeque::new()) };
    /// Keys held now: let go of when the focus goes, or they'd stay held.
    static HELD: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// Movement since the last tic, and the buttons down.
    static MOUSE: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    static SENT_BUTTONS: Cell<i32> = const { Cell::new(0) };
    /// A game was paused by closing the window: opening it again carries on.
    static WE_PAUSED: Cell<bool> = const { Cell::new(false) };
}

fn hwnd() -> Option<HWND> {
    let h = WINDOW.get();
    (h != 0).then_some(HWND(h as *mut c_void))
}

fn in_game() -> bool {
    unsafe { room::doom::g_game::usergame != 0 }
}

fn paused() -> bool {
    unsafe { room::doom::g_game::paused != 0 }
}

fn tap(key: u8) {
    KEYS.with_borrow_mut(|k| {
        k.push_back((true, key));
        k.push_back((false, key));
    });
}

/// The next key, for the engine.
pub fn next_key() -> Option<(bool, u8)> {
    KEYS.with_borrow_mut(VecDeque::pop_front)
}

pub fn is_open() -> bool {
    hwnd().is_some_and(|h| unsafe { IsWindowVisible(h).as_bool() })
}

fn doom_key(vk: u16) -> Option<u8> {
    let vk = VIRTUAL_KEY(vk);
    Some(match vk {
        VK_RETURN => KEY_ENTER,
        VK_ESCAPE => KEY_ESCAPE,
        VK_LEFT => KEY_LEFTARROW,
        VK_RIGHT => KEY_RIGHTARROW,
        VK_UP => KEY_UPARROW,
        VK_DOWN => KEY_DOWNARROW,
        VK_CONTROL => KEY_FIRE,
        VK_SPACE => KEY_USE,
        VK_SHIFT => KEY_RSHIFT,
        VK_MENU => KEY_LALT,
        VK_OEM_COMMA => KEY_STRAFE_L,
        VK_OEM_PERIOD => KEY_STRAFE_R,
        VK_OEM_PLUS => KEY_EQUALS,
        VK_OEM_MINUS => KEY_MINUS,
        VK_BACK => KEY_BACKSPACE,
        VK_TAB => KEY_TAB,
        VK_PAUSE => KEY_PAUSE,
        VK_F1 => KEY_F1,
        VK_F2 => KEY_F2,
        VK_F3 => KEY_F3,
        VK_F4 => KEY_F4,
        VK_F5 => KEY_F5,
        VK_F6 => KEY_F6,
        VK_F7 => KEY_F7,
        VK_F8 => KEY_F8,
        VK_F9 => KEY_F9,
        VK_F10 => KEY_F10,
        VK_F11 => KEY_F11,
        VK_F12 => KEY_F12,
        VIRTUAL_KEY(k) if (0x30..=0x39).contains(&k) => k as u8,
        VIRTUAL_KEY(k) if (0x41..=0x5A).contains(&k) => k as u8 + 32,
        _ => return None,
    })
}

fn grab(h: HWND) {
    if GRABBED.replace(true) {
        return;
    }
    clip(h);
    unsafe { ShowCursor(false) };
}

/// Keeps the pointer in the window. Windows lets go of a clip whenever the foreground changes, so this is said again each tic.
fn clip(h: HWND) {
    unsafe {
        let mut area = RECT::default();
        let _ = GetClientRect(h, &mut area);
        let mut corner = POINT { x: area.left, y: area.top };
        let _ = ClientToScreen(h, &mut corner);
        let on_screen = RECT { left: corner.x, top: corner.y, right: corner.x + area.right, bottom: corner.y + area.bottom };
        let _ = ClipCursor(Some(&on_screen));
    }
}

fn release() {
    if !GRABBED.replace(false) {
        return;
    }
    unsafe {
        let _ = ClipCursor(None);
        ShowCursor(true);
    }
    let held: Vec<u8> = HELD.with_borrow_mut(std::mem::take);
    KEYS.with_borrow_mut(|k| k.extend(held.into_iter().map(|key| (false, key))));
    MOUSE.set((0, 0));
}

fn raw_mouse(lparam: LPARAM) {
    let mut data = RAWINPUT::default();
    let mut size = size_of::<RAWINPUT>() as u32;
    let read = unsafe { GetRawInputData(HRAWINPUT(lparam.0 as *mut c_void), RID_INPUT, Some(&mut data as *mut _ as *mut c_void), &mut size, size_of::<RAWINPUTHEADER>() as u32) };
    if read == u32::MAX || data.header.dwType != RIM_TYPEMOUSE.0 || !GRABBED.get() {
        return;
    }
    let mouse = unsafe { data.data.mouse };
    let flags = unsafe { mouse.Anonymous.Anonymous.usButtonFlags };
    let mut buttons = BUTTONS.get();
    for (down, up, bit) in [(0x0001u16, 0x0002u16, 1), (0x0004, 0x0008, 2), (0x0010, 0x0020, 4)] {
        if flags & down != 0 {
            buttons |= bit;
        }
        if flags & up != 0 {
            buttons &= !bit;
        }
    }
    BUTTONS.set(buttons);
    if mouse.usFlags == MOUSE_MOVE_RELATIVE {
        let (x, y) = MOUSE.get();
        MOUSE.set((x + mouse.lLastX, y + mouse.lLastY));
    }
}

thread_local! {
    static BUTTONS: Cell<i32> = const { Cell::new(0) };
}

unsafe extern "system" fn proc(h: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
            let repeat = down && (lparam.0 >> 30) & 1 != 0;
            if !repeat && let Some(key) = doom_key(wparam.0 as u16) {
                KEYS.with_borrow_mut(|k| k.push_back((down, key)));
                HELD.with_borrow_mut(|held| {
                    held.retain(|k| *k != key);
                    if down {
                        held.push(key);
                    }
                });
            }
            // Alt+F4 still closes it.
            if message == WM_SYSKEYDOWN || message == WM_SYSKEYUP {
                return unsafe { DefWindowProcW(h, message, wparam, lparam) };
            }
            LRESULT(0)
        }
        // Alt alone would open the window's menu, and stop the game.
        WM_SYSCOMMAND if wparam.0 & 0xFFF0 == SC_KEYMENU as usize => LRESULT(0),
        WM_INPUT => {
            raw_mouse(lparam);
            unsafe { DefWindowProcW(h, message, wparam, lparam) }
        }
        WM_ACTIVATE => {
            if wparam.0 & 0xFFFF == 0 {
                release();
            } else {
                grab(h);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            close();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(h, message, wparam, lparam) },
    }
}

fn create() -> Option<HWND> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let module = GetModuleHandleW(None).ok()?;
        let instance = HINSTANCE(module.0);
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: w!("PimpMyShellPoom"),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            ..Default::default()
        };
        RegisterClassW(&class);
        let scale = GetDpiForSystem() as i32;
        let (width, height) = (WIDTH * scale / 96, HEIGHT * scale / 96);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut frame = RECT { left: 0, top: 0, right: width, bottom: height };
        let _ = AdjustWindowRect(&mut frame, style, false);
        let (w, h) = (frame.right - frame.left, frame.bottom - frame.top);
        let (x, y) = ((GetSystemMetrics(SM_CXSCREEN) - w) / 2, ((GetSystemMetrics(SM_CYSCREEN) - h) / 2).max(0));
        let window = match CreateWindowExW(WINDOW_EX_STYLE(0), w!("PimpMyShellPoom"), w!("Space Pimpin'"), style, x, y, w, h, None, None, Some(instance), None) {
            Ok(window) => window,
            Err(e) => {
                eprintln!("pms-poom: no window: {e}");
                return None;
            }
        };
        // Only while this window has the focus: no RIDEV_INPUTSINK.
        let device = RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RAWINPUTDEVICE_FLAGS(0), hwndTarget: window };
        let _ = RegisterRawInputDevices(&[device], size_of::<RAWINPUTDEVICE>() as u32);
        WINDOW.set(window.0 as isize);
        Some(window)
    }
}

/// Opens the window, carrying on with a game that closing it paused.
pub fn open() {
    let Some(h) = hwnd().or_else(create) else { return };
    unsafe {
        // Not ShowWindow: the first time, that goes by how this process was started, which is hidden.
        let _ = SetWindowPos(h, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(h);
    }
    if WE_PAUSED.replace(false) && paused() {
        tap(KEY_PAUSE);
    }
}

fn close() {
    release();
    if let Some(h) = hwnd() {
        let _ = unsafe { ShowWindow(h, SW_HIDE) };
    }
    if in_game() && !paused() {
        tap(KEY_PAUSE);
        WE_PAUSED.set(true);
    }
}

/// Takes the window's messages, and hands the engine what the mouse did. Once a tic.
pub fn pump() {
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    if GRABBED.get()
        && let Some(h) = hwnd()
    {
        clip(h);
    }
    let (x, _) = MOUSE.replace((0, 0));
    let buttons = BUTTONS.get();
    if GRABBED.get() && (x != 0 || buttons != SENT_BUTTONS.get()) {
        SENT_BUTTONS.set(buttons);
        // Turning only: pushing the mouse forward walking you into things isn't what anyone expects now.
        let event = event_t { type_: EV_MOUSE, data1: buttons, data2: x * 2, data3: 0, data4: 0 };
        D_PostEvent(&event);
    }
}

/// Shows the engine's frame, at 4:3 whatever its own proportions.
pub fn draw(source: &[u32]) {
    let Some(h) = hwnd() else { return };
    unsafe {
        let mut area = RECT::default();
        let _ = GetClientRect(h, &mut area);
        let dc = GetDC(Some(h));
        SetStretchBltMode(dc, COLORONCOLOR);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: DOOMGENERIC_RESX as i32,
                biHeight: -(DOOMGENERIC_RESY as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        StretchDIBits(dc, 0, 0, area.right, area.bottom, 0, 0, DOOMGENERIC_RESX as i32, DOOMGENERIC_RESY as i32, Some(source.as_ptr() as *const c_void), &info, DIB_RGB_COLORS, SRCCOPY);
        ReleaseDC(Some(h), dc);
    }
}
