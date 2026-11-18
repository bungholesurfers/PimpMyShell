//! PimpMyShell's sandboxes: Windows AppContainers, one for each kind of helper (`Helper`), made, let into what they need, and
//! read back from a running process to say what it's in.
//!
//! A container is named for its kind and for where PimpMyShell is (`PimpMyShell.<helper>.<8 hex digits>`), so that an
//! installed copy and a portable one never share one, as Chrome, Edge and Firefox do theirs. Its profile is registered with
//! Windows the first time a helper of that kind starts sandboxed, which needs no administrator, and is taken away again when
//! the sandbox is switched off or PimpMyShell's uninstalled (`remove`).
//!
//! What a helper may reach:
//! - **PimpMyShell's program folder**, and Streamlink's, to read and run: through a capability of PimpMyShell's own
//!   (`INSTALL_FILES`), which those folders are given, once, and every helper holds. Checked before each start, so a
//!   portable copy in a folder that can't be given it is refused, and says so, rather than falling over halfway.
//! - **Its own folders** (saves, a shader cache, Streamlink's settings), given to its container alone.
//! - **The network** only with `internetClient`, for the helpers whose work is fetching. A proxy on this PC (127.0.0.1)
//!   can't be reached from any container, whatever it's given.
//! - Its own folder under `%LOCALAPPDATA%\Packages`, where Windows keeps what it writes.
//!
//! Everything else of yours, your files and other programs among them, it can't reach. See pms-spawn, which starts the
//! process inside it.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

/// The kinds of helper that can be sandboxed, each in a container of its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Helper {
    /// The Twatch TV player, pms-twatch.exe.
    Twatch,
    /// Streamlink, in its own Python.
    Streamlink,
    /// The game in the console (POOM).
    Poom,
    /// pms-fetch.exe: Twitch's pictures, and the stream's first playlist.
    Fetch,
}

impl Helper {
    pub const ALL: [Helper; 4] = [Helper::Fetch, Helper::Twatch, Helper::Streamlink, Helper::Poom];

    /// Its name in the settings, and in its container's.
    pub fn key(self) -> &'static str {
        match self {
            Helper::Twatch => "twatch",
            Helper::Streamlink => "streamlink",
            Helper::Poom => "poom",
            Helper::Fetch => "fetch",
        }
    }

    /// What its container says it is, where Windows shows one.
    fn display(self) -> &'static str {
        match self {
            Helper::Twatch => "PimpMyShell: Twatch TV's player",
            Helper::Streamlink => "PimpMyShell: Streamlink",
            Helper::Poom => "PimpMyShell: the game in the PIMPBOX",
            Helper::Fetch => "PimpMyShell: pictures and playlists",
        }
    }

    /// Whether its container is the stricter kind (LPAC), which ALL APPLICATION PACKAGES doesn't count for: most of Windows'
    /// own registry and COM are closed to it, but for what it's given.
    fn lpac(self) -> bool {
        matches!(self, Helper::Poom | Helper::Fetch)
    }

    /// Whether it fetches from the internet itself.
    fn online(self) -> bool {
        matches!(self, Helper::Twatch | Helper::Streamlink | Helper::Fetch)
    }
}

/// The capability every helper holds, which PimpMyShell's program folder and Streamlink's are given to read and run.
pub const INSTALL_FILES: &str = "pimpmyshellInstallFiles";

/// The capabilities a helper can hold, by name, for saying what it's been given. Windows works their SIDs out from these.
const CAPABILITIES: [&str; 6] = ["internetClient", "internetClientServer", "privateNetworkClientServer", "registryRead", "lpacCom", INSTALL_FILES];

/// A short mark of where PimpMyShell is: 8 hex digits of its folder's path, so that each copy has containers of its own.
fn install_mark() -> String {
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
    let hash = dir.to_string_lossy().to_lowercase().bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    format!("{hash:08x}")
}

/// The name of `helper`'s container, for this copy of PimpMyShell.
pub fn name(helper: Helper) -> String {
    format!("PimpMyShell.{}.{}", helper.key(), install_mark())
}

/// A helper's container, ready to start it in: made if it wasn't, PimpMyShell's folder let in, and its capabilities.
/// `folders` are what else it may read (and write, where it's true). `online`: whether it may reach the internet itself
/// (one that goes through the proxy may not: it reaches it through a relay, relay.rs).
pub fn container(helper: Helper, folders: &[(&Path, bool)], online: bool) -> Result<pms_spawn::Container, String> {
    let program = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = program.parent().ok_or("PimpMyShell's folder")?;
    if is_remote(dir) {
        return Err(format!("{} is on a network drive, where a sandbox can't be given what it needs", crate::shown(dir)));
    }
    let install = capability(INSTALL_FILES).ok_or("PimpMyShell's capability")?;
    grant(dir, &install, READ_RUN, false)?;
    let (sid, folder) = profile(&name(helper), helper.display())?;
    for (path, write) in folders {
        grant(path, &sid, if *write { CHANGE } else { READ_RUN }, false)?;
    }
    let mut capabilities = vec![install];
    if online && helper.online() {
        capabilities.extend(capability("internetClient"));
    }
    if helper.lpac() {
        // Without it, an LPAC can't so much as start Winsock or Direct3D: they read the registry as they load.
        capabilities.extend(capability("registryRead"));
    }
    Ok(pms_spawn::Container { sid, capabilities, lpac: helper.lpac(), folder })
}

/// Which helpers the settings last said are sandboxed: for what starts them away from the settings (Streamlink, the player).
static WANTED: std::sync::RwLock<Option<crate::config::Sandboxes>> = std::sync::RwLock::new(None);

/// Takes up what the settings say about sandboxes (net::follow does this).
pub fn follow(config: &crate::config::Config) {
    if let Ok(mut w) = WANTED.write() {
        *w = Some(config.sandbox.clone().unwrap_or_default());
    }
}

/// Whether `helper` is to be sandboxed, as the settings last said.
pub fn wanted(helper: Helper) -> bool {
    WANTED.read().ok().and_then(|w| w.as_ref().map(|s| s.on(helper))).unwrap_or_else(|| crate::config::Sandboxes::default().on(helper))
}

/// What a helper that goes online, and is to be sandboxed, is started in: its container, and the proxy address it's to use in
/// place of the one in the settings, if `through` goes through one. With a proxy, it's given no network of its own: only a
/// relay in its container (relay.rs), which reaches the proxy and nothing else, so it can't go around it. If it can't be
/// sandboxed, why, which is also said for the settings: it isn't started then. The address it's given goes as `session`, if
/// there's one (net::route_as): the relay passes on whatever login the helper gives the proxy.
pub fn for_online(helper: Helper, through: crate::net::Use, session: Option<&crate::net::Session>, folders: &[(&Path, bool)]) -> Result<Option<(pms_spawn::Container, Option<String>)>, String> {
    if !wanted(helper) {
        return Ok(None);
    }
    let made = match crate::net::route_as(through, session) {
        Some(proxy) => container(helper, folders, false).and_then(|c| crate::relay::through(helper, through, &c, &proxy).map(|relayed| (c, Some(relayed)))),
        None => {
            // No proxy now: a relay from when there was one has nothing more to do.
            crate::relay::stop(helper);
            container(helper, folders, true).map(|c| (c, None))
        }
    };
    said(helper, made.as_ref().err().map_or("", String::as_str));
    made.map(Some)
}

/// What went wrong with `helper`'s sandbox, last: for the settings to say. Nothing, once it's started in it.
pub fn said(helper: Helper, problem: &str) {
    let path = pms_tap::data_dir().join("sandbox.txt");
    let mut lines: Vec<String> = std::fs::read_to_string(&path).unwrap_or_default().lines().filter(|l| !l.starts_with(&format!("{}\t", helper.key()))).map(str::to_string).collect();
    if !problem.is_empty() {
        lines.push(format!("{}\t{}", helper.key(), problem.replace(['\n', '\t'], " ")));
    }
    let _ = std::fs::write(&path, lines.iter().map(|l| format!("{l}\n")).collect::<String>());
}

/// What `said` last said about `helper`, if anything.
pub fn trouble(helper: Helper) -> Option<String> {
    let text = std::fs::read_to_string(pms_tap::data_dir().join("sandbox.txt")).ok()?;
    text.lines().find_map(|l| l.strip_prefix(&format!("{}\t", helper.key())).map(str::to_string))
}

/// A folder made readable to every helper, through the capability they hold: Streamlink's, say.
pub fn let_helpers_read(folder: &Path) -> Result<(), String> {
    let install = capability(INSTALL_FILES).ok_or("PimpMyShell's capability")?;
    grant(folder, &install, READ_RUN, false)
}

/// Takes `helper`'s container away, and what it was let into of `folders`: for when its sandbox is switched off, or
/// PimpMyShell is uninstalled. One that was never made is nothing to take away.
pub fn remove(helper: Helper, folders: &[&Path]) -> Result<(), String> {
    let name = name(helper);
    if let Some(sid) = derived(&name) {
        for folder in folders {
            grant(folder, &sid, 0, true)?;
        }
    }
    let hr = unsafe { DeleteAppContainerProfile(wide(&name).as_ptr()) };
    if hr < 0 { Err(format!("{name}: couldn't be taken away ({hr:#x})")) } else { Ok(()) }
}

/// Takes `helper`'s sandbox down: its container, and what it was let into. For when it's switched off.
pub fn take_down(helper: Helper) -> Result<(), String> {
    crate::relay::stop(helper);
    let folders: Vec<PathBuf> = match helper {
        Helper::Twatch | Helper::Fetch => Vec::new(),
        Helper::Streamlink => vec![crate::streamlink::home()],
        Helper::Poom => vec![crate::wads::dir(), crate::wads::saves()],
    };
    remove(helper, &folders.iter().map(PathBuf::as_path).collect::<Vec<_>>())?;
    said(helper, "");
    if helper == Helper::Streamlink {
        let install = capability(INSTALL_FILES).ok_or("PimpMyShell's capability")?;
        grant(&crate::streamlink::dir(), &install, 0, true)?;
    }
    Ok(())
}

/// Every sandbox taken down, and PimpMyShell's folder given back as it was: for `pms sandbox off`, which uninstalling runs.
pub fn take_all_down() -> Result<(), String> {
    let mut failed = Vec::new();
    for helper in Helper::ALL {
        if let Err(e) = take_down(helper) {
            failed.push(e);
        }
    }
    if let Err(e) = forget_program_folder() {
        failed.push(e);
    }
    if failed.is_empty() { Ok(()) } else { Err(failed.join("; ")) }
}

/// Takes PimpMyShell's capability off its program folder: for after every container is gone.
pub fn forget_program_folder() -> Result<(), String> {
    let program = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = program.parent().ok_or("PimpMyShell's folder")?;
    let install = capability(INSTALL_FILES).ok_or("PimpMyShell's capability")?;
    grant(dir, &install, 0, true)
}

/// What a running process is in, as its token says: not what it was meant to be in.
#[derive(Clone, Debug, PartialEq)]
pub enum Sandbox {
    /// Not in a container.
    None,
    /// In one: its name, if it's one of PimpMyShell's (else its SID), whether it's the stricter kind, its SID, and the
    /// capabilities it holds, by name where they're known.
    Container { name: String, lpac: Option<bool>, sid: String, capabilities: Vec<String> },
}

/// What process `pid` is in. None if it can't be asked.
pub fn of_process(pid: u32) -> Option<Sandbox> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut token = null_mut();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token) != 0;
        CloseHandle(process);
        if !opened {
            return None;
        }
        let said = described(token);
        CloseHandle(token);
        said
    }
}

unsafe fn described(token: *mut c_void) -> Option<Sandbox> {
    unsafe {
        let mut contained = 0u32;
        let mut needed = 0u32;
        if GetTokenInformation(token, TOKEN_IS_APP_CONTAINER, (&raw mut contained).cast(), 4, &mut needed) == 0 {
            return None;
        }
        if contained == 0 {
            return Some(Sandbox::None);
        }
        let container = token_info(token, TOKEN_APP_CONTAINER_SID)?;
        let sid_ptr = *container.as_ptr().cast::<*mut c_void>();
        let sid = sid_string(sid_ptr);
        let name = Helper::ALL.iter().map(|h| name(*h)).find(|n| derived(n).is_some_and(|d| sid_string(d.as_ptr().cast_mut().cast()) == sid)).unwrap_or_else(|| sid.clone());
        let mut capabilities = Vec::new();
        if let Some(groups) = token_info(token, TOKEN_CAPABILITIES) {
            let count = *groups.as_ptr().cast::<u32>() as usize;
            let entries = groups.as_ptr().add(1).cast::<SidAndAttributes>();
            let known: Vec<(String, &str)> = CAPABILITIES.iter().filter_map(|n| capability(n).map(|s| (sid_string(s.as_ptr().cast_mut().cast()), *n))).collect();
            for i in 0..count {
                let text = sid_string((*entries.add(i)).sid);
                capabilities.push(known.iter().find(|(s, _)| *s == text).map_or(text, |(_, n)| (*n).to_string()));
            }
        }
        Some(Sandbox::Container { name, lpac: is_lpac(token), sid, capabilities })
    }
}

/// Whether a container's token is the stricter kind (LPAC): Windows marks it with a security attribute, `WIN://NOALLAPPPKG`.
/// (TokenIsLessPrivilegedAppContainer, which ought to say, isn't answered on every Windows.) None if the token won't say.
unsafe fn is_lpac(token: *mut c_void) -> Option<bool> {
    unsafe {
        let info = token_info(token, TOKEN_SECURITY_ATTRIBUTES)?;
        // TOKEN_SECURITY_ATTRIBUTES_INFORMATION: a version, a count, and a pointer to that many TOKEN_SECURITY_ATTRIBUTE_V1,
        // each starting with its name as a UNICODE_STRING: its length in bytes, and then (at 8) where its letters are.
        let base = info.as_ptr().cast::<u8>();
        let count = *base.add(4).cast::<u32>() as usize;
        let attributes = *base.add(8).cast::<*const u8>();
        if attributes.is_null() {
            return Some(false);
        }
        const SIZE: usize = 16 + 2 + 2 + 4 + 4 + 4 + 8;
        Some((0..count).any(|i| {
            let entry = attributes.add(i * SIZE);
            let (bytes, letters) = (*entry.cast::<u16>() as usize, *entry.add(8).cast::<*const u16>());
            !letters.is_null() && String::from_utf16_lossy(std::slice::from_raw_parts(letters, bytes / 2)) == "WIN://NOALLAPPPKG"
        }))
    }
}

/// A token's information of `class`, as Windows lays it out, in memory aligned as its structures want.
unsafe fn token_info(token: *mut c_void, class: u32) -> Option<Vec<usize>> {
    unsafe {
        let mut needed = 0u32;
        GetTokenInformation(token, class, null_mut(), 0, &mut needed);
        if needed == 0 {
            return None;
        }
        let mut buffer = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
        (GetTokenInformation(token, class, buffer.as_mut_ptr().cast(), needed, &mut needed) != 0).then_some(buffer)
    }
}

/// The container called `name`'s profile: made if it isn't registered yet. Its SID, and its folder.
fn profile(name: &str, display: &str) -> Result<(Vec<u8>, PathBuf), String> {
    // One at a time: Windows' making of a profile isn't safe to race. Two threads making the same one (several TwaTVs asking
    // Streamlink at once, the first time after its sandbox was switched on) each find it half-made, and all of them fail:
    // 0x8007000a, its folder not there (0x80070002), its key deleted under it (0x800703fa).
    static MAKING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = MAKING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut sid: *mut c_void = null_mut();
    let hr = unsafe { CreateAppContainerProfile(wide(name).as_ptr(), wide(display).as_ptr(), wide("A sandbox for one of PimpMyShell's helpers. PimpMyShell takes it away when its sandbox is switched off.").as_ptr(), null(), 0, &mut sid) };
    let sid = if hr == ALREADY_EXISTS {
        derived(name).ok_or_else(|| format!("{name}: its SID"))?
    } else if hr < 0 {
        return Err(format!("{name}: couldn't be made ({hr:#x})"));
    } else {
        let bytes = sid_bytes(sid);
        unsafe { FreeSid(sid) };
        bytes
    };
    let mut path: *mut u16 = null_mut();
    let text = wide(&sid_string(sid.as_ptr().cast_mut().cast()));
    let hr = unsafe { GetAppContainerFolderPath(text.as_ptr(), &mut path) };
    if hr < 0 || path.is_null() {
        return Err(format!("{name}: its folder ({hr:#x})"));
    }
    let folder = unsafe { PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(path, (0..).take_while(|i| *path.add(*i) != 0).count()))) };
    unsafe { CoTaskMemFree(path.cast()) };
    Ok((sid, folder))
}

/// The SID Windows works out for the container called `name`, made or not.
fn derived(name: &str) -> Option<Vec<u8>> {
    let mut sid: *mut c_void = null_mut();
    if unsafe { DeriveAppContainerSidFromAppContainerName(wide(name).as_ptr(), &mut sid) } < 0 || sid.is_null() {
        return None;
    }
    let bytes = sid_bytes(sid);
    unsafe { FreeSid(sid) };
    Some(bytes)
}

/// The SID of the capability called `name`: Windows works it out from the name (and knows the old ones, like
/// internetClient, by their numbers).
fn capability(name: &str) -> Option<Vec<u8>> {
    type Derive = unsafe extern "system" fn(*const u16, *mut *mut *mut c_void, *mut u32, *mut *mut *mut c_void, *mut u32) -> i32;
    unsafe {
        let module = LoadLibraryW(wide("kernelbase.dll").as_ptr());
        let found = GetProcAddress(module, c"DeriveCapabilitySidsFromName".as_ptr().cast());
        if found.is_null() {
            return None;
        }
        let derive: Derive = std::mem::transmute(found);
        let (mut groups, mut group_count, mut sids, mut count) = (null_mut(), 0u32, null_mut(), 0u32);
        if derive(wide(name).as_ptr(), &mut groups, &mut group_count, &mut sids, &mut count) == 0 || count == 0 {
            return None;
        }
        let bytes = sid_bytes(*sids);
        for i in 0..group_count as usize {
            LocalFree((*groups.add(i)).cast());
        }
        for i in 0..count as usize {
            LocalFree((*sids.add(i)).cast());
        }
        LocalFree(groups.cast());
        LocalFree(sids.cast());
        Some(bytes)
    }
}

/// A SID's bytes, copied out of memory Windows owns.
fn sid_bytes(sid: *mut c_void) -> Vec<u8> {
    unsafe { std::slice::from_raw_parts(sid.cast::<u8>(), GetLengthSid(sid) as usize).to_vec() }
}

fn sid_string(sid: *mut c_void) -> String {
    let mut text: *mut u16 = null_mut();
    unsafe {
        if ConvertSidToStringSidW(sid, &mut text) == 0 {
            return "?".into();
        }
        let said = String::from_utf16_lossy(std::slice::from_raw_parts(text, (0..).take_while(|i| *text.add(*i) != 0).count()));
        LocalFree(text.cast());
        said
    }
}

/// Read, and run what's there.
const READ_RUN: u32 = 0x0012_00A9;
/// That, and write, make and remove what's in it: what a folder of a helper's own needs.
const CHANGE: u32 = 0x0013_01BF;

/// Gives `sid` `rights` on `path` and everything in it, inheritably: added to what's there, which is otherwise left as it
/// is. Nothing's changed if it has them already. `revoke` takes away whatever it was given instead.
fn grant(path: &Path, sid: &[u8], rights: u32, revoke: bool) -> Result<(), String> {
    if !path.exists() {
        return if revoke { Ok(()) } else { Err(format!("{} isn't there", crate::shown(path))) };
    }
    unsafe {
        let name = wide(&path.to_string_lossy());
        let (mut dacl, mut descriptor) = (null_mut(), null_mut());
        let read = GetNamedSecurityInfoW(name.as_ptr(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), &mut dacl, null_mut(), &mut descriptor);
        if read != 0 {
            return Err(format!("{}: its permissions couldn't be read (error {read})", crate::shown(path)));
        }
        // A volume without permissions (FAT32, exFAT) has nothing to give: everything there is open to everybody already.
        if dacl.is_null() {
            LocalFree(descriptor);
            return Ok(());
        }
        let has = has_ace(dacl, sid, rights);
        if (has && !revoke) || (revoke && !has_ace(dacl, sid, 0)) {
            LocalFree(descriptor);
            return Ok(());
        }
        let access = ExplicitAccess {
            permissions: rights,
            mode: if revoke { REVOKE_ACCESS } else { GRANT_ACCESS },
            inheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
            trustee: Trustee { multiple: null_mut(), operation: 0, form: TRUSTEE_IS_SID, kind: TRUSTEE_IS_UNKNOWN, name: sid.as_ptr().cast_mut().cast() },
        };
        let mut updated = null_mut();
        let made = SetEntriesInAclW(1, &access, dacl, &mut updated);
        let written = if made == 0 { SetNamedSecurityInfoW(name.as_ptr().cast_mut(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, null_mut(), null_mut(), updated, null_mut()) } else { made };
        if !updated.is_null() {
            LocalFree(updated);
        }
        LocalFree(descriptor);
        if written != 0 { Err(format!("{}: its permissions couldn't be changed (error {written})", crate::shown(path))) } else { Ok(()) }
    }
}

/// Whether `dacl` has an allowing entry for `sid` with at least `rights`.
unsafe fn has_ace(dacl: *mut c_void, sid: &[u8], rights: u32) -> bool {
    unsafe {
        let count = *dacl.cast::<u8>().add(4).cast::<u16>();
        for i in 0..u32::from(count) {
            let mut ace: *mut c_void = null_mut();
            if GetAce(dacl, i, &mut ace) == 0 {
                continue;
            }
            // ACCESS_ALLOWED_ACE: its header (type, flags, size), its mask, and then the SID.
            let kind = *ace.cast::<u8>();
            if kind != 0 {
                continue;
            }
            let mask = *ace.cast::<u8>().add(4).cast::<u32>();
            let ace_sid = ace.cast::<u8>().add(8);
            let len = GetLengthSid(ace_sid.cast()) as usize;
            if std::slice::from_raw_parts(ace_sid, len) == sid && mask & rights == rights {
                return true;
            }
        }
        false
    }
}

/// Whether `path` is on a network drive, where a sandbox can't be given anything.
fn is_remote(path: &Path) -> bool {
    let text = path.to_string_lossy();
    if text.starts_with(r"\\") && !text.starts_with(r"\\?\") {
        return true;
    }
    let root: String = text.trim_start_matches(r"\\?\").chars().take(3).collect();
    unsafe { GetDriveTypeW(wide(&root).as_ptr()) == DRIVE_REMOTE }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

const ALREADY_EXISTS: i32 = 0x8007_00B7_u32 as i32;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const TOKEN_QUERY: u32 = 8;
const TOKEN_IS_APP_CONTAINER: u32 = 29;
const TOKEN_CAPABILITIES: u32 = 30;
const TOKEN_APP_CONTAINER_SID: u32 = 31;
const TOKEN_SECURITY_ATTRIBUTES: u32 = 39;
const SE_FILE_OBJECT: u32 = 1;
const DACL_SECURITY_INFORMATION: u32 = 4;
const GRANT_ACCESS: u32 = 1;
const REVOKE_ACCESS: u32 = 4;
const SUB_CONTAINERS_AND_OBJECTS_INHERIT: u32 = 3;
const TRUSTEE_IS_SID: u32 = 0;
const TRUSTEE_IS_UNKNOWN: u32 = 0;
const DRIVE_REMOTE: u32 = 4;

#[repr(C)]
struct SidAndAttributes {
    sid: *mut c_void,
    attributes: u32,
}

#[repr(C)]
struct Trustee {
    multiple: *mut c_void,
    operation: u32,
    form: u32,
    kind: u32,
    name: *mut c_void,
}

#[repr(C)]
struct ExplicitAccess {
    permissions: u32,
    mode: u32,
    inheritance: u32,
    trustee: Trustee,
}

#[link(name = "userenv")]
unsafe extern "system" {
    fn CreateAppContainerProfile(name: *const u16, display: *const u16, description: *const u16, capabilities: *const SidAndAttributes, count: u32, sid: *mut *mut c_void) -> i32;
    fn DeleteAppContainerProfile(name: *const u16) -> i32;
    fn DeriveAppContainerSidFromAppContainerName(name: *const u16, sid: *mut *mut c_void) -> i32;
    fn GetAppContainerFolderPath(sid: *const u16, path: *mut *mut u16) -> i32;
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn FreeSid(sid: *mut c_void) -> *mut c_void;
    fn GetLengthSid(sid: *mut c_void) -> u32;
    fn ConvertSidToStringSidW(sid: *mut c_void, text: *mut *mut u16) -> i32;
    fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
    fn GetTokenInformation(token: *mut c_void, class: u32, info: *mut c_void, len: u32, needed: *mut u32) -> i32;
    fn GetNamedSecurityInfoW(name: *const u16, kind: u32, what: u32, owner: *mut *mut c_void, group: *mut *mut c_void, dacl: *mut *mut c_void, sacl: *mut *mut c_void, descriptor: *mut *mut c_void) -> u32;
    fn SetNamedSecurityInfoW(name: *mut u16, kind: u32, what: u32, owner: *mut c_void, group: *mut c_void, dacl: *mut c_void, sacl: *mut c_void) -> u32;
    fn SetEntriesInAclW(count: u32, entries: *const ExplicitAccess, old: *mut c_void, new: *mut *mut c_void) -> u32;
    fn GetAce(acl: *mut c_void, index: u32, ace: *mut *mut c_void) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetDriveTypeW(root: *const u16) -> u32;
}

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoTaskMemFree(memory: *mut c_void);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_this_copys_own_and_short_enough() {
        for helper in Helper::ALL {
            let n = name(helper);
            assert!(n.starts_with("PimpMyShell.") && n.len() <= 50 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '.'), "{n}");
        }
        assert_ne!(name(Helper::Twatch), name(Helper::Poom));
    }

    #[test]
    fn capabilities_and_containers_have_sids() {
        let install = capability(INSTALL_FILES).expect("ours");
        assert_eq!(sid_string(install.as_ptr().cast_mut().cast()).split('-').take(4).collect::<Vec<_>>(), ["S", "1", "15", "3"], "a capability");
        assert_eq!(sid_string(capability("internetClient").unwrap().as_ptr().cast_mut().cast()), "S-1-15-3-1");
        let derived = derived("PimpMyShell.test.name").expect("derived");
        assert!(sid_string(derived.as_ptr().cast_mut().cast()).starts_with("S-1-15-2-"));
    }

    #[test]
    fn every_running_helper_can_be_asked_what_its_in() {
        // Whatever's running: a helper in a container, if there's one, is read without falling over.
        for p in crate::procs::ours() {
            let _ = of_process(p.pid);
        }
    }

    /// Several TwaTVs asking Streamlink at once each want its container: all of them get it, made once, and none finds it
    /// half-made (as six did on 2026-11-18, with four different errors between them).
    #[test]
    fn a_container_wanted_from_many_threads_at_once_is_made_once_for_all_of_them() {
        let name = format!("PimpMyShell.testrace.{}", std::process::id());
        let gate = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (name, gate) = (name.clone(), gate.clone());
                std::thread::spawn(move || {
                    gate.wait();
                    profile(&name, "PimpMyShell: a test")
                })
            })
            .collect();
        let got: Vec<Result<(Vec<u8>, PathBuf), String>> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        let _ = unsafe { DeleteAppContainerProfile(wide(&name).as_ptr()) };
        let failed: Vec<&String> = got.iter().filter_map(|g| g.as_ref().err()).collect();
        assert!(failed.is_empty(), "{failed:?}");
        assert!(got.windows(2).all(|w| w[0] == w[1]), "the same container for all");
    }

    #[test]
    fn this_process_is_in_no_container() {
        assert_eq!(of_process(std::process::id()), Some(Sandbox::None));
    }
}
