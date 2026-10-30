//! Software update, as Nagare does it: whether a newer PimpMyShell is out, and, for a copy the installer installed,
//! fetching that version's installer and running it silently - which stops the daemon, installs over it and starts it
//! again.
//!
//! What is stable enough to update to is the tag in the repository's `update/stable.txt`; on the latest channel, the
//! newest release published instead, its tag read off where GitHub's latest-release link redirects. Each release
//! carries a manifest of its installer, signed with the release key (`signing.rs`): the manifest is believed only if
//! the key signed it, names that very version and an installer named for it, and the installer is kept only if its
//! size and SHA-256 are the manifest's. The unsigned tag can at worst hold updates back.
//!
//! Every request goes to GitHub alone, over HTTPS alone - each redirect checked too - with TLS checked against
//! Mozilla's roots, bundled, timeouts, and a cap on every body's size. Nothing is sent but the requests themselves.
//! The installer is downloaded into %TEMP%\pms-update.
//!
//! PMS_UPDATE_SOURCE=FOLDER reads the files from FOLDER instead (stable.txt, latest.txt, the manifest, its
//! signature and the installer, side by side), to try all of it without publishing anything.

use std::path::{Path, PathBuf};

use crate::{signing, version};

/// Where the releases will live. (Not published yet.)
pub const REPO: &str = "bungholesurfers/PimpMyShell";
pub const RELEASES_PAGE: &str = "https://github.com/bungholesurfers/PimpMyShell/releases/latest";
const STABLE_URL: &str = "https://raw.githubusercontent.com/bungholesurfers/PimpMyShell/main/update/stable.txt";
/// The only hosts a request, or a redirect, may go to.
const HOSTS: [&str; 4] = ["github.com", "raw.githubusercontent.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];
const MAX_TAG: u64 = 64;
const MAX_MANIFEST: u64 = 4096;
const MAX_SIGNATURE: u64 = 256;
const MAX_INSTALLER: u64 = 256 * 1024 * 1024;

/// What a check found.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    UpToDate { stable: String },
    /// A newer version is out, but this copy wasn't installed by the installer (a development build).
    Available { version: String },
    /// Downloaded and checked, waiting to be run; `sha256` is the manifest's, checked again just before it runs.
    Ready { version: String, path: PathBuf, sha256: String },
    /// This version's installer ran before and PimpMyShell is still older: it didn't take. Rather than install it
    /// again and again, the check stops here and points at the releases page.
    Stuck { version: String },
}

/// Whether this copy was installed by the installer (and so can update itself): it runs from
/// %LOCALAPPDATA%\Programs\PimpMyShell.
pub fn installed() -> bool {
    let here = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    let programs = std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Programs").join("PimpMyShell"));
    matches!((here, programs), (Some(a), Some(b)) if a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()))
}

/// What a check is asked to do.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ask {
    /// Download the new version's installer too (only an installed copy does).
    pub install: bool,
    /// A check the daemon made by itself: a version whose installer ran before without taking is left alone
    /// (`Stuck`); one the user asked for tries it again.
    pub automatic: bool,
    /// Reinstall: the channel's version whatever this one is - the same, or even older - still only as its signed
    /// manifest describes it.
    pub force: bool,
    /// The newest release published, rather than the stable tag.
    pub latest: bool,
}

/// The check, and (for an installed copy, with `install`) the download. Blocking: call it from a thread of its own.
/// `progress` hears (done, total) bytes while downloading. Nothing is installed here: `install()` does that, once
/// the user has said so.
pub fn check(ask: Ask, progress: &dyn Fn(u64, u64)) -> Result<Status, String> {
    let source = Source::new();
    let which = if ask.latest { "latest release's" } else { "stable" };
    let tag = if ask.latest { source.latest()? } else { String::from_utf8(source.fetch(STABLE_URL, MAX_TAG)?).map_err(|_| "the stable tag is not text")? };
    let tag = tag.trim();
    let stable = version::parse(tag).ok_or(format!("the {which} tag {tag:?} is not a version"))?;
    let stable = format!("{}.{}.{}", stable.0, stable.1, stable.2);
    if !ask.force && !version::newer(&stable, version::CURRENT) {
        return Ok(Status::UpToDate { stable });
    }
    // Only the tag's own characters go into the URLs: v, digits and dots, which parse() has checked.
    let tag = format!("v{stable}");
    let manifest = source.fetch(&release_url(&tag, signing::MANIFEST), MAX_MANIFEST)?;
    let signature = String::from_utf8(source.fetch(&release_url(&tag, signing::SIGNATURE), MAX_SIGNATURE)?).map_err(|_| "the signature is not text")?;
    let m = signing::verify(&manifest, &signature, signing::PUBLIC_KEYS)?;
    if m.version != stable {
        return Err(format!("the release's manifest is for {}, not {stable}", m.version));
    }
    if m.size > MAX_INSTALLER {
        return Err(format!("the installer would be {} MB", m.size / 1_000_000));
    }
    if !ask.install || !installed() {
        return Ok(Status::Available { version: stable });
    }
    if ask.automatic && tried().as_deref() == Some(stable.as_str()) {
        return Ok(Status::Stuck { version: stable });
    }
    let path = download(&source, &tag, &m, progress)?;
    Ok(Status::Ready { version: stable, path, sha256: m.sha256 })
}

/// The file in the download folder naming the version whose installer last ran.
const TRIED: &str = "tried.txt";

/// The version whose installer last ran, while PimpMyShell is still older than it: that install didn't take.
pub fn tried() -> Option<String> {
    let v = std::fs::read_to_string(folder().join(TRIED)).ok()?;
    let v = v.trim().to_string();
    version::newer(&v, version::CURRENT).then_some(v)
}

/// Clears away a finished update: once PimpMyShell is the version last installed, or newer, the installer and what
/// went with it go. The daemon calls it a minute into each run (the installer that started it has exited by then).
pub fn tidy() {
    let dir = folder();
    if let Ok(v) = std::fs::read_to_string(dir.join(TRIED))
        && !version::newer(v.trim(), version::CURRENT)
    {
        clear(&dir);
    }
}

/// Whether `name` is a file the updater puts in its folder: an installer (`pms-setup-*.exe`, `.exe.part` while it
/// downloads), the log it ran with, `tried.txt`.
fn ours(name: &str) -> bool {
    name == TRIED || (name.starts_with("pms-setup-") && [".exe", ".exe.part", ".log"].iter().any(|end| name.ends_with(end)))
}

/// Empties the download folder of the updater's own files (`ours`) and removes it if that leaves it empty. Nothing
/// else in it is touched, nor anything that isn't a plain file (a folder, a link), nor anything outside it.
fn clear(dir: &Path) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        // DirEntry's file type is the entry's own: a link is a link, not what it points at.
        if ours(&entry.file_name().to_string_lossy()) && entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let _ = std::fs::remove_dir(dir);
}

/// The daemon's checks: the first a run makes waits FIRST after it starts (a random time in it, so that sign-in
/// isn't slowed and many PCs don't ask GitHub at once), the next waits EVERY after one that worked, RETRY after one
/// that failed. The last check's time is kept in `checked.txt`, so that signing in again doesn't check again.
pub const FIRST: (u64, u64) = (60, 30 * 60);
pub const EVERY: (u64, u64) = (24 * 3600, 48 * 3600);
pub const RETRY: (u64, u64) = (2 * 3600, 6 * 3600);
const CHECKED: &str = "update-checked.txt";

/// Seconds since 1970.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A random time in `range` (seconds).
pub fn random_secs((low, high): (u64, u64)) -> u64 {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    low + u64::from_le_bytes(b) % (high - low).max(1)
}

/// When the daemon's next check is due (unix seconds): a while after it starts, and no sooner than a day or two
/// after the last check that worked.
pub fn next_check() -> u64 {
    // Trying it out (PMS_UPDATE_SOURCE): the first check comes at once, whenever the last one was.
    if std::env::var_os("PMS_UPDATE_SOURCE").is_some() {
        return unix_now() + 5;
    }
    let first = unix_now() + random_secs(FIRST);
    let last = std::fs::read_to_string(pms_tap::data_dir().join(CHECKED)).ok().and_then(|t| t.trim().parse::<u64>().ok());
    // A time in the future (a clock set back) counts as no time at all.
    match last.filter(|&t| t <= unix_now()) {
        Some(t) => first.max(t + random_secs(EVERY)),
        None => first,
    }
}

/// Notes that a check worked, now.
pub fn checked() {
    let _ = std::fs::write(pms_tap::data_dir().join(CHECKED), unix_now().to_string());
}

/// A version's release page on GitHub. `version` is one `version::parse` took (digits and dots).
pub fn release_page(version: &str) -> String {
    format!("https://github.com/{REPO}/releases/tag/v{version}")
}

fn release_url(tag: &str, file: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/{tag}/{file}")
}

fn folder() -> PathBuf {
    std::env::temp_dir().join("pms-update")
}

/// Downloads the manifest's installer into %TEMP%\pms-update, checks its size and SHA-256, returns its path.
fn download(source: &Source, tag: &str, m: &signing::Manifest, progress: &dyn Fn(u64, u64)) -> Result<PathBuf, String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let _one_at_a_time = DownloadLock::take()?;
    let dir = folder();
    clear(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = dir.join(format!("{}.part", m.installer));
    let path = dir.join(&m.installer);
    let mut reader = source.open(&release_url(tag, &m.installer), m.size)?;
    let mut file = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
    let mut hash = Sha256::new();
    let (mut done, mut buf) = (0u64, vec![0u8; 64 * 1024]);
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("the download broke off: {e}"))?;
        if n == 0 {
            break;
        }
        done += n as u64;
        if done > m.size {
            return Err("the installer is larger than its manifest says".into());
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", part.display()))?;
        progress(done, m.size);
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    if done != m.size {
        return Err(format!("the installer is {done} bytes, its manifest says {}", m.size));
    }
    if signing::hex(&hash.finalize()) != m.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err("the installer's SHA-256 is not its manifest's".into());
    }
    std::fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// The download folder is one: the daemon and the settings could otherwise each clear it under the other's download.
/// A named mutex for the session, held while a download runs; the second to come waits (up to the downloads'
/// own time limit) and then downloads afresh.
struct DownloadLock(windows::Win32::Foundation::HANDLE);

impl DownloadLock {
    fn take() -> Result<DownloadLock, String> {
        use windows::Win32::Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
        let handle = unsafe { CreateMutexW(None, false, windows_core::w!(r"Local\PimpMyShellUpdateDownload")) }.map_err(|e| format!("the download's lock: {e}"))?;
        let lock = DownloadLock(handle);
        // An abandoned mutex (a process died mid-download) is ours all the same: the folder is cleared first anyway.
        match unsafe { WaitForSingleObject(handle, 600_000) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(lock),
            _ => Err("another download of the update is still running".into()),
        }
    }
}

impl Drop for DownloadLock {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::ReleaseMutex;
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

/// Runs the downloaded installer of `version` silently, on its own: it stops the daemon, installs, and starts it
/// again. Its log goes beside it, and `tried` names the version first. The file is hashed once more first, against
/// the signed manifest's `sha256`: it sat in %TEMP% since the download, and is run only as it was checked. The
/// updater's own test variable isn't handed on.
pub fn install(path: &Path, version: &str, sha256: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    // Held open, sharing only reading, from the hash until the installer has started: nothing can write it between.
    let mut held = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    held.read_to_end(&mut bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if signing::hex(&Sha256::digest(&bytes)) != sha256 {
        drop(held);
        let _ = std::fs::remove_file(path);
        return Err("the downloaded installer changed since it was checked; it was removed".into());
    }
    std::fs::write(folder().join(TRIED), version).map_err(|e| format!("{}: {e}", folder().display()))?;
    let log = path.with_extension("log");
    let started = std::process::Command::new(path)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"])
        .arg(format!("/LOG={}", log.display()))
        .env_remove("PMS_UPDATE_SOURCE")
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start the installer: {e}"));
    drop(held);
    started
}

/// Where the files come from: GitHub, or a folder (PMS_UPDATE_SOURCE).
enum Source {
    Web(ureq::Agent),
    Folder(PathBuf),
}

impl Source {
    fn new() -> Self {
        match std::env::var_os("PMS_UPDATE_SOURCE") {
            Some(dir) => Source::Folder(PathBuf::from(dir)),
            None => Source::Web(agent()),
        }
    }

    fn fetch(&self, url: &str, limit: u64) -> Result<Vec<u8>, String> {
        use std::io::Read;
        let mut out = Vec::new();
        self.open(url, limit)?.read_to_end(&mut out).map_err(|e| format!("reading {}: {e}", host(url)))?;
        Ok(out)
    }

    fn latest(&self) -> Result<String, String> {
        match self {
            Source::Web(agent) => latest_tag(agent),
            Source::Folder(_) => String::from_utf8(self.fetch("latest.txt", MAX_TAG)?).map_err(|_| "latest.txt is not text".into()),
        }
    }

    fn open(&self, url: &str, limit: u64) -> Result<Box<dyn std::io::Read + Send>, String> {
        match self {
            Source::Web(agent) => get(agent, url, limit),
            Source::Folder(dir) => {
                use std::io::Read;
                let path = dir.join(url.rsplit('/').next().unwrap_or_default());
                let file = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(Box::new(file.take(limit + 1)))
            }
        }
    }
}

fn host(url: &str) -> &str {
    url.strip_prefix("https://").unwrap_or(url).split(['/', ':', '?']).next().unwrap_or_default()
}

/// Whether a request (or redirect) to `url` may be made: HTTPS, no user or port, one of GitHub's hosts.
pub fn allowed(url: &str) -> Result<(), String> {
    let Some(rest) = url.strip_prefix("https://") else { return Err(format!("refused a request that is not HTTPS: {url}")) };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') || authority.contains(':') {
        return Err(format!("refused a request with a user or a port: {url}"));
    }
    if !HOSTS.contains(&authority.to_ascii_lowercase().as_str()) {
        return Err(format!("refused a request to {authority}, which is not GitHub"));
    }
    Ok(())
}

fn agent() -> ureq::Agent {
    use std::time::Duration;
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::WebPki).build();
    ureq::Agent::config_builder()
        .https_only(true)
        .tls_config(tls)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .user_agent(format!("PimpMyShell/{} (update check)", version::CURRENT))
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_global(Some(Duration::from_secs(600)))
        .build()
        .into()
}

/// GET `url`, following up to five redirects by hand, each checked by `allowed`.
fn get(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Box<dyn std::io::Read + Send>, String> {
    let mut url = url.to_string();
    for _ in 0..=5 {
        allowed(&url)?;
        let response = agent.get(&url).call().map_err(|e| format!("{}: {e}", host(&url)))?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            let location = response.headers().get("location").and_then(|l| l.to_str().ok()).ok_or(format!("{} redirected nowhere", host(&url)))?;
            url = if location.starts_with('/') { format!("https://{}{location}", host(&url)) } else { location.to_string() };
            continue;
        }
        if status != 200 {
            return Err(format!("{} answered {status}", host(&url)));
        }
        if response.headers().get("content-length").and_then(|l| l.to_str().ok()).and_then(|l| l.parse::<u64>().ok()).is_some_and(|n| n > limit) {
            return Err(format!("{} sent more than expected", host(&url)));
        }
        return Ok(Box::new(response.into_body().into_with_config().limit(limit.saturating_add(1)).reader()));
    }
    Err("too many redirects".into())
}

/// The newest release's tag, from where GitHub's latest-release link redirects (a HEAD request).
fn latest_tag(agent: &ureq::Agent) -> Result<String, String> {
    allowed(RELEASES_PAGE)?;
    let response = agent.head(RELEASES_PAGE).call().map_err(|e| format!("{}: {e}", host(RELEASES_PAGE)))?;
    let status = response.status().as_u16();
    if !(300..400).contains(&status) {
        return Err(format!("{} answered {status} for the latest release", host(RELEASES_PAGE)));
    }
    let location = response.headers().get("location").and_then(|l| l.to_str().ok()).ok_or("github.com named no latest release")?;
    latest_from(location)
}

fn latest_from(location: &str) -> Result<String, String> {
    let location = location.strip_prefix("https://github.com").unwrap_or(location);
    let prefix = format!("/{REPO}/releases/tag/");
    location
        .strip_prefix(prefix.as_str())
        .filter(|tag| !tag.is_empty() && !tag.contains(['/', '?', '#']))
        .map(str::to_string)
        .ok_or("no release is published yet".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest() {
        assert_eq!(latest_from("https://github.com/bungholesurfers/PimpMyShell/releases/tag/v0.1.3").as_deref(), Ok("v0.1.3"));
        assert!(latest_from("https://github.com/bungholesurfers/PimpMyShell/releases").is_err());
        assert!(latest_from("https://github.com/someone/Else/releases/tag/v9.9.9").is_err());
    }

    #[test]
    fn only_ours_cleared() {
        for n in ["tried.txt", "pms-setup-0.1.1.exe", "pms-setup-0.1.1.exe.part", "pms-setup-0.1.1.log"] {
            assert!(ours(n), "{n}");
        }
        for n in ["pms.toml", "setup.exe", "pms-setup-0.1.1.txt", "x-pms-setup-0.1.1.exe", "tried.txt.bak", "pms-setup-0.1.1.exe.lnk"] {
            assert!(!ours(n), "{n}");
        }
        let dir = std::env::temp_dir().join(format!("pms-clear-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("pms-setup-9.9.9.exe")).unwrap(); // a folder named like an installer stays
        std::fs::write(dir.join("pms-setup-9.9.9.log"), "x").unwrap();
        std::fs::write(dir.join("keep.txt"), "x").unwrap();
        clear(&dir);
        assert!(!dir.join("pms-setup-9.9.9.log").exists());
        assert!(dir.join("keep.txt").exists() && dir.join("pms-setup-9.9.9.exe").is_dir());
        std::fs::remove_file(dir.join("keep.txt")).unwrap();
        std::fs::remove_dir(dir.join("pms-setup-9.9.9.exe")).unwrap();
        clear(&dir);
        assert!(!dir.exists());
    }

    #[test]
    fn hosts() {
        assert!(allowed("https://github.com/bungholesurfers/PimpMyShell/releases/download/v0.1.1/x").is_ok());
        assert!(allowed("http://github.com/x").is_err());
        assert!(allowed("https://github.com.evil.example/x").is_err());
        assert!(allowed("https://github.com:8443/x").is_err());
        assert!(allowed("https://user@github.com/x").is_err());
    }
}
