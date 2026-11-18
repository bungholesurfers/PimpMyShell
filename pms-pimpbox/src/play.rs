//! The console's window: its display, on your desktop, opened by a click on the taskbar's screen. It shows whatever the
//! console's putting out, the console coming on, a game, or nothing much, and it's where a game is played from. It stays
//! when a game ends. Input is simply this window's: there are no hooks, and raw mouse input only comes while it has the
//! focus. So nothing can outlive it: Alt+Tab, a click elsewhere or closing it lets go of the pointer, because losing the
//! focus does. Closing it only puts it away (and pauses a game in progress), for the taskbar to keep.
//!
//! While it has the keys and the mouse, it says how to get them back, over the picture (capture.rs); and it counts each time
//! it takes them, and whether it has them, on the window itself (TAKEN_PROPERTY, HELD_PROPERTY), so that the remote docked
//! beside it can flash too, and the PIMPBOX card beside it say so.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use poom::link::keys::*;
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
    /// The last picture shown, and its size: drawn again when only what's over it has changed.
    static FRAME: RefCell<(Vec<u32>, (usize, usize))> = const { RefCell::new((Vec::new(), (0, 0))) };
    /// Where the window's drawn first, before it's put up whole (a memory DC and its bitmap, and their size), and when it last was.
    static BACK: Cell<(isize, isize, i32, i32)> = const { Cell::new((0, 0, 0, 0)) };
    static SHOWN: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
    /// How many times it's taken the keys and the mouse.
    static TAKEN: Cell<usize> = const { Cell::new(0) };
    /// How much taller than its picture the window is just now, for the bar under it (capture.rs); and where its top was
    /// before it grew, and how far it's been moved up from there to fit on the screen.
    static EXTRA: Cell<i32> = const { Cell::new(0) };
    static GREW_FROM: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
}

/// The properties on the window that say how many times it's taken the keys and the mouse, and whether it has them now (1) or
/// not (0), for the taskbar to see: the PIMPBOX card beside the window says so too. Named by number (an integer atom), not
/// by a string: this runs in a job with an atom table of its own (no global atoms), where a string's atom would be one the
/// taskbar couldn't look up. The taskbar's remote.rs has the same two.
const TAKEN_PROPERTY: windows_core::PCWSTR = windows_core::PCWSTR(0xB7A1 as *const u16);
const HELD_PROPERTY: windows_core::PCWSTR = windows_core::PCWSTR(0xB7A2 as *const u16);

fn say_held(h: HWND, held: bool) {
    let _ = unsafe { SetPropW(h, HELD_PROPERTY, Some(windows::Win32::Foundation::HANDLE(usize::from(held) as *mut c_void))) };
}

fn hwnd() -> Option<HWND> {
    let h = WINDOW.get();
    (h != 0).then_some(HWND(h as *mut c_void))
}

fn say(said: Said) {
    SAID.with_borrow_mut(|s| s.push(said));
}

/// The window, if it's been made.
pub fn window() -> Option<HWND> {
    hwnd()
}

pub fn is_open() -> bool {
    hwnd().is_some_and(|h| unsafe { IsWindowVisible(h).as_bool() })
}

fn doom_key(vk: u16) -> Option<u8> {
    let vk = VIRTUAL_KEY(vk);
    Some(match vk {
        VK_RETURN => ENTER,
        VK_ESCAPE => ESCAPE,
        VK_LEFT => LEFTARROW,
        VK_RIGHT => RIGHTARROW,
        VK_UP => UPARROW,
        VK_DOWN => DOWNARROW,
        VK_CONTROL => FIRE,
        VK_SPACE => USE,
        VK_SHIFT => RSHIFT,
        VK_MENU => LALT,
        VK_OEM_COMMA => STRAFE_L,
        VK_OEM_PERIOD => STRAFE_R,
        VK_OEM_PLUS => EQUALS,
        VK_OEM_MINUS => MINUS,
        VK_BACK => BACKSPACE,
        VK_TAB => TAB,
        VK_PAUSE => PAUSE,
        VIRTUAL_KEY(k) if (VK_F1.0..=VK_F12.0).contains(&k) => F[usize::from(k - VK_F1.0)],
        VIRTUAL_KEY(k) if (0x30..=0x39).contains(&k) => k as u8,
        VIRTUAL_KEY(k) if (0x41..=0x5A).contains(&k) => k as u8 + 32,
        _ => return None,
    })
}

fn grab(h: HWND) {
    // Only while it's really the window in front: Windows may activate it without letting it come forward (another program had
    // the foreground just then), and a window that isn't in front gets no word when the foreground moves on, so it would
    // never let go.
    if !PLAYING.get() || LET_GO.get() || unsafe { GetForegroundWindow() } != h || GRABBED.replace(true) {
        return;
    }
    clip(h);
    unsafe { ShowCursor(false) };
    // Said: over the picture, and on the window, for the remote beside it.
    crate::capture::taken();
    TAKEN.set(TAKEN.get() + 1);
    let _ = unsafe { SetPropW(h, TAKEN_PROPERTY, Some(windows::Win32::Foundation::HANDLE(TAKEN.get() as *mut c_void))) };
    say_held(h, true);
    // And its title bar flashes twice, as the remote's beside it does.
    let flash = FLASHWINFO { cbSize: size_of::<FLASHWINFO>() as u32, hwnd: h, dwFlags: FLASHW_CAPTION, uCount: 2, dwTimeout: 110 };
    let _ = unsafe { FlashWindowEx(&flash) };
    show();
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

fn release() {
    if !GRABBED.replace(false) {
        return;
    }
    crate::capture::given();
    if let Some(h) = hwnd() {
        say_held(h, false);
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
        // And the bar under the picture, if it's there just now.
        let mut frame = RECT { left: 0, top: 0, right: width, bottom: height + EXTRA.get() };
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
        // Its icon, on its title bar and its taskbar button: the exe's own (resource 1, which build.rs draws), at each size.
        let icon = |side: i32| LoadImageW(Some(instance), windows_core::PCWSTR(std::ptr::without_provenance(1)), IMAGE_ICON, side, side, LR_DEFAULTCOLOR).ok().map(|h| HICON(h.0));
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: w!("PimpMyShellPimpbox"),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            hIcon: icon(GetSystemMetrics(SM_CXICON)).unwrap_or_default(),
            hIconSm: icon(GetSystemMetrics(SM_CXSMICON)).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&class);
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
    if let Some(h) = hwnd() {
        let in_front = unsafe { GetForegroundWindow() } == h;
        if GRABBED.get() && !in_front {
            // Not in front any more, whether or not it was told: the keys and the mouse go back. Never kept by a window that's
            // behind.
            release();
            LET_GO.set(false);
        } else if GRABBED.get() {
            clip(h);
        } else if in_front && is_open() {
            // In front, and playing, and not let go of (grab says): taken now. Activation says so a moment before Windows
            // has it in front, which is too soon.
            grab(h);
        }
    }
    let (across, _) = MOUSE.replace((0, 0));
    let buttons = BUTTONS.get();
    if GRABBED.get() && (across != 0 || buttons != SENT_BUTTONS.get()) {
        SENT_BUTTONS.set(buttons);
        say(Said::Mouse { buttons, across });
    }
    SAID.with_borrow_mut(std::mem::take)
}

/// Shows a frame, `across` by `down` pixels: all of the window, with whatever's over it.
pub fn draw(source: &[u32], size: (usize, usize)) {
    if hwnd().is_none() {
        return;
    }
    FRAME.with_borrow_mut(|(frame, at)| {
        frame.clear();
        frame.extend_from_slice(source);
        *at = size;
    });
    show();
}

/// Between pictures: the bar under the picture is drawn again while it's moving, about sixty times a second, and the window
/// grows or shrinks to have room for as much of it as there is.
pub fn tick() {
    let Some(h) = hwnd() else { return };
    if !is_open() || !crate::capture::moving() && EXTRA.get() == 0 {
        return;
    }
    if SHOWN.get().is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(16)) {
        return;
    }
    let mut area = RECT::default();
    let _ = unsafe { GetClientRect(h, &mut area) };
    let picture = area.bottom - EXTRA.get();
    let want = (crate::capture::tall(picture) as f32 * crate::capture::up()).round() as i32;
    if want != EXTRA.get() {
        grow(h, area.right, picture, want);
    }
    show();
}

/// Makes the window's inside `width` by `picture` + `extra`: taller than its picture by that much, for the bar. It grows
/// downwards, and where that would go off the screen it moves up as far as it must; shrunk back, it's where it was.
fn grow(h: HWND, width: i32, picture: i32, extra: i32) {
    unsafe {
        let mut at = RECT::default();
        if GetWindowRect(h, &mut at).is_err() {
            return;
        }
        if EXTRA.get() == 0 {
            GREW_FROM.set((at.top, 0));
        }
        let mut frame = RECT { left: 0, top: 0, right: width, bottom: picture + extra };
        let _ = AdjustWindowRect(&mut frame, STYLE, false);
        let tall = frame.bottom - frame.top;
        let mut screen = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let room = if GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut screen).as_bool() { screen.rcWork } else { at };
        let (from, _) = GREW_FROM.get();
        let up = (from + tall - room.bottom).clamp(0, (from - room.top).max(0));
        GREW_FROM.set((from, up));
        let _ = SetWindowPos(h, None, at.left, from - up, at.right - at.left, tall, SWP_NOZORDER | SWP_NOACTIVATE);
        EXTRA.set(extra);
        if GRABBED.get() {
            clip(h);
        }
    }
}

/// The memory DC the window's drawn in first, `width` by `height`: made again when that's changed.
fn back(window_dc: windows::Win32::Graphics::Gdi::HDC, width: i32, height: i32) -> Option<windows::Win32::Graphics::Gdi::HDC> {
    use windows::Win32::Graphics::Gdi::{CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, HBITMAP, HDC, SelectObject};
    let (dc, bitmap, w, h) = BACK.get();
    if dc != 0 && w == width && h == height {
        return Some(HDC(dc as *mut c_void));
    }
    unsafe {
        if dc != 0 {
            let _ = DeleteDC(HDC(dc as *mut c_void));
            let _ = DeleteObject(HBITMAP(bitmap as *mut c_void).into());
        }
        let made = CreateCompatibleDC(Some(window_dc));
        let bitmap = CreateCompatibleBitmap(window_dc, width, height);
        if made.is_invalid() || bitmap.is_invalid() {
            BACK.set((0, 0, 0, 0));
            return None;
        }
        SelectObject(made, bitmap.into());
        BACK.set((made.0 as isize, bitmap.0 as isize, width, height));
        Some(made)
    }
}

/// Puts the last picture up, and what's over it, all at once.
fn show() {
    use windows::Win32::Graphics::Gdi::BitBlt;
    let Some(h) = hwnd() else { return };
    SHOWN.set(Some(std::time::Instant::now()));
    FRAME.with_borrow(|(frame, (across, down))| unsafe {
        let (across, down) = (*across, *down);
        if frame.len() < across * down || across == 0 {
            return;
        }
        let mut area = RECT::default();
        let _ = GetClientRect(h, &mut area);
        if area.right <= 0 || area.bottom <= 0 {
            return;
        }
        let window_dc = GetDC(Some(h));
        // Straight onto the window, if there's no memory to draw in first: the picture, then, without the bar.
        let dc = back(window_dc, area.right, area.bottom).unwrap_or(window_dc);
        // The picture's above the bar, all of it: the bar's room is the window's own, grown for it.
        let (extra, picture) = (EXTRA.get(), area.bottom - EXTRA.get());
        // Whole pixels, when it's shown a whole number of times its size; smoothed, when it had to be made to fit.
        let whole = area.right % across as i32 == 0 && picture % down as i32 == 0;
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
        StretchDIBits(dc, 0, 0, area.right, picture, 0, 0, across as i32, down as i32, Some(frame.as_ptr() as *const c_void), &info, DIB_RGB_COLORS, SRCCOPY);
        if dc != window_dc {
            crate::capture::draw(dc, area.right, picture, extra, crate::capture::tall(picture), pms_lang::pimpbox::CAPTURED.t());
            let _ = BitBlt(window_dc, 0, 0, area.right, area.bottom, Some(dc), 0, 0, SRCCOPY);
        }
        ReleaseDC(Some(h), window_dc);
    });
}
