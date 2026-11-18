//! The way a sandboxed helper reaches the proxy. An AppContainer can't reach anything on 127.0.0.1 that isn't in the same
//! container, whatever it's given, and it's often there a proxy is (Tor's). So the helper is given no network at all, and its
//! proxy is a relay in its own container (`pms-fetch.exe relay`, which is small and draws nothing, so it runs in the strictest
//! container too), which it reaches on 127.0.0.1, as processes in one container may.
//! For each connection it's asked for, the relay asks the process that started it (the daemon, or the settings, for a test)
//! for a socket; that process connects it to the proxy, and only ever the proxy, and hands it over (`WSADuplicateSocket`);
//! and the relay passes the bytes between the two. What goes through is the helper's own SOCKS talk with the proxy, with its
//! user name and password: the relay sees nothing it doesn't already pass on.
//!
//! So Windows itself keeps a sandboxed helper to the proxy: there's no connection it can make that doesn't go there.
//!
//! The relay's talk with the process that started it, a line at a time: it says `port <n>` once it's listening, then
//! `socket <id>` for each connection; it's answered `socket <id> <WSAPROTOCOL_INFOW in hexadecimal>`, or `socket <id> none`.

use std::io::{BufRead, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::windows::io::AsRawSocket;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::sandbox::Helper;

/// sizeof(WSAPROTOCOL_INFOW).
const INFO: usize = 628;

#[link(name = "ws2_32")]
unsafe extern "system" {
    fn WSADuplicateSocketW(socket: usize, pid: u32, info: *mut u8) -> i32;
}

/// A relay that's running, for one helper's container.
struct Running {
    child: pms_spawn::Child,
    port: u16,
}

/// The relays this process has started, one for each helper that needs one.
static RELAYS: Mutex<Vec<(Helper, Running)>> = Mutex::new(Vec::new());

/// Where `helper` should send what goes through `proxy` (as net::route_for has it): a relay in `container`, its own, on
/// 127.0.0.1. Started if it isn't running.
pub fn through(helper: Helper, what: crate::net::Use, container: &pms_spawn::Container, proxy: &str) -> Result<String, String> {
    let (scheme, rest) = proxy.split_once("://").ok_or("not a proxy's address")?;
    let (login, _) = rest.rsplit_once('@').map_or((None, rest), |(l, h)| (Some(l), h));
    let port = relay_port(helper, what, container)?;
    // Names are always looked up by the proxy: the helper can't look anything up itself.
    let scheme = if scheme == "socks5" { "socks5h" } else { scheme };
    Ok(match login {
        Some(login) => format!("{scheme}://{login}@127.0.0.1:{port}"),
        None => format!("{scheme}://127.0.0.1:{port}"),
    })
}

fn relay_port(helper: Helper, what: crate::net::Use, container: &pms_spawn::Container) -> Result<u16, String> {
    let mut relays = RELAYS.lock().map_err(|_| "the relays")?;
    relays.retain_mut(|(_, r)| r.child.try_wait().is_none());
    if let Some((_, r)) = relays.iter().find(|(h, _)| *h == helper) {
        return Ok(r.port);
    }
    let Some(exe) = crate::parts::path(crate::fetch::FETCH).filter(|p| p.is_file()) else {
        crate::parts::found_missing(crate::fetch::FETCH);
        return Err(format!("{} isn't there", crate::fetch::FETCH));
    };
    let mut child = pms_spawn::Command::new(exe)
        .arg("relay")
        .stdin(pms_spawn::Stdio::Piped)
        .stdout(pms_spawn::Stdio::Piped)
        .mitigations(pms_spawn::mitigate::USUAL | pms_spawn::mitigate::NO_WIN32K)
        .container(container.clone())
        .power(crate::efficiency::flags(crate::efficiency::Part::Helpers))
        .spawn()?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else { return Err("the relay: no pipes".into()) };
    let pid = child.id();
    let mut lines = std::io::BufReader::new(stdout).lines();
    let port: u16 = match lines.next() {
        Some(Ok(line)) => line.strip_prefix("port ").and_then(|p| p.parse().ok()).ok_or_else(|| format!("the relay said {line:?}"))?,
        _ => return Err(format!("the relay didn't start (it ended with {:#x})", child.wait())),
    };
    // Its asks, answered as they come, each on a thread of its own: a proxy that's slow to answer holds up only its own.
    let answers = Arc::new(Mutex::new(stdin));
    std::thread::spawn(move || {
        for line in lines.map_while(Result::ok) {
            let Some(id) = line.strip_prefix("socket ").map(str::to_string) else { continue };
            let answers = answers.clone();
            std::thread::spawn(move || {
                let said = match connected(pid, what) {
                    Ok(hex) => format!("socket {id} {hex}\n"),
                    Err(e) => {
                        crate::log(format!("relay: no connection to the proxy: {e}"));
                        format!("socket {id} none\n")
                    }
                };
                if let Ok(mut to) = answers.lock() {
                    let _ = to.write_all(said.as_bytes());
                }
            });
        }
    });
    crate::log(format!("relay: {}'s sandbox reaches the proxy through a relay of its own", helper.key()));
    relays.push((helper, Running { child, port }));
    Ok(port)
}

/// Stops `helper`'s relay, if it has one: its sandbox is being taken down.
pub fn stop(helper: Helper) {
    if let Ok(mut relays) = RELAYS.lock() {
        relays.retain_mut(|(h, r)| {
            if *h == helper {
                r.child.kill();
            }
            *h != helper
        });
    }
}

/// A connection to the proxy, made here and handed to process `pid`: its WSAPROTOCOL_INFOW, in hexadecimal. Only ever to
/// the proxy the settings name for what's being relayed.
pub(crate) fn connected(pid: u32, what: crate::net::Use) -> Result<String, String> {
    let proxy = crate::net::route_for(what).ok_or("there's no proxy")?;
    let rest = proxy.split_once("://").map_or(proxy.as_str(), |(_, r)| r);
    let host_port = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
    let address = host_port.to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or("the proxy's address")?;
    let socket = TcpStream::connect_timeout(&address, Duration::from_secs(15)).map_err(|e| e.to_string())?;
    let mut info = [0u8; INFO];
    if unsafe { WSADuplicateSocketW(socket.as_raw_socket() as usize, pid, info.as_mut_ptr()) } != 0 {
        return Err(format!("couldn't hand it over: {}", std::io::Error::last_os_error()));
    }
    Ok(info.iter().map(|b| format!("{b:02x}")).collect())
}
