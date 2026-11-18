//! Tries the network as PimpMyShell uses it: `net <proxy or ""> <live|streams|video|updates|downloads|iss> <url>` makes one request for
//! that use, through the proxy if one's given, and says what came back. By way of pms-fetch.exe, as everything is: it has to be
//! beside this example's exe (target\release\examples).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [proxy, what, url] = args.as_slice() else {
        eprintln!("usage: net <proxy or \"\"> <live|streams|video|updates|downloads|iss> <url>");
        std::process::exit(2);
    };
    let use_ = match what.as_str() {
        "live" => pms::net::Use::Live,
        "streams" => pms::net::Use::Streams,
        "video" => pms::net::Use::Video,
        "updates" => pms::net::Use::Updates,
        "iss" => pms::net::Use::Iss,
        _ => pms::net::Use::Downloads,
    };
    // Every use through it: nothing is, until it's switched on.
    let config = pms::config::parse(&format!("[network]\nproxy = \"{proxy}\"\nlive_checks = true\niss = true\nstreams = true\nupdates = true\ndownloads = true\n")).expect("config");
    pms::net::follow(&config);
    println!("through: {:?}", pms::net::proxy_for(use_));
    match pms::fetch::fetch(use_, pms::fetch::Kind::Head, url, std::time::Duration::from_secs(20), 0, 0) {
        Ok(got) => println!("{url}: {}", got.status),
        Err(e) => println!("{url}: {e}"),
    }
}
