//! Fetches for the daemon (fetch.rs there), so that what comes from the internet is opened here rather than in it: the HTTPS,
//! Twitch's preview pictures, decoded, and speed demos, unzipped. The daemon gets back only plain pixels, and bytes it asked
//! for no more of than it can use. Sandboxed when the settings say, in the strictest kind (LPAC), and then with windows and
//! drawing (win32k) switched off too, which nothing here needs: a JPEG or a zip that took it over would find little to reach.
//!
//! It's told on stdin, a line at a time; first `agent <the user agent>`, and `brokered` if its connections through the proxy
//! are to come from the daemon (broker.rs: in its sandbox, it has no network of its own), then for each fetch:
//!
//!   <id> <head|get|jpeg|download|lmp> <seconds it may take> <redirects> <most bytes> <proxy, or -> <url>
//!
//! and answers each on stdout, as it's done, with a line and then that many bytes:
//!
//!   <id> <HTTP status> <bytes that follow> <width> <height>
//!
//! (for `jpeg`, the picture as 8-bit RGB, width by height; for `lmp`, a zip's one demo, unzipped; width and height are 0 for
//! anything but a picture), or `<id> failed <why>`. A
//! `download` says how far it's got as it goes, `<id> progress <bytes>`, every quarter of a megabyte. A redirect it wasn't to
//! follow (more than it was allowed) comes back as its status, with where it goes to as the bytes. When
//! stdin closes, it ends. Brokered, it also asks for sockets on stdout, and is answered on stdin (broker.rs).
//!
//! `pms-fetch.exe relay` is something else: the relay in a sandboxed helper's container (relay.rs).

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod broker;
mod relay;

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The largest picture it'll decode, each way: Twitch's previews are 320 by 180.
const MOST_SIDE: u16 = 1920;

/// What a fetch came to: its status, the bytes, and for a picture its size.
struct Got {
    status: u16,
    body: Vec<u8>,
    size: (u16, u16),
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("relay") {
        // What went wrong, for whoever started it to say.
        if let Err(e) = relay::serve() {
            let mut out = std::io::stdout();
            let _ = writeln!(out, "failed {e}");
            let _ = out.flush();
        }
        return;
    }
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let mut how = How { user_agent: String::from("PimpMyShell"), brokered: None };
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        if let Some(agent) = line.strip_prefix("agent ") {
            how.user_agent = agent.trim().to_string();
            continue;
        }
        if line == "brokered" {
            how.brokered = Some(out.clone());
            continue;
        }
        if let Some(rest) = line.strip_prefix("socket ") {
            broker::answered(rest);
            continue;
        }
        let (out, how) = (out.clone(), how.clone());
        // Each on a thread of its own: one that's slow holds up nobody else's.
        std::thread::spawn(move || {
            let mut words = line.splitn(7, ' ');
            let id = words.next().unwrap_or("0").to_string();
            let progress = |got: u64| {
                if let Ok(mut out) = out.lock() {
                    let _ = writeln!(out, "{id} progress {got}");
                    let _ = out.flush();
                }
            };
            let said = match asked(&mut words, &how, &progress) {
                Ok(got) => {
                    let mut said = format!("{id} {} {} {} {}\n", got.status, got.body.len(), got.size.0, got.size.1).into_bytes();
                    said.extend_from_slice(&got.body);
                    said
                }
                Err(why) => format!("{id} failed {}\n", why.replace(['\r', '\n'], " ")).into_bytes(),
            };
            if let Ok(mut out) = out.lock() {
                let _ = out.write_all(&said);
                let _ = out.flush();
            }
        });
    }
}

/// How it fetches: as whom, and whether connections through the proxy come from the daemon (and if they do, where to ask).
#[derive(Clone)]
struct How {
    user_agent: String,
    brokered: Option<Arc<Mutex<std::io::Stdout>>>,
}

/// Does what the rest of a line asks.
fn asked<'a>(words: &mut impl Iterator<Item = &'a str>, how: &How, progress: &dyn Fn(u64)) -> Result<Got, String> {
    let (Some(kind), Some(patience), Some(redirects), Some(most), Some(proxy), Some(url)) = (words.next(), words.next(), words.next(), words.next(), words.next(), words.next()) else {
        return Err("a line it doesn't understand".into());
    };
    let patience = Duration::from_secs(patience.parse().map_err(|_| "how long")?);
    let redirects: u32 = redirects.parse().map_err(|_| "how many redirects")?;
    let most: u64 = most.parse().map_err(|_| "how many bytes")?;
    let agent = agent(patience, redirects, how, (proxy != "-").then_some(proxy))?;
    let response = match kind {
        "head" => agent.head(url).call(),
        "get" | "jpeg" | "download" | "lmp" => agent.get(url).call(),
        other => return Err(format!("{other}: not something it does")),
    }
    .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    // A redirect it wasn't to follow: where to, for whoever asked to follow it (or not) themselves.
    if (300..400).contains(&status) {
        let location = response.headers().get("location").and_then(|l| l.to_str().ok()).unwrap_or_default();
        return Ok(Got { status, body: location.as_bytes().iter().copied().take(4096).collect(), size: (0, 0) });
    }
    if kind == "head" || status != 200 && (kind == "jpeg" || kind == "lmp") {
        return Ok(Got { status, body: Vec::new(), size: (0, 0) });
    }
    if kind == "download" {
        return Ok(Got { status, body: downloaded(response.into_body().into_reader(), most, progress)?, size: (0, 0) });
    }
    let body = response.into_body().with_config().limit(most).read_to_vec().map_err(|e| e.to_string())?;
    if kind == "get" {
        return Ok(Got { status, body, size: (0, 0) });
    }
    if kind == "lmp" {
        return Ok(Got { status, body: demo_in(&body, most)?, size: (0, 0) });
    }
    let (rgb, size) = decoded(&body)?;
    Ok(Got { status, body: rgb, size })
}

/// What fetches: HTTPS only, nobody's certificates but its own, through `proxy` if there's one and never any other. Brokered,
/// through the proxy, its connections are the daemon's sockets (broker.rs), and names are never looked up here.
fn agent(patience: Duration, redirects: u32, how: &How, proxy: Option<&str>) -> Result<ureq::Agent, String> {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::WebPki).build();
    let proxy = proxy.map(|p| ureq::Proxy::new(p).map_err(|e| format!("the proxy: {e}"))).transpose()?;
    let brokered = how.brokered.clone().filter(|_| proxy.is_some());
    let config = ureq::Agent::config_builder()
        .https_only(true)
        .tls_config(tls)
        .max_redirects(redirects)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .user_agent(&how.user_agent)
        .timeout_connect(Some(Duration::from_secs(20).min(patience)))
        .timeout_global(Some(patience))
        .proxy(proxy)
        .build();
    Ok(match brokered {
        Some(out) => {
            use ureq::unversioned::transport::Connector;
            let connector = broker::Brokered { out }.chain(ureq::unversioned::transport::RustlsConnector::default());
            ureq::Agent::with_parts(config, connector, ureq::unversioned::resolver::DefaultResolver::default())
        }
        None => config.into(),
    })
}

/// What `reader` has, `most` bytes at most, saying how much has come every quarter of a megabyte.
fn downloaded(mut reader: impl std::io::Read, most: u64, progress: &dyn Fn(u64)) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut chunk = vec![0; 256 * 1024];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() as u64 > most {
            return Err("it's bigger than it should be".into());
        }
        progress(bytes.len() as u64);
    }
}

/// What a fault in what was fetched (rather than in fetching it) begins with, so that whoever asked knows it would be the same
/// every time: fetch.rs's `UNUSABLE`.
const UNUSABLE: &str = "unusable: ";

/// The one demo in a zip (a speed demo, as the archive keeps them: the .lmp and a .txt about it), unzipped: `most` bytes at the
/// most, whatever the zip says it is. A zip that isn't one, or hasn't exactly one demo in it, is UNUSABLE.
fn demo_in(zip: &[u8], most: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let unusable = |why: String| format!("{UNUSABLE}{why}");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| unusable(format!("the zip: {e}")))?;
    let demos: Vec<usize> = (0..archive.len()).filter(|i| archive.name_for_index(*i).is_some_and(|n| n.is_ok_and(|n| n.to_ascii_lowercase().ends_with(".lmp")))).collect();
    let [at] = demos[..] else { return Err(unusable(format!("a zip with {} demos in it, not one", demos.len()))) };
    let file = archive.by_index(at).map_err(|e| unusable(format!("the zip: {e}")))?;
    if file.size() > most {
        return Err(unusable("a demo bigger than it should be".into()));
    }
    let mut bytes = Vec::new();
    file.take(most + 1).read_to_end(&mut bytes).map_err(|e| unusable(format!("the zip: {e}")))?;
    if bytes.len() as u64 > most {
        return Err(unusable("a demo bigger than it says".into()));
    }
    Ok(bytes)
}

/// A JPEG, as 8-bit RGB, and its size: no bigger than MOST_SIDE each way.
fn decoded(bytes: &[u8]) -> Result<(Vec<u8>, (u16, u16)), String> {
    let mut decoder = jpeg_decoder::Decoder::new(bytes);
    decoder.read_info().map_err(|e| e.to_string())?;
    let info = decoder.info().ok_or("no size")?;
    if info.width == 0 || info.height == 0 || info.width > MOST_SIDE || info.height > MOST_SIDE {
        return Err(format!("a picture {} by {}", info.width, info.height));
    }
    decoder.set_max_decoding_buffer_size(usize::from(MOST_SIDE) * usize::from(MOST_SIDE) * 4);
    let pixels = decoder.decode().map_err(|e| e.to_string())?;
    let rgb = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels,
        jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|v| [*v; 3]).collect(),
        other => return Err(format!("a picture in {other:?}")),
    };
    if rgb.len() != usize::from(info.width) * usize::from(info.height) * 3 {
        return Err("a picture that isn't the size it says".into());
    }
    Ok((rgb, (info.width, info.height)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_isnt_a_jpeg_is_refused() {
        assert!(decoded(b"").is_err());
        assert!(decoded(b"\xff\xd8\xff\xe0 not really").is_err());
    }

    #[test]
    fn a_line_it_doesnt_understand_is_said_to_be() {
        let how = How { user_agent: "test".into(), brokered: None };
        assert!(asked(&mut "get 10".split(' '), &how, &|_| {}).is_err());
        assert!(asked(&mut "post 10 0 100 - https://example.invalid/".split(' '), &how, &|_| {}).is_err());
    }

    #[test]
    fn a_download_stops_at_the_most_and_says_how_far_it_got() {
        let said = std::cell::Cell::new(0);
        assert_eq!(downloaded(&[7u8; 1000][..], 1000, &|got| said.set(got)).unwrap().len(), 1000);
        assert_eq!(said.get(), 1000);
        assert!(downloaded(&[7u8; 1001][..], 1000, &|_| {}).is_err());
    }

    /// A zip of the files given, deflated.
    fn zipped(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn a_zips_one_demo_comes_out_and_nothing_else_does() {
        let demo = [109u8, 3, 1, 2, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0x80];
        let zip = zipped(&[("e1m2-021.txt", b"Player: someone"), ("E1M2-021.LMP", &demo)]);
        assert_eq!(demo_in(&zip, 1000).unwrap(), demo);
        assert!(demo_in(&zip, 10).is_err_and(|e| e.starts_with(UNUSABLE)), "bigger than it may be, every time");
        assert!(demo_in(&zipped(&[("a.lmp", &demo), ("b.lmp", &demo)]), 1000).is_err(), "two demos");
        assert!(demo_in(&zipped(&[("notes.txt", b"none")]), 1000).is_err(), "no demo");
        assert!(demo_in(b"not a zip", 1000).is_err());
        // A demo that unzips to far more than it was zipped as stops at the most.
        assert!(demo_in(&zipped(&[("big.lmp", &vec![0u8; 1 << 20])]), 4096).is_err());
    }
}
