//! What goes between the console and the game it's playing, which is a process of its own (so that the console's still
//! there, and its window with it, when the game ends): the game's pictures one way, in memory they share, and what's done
//! at the window the other, down a pipe.

use std::sync::atomic::{AtomicU64, Ordering};

use room::doom::doomgeneric::{DOOMGENERIC_RESX, DOOMGENERIC_RESY};
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{CreateFileMappingW, FILE_MAP_ALL_ACCESS, MapViewOfFile, PAGE_READWRITE, UnmapViewOfFile};
use windows_core::HSTRING;

/// The size of the game's picture.
pub const ACROSS: usize = DOOMGENERIC_RESX;
pub const DOWN: usize = DOOMGENERIC_RESY;

const PICTURE: usize = ACROSS * DOWN * 4;
const PICTURES_AT: usize = 64;
/// Two, so that the newest is never the one being drawn over.
const BYTES: usize = PICTURES_AT + 2 * PICTURE;

/// The game's pictures, seen from either side.
pub struct Pictures {
    mapping: HANDLE,
    view: *mut u8,
}

impl Pictures {
    /// The pictures called `name`: made, if they aren't there yet.
    pub fn open(name: &str) -> Result<Pictures, String> {
        let mapping = unsafe { CreateFileMappingW(INVALID_HANDLE_VALUE, None, PAGE_READWRITE, 0, BYTES as u32, &HSTRING::from(name)) }.map_err(|e| format!("{name}: {e}"))?;
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, BYTES) };
        if view.Value.is_null() {
            let _ = unsafe { CloseHandle(mapping) };
            return Err(format!("{name}: couldn't map it"));
        }
        Ok(Pictures { mapping, view: view.Value.cast() })
    }

    /// How many pictures have been put up.
    fn count(&self) -> &AtomicU64 {
        unsafe { &*self.view.cast::<AtomicU64>() }
    }

    fn slot(&self, n: u64) -> *mut u8 {
        unsafe { self.view.add(PICTURES_AT + (n % 2) as usize * PICTURE) }
    }

    /// Puts up a picture: `ACROSS` by `DOWN` pixels.
    pub fn put(&self, pixels: &[u32]) {
        assert_eq!(pixels.len() * 4, PICTURE);
        let next = self.count().load(Ordering::Relaxed) + 1;
        unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr().cast::<u8>(), self.slot(next), PICTURE) };
        self.count().store(next, Ordering::Release);
    }

    /// The newest picture into `out`, if it's newer than `after`: which it is.
    pub fn newest(&self, after: u64, out: &mut Vec<u32>) -> Option<u64> {
        let n = self.count().load(Ordering::Acquire);
        if n == after || n == 0 {
            return None;
        }
        out.resize(ACROSS * DOWN, 0);
        unsafe { std::ptr::copy_nonoverlapping(self.slot(n), out.as_mut_ptr().cast::<u8>(), PICTURE) };
        Some(n)
    }
}

impl Drop for Pictures {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS { Value: self.view.cast() });
            let _ = CloseHandle(self.mapping);
        }
    }
}

/// What the console tells the game.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Said {
    /// A key, by the game's own number for it, going down or coming up.
    Key { down: bool, key: u8 },
    /// The mouse: which buttons are down, and how far it's gone across since it last said.
    Mouse { buttons: i32, across: i32 },
    /// The window's been opened, or closed: closing it pauses a game in progress, and opening it carries on.
    Opened,
    Closed,
    /// Nobody's watching, or somebody is again: with nobody watching the game stands still.
    Rest,
    Wake,
}

impl Said {
    /// As it goes down the pipe: eight bytes.
    pub fn pack(self) -> [u8; 8] {
        let (kind, a, x) = match self {
            Said::Key { down, key } => (1, u8::from(down), i32::from(key)),
            Said::Mouse { buttons, across } => (2, buttons as u8, across),
            Said::Opened => (3, 0, 0),
            Said::Closed => (4, 0, 0),
            Said::Rest => (5, 0, 0),
            Said::Wake => (6, 0, 0),
        };
        let x = x.to_le_bytes();
        [kind, a, 0, 0, x[0], x[1], x[2], x[3]]
    }

    pub fn unpack(b: [u8; 8]) -> Option<Said> {
        let x = i32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        Some(match b[0] {
            1 => Said::Key { down: b[1] != 0, key: x as u8 },
            2 => Said::Mouse { buttons: i32::from(b[1]), across: x },
            3 => Said::Opened,
            4 => Said::Closed,
            5 => Said::Rest,
            6 => Said::Wake,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_said_arrives_as_it_was_said() {
        for said in [Said::Key { down: true, key: 173 }, Said::Key { down: false, key: 27 }, Said::Mouse { buttons: 5, across: -1234 }, Said::Opened, Said::Closed, Said::Rest, Said::Wake] {
            assert_eq!(Said::unpack(said.pack()), Some(said));
        }
        assert_eq!(Said::unpack([0; 8]), None);
    }

    #[test]
    fn the_newest_picture_is_the_one_seen() {
        let name = format!("Local\\PimpMyShell.Pimpbox.Test.{}", std::process::id());
        let (game, console) = (Pictures::open(&name).unwrap(), Pictures::open(&name).unwrap());
        let mut seen = Vec::new();
        assert_eq!(console.newest(0, &mut seen), None, "none yet");
        game.put(&vec![7; ACROSS * DOWN]);
        game.put(&vec![9; ACROSS * DOWN]);
        assert_eq!(console.newest(0, &mut seen), Some(2));
        assert!(seen.iter().all(|p| *p == 9));
        assert_eq!(console.newest(2, &mut seen), None, "and none newer");
    }
}
