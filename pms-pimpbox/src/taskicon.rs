//! The console window's icon (its taskbar button, its title bar, Alt+Tab) as the boot animation plays, in the animation's own
//! colors (pimpbox_boot::look, and each look's colors in pimpbox_boot::icon, which pms-art sets): as it warms up and its name
//! rains down, for each flash of rain, once the ball's a mirror ball, and from the mark on (as the program file has it), and
//! from then on. Drawn here as it runs, once for each look, at the sizes Windows uses on this PC; and handed to the
//! window only when the look changes: a dozen times a boot, which is nothing to the taskbar that shows it.

use std::cell::{Cell, RefCell};

use pimpbox_boot::Look;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, GetSystemMetrics, HICON, ICON_BIG, ICON_SMALL, ICONINFO, SM_CXICON, SM_CXSMICON, SendMessageW, WM_SETICON};

use pimpbox_boot::icon::{Colors, colors};

/// A look at the sizes its icons were made at (big, small).
#[derive(Clone, Copy, PartialEq)]
struct Made {
    look: Look,
    sizes: (i32, i32),
}

thread_local! {
    /// The icons made so far: each look's, big and small.
    static MADE: RefCell<Vec<(Made, HICON, HICON)>> = const { RefCell::new(Vec::new()) };
    /// What the window was last given, and which window that was.
    static GIVEN: Cell<Option<(isize, Made)>> = const { Cell::new(None) };
}

/// The sizes to make the window's icons at: big, as the taskbar shows it (24 pixels at 100%, scaled as the window's screen is;
/// it draws the big icon, and one of any other size it resamples, which blurs a crisp mark), and small, for the title bar.
fn sizes(h: HWND) -> (i32, i32) {
    let dpi = unsafe { GetDpiForWindow(h) } as i32;
    let big = if dpi > 0 { (24 * dpi + 48) / 96 } else { unsafe { GetSystemMetrics(SM_CXICON) } };
    let small = if dpi > 0 { unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi as u32) } } else { unsafe { GetSystemMetrics(SM_CXSMICON) } };
    (big, small)
}

/// An icon `size` pixels square, in `colors`.
fn made(size: i32, colors: Colors) -> Option<HICON> {
    let size = size.max(16);
    let pixels = pimpbox_boot::icon::pixels(size as u32, colors, pimpbox_boot::icon::shape());
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: size, biHeight: -size, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        // BGRA, top row first, as the section was made.
        let to = std::slice::from_raw_parts_mut(bits.cast::<u8>(), (size * size * 4) as usize);
        for (px, [r, g, b, a]) in to.as_chunks_mut::<4>().0.iter_mut().zip(pixels) {
            *px = [b, g, r, a];
        }
        // The mask's all clear: the alpha says what's there.
        let mask = CreateBitmap(size, size, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color }).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

/// The window `h` shows `look` from now on: given only if that's a change (or it's another window).
pub fn show(h: HWND, look: Look) {
    let want = Made { look, sizes: sizes(h) };
    if GIVEN.get() == Some((h.0 as isize, want)) {
        return;
    }
    let icons = MADE.with_borrow_mut(|kept| {
        if let Some((_, big, small)) = kept.iter().find(|(m, ..)| *m == want) {
            return Some((*big, *small));
        }
        let (big, small) = (made(want.sizes.0, colors(look))?, made(want.sizes.1, colors(look))?);
        kept.push((want, big, small));
        Some((big, small))
    });
    let Some((big, small)) = icons else { return };
    unsafe {
        SendMessageW(h, WM_SETICON, Some(WPARAM(ICON_BIG as usize)), Some(LPARAM(big.0 as isize)));
        SendMessageW(h, WM_SETICON, Some(WPARAM(ICON_SMALL as usize)), Some(LPARAM(small.0 as isize)));
    }
    GIVEN.set(Some((h.0 as isize, want)));
}
