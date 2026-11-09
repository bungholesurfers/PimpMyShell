//! Streamlink, which turns a channel's name into a stream to play. It's fetched on request from the project's own Windows builds,
//! and kept in a folder of its own. It isn't in the installer, and nothing runs it until there's a favorite to play.
//!
//! It reads Twitch's website, which changes without warning, and a new Streamlink is then what's needed, so a build pinned here
//! could leave the sets dark until the next PimpMyShell. So it takes the newest stable release, and takes it only if GitHub says
//! that GitHub's runners uploaded it (`github-actions[bot]`, not a person), and it hashes to the digest GitHub keeps for it. The
//! pinned build is what's fetched if GitHub won't say what's newest.
//!
//! Those checks prove it's the file the project published, not that it's safe, so nothing new comes in unasked. Once it's here
//! it's only replaced when you say: if it fails on a channel that's live, all that happens by itself is finding out whether
//! there's a newer one, and saying so (`twatch/mod.rs`, `notices::newer_streamlink`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::wads::Download;

/// What GitHub says about the newest stable release.
const NEWEST: &str = "https://api.github.com/repos/streamlink/windows-builds/releases/latest";
/// Where the releases' files are, and nowhere else will do.
const FILES: &str = "https://github.com/streamlink/windows-builds/releases/download/";
/// Where it comes from, for the settings to say.
pub const URL: &str = "https://github.com/streamlink/windows-builds/releases/latest";
/// Who has to have uploaded it: GitHub's runners, which built it from the project's source.
const UPLOADER: &str = "github-actions[bot]";
/// The most that could be a real build.
const BIGGEST: u64 = 400 << 20;
/// How long it gets to find a stream.
const PATIENCE: Duration = Duration::from_secs(45);

/// A build, and exactly what it should be.
#[derive(Clone, PartialEq, Debug)]
struct Build {
    version: String,
    file: String,
    url: String,
    size: u64,
    sha256: String,
}

/// The build that's known to work with this version, for when GitHub won't say what's newest.
fn known() -> Build {
    Build {
        version: "8.6.3-1".into(),
        file: "streamlink-8.6.3-1-py315-x86_64.zip".into(),
        url: format!("{FILES}8.6.3-1/streamlink-8.6.3-1-py315-x86_64.zip"),
        size: 84_611_513,
        sha256: "3b1f1470d4b135080a8107ca2917379ca3ff5acadbcdc215568b79eea1450497".into(),
    }
}

/// The zip in a release, as GitHub describes it: only one that passes all of this will be fetched.
fn read_release(json: &str) -> Result<Build, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("GitHub's answer: {e}"))?;
    let tag = v["tag_name"].as_str().filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))).ok_or("no version in GitHub's answer")?;
    if v["prerelease"].as_bool() != Some(false) || v["draft"].as_bool() != Some(false) {
        return Err(format!("{tag} isn't a stable release"));
    }
    let start = format!("streamlink-{tag}-");
    let assets = v["assets"].as_array().ok_or("no files in GitHub's answer")?;
    let asset = assets.iter().find(|a| a["name"].as_str().is_some_and(|n| n.starts_with(&start) && n.ends_with("-x86_64.zip"))).ok_or_else(|| format!("{tag} has no 64-bit zip"))?;
    if asset["uploader"]["login"].as_str() != Some(UPLOADER) {
        return Err(format!("{tag} wasn't uploaded by GitHub's runners"));
    }
    let sha256 = asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).filter(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit())).ok_or_else(|| format!("{tag} has no digest"))?;
    let (file, size) = (asset["name"].as_str().unwrap_or_default(), asset["size"].as_u64().unwrap_or(0));
    let url = asset["browser_download_url"].as_str().filter(|u| *u == format!("{FILES}{tag}/{file}")).ok_or_else(|| format!("{tag} isn't where it should be"))?;
    if size == 0 || size > BIGGEST {
        return Err(format!("{tag} is the wrong size"));
    }
    Ok(Build { version: tag.to_string(), file: file.to_string(), url: url.to_string(), size, sha256: sha256.to_ascii_lowercase() })
}

fn newest() -> Result<Build, String> {
    let response = crate::wads::agent(crate::net::Use::Downloads, Duration::from_secs(20), 3).get(NEWEST).header("Accept", "application/vnd.github+json").call().map_err(|e| format!("{NEWEST}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("{NEWEST}: {}", response.status()));
    }
    read_release(&response.into_body().read_to_string().map_err(|e| format!("{NEWEST}: {e}"))?)
}

/// A stream it found.
#[derive(Clone, PartialEq, Debug)]
pub struct Stream {
    /// The playlist of every rendition.
    pub master: String,
    pub title: String,
    pub category: String,
}

/// What it said about a channel.
#[derive(Clone, PartialEq, Debug)]
pub enum Asked {
    Live(Stream),
    Offline,
    Failed(String),
}

fn dir() -> PathBuf {
    pms_tap::data_dir().join("streamlink")
}

/// The program in `folder`: the build keeps everything in one folder named for its version.
fn find(folder: &Path) -> Option<PathBuf> {
    std::fs::read_dir(folder).ok()?.flatten().map(|e| e.path().join("bin").join("streamlink.exe")).find(|p| p.is_file())
}

/// The program, if it's been fetched.
pub fn exe() -> Option<PathBuf> {
    find(&dir())
}

/// Which version the folder a build keeps everything in is, by its name.
fn version_of(folder: &str) -> Option<&str> {
    folder.strip_prefix("streamlink-")?.split("-py").next().filter(|v| !v.is_empty())
}

/// Which version is here.
pub fn version() -> Option<String> {
    let bin = exe()?;
    let folder = bin.parent()?.parent()?.file_name()?.to_str()?;
    version_of(folder).map(str::to_string)
}

/// The newest version there is, if it isn't the one that's here. Only asks: nothing's fetched. Takes seconds: not on a thread
/// that's drawing.
pub fn newer() -> Result<Option<String>, String> {
    let build = newest()?;
    Ok((version().as_deref() != Some(build.version.as_str())).then_some(build.version))
}

/// Fetches the newest there is, replacing any that's here: false if it was already here. For when you've asked. Takes minutes:
/// not on a thread that's drawing.
pub fn install(progress: &dyn Fn(u64, u64)) -> Result<bool, String> {
    let have = version();
    let build = match newest() {
        Ok(build) => build,
        // With one here that's the one to keep. Without one, the one we know is better than none.
        Err(e) if have.is_some() => return Err(e),
        Err(e) => {
            crate::log(format!("Streamlink: {e}, so the build we know"));
            known()
        }
    };
    if have.as_deref() == Some(build.version.as_str()) {
        return Ok(false);
    }
    let bytes = crate::wads::fetch(&Download { file: &build.file, url: &build.url, size: build.size, sha256: &build.sha256 }, progress)?;
    let (folder, fresh) = (dir(), dir().with_file_name("streamlink.new"));
    let _ = std::fs::remove_dir_all(&fresh);
    std::fs::create_dir_all(&fresh).map_err(|e| format!("{}: {e}", fresh.display()))?;
    let unpacked = zip::ZipArchive::new(std::io::Cursor::new(bytes)).and_then(|mut zip| zip.extract(&fresh)).map_err(|e| e.to_string());
    if find(&fresh).is_none() {
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(unpacked.err().unwrap_or_else(|| "there's no streamlink.exe in it".into()));
    }
    remove()?;
    std::fs::rename(&fresh, &folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    crate::log(format!("Streamlink {} is here", build.version));
    Ok(true)
}

/// Deletes it.
pub fn remove() -> Result<(), String> {
    match std::fs::remove_dir_all(dir()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// What `streamlink --json` printed.
fn read(json: &str) -> Asked {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Asked::Failed("it said something that isn't JSON".into()) };
    if let Some(error) = v["error"].as_str() {
        return if error.contains("No playable streams") { Asked::Offline } else { Asked::Failed(error.to_string()) };
    }
    let Some(streams) = v["streams"].as_object().filter(|s| !s.is_empty()) else { return Asked::Offline };
    let master = streams.values().find_map(|s| s["master"].as_str());
    let best = streams.get("best").and_then(|s| s["url"].as_str());
    let Some(master) = master.or(best) else { return Asked::Failed("no address for the stream".into()) };
    let text = |key: &str| v["metadata"][key].as_str().unwrap_or_default().to_string();
    Asked::Live(Stream { master: master.to_string(), title: text("title"), category: text("category") })
}

/// Whether it can reach Twitch, the way it goes: Twitch's own channel, live or not, is an answer either way.
pub fn try_out() -> Result<(), String> {
    if exe().is_none() {
        return Err("Streamlink isn't installed. It's under Twatch TV.".into());
    }
    match ask("twitch") {
        Asked::Live(_) | Asked::Offline => Ok(()),
        Asked::Failed(why) => Err(format!("That didn't work: {why}")),
    }
}

/// Asks it about the Twitch channel `handle`, which must be one `live::valid` passes. Takes seconds: not on a thread that's drawing.
pub fn ask(handle: &str) -> Asked {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Some(exe) = exe() else { return Asked::Failed("Streamlink isn't here".into()) };
    let mut command = std::process::Command::new(exe);
    // Through the proxy, if that's where finding streams goes: Streamlink takes socks5 and socks5h addresses as they are.
    if let Some(proxy) = crate::net::route_for(crate::net::Use::Streams) {
        command.args(["--http-proxy", &proxy]);
    }
    let mut child = match command
        .args(["--json", &format!("https://www.twitch.tv/{handle}")])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return Asked::Failed(e.to_string()),
    };
    let began = Instant::now();
    let Some(mut out) = child.stdout.take() else { return Asked::Failed("no output".into()) };
    let reading = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut out, &mut text);
        text
    });
    while matches!(child.try_wait(), Ok(None)) {
        if began.elapsed() > PATIENCE {
            let _ = child.kill();
            let _ = child.wait();
            return Asked::Failed("it took too long".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    read(&reading.join().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool, name: &str, uploader: &str, digest: &str, url: &str) -> String {
        let asset = serde_json::json!({ "name": name, "size": 84611513, "digest": digest, "uploader": { "login": uploader }, "browser_download_url": url });
        let other = serde_json::json!({ "name": format!("streamlink-{tag}-py315-x86_64.exe"), "size": 1, "digest": "sha256:0", "uploader": { "login": "someone" }, "browser_download_url": "https://example.com/a" });
        serde_json::json!({ "tag_name": tag, "prerelease": prerelease, "draft": false, "assets": [other, asset] }).to_string()
    }

    const DIGEST: &str = "sha256:3b1f1470d4b135080a8107ca2917379ca3ff5acadbcdc215568b79eea1450497";
    const ZIP: &str = "streamlink-9.0.0-1-py315-x86_64.zip";
    const AT: &str = "https://github.com/streamlink/windows-builds/releases/download/9.0.0-1/streamlink-9.0.0-1-py315-x86_64.zip";

    #[test]
    fn a_stable_release_built_by_the_runners_is_taken() {
        let build = read_release(&release("9.0.0-1", false, ZIP, UPLOADER, DIGEST, AT)).unwrap();
        assert_eq!(build, Build { version: "9.0.0-1".into(), file: ZIP.into(), url: AT.into(), size: 84_611_513, sha256: DIGEST[7..].into() });
    }

    #[test]
    fn nothing_else_is() {
        assert!(read_release(&release("9.0.0-1", true, ZIP, UPLOADER, DIGEST, AT)).is_err(), "a prerelease");
        assert!(read_release(&release("9.0.0-1", false, ZIP, "somebody", DIGEST, AT)).is_err(), "uploaded by a person");
        assert!(read_release(&release("9.0.0-1", false, ZIP, UPLOADER, "", AT)).is_err(), "no digest");
        assert!(read_release(&release("9.0.0-1", false, ZIP, UPLOADER, "sha256:abc", AT)).is_err(), "half a digest");
        assert!(read_release(&release("9.0.0-1", false, ZIP, UPLOADER, DIGEST, "https://example.com/streamlink.zip")).is_err(), "somewhere else");
        assert!(read_release(&release("9.0.0-1", false, "streamlink-9.0.0-1-py315-arm64.zip", UPLOADER, DIGEST, AT)).is_err(), "the wrong machine");
        assert!(read_release(&release("../evil", false, ZIP, UPLOADER, DIGEST, AT)).is_err(), "an odd version");
        assert!(read_release("{\"message\":\"API rate limit exceeded\"}").is_err());
    }

    #[test]
    fn the_folder_says_which_version_it_is() {
        assert_eq!(version_of("streamlink-8.6.3-1-py315-x86_64"), Some("8.6.3-1"));
        assert_eq!(version_of("something"), None);
    }

    #[test]
    fn a_live_channel_gives_its_master_playlist_and_what_is_on() {
        let json = r#"{"plugin":"twitch","metadata":{"id":"1","author":"crab","category":"Just Chatting","title":"pinching"},
            "streams":{"audio_only":{"type":"hls","url":"https://a/audio.m3u8","master":"https://usher/master.m3u8"},
                       "best":{"type":"hls","url":"https://a/best.m3u8","master":"https://usher/master.m3u8"}}}"#;
        assert_eq!(read(json), Asked::Live(Stream { master: "https://usher/master.m3u8".into(), title: "pinching".into(), category: "Just Chatting".into() }));
    }

    #[test]
    fn without_a_master_the_best_stream_will_do() {
        let json = r#"{"streams":{"best":{"type":"hls","url":"https://a/best.m3u8"}}}"#;
        assert!(matches!(read(json), Asked::Live(s) if s.master == "https://a/best.m3u8"));
    }

    #[test]
    fn an_offline_channel_is_not_a_failure() {
        assert_eq!(read(r#"{"error":"No playable streams found on this URL: https://www.twitch.tv/crab"}"#), Asked::Offline);
        assert_eq!(read(r#"{"streams":{}}"#), Asked::Offline);
    }

    #[test]
    fn anything_else_is() {
        assert!(matches!(read(r#"{"error":"Unable to open URL"}"#), Asked::Failed(_)));
        assert!(matches!(read("Traceback"), Asked::Failed(_)));
    }
}
