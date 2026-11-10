//! The game data a screen can play, kept in `%LOCALAPPDATA%\PimpMyShell\wads` (it's big, and it's this machine's). None comes
//! with PimpMyShell. Each is fetched only when asked for, and kept only if it hashes to exactly what's expected, whoever
//! served it:
//!
//! - Freedoom: free content under a BSD licence, from its own release on GitHub, with its licence kept beside it.
//! - The Doom shareware episode: id Software's, unmodified, as Debian ships it. Its licence comes first (`SHAREWARE_LICENSE`).
//! - Your own: any WAD, copied in.
//!
//! PMS_WAD_SOURCE=FOLDER reads the files from a folder instead, for trying this out. PMS_POOM_HOME=FOLDER keeps the console's
//! games, settings and saves there instead of in `vidya\poom` (PGO training does, to leave yours alone).

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// id Software's licence for the shareware episode, as it came with it.
pub const SHAREWARE_LICENSE: &str = include_str!("../../wads/doom-shareware-license.txt");

const FREEDOOM: Download<'static> = Download {
    file: "freedoom-0.13.0.zip",
    url: "https://github.com/freedoom/freedoom/releases/download/v0.13.0/freedoom-0.13.0.zip",
    size: 24_143_781,
    sha256: "3f9b264f3e3ce503b4fb7f6bdcb1f419d93c7b546f4df3e874dd878db9688f59",
};
const FREEDOOM_WAD: &str = "freedoom1.wad";
const FREEDOOM_LICENCE: &str = "freedoom-COPYING.txt";
const FREEDOOM_WAD_SHA256: &str = "7323bcc168c5a45ff10749b339960e98314740a734c30d4b9f3337001f9e703d";
const SHAREWARE: Download<'static> = Download {
    file: "doom-wad-shareware_1.9.fixed-5_all.deb",
    url: "https://deb.debian.org/debian/pool/non-free/d/doom-wad-shareware/doom-wad-shareware_1.9.fixed-5_all.deb",
    size: 1_366_368,
    sha256: "5802f176c0303e228095b5312def53de602781cf4c53e79842257484a0d9e938",
};
const SHAREWARE_WAD: &str = "doom1.wad";
const SHAREWARE_WAD_SHA256: &str = "1d7d43be501e67d927e415e0b8f3e29c3bf33075e859721816f652a526cac771";

/// A file on the net, and exactly what it should be.
pub(crate) struct Download<'a> {
    pub file: &'a str,
    pub url: &'a str,
    pub size: u64,
    pub sha256: &'a str,
}

/// Where `wad` comes from, for the settings to say.
pub fn url(wad: Wad) -> &'static str {
    match wad {
        Wad::Freedoom => FREEDOOM.url,
        Wad::Shareware => SHAREWARE.url,
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Wad {
    Freedoom,
    Shareware,
}

fn local() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("PimpMyShell")
}

/// Where games of every kind go, each in a folder of its own.
pub fn vidya() -> PathBuf {
    local().join("vidya")
}

/// Everything of the console's: its games in `wads`, and its settings and saved games beside them.
pub fn home() -> PathBuf {
    elsewhere().unwrap_or_else(|| vidya().join("poom"))
}

/// The console's home when it's been put somewhere else (PMS_POOM_HOME): there, and nowhere an earlier version kept things.
fn elsewhere() -> Option<PathBuf> {
    std::env::var_os("PMS_POOM_HOME").map(PathBuf::from)
}

/// The first of `places` that's a folder, or the first of them, where none is yet: where something is, while it hasn't moved.
fn first_there(places: [PathBuf; 3]) -> PathBuf {
    places.iter().find(|p| p.is_dir()).unwrap_or(&places[0]).clone()
}

/// Where the console keeps its boot animation's shader compiled, so that it's only compiled once: its own folder among the
/// games', beside its game program's. Emptying it costs a moment the next time the console comes on, and no more.
pub fn shaders() -> PathBuf {
    let home = home();
    home.parent().map_or_else(|| home.join("shaders"), |games| games.join("pimpbox").join("shaders"))
}

/// Where WADs are kept. While they're still where an earlier version kept them, and haven't moved (`move_old`), there.
pub fn dir() -> PathBuf {
    if elsewhere().is_some() {
        return home().join("wads");
    }
    first_there([home().join("wads"), local().join("poom").join("wads"), local().join("wads")])
}

/// Where a game keeps its settings and saved games: the console's own folder. The same for where it was before.
pub fn saves() -> PathBuf {
    if elsewhere().is_some() {
        return home();
    }
    first_there([home(), local().join("poom"), local().join("doom")])
}

/// Moves the console's folders to where they go from 0.1.5 on, `vidya\poom`: `doom` (its saves) and, briefly, `poom` become it,
/// and `wads` goes inside it. Only renames: nothing is copied, and nothing already in the new place is touched. What can't move
/// stays where it was, and is still found there. Before the console starts: a game holds its WAD open.
pub fn move_old() {
    move_into(&local().join("poom"), &home());
    move_into(&local().join("doom"), &home());
    move_into(&local().join("wads"), &home().join("wads"));
}

/// Everything in `from` into `to`, unless something by its name is there already; then `from`, if that's left it empty.
fn move_into(from: &Path, to: &Path) {
    if !from.is_dir() {
        return;
    }
    // The whole folder at once when there's nothing in the way: one rename, and the quickest.
    if !to.exists() {
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::rename(from, to) {
            Ok(()) => return crate::log(format!("moved {} to {}", from.display(), to.display())),
            Err(e) => crate::log(format!("couldn't move {} to {}: {e}; moving what's in it instead", from.display(), to.display())),
        }
    }
    let _ = std::fs::create_dir_all(to);
    let Ok(entries) = std::fs::read_dir(from) else { return };
    for entry in entries.flatten() {
        let target = to.join(entry.file_name());
        if target.exists() {
            crate::log(format!("left {} where it is: there's one by that name in {}", entry.path().display(), to.display()));
        } else if let Err(e) = std::fs::rename(entry.path(), &target) {
            crate::log(format!("couldn't move {}: {e}", entry.path().display()));
        }
    }
    // Only an empty folder goes this way.
    let _ = std::fs::remove_dir(from);
}

/// The WADs there are, by file name.
pub fn list() -> Vec<String> {
    list_in(&dir())
}

fn list_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = entries.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.to_ascii_lowercase().ends_with(".wad")).collect();
    names.sort_by_key(|n| n.to_ascii_lowercase());
    names
}

fn choice_file() -> PathBuf {
    dir().join("chosen.txt")
}

/// Plays `name` from now on.
pub fn choose(name: &str) -> Result<(), String> {
    std::fs::write(choice_file(), name).map_err(|e| e.to_string())
}

/// The one to play: the one chosen, while it's there; else the real thing over the free one, a whole game over an episode, then
/// by name.
pub fn chosen() -> Option<PathBuf> {
    let names = list();
    let wanted = std::fs::read_to_string(choice_file()).unwrap_or_default();
    names.iter().find(|n| n.eq_ignore_ascii_case(wanted.trim())).or_else(|| pick(&names)).map(|n| dir().join(n))
}

/// What to call a game.
pub fn title(file: &str) -> String {
    match file.to_ascii_lowercase().as_str() {
        "freedoom1.wad" => "Freedoom: Phase 1".into(),
        "freedoom2.wad" => "Freedoom: Phase 2".into(),
        "doom1.wad" => "Doom (shareware episode)".into(),
        "doom.wad" => "Doom".into(),
        "doom2.wad" => "Doom II".into(),
        "plutonia.wad" => "Final Doom: The Plutonia Experiment".into(),
        "tnt.wad" => "Final Doom: TNT: Evilution".into(),
        _ => file.into(),
    }
}

fn pick(names: &[String]) -> Option<&String> {
    const FIRST: [&str; 6] = ["doom2.wad", "doom.wad", "doom1.wad", "freedoom2.wad", "freedoom1.wad", ""];
    let rank = |n: &String| FIRST.iter().position(|f| n.eq_ignore_ascii_case(f)).unwrap_or(5);
    // A WAD of your own goes first: you put it there.
    names.iter().find(|n| rank(n) == 5).or_else(|| names.iter().min_by_key(|n| rank(n)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What `d` says it is, checked.
/// Something to make requests for `what` with (net.rs), through the proxy if that's what's wanted.
pub(crate) fn agent(what: crate::net::Use, patience: std::time::Duration, redirects: u32) -> ureq::Agent {
    crate::net::agent(what, patience, redirects, &format!("PimpMyShell/{}", crate::version::CURRENT))
}

pub(crate) fn fetch(d: &Download, progress: &dyn Fn(u64, u64)) -> Result<Vec<u8>, String> {
    use std::time::Duration;
    let mut reader: Box<dyn Read> = match std::env::var_os("PMS_WAD_SOURCE") {
        Some(folder) => Box::new(std::fs::File::open(Path::new(&folder).join(d.file)).map_err(|e| format!("{}: {e}", d.file))?),
        None => {
            let response = agent(crate::net::Use::Downloads, Duration::from_secs(600), 3).get(d.url).call().map_err(|e| format!("{}: {e}", d.url))?;
            if !response.status().is_success() {
                return Err(format!("{}: {}", d.url, response.status()));
            }
            Box::new(response.into_body().into_reader())
        }
    };
    let mut bytes = Vec::with_capacity(d.size as usize);
    let mut chunk = vec![0; 256 * 1024];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| format!("{}: {e}", d.url))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() as u64 > d.size {
            return Err("that's not the file it should be: it's too big".into());
        }
        progress(bytes.len() as u64, d.size);
    }
    if hex(&Sha256::digest(&bytes)) != d.sha256 {
        return Err("that's not the file it should be: its hash doesn't match".into());
    }
    Ok(bytes)
}

/// A member of an `ar` archive (what a .deb is), by the start of its name.
fn ar_member<'a>(archive: &'a [u8], name: &str) -> Option<&'a [u8]> {
    let mut at = archive.strip_prefix(b"!<arch>\n")?;
    while at.len() >= 60 {
        let (head, rest) = at.split_at(60);
        let size: usize = std::str::from_utf8(&head[48..58]).ok()?.trim().parse().ok()?;
        let body = rest.get(..size)?;
        if head[..16].starts_with(name.as_bytes()) {
            return Some(body);
        }
        at = rest.get(size + (size & 1)..).unwrap_or_default();
    }
    None
}

/// A file in a tar archive, by the end of its path.
fn tar_file<'a>(tar: &'a [u8], name: &str) -> Option<&'a [u8]> {
    let mut at = 0;
    while at + 512 <= tar.len() {
        let head = &tar[at..at + 512];
        if head[0] == 0 {
            return None;
        }
        let path = head[..100].split(|b| *b == 0).next()?;
        let size = usize::from_str_radix(std::str::from_utf8(&head[124..135]).ok()?.trim_matches(['\0', ' ']), 8).ok()?;
        if path.ends_with(name.as_bytes()) {
            return tar.get(at + 512..at + 512 + size);
        }
        at += 512 + size.div_ceil(512) * 512;
    }
    None
}

/// The WAD inside Debian's package.
fn unpack_shareware(deb: &[u8]) -> Result<Vec<u8>, String> {
    let data = ar_member(deb, "data.tar.xz").ok_or("no data in the package")?;
    let mut tar = Vec::new();
    lzma_rs::xz_decompress(&mut std::io::Cursor::new(data), &mut tar).map_err(|e| format!("unpacking it: {e:?}"))?;
    let wad = tar_file(&tar, SHAREWARE_WAD).ok_or("no WAD in the package")?;
    if hex(&Sha256::digest(wad)) != SHAREWARE_WAD_SHA256 {
        return Err("that's not the WAD it should be".into());
    }
    Ok(wad.to_vec())
}

/// A file in a zip, by the end of its path.
pub(crate) fn zip_file(zip: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| e.to_string())?;
    // A name that won't read can't be the one wanted.
    let path = archive.file_names().filter_map(Result::ok).find(|n| n.ends_with(name)).map(|n| n.into_owned()).ok_or_else(|| format!("no {name} in it"))?;
    let mut file = archive.by_name(&path).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// The WAD in Freedoom's release, kept with the licence it has to stay with.
fn unpack_freedoom(zip: &[u8]) -> Result<PathBuf, String> {
    let wad = zip_file(zip, FREEDOOM_WAD)?;
    if hex(&Sha256::digest(&wad)) != FREEDOOM_WAD_SHA256 {
        return Err("that's not the WAD it should be".into());
    }
    let licence = zip_file(zip, "COPYING.txt")?;
    keep(FREEDOOM_LICENCE, &licence)?;
    keep(FREEDOOM_WAD, &wad)
}

fn keep(name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let dir = dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (path, part) = (dir.join(name), dir.join(format!("{name}.part")));
    std::fs::write(&part, bytes).map_err(|e| format!("{}: {e}", part.display()))?;
    std::fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Fetches `wad`. For the shareware, its licence has to have been accepted first.
pub fn get(wad: Wad, progress: &dyn Fn(u64, u64)) -> Result<PathBuf, String> {
    match wad {
        Wad::Freedoom => unpack_freedoom(&fetch(&FREEDOOM, progress)?),
        Wad::Shareware => keep(SHAREWARE_WAD, &unpack_shareware(&fetch(&SHAREWARE, progress)?)?),
    }
}

/// Copies a WAD of your own in. It has to be one: they start `IWAD` or `PWAD`.
pub fn add(path: &Path) -> Result<PathBuf, String> {
    let mut magic = [0; 4];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).map_err(|e| format!("{}: {e}", path.display()))?;
    if &magic != b"IWAD" {
        return Err("that isn't a game's WAD (an IWAD)".into());
    }
    let name = path.file_name().and_then(|n| n.to_str()).ok_or("that file's name won't do")?;
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    keep(name, &bytes)
}

/// Removes a WAD, by file name.
pub fn remove(name: &str) -> Result<(), String> {
    if name.contains(['/', '\\']) || !name.to_ascii_lowercase().ends_with(".wad") {
        return Err(format!("{name}: not a WAD"));
    }
    std::fs::remove_file(dir().join(name)).map_err(|e| format!("{name}: {e}"))?;
    if name.eq_ignore_ascii_case(FREEDOOM_WAD) {
        let _ = std::fs::remove_file(dir().join(FREEDOOM_LICENCE));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zip as releases come: deflated, in a folder named for its version, as Freedoom's and Streamlink's are.
    fn a_release() -> (Vec<u8>, Vec<u8>) {
        use std::io::Write;
        let wad: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8 ^ (i / 997) as u8).collect();
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("freedoom-0.13.0/freedoom1.wad", deflated).unwrap();
        zip.write_all(&wad).unwrap();
        zip.start_file("freedoom-0.13.0/COPYING.txt", deflated).unwrap();
        zip.write_all(b"the licence").unwrap();
        (zip.finish().unwrap().into_inner(), wad)
    }

    #[test]
    fn a_file_comes_out_of_a_zip_by_the_end_of_its_path() {
        let (zip, wad) = a_release();
        assert_eq!(zip_file(&zip, "freedoom1.wad").unwrap(), wad);
        assert_eq!(zip_file(&zip, "COPYING.txt").unwrap(), b"the licence");
        assert!(zip_file(&zip, "doom2.wad").is_err());
        assert!(zip_file(b"not a zip", "freedoom1.wad").is_err());
    }

    #[test]
    fn a_zip_unpacks_whole_as_streamlink_does() {
        let (zip, wad) = a_release();
        let dir = std::env::temp_dir().join(format!("pms-zip-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        zip::ZipArchive::new(std::io::Cursor::new(zip)).and_then(|mut z| z.extract(&dir)).unwrap();
        assert_eq!(std::fs::read(dir.join("freedoom-0.13.0").join("freedoom1.wad")).unwrap(), wad);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_consoles_folders_move_by_renaming_and_nothing_is_overwritten() {
        let root = std::env::temp_dir().join(format!("pms-wads-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (doom, wads, poom) = (root.join("doom"), root.join("wads"), root.join("vidya").join("poom"));
        std::fs::create_dir_all(doom.join(".savegame")).unwrap();
        std::fs::write(doom.join(".savegame").join("doomsav0.dsg"), "my game").unwrap();
        std::fs::create_dir_all(&wads).unwrap();
        std::fs::write(wads.join("freedoom1.wad"), "game").unwrap();
        std::fs::write(wads.join("chosen.txt"), "freedoom1.wad").unwrap();
        // The saves folder moves whole, then the games go inside it.
        move_into(&doom, &poom);
        move_into(&wads, &poom.join("wads"));
        assert_eq!(std::fs::read_to_string(poom.join(".savegame").join("doomsav0.dsg")).unwrap(), "my game");
        assert_eq!(std::fs::read_to_string(poom.join("wads").join("chosen.txt")).unwrap(), "freedoom1.wad");
        assert!(!doom.exists() && !wads.exists(), "the old folders, left empty, are gone");
        // With something already in the new place, only what isn't there moves, and the rest stays where it was.
        std::fs::create_dir_all(&wads).unwrap();
        std::fs::write(wads.join("chosen.txt"), "doom1.wad").unwrap();
        std::fs::write(wads.join("doom1.wad"), "episode").unwrap();
        move_into(&wads, &poom.join("wads"));
        assert_eq!(std::fs::read_to_string(poom.join("wads").join("chosen.txt")).unwrap(), "freedoom1.wad", "not overwritten");
        assert_eq!(std::fs::read_to_string(poom.join("wads").join("doom1.wad")).unwrap(), "episode");
        assert_eq!(std::fs::read_to_string(wads.join("chosen.txt")).unwrap(), "doom1.wad", "left where it was");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn s(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn the_real_thing_goes_before_the_free_one_and_yours_before_both() {
        assert_eq!(pick(&s(&[])), None);
        assert_eq!(pick(&s(&["freedoom1.wad"])).unwrap(), "freedoom1.wad");
        assert_eq!(pick(&s(&["doom1.wad", "freedoom1.wad"])).unwrap(), "doom1.wad");
        assert_eq!(pick(&s(&["DOOM2.WAD", "doom1.wad"])).unwrap(), "DOOM2.WAD");
        assert_eq!(pick(&s(&["doom1.wad", "plutonia.wad"])).unwrap(), "plutonia.wad");
    }

    #[test]
    fn games_have_names() {
        assert_eq!(title("DOOM1.WAD"), "Doom (shareware episode)");
        assert_eq!(title("mine.wad"), "mine.wad");
    }

    #[test]
    fn a_member_of_an_archive_is_found() {
        let member = |name: &str, body: &[u8]| {
            let mut m = format!("{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", 0, 0, 0, 100644, body.len()).into_bytes();
            m.extend_from_slice(body);
            if body.len() % 2 == 1 {
                m.push(b'\n');
            }
            m
        };
        let mut archive = b"!<arch>\n".to_vec();
        archive.extend(member("debian-binary", b"2.0"));
        archive.extend(member("data.tar.xz", b"payload"));
        assert_eq!(ar_member(&archive, "data.tar.xz"), Some(&b"payload"[..]));
        assert_eq!(ar_member(&archive, "nope"), None);
        assert_eq!(ar_member(b"not an archive", "x"), None);
    }

    #[test]
    fn a_file_in_a_tar_is_found() {
        let entry = |name: &str, body: &[u8]| {
            let mut head = vec![0u8; 512];
            head[..name.len()].copy_from_slice(name.as_bytes());
            head[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
            let mut e = head;
            e.extend_from_slice(body);
            e.resize(e.len().div_ceil(512) * 512, 0);
            e
        };
        let mut tar = entry("./usr/share/doc/copyright", b"words");
        tar.extend(entry("./usr/share/games/doom/doom1.wad", b"IWAD..."));
        tar.extend(vec![0; 1024]);
        assert_eq!(tar_file(&tar, "doom1.wad"), Some(&b"IWAD..."[..]));
        assert_eq!(tar_file(&tar, "doom2.wad"), None);
    }

    #[test]
    fn nothing_outside_the_folder_can_be_removed() {
        assert!(remove("..\\pms.toml").is_err());
        assert!(remove("../x.wad").is_err());
        assert!(remove("pms.toml").is_err());
    }

    #[test]
    fn the_licence_is_the_whole_of_it() {
        assert!(SHAREWARE_LICENSE.contains("LIMITED USE SOFTWARE LICENSE AGREEMENT"));
        assert!(SHAREWARE_LICENSE.contains("make"));
        assert!(SHAREWARE_LICENSE.trim_end().ends_with("MATTER.") || SHAREWARE_LICENSE.contains("YOU ACKNOWLEDGE"));
    }
}
