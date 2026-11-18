//! How PimpMyShell starts its helpers (Streamlink, the Twatch player, the console, the game in it), so that each can do
//! as little as its work needs:
//!
//! - **A job of its own**, closed with the parent: when the daemon goes, however it goes, its helpers go with it. The job
//!   can also cap how many processes the helper may have (itself only, for most), and keep it from the clipboard, global
//!   atoms, the desktop and the display settings.
//! - **Only the handles it's given.** Its pipes, and whatever's named in `inherit` (shared memory, say), and nothing else
//!   the parent happens to have open: `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`.
//! - **No console**: `DETACHED_PROCESS`. Its standard handles are pipes, or the null device.
//! - **An environment without proxies or PimpMyShell's own variables**, but for the ones it's given: a helper is told what
//!   it needs, and Windows' or a shell's proxy settings never decide where its traffic goes.
//! - **Mitigations** chosen per helper: no injected AppInit DLLs or global hooks (extension points), strict handle checks,
//!   the heap ending the process when it's corrupted, no DLLs from network shares, and, for one that draws nothing, no
//!   win32k at all.
//! - **No processes of its own**, for one that has no need of any (the child process policy).
//!
//! - **An AppContainer**, for one that's sandboxed (`container`): Windows' own sandbox, which keeps it from your files, other
//!   programs and the network, but for what it's been let have. Making the container, and letting it into folders, is the
//!   daemon's (pms::sandbox): this only starts a process in one.
//! - **Efficiency mode**, for one whose work is never in a hurry (`power`): Windows' EcoQoS, the slowest clock speed that will
//!   do, and on a processor that has them, the efficient cores. Not inherited: a helper that starts another says so for it.
//!
//! Each is something Windows enforces, set as the process is made, before it runs any of its own code.

use std::ffi::{OsStr, OsString, c_void};
use std::fs::File;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

type Handle = *mut c_void;

/// Mitigations, as `PROCESS_CREATION_MITIGATION_POLICY_*_ALWAYS_ON` has them.
pub mod mitigate {
    /// A corrupted heap ends the process, rather than letting it carry on.
    pub const HEAP_TERMINATE: u64 = 1 << 12;
    /// Using a handle that isn't one ends the process.
    pub const STRICT_HANDLES: u64 = 1 << 24;
    /// No win32k system calls at all: no windows, no GDI, no Direct3D. Only for a helper that draws nothing.
    pub const NO_WIN32K: u64 = 1 << 28;
    /// No AppInit DLLs, Winsock layered providers, global window hooks or old-style IMEs loaded into it.
    pub const EXTENSION_POINTS_OFF: u64 = 1 << 32;
    /// No DLLs from network shares.
    pub const NO_REMOTE_IMAGES: u64 = 1 << 52;
    /// What costs nothing to a program of ours.
    pub const USUAL: u64 = HEAP_TERMINATE | STRICT_HANDLES | EXTENSION_POINTS_OFF | NO_REMOTE_IMAGES;
    /// What costs nothing to somebody else's program, whose handle use isn't ours to vouch for.
    pub const THEIRS: u64 = HEAP_TERMINATE | EXTENSION_POINTS_OFF | NO_REMOTE_IMAGES;
}

/// Efficiency mode, as `PROCESS_POWER_THROTTLING_*` has it: what `power` takes, any of these together.
pub mod power {
    /// Windows decides, as it does for any program: by whether its window can be seen, whether it's playing sound, the battery.
    pub const WINDOWS: u32 = 0;
    /// EcoQoS: the slowest clock speed that will do, and the efficient cores where there are some. Windows 11 does this on mains
    /// power too; Windows 10, only on battery.
    pub const EFFICIENT: u32 = 0x1;
    /// The finer timer it may ask for (timeBeginPeriod) is ignored, so Windows doesn't wake the processor more often for it.
    /// Rust's own sleeps don't ask: they use a high-resolution timer of their own, which this leaves alone.
    pub const COARSE_TIMERS: u32 = 0x4;
}

/// Puts a process in efficiency mode (`power`, from `power`), or leaves it to Windows (`power::WINDOWS`): with a handle that
/// may set its information, the limited kind being enough. Whether Windows took it (before Windows 10's 1709, it can't).
fn set_power(process: Handle, power: u32) -> bool {
    // Each call says all of it: what's left out of the mask is Windows' to decide again.
    let state = PowerThrottling { version: 1, control: power, state: power };
    unsafe { SetProcessInformation(process, PROCESS_POWER_THROTTLING, (&raw const state).cast(), size_of::<PowerThrottling>() as u32) != 0 }
}

/// What a process has asked for (or been put in): its `power` flags, or None if it can't be asked (a handle that may query
/// its information, the limited kind being enough; before Windows 11's 22H2, nobody can be). What it asked for, not what
/// Windows does with it, which nothing says.
fn power_of(process: Handle) -> Option<u32> {
    let mut state = PowerThrottling { version: 1, control: 0, state: 0 };
    let asked = unsafe { GetProcessInformation(process, PROCESS_POWER_THROTTLING, (&raw mut state).cast(), size_of::<PowerThrottling>() as u32) };
    (asked != 0).then_some(state.state & state.control)
}

/// This process in efficiency mode, or left to Windows: whether Windows took it.
pub fn set_own_power(power: u32) -> bool {
    set_power(unsafe { GetCurrentProcess() }, power)
}

/// What the process `pid` has asked for, as `power` flags: None where it can't be told.
pub fn power_of_process(pid: u32) -> Option<u32> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let process = Owned(process);
        power_of(process.0)
    }
}

/// What one of its standard handles is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stdio {
    /// The null device: nothing comes in, and what goes out goes nowhere.
    Null,
    /// A pipe, whose other end the parent gets as a `File`.
    Piped,
}

/// A helper, before it's started.
pub struct Command {
    program: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    cwd: Option<PathBuf>,
    stdio: [Stdio; 3],
    inherit: Vec<RawHandle>,
    processes: u32,
    no_children: bool,
    mitigations: u64,
    ui_limits: bool,
    with_parent: bool,
    container: Option<Container>,
    power: u32,
}

/// An AppContainer to start a helper in: the container's SID, the capabilities it's given (each a SID too), whether it's the
/// stricter kind (LPAC, which ALL APPLICATION PACKAGES doesn't count for), and its own folder, where Windows keeps what it
/// writes (Windows points its TEMP and LOCALAPPDATA there by itself). SIDs are as Windows lays them out in memory.
#[derive(Clone, Debug)]
pub struct Container {
    pub sid: Vec<u8>,
    pub capabilities: Vec<Vec<u8>>,
    pub lpac: bool,
    pub folder: PathBuf,
}

/// Variables a helper doesn't get from its parent, whatever they hold: the proxy is PimpMyShell's to give.
const DROPPED: [&str; 8] = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY", "FTP_PROXY", "SOCKS_PROXY", "REQUESTS_CA_BUNDLE", "SSL_CERT_FILE"];

impl Command {
    pub fn new(program: impl AsRef<Path>) -> Command {
        Command {
            program: program.as_ref().to_path_buf(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
            stdio: [Stdio::Null; 3],
            inherit: Vec::new(),
            processes: 1,
            no_children: true,
            mitigations: mitigate::USUAL,
            ui_limits: true,
            with_parent: true,
            container: None,
            power: power::WINDOWS,
        }
    }

    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Command {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    pub fn args<I: IntoIterator<Item = S>, S: AsRef<OsStr>>(mut self, args: I) -> Command {
        self.args.extend(args.into_iter().map(|a| a.as_ref().to_os_string()));
        self
    }

    /// A variable it's given, over what it would have had.
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Command {
        self.env.push((key.as_ref().to_os_string(), value.as_ref().to_os_string()));
        self
    }

    pub fn current_dir(mut self, dir: impl AsRef<Path>) -> Command {
        self.cwd = Some(dir.as_ref().to_path_buf());
        self
    }

    pub fn stdin(mut self, how: Stdio) -> Command {
        self.stdio[0] = how;
        self
    }

    pub fn stdout(mut self, how: Stdio) -> Command {
        self.stdio[1] = how;
        self
    }

    pub fn stderr(mut self, how: Stdio) -> Command {
        self.stdio[2] = how;
        self
    }

    /// A handle it inherits. It has it by `shared.number()`: tell it that, on its command line, say.
    pub fn inherit(mut self, shared: &Shared) -> Command {
        self.inherit.push(shared.0.0);
        self
    }

    /// How many processes its job may hold, itself included (1, unless said). 0 for no limit.
    pub fn processes(mut self, most: u32) -> Command {
        self.processes = most;
        self
    }

    /// Whether it may start programs of its own at all (no, unless said).
    pub fn children(mut self, allowed: bool) -> Command {
        self.no_children = !allowed;
        self
    }

    /// Its mitigations, from `mitigate` (`USUAL`, unless said).
    pub fn mitigations(mut self, flags: u64) -> Command {
        self.mitigations = flags;
        self
    }

    /// Whether its job keeps it from the clipboard, global atoms, the desktop, the display settings and the system's
    /// parameters (yes, unless said).
    pub fn ui_limits(mut self, on: bool) -> Command {
        self.ui_limits = on;
        self
    }

    /// Whether it ends when the parent lets go of it, or the parent ends (yes, unless said). One that doesn't still can't
    /// leave its job, and ends when it ends of itself: for a build that has to end as a program does to write what it
    /// recorded (PGO training).
    pub fn ends_with_parent(mut self, yes: bool) -> Command {
        self.with_parent = yes;
        self
    }

    /// Starts it in this AppContainer.
    pub fn container(mut self, container: Container) -> Command {
        self.container = Some(container);
        self
    }

    /// Starts it in efficiency mode (`power`, from `power`), or left to Windows (`power::WINDOWS`, unless said). If Windows
    /// won't have it, it's started anyway: it's only what it costs.
    pub fn power(mut self, power: u32) -> Command {
        self.power = power;
        self
    }

    /// The command line as Windows wants it: each argument quoted where it has to be, the way the C runtime reads them back.
    fn command_line(&self) -> Vec<u16> {
        let mut line = String::new();
        for (i, arg) in std::iter::once(self.program.as_os_str()).chain(self.args.iter().map(OsString::as_os_str)).enumerate() {
            if i > 0 {
                line.push(' ');
            }
            quote(&arg.to_string_lossy(), &mut line);
        }
        wide(&line)
    }

    /// The environment block: the parent's, without proxies or PimpMyShell's own variables, with what it's given, sorted.
    fn environment(&self) -> Vec<u16> {
        let dropped = |key: &str| DROPPED.iter().any(|d| d.eq_ignore_ascii_case(key)) || key.to_ascii_uppercase().starts_with("PMS_");
        let mut vars: Vec<(String, String)> = std::env::vars_os().map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned())).filter(|(k, _)| !dropped(k)).collect();
        // In a container, Windows points TEMP, TMP and LOCALAPPDATA at the container's own folder itself, working it out from
        // the LOCALAPPDATA it's given: so that's left as it is.
        for (key, value) in &self.env {
            let key = key.to_string_lossy().into_owned();
            vars.retain(|(k, _)| !k.eq_ignore_ascii_case(&key));
            vars.push((key, value.to_string_lossy().into_owned()));
        }
        vars.sort_by_key(|(k, _)| k.to_ascii_uppercase());
        let mut block: Vec<u16> = Vec::new();
        for (key, value) in vars {
            block.extend(format!("{key}={value}").encode_utf16());
            block.push(0);
        }
        block.push(0);
        block
    }

    /// Starts it.
    pub fn spawn(self) -> Result<Child, String> {
        let name = self.program.file_name().map_or_else(|| self.program.display().to_string(), |n| n.to_string_lossy().into_owned());
        let fail = |what: &str| format!("{name}: {what}: {}", std::io::Error::last_os_error());
        unsafe {
            let job = make_job(self.processes, self.ui_limits, self.with_parent).ok_or_else(|| fail("its job"))?;
            let job = Owned(job);
            // Its ends of the pipes, and copies of what it inherits, made inheritable for this alone: the handle list then
            // gives it these and nothing else.
            let mut theirs: Vec<Owned> = Vec::new();
            let mut ours: [Option<File>; 3] = [None, None, None];
            for (at, how) in self.stdio.iter().enumerate() {
                let handle = match how {
                    Stdio::Null => null_device().ok_or_else(|| fail("the null device"))?,
                    Stdio::Piped => {
                        let (mut read, mut write) = (null_mut(), null_mut());
                        if CreatePipe(&mut read, &mut write, null(), 0) == 0 {
                            return Err(fail("a pipe"));
                        }
                        let (mine, child) = if at == 0 { (write, read) } else { (read, write) };
                        ours[at] = Some(File::from_raw_handle(mine));
                        let shared = inheritable(child);
                        CloseHandle(child);
                        shared.ok_or_else(|| fail("a pipe"))?
                    }
                };
                theirs.push(Owned(handle));
            }
            let list: Vec<Handle> = theirs.iter().map(|h| h.0).chain(self.inherit.iter().copied()).collect();
            let job_list = [job.0];
            let mitigations: [u64; 2] = [self.mitigations, 0];
            let no_children: u32 = 1;
            let lpac: u32 = 1;
            // The container and its capabilities, as Windows takes them: pointers into the SIDs held above, which outlive this.
            let mut granted: Vec<SidAndAttributes> = self.container.iter().flat_map(|c| c.capabilities.iter()).map(|sid| SidAndAttributes { sid: sid.as_ptr().cast_mut().cast(), attributes: SE_GROUP_ENABLED }).collect();
            let security = self.container.as_ref().map(|c| SecurityCapabilities {
                container: c.sid.as_ptr().cast_mut().cast(),
                capabilities: if granted.is_empty() { null_mut() } else { granted.as_mut_ptr() },
                count: granted.len() as u32,
                reserved: 0,
            });
            let is_lpac = self.container.as_ref().is_some_and(|c| c.lpac);
            let count = 2 + u32::from(self.mitigations != 0) + u32::from(self.no_children) + u32::from(security.is_some()) + u32::from(is_lpac);
            let mut attributes = Attributes::new(count).ok_or_else(|| fail("its attributes"))?;
            if let Some(security) = &security {
                attributes.set(SECURITY_CAPABILITIES, (security as *const SecurityCapabilities).cast(), size_of::<SecurityCapabilities>()).ok_or_else(|| fail("its container"))?;
            }
            if is_lpac {
                attributes.set(ALL_APPLICATION_PACKAGES_POLICY, (&raw const lpac).cast(), 4).ok_or_else(|| fail("its container's strictness"))?;
            }
            attributes.set(HANDLE_LIST, list.as_ptr().cast(), size_of_val(list.as_slice())).ok_or_else(|| fail("the handles it inherits"))?;
            attributes.set(JOB_LIST, job_list.as_ptr().cast(), size_of_val(&job_list)).ok_or_else(|| fail("its job"))?;
            if self.mitigations != 0 {
                attributes.set(MITIGATION_POLICY, mitigations.as_ptr().cast(), size_of_val(&mitigations)).ok_or_else(|| fail("its mitigations"))?;
            }
            if self.no_children {
                attributes.set(CHILD_PROCESS_POLICY, (&raw const no_children).cast(), 4).ok_or_else(|| fail("its child process policy"))?;
            }
            let mut info: StartupInfoExW = std::mem::zeroed();
            info.startup.cb = size_of::<StartupInfoExW>() as u32;
            // No busy pointer for it: Windows would keep one going for seconds after a program starts, until it asked for its first
            // message, and a helper isn't something you started, nor (without win32k) able to ask.
            info.startup.flags = STARTF_USESTDHANDLES | STARTF_FORCEOFFFEEDBACK;
            (info.startup.stdin, info.startup.stdout, info.startup.stderr) = (list[0], list[1], list[2]);
            info.attributes = attributes.list();
            let mut line = self.command_line();
            let mut environment = self.environment();
            let program = wide(&self.program.to_string_lossy());
            let cwd = self.cwd.as_ref().map(|d| wide(&d.to_string_lossy()));
            let mut made: ProcessInformation = std::mem::zeroed();
            // Efficiency mode isn't one of the attributes a process can be made with: it's made suspended, put in it, and only
            // then let go, so it's in it from its first instruction.
            let suspended = if self.power != power::WINDOWS { CREATE_SUSPENDED } else { 0 };
            let flags = DETACHED_PROCESS | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT | suspended;
            // No box on your desktop from it, ever: one that can't start (a DLL that won't load, or won't set itself up, as
            // user32 won't without win32k), or that falls over, just ends, with why as its exit code, for whoever started it to
            // say. A helper takes the error mode this process has as it's made: so it's this, for that moment.
            let before = GetErrorMode();
            SetErrorMode(before | SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);
            let started = CreateProcessW(program.as_ptr(), line.as_mut_ptr(), null(), null(), 1, flags, environment.as_mut_ptr().cast(), cwd.as_ref().map_or(null(), |d| d.as_ptr()), &info.startup, &mut made);
            SetErrorMode(before);
            if started == 0 {
                return Err(fail("it didn't start"));
            }
            if suspended != 0 {
                set_power(made.process, self.power);
                if ResumeThread(made.thread) == u32::MAX {
                    let why = fail("it couldn't be let go");
                    TerminateJobObject(job.0, 1);
                    CloseHandle(made.thread);
                    CloseHandle(made.process);
                    return Err(why);
                }
            }
            CloseHandle(made.thread);
            let [stdin, stdout, stderr] = ours;
            Ok(Child { process: Owned(made.process), job, pid: made.pid, stdin, stdout, stderr })
        }
    }
}

/// A handle made ready to hand to a helper: a copy of it that can be inherited, which the helper gets by the same number
/// (`number`), closed here when this is dropped. Make it just before starting the helper, and drop it just after: a program
/// started in between by other means than this, that inherits everything it can, would get it too.
pub struct Shared(Owned);

impl Shared {
    /// `handle` is its number, as this process has it.
    pub fn new(handle: usize) -> Result<Shared, String> {
        unsafe { inheritable(handle as Handle) }.map(|h| Shared(Owned(h))).ok_or_else(|| format!("a handle to hand over: {}", std::io::Error::last_os_error()))
    }

    /// The number the helper has it by.
    pub fn number(&self) -> usize {
        self.0.0 as usize
    }
}

/// A helper that's been started. Dropped, it's ended, with everything in its job (unless it was started not to be).
pub struct Child {
    process: Owned,
    job: Owned,
    pid: u32,
    pub stdin: Option<File>,
    pub stdout: Option<File>,
    pub stderr: Option<File>,
}

impl Child {
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// Its process, for asking about (how much memory, its token): valid for as long as this is.
    pub fn handle(&self) -> RawHandle {
        self.process.0
    }

    /// Puts it in efficiency mode (`power`, from `power`), or leaves it to Windows, from now on: whether Windows took it.
    pub fn set_power(&self, power: u32) -> bool {
        set_power(self.process.0, power)
    }

    /// What it's asked for, as `power` flags: None where that can't be told (before Windows 11's 22H2).
    pub fn power(&self) -> Option<u32> {
        power_of(self.process.0)
    }

    /// Ends it, and anything it started.
    pub fn kill(&mut self) {
        unsafe { TerminateJobObject(self.job.0, 1) };
    }

    /// Its exit code, once it's ended; None while it runs.
    pub fn try_wait(&mut self) -> Option<u32> {
        let mut code = 0u32;
        unsafe { (WaitForSingleObject(self.process.0, 0) == 0 && GetExitCodeProcess(self.process.0, &mut code) != 0).then_some(code) }
    }

    /// Waits for it to end, and says how.
    pub fn wait(&mut self) -> u32 {
        unsafe { WaitForSingleObject(self.process.0, u32::MAX) };
        self.try_wait().unwrap_or(1)
    }
}

/// A job for one helper: ended with the last handle to it (the parent's), unless `with_parent` says otherwise, and with no
/// way out for what's in it.
unsafe fn make_job(processes: u32, ui_limits: bool, with_parent: bool) -> Option<Handle> {
    unsafe {
        let job = CreateJobObjectW(null(), null());
        if job.is_null() {
            return None;
        }
        let mut limits: ExtendedLimits = std::mem::zeroed();
        limits.basic.flags = JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
        if with_parent {
            limits.basic.flags |= JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        }
        if processes > 0 {
            limits.basic.flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            limits.basic.active_processes = processes;
        }
        let ok = SetInformationJobObject(job, JOB_EXTENDED_LIMITS, (&raw const limits).cast(), size_of::<ExtendedLimits>() as u32) != 0;
        let ui: u32 = JOB_OBJECT_UILIMIT_DESKTOP | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS | JOB_OBJECT_UILIMIT_EXITWINDOWS | JOB_OBJECT_UILIMIT_GLOBALATOMS | JOB_OBJECT_UILIMIT_READCLIPBOARD | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS | JOB_OBJECT_UILIMIT_WRITECLIPBOARD;
        let ok = ok && (!ui_limits || SetInformationJobObject(job, JOB_UI_LIMITS, (&raw const ui).cast(), 4) != 0);
        if !ok {
            CloseHandle(job);
            return None;
        }
        Some(job)
    }
}

/// An inheritable copy of `handle`, for one child.
unsafe fn inheritable(handle: Handle) -> Option<Handle> {
    unsafe {
        let me = GetCurrentProcess();
        let mut copy = null_mut();
        (DuplicateHandle(me, handle, me, &mut copy, 0, 1, DUPLICATE_SAME_ACCESS) != 0).then_some(copy)
    }
}

/// The null device, inheritable, for reading and writing.
unsafe fn null_device() -> Option<Handle> {
    let sa = SecurityAttributes { length: size_of::<SecurityAttributes>() as u32, descriptor: null_mut(), inherit: 1 };
    let handle = unsafe { CreateFileW(wide("NUL").as_ptr(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, &sa, OPEN_EXISTING, 0, null_mut()) };
    (handle as isize != -1).then_some(handle)
}

/// An argument as the C runtime would read it back: in quotes if it has a space, a tab or a quote in it, or nothing; a
/// quote inside escaped, and the backslashes before a quote, or before the closing one, doubled.
fn quote(arg: &str, line: &mut String) {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"', '\n']) {
        line.push_str(arg);
        return;
    }
    line.push('"');
    let mut slashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                line.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                slashes = 0;
            }
            _ => {
                line.extend(std::iter::repeat_n('\\', slashes));
                slashes = 0;
            }
        }
        if c != '\\' {
            line.push(c);
        }
    }
    line.extend(std::iter::repeat_n('\\', slashes * 2));
    line.push('"');
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// A handle closed when it's dropped.
struct Owned(Handle);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

// Handles are for any thread.
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}

/// A process's attribute list, freed when it's dropped. What's set in it must outlive it.
struct Attributes(Vec<u8>);

impl Attributes {
    fn new(count: u32) -> Option<Attributes> {
        let mut size = 0usize;
        unsafe { InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size) };
        let mut list = Attributes(vec![0u8; size]);
        (unsafe { InitializeProcThreadAttributeList(list.list(), count, 0, &mut size) } != 0).then_some(list)
    }

    fn list(&mut self) -> *mut c_void {
        self.0.as_mut_ptr().cast()
    }

    unsafe fn set(&mut self, attribute: usize, value: *const c_void, size: usize) -> Option<()> {
        (unsafe { UpdateProcThreadAttribute(self.list(), 0, attribute, value, size, null_mut(), null_mut()) } != 0).then_some(())
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.list()) };
    }
}

const HANDLE_LIST: usize = 0x0002_0002;
const SECURITY_CAPABILITIES: usize = 0x0002_0009;
const ALL_APPLICATION_PACKAGES_POLICY: usize = 0x0002_000F;
const SE_GROUP_ENABLED: u32 = 4;
const MITIGATION_POLICY: usize = 0x0002_0007;
const JOB_LIST: usize = 0x0002_000D;
const CHILD_PROCESS_POLICY: usize = 0x0002_000E;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const CREATE_SUSPENDED: u32 = 0x0000_0004;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
/// ProcessPowerThrottling, of PROCESS_INFORMATION_CLASS.
const PROCESS_POWER_THROTTLING: u32 = 4;
const STARTF_USESTDHANDLES: u32 = 0x100;
const STARTF_FORCEOFFFEEDBACK: u32 = 0x80;
const SEM_FAILCRITICALERRORS: u32 = 0x0001;
const SEM_NOGPFAULTERRORBOX: u32 = 0x0002;
const SEM_NOOPENFILEERRORBOX: u32 = 0x8000;
const DUPLICATE_SAME_ACCESS: u32 = 2;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const JOB_EXTENDED_LIMITS: u32 = 9;
const JOB_UI_LIMITS: u32 = 4;
const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x0008;
const JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION: u32 = 0x0400;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
const JOB_OBJECT_UILIMIT_READCLIPBOARD: u32 = 0x0002;
const JOB_OBJECT_UILIMIT_WRITECLIPBOARD: u32 = 0x0004;
const JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS: u32 = 0x0008;
const JOB_OBJECT_UILIMIT_DISPLAYSETTINGS: u32 = 0x0010;
const JOB_OBJECT_UILIMIT_GLOBALATOMS: u32 = 0x0020;
const JOB_OBJECT_UILIMIT_DESKTOP: u32 = 0x0040;
const JOB_OBJECT_UILIMIT_EXITWINDOWS: u32 = 0x0080;

#[repr(C)]
struct SidAndAttributes {
    sid: *mut c_void,
    attributes: u32,
}

#[repr(C)]
struct SecurityCapabilities {
    container: *mut c_void,
    capabilities: *mut SidAndAttributes,
    count: u32,
    reserved: u32,
}

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit: i32,
}

#[repr(C)]
struct StartupInfoW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_chars: u32,
    y_chars: u32,
    fill: u32,
    flags: u32,
    show: u16,
    cb_reserved2: u16,
    reserved2: *mut u8,
    stdin: Handle,
    stdout: Handle,
    stderr: Handle,
}

#[repr(C)]
struct StartupInfoExW {
    startup: StartupInfoW,
    attributes: *mut c_void,
}

#[repr(C)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    pid: u32,
    tid: u32,
}

/// PROCESS_POWER_THROTTLING_STATE.
#[repr(C)]
struct PowerThrottling {
    version: u32,
    control: u32,
    state: u32,
}

#[repr(C)]
struct BasicLimits {
    per_process_time: i64,
    per_job_time: i64,
    flags: u32,
    min_working_set: usize,
    max_working_set: usize,
    active_processes: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
struct IoCounters {
    counts: [u64; 6],
}

#[repr(C)]
struct ExtendedLimits {
    basic: BasicLimits,
    io: IoCounters,
    process_memory: usize,
    job_memory: usize,
    peak_process_memory: usize,
    peak_job_memory: usize,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn InitializeProcThreadAttributeList(list: *mut c_void, count: u32, flags: u32, size: *mut usize) -> i32;
    fn UpdateProcThreadAttribute(list: *mut c_void, flags: u32, attribute: usize, value: *const c_void, size: usize, previous: *mut c_void, returned: *mut usize) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: *const c_void, ta: *const c_void, inherit: i32, flags: u32, env: *const c_void, dir: *const u16, si: *const StartupInfoW, pi: *mut ProcessInformation) -> i32;
    fn CreatePipe(read: *mut Handle, write: *mut Handle, sa: *const SecurityAttributes, size: u32) -> i32;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const SecurityAttributes, disposition: u32, flags: u32, template: Handle) -> Handle;
    fn DuplicateHandle(from: Handle, handle: Handle, to: Handle, copy: *mut Handle, access: u32, inherit: i32, options: u32) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn CreateJobObjectW(sa: *const c_void, name: *const u16) -> Handle;
    fn SetInformationJobObject(job: Handle, class: u32, info: *const c_void, len: u32) -> i32;
    fn TerminateJobObject(job: Handle, code: u32) -> i32;
    fn WaitForSingleObject(handle: Handle, ms: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetErrorMode() -> u32;
    fn SetErrorMode(mode: u32) -> u32;
    fn ResumeThread(thread: Handle) -> u32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn SetProcessInformation(process: Handle, class: u32, info: *const c_void, size: u32) -> i32;
    fn GetProcessInformation(process: Handle, class: u32, info: *mut c_void, size: u32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn arguments_come_back_as_they_went() {
        let line = |args: &[&str]| {
            let mut out = String::new();
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                quote(a, &mut out);
            }
            out
        };
        assert_eq!(line(&["plain", "two words", ""]), r#"plain "two words" """#);
        assert_eq!(line(&[r#"say "hi""#]), r#""say \"hi\"""#);
        assert_eq!(line(&[r"C:\a dir\"]), r#""C:\a dir\\""#);
        assert_eq!(line(&[r"C:\plain\path"]), r"C:\plain\path");
    }

    /// cmd.exe, started as a helper: what it echoes comes back up its pipe, and it can't start a program of its own.
    #[test]
    fn a_helper_says_what_it_says_and_can_start_nothing() {
        let cmd = PathBuf::from(std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into()));
        let mut child = Command::new(&cmd).args(["/c", "echo", "hello"]).stdout(Stdio::Piped).mitigations(mitigate::THEIRS).spawn().expect("cmd starts");
        let mut said = String::new();
        child.stdout.take().unwrap().read_to_string(&mut said).unwrap();
        assert_eq!(said.trim(), "hello");
        assert_eq!(child.wait(), 0);
        // whoami.exe is a program of its own: refused.
        let mut child = Command::new(&cmd).args(["/c", "whoami"]).stdout(Stdio::Piped).stderr(Stdio::Piped).mitigations(mitigate::THEIRS).spawn().unwrap();
        let mut said = String::new();
        child.stdout.take().unwrap().read_to_string(&mut said).unwrap();
        assert_ne!(child.wait(), 0, "it ran whoami: {said}");
    }

    #[test]
    fn its_environment_has_no_proxy_in_it_but_what_its_given() {
        // SAFETY: tests in this file don't read these variables from other threads.
        unsafe { std::env::set_var("HTTPS_PROXY", "socks5://example:1") };
        let cmd = PathBuf::from(std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into()));
        let mut child = Command::new(&cmd).args(["/c", "set"]).env("PMS_GIVEN", "yes").stdout(Stdio::Piped).mitigations(mitigate::THEIRS).spawn().unwrap();
        let mut said = String::new();
        child.stdout.take().unwrap().read_to_string(&mut said).unwrap();
        child.wait();
        assert!(!said.to_ascii_uppercase().contains("HTTPS_PROXY"), "{said}");
        assert!(said.contains("PMS_GIVEN=yes"));
    }

    /// A program that needs user32, started without win32k, can't set user32 up: it ends there, saying so in its exit code
    /// (STATUS_DLL_INIT_FAILED), and puts no box on the desktop that would keep it waiting for a click.
    #[test]
    fn one_that_cant_start_ends_without_a_box() {
        let notepad = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join("System32").join("notepad.exe");
        let mut child = Command::new(&notepad).mitigations(mitigate::THEIRS | mitigate::NO_WIN32K).spawn().expect("it's made");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let code = loop {
            if let Some(code) = child.try_wait() {
                break code;
            }
            if std::time::Instant::now() > until {
                child.kill();
                panic!("it's still there: waiting on a box?");
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        assert_eq!(code, 0xC000_0142);
    }

    /// Started in efficiency mode, it's in it from the start, and can be taken out and put back; this process can too.
    #[test]
    fn efficiency_mode_is_had_from_the_start_and_can_be_changed() {
        let cmd = PathBuf::from(std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into()));
        let both = power::EFFICIENT | power::COARSE_TIMERS;
        let mut child = Command::new(&cmd).stdin(Stdio::Piped).stdout(Stdio::Piped).mitigations(mitigate::THEIRS).power(both).spawn().unwrap();
        // Before 22H2 nothing can be asked: then there's nothing to see here.
        if child.power().is_some() {
            assert_eq!(child.power(), Some(both));
            assert_eq!(power_of_process(child.id()), Some(both));
            assert!(child.set_power(power::WINDOWS));
            assert_eq!(child.power(), Some(power::WINDOWS));
            assert!(child.set_power(power::EFFICIENT));
            assert_eq!(child.power(), Some(power::EFFICIENT), "what's left out isn't kept from before");
        }
        // And it runs: it echoes what it's told.
        let mut stdin = child.stdin.take().unwrap();
        let _ = stdin.write_all(b"echo awake\r\nexit\r\n");
        drop(stdin);
        let mut said = String::new();
        child.stdout.take().unwrap().read_to_string(&mut said).unwrap();
        assert!(said.contains("awake"), "{said}");
        assert_eq!(child.wait(), 0);
        let me = unsafe { GetCurrentProcess() };
        let mine = power_of(me);
        assert!(set_own_power(both));
        if mine.is_some() {
            assert_eq!(power_of(me), Some(both));
        }
        set_own_power(mine.unwrap_or(power::WINDOWS));
    }

    #[test]
    fn dropped_it_ends() {
        let cmd = PathBuf::from(std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into()));
        let mut child = Command::new(&cmd).stdin(Stdio::Piped).stdout(Stdio::Piped).mitigations(mitigate::THEIRS).spawn().unwrap();
        let (mut stdin, process) = (child.stdin.take().unwrap(), child.handle() as usize);
        let mut copy = null_mut();
        unsafe { DuplicateHandle(GetCurrentProcess(), process as Handle, GetCurrentProcess(), &mut copy, 0, 0, DUPLICATE_SAME_ACCESS) };
        let _ = stdin.write_all(b"rem still here\r\n");
        assert!(child.try_wait().is_none(), "it waits for more");
        drop(child);
        assert_eq!(unsafe { WaitForSingleObject(copy, 5000) }, 0, "its job closed, and it ended");
        unsafe { CloseHandle(copy) };
    }
}
