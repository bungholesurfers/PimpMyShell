//! Read or set the main display's scaling (100%, 125%, 150%, ...), live, the way Settings does.
//!
//!   cargo run --release --example set_scale -- [percent]
//!
//! Uses the undocumented DisplayConfig "source DPI scale" requests (-3 get, -4 set), which take
//! the scale as steps relative to the display's recommended scale.

use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::LUID;

const STEPS: [u32; 12] = [100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500];

#[repr(C)]
struct Header {
    kind: i32,
    size: u32,
    adapter: LUID,
    id: u32,
}

#[repr(C)]
struct GetScale {
    header: Header,
    min_rel: i32,
    cur_rel: i32,
    max_rel: i32,
}

#[repr(C)]
struct SetScale {
    header: Header,
    rel: i32,
}

fn main() {
    let (mut np, mut nm) = (0u32, 0u32);
    unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) }.ok().expect("buffer sizes");
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
    unsafe { QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None) }.ok().expect("query");
    let src = paths[0].sourceInfo;
    let header = |kind: i32, size: usize| Header { kind, size: size as u32, adapter: src.adapterId, id: src.id };

    let mut get = GetScale { header: header(-3, size_of::<GetScale>()), min_rel: 0, cur_rel: 0, max_rel: 0 };
    let r = unsafe { DisplayConfigGetDeviceInfo(&mut get.header as *mut Header as *mut DISPLAYCONFIG_DEVICE_INFO_HEADER) };
    assert_eq!(r, 0, "get scale failed ({r})");
    // min_rel is how many steps below the recommended scale the smallest (100%) is.
    let recommended = (-get.min_rel) as usize;
    let current = STEPS[(recommended as i32 + get.cur_rel) as usize];
    println!("current {current}% (recommended {}%)", STEPS[recommended]);

    if let Some(want) = std::env::args().nth(1).and_then(|a| a.trim_end_matches('%').parse::<u32>().ok()) {
        let idx = STEPS.iter().position(|&s| s == want).expect("one of 100,125,150,...") as i32;
        let rel = idx - recommended as i32;
        assert!(rel >= get.min_rel && rel <= get.max_rel, "{want}% is outside this display's range");
        let mut set = SetScale { header: header(-4, size_of::<SetScale>()), rel };
        let r = unsafe { DisplayConfigSetDeviceInfo(&mut set.header as *mut Header as *mut DISPLAYCONFIG_DEVICE_INFO_HEADER) };
        assert_eq!(r, 0, "set scale failed ({r})");
        println!("set {want}%");
    }
}
