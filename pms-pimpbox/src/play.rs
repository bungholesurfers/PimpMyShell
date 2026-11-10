//! The console's window: its display, on your desktop, opened by a click on the taskbar's screen. It shows whatever the
//! console's putting out, the console coming on, a game, or nothing much, and it's where a game is played from. It stays
//! when a game ends. Input is simply this window's: there are no hooks, and raw mouse input only comes while it has the
//! focus. So nothing can outlive it: Alt+Tab, a click elsewhere or closing it lets go of the pointer, because losing the
//! focus does. Closing it only puts it away (and pauses a game in progress), for the taskbar to keep.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use room::doom::doomkeys::*;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLACK_BRUSH, ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, COLORONCOLOR, DIB_RGB_COLORS, HALFTONE, GetDC, GetStockObject, HBRUSH, ReleaseDC, SRCCOPY, SetStretchBltMode, StretchDIBits,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Input::{GetRawInputData, HRAWINPUT, MOUSE_MOVE_RELATIVE, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT, RIM_TYPEMOUSE, RegisterRawInputDevices, RAWINPUTDEVICE_FLAGS};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_core::w;

use crate::link::Said;

const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0);

thread_local! {
    static WINDOW: Cell<isize> = const { Cell::new(0) };
    static GRABBED: Cell<bool> = const { Cell::new(false) };
    /// There's a game showing, which the mouse plays: only then is the pointer the window's to keep.
    static PLAYING: Cell<bool> = const { Cell::new(false) };
    /// Let go of with the keys for that (Ctrl+Alt+G): it isn't taken again until there's a click in the window.
    static LET_GO: Cell<bool> = const { Cell::new(false) };
    /// What's been done at the window since it was last asked: for whatever's playing.
    static SAID: RefCell<Vec<Said>> = const { RefCell::new(Vec::new()) };
    /// Keys held now: let go of when the focus goes, or they'd stay held.
    static HELD: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// Movement since it was last said, and the buttons down, and the buttons last said.
    static MOUSE: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    static BUTTONS: Cell<i32> = const { Cell::new(0) };
    static SENT_BUTTONS: Cell<i32> = const { Cell::new(0) };
}

fn hwnd() -> Option<HWND> {
    let h = WINDOW.get();
    (h != 0).then_some(HWND(h as *mut c_void))
}

fn say(said: Said) {
    SAID.with_borrow_mut(|s| s.push(said));
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
    if !PLAYING.get() || LET_GO.get() || GRABBED.replace(true) {
        return;
    }
    title(h, true);
    clip(h);
    unsafe { ShowCursor(false) };
}

/// Keeps the pointer in the window. Windows lets go of a clip whenever the foreground changes, so this is said again each time
/// the window's messages are taken.
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

/// What the window's called: with how to get the keys and the mouse back, while it has them.
fn title(h: HWND, held: bool) {
    let _ = unsafe { SetWindowTextW(h, if held { w!("PIMPBOX - Ctrl+Alt+G lets go of the keys and the mouse") } else { w!("PIMPBOX") }) };
}

fn release() {
    if !GRABBED.replace(false) {
        return;
    }
    if let Some(h) = hwnd() {
        title(h, false);
    }
    unsafe {
        let _ = ClipCursor(None);
        ShowCursor(true);
    }
    for key in HELD.with_borrow_mut(std::mem::take) {
        say(Said::Key { down: false, key });
    }
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

unsafe extern "system" fn proc(h: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
            let repeat = down && (lparam.0 >> 30) & 1 != 0;
            // Ctrl+Alt+G: the keys and the mouse are yours again, until you click in the window.
            let held = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
            if down && wparam.0 == 0x47 && held(VK_CONTROL) && held(VK_MENU) {
                if GRABBED.get() {
                    LET_GO.set(true);
                    release();
                }
                return LRESULT(0);
            }
            if !repeat && let Some(key) = doom_key(wparam.0 as u16) {
                say(Said::Key { down, key });
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
                LET_GO.set(false);
            } else {
                grab(h);
            }
            LRESULT(0)
        }
        // A click in the picture, and it has them again.
        WM_LBUTTONDOWN => {
            LET_GO.set(false);
            grab(h);
            LRESULT(0)
        }
        WM_CLOSE => {
            close();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(h, message, wparam, lparam) },
    }
}

/// How big to show a picture `across` by `down` where there's `free` room: a whole number of times its size, as many as
/// fit; or, where it doesn't fit even once, as much of that as does, the same shape.
fn fit((across, down): (i32, i32), free: (i32, i32)) -> (i32, i32) {
    let times = (free.0 / across).min(free.1 / down);
    if times >= 1 {
        return (across * times, down * times);
    }
    let wide = free.0.min(free.1 * across / down).max(1);
    (wide, (wide * down / across).max(1))
}

/// Where the window goes, and how big it is, frame and all, for the console's picture as it is now: each of the picture's
/// pixels a whole number of the screen's across and down, as many as there's room for on the screen, in its own pixels, so
/// that it's as big a window on a dense screen as on any other; in the middle of the screen. (A picture too big for the
/// screen even one to one is made to fit, and isn't as sharp.)
fn place(now: Option<HWND>) -> (i32, i32, i32, i32) {
    unsafe {
        let (across, down) = crate::video::mode();
        let mut room = RECT::default();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, Some((&raw mut room).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).is_err() {
            room = RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) };
        }
        // A window that's there already stays on the screen it's on, about the place it's at: the room is that screen's.
        let mut middle = None;
        if let Some(window) = now {
            let mut screen = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let mut at = RECT::default();
            if GetMonitorInfoW(MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST), &mut screen).as_bool() && GetWindowRect(window, &mut at).is_ok() {
                room = screen.rcWork;
                middle = Some(((at.left + at.right) / 2, (at.top + at.bottom) / 2));
            }
        }
        let mut edges = RECT::default();
        let _ = AdjustWindowRect(&mut edges, STYLE, false);
        let (free_w, free_h) = (room.right - room.left - (edges.right - edges.left), room.bottom - room.top - (edges.bottom - edges.top));
        let (width, height) = fit((across as i32, down as i32), (free_w, free_h));
        let mut frame = RECT { left: 0, top: 0, right: width, bottom: height };
        let _ = AdjustWindowRect(&mut frame, STYLE, false);
        let (w, h) = (frame.right - frame.left, frame.bottom - frame.top);
        // In the middle of the screen, the first time; after that, its middle where its middle was, as far as it fits there.
        let (cx, cy) = middle.unwrap_or(((room.left + room.right) / 2, (room.top + room.bottom) / 2));
        let within = |at: i32, size: i32, from: i32, to: i32| at.min(to - size).max(from);
        (within(cx - w / 2, w, room.left, room.right), within(cy - h / 2, h, room.top, room.bottom), w, h)
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
            lpszClassName: w!("PimpMyShellPimpbox"),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            ..Default::default()
        };
        RegisterClassW(&class);
        let (x, y, w, h) = place(None);
        let window = match CreateWindowExW(WINDOW_EX_STYLE(0), w!("PimpMyShellPimpbox"), w!("PIMPBOX"), STYLE, x, y, w, h, None, None, Some(instance), None) {
            Ok(window) => window,
            Err(e) => {
                eprintln!("pms-pimpbox: no window: {e}");
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

/// Says whether there's a game showing. With one, the window keeps the pointer while it has the focus, and hides it; with
/// none (the console coming on, or nothing playing) the pointer's left alone, free to go to the window's close button or
/// anywhere else.
pub fn playing(now: bool) {
    if PLAYING.replace(now) == now {
        return;
    }
    match hwnd() {
        Some(h) if now && is_open() && unsafe { GetForegroundWindow() } == h => grab(h),
        _ if !now => release(),
        _ => {}
    }
}

/// Opens the window.
pub fn open() {
    let Some(h) = hwnd().or_else(create) else { return };
    unsafe {
        // Not ShowWindow: the first time, that goes by how this process was started, which is hidden.
        let _ = SetWindowPos(h, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(h);
    }
    say(Said::Opened);
}

fn close() {
    release();
    if let Some(h) = hwnd() {
        let _ = unsafe { ShowWindow(h, SW_HIDE) };
    }
    say(Said::Closed);
}

/// The console's picture has changed size: so does the window, if there is one, where it is or as near as it fits.
pub fn fit_mode() {
    let Some(h) = hwnd() else { return };
    let (x, y, w, h_) = place(Some(h));
    unsafe {
        let _ = SetWindowPos(h, None, x, y, w, h_, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    if GRABBED.get() {
        clip(h);
    }
}

/// Takes the window's messages, and gives back what was done at it since the last time: keys, the mouse, its opening and
/// closing.
pub fn messages() -> Vec<Said> {
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
    let (across, _) = MOUSE.replace((0, 0));
    let buttons = BUTTONS.get();
    if GRABBED.get() && (across != 0 || buttons != SENT_BUTTONS.get()) {
        SENT_BUTTONS.set(buttons);
        say(Said::Mouse { buttons, across });
    }
    SAID.with_borrow_mut(std::mem::take)
}

/// Shows a frame, `across` by `down` pixels: all of the window.
pub fn draw(source: &[u32], (across, down): (usize, usize)) {
    let Some(h) = hwnd() else { return };
    unsafe {
        let mut area = RECT::default();
        let _ = GetClientRect(h, &mut area);
        let dc = GetDC(Some(h));
        // Whole pixels, when it's shown a whole number of times its size; smoothed, when it had to be made to fit.
        let whole = area.right % across as i32 == 0 && area.bottom % down as i32 == 0;
        SetStretchBltMode(dc, if whole { COLORONCOLOR } else { HALFTONE });
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: across as i32,
                biHeight: -(down as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        StretchDIBits(dc, 0, 0, area.right, area.bottom, 0, 0, across as i32, down as i32, Some(source.as_ptr() as *const c_void), &info, DIB_RGB_COLORS, SRCCOPY);
        ReleaseDC(Some(h), dc);
    }
}
