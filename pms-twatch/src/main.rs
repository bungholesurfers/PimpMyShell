//! Plays a live stream onto a screen, with Windows' own media stack: `pms-twatch --screen <mapping> <event> <cap> <floor>`, the
//! screen's two handles handed to it (pms-spawn), the last two as below, which it also starts at. A process of its own so that a stream it can't cope with is nobody else's problem, and so
//! that the daemon stays small.
//!
//! The video never reaches a window. Each frame is scaled on the GPU to a few times the size the screen wants and averaged
//! down to size, on the GPU too where it can be, which keeps a picture this small from shimmering; only what's that size is
//! read back. The audio plays only if it's asked to.
//!
//! The daemon talks to it on stdin, a line at a time, and when stdin closes it stops. First, before anything plays:
//!
//!   proxy <address>       a socks5:// or socks5h:// address, if the video goes through the proxy
//!   play <address>        the stream's master playlist, which carries Twitch's token for it
//!
//! both on stdin rather than its command line or environment, where any program could read them. Then, as it plays:
//!
//!   cap <bits a second>   go no higher than this (0: no limit)
//!   floor <bits a second> go no lower, which keeps it off an audio-only rendition
//!   mute <0 or 1>
//!   volume <0 to 100>
//!   video <0 or 1>        whether to bother with the picture: with no display to time itself by, Windows' player keeps a CPU thread
//!                         flat out for as long as it's serving frames, taken or not. It keeps playing, sound included, and a few
//!                         seconds after this comes back on so do the frames.
//!   media <0 or 1>        whether it shows in Windows' media controls (the volume flyout, the lock screen, media keys). It starts
//!                         off: left to itself, Windows' player puts itself there, and takes the media keys.
//!   title <words>         what the media controls call it, while it's there
//!   fps <frames a second> at most how many frames a second to take (0: every one there is): those between cost nothing
//!   direct <0 or 1>       whether Explorer is showing the surface it said (`surface`): then frames go there alone
//!
//! and it answers on stdout: `playing <width> <height>`, `buffering`, `rate <bits a second>`, `failed <why>`, `ended`, and
//! `surface <handle>`: a DirectComposition surface that each frame is presented to as well, straight from the GPU, for the
//! daemon to hand Explorer (0: there's none any more).
//!
//! With a proxy given, nothing goes over the network the way Windows' player would send it: Windows' player is given an HTTP
//! client whose only filter is ours (ByProxy), so every request it makes, the first playlist included, comes here and is made
//! through the proxy, and Windows' own HTTP stack never sees one. Quality changes stay Windows' own, seamless, as without one.

#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use pms_screen::Screen;
use windows::Foundation::{IClosable, IClosable_Impl, IReference, PropertyValue, TypedEventHandler, Uri};
use windows::Graphics::DirectX::Direct3D11::IDirect3DSurface;
use windows::Media::Core::MediaSource;
use windows::Media::MediaPlaybackType;
use windows::Media::Playback::{IMediaPlaybackSource, MediaPlaybackSession, MediaPlaybackState, MediaPlayer, MediaPlayerFailedEventArgs};
use windows::Media::Streaming::Adaptive::{AdaptiveMediaSource, AdaptiveMediaSourceCreationStatus, AdaptiveMediaSourcePlaybackBitrateChangedEventArgs};
use windows::Storage::Streams::DataWriter;
use windows::Web::Http::Filters::{IHttpFilter, IHttpFilter_Impl};
use windows::Web::Http::{HttpBufferContent, HttpClient, HttpMethod, HttpProgress, HttpRequestMessage, HttpResponseMessage, HttpStatusCode};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HMODULE, S_OK};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::DirectComposition::DCompositionCreateSurfaceHandle;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_RESOURCE_MISC_GENERATE_MIPS, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactoryMedia, IDXGIKeyedMutex,
    IDXGISurface, IDXGISwapChain1,
};
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11SurfaceFromDXGISurface;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows_core::{HRESULT, HSTRING, IInspectable, Interface, Ref, implement};
use windows_future::IAsyncOperationWithProgress;

type R<T> = Result<T, String>;

/// How many times finer than the screen a frame is drawn, each way, before it's averaged down: a power of two.
const FINE: u32 = 4;
/// The mip levels that takes: one for each halving, and the one it starts from. The last is the size wanted.
const LEVELS: u32 = FINE.trailing_zeros() + 1;
const _: () = assert!(FINE.is_power_of_two() && FINE > 1);

fn say(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn w(e: windows_core::Error) -> String {
    e.to_string()
}

/// Where frames go at the size that's wanted. Windows' player draws each frame FINE times bigger each way onto `drawn`; then,
/// where it can (`mips`), the GPU averages it down: it's copied into a texture with mip levels, which the GPU makes (the last
/// level is the 4×4 average, the size wanted), and only that's read back. Where it can't, the whole big frame is read back and
/// averaged here, on the CPU.
///
/// What the CPU spends on a frame (2026-11-17, 85×48, NVIDIA): ~6 million cycles with the frame averaged here, ~4 with the GPU
/// averaging, ~3 with `drawn` shared as well. Most of what's left is Windows' player's own copy, the same whatever the size: its
/// device isn't ours, so for a texture of ours that isn't shared it makes a shared one, draws on that and copies it over, a
/// new one every frame. Given one that's shared already, with a keyed mutex, it draws straight onto it.
struct Target {
    width: u32,
    height: u32,
    surface: IDirect3DSurface,
    drawn: ID3D11Texture2D,
    /// `drawn`'s keyed mutex, when it's shared: Windows' player takes it while it draws, and so do we while the GPU copies.
    keyed: Option<IDXGIKeyedMutex>,
    /// The texture with mip levels and its view, for the GPU to average with: None where it's done here. (The player won't draw
    /// on one itself: a level's own surface isn't one it takes.)
    mips: Option<(ID3D11Texture2D, ID3D11ShaderResourceView)>,
    readable: ID3D11Texture2D,
    out: Vec<u8>,
}

/// The key that Windows' player takes `drawn`'s mutex with, and gives it back with.
const KEY: u64 = 0;

struct Frames {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    target: Option<Target>,
    screen: Screen,
    /// Whether the GPU averages, and whether `drawn` is shared: each until it turns out it can't be.
    on_gpu: bool,
    shared: bool,
    /// When the next frame's to be taken, at most `FPS` a second; the ones between are let go.
    next: Option<Instant>,
    /// The picture straight to the taskbar, where it can be: None until there's a frame for it, and for good once it can't.
    shown: Option<Shown>,
    can_show: bool,
}

/// The picture straight to the taskbar: a swap chain on a DirectComposition surface, which Explorer shows (the daemon hands
/// it the surface's handle, said on stdout as `surface <handle>`). Each frame goes from the GPU's average onto it and is
/// presented, with nothing read back: once Explorer's showing it (`direct 1`), the screen's memory only gets a count.
struct Shown {
    /// The surface's handle: kept for as long as the swap chain, so that it's there to be taken.
    handle: HANDLE,
    chain: IDXGISwapChain1,
    back: Option<ID3D11Texture2D>,
    width: u32,
    height: u32,
}

impl Drop for Shown {
    fn drop(&mut self) {
        self.back = None;
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

/// Explorer is showing the surface: frames go there alone (`direct`).
static DIRECT: AtomicBool = AtomicBool::new(false);

/// What a DirectComposition surface's handle may do: be read and written (dcomp.h).
const COMPOSITIONOBJECT_ALL_ACCESS: u32 = 0x0003;

/// A swap chain of `width` by `height` on a new DirectComposition surface, on `device`'s GPU.
fn shown(device: &ID3D11Device, width: u32, height: u32) -> R<Shown> {
    let handle = unsafe { DCompositionCreateSurfaceHandle(COMPOSITIONOBJECT_ALL_ACCESS, None) }.map_err(|e| format!("its surface: {e}"))?;
    let made = (|| -> R<(IDXGISwapChain1, ID3D11Texture2D)> {
        // From the factory of the device's own adapter, so that the two are on the same GPU.
        let adapter = unsafe { device.cast::<IDXGIDevice>().map_err(w)?.GetAdapter() }.map_err(w)?;
        let factory: IDXGIFactoryMedia = unsafe { adapter.GetParent() }.map_err(w)?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            ..Default::default()
        };
        let chain = unsafe { factory.CreateSwapChainForCompositionSurfaceHandle(device, Some(handle), &desc, None) }.map_err(|e| format!("its swap chain: {e}"))?;
        let back = unsafe { chain.GetBuffer(0) }.map_err(w)?;
        Ok((chain, back))
    })();
    match made {
        Ok((chain, back)) => Ok(Shown { handle, chain, back: Some(back), width, height }),
        Err(e) => {
            let _ = unsafe { CloseHandle(handle) };
            Err(e)
        }
    }
}

/// Puts level `level` of `from`, `width` by `height`, on the taskbar by way of `shown` (made, or made the size, as needed).
/// Whether there's a new surface to say: its handle.
fn present(device: &ID3D11Device, context: &ID3D11DeviceContext, shown: &mut Option<Shown>, from: &ID3D11Texture2D, level: u32, width: u32, height: u32) -> R<Option<HANDLE>> {
    let mut new = None;
    if shown.is_none() {
        let s = self::shown(device, width, height)?;
        new = Some(s.handle);
        *shown = Some(s);
    }
    let s = shown.as_mut().ok_or("no swap chain")?;
    if (s.width, s.height) != (width, height) {
        // Every hold on its buffers goes before they're made again; the surface, and so its handle, stays.
        s.back = None;
        unsafe { s.chain.ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0)) }.map_err(|e| format!("its new size: {e}"))?;
        s.back = Some(unsafe { s.chain.GetBuffer(0) }.map_err(w)?);
        (s.width, s.height) = (width, height);
    }
    let back = s.back.as_ref().ok_or("no buffer")?;
    unsafe {
        context.CopySubresourceRegion(back, 0, 0, 0, 0, from, level, None);
        s.chain.Present(0, DXGI_PRESENT(0)).ok().map_err(|e| format!("presenting: {e}"))?;
    }
    Ok(new)
}

/// The most frames a second taken (`fps`): 0, every one there is.
static FPS: AtomicU32 = AtomicU32::new(0);

// The device is made for use from any thread, and the context is only ever used with the mutex around this held.
unsafe impl Send for Frames {}

fn texture(device: &ID3D11Device, width: u32, height: u32, readable: bool, mips: u32, shared: bool) -> R<ID3D11Texture2D> {
    let mut misc = if mips > 1 { D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32 } else { 0 };
    if shared {
        misc |= D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0 as u32;
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: mips,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: if readable { D3D11_USAGE_STAGING } else { D3D11_USAGE_DEFAULT },
        BindFlags: if readable { 0 } else { (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32 },
        CPUAccessFlags: if readable { D3D11_CPU_ACCESS_READ.0 as u32 } else { 0 },
        MiscFlags: misc,
    };
    let mut made = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut made)) }.map_err(w)?;
    made.ok_or_else(|| "no texture".to_string())
}

/// What the player draws on, `width`×`height` wanted: averaged on the GPU (`on_gpu`) or here, and `shared` or not.
fn target(device: &ID3D11Device, width: u32, height: u32, on_gpu: bool, shared: bool) -> R<Target> {
    let (fine_w, fine_h) = (width * FINE, height * FINE);
    let out = vec![0; (width * height * 4) as usize];
    let drawn = texture(device, fine_w, fine_h, false, 1, shared)?;
    let keyed = if shared { Some(drawn.cast::<IDXGIKeyedMutex>().map_err(w)?) } else { None };
    let dxgi: IDXGISurface = drawn.cast().map_err(w)?;
    let surface: IDirect3DSurface = unsafe { CreateDirect3D11SurfaceFromDXGISurface(&dxgi) }.map_err(w)?.cast().map_err(w)?;
    if on_gpu {
        // Each level down averages 2×2 of the one above: the last is the size wanted.
        let mipped = texture(device, fine_w, fine_h, false, LEVELS, false)?;
        let mut view = None;
        unsafe { device.CreateShaderResourceView(&mipped, None, Some(&mut view)) }.map_err(w)?;
        let readable = texture(device, width, height, true, 1, false)?;
        return Ok(Target { width, height, surface, drawn, keyed, mips: Some((mipped, view.ok_or("no view")?)), readable, out });
    }
    let readable = texture(device, fine_w, fine_h, true, 1, false)?;
    Ok(Target { width, height, surface, drawn, keyed, mips: None, readable, out })
}

/// Averages `fine`, `FINE` times the size each way with `pitch` bytes a row, down to `out`, opaque.
fn average(fine: &[u8], pitch: usize, width: usize, height: usize, out: &mut [u8]) {
    let (step, dots) = (FINE as usize, FINE * FINE);
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0u32; 3];
            for dy in 0..step {
                let row = &fine[(y * step + dy) * pitch + x * step * 4..][..step * 4];
                for dot in row.as_chunks::<4>().0 {
                    for (s, v) in sum.iter_mut().zip(dot) {
                        *s += u32::from(*v);
                    }
                }
            }
            let at = (y * width + x) * 4;
            for (o, s) in out[at..at + 3].iter_mut().zip(sum) {
                *o = (s / dots) as u8;
            }
            out[at + 3] = 255;
        }
    }
}

/// Copies `rows` rows of `width` pixels, `pitch` bytes apart, into `out`, opaque.
fn copy_rows(from: &[u8], pitch: usize, width: usize, rows: usize, out: &mut [u8]) {
    for y in 0..rows {
        let row = &mut out[y * width * 4..][..width * 4];
        row.copy_from_slice(&from[y * pitch..][..width * 4]);
        for dot in row.as_chunks_mut::<4>().0 {
            dot[3] = 255;
        }
    }
}

/// Whether a frame offered `now` is one to take, at most `fps` a second (0: all of them), the next one due at `next`. A frame
/// a little early (a quarter of the time between frames) still counts as on time, so that 60 a second become 30 evenly.
fn due(now: Instant, next: &mut Option<Instant>, fps: u32) -> bool {
    if fps == 0 {
        return true;
    }
    let every = std::time::Duration::from_secs(1) / fps;
    if let Some(at) = *next
        && now + every / 4 < at
    {
        return false;
    }
    *next = Some(match *next {
        Some(at) if now < at + every => at + every,
        _ => now + every,
    });
    true
}

impl Frames {
    /// Takes the frame there is, if one's wanted and it's due.
    fn take(&mut self, player: &MediaPlayer) -> R<()> {
        let Some((width, height)) = self.screen.wanted() else { return Ok(()) };
        if !due(Instant::now(), &mut self.next, FPS.load(Ordering::Relaxed)) {
            return Ok(());
        }
        if self.target.as_ref().is_none_or(|t| (t.width, t.height) != (width, height)) {
            self.target = None;
            // The best there can be first, then without what didn't work, each given up on for good.
            while self.target.is_none() {
                match target(&self.device, width, height, self.on_gpu, self.shared) {
                    Ok(t) => self.target = Some(t),
                    Err(e) if self.shared => {
                        self.shared = false;
                        say(&format!("note frames: not shared from now on, as it couldn't be: {e}"));
                    }
                    Err(e) if self.on_gpu => {
                        self.on_gpu = false;
                        say(&format!("note frames: averaged here from now on, as the GPU couldn't: {e}"));
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        let t = self.target.as_mut().ok_or("no target")?;
        player.CopyFrameToVideoSurface(&t.surface).map_err(w)?;
        if let Some(k) = &t.keyed {
            // Windows' player has given it back by now; if it's kept it, or used another key, it's to be a texture of ours alone.
            let got = unsafe { (Interface::vtable(k).AcquireSync)(Interface::as_raw(k), KEY, 50) };
            if got != S_OK {
                self.shared = false;
                self.target = None;
                say(&format!("note frames: not shared from now on, as the player didn't give it back ({:#x})", got.0));
                return Ok(());
            }
        }
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            // The frame goes where it's averaged, then the mutex goes back: the copy itself happens on the GPU, in turn.
            let fine_rows = if let Some((mipped, _)) = &t.mips {
                self.context.CopySubresourceRegion(mipped, 0, 0, 0, 0, &t.drawn, 0, None);
                None
            } else {
                self.context.CopyResource(&t.readable, &t.drawn);
                Some(height * FINE)
            };
            if let Some(k) = &t.keyed {
                let _ = k.ReleaseSync(KEY);
            }
            if let Some((mipped, view)) = &t.mips {
                self.context.GenerateMips(view);
                // Straight to the taskbar, where it can be; and once Explorer's showing it, only that, and a count.
                if self.can_show {
                    match present(&self.device, &self.context, &mut self.shown, mipped, LEVELS - 1, width, height) {
                        Ok(Some(handle)) => say(&format!("surface {}", handle.0 as usize)),
                        Ok(None) => {}
                        Err(e) => {
                            self.can_show = false;
                            self.shown = None;
                            say(&format!("note frames: not straight to the taskbar from now on: {e}"));
                            say("surface 0");
                        }
                    }
                    if self.shown.is_some() && DIRECT.load(Ordering::Relaxed) {
                        self.screen.counted();
                        return Ok(());
                    }
                }
                self.context.CopySubresourceRegion(&t.readable, 0, 0, 0, 0, mipped, LEVELS - 1, None);
            }
            self.context.Map(&t.readable, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).map_err(w)?;
            let pitch = mapped.RowPitch as usize;
            let bytes = std::slice::from_raw_parts(mapped.pData as *const u8, pitch * fine_rows.unwrap_or(height) as usize);
            match fine_rows {
                Some(_) => average(bytes, pitch, width as usize, height as usize, &mut t.out),
                None => copy_rows(bytes, pitch, width as usize, height as usize, &mut t.out),
            }
            self.context.Unmap(&t.readable, 0);
        }
        self.screen.publish(width, height, &t.out);
        Ok(())
    }
}

fn device() -> R<(ID3D11Device, ID3D11DeviceContext)> {
    let (mut device, mut context) = (None, None);
    unsafe {
        D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context))
    }
    .map_err(w)?;
    Ok((device.ok_or("no device")?, context.ok_or("no context")?))
}

fn bits(v: u32) -> R<Option<IReference<u32>>> {
    if v == 0 {
        return Ok(None);
    }
    Ok(Some(PropertyValue::CreateUInt32(v).map_err(w)?.cast().map_err(w)?))
}

/// In Windows' media controls, or not. Both have to say so: the player's own handling puts it there, and the controls it has
/// would stay there without it.
fn media(player: &MediaPlayer, on: bool) -> R<()> {
    player.CommandManager().map_err(w)?.SetIsEnabled(on).map_err(w)?;
    player.SystemMediaTransportControls().map_err(w)?.SetIsEnabled(on).map_err(w)
}

/// What the media controls call it.
fn title(player: &MediaPlayer, words: &str) -> R<()> {
    let shown = player.SystemMediaTransportControls().map_err(w)?.DisplayUpdater().map_err(w)?;
    shown.SetType(MediaPlaybackType::Video).map_err(w)?;
    shown.VideoProperties().map_err(w)?.SetTitle(&HSTRING::from(words)).map_err(w)?;
    shown.Update().map_err(w)
}

/// What comes in on stdin, until it closes.
fn listen(player: &MediaPlayer, source: &AdaptiveMediaSource) {
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let (word, words) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        let value: u32 = words.trim().parse().unwrap_or(0);
        let done = match word {
            "media" => media(player, value != 0),
            "title" => title(player, words.trim()),
            "cap" => bits(value).and_then(|v| source.SetDesiredMaxBitrate(v.as_ref()).map_err(w)),
            "floor" => bits(value).and_then(|v| source.SetDesiredMinBitrate(v.as_ref()).map_err(w)),
            "mute" => player.SetIsMuted(value != 0).map_err(w),
            "volume" => player.SetVolume(f64::from(value.min(100)) / 100.0).map_err(w),
            "video" => player.SetIsVideoFrameServerEnabled(value != 0).map_err(w),
            "fps" => {
                FPS.store(value, Ordering::Relaxed);
                Ok(())
            }
            "direct" => {
                DIRECT.store(value != 0, Ordering::Relaxed);
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = done {
            say(&format!("note {word}: {e}"));
        }
    }
}

/// What fetches through the proxy: HTTPS only, nobody's certificates but its own, and never without the proxy.
fn proxied(address: &str) -> R<ureq::Agent> {
    use std::time::Duration;
    let proxy = ureq::Proxy::new(address).map_err(|e| format!("the proxy: {e}"))?;
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::WebPki).build();
    Ok(ureq::Agent::config_builder()
        .https_only(true)
        .tls_config(tls)
        .proxy(Some(proxy))
        .max_redirects(3)
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into())
}

/// Windows' player's HTTP, when there's a proxy: the only filter its HTTP client has, so every request it makes comes here, and
/// is made through the proxy, by `ureq`. There's nothing beneath it: Windows' own HTTP stack is never asked to send one.
#[implement(IHttpFilter, IClosable)]
struct ByProxy(ureq::Agent);

impl IClosable_Impl for ByProxy_Impl {
    fn Close(&self) -> windows_core::Result<()> {
        Ok(())
    }
}

impl IHttpFilter_Impl for ByProxy_Impl {
    fn SendRequestAsync(&self, request: Ref<HttpRequestMessage>) -> windows_core::Result<IAsyncOperationWithProgress<HttpResponseMessage, HttpProgress>> {
        let request = request.ok()?.clone();
        let agent = self.0.clone();
        Ok(IAsyncOperationWithProgress::spawn(move || {
            answer(&agent, &request).map_err(|e| {
                say(&format!("note fetch: {e}"));
                // A failure as Windows' own HTTP would have had one (ERROR_INTERNET_CONNECTION_RESET): it tries again, or gives
                // up, as it would.
                windows_core::Error::new(HRESULT(0x8007_2EFD_u32 as i32), e)
            })
        }))
    }
}

/// What `request` gets, asked for through `agent`: GET and HEAD, all Windows' player asks, with the range it wants.
fn answer(agent: &ureq::Agent, request: &HttpRequestMessage) -> R<HttpResponseMessage> {
    let method = request.Method().map_err(w)?.Method().map_err(w)?.to_string();
    let url = request.RequestUri().map_err(w)?.AbsoluteUri().map_err(w)?.to_string();
    let mut sent = match method.as_str() {
        "GET" => agent.get(&url),
        "HEAD" => agent.head(&url),
        other => return Err(format!("{other}: not something it's ever asked for")),
    };
    let asked = request.Headers().map_err(w)?;
    for name in ["Range", "Accept"] {
        let key = HSTRING::from(name);
        if asked.HasKey(&key).unwrap_or(false) {
            sent = sent.header(name, &asked.Lookup(&key).map_err(w)?.to_string());
        }
    }
    let response = sent.call().map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let landed = ureq::ResponseExt::get_uri(&response).to_string();
    let headers: Vec<(String, String)> = response.headers().iter().filter_map(|(k, v)| Some((k.as_str().to_ascii_lowercase(), v.to_str().ok()?.to_string()))).collect();
    let body = if method == "HEAD" { Vec::new() } else { response.into_body().with_config().limit(64 << 20).read_to_vec().map_err(|e| e.to_string())? };

    let answer = HttpResponseMessage::Create(HttpStatusCode(i32::from(status))).map_err(w)?;
    let content = HttpBufferContent::CreateFromBuffer(&buffer(&body)?).map_err(w)?;
    let (headers_of_content, headers_of_answer) = (content.Headers().map_err(w)?, answer.Headers().map_err(w)?);
    for (name, value) in headers {
        // The length is the body's own, and what the connection was is nothing to do with it.
        if matches!(name.as_str(), "content-length" | "transfer-encoding" | "connection" | "keep-alive") {
            continue;
        }
        let (name, value) = (HSTRING::from(name.as_str()), HSTRING::from(value));
        if name.to_string().starts_with("content-") || name == "expires" || name == "last-modified" {
            let _ = headers_of_content.TryAppendWithoutValidation(&name, &value);
        } else {
            let _ = headers_of_answer.TryAppendWithoutValidation(&name, &value);
        }
    }
    answer.SetContent(&content).map_err(w)?;
    // Where it was redirected to is where what's in it is relative to, as it would be had Windows followed the redirect.
    let asked_for = if landed == url { request.clone() } else { HttpRequestMessage::Create(&HttpMethod::Get().map_err(w)?, &Uri::CreateUri(&HSTRING::from(landed)).map_err(w)?).map_err(w)? };
    answer.SetRequestMessage(&asked_for).map_err(w)?;
    Ok(answer)
}

/// The bytes, as something WinRT can read.
fn buffer(bytes: &[u8]) -> R<windows::Storage::Streams::IBuffer> {
    let writer = DataWriter::new().map_err(w)?;
    writer.WriteBytes(bytes).map_err(w)?;
    writer.DetachBuffer().map_err(w)
}


fn run() -> R<()> {
    // Its screen comes by handle; what's secret (the stream's address, with Twitch's token in it, and the proxy, which can
    // carry a password) comes down stdin, first: `proxy <address>` if there's one, then `play <address>`.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [flag, mapping, frame, cap, floor] = args.as_slice() else { return Err("usage: pms-twatch --screen <mapping> <event> <cap> <floor>, then `play <url>` on stdin".into()) };
    if flag != "--screen" {
        return Err("usage: pms-twatch --screen <mapping> <event> <cap> <floor>".into());
    }
    let (cap, floor): (u32, u32) = (cap.parse().unwrap_or(0), floor.parse().unwrap_or(0));
    let (mut proxy, mut url) = (None, String::new());
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| format!("stdin: {e}"))?;
        match line.split_once(' ') {
            Some(("proxy", p)) => proxy = Some(p.trim().to_string()).filter(|p| !p.is_empty()),
            Some(("play", u)) => {
                url = u.trim().to_string();
                break;
            }
            _ => {}
        }
    }
    if url.is_empty() {
        return Err("nothing to play".into());
    }
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(w)?;
    let screen = Screen::from_handles(mapping.parse().map_err(|_| "--screen: a handle's a number")?, frame.parse().map_err(|_| "--screen: a handle's a number")?)?;
    let (device, context) = device()?;
    let frames = Arc::new(Mutex::new(Frames { device, context, target: None, screen, on_gpu: true, shared: true, next: None, shown: None, can_show: true }));

    let uri = Uri::CreateUri(&HSTRING::from(url.as_str())).map_err(w)?;
    let made = match proxy {
        Some(proxy) => {
            let filter: IHttpFilter = ByProxy(proxied(&proxy)?).into();
            AdaptiveMediaSource::CreateFromUriWithDownloaderAsync(&uri, &HttpClient::Create(&filter).map_err(w)?)
        }
        None => AdaptiveMediaSource::CreateFromUriAsync(&uri),
    }
    .map_err(w)?
    .join()
    .map_err(w)?;
    let status = made.Status().map_err(w)?;
    if status != AdaptiveMediaSourceCreationStatus::Success {
        return Err(format!("the stream wouldn't open ({})", status.0));
    }
    let source = made.MediaSource().map_err(w)?;
    source.SetDesiredMaxBitrate(bits(cap)?.as_ref()).map_err(w)?;
    source.SetDesiredMinBitrate(bits(floor)?.as_ref()).map_err(w)?;
    // Left alone it starts at the bottom and climbs for half a minute.
    if cap != 0 {
        let available = source.AvailableBitrates().map_err(w)?;
        if available.into_iter().any(|rate| rate == cap) {
            source.SetInitialBitrate(cap).map_err(w)?;
        }
    }
    source
        .PlaybackBitrateChanged(&TypedEventHandler::<AdaptiveMediaSource, AdaptiveMediaSourcePlaybackBitrateChangedEventArgs>::new(|_, args| {
            if let Some(args) = args.as_ref() {
                say(&format!("rate {}", args.NewValue()?));
            }
            Ok(())
        }))
        .map_err(w)?;

    let player = MediaPlayer::new().map_err(w)?;
    player.SetIsVideoFrameServerEnabled(true).map_err(w)?;
    player.SetIsMuted(true).map_err(w)?;
    player.SetRealTimePlayback(true).map_err(w)?;
    player.SetAutoPlay(true).map_err(w)?;
    media(&player, false)?;
    let taking = frames.clone();
    player
        .VideoFrameAvailable(&TypedEventHandler::<MediaPlayer, IInspectable>::new(move |player, _| {
            if let (Some(player), Ok(mut frames)) = (player.as_ref(), taking.lock())
                && let Err(e) = frames.take(player)
            {
                say(&format!("note frame: {e}"));
            }
            Ok(())
        }))
        .map_err(w)?;
    player
        .MediaFailed(&TypedEventHandler::<MediaPlayer, MediaPlayerFailedEventArgs>::new(|_, args| {
            let why = args.as_ref().and_then(|a| a.ErrorMessage().ok()).map_or_else(String::new, |m| m.to_string());
            say(&format!("failed {}", why.replace(['\r', '\n'], " ")));
            Ok(())
        }))
        .map_err(w)?;
    player
        .MediaEnded(&TypedEventHandler::<MediaPlayer, IInspectable>::new(|_, _| {
            say("ended");
            Ok(())
        }))
        .map_err(w)?;
    let session = player.PlaybackSession().map_err(w)?;
    let playing = source.clone();
    session
        .PlaybackStateChanged(&TypedEventHandler::<MediaPlaybackSession, IInspectable>::new(move |session, _| {
            if let Some(session) = session.as_ref() {
                match session.PlaybackState()? {
                    MediaPlaybackState::Playing => {
                        say(&format!("playing {} {}", session.NaturalVideoWidth()?, session.NaturalVideoHeight()?));
                        // It only says when the rate changes, so this is the first anyone hears of it.
                        say(&format!("rate {}", playing.CurrentPlaybackBitrate()?));
                    }
                    MediaPlaybackState::Buffering => say("buffering"),
                    _ => {}
                }
            }
            Ok(())
        }))
        .map_err(w)?;
    player.SetSource(&MediaSource::CreateFromAdaptiveMediaSource(&source).map_err(w)?.cast::<IMediaPlaybackSource>().map_err(w)?).map_err(w)?;
    listen(&player, &source);
    let _ = player.Close();
    Ok(())
}

fn main() {
    pms_screen::started();
    if let Err(e) = run() {
        say(&format!("failed {e}"));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averaging_down_leaves_a_flat_picture_flat_and_opaque() {
        let (width, height, pitch) = (2usize, 1usize, 2 * FINE as usize * 4 + 8);
        let mut fine = vec![0u8; pitch * FINE as usize];
        for row in fine.chunks_exact_mut(pitch) {
            for dot in row[..2 * FINE as usize * 4].as_chunks_mut::<4>().0 {
                dot.copy_from_slice(&[10, 20, 30, 0]);
            }
        }
        let mut out = vec![0; width * height * 4];
        average(&fine, pitch, width, height, &mut out);
        assert_eq!(out, [10, 20, 30, 255, 10, 20, 30, 255]);
    }

    #[test]
    fn averaging_down_mixes_what_is_under_each_pixel() {
        let pitch = FINE as usize * 4;
        let mut fine = vec![0u8; pitch * FINE as usize];
        for row in fine.chunks_exact_mut(pitch) {
            row[..pitch / 2].fill(200);
        }
        let mut out = vec![0; 4];
        average(&fine, pitch, 1, 1, &mut out);
        assert_eq!(out, [100, 100, 100, 255]);
    }

    #[test]
    fn rows_read_back_lose_their_padding_and_come_out_opaque() {
        let from = [1, 2, 3, 0, 4, 5, 6, 0, 99, 99, 7, 8, 9, 0, 10, 11, 12, 0, 99, 99];
        let mut out = vec![0; 16];
        copy_rows(&from, 10, 2, 2, &mut out);
        assert_eq!(out, [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]);
    }

    /// How many of `offered` frames a second, for `secs` seconds, are taken at most `fps` a second.
    fn taken(offered: u32, fps: u32, secs: u32) -> usize {
        let (start, mut next) = (Instant::now(), None);
        let gap = std::time::Duration::from_secs(1) / offered;
        (0..offered * secs).filter(|i| due(start + gap * *i, &mut next, fps)).count()
    }

    #[test]
    fn the_frame_cap_halves_sixty_evenly_and_lets_thirty_through() {
        assert_eq!(taken(60, 30, 10), 300);
        assert_eq!(taken(30, 30, 10), 300);
        assert_eq!(taken(60, 0, 10), 600);
        // Three of every five.
        assert_eq!(taken(50, 30, 10), 300);
        assert_eq!(taken(60, 15, 10), 150);
        // Every other one, not two together and then a gap.
        let (start, mut next) = (Instant::now(), None);
        let gap = std::time::Duration::from_secs(1) / 60;
        let kept: Vec<bool> = (0..8).map(|i| due(start + gap * i, &mut next, 30)).collect();
        assert_eq!(kept, [true, false, true, false, true, false, true, false]);
    }

    #[test]
    fn the_frame_cap_starts_again_after_a_gap_rather_than_catching_up() {
        let (start, mut next) = (Instant::now(), None);
        assert!(due(start, &mut next, 30));
        // Nothing for a second, then 60 a second again: the first is taken, and the next not straight after it.
        let back = start + std::time::Duration::from_secs(1);
        assert!(due(back, &mut next, 30));
        assert!(!due(back + std::time::Duration::from_secs(1) / 60, &mut next, 30));
    }
}
