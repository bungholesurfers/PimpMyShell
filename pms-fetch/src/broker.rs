//! Connections by way of the daemon, for when pms-fetch is sandboxed and its fetching goes through the proxy. Its sandbox, the
//! strictest kind, has no network of its own then, and can't so much as listen on 127.0.0.1 (which is why it has no relay, as the
//! other helpers do): for each connection it asks the daemon for a socket, which the daemon connects to the proxy, and only ever
//! the proxy, and hands over (`WSADuplicateSocket`). Here, it asks the proxy for where it's going, by name (SOCKS5, so the proxy
//! looks the name up), with the user name and password the proxy address carries, and the TLS goes over that as it would anyway.
//!
//! Asked for on stdout (`socket <n>`), answered on stdin (`socket <n> <WSAPROTOCOL_INFOW in hexadecimal>`, or `none`).

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::windows::io::FromRawSocket;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ureq::unversioned::transport::{Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, Transport};

/// sizeof(WSAPROTOCOL_INFOW).
pub const INFO: usize = 628;

#[link(name = "ws2_32")]
unsafe extern "system" {
    fn WSASocketW(af: i32, kind: i32, protocol: i32, info: *const u8, group: u32, flags: u32) -> usize;
    fn WSAStartup(version: u16, data: *mut u8) -> i32;
}

/// Starts Winsock, once: Rust's own networking does that only when it's first used, and a socket handed over comes before that.
fn winsock() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // WSADATA, 408 bytes on 64-bit Windows; never looked at.
        let mut data = [0u8; 512];
        unsafe { WSAStartup(0x0202, data.as_mut_ptr()) };
    });
}

/// The sockets asked for and not yet come, by number, each with where its WSAPROTOCOL_INFOW goes (None: there's none).
type Waiting = HashMap<u64, Sender<Option<Vec<u8>>>>;
static WAITING: Mutex<Option<Waiting>> = Mutex::new(None);
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// An answer from the daemon, `socket <n> <hex or none>`: to whoever asked.
pub fn answered(rest: &str) {
    let mut words = rest.split(' ');
    let (Some(id), Some(hex)) = (words.next().and_then(|i| i.parse::<u64>().ok()), words.next()) else { return };
    let info = (hex != "none").then(|| (0..hex.len()).step_by(2).filter_map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect::<Vec<u8>>()).filter(|b| b.len() == INFO);
    if let Some(back) = WAITING.lock().ok().and_then(|mut w| w.as_mut().and_then(|w| w.remove(&id))) {
        let _ = back.send(info);
    }
}

/// A socket connected to the proxy, from the daemon.
fn socket(out: &Mutex<io::Stdout>, patience: Duration) -> io::Result<TcpStream> {
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (back, answer) = channel();
    WAITING.lock().map_err(|_| io::Error::other("waiting"))?.get_or_insert_with(HashMap::new).insert(id, back);
    {
        let mut out = out.lock().map_err(|_| io::Error::other("stdout"))?;
        writeln!(out, "socket {id}")?;
        out.flush()?;
    }
    let info = answer.recv_timeout(patience).ok().flatten().ok_or_else(|| io::Error::other("no connection to the proxy came"))?;
    winsock();
    let raw = unsafe { WSASocketW(-1, -1, -1, info.as_ptr(), 0, 0x01) };
    if raw == usize::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { TcpStream::from_raw_socket(raw as u64) })
}

/// Asks the proxy, on `stream`, for `host`:`port`: SOCKS5, by name, with `login` if there's one.
fn socks(stream: &mut TcpStream, host: &str, port: u16, login: Option<(&str, &str)>) -> io::Result<()> {
    let refused = |why: &str| io::Error::other(format!("the proxy: {why}"));
    stream.write_all(if login.is_some() { &[5, 2, 0, 2] } else { &[5, 1, 0] })?;
    let mut reply = [0u8; 2];
    stream.read_exact(&mut reply)?;
    match (reply, login) {
        ([5, 0], _) => {}
        ([5, 2], Some((user, password))) => {
            let (Ok(u), Ok(p)) = (u8::try_from(user.len()), u8::try_from(password.len())) else { return Err(refused("a user name or password that's too long")) };
            let mut asked = vec![1, u];
            asked.extend_from_slice(user.as_bytes());
            asked.push(p);
            asked.extend_from_slice(password.as_bytes());
            stream.write_all(&asked)?;
            stream.read_exact(&mut reply)?;
            if reply[1] != 0 {
                return Err(refused("it wouldn't take the user name and password"));
            }
        }
        _ => return Err(refused("it wants something it wasn't offered")),
    }
    let length = u8::try_from(host.len()).map_err(|_| refused("a name that's too long"))?;
    let mut asked = vec![5, 1, 0, 3, length];
    asked.extend_from_slice(host.as_bytes());
    asked.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&asked)?;
    let mut head = [0u8; 4];
    stream.read_exact(&mut head)?;
    if head[0] != 5 || head[1] != 0 {
        return Err(refused(&format!("it couldn't connect ({})", head[1])));
    }
    let rest = match head[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut n = [0u8];
            stream.read_exact(&mut n)?;
            usize::from(n[0])
        }
        _ => return Err(refused("an answer it doesn't understand")),
    };
    let mut skip = vec![0u8; rest + 2];
    stream.read_exact(&mut skip)
}

/// The connector, first in ureq's chain (TLS goes over what it makes): a socket from the daemon, through the proxy.
pub struct Brokered {
    pub out: Arc<Mutex<io::Stdout>>,
}

impl fmt::Debug for Brokered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Brokered")
    }
}

impl<In: Transport> Connector<In> for Brokered {
    type Out = Socket;

    fn connect(&self, details: &ConnectionDetails, _: Option<In>) -> Result<Option<Socket>, ureq::Error> {
        let proxy = details.config.proxy().ok_or_else(|| ureq::Error::Io(io::Error::other("brokered, with no proxy")))?;
        let host = details.uri.host().ok_or(ureq::Error::HostNotFound)?.trim_matches(['[', ']']);
        let port = details.uri.port_u16().unwrap_or(if details.uri.scheme_str() == Some("https") { 443 } else { 80 });
        let patience = details.timeout.not_zero().map_or(Duration::from_secs(20), |t| *t);
        let mut stream = socket(&self.out, patience)?;
        stream.set_read_timeout(Some(patience))?;
        stream.set_write_timeout(Some(patience))?;
        let login = proxy.username().map(|u| (u, proxy.password().unwrap_or("")));
        socks(&mut stream, host, port, login)?;
        let _ = stream.set_nodelay(true);
        Ok(Some(Socket { stream, buffers: LazyBuffers::new(details.config.input_buffer_size(), details.config.output_buffer_size()) }))
    }
}

/// A connection through the proxy, as ureq's transport.
pub struct Socket {
    stream: TcpStream,
    buffers: LazyBuffers,
}

impl fmt::Debug for Socket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Socket")
    }
}

impl Transport for Socket {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.stream.set_write_timeout(timeout.for_write().not_zero().map(|t| *t))?;
        let output = &self.buffers.output()[..amount];
        self.stream.write_all(output)?;
        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.stream.set_read_timeout(timeout.for_read().not_zero().map(|t| *t))?;
        let input = self.buffers.input_append_buf();
        let amount = loop {
            match self.stream.read(input) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                other => break other?,
            }
        };
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        // Never kept for another request: one connection, one socket asked for.
        false
    }
}
