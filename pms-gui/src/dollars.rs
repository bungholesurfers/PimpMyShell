//! What the settings window (winui.rs) floats between its acrylic and its controls: dollar bills and a word from our
//! sponsors (who aren't; see `copy::LEGAL`), tiled, drifting up and to the right. Crisp in the nav pane; behind the settings, where people read, it's
//! big gold dollar signs instead, soft-edged, which still read as what they are through a blur. (Drawn once, into a second tile, so that
//! they cost a frame nothing.) A swap chain in a SwapChainPanel behind everything else, transparent but for
//! the tile, drawn on a thread of its own so that the window's own work never makes it stutter (and it never holds
//! the window up). With Windows' animation effects off, it holds still. Anything failing here only means no dollars.
//!
//! The bills are the original ones or flags (flag.rs), as `settings_bill` in pms-art's design says. Flags wave: the bill is
//! drawn flat once, and each frame the tile is put together again from it, a pixel's width at a time, and blurred again.
//! The tile being small, that's little work.

use std::f32::consts::TAU;
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

use reactor_core::Interface as _;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D_SIZE_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_COMPOSITE_MODE_SOURCE_OVER, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device};
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, GetClassNameW, GetClientRect, IsIconic, IsWindow, IsWindowVisible, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows_core::{BOOL, Interface, Result, w};
use windows_numerics::{Matrix3x2, Vector2};
use windows_reactor::{ElementRef, SwapChainPanel};

use crate::flag;

/// The tile, its strength over the backdrop, and how fast it drifts on each axis (all DIPs).
const TILE_W: f32 = 288.0;
const TILE_H: f32 = 184.0;
const STRENGTH: f32 = 0.3;
/// Behind the settings: how soft the gold dollar signs' edges are (DIPs, roughly), how strong they are there, and how big.
const BLUR: f32 = 3.0;
const STRENGTH_BEHIND: f32 = 0.2;
const GOLD_SIZE: f32 = 120.0;
/// Gold: yellow, a little warm.
const GOLD: D2D1_COLOR_F = D2D1_COLOR_F { r: 1.0, g: 0.78, b: 0.16, a: 1.0 };
const DRIFT: f32 = 16.0;
/// See `copy::LEGAL`.
const SLOGAN: &str = "BUY $MSFT";
/// Banknote green.
const GREEN: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.52, g: 0.78, b: 0.42, a: 1.0 };
/// pms-art's design, which says which bills these are: `settings_bill`, flags unless it says otherwise.
const DESIGN: &str = include_str!("../../art/design.json");

fn flags() -> bool {
    !DESIGN.contains(r#""settings_bill": "classic""#)
}
/// The least time between frames, should presenting not wait for the screen.
const FRAME: Duration = Duration::from_millis(14);

/// The layer, as the window sees it: started on a tick, once the window and its panel exist.
#[derive(Default)]
pub struct Layer {
    /// The swap chain, until the panel has taken it.
    chain: Option<reactor_core::IUnknown>,
    started: bool,
    /// How wide the nav pane is (DIPs): the crisp part, left of the blurred.
    pane: Arc<AtomicU32>,
}

impl Layer {
    /// Starts the layer if it hasn't been, and hands the panel its swap chain. Call until it's done: the panel may not
    /// be published yet.
    pub fn attach(&mut self, panel: &ElementRef<SwapChainPanel>) {
        if !self.started && let Some(window) = window() {
            self.started = true;
            self.chain = start(window, self.pane.clone());
        }
        if let Some(chain) = &self.chain
            && panel.request_set_swap_chain(chain.clone(), |_| {})
        {
            self.chain = None;
        }
    }
}

impl Layer {
    pub fn set_pane(&self, width: u32) {
        self.pane.store(width, Ordering::Relaxed);
    }
}

/// This thread's WinUI window.
fn window() -> Option<isize> {
    unsafe extern "system" fn each(hwnd: HWND, out: LPARAM) -> BOOL {
        let mut class = [0u16; 64];
        let n = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
        if String::from_utf16_lossy(&class[..n]).starts_with("WinUIDesktop") && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            unsafe { *(out.0 as *mut isize) = hwnd.0 as isize };
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut found = 0isize;
    let _ = unsafe { EnumThreadWindows(GetCurrentThreadId(), Some(each), LPARAM(&mut found as *mut isize as isize)) };
    (found != 0).then_some(found)
}

/// Starts the drawing thread, and returns its swap chain.
fn start(window: isize, pane: Arc<AtomicU32>) -> Option<reactor_core::IUnknown> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = run(HWND(window as *mut c_void), &pane, &tx);
    });
    let raw = rx.recv().ok()?;
    Some(unsafe { reactor_core::IUnknown::from_raw(raw as *mut c_void) })
}

/// The window's client area in pixels, at least 1 by 1, and its DPI.
fn client(hwnd: HWND) -> (u32, u32, f32) {
    let mut r = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut r) };
    ((r.right - r.left).max(1) as u32, (r.bottom - r.top).max(1) as u32, unsafe { GetDpiForWindow(hwnd) }.max(96) as f32)
}

/// Whether Windows' animation effects are on.
fn animating() -> bool {
    let mut on = BOOL(1);
    let _ = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut BOOL as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) };
    on.as_bool()
}

fn run(hwnd: HWND, pane: &AtomicU32, tx: &Sender<usize>) -> Result<()> {
    unsafe {
        let mut device: Option<ID3D11Device> = None;
        D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, None)?;
        let device = device.ok_or_else(windows_core::Error::empty)?;
        let dxgi: IDXGIDevice = device.cast()?;
        let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
        let (mut width, mut height, mut dpi) = client(hwnd);
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            ..Default::default()
        };
        let swap = factory.CreateSwapChainForComposition(&device, &desc, None)?;
        let _ = tx.send(swap.clone().into_raw() as usize);

        let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let dc = d2d.CreateDevice(&dxgi)?.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
        let moving = animating();
        let began = Instant::now();
        // What depends on the window's size and DPI, made again when they change.
        let mut tile_for = 0.0;
        let mut tile: Option<Tiles> = None;
        let mut target = false;
        let mut drawn = false;
        let mut drawn_pane = u32::MAX;
        loop {
            if !IsWindow(Some(hwnd)).as_bool() {
                return Ok(());
            }
            if IsIconic(hwnd).as_bool() {
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            let (w, h, d) = client(hwnd);
            if (w, h, d) != (width, height, dpi) || !target {
                (width, height, dpi) = (w, h, d);
                dc.SetTarget(None);
                swap.ResizeBuffers(2, w, h, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SWAP_CHAIN_FLAG(0))?;
                // The panel shows a pixel of the swap chain per DIP unless told otherwise.
                let shrink = 96.0 / d;
                swap.cast::<IDXGISwapChain2>()?.SetMatrixTransform(&DXGI_MATRIX_3X2_F { _11: shrink, _22: shrink, ..Default::default() })?;
                dc.SetDpi(d, d);
                let surface: IDXGISurface = swap.GetBuffer(0)?;
                let props = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                    dpiX: d,
                    dpiY: d,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    ..Default::default()
                };
                dc.SetTarget(&dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?);
                target = true;
                drawn = false;
                if tile_for != d {
                    tile = Some(make_tiles(&dc)?);
                    tile_for = d;
                }
            }
            let nav = pane.load(Ordering::Relaxed);
            if drawn && !moving && nav == drawn_pane {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            let frame = Instant::now();
            let drift = if moving { began.elapsed().as_secs_f32() * DRIFT } else { 0.0 };
            let (dips_w, dips_h) = (w as f32 * 96.0 / d, h as f32 * 96.0 / d);
            if moving && let Some(wave) = tile.as_ref().and_then(|t| t.wave.as_ref()) {
                wave.draw(&dc, flag::REST - began.elapsed().as_secs_f32() * TAU / flag::PERIOD, d)?;
            }
            dc.BeginDraw();
            dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            if let Some(Tiles { crisp, soft, .. }) = &tile {
                let split = (nav as f32).min(dips_w);
                let moved = Matrix3x2::translation(drift % TILE_W, -(drift % TILE_H));
                crisp.SetTransform(&moved);
                soft.SetTransform(&moved);
                dc.FillRectangle(&D2D_RECT_F { left: 0.0, top: 0.0, right: split, bottom: dips_h }, crisp);
                dc.FillRectangle(&D2D_RECT_F { left: split, top: 0.0, right: dips_w, bottom: dips_h }, soft);
            }
            dc.EndDraw(None, None)?;
            swap.Present(1, DXGI_PRESENT(0)).ok()?;
            drawn = true;
            drawn_pane = nav;
            std::thread::sleep(FRAME.saturating_sub(frame.elapsed()));
        }
    }
}

/// The tile as two brushes that repeat, crisp and blurred, and what keeps them moving if the bills are flags.
struct Tiles {
    crisp: ID2D1BitmapBrush1,
    soft: ID2D1BitmapBrush1,
    wave: Option<Wave>,
}

/// What each frame of waving flags is made from, and into.
struct Wave {
    slogans: ID2D1Bitmap,
    flat: ID2D1Bitmap,
    crisp: ID2D1Bitmap1,
}

impl Wave {
    /// Puts the tile together at `phase`: the slogans, then each flag a pixel's width at a time. `dc` is left with its target as it
    /// was.
    unsafe fn draw(&self, dc: &ID2D1DeviceContext, phase: f32, dpi: f32) -> Result<()> {
        unsafe {
            let area = D2D_RECT_F { left: 0.0, top: 0.0, right: TILE_W, bottom: TILE_H };
            let clear = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
            let target = dc.GetTarget()?;
            let step = 96.0 / dpi;
            dc.SetTarget(&self.crisp);
            dc.BeginDraw();
            dc.Clear(Some(&clear));
            dc.DrawBitmap(&self.slogans, Some(&area), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
            // Each strip's edges lie on pixels, and its top and bottom in the picture's clear margin.
            dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
            for (x, middle, ahead) in flag::FLAGS {
                let x = (x / step).round() * step;
                let mut at = 0.0;
                while at < flag::FLAT_W {
                    let b = flag::bend((at + step / 2.0) / flag::FLAT_W, phase + ahead);
                    let half = flag::FLAT_H / 2.0 * b.stretch;
                    let to = D2D_RECT_F { left: x + at, top: middle + b.lift - half, right: x + at + step, bottom: middle + b.lift + half };
                    let from = D2D_RECT_F { left: at, top: 0.0, right: at + step, bottom: flag::FLAT_H };
                    dc.DrawBitmap(&self.flat, Some(&to), b.light, D2D1_INTERPOLATION_MODE_LINEAR, Some(&from), None);
                    at += step;
                }
            }
            dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            dc.EndDraw(None, None)?;
            dc.SetTarget(&target);
            Ok(())
        }
    }
}

/// The tile, as `Tiles` says: the bills, and the gold dollar signs, whose softening wraps around the tile's edges so that it has no
/// seams either. `dc` is left with its target as it was.
unsafe fn make_tiles(dc: &ID2D1DeviceContext) -> Result<Tiles> {
    unsafe {
        let (mut dpi_x, mut dpi_y) = (0.0, 0.0);
        dc.GetDpi(&mut dpi_x, &mut dpi_y);
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: dpi_x,
            dpiY: dpi_y,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
            ..Default::default()
        };
        let waving = flags();
        let still = if waving { draw_slogans(dc)? } else { draw_tile(dc)? };
        let size = still.GetPixelSize();
        // Flags are drawn into this each frame. The original bills are drawn once, and it's that picture.
        let moving: Option<ID2D1Bitmap1> = if waving { Some(dc.CreateBitmap(size, None, 0, &props)?) } else { None };
        let crisp: ID2D1Bitmap = match &moving {
            Some(bitmap) => bitmap.cast()?,
            None => still.clone(),
        };
        let soft = dc.CreateBitmap(size, None, 0, &props)?;
        let gold = draw_gold(dc)?;
        let wrapped = dc.CreateEffect(&CLSID_D2D1Border)?;
        wrapped.SetInput(0, &gold.cast::<ID2D1Image>()?, true);
        let wrap = (D2D1_BORDER_EDGE_MODE_WRAP.0 as u32).to_le_bytes();
        wrapped.SetValue(D2D1_BORDER_PROP_EDGE_MODE_X.0 as u32, D2D1_PROPERTY_TYPE_ENUM, &wrap)?;
        wrapped.SetValue(D2D1_BORDER_PROP_EDGE_MODE_Y.0 as u32, D2D1_PROPERTY_TYPE_ENUM, &wrap)?;
        let blur = dc.CreateEffect(&CLSID_D2D1GaussianBlur)?;
        blur.SetInput(0, &wrapped.GetOutput()?, true);
        blur.SetValue(D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &BLUR.to_le_bytes())?;
        let target = dc.GetTarget()?;
        dc.SetTarget(&soft);
        dc.BeginDraw();
        dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        let area = D2D_RECT_F { left: 0.0, top: 0.0, right: TILE_W, bottom: TILE_H };
        dc.DrawImage(&blur.GetOutput()?, None, Some(&area), D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
        dc.EndDraw(None, None)?;
        dc.SetTarget(&target);
        let repeat = D2D1_BITMAP_BRUSH_PROPERTIES1 { extendModeX: D2D1_EXTEND_MODE_WRAP, extendModeY: D2D1_EXTEND_MODE_WRAP, interpolationMode: D2D1_INTERPOLATION_MODE_LINEAR };
        let brush = |bitmap: &ID2D1Bitmap, opacity: f32| dc.CreateBitmapBrush(bitmap, Some(&repeat), Some(&D2D1_BRUSH_PROPERTIES { opacity, transform: Matrix3x2::identity() }));
        let tiles = Tiles { crisp: brush(&crisp, STRENGTH)?, soft: brush(&soft.cast()?, STRENGTH_BEHIND)?, wave: None };
        let Some(moving) = moving else { return Ok(tiles) };
        let wave = Wave { slogans: still, flat: draw_flag(dc)?, crisp: moving };
        wave.draw(dc, flag::REST, dpi_x)?;
        Ok(Tiles { wave: Some(wave), ..tiles })
    }
}

/// Big, heavy dollar signs in gold, two to a tile, for behind the settings.
unsafe fn draw_gold(dc: &ID2D1DeviceContext) -> Result<ID2D1Bitmap> {
    unsafe {
        let rt = dc.CreateCompatibleRenderTarget(Some(&D2D_SIZE_F { width: TILE_W, height: TILE_H }), None, None, D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE)?;
        let ink = rt.CreateSolidColorBrush(&GOLD, None)?;
        let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let font = write.CreateTextFormat(w!("Segoe UI Black"), None, DWRITE_FONT_WEIGHT_BLACK, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, GOLD_SIZE, w!("en-us"))?;
        font.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        font.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        let sign: Vec<u16> = "$".encode_utf16().collect();
        rt.BeginDraw();
        rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        for x in [TILE_W / 4.0, TILE_W * 3.0 / 4.0] {
            rt.DrawText(&sign, &font, &D2D_RECT_F { left: x - 70.0, top: 0.0, right: x + 70.0, bottom: TILE_H }, &ink, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        }
        rt.EndDraw(None, None)?;
        rt.GetBitmap()
    }
}

/// The font for what's written on the tile.
unsafe fn tile_font(size: f32) -> Result<IDWriteTextFormat> {
    unsafe {
        let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let f = write.CreateTextFormat(w!("Segoe UI"), None, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("en-us"))?;
        f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(f)
    }
}

/// What holds still between the flags: the slogans.
unsafe fn draw_slogans(dc: &ID2D1DeviceContext) -> Result<ID2D1Bitmap> {
    unsafe {
        let rt = dc.CreateCompatibleRenderTarget(Some(&D2D_SIZE_F { width: TILE_W, height: TILE_H }), None, None, D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE)?;
        let ink = rt.CreateSolidColorBrush(&GREEN, None)?;
        let font = tile_font(21.0)?;
        let words: Vec<u16> = SLOGAN.encode_utf16().collect();
        rt.BeginDraw();
        rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        for (x, y) in flag::SLOGANS {
            rt.DrawText(&words, &font, &D2D_RECT_F { left: x - 70.0, top: y - 20.0, right: x + 70.0, bottom: y + 20.0 }, &ink, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        }
        rt.EndDraw(None, None)?;
        rt.GetBitmap()
    }
}

/// The flag, flat: its trail of blocks, and the bill.
unsafe fn draw_flag(dc: &ID2D1DeviceContext) -> Result<ID2D1Bitmap> {
    unsafe {
        let rt = dc.CreateCompatibleRenderTarget(Some(&D2D_SIZE_F { width: flag::FLAT_W, height: flag::FLAT_H }), None, None, D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE)?;
        let ink = rt.CreateSolidColorBrush(&GREEN, None)?;
        let paper = rt.CreateSolidColorBrush(&D2D1_COLOR_F { a: flag::PAPER, ..GREEN }, None)?;
        let (x, y) = (flag::BILL_X + flag::BILL_W / 2.0, flag::BILL_Y + flag::BILL_H / 2.0);
        let edge = |inset: f32, radius: f32| D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F { left: flag::BILL_X + inset, top: flag::BILL_Y + inset, right: flag::BILL_X + flag::BILL_W - inset, bottom: flag::BILL_Y + flag::BILL_H - inset },
            radiusX: radius,
            radiusY: radius,
        };
        let font = tile_font(17.0)?;
        rt.BeginDraw();
        rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        for (bx, by, bw, bh, opacity) in flag::blocks() {
            let block = rt.CreateSolidColorBrush(&D2D1_COLOR_F { a: opacity, ..GREEN }, None)?;
            rt.FillRectangle(&D2D_RECT_F { left: bx, top: by, right: bx + bw, bottom: by + bh }, &block);
        }
        rt.FillRoundedRectangle(&edge(0.0, 5.0), &paper);
        rt.DrawRoundedRectangle(&edge(0.9, 5.0), &ink, 1.8, None);
        rt.DrawRoundedRectangle(&edge(4.0, 2.5), &ink, 0.8, None);
        rt.DrawEllipse(&D2D1_ELLIPSE { point: Vector2 { X: x, Y: y }, radiusX: 10.5, radiusY: 10.5 }, &ink, 1.2, None);
        let sign: Vec<u16> = "$".encode_utf16().collect();
        rt.DrawText(&sign, &font, &D2D_RECT_F { left: x - 20.0, top: y - 20.5, right: x + 20.0, bottom: y + 19.5 }, &ink, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        for (dx, dy) in [(-25.0, -8.5), (25.0, -8.5), (-25.0, 8.5), (25.0, 8.5)] {
            rt.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: x + dx, Y: y + dy }, radiusX: 1.4, radiusY: 1.4 }, &ink);
        }
        rt.EndDraw(None, None)?;
        rt.GetBitmap()
    }
}

/// The tile: two bills, each tilted its own way, with the slogan between them.
unsafe fn draw_tile(dc: &ID2D1DeviceContext) -> Result<ID2D1Bitmap> {
    unsafe {
        let rt = dc.CreateCompatibleRenderTarget(Some(&D2D_SIZE_F { width: TILE_W, height: TILE_H }), None, None, D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE)?;
        let ink = rt.CreateSolidColorBrush(&GREEN, None)?;
        let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let font = |size: f32| -> Result<IDWriteTextFormat> {
            let f = write.CreateTextFormat(w!("Segoe UI"), None, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("en-us"))?;
            f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            Ok(f)
        };
        let (big, small) = (font(21.0)?, font(17.0)?);
        let say = |words: &str, format: &IDWriteTextFormat, x: f32, y: f32| {
            let at = D2D_RECT_F { left: x - 70.0, top: y - 20.0, right: x + 70.0, bottom: y + 20.0 };
            let words: Vec<u16> = words.encode_utf16().collect();
            rt.DrawText(&words, format, &at, &ink, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
        };
        let bill = |x: f32, y: f32, tilt: f32| {
            rt.SetTransform(&Matrix3x2::rotation_around(tilt, Vector2 { X: x, Y: y }));
            let edge = |inset: f32, radius: f32| D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F { left: x - 36.0 + inset, top: y - 19.0 + inset, right: x + 36.0 - inset, bottom: y + 19.0 - inset },
                radiusX: radius,
                radiusY: radius,
            };
            rt.DrawRoundedRectangle(&edge(0.0, 5.0), &ink, 1.8, None);
            rt.DrawRoundedRectangle(&edge(4.0, 2.5), &ink, 0.8, None);
            rt.DrawEllipse(&D2D1_ELLIPSE { point: Vector2 { X: x, Y: y }, radiusX: 10.5, radiusY: 10.5 }, &ink, 1.2, None);
            say("$", &small, x, y - 0.5);
            for (dx, dy) in [(-25.0, -8.5), (25.0, -8.5), (-25.0, 8.5), (25.0, 8.5)] {
                rt.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: x + dx, Y: y + dy }, radiusX: 1.4, radiusY: 1.4 }, &ink);
            }
            rt.SetTransform(&Matrix3x2::identity());
        };
        rt.BeginDraw();
        rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        bill(50.0, 48.0, -14.0);
        bill(TILE_W / 2.0 + 56.0, TILE_H / 2.0 + 48.0, 11.0);
        say(SLOGAN, &big, TILE_W / 2.0 + 62.0, 48.0);
        say(SLOGAN, &big, 74.0, TILE_H / 2.0 + 48.0);
        rt.EndDraw(None, None)?;
        rt.GetBitmap()
    }
}
