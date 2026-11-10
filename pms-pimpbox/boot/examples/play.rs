//! The boot animation in a window, drawn by the graphics card at the window's own size, at the screen's own pace.
//!
//!   play [--loop] [--full]
//!
//! Any key or a click ends it; so does its ending, unless it's to go round again. It says how many frames it showed.

use std::time::Instant;

use pimpbox_boot::gpu::Gpu;
use pimpbox_boot::LENGTH;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D11::{ID3D11RenderTargetView, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, Result, w};

extern "system" fn proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_KEYDOWN | WM_LBUTTONDOWN | WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (again, full) = (args.iter().any(|a| a == "--loop"), args.iter().any(|a| a == "--full"));
    let boot = pimpbox_boot::fonts::boot();
    let gpu = Gpu::new(boot)?;
    unsafe {
        let module = GetModuleHandleW(None)?;
        let class = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: module.into(), lpszClassName: w!("PimpboxBoot"), hCursor: LoadCursorW(None, IDC_ARROW)?, ..Default::default() };
        RegisterClassW(&class);
        let (style, x, y, wide, tall) = if full {
            (WS_POPUP, 0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
        } else {
            let mut area = RECT { left: 0, top: 0, right: 1280, bottom: 960 };
            let _ = AdjustWindowRect(&mut area, WS_OVERLAPPEDWINDOW, false);
            (WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, CW_USEDEFAULT, area.right - area.left, area.bottom - area.top)
        };
        let window = CreateWindowExW(WINDOW_EX_STYLE(0), w!("PimpboxBoot"), w!("PIMPBOX"), style | WS_VISIBLE, x, y, wide, tall, None, None, Some(module.into()), None)?;

        let factory: IDXGIFactory2 = gpu.device().cast::<IDXGIDevice>()?.GetAdapter()?.GetParent()?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            ..Default::default()
        };
        let chain = factory.CreateSwapChainForHwnd(gpu.device(), window, &desc, None, None)?;
        let mut target: Option<(ID3D11RenderTargetView, u32, u32)> = None;

        let (began, mut shown) = (Instant::now(), 0u32);
        'showing: loop {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                if message.message == WM_QUIT {
                    break 'showing;
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            let mut t = began.elapsed().as_secs_f32();
            if t >= LENGTH {
                if !again {
                    break;
                }
                t %= LENGTH;
            }
            // Somewhere to draw to that's the size the window is now.
            let mut area = RECT::default();
            GetClientRect(window, &mut area)?;
            let (w, h) = (area.right.max(1) as u32, area.bottom.max(1) as u32);
            if target.as_ref().is_none_or(|(_, tw, th)| (*tw, *th) != (w, h)) {
                // The old one has to be let go of first.
                drop(target.take());
                chain.ResizeBuffers(0, w, h, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
                let back: ID3D11Texture2D = chain.GetBuffer(0)?;
                let mut view = None;
                gpu.device().CreateRenderTargetView(&back, None, Some(&mut view))?;
                target = view.map(|view| (view, w, h));
            }
            if let Some((view, w, h)) = &target {
                gpu.draw(t, view, *w, *h);
            }
            // Waits for the screen: its pace is ours.
            chain.Present(1, DXGI_PRESENT(0)).ok()?;
            shown += 1;
        }
        let took = began.elapsed().as_secs_f32();
        println!("{shown} frames in {took:.1} s: {:.1} a second", shown as f32 / took);
    }
    Ok(())
}
