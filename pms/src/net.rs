//! The network: what PimpMyShell asks of it, and the proxy it may go through.
//!
//! Every request PimpMyShell makes itself is made here, over HTTPS, with ureq. A SOCKS5 proxy (`proxy` in pms-network.toml) can carry any
//! or all of them, by what they're for, and Streamlink's and the video's too: Streamlink is told with `--http-proxy`, and the
//! video player (pms-twatch) fetches through it itself rather than leave it to Windows. With `socks5h`, names are looked up at
//! the proxy as well, so nothing about where it's going leaves this PC but by way of it. A request that should go through the
//! proxy never goes without it: if the proxy can't be reached, it fails.

use std::sync::RwLock;
use std::time::Duration;

/// What a request is for, which says whether it goes through the proxy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Use {
    /// Twitch's preview pictures: who's live, and the stills.
    Live,
    /// Streamlink finding a stream.
    Streams,
    /// The video itself, and its playlists: always the way `Streams` goes, as the same somebody. The address Streamlink gets for a
    /// stream names the IP address that asked for it, so going two ways would tie them together.
    Video,
    /// PimpMyShell's own updates.
    Updates,
    /// Games, and Streamlink itself.
    Downloads,
}

/// The proxy, and what it's for: as the settings last said.
static PROXY: RwLock<Option<crate::config::Network>> = RwLock::new(None);

/// Takes up what `config` says about the network, for every request from now on.
pub fn follow(config: &crate::config::Config) {
    if let Ok(mut p) = PROXY.write() {
        *p = Some(config.network.clone().unwrap_or_default());
    }
}

/// Whether `address` can be a proxy: socks5:// or socks5h://, a host, and a port, with a user and password if it likes.
pub fn check(address: &str) -> Result<(), String> {
    let rest = address.strip_prefix("socks5h://").or_else(|| address.strip_prefix("socks5://")).ok_or("It has to start with socks5:// or socks5h://")?;
    let host_port = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
    let (host, port) = host_port.rsplit_once(':').ok_or("It needs a port, like :1080")?;
    if host.is_empty() || host.contains('/') {
        return Err("It needs a host, like 127.0.0.1".into());
    }
    port.parse::<u16>().ok().filter(|p| *p != 0).ok_or("The port has to be a number from 1 to 65535")?;
    ureq::Proxy::new(address).map(drop).map_err(|e| e.to_string())
}

/// The proxy for `what`, if it goes through one: its address, as the settings have it, which may be one that can't be used (then
/// what it's for fails, rather than go without it).
pub fn proxy_for(what: Use) -> Option<String> {
    let p = PROXY.read().ok()?;
    let n = p.as_ref()?;
    let on = match what {
        Use::Live => n.live_checks,
        Use::Streams | Use::Video => n.streams,
        Use::Updates => n.updates,
        Use::Downloads => n.downloads,
    };
    Some(n.proxy.trim().to_string()).filter(|a| on && !a.is_empty())
}

/// Somewhere nothing can connect to: what a proxy that can't be used is taken as, so that what it was for fails.
const NOWHERE: &str = "socks5://0.0.0.0:9";

/// The proxy for `what` as something can be told it: the address, as somebody of its own if each use is to be kept apart; or, if
/// it can't be used, nowhere.
pub fn route_for(what: Use) -> Option<String> {
    let isolate = PROXY.read().ok().and_then(|p| p.as_ref().map(|n| n.isolate)).unwrap_or(false);
    proxy_for(what).map(|a| match check(&a) {
        Ok(()) if isolate => as_somebody(&a, what),
        Ok(()) => a,
        Err(_) => NOWHERE.to_string(),
    })
}

/// `address` with the user name and password `what` goes by this time, in place of any it has. Tor keeps what comes with different
/// ones apart, on circuits of their own.
fn as_somebody(address: &str, what: Use) -> String {
    let (scheme, rest) = address.split_once("://").unwrap_or(("socks5h", address));
    let host = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
    format!("{scheme}://{}@{host}", identity(what))
}

/// The user name and password `what` goes by: made up at random the first time they're wanted, from the operating system's random
/// numbers, and kept until PimpMyShell exits.
fn identity(what: Use) -> &'static str {
    static MADE: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let random = || {
        let mut bytes = [0u8; 8];
        let _ = getrandom::fill(&mut bytes);
        u64::from_le_bytes(bytes)
    };
    let all = MADE.get_or_init(|| USES.iter().map(|_| format!("pms{:016x}:{:016x}{:016x}", random(), random(), random())).collect());
    let what = if what == Use::Video { Use::Streams } else { what };
    let at = USES.iter().position(|(u, ..)| *u == what).unwrap_or(0);
    &all[at]
}

/// Something to make requests for `what` with, through the proxy if that's what's wanted: HTTPS only, and nobody's certificate
/// store but its own. Anything but an error comes back as it is, so a redirect that isn't followed can be read.
pub fn agent(what: Use, patience: Duration, redirects: u32, user_agent: &str) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::WebPki).build();
    let config = ureq::Agent::config_builder()
        .https_only(true)
        .tls_config(tls)
        .max_redirects(redirects)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .user_agent(user_agent)
        .timeout_connect(Some(Duration::from_secs(20).min(patience)))
        .timeout_global(Some(patience));
    // Without one of ours, nothing: a proxy the environment names isn't one you chose here. One of ours that can't be used is
    // nowhere, so that nothing meant for it goes without it.
    let proxy = route_for(what).and_then(|a| ureq::Proxy::new(&a).ok());
    config.proxy(proxy).build().into()
}

/// Tries the way `what` goes out: one small request to where it goes, through the proxy if that's its way, as it would go. What
/// came of it, in words. Takes seconds: not on a thread that's drawing.
pub fn try_out(what: Use) -> Result<String, String> {
    let began = std::time::Instant::now();
    let how = if proxy_for(what).is_some() { "through the proxy" } else { "directly, not through the proxy" };
    let (name, url) = match what {
        Use::Live => ("Twitch's pictures", "https://static-cdn.jtvnw.net/previews-ttv/live_user_twitch-320x180.jpg"),
        // Both halves: Streamlink, then where the video comes from, the way it goes.
        Use::Streams | Use::Video => {
            crate::streamlink::try_out()?;
            agent(Use::Video, Duration::from_secs(20), 0, &format!("PimpMyShell/{} (network test)", crate::version::CURRENT)).head("https://usher.ttvnw.net/").call().map_err(|e| format!("Streamlink worked, but the video service didn't: {e}"))?;
            return Ok(format!("Streamlink and Twitch's video service both answered {how}, in {} ms.", began.elapsed().as_millis()));
        }
        Use::Updates => ("GitHub", crate::update::STABLE_URL),
        Use::Downloads => ("GitHub", "https://api.github.com/"),
    };
    // Any answer at all is the way working: what it says doesn't matter here.
    agent(what, Duration::from_secs(20), 0, &format!("PimpMyShell/{} (network test)", crate::version::CURRENT)).head(url).call().map_err(|e| format!("That didn't work: {e}"))?;
    Ok(format!("Reached {name} {how} in {} ms.", began.elapsed().as_millis()))
}

/// Everything PimpMyShell and the programs it runs use the network for, for the settings to say.
pub const USES: [(Use, &str, &str, &str); 4] = [
    (
        Use::Live,
        "Who's live, and pictures",
        "Twatch TV",
        "A small request for each favorite's preview picture, from https://static-cdn.jtvnw.net/previews-ttv/, as often as you set under Twatch TV (every 10 to 15 minutes unless you change it, twice as long on battery), with no account. With pictures instead of video, the picture itself, once a minute",
    ),
    (
        Use::Streams,
        "Finding and playing streams",
        "Twatch TV",
        "When a favorite is live and a TwaTV plays video, Streamlink asks Twitch for the stream: https://gql.twitch.tv and https://usher.ttvnw.net. Then the stream's playlists and video come from Twitch's video servers, about 1 to 1.5 Mbps a TwaTV at 480p, for as long as one plays. The two go the same way, as the same somebody: the address Twitch gives for a stream names the IP address that asked for it. Streamlink is told to use the proxy, and PimpMyShell fetches the video itself and hands it to Windows' player",
    ),
    (
        Use::Updates,
        "Checking for updates",
        "PimpMyShell",
        "What's newest, every day or two if you said so: on Stable, from https://raw.githubusercontent.com/bungholesurfers/PimpMyShell/main/update/stable.txt, and on Latest, from https://github.com/bungholesurfers/PimpMyShell/releases/latest. Then the release itself, from https://github.com/bungholesurfers/PimpMyShell/releases",
    ),
    (
        Use::Downloads,
        "Downloads",
        "PIMPBOX and Twatch TV",
        "When you select Download: Freedoom from https://github.com/freedoom/freedoom/releases, Doom's shareware episode from https://deb.debian.org, Streamlink from https://github.com/streamlink/windows-builds by way of https://api.github.com, and, by itself, now and then while a live stream won't open, asking there whether a newer Streamlink would fix it",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_socks5_addresses_with_a_host_and_port_will_do() {
        for good in ["socks5://127.0.0.1:1080", "socks5h://proxy.example:9050", "socks5h://me:secret@10.0.0.2:1080"] {
            assert!(check(good).is_ok(), "{good}");
        }
        for bad in ["", "127.0.0.1:1080", "http://127.0.0.1:8080", "socks5://127.0.0.1", "socks5://:1080", "socks5://host:0", "socks5://host:99999"] {
            assert!(check(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_proxy_carries_only_what_it_is_for() {
        let mut config = crate::config::parse("").unwrap();
        follow(&config);
        assert_eq!(proxy_for(Use::Live), None, "none set");
        config.network = Some(crate::config::Network { proxy: "socks5h://127.0.0.1:9050".into(), ..Default::default() });
        follow(&config);
        assert_eq!(proxy_for(Use::Live), None, "nothing goes through it until it's switched on");
        config.network = Some(crate::config::Network { proxy: "socks5h://127.0.0.1:9050".into(), live_checks: true, ..Default::default() });
        follow(&config);
        assert_eq!(proxy_for(Use::Live).as_deref(), Some("socks5h://127.0.0.1:9050"));
        assert_eq!(proxy_for(Use::Updates), None, "switched off for updates");
        config.network = Some(crate::config::Network { proxy: "socks5h://me:mine@127.0.0.1:9050".into(), live_checks: true, streams: true, isolate: true, ..Default::default() });
        follow(&config);
        let (live, video) = (route_for(Use::Live).unwrap(), route_for(Use::Video).unwrap());
        assert!(live.starts_with("socks5h://pms") && live.ends_with("@127.0.0.1:9050") && !live.contains("me:mine"), "somebody of its own: {live}");
        assert_ne!(live, video, "each use is somebody else");
        assert_eq!(route_for(Use::Streams), Some(video), "but finding a stream and playing it are the same somebody");
        assert_eq!(live, route_for(Use::Live).unwrap(), "and stays that somebody");
        assert!(check(&live).is_ok());
        config.network = Some(crate::config::Network { proxy: "not a proxy".into(), streams: true, ..Default::default() });
        follow(&config);
        assert_eq!(proxy_for(Use::Video).as_deref(), Some("not a proxy"), "one that can't be used is still the way, and fails");
        let failed = agent(Use::Video, Duration::from_secs(5), 0, "test").head("https://example.com/").call();
        assert!(failed.is_err(), "it doesn't go without it");
        follow(&crate::config::parse("").unwrap());
    }
}
