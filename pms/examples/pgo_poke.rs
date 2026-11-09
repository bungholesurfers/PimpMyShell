//! What PGO training (`cargo xtask pgo`) does to PimpMyShell, as somebody would: fetches a game, presses a TV's remote, clicks a
//! screen, plays in POOM's window.
//!
//!   pgo_poke wad                       fetches Freedoom into the console's home (PMS_POOM_HOME), through your proxy if you
//!                                      have one, and chooses it
//!   pgo_poke press <screen> <button>   a remote's button: up down shuffle power game-power game
//!   pgo_poke click <screen>            a click on the picture (on the console, `poom`, that opens its window)
//!   pgo_poke keys <key[:ms]>..         keys for POOM's window, each held `ms` (120 if not said): esc enter up down left right
//!                                      fire use tab, or wait:ms
//!   pgo_poke close                     closes POOM's window, as its close button does

use std::time::Duration;

use pms_screen::{Button, Screen};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_CLOSE, WM_KEYDOWN, WM_KEYUP};
use windows_core::w;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    let result = match arg(0) {
        Some("wad") => wad(),
        Some("press") => match (arg(1), arg(2).and_then(button)) {
            (Some(name), Some(b)) => Screen::open(name).map(|s| s.press(b)),
            _ => Err("press <screen> <button>".into()),
        },
        Some("click") => match arg(1) {
            Some(name) => Screen::open(name).map(|s| {
                s.click();
            }),
            None => Err("click <screen>".into()),
        },
        Some("keys") => keys(&args[1..]),
        Some("close") => poom().map(|h| drop(unsafe { PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)) })),
        _ => Err("pgo_poke wad | press <screen> <button> | click <screen> | keys <key[:ms]>.. | close".into()),
    };
    if let Err(e) = result {
        eprintln!("pgo_poke: {e}");
        std::process::exit(1);
    }
}

/// Freedoom, fetched as the settings fetch it: through the proxy your network settings ask for, or nowhere when they don't load.
fn wad() -> Result<(), String> {
    let mine = pms::config::default_path();
    if mine.is_file() {
        pms::net::follow(&pms::config::load(&mine).map_err(|e| format!("your settings didn't load, so nothing's fetched: {e}"))?);
    }
    let last = std::cell::Cell::new(0);
    let path = pms::wads::get(pms::wads::Wad::Freedoom, &|got, of| {
        let quarter = got * 4 / of.max(1);
        if quarter > last.replace(quarter) {
            println!("Freedoom: {}%", quarter * 25);
        }
    })?;
    pms::wads::choose("freedoom1.wad")?;
    println!("Freedoom: {}", path.display());
    Ok(())
}

fn button(name: &str) -> Option<Button> {
    Some(match name {
        "up" => Button::ChannelUp,
        "down" => Button::ChannelDown,
        "shuffle" => Button::Shuffle,
        "power" => Button::TvPower,
        "game-power" => Button::GamePower,
        "game" => Button::GameChannel,
        _ => return None,
    })
}

fn poom() -> Result<HWND, String> {
    unsafe { FindWindowW(w!("PimpMyShellPoom"), None) }.map_err(|_| "POOM's window isn't open".into())
}

/// Each key down, held, and up, posted to POOM's window: it reads them from its messages, focus or none.
fn keys(keys: &[String]) -> Result<(), String> {
    let window = poom()?;
    for key in keys {
        let (name, ms) = key.split_once(':').map_or((key.as_str(), 120), |(k, ms)| (k, ms.parse().unwrap_or(120)));
        let vk: usize = match name {
            "wait" => {
                std::thread::sleep(Duration::from_millis(ms));
                continue;
            }
            "esc" => 0x1B,
            "enter" => 0x0D,
            "left" => 0x25,
            "up" => 0x26,
            "right" => 0x27,
            "down" => 0x28,
            "fire" => 0x11,
            "use" => 0x20,
            "tab" => 0x09,
            other => return Err(format!("no key `{other}`")),
        };
        unsafe { PostMessageW(Some(window), WM_KEYDOWN, WPARAM(vk), LPARAM(1)) }.map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(ms));
        // An up has bits 30 and 31 set: it was down, and it's going up.
        unsafe { PostMessageW(Some(window), WM_KEYUP, WPARAM(vk), LPARAM(0xC000_0001u32 as isize)) }.map_err(|e| e.to_string())?;
        // The menu reads a key a tic: give each a few.
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}
