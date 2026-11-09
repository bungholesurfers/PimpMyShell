//! A live picture for the taskbar, in the manner of WindowMaker's dockapps: a program draws frames, and a `screen` meter
//! shows them. The frames cross between processes in a block of shared memory, so that what draws them can't take Explorer
//! down with it, and so that a licence of its own stays its own.
//!
//! Three slots: frame `n` lies in slot `n % 3`. The writer fills the next slot and only then moves `seq` on, so a reader
//! always finds a finished frame, and neither ever waits for the other. An event says there's a new frame. No system call
//! is needed to read one, and at taskbar size a frame is a few KB. The reader says what size it wants in the header; the
//! writer draws that size, so nothing is scaled on Explorer's UI thread.
//!
//! Pixels are BGRA, premultiplied, top row first: what a `WriteableBitmap` holds.

mod logo;

pub use logo::{LOGO_BLUE, LOGO_NAME, logo};

use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows::Win32::System::Memory::{CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW, PAGE_READWRITE, UnmapViewOfFile};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows_core::HSTRING;

/// The biggest frame, either way: a taskbar at 350% is about 170 pixels tall.
pub const MAX_SIDE: u32 = 256;
const SLOTS: usize = 3;
const SLOT_BYTES: usize = (MAX_SIDE * MAX_SIDE * 4) as usize;
const PIXELS_AT: usize = 4096;
const BYTES: usize = PIXELS_AT + SLOTS * SLOT_BYTES;
const MAGIC: u32 = u32::from_le_bytes(*b"PMS4");
/// As much as a screen can say about itself, in the room before the pixels.
const INFO_BYTES: usize = 3072;

#[repr(C)]
struct Header {
    magic: AtomicU32,
    /// How many frames are finished.
    seq: AtomicU64,
    /// What the reader would like, in pixels.
    wanted: [AtomicU32; 2],
    /// Each slot's frame, in pixels.
    sizes: [[AtomicU32; 2]; SLOTS],
    /// How many times the screen has been clicked.
    clicks: AtomicU32,
    /// The process drawing it, for the reader to let it come to the foreground.
    writer: AtomicU32,
    /// Another that may need to, the game behind a TV.
    also: AtomicU32,
    /// How many times each button (`Button`) has been pressed.
    presses: [AtomicU32; 24],
    /// What's on, as `Status` packs it, for whatever shows the buttons.
    status: AtomicU32,
    /// When a reader last looked, in milliseconds since 1970: a game with nobody watching can rest.
    watched: AtomicU64,
    /// A channel asked for from the list, until the TV has taken it.
    wanted_channel: AtomicU32,
    /// How long `info` is, and what the writer has to say, for whatever shows the buttons.
    info_len: AtomicU32,
    info: [AtomicU8; INFO_BYTES],
}

/// The buttons on a screen's remote.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Button {
    ChannelUp,
    ChannelDown,
    Shuffle,
    TvPower,
    GamePower,
    /// Straight to the console's channel.
    GameChannel,
    /// Sound on or off, on a set that has any.
    Mute,
    /// One more set on the taskbar.
    AddTv,
    /// A Twatch TV goes by your favorites.
    Follow,
    /// It stays on this channel until that ends.
    Stay,
    /// It's only ever this channel.
    Only,
    /// The highest quality for every Twatch TV.
    QualityUp,
    QualityDown,
    /// How long every TV stays on a channel while shuffling.
    ShuffleSooner,
    ShuffleLater,
    /// Whether shuffling TVs each go a little late, or all together.
    Jitter,
    /// This TV goes from the taskbar.
    NukeTv,
    /// How loud every Twatch TV is.
    VolumeUp,
    VolumeDown,
    /// Another channel for the favorites: the settings, where they're kept.
    AddFavorite,
}

/// The channels there are, and what a remote calls them.
pub const CHANNELS: [(u32, &str); 6] = [(2, "TEST CARD"), (3, "CRAB"), (4, "STATIC"), (5, "PIMPCAST"), (6, "POOM"), (1295, "MIDNIGHT")];
pub const GAME_CHANNEL: u32 = 6;

pub const BUTTONS: [Button; 20] = [
    Button::ChannelUp,
    Button::ChannelDown,
    Button::Shuffle,
    Button::TvPower,
    Button::GamePower,
    Button::GameChannel,
    Button::Mute,
    Button::AddTv,
    Button::Follow,
    Button::Stay,
    Button::Only,
    Button::QualityUp,
    Button::QualityDown,
    Button::ShuffleSooner,
    Button::ShuffleLater,
    Button::Jitter,
    Button::NukeTv,
    Button::VolumeUp,
    Button::VolumeDown,
    Button::AddFavorite,
];

/// For what draws a screen, first thing: Windows keeps the pointer busy for seconds after a program starts, until it asks for its
/// first message, and one without a window would be a while. Asking ends that.
pub fn started() {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_NULL};
    let mut message = MSG::default();
    unsafe {
        // Only a thread that's peeked has a queue to post to.
        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
        if PostThreadMessageW(GetCurrentThreadId(), WM_NULL, WPARAM(0), LPARAM(0)).is_ok() {
            let _ = GetMessageW(&mut message, None, 0, 0);
        }
    }
}

/// What a screen is doing.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Status {
    pub channel: u32,
    pub tv_on: bool,
    pub shuffle: bool,
    /// A game is ready to play, and whether its console is on.
    pub has_game: bool,
    pub game_on: bool,
    /// Off for a while, and put away: it's only a small button now.
    pub collapsed: bool,
    /// One of the newer sets, with a remote to match.
    pub modern: bool,
    /// Its sound is off.
    pub muted: bool,
    /// It only ever shows one channel.
    pub only: bool,
    /// Not a set but a button beside one: all there is to say about it is in its `info`.
    pub note: bool,
    /// Shuffling TVs each go a little late, instead of all together.
    pub jitter: bool,
}

impl Status {
    fn pack(self) -> u32 {
        let flags = [self.tv_on, self.shuffle, self.has_game, self.game_on, self.collapsed, self.modern, self.muted, self.only, self.note, self.jitter];
        flags.iter().enumerate().fold(self.channel << 10, |v, (i, on)| v | u32::from(*on) << i)
    }

    fn unpack(v: u32) -> Status {
        let on = |bit: u32| v & 1 << bit != 0;
        Status { channel: v >> 10, tv_on: on(0), shuffle: on(1), has_game: on(2), game_on: on(3), collapsed: on(4), modern: on(5), muted: on(6), only: on(7), note: on(8), jitter: on(9) }
    }
}

/// Milliseconds since 1970: a clock every process and thread reads the same.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// One screen, seen from either side.
pub struct Screen {
    mapping: HANDLE,
    view: *mut u8,
    event: HANDLE,
}

// The memory is shared by design: every field of the header is atomic, and a slot is only written while it isn't the newest.
unsafe impl Send for Screen {}
unsafe impl Sync for Screen {}

fn names(name: &str) -> (HSTRING, HSTRING) {
    (HSTRING::from(format!("Local\\PimpMyShell.Screen.{name}")), HSTRING::from(format!("Local\\PimpMyShell.Screen.{name}.Frame")))
}

impl Screen {
    /// The screen called `name`, for drawing on: made if it isn't there yet.
    pub fn create(name: &str) -> Result<Screen, String> {
        let (mapping, _) = names(name);
        let handle = unsafe { CreateFileMappingW(INVALID_HANDLE_VALUE, None, PAGE_READWRITE, 0, BYTES as u32, &mapping) }.map_err(|e| format!("{name}: {e}"))?;
        let screen = Screen::mapped(name, handle)?;
        screen.header().writer.store(std::process::id(), Ordering::Relaxed);
        screen.header().magic.store(MAGIC, Ordering::Release);
        Ok(screen)
    }

    /// The screen called `name`, for showing: an error until something has made it.
    pub fn open(name: &str) -> Result<Screen, String> {
        let (mapping, _) = names(name);
        let handle = unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS.0, false, &mapping) }.map_err(|e| format!("{name}: {e}"))?;
        let screen = Screen::mapped(name, handle)?;
        if screen.header().magic.load(Ordering::Acquire) != MAGIC {
            return Err(format!("{name}: not a screen"));
        }
        Ok(screen)
    }

    fn mapped(name: &str, mapping: HANDLE) -> Result<Screen, String> {
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, BYTES) };
        if view.Value.is_null() {
            let _ = unsafe { CloseHandle(mapping) };
            return Err(format!("{name}: couldn't map it"));
        }
        let (_, event) = names(name);
        let event = match unsafe { CreateEventW(None, false, false, &event) } {
            Ok(e) => e,
            Err(e) => {
                let _ = unsafe { UnmapViewOfFile(view) };
                let _ = unsafe { CloseHandle(mapping) };
                return Err(format!("{name}: {e}"));
            }
        };
        Ok(Screen { mapping, view: view.Value as *mut u8, event })
    }

    fn header(&self) -> &Header {
        unsafe { &*(self.view as *const Header) }
    }

    fn slot(&self, n: u64) -> *mut u8 {
        unsafe { self.view.add(PIXELS_AT + (n % SLOTS as u64) as usize * SLOT_BYTES) }
    }

    /// Asks for frames of `width` by `height` pixels.
    pub fn want(&self, width: u32, height: u32) {
        let h = self.header();
        h.wanted[0].store(width.min(MAX_SIDE), Ordering::Relaxed);
        h.wanted[1].store(height.min(MAX_SIDE), Ordering::Relaxed);
    }

    /// The size of frame that's wanted, if one has been asked for.
    pub fn wanted(&self) -> Option<(u32, u32)> {
        let h = self.header();
        let size = (h.wanted[0].load(Ordering::Relaxed), h.wanted[1].load(Ordering::Relaxed));
        (size.0 > 0 && size.1 > 0).then_some(size)
    }

    /// Puts up a frame of `width` by `height` pixels. False if it's too big, or `pixels` is the wrong length.
    pub fn publish(&self, width: u32, height: u32, pixels: &[u8]) -> bool {
        if width > MAX_SIDE || height > MAX_SIDE || pixels.len() != (width * height * 4) as usize {
            return false;
        }
        let h = self.header();
        let next = h.seq.load(Ordering::Relaxed) + 1;
        unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), self.slot(next), pixels.len()) };
        let size = &h.sizes[(next % SLOTS as u64) as usize];
        size[0].store(width, Ordering::Relaxed);
        size[1].store(height, Ordering::Relaxed);
        h.seq.store(next, Ordering::Release);
        let _ = unsafe { SetEvent(self.event) };
        true
    }

    /// Copies the newest frame into `out`, if it's newer than `after` and of `width` by `height` pixels. Returns its
    /// number. A frame of another size is one drawn before the size was asked for: it's skipped.
    pub fn latest(&self, after: u64, width: u32, height: u32, out: &mut [u8]) -> Option<u64> {
        let h = self.header();
        let n = h.seq.load(Ordering::Acquire);
        let size = &h.sizes[(n % SLOTS as u64) as usize];
        let fits = size[0].load(Ordering::Relaxed) == width && size[1].load(Ordering::Relaxed) == height;
        if n == after || !fits || out.len() != (width * height * 4) as usize {
            return None;
        }
        unsafe { std::ptr::copy_nonoverlapping(self.slot(n), out.as_mut_ptr(), out.len()) };
        Some(n)
    }

    /// Says the screen was clicked. Returns the process drawing it.
    pub fn click(&self) -> u32 {
        let h = self.header();
        h.clicks.fetch_add(1, Ordering::Relaxed);
        h.writer.load(Ordering::Relaxed)
    }

    /// How many times the screen has been clicked.
    pub fn clicks(&self) -> u32 {
        self.header().clicks.load(Ordering::Relaxed)
    }

    /// Asks for a channel.
    pub fn tune(&self, channel: u32) {
        self.header().wanted_channel.store(channel, Ordering::Relaxed);
    }

    /// The channel asked for, if one has been since the last time this was called.
    pub fn take_tune(&self) -> Option<u32> {
        Some(self.header().wanted_channel.swap(0, Ordering::Relaxed)).filter(|c| *c != 0)
    }

    /// Says `button` was pressed.
    pub fn press(&self, button: Button) {
        self.header().presses[button as usize].fetch_add(1, Ordering::Relaxed);
    }

    /// How many times `button` has been pressed.
    pub fn presses(&self, button: Button) -> u32 {
        self.header().presses[button as usize].load(Ordering::Relaxed)
    }

    pub fn set_status(&self, status: Status) {
        self.header().status.store(status.pack(), Ordering::Relaxed);
    }

    pub fn status(&self) -> Status {
        Status::unpack(self.header().status.load(Ordering::Relaxed))
    }

    /// Says something about itself: the first `INFO_BYTES` of `text`. A reader catching it halfway only sees it wrong until it
    /// next looks, which is all this is for.
    pub fn set_info(&self, text: &str) {
        let h = self.header();
        let bytes = &text.as_bytes()[..text.len().min(INFO_BYTES)];
        for (slot, byte) in h.info.iter().zip(bytes) {
            slot.store(*byte, Ordering::Relaxed);
        }
        h.info_len.store(bytes.len() as u32, Ordering::Release);
    }

    pub fn info(&self) -> String {
        let h = self.header();
        let n = (h.info_len.load(Ordering::Acquire) as usize).min(INFO_BYTES);
        let bytes: Vec<u8> = h.info[..n].iter().map(|b| b.load(Ordering::Relaxed)).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Names another process that may come to the foreground on a click.
    pub fn set_also(&self, process: u32) {
        self.header().also.store(process, Ordering::Relaxed);
    }

    pub fn also(&self) -> u32 {
        self.header().also.load(Ordering::Relaxed)
    }

    /// Says someone's watching.
    pub fn watch(&self) {
        self.header().watched.store(now_ms(), Ordering::Relaxed);
    }

    /// Whether anyone has been, in the last `ms`.
    pub fn watched_within(&self, ms: u64) -> bool {
        now_ms().saturating_sub(self.header().watched.load(Ordering::Relaxed)) < ms
    }

    /// The process drawing it.
    pub fn writer(&self) -> u32 {
        self.header().writer.load(Ordering::Relaxed)
    }

    /// Waits up to `ms` for a new frame.
    pub fn wait(&self, ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.event, ms) == WAIT_OBJECT_0 }
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = unsafe { UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: self.view as *mut _ }) };
        let _ = unsafe { CloseHandle(self.mapping) };
        let _ = unsafe { CloseHandle(self.event) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique(name: &str) -> String {
        format!("test-{name}-{}", std::process::id())
    }

    #[test]
    fn nothing_to_open_until_its_made() {
        assert!(Screen::open(&unique("none")).is_err());
    }

    #[test]
    fn a_frame_goes_across_and_the_size_comes_back() {
        let name = unique("across");
        let writer = Screen::create(&name).unwrap();
        let reader = Screen::open(&name).unwrap();
        assert_eq!(writer.wanted(), None);
        reader.want(2, 1);
        assert_eq!(writer.wanted(), Some((2, 1)));
        assert!(writer.publish(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]));
        assert!(reader.wait(1000));
        let mut out = [0u8; 8];
        let n = reader.latest(0, 2, 1, &mut out).unwrap();
        assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(reader.latest(n, 2, 1, &mut out), None, "nothing new");
    }

    #[test]
    fn buttons_status_and_watching_cross_over() {
        let name = unique("remote");
        let writer = Screen::create(&name).unwrap();
        let reader = Screen::open(&name).unwrap();
        assert_eq!(writer.presses(Button::Shuffle), 0);
        reader.press(Button::Shuffle);
        assert_eq!(writer.presses(Button::Shuffle), 1);
        assert_eq!(writer.presses(Button::ChannelUp), 0);
        let status = Status { channel: 1295, tv_on: true, shuffle: true, has_game: false, game_on: true, collapsed: false, modern: true, muted: true, only: true, note: true, jitter: true };
        writer.set_status(status);
        assert_eq!(reader.status(), status);
        assert_eq!(reader.info(), "");
        writer.set_info("LIVE\tcrab");
        assert_eq!(reader.info(), "LIVE\tcrab");
        writer.set_info(&"x".repeat(INFO_BYTES + 5));
        assert_eq!(reader.info().len(), INFO_BYTES, "it's cut, not refused");
        assert!(!writer.watched_within(10_000));
        reader.watch();
        assert!(writer.watched_within(10_000));
        writer.set_also(42);
        assert_eq!(reader.also(), 42);
    }

    #[test]
    fn a_click_reaches_the_writer() {
        let name = unique("click");
        let writer = Screen::create(&name).unwrap();
        let reader = Screen::open(&name).unwrap();
        assert_eq!(writer.clicks(), 0);
        assert_eq!(reader.click(), std::process::id());
        assert_eq!(writer.clicks(), 1);
    }

    #[test]
    fn a_reader_always_gets_the_newest() {
        let name = unique("newest");
        let writer = Screen::create(&name).unwrap();
        let reader = Screen::open(&name).unwrap();
        for i in 0..10u8 {
            writer.publish(1, 1, &[i; 4]);
        }
        let mut out = [0u8; 4];
        assert_eq!(reader.latest(0, 1, 1, &mut out), Some(10));
        assert_eq!(out, [9; 4]);
    }

    #[test]
    fn what_doesnt_fit_is_refused() {
        let writer = Screen::create(&unique("big")).unwrap();
        assert!(!writer.publish(MAX_SIDE + 1, 1, &[0; (MAX_SIDE as usize + 1) * 4]));
        assert!(!writer.publish(2, 2, &[0; 4]));
        writer.publish(1, 1, &[0; 4]);
        let mut out = [0u8; 16];
        assert_eq!(writer.latest(0, 2, 2, &mut out), None, "a frame of another size is skipped");
    }
}
