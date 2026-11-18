//! The network: what PimpMyShell asks of it, and the proxy it may go through.
//!
//! Every request PimpMyShell makes itself is made by pms-fetch.exe (fetch.rs), over HTTPS, with ureq: none by the daemon or the
//! settings themselves, which don't so much as hold a TLS library. A SOCKS5 proxy (`proxy` in pms-network.toml) can carry any
//! or all of them, by what they're for, and Streamlink's and the video's too: Streamlink is told with `--http-proxy`, and the
//! video player (pms-twatch) fetches through it itself rather than leave it to Windows. With `socks5h`, names are looked up at
//! the proxy as well, so nothing about where it's going leaves this PC but by way of it. A request that should go through the
//! proxy never goes without it: if the proxy can't be reached, it fails.

use std::sync::RwLock;
use std::time::Duration;

use crate::lang::net as t;

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
    /// Number One's telemetry from the ISS, on TV channel 37: NASA's ISSLive!, by way of Lightstreamer.
    Iss,
}

/// The proxy, and what it's for: as the settings last said.
static PROXY: RwLock<Option<crate::config::Network>> = RwLock::new(None);

/// Takes up what `config` says about the network, for every request from now on.
pub fn follow(config: &crate::config::Config) {
    if let Ok(mut p) = PROXY.write() {
        *p = Some(config.network.clone().unwrap_or_default());
    }
    // Which helpers are sandboxed goes with it, and which are in efficiency mode: whatever starts a helper that goes online
    // has followed this first.
    crate::sandbox::follow(config);
    crate::efficiency::follow(config);
}

/// Whether `what` goes through a proxy on this PC (127.0.0.1, ::1, localhost), which no AppContainer can reach.
pub fn proxy_is_here(what: Use) -> bool {
    proxy_for(what).is_some_and(|address| {
        let rest = address.split_once("://").map_or(address.as_str(), |(_, r)| r);
        let host_port = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
        let host = host_port.rsplit_once(':').map_or(host_port, |(h, _)| h).trim_matches(['[', ']']);
        host.eq_ignore_ascii_case("localhost") || host == "::1" || host.starts_with("127.")
    })
}

/// Whether `address` can be a proxy: socks5:// or socks5h://, a host, and a port, with a user and password if it likes.
pub fn check(address: &str) -> Result<(), String> {
    let rest = address.strip_prefix("socks5h://").or_else(|| address.strip_prefix("socks5://")).ok_or(t::PROXY_SCHEME.t())?;
    let host_port = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
    let (host, port) = host_port.rsplit_once(':').ok_or(t::PROXY_PORT.t())?;
    if host.is_empty() || host.contains('/') {
        return Err(t::PROXY_HOST.t().into());
    }
    port.parse::<u16>().ok().filter(|p| *p != 0).ok_or(t::PROXY_PORT_NUMBER.t())?;
    // It goes to pms-fetch.exe on a line of words.
    if address.contains(char::is_whitespace) {
        return Err(t::PROXY_SPACE.t().into());
    }
    Ok(())
}

/// The proxy for `what`, if it goes through one: its address, as the settings have it, which may be one that can't be used (then
/// what it's for fails, rather than go without it).
pub fn proxy_for(what: Use) -> Option<String> {
    proxy_in(PROXY.read().ok()?.as_ref()?, what)
}

/// The same, as `n` has it.
fn proxy_in(n: &crate::config::Network, what: Use) -> Option<String> {
    let on = match what {
        Use::Live => n.live_checks,
        Use::Iss => n.iss,
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
    route_in(PROXY.read().ok()?.as_ref()?, what)
}

/// The same, as `session` if there's one and each use is kept apart (with `isolate`; otherwise the proxy's own login is the only
/// one it takes): so that everything one stream does, from finding it to the last of its video, goes as that stream's somebody,
/// and no two streams as the same one.
pub fn route_as(what: Use, session: Option<&Session>) -> Option<String> {
    let guard = PROXY.read().ok()?;
    let n = guard.as_ref()?;
    let route = route_in(n, what)?;
    Some(match session {
        Some(s) if n.isolate && route != NOWHERE => {
            let (scheme, rest) = route.split_once("://").unwrap_or(("socks5h", &route));
            let host = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
            format!("{scheme}://{}@{host}", s.0.login)
        }
        _ => route,
    })
}

/// A login of one stream's own: made up when it's looked for, and gone with it (when the last copy's dropped). Each that's in use
/// is listed with the daemon's own, for the settings to show.
#[derive(Clone)]
pub struct Session(std::sync::Arc<SessionInner>);

struct SessionInner {
    /// `user:password`.
    login: String,
    /// What it's for, as the settings say it: which TwaTV.
    label: String,
}

impl Session {
    /// A new one, for `label` (which TwaTV, say: nothing that names anyone).
    pub fn new(label: &str) -> Session {
        let made = Session(std::sync::Arc::new(SessionInner { login: made_up_one(), label: label.replace(['\t', '\r', '\n'], " ") }));
        if let Ok(mut s) = SESSIONS.lock() {
            s.push(std::sync::Arc::downgrade(&made.0));
        }
        write_logins();
        made
    }
}

impl PartialEq for Session {
    fn eq(&self, other: &Session) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Never its password: it's shown in the settings, and nowhere else.
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Session({})", self.0.label)
    }
}

impl Drop for SessionInner {
    fn drop(&mut self) {
        // Its own weak reference is dead by now: the list loses it as it's written again.
        write_logins();
    }
}

/// The streams' logins in use now.
static SESSIONS: std::sync::Mutex<Vec<std::sync::Weak<SessionInner>>> = std::sync::Mutex::new(Vec::new());

/// One made-up login, from the operating system's random numbers.
fn made_up_one() -> String {
    let random = || {
        let mut bytes = [0u8; 8];
        let _ = getrandom::fill(&mut bytes);
        u64::from_le_bytes(bytes)
    };
    format!("pms{:016x}:{:016x}{:016x}", random(), random(), random())
}

/// The same, as `n` has it.
fn route_in(n: &crate::config::Network, what: Use) -> Option<String> {
    proxy_in(n, what).map(|a| match check(&a) {
        Ok(()) if n.isolate => as_somebody(&a, what),
        Ok(()) => a,
        Err(_) => NOWHERE.to_string(),
    })
}

/// How `what` goes as `n` has it, and as whom: for the settings to show, by each use.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Login {
    /// Not through the proxy.
    Direct,
    /// Through a proxy whose address can't be used: so nowhere, and it fails.
    Nowhere,
    /// Through the proxy, with no user name or password.
    Without,
    /// Through the proxy, as this user, with this password.
    As { user: String, password: String },
}

/// The SOCKS login `what` goes as now, by the settings in `n`: with each use kept apart (`isolate`), the one this run of the daemon
/// made up for it; otherwise whatever the proxy's address has in it, the same for every use.
pub fn login(n: &crate::config::Network, what: Use) -> Login {
    let Some(route) = route_in(n, what) else { return Login::Direct };
    if route == NOWHERE {
        return Login::Nowhere;
    }
    let rest = route.split_once("://").map_or(route.as_str(), |(_, r)| r);
    match rest.rsplit_once('@').map(|(who, _)| who.split_once(':').unwrap_or((who, ""))) {
        Some((user, password)) => Login::As { user: user.to_string(), password: password.to_string() },
        None => Login::Without,
    }
}

/// `address` with the user name and password `what` goes by this time, in place of any it has. Tor keeps what comes with different
/// ones apart, on circuits of their own.
fn as_somebody(address: &str, what: Use) -> String {
    let (scheme, rest) = address.split_once("://").unwrap_or(("socks5h", address));
    let host = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
    format!("{scheme}://{}@{host}", identity(what))
}

/// Where the daemon keeps the user names and passwords it made up for this run, for the settings to show and go by.
fn logins_file() -> std::path::PathBuf {
    pms_tap::data_dir().join("socks-logins.txt")
}

/// What each use goes as, made up at random from the operating system's random numbers: in `USES`' order.
fn made_up() -> Vec<String> {
    USES.iter().map(|_| made_up_one()).collect()
}

/// The daemon's own, for this run: set by `begin`.
static DAEMONS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// For the daemon, as it starts: new user names and passwords for this run, kept until it exits, and written where the settings
/// read them (only this Windows user can, as with the rest of PimpMyShell's data). Every process of PimpMyShell's then goes by the
/// same ones, the settings' Test as well, so that a test goes as what it tests does. Each stream's own (`Session`) are listed
/// there too, as they come and go.
pub fn begin() {
    DAEMONS.get_or_init(made_up);
    write_logins();
}

/// The daemon's logins, and each stream's in use, into the file the settings read: `<use>\t<login>` a line, then `stream\t<what
/// for>\t<login>` for each stream's. Only the daemon's process writes it.
fn write_logins() {
    let Some(made) = DAEMONS.get() else { return };
    let mut text: String = USES.iter().zip(made).map(|((what, ..), login)| format!("{}\t{login}\n", key(*what))).collect();
    // Taken out of the list first, and gone through after it's let go: one of these may be the last of its session by the time
    // it's dropped, and its drop writes this again. They're dropped as this ends, after it's written, so that's the last word.
    let live: Vec<_> = SESSIONS
        .lock()
        .map(|mut sessions| {
            sessions.retain(|s| s.strong_count() > 0);
            sessions.iter().filter_map(std::sync::Weak::upgrade).collect()
        })
        .unwrap_or_default();
    for s in &live {
        text.push_str(&format!("stream\t{}\t{}\n", s.label, s.login));
    }
    if let Err(e) = std::fs::create_dir_all(pms_tap::data_dir()).and_then(|()| std::fs::write(logins_file(), text)) {
        crate::log(format!("the proxy's logins for this run couldn't be kept for the settings: {e}"));
    }
}

/// Each stream's own login in use now, as the running daemon lists them: what it's for, then the user name and password. None
/// if it isn't running, or streams aren't kept apart.
pub fn stream_logins(n: &crate::config::Network) -> Vec<(String, String, String)> {
    if !n.isolate || !matches!(login(n, Use::Video), Login::As { .. }) || !crate::daemon::running() {
        return Vec::new();
    }
    let text = std::fs::read_to_string(logins_file()).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let mut parts = l.strip_prefix("stream\t")?.splitn(2, '\t');
            let (label, login) = (parts.next()?, parts.next()?);
            let (user, password) = login.split_once(':')?;
            Some((label.to_string(), user.to_string(), password.to_string()))
        })
        .collect()
}

/// For the daemon, as it stops: they're no one's now.
pub fn end() {
    let _ = std::fs::remove_file(logins_file());
}

fn key(what: Use) -> &'static str {
    match what {
        Use::Live => "live",
        Use::Streams | Use::Video => "streams",
        Use::Updates => "updates",
        Use::Downloads => "downloads",
        Use::Iss => "iss",
    }
}

/// The ones the running daemon made, from its file: none if it isn't running, or the file isn't whole.
fn the_daemons() -> Option<Vec<String>> {
    if !crate::daemon::running() {
        return None;
    }
    let text = std::fs::read_to_string(logins_file()).ok()?;
    USES.iter().map(|(what, ..)| text.lines().find_map(|l| l.strip_prefix(key(*what)).and_then(|l| l.strip_prefix('\t'))).filter(|l| l.contains(':') && !l.contains(char::is_whitespace)).map(str::to_string)).collect()
}

/// The user name and password `what` goes by: the daemon's for this run; or, with no daemon running, this process's own, made up
/// the first time they're wanted and kept until it exits.
fn identity(what: Use) -> String {
    static OWN: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let at = USES.iter().position(|(u, ..)| key(*u) == key(what)).unwrap_or(0);
    if let Some(ours) = DAEMONS.get() {
        return ours[at].clone();
    }
    the_daemons().map_or_else(|| OWN.get_or_init(made_up)[at].clone(), |theirs| theirs[at].clone())
}


/// Tries the way `what` goes out: one small request to where it goes, through the proxy if that's its way, as it would go (by way of
/// pms-fetch.exe, in its sandbox if it has one, as everything else is). What came of it, in words. Takes seconds: not on a thread
/// that's drawing.
pub fn try_out(what: Use) -> Result<String, String> {
    let began = std::time::Instant::now();
    let proxied = proxy_for(what).is_some();
    let ms = || began.elapsed().as_millis();
    let (name, url) = match what {
        Use::Live => (t::TRY_PICTURES.t(), "https://static-cdn.jtvnw.net/previews-ttv/live_user_twitch-320x180.jpg"),
        // Both halves: Streamlink, then where the video comes from, the way it goes.
        Use::Streams | Use::Video => {
            crate::streamlink::try_out()?;
            crate::fetch::fetch(Use::Video, crate::fetch::Kind::Head, "https://usher.ttvnw.net/", Duration::from_secs(20), 0, 0).map_err(|e| t::TRY_HALF.with(&[("error", &e)]))?;
            let said = if proxied { t::TRY_BOTH_PROXY } else { t::TRY_BOTH_DIRECT };
            return Ok(said.with(&[("ms", &ms())]));
        }
        Use::Updates => ("GitHub", crate::update::STABLE_URL),
        Use::Downloads => ("GitHub", "https://api.github.com/"),
        Use::Iss => (t::TRY_ISS.t(), crate::iss::tlcp::SERVER),
    };
    // Any answer at all is the way working: what it says doesn't matter here.
    crate::fetch::fetch(what, crate::fetch::Kind::Head, url, Duration::from_secs(20), 0, 0).map_err(|e| t::TRY_FAILED.with(&[("error", &e)]))?;
    let said = if proxied { t::TRY_REACHED_PROXY } else { t::TRY_REACHED_DIRECT };
    Ok(said.with(&[("name", &name), ("ms", &ms())]))
}

/// Everything PimpMyShell and the programs it runs use the network for, for the settings to say: what it is, what it's part of
/// (the uses of one part go together), and all of where it goes, and when. Their words are in lang/ (`[net]`).
pub const USES: [(Use, t::Id, t::Id, t::Id); 5] = [
    (Use::Live, t::LIVE, t::PART_TWATCH, t::LIVE_ABOUT),
    (Use::Streams, t::STREAMS, t::PART_TWATCH, t::STREAMS_ABOUT),
    (Use::Iss, t::ISS, t::PART_TV, t::ISS_ABOUT),
    (Use::Updates, t::UPDATES, t::PART_PIMPMYSHELL, t::UPDATES_ABOUT),
    (Use::Downloads, t::DOWNLOADS, t::PART_DOWNLOADS, t::DOWNLOADS_ABOUT),
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
    fn each_use_shows_the_login_it_goes_as() {
        let mut n = crate::config::Network { proxy: "socks5h://me:secret@10.0.0.2:1080".into(), live_checks: true, streams: true, ..Default::default() };
        assert_eq!(login(&n, Use::Live), Login::As { user: "me".into(), password: "secret".into() });
        assert_eq!(login(&n, Use::Updates), Login::Direct, "not through the proxy");
        n.isolate = true;
        let (Login::As { user: live, .. }, Login::As { user: video, .. }, Login::As { user: streams, .. }) = (login(&n, Use::Live), login(&n, Use::Video), login(&n, Use::Streams)) else { panic!("each has one of its own") };
        assert!(live.starts_with("pms") && live != streams && video == streams, "made up, one each, the video as Streamlink");
        n.isolate = false;
        n.proxy = "socks5h://10.0.0.2:1080".into();
        assert_eq!(login(&n, Use::Live), Login::Without);
        n.proxy = "http://10.0.0.2:8080".into();
        assert_eq!(login(&n, Use::Live), Login::Nowhere);
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
        assert_eq!(route_for(Use::Streams).as_ref(), Some(&video), "but finding a stream and playing it are the same somebody");
        assert_eq!(live, route_for(Use::Live).unwrap(), "and stays that somebody");
        assert!(check(&live).is_ok());
        // Each stream is somebody of its own, from finding it to playing it; and nobody else is it.
        let (one, two) = (Session::new("1"), Session::new("2"));
        let (found, played) = (route_as(Use::Streams, Some(&one)).unwrap(), route_as(Use::Video, Some(&one)).unwrap());
        assert_eq!(found, played, "a stream's finding and playing are one somebody");
        assert!(found.starts_with("socks5h://pms") && found.ends_with("@127.0.0.1:9050") && check(&found).is_ok(), "{found}");
        assert_ne!(found, route_as(Use::Video, Some(&two)).unwrap(), "and another stream's somebody else");
        assert_ne!(found, video, "and not the use's own, which Test goes by");
        assert_eq!(route_as(Use::Video, None), Some(video.clone()));
        assert_eq!(route_as(Use::Updates, Some(&one)), None, "it doesn't send what doesn't go through the proxy there");
        assert!(!format!("{one:?}").contains(found.split_once("://").unwrap().1.split_once('@').unwrap().0), "its Debug never shows the login");
        // Without each use kept apart, the address's own login is the only one the proxy takes.
        config.network = Some(crate::config::Network { proxy: "socks5h://me:mine@127.0.0.1:9050".into(), streams: true, ..Default::default() });
        follow(&config);
        assert_eq!(route_as(Use::Video, Some(&one)).as_deref(), Some("socks5h://me:mine@127.0.0.1:9050"));
        config.network = Some(crate::config::Network { proxy: "not a proxy".into(), streams: true, isolate: true, ..Default::default() });
        follow(&config);
        assert_eq!(route_as(Use::Video, Some(&one)).as_deref(), Some(NOWHERE), "nowhere is nowhere, whoever asks");
        config.network = Some(crate::config::Network { proxy: "not a proxy".into(), streams: true, ..Default::default() });
        follow(&config);
        assert_eq!(proxy_for(Use::Video).as_deref(), Some("not a proxy"), "one that can't be used is still the way, and fails");
        assert_eq!(route_for(Use::Video).as_deref(), Some(NOWHERE), "it doesn't go without it: it goes nowhere");
        assert!(check("socks5h://127.0.0.1:9050 ").is_err() && check("socks5h://a b@127.0.0.1:9050").is_err(), "a space would break pms-fetch's line");
        follow(&crate::config::parse("").unwrap());
    }
}
