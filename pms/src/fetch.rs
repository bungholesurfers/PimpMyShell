//! What PimpMyShell fetches by way of pms-fetch.exe, a helper of its own, so that what comes from the internet (the HTTPS itself,
//! Twitch's preview pictures, decoded, playlists, downloads) is handled there rather than here. It's sandboxed when the settings
//! say (the strictest kind, and without windows or drawing too). Each process that fetches (the daemon, the settings) has its
//! own, started when first wanted.
//!
//! There are two kinds, each a process of its own, so that what doesn't go through the proxy never opens a way around it for what
//! does:
//! - **Through the proxy.** Sandboxed, it has no network of its own, and (the strictest kind) can't even listen on 127.0.0.1 for
//!   a relay, as the other helpers have: it asks here for each connection, on its pipe, and is handed a socket connected to the
//!   proxy, and only ever the proxy (relay::connected), which it asks for where it's going itself (its broker.rs).
//! - **Without one.** Sandboxed, it's given the internet itself (`internetClient`).
//!
//! Each is started again when its sandbox is switched on or off. Which proxy, and as whom, is said with each request.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::net::Use;
use crate::sandbox::Helper;

/// The program.
pub const FETCH: &str = "pms-fetch.exe";

/// What a fetch is to come back as.
#[derive(Clone, Copy)]
pub enum Kind {
    /// Only the status.
    Head,
    /// The bytes, up to the most asked for.
    Get,
    /// A JPEG, as 8-bit RGB.
    Jpeg,
    /// The bytes, up to the most asked for, saying how far it's got as it goes.
    Download,
    /// A zip's one demo (a .lmp), unzipped there, up to the most asked for.
    Lmp,
}

/// What came back: the HTTP status, the bytes, and for a picture its width and height. For a redirect that wasn't followed, the
/// bytes are where it goes.
pub struct Got {
    pub status: u16,
    pub body: Vec<u8>,
    pub width: usize,
    pub height: usize,
}

/// What a failure begins with when it's what was fetched that's at fault (a zip with no demo in it, say), not the fetching: it
/// would be the same every time. pms-fetch's own `UNUSABLE`.
pub const UNUSABLE: &str = "unusable: ";

/// Every use that's fetched by way of it: any of them can be the one whose proxy a socket's connected to.
const USES: [Use; 5] = [Use::Live, Use::Video, Use::Downloads, Use::Updates, Use::Iss];

/// What's heard about a fetch: how far a download's got, and then how it went.
enum Heard {
    Progress(u64),
    Done(Result<Got, String>),
}

type Waiting = HashMap<u64, Sender<Heard>>;

/// A helper that's running.
struct Running {
    child: pms_spawn::Child,
    to: Arc<Mutex<File>>,
    waiting: Arc<Mutex<Waiting>>,
    /// Whether it was started sandboxed.
    sandboxed: bool,
}

/// The two: [without a proxy, through one].
static RUNNING: Mutex<[Option<Running>; 2]> = Mutex::new([None, None]);
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Fetches `url` for `what`, as `kind`: no more than `most` bytes, in no more than `patience`, after no more than `redirects`.
pub fn fetch(what: Use, kind: Kind, url: &str, patience: Duration, redirects: u32, most: usize) -> Result<Got, String> {
    fetch_showing(what, None, kind, url, patience, redirects, most, &|_, _| {})
}

/// The same, as `session` goes through the proxy (net::route_as): for what one stream fetches, as that stream's somebody.
pub fn fetch_as(what: Use, session: &crate::net::Session, kind: Kind, url: &str, patience: Duration, redirects: u32, most: usize) -> Result<Got, String> {
    fetch_showing(what, Some(session), kind, url, patience, redirects, most, &|_, _| {})
}

/// Downloads `url` for `what`, `size` bytes at most, after no more than `redirects`, saying how far it's got (`progress(got,
/// size)`) as it goes. What comes back is whatever was served: checking it's what it should be is the caller's. A redirect beyond
/// those it may follow comes back as its status, with where it goes as the body.
pub fn download(what: Use, url: &str, size: u64, redirects: u32, progress: &dyn Fn(u64, u64)) -> Result<Got, String> {
    let most = usize::try_from(size).map_err(|_| "too big")?;
    fetch_showing(what, None, Kind::Download, url, Duration::from_secs(600), redirects, most, &|got, _| progress(got, size))
}

#[allow(clippy::too_many_arguments)]
fn fetch_showing(what: Use, session: Option<&crate::net::Session>, kind: Kind, url: &str, patience: Duration, redirects: u32, most: usize, progress: &dyn Fn(u64, u64)) -> Result<Got, String> {
    if url.contains([' ', '\r', '\n']) {
        return Err("an address with a space in it".into());
    }
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (back, heard) = channel();
    let route = crate::net::route_as(what, session);
    {
        let mut running = RUNNING.lock().map_err(|_| "the fetcher")?;
        let slot = &mut running[usize::from(route.is_some())];
        let sandboxed = crate::sandbox::wanted(Helper::Fetch);
        if slot.as_mut().is_some_and(|r| r.sandboxed != sandboxed || r.child.try_wait().is_some()) {
            stop(slot);
        }
        if slot.is_none() {
            *slot = Some(start(sandboxed, route.is_some())?);
        }
        let r = slot.as_mut().ok_or("the fetcher")?;
        let kind = match kind {
            Kind::Head => "head",
            Kind::Get => "get",
            Kind::Jpeg => "jpeg",
            Kind::Download => "download",
            Kind::Lmp => "lmp",
        };
        r.waiting.lock().map_err(|_| "the fetcher")?.insert(id, back);
        let line = format!("{id} {kind} {} {redirects} {most} {} {url}\n", patience.as_secs().max(1), route.as_deref().unwrap_or("-"));
        let written = r.to.lock().map_err(|_| std::io::Error::other("the pipe")).and_then(|mut to| to.write_all(line.as_bytes()).and_then(|()| to.flush()));
        if let Err(e) = written {
            stop(slot);
            return Err(format!("the fetcher: {e}"));
        }
    }
    // A download that's still coming in is still going: it's how long since anything was heard that counts.
    loop {
        match heard.recv_timeout(patience.min(Duration::from_secs(120)) + Duration::from_secs(5)) {
            Ok(Heard::Progress(got)) => progress(got, most as u64),
            Ok(Heard::Done(done)) => return done,
            Err(_) => return Err("the fetcher didn't answer".into()),
        }
    }
}

fn stop(slot: &mut Option<Running>) {
    if let Some(mut r) = slot.take() {
        r.child.kill();
        if let Ok(mut waiting) = r.waiting.lock() {
            for (_, back) in waiting.drain() {
                let _ = back.send(Heard::Done(Err("the fetcher was started again".into())));
            }
        }
    }
}

/// Starts one: for what goes through the proxy (`proxied`), or for what doesn't.
fn start(sandboxed: bool, proxied: bool) -> Result<Running, String> {
    let Some(exe) = crate::parts::path(FETCH).filter(|p| p.is_file()) else {
        crate::parts::found_missing(FETCH);
        return Err(format!("{FETCH} isn't there"));
    };
    // Sandboxed, the one for the proxy has no network of its own: it's handed its connections.
    let brokered = sandboxed && proxied;
    let container = if sandboxed {
        let made = crate::sandbox::container(Helper::Fetch, &[], !proxied);
        crate::sandbox::said(Helper::Fetch, made.as_ref().err().map_or("", String::as_str));
        Some(made?)
    } else {
        None
    };
    // Without windows or drawing only when sandboxed: switched off, the sandbox is all of it, so that a PC whose security software
    // won't have that still has a switch that brings fetching back.
    let mitigations = if sandboxed { pms_spawn::mitigate::USUAL | pms_spawn::mitigate::NO_WIN32K } else { pms_spawn::mitigate::USUAL };
    let command = pms_spawn::Command::new(exe).stdin(pms_spawn::Stdio::Piped).stdout(pms_spawn::Stdio::Piped).mitigations(mitigations).power(crate::efficiency::flags(crate::efficiency::Part::Helpers));
    let mut child = match container {
        Some(c) => command.container(c),
        None => command,
    }
    .spawn()?;
    let (Some(mut to), Some(from)) = (child.stdin.take(), child.stdout.take()) else { return Err("the fetcher: no pipes".into()) };
    let mut said = format!("agent PimpMyShell/{}\n", crate::version::CURRENT);
    if brokered {
        said.push_str("brokered\n");
    }
    to.write_all(said.as_bytes()).map_err(|e| e.to_string())?;
    let (to, waiting): (_, Arc<Mutex<Waiting>>) = (Arc::new(Mutex::new(to)), Arc::default());
    let sockets = brokered.then(|| (child.id(), to.clone()));
    let answers = waiting.clone();
    std::thread::spawn(move || read_answers(from, &answers, sockets));
    let how = match (sandboxed, proxied) {
        (true, true) => ", sandboxed, for what goes through the proxy, with sockets from here to it",
        (true, false) => ", sandboxed, for what doesn't go through the proxy",
        (false, true) => ", for what goes through the proxy",
        (false, false) => "",
    };
    crate::log(format!("fetch: {FETCH} started{how}"));
    Ok(Running { child, to, waiting, sandboxed })
}

/// A socket connected to the proxy, for process `pid`, as relay::connected hands it over: to whichever use's proxy goes through
/// one now (there's one proxy, whoever it's asked as).
fn to_the_proxy(pid: u32) -> Result<String, String> {
    let what = USES.into_iter().find(|u| crate::net::route_for(*u).is_some()).ok_or("there's no proxy")?;
    crate::relay::connected(pid, what)
}

/// Hands each answer to whoever's waiting for it, until the helper ends; then whoever's left is told. Brokered (its process id and
/// its stdin), it's also handed the sockets it asks for, each connected to the proxy.
fn read_answers(from: File, waiting: &Mutex<Waiting>, brokered: Option<(u32, Arc<Mutex<File>>)>) {
    let mut from = BufReader::new(from);
    let mut line = String::new();
    loop {
        line.clear();
        if from.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let said = line.trim_end();
        if let Some(asked) = said.strip_prefix("socket ") {
            let Some((pid, to)) = &brokered else { break };
            let (asked, pid, to) = (asked.to_string(), *pid, to.clone());
            // On a thread of its own: a proxy that's slow to answer holds up only this one.
            std::thread::spawn(move || {
                let said = match to_the_proxy(pid) {
                    Ok(hex) => format!("socket {asked} {hex}\n"),
                    Err(e) => {
                        crate::log(format!("fetch: no connection to the proxy: {e}"));
                        format!("socket {asked} none\n")
                    }
                };
                if let Ok(mut to) = to.lock() {
                    let _ = to.write_all(said.as_bytes()).and_then(|()| to.flush());
                }
            });
            continue;
        }
        let mut words = said.splitn(5, ' ');
        let Some(id) = words.next().and_then(|i| i.parse::<u64>().ok()) else { break };
        let heard = match (words.next(), words.next(), words.next(), words.next()) {
            (Some("failed"), ..) => Heard::Done(Err(said.splitn(3, ' ').nth(2).unwrap_or_default().to_string())),
            (Some("progress"), Some(got), ..) => {
                let Ok(got) = got.parse() else { break };
                Heard::Progress(got)
            }
            (Some(status), Some(length), Some(width), Some(height)) => {
                let (Ok(status), Ok(length), Ok(width), Ok(height)) = (status.parse(), length.parse::<usize>(), width.parse(), height.parse()) else { break };
                let mut body = vec![0; length];
                if from.read_exact(&mut body).is_err() {
                    break;
                }
                Heard::Done(Ok(Got { status, body, width, height }))
            }
            _ => break,
        };
        let done = matches!(heard, Heard::Done(_));
        let Ok(mut w) = waiting.lock() else { break };
        let back = if done { w.remove(&id) } else { w.get(&id).cloned() };
        if let Some(back) = back {
            let _ = back.send(heard);
        }
    }
    if let Ok(mut waiting) = waiting.lock() {
        for (_, back) in waiting.drain() {
            let _ = back.send(Heard::Done(Err(format!("{FETCH} ended"))));
        }
    }
}
