//! Plays a live stream onto a screen, with Windows' own media stack: `pms-twatch <master playlist> <screen> <cap> <floor>`, the last
//! two as below, which it also starts at. A process of its own so that a stream it can't cope with is nobody else's problem, and so
//! that the daemon stays small.
//!
//! The video never reaches a window. Each frame is scaled on the GPU to a few times the size the screen wants, read back, and
//! averaged down to size, which keeps a picture this small from shimmering; the audio plays only if it's asked to.
//!
//! The daemon talks to it on stdin, a line at a time, and when stdin closes it stops:
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
//!
//! and it answers on stdout: `playing <width> <height>`, `buffering`, `rate <bits a second>`, `failed <why>`, `ended`.
//!
//! With PMS_TWATCH_PROXY set (a socks5:// or socks5h:// address), nothing goes over the network the way Windows' player would send
//! it: the playlists and the video come through the proxy, fetched here, and are handed to the player.

#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

use pms_screen::Screen;
use windows::Foundation::{IReference, PropertyValue, TypedEventHandler, Uri};
use windows::Graphics::DirectX::Direct3D11::IDirect3DSurface;
use windows::Media::Core::MediaSource;
use windows::Media::MediaPlaybackType;
use windows::Media::Playback::{IMediaPlaybackSource, MediaPlaybackSession, MediaPlaybackState, MediaPlayer, MediaPlayerFailedEventArgs};
use windows::Media::Streaming::Adaptive::{AdaptiveMediaSource, AdaptiveMediaSourceCreationStatus, AdaptiveMediaSourceDownloadRequestedEventArgs, AdaptiveMediaSourcePlaybackBitrateChangedEventArgs};
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGISurface;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11SurfaceFromDXGISurface;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows_core::{HSTRING, IInspectable, Interface};

type R<T> = Result<T, String>;

/// How many times finer than the screen a frame is read back, each way, before it's averaged down.
const FINE: u32 = 4;

fn say(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn w(e: windows_core::Error) -> String {
    e.to_string()
}

/// Where frames go at the size that's wanted: the texture the player draws on, and the one read back from.
struct Target {
    width: u32,
    height: u32,
    surface: IDirect3DSurface,
    drawn: ID3D11Texture2D,
    readable: ID3D11Texture2D,
    out: Vec<u8>,
}

struct Frames {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    target: Option<Target>,
    screen: Screen,
}

// The device is made for use from any thread, and the context is only ever used with the mutex around this held.
unsafe impl Send for Frames {}

fn texture(device: &ID3D11Device, width: u32, height: u32, readable: bool) -> R<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: if readable { D3D11_USAGE_STAGING } else { D3D11_USAGE_DEFAULT },
        BindFlags: if readable { 0 } else { (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32 },
        CPUAccessFlags: if readable { D3D11_CPU_ACCESS_READ.0 as u32 } else { 0 },
        MiscFlags: 0,
    };
    let mut made = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut made)) }.map_err(w)?;
    made.ok_or_else(|| "no texture".to_string())
}

fn target(device: &ID3D11Device, width: u32, height: u32) -> R<Target> {
    let (fine_w, fine_h) = (width * FINE, height * FINE);
    let drawn = texture(device, fine_w, fine_h, false)?;
    let readable = texture(device, fine_w, fine_h, true)?;
    let dxgi: IDXGISurface = drawn.cast().map_err(w)?;
    let surface: IDirect3DSurface = unsafe { CreateDirect3D11SurfaceFromDXGISurface(&dxgi) }.map_err(w)?.cast().map_err(w)?;
    Ok(Target { width, height, surface, drawn, readable, out: vec![0; (width * height * 4) as usize] })
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

impl Frames {
    fn take(&mut self, player: &MediaPlayer) -> R<()> {
        let Some((width, height)) = self.screen.wanted() else { return Ok(()) };
        if self.target.as_ref().is_none_or(|t| (t.width, t.height) != (width, height)) {
            self.target = Some(target(&self.device, width, height)?);
        }
        let t = self.target.as_mut().ok_or("no target")?;
        player.CopyFrameToVideoSurface(&t.surface).map_err(w)?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context.CopyResource(&t.readable, &t.drawn);
            self.context.Map(&t.readable, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).map_err(w)?;
            let pitch = mapped.RowPitch as usize;
            let bytes = std::slice::from_raw_parts(mapped.pData as *const u8, pitch * (height * FINE) as usize);
            average(bytes, pitch, width as usize, height as usize, &mut t.out);
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
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into())
}

/// `url`, through `agent`, from `offset` for `length` bytes if those are given.
fn fetch(agent: &ureq::Agent, url: &str, range: Option<(u64, Option<u64>)>) -> R<Vec<u8>> {
    let mut request = agent.get(url);
    if let Some((offset, length)) = range {
        let end = length.filter(|l| *l > 0).map_or(String::new(), |l| (offset + l - 1).to_string());
        request = request.header("Range", &format!("bytes={offset}-{end}"));
    }
    let response = request.call().map_err(|e| e.to_string())?;
    response.into_body().with_config().limit(64 << 20).read_to_vec().map_err(|e| e.to_string())
}

/// The bytes, as something WinRT can read.
fn buffer(bytes: &[u8]) -> R<windows::Storage::Streams::IBuffer> {
    let writer = DataWriter::new().map_err(w)?;
    writer.WriteBytes(bytes).map_err(w)?;
    writer.DetachBuffer().map_err(w)
}

/// The master playlist at `url`, fetched through `agent`, as the player's source. Everything it goes on to fetch, playlists and
/// video alike, is asked for by its DownloadRequested event, and fetched here, through `agent`, in its place.
fn source_by_proxy(url: &str, uri: &Uri, agent: &ureq::Agent) -> R<AdaptiveMediaSource> {
    let master = fetch(agent, url, None).map_err(|e| format!("the playlist, through the proxy: {e}"))?;
    let stream = InMemoryRandomAccessStream::new().map_err(w)?;
    stream.WriteAsync(&buffer(&master)?).map_err(w)?.join().map_err(w)?;
    stream.Seek(0).map_err(w)?;
    let made = AdaptiveMediaSource::CreateFromStreamAsync(&stream, uri, &HSTRING::from("application/vnd.apple.mpegurl")).map_err(w)?.join().map_err(w)?;
    let status = made.Status().map_err(w)?;
    if status != AdaptiveMediaSourceCreationStatus::Success {
        return Err(format!("the stream wouldn't open ({})", status.0));
    }
    let source = made.MediaSource().map_err(w)?;
    let agent = agent.clone();
    source
        .DownloadRequested(&TypedEventHandler::<AdaptiveMediaSource, AdaptiveMediaSourceDownloadRequestedEventArgs>::new(move |_, args| {
            let Some(args) = args.as_ref() else { return Ok(()) };
            let (deferral, result) = (args.GetDeferral()?, args.Result()?);
            let url = args.ResourceUri()?.AbsoluteUri()?.to_string();
            let offset = args.ResourceByteRangeOffset().ok().and_then(|r| r.Value().ok());
            let length = args.ResourceByteRangeLength().ok().and_then(|r| r.Value().ok());
            let agent = agent.clone();
            std::thread::spawn(move || {
                let done = fetch(&agent, &url, offset.map(|o| (o, length))).and_then(|bytes| buffer(&bytes)).and_then(|b| result.SetBuffer(&b).map_err(w));
                if let Err(e) = done {
                    say(&format!("note fetch: {e}"));
                    // A failure as Windows would have had one: it tries again, or gives up, as it would.
                    let _ = result.SetExtendedStatus(0x8007_2EFD_u32 as i32 as u32);
                }
                let _ = deferral.Complete();
            });
            Ok(())
        }))
        .map_err(w)?;
    Ok(source)
}

fn run() -> R<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [url, name, cap, floor] = args.as_slice() else { return Err("usage: pms-twatch <master playlist> <screen> <cap> <floor>".into()) };
    let (cap, floor): (u32, u32) = (cap.parse().unwrap_or(0), floor.parse().unwrap_or(0));
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.map_err(w)?;
    let screen = Screen::create(name)?;
    let (device, context) = device()?;
    let frames = Arc::new(Mutex::new(Frames { device, context, target: None, screen }));

    let uri = Uri::CreateUri(&HSTRING::from(url.as_str())).map_err(w)?;
    let source = match std::env::var("PMS_TWATCH_PROXY").ok().filter(|p| !p.is_empty()) {
        Some(proxy) => source_by_proxy(url, &uri, &proxied(&proxy)?)?,
        None => {
            let made = AdaptiveMediaSource::CreateFromUriAsync(&uri).map_err(w)?.join().map_err(w)?;
            let status = made.Status().map_err(w)?;
            if status != AdaptiveMediaSourceCreationStatus::Success {
                return Err(format!("the stream wouldn't open ({})", status.0));
            }
            made.MediaSource().map_err(w)?
        }
    };
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
            for dot in row[..2 * FINE as usize * 4].chunks_exact_mut(4) {
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
}
