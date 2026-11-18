//! The relay in a helper's sandbox (`pms-fetch.exe relay`): the helper's way to the proxy. It listens on 127.0.0.1, which the
//! helper can reach because they share a container, and passes each connection on to a socket the daemon connected to the
//! proxy, and only ever the proxy, and handed over. See pms/src/relay.rs, the daemon's side, for why and how.

use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::windows::io::FromRawSocket;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// sizeof(WSAPROTOCOL_INFOW).
const INFO: usize = 628;

#[link(name = "ws2_32")]
unsafe extern "system" {
    fn WSASocketW(af: i32, kind: i32, protocol: i32, info: *const u8, group: u32, flags: u32) -> usize;
}

/// The connections waiting for their sockets, by number, each with where to send its socket (None: there's none).
type Waiting = HashMap<u64, Sender<Option<Vec<u8>>>>;

/// The relay itself (`pms-fetch.exe relay`), in a helper's container: listens on 127.0.0.1, and passes each connection on to a
/// socket it's handed. Ends when its stdin does.
pub fn serve() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let out = Arc::new(Mutex::new(std::io::stdout()));
    say(&out, &format!("port {port}"));
    // What comes back, by the connection it's for.
    let waiting: Arc<Mutex<Waiting>> = Arc::default();
    let back = waiting.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            let mut words = line.split(' ');
            let (Some("socket"), Some(id), Some(hex)) = (words.next(), words.next().and_then(|i| i.parse::<u64>().ok()), words.next()) else { continue };
            let info = (hex != "none").then(|| (0..hex.len()).step_by(2).filter_map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect::<Vec<u8>>()).filter(|b| b.len() == INFO);
            if let Some(to) = back.lock().ok().and_then(|mut w| w.remove(&id)) {
                let _ = to.send(info);
            }
        }
        // Whoever started it has gone: so does it.
        std::process::exit(0);
    });
    for (id, incoming) in (0u64..).zip(listener.incoming()) {
        let Ok(incoming) = incoming else { continue };
        let (to, from) = channel();
        if let Ok(mut w) = waiting.lock() {
            w.insert(id, to);
        }
        say(&out, &format!("socket {id}"));
        std::thread::spawn(move || {
            let Ok(Some(info)) = from.recv_timeout(Duration::from_secs(30)) else { return };
            let raw = unsafe { WSASocketW(-1, -1, -1, info.as_ptr(), 0, 0x01) };
            if raw == usize::MAX {
                return;
            }
            let outgoing = unsafe { TcpStream::from_raw_socket(raw as u64) };
            pass(incoming, outgoing);
        });
    }
    Ok(())
}

fn say(out: &Mutex<std::io::Stdout>, line: &str) {
    if let Ok(mut out) = out.lock() {
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

/// Passes bytes both ways between `a` and `b` until either ends.
fn pass(a: TcpStream, b: TcpStream) {
    let (Ok(a2), Ok(b2)) = (a.try_clone(), b.try_clone()) else { return };
    let one_way = |mut from: TcpStream, mut to: TcpStream| {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            match from.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if to.write_all(&buffer[..n]).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = to.shutdown(std::net::Shutdown::Write);
    };
    let there = std::thread::spawn(move || one_way(a, b2));
    one_way(b, a2);
    let _ = there.join();
}
