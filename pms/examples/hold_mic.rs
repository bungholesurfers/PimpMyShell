//! Hold the default microphone open for a while, to test the hot-mic alert.
//!
//!   cargo run --release --example hold_mic -- [seconds] [mute]
//!
//! `mute` also mutes the default microphone meanwhile (restoring its setting afterwards).

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree};

fn main() -> windows_core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let secs: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(10);
    let mute = args.iter().any(|a| a == "mute");
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let mic = devices.GetDefaultAudioEndpoint(eCapture, eConsole)?;
        let volume: IAudioEndpointVolume = mic.Activate(CLSCTX_ALL, None)?;
        let was_muted = volume.GetMute()?.as_bool();
        let client: IAudioClient = mic.Activate(CLSCTX_ALL, None)?;
        let format = client.GetMixFormat()?;
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 10_000_000, 0, format, None)?;
        CoTaskMemFree(Some(format as _));
        client.Start()?;
        println!("microphone open for {secs}s");
        if mute {
            std::thread::sleep(std::time::Duration::from_secs(secs / 2));
            volume.SetMute(true, std::ptr::null())?;
            println!("muted");
            std::thread::sleep(std::time::Duration::from_secs(secs - secs / 2));
            volume.SetMute(was_muted, std::ptr::null())?;
        } else {
            std::thread::sleep(std::time::Duration::from_secs(secs));
        }
        client.Stop()?;
    }
    println!("released");
    Ok(())
}
