//! `cargo xtask`: PimpMyShell's build tasks, in Rust rather than shell scripts, so that they run wherever Cargo does.
//! Run from the workspace; `cargo xtask help` lists them.
//!
//! Everyday release builds are for working on PimpMyShell: no LTO, so a change rebuilds in seconds. Releases can take
//! every last percent with profile-guided optimisation, as Nagare's do, in steps or all at once (`pgo`): an
//! instrumented build (`pgo-instrument`), a training run with it on this very taskbar (`pgo-train`), what it
//! recorded merged into `pgo/pms.profdata` (POOM's into `pms-poom/pgo/poom.profdata`) beside a note of the compiler
//! (`pgo-merge`), and the optimised build from that, with fat LTO over one codegen unit (`pgo-build`). A profile fits only the compiler that recorded it
//! and the code it was recorded from: after big changes, train again.

use std::path::{Path, PathBuf};
use std::process::Command;

const USAGE: &str = "\
cargo xtask <task>:
  build                release build of every exe and the TAP: target/release
  check                clippy (warnings are errors) and the tests
  installer [--no-build] [--pgo] [--install]
                       build the release exes (unless --no-build; with --pgo, the optimised build from the kept
                       profile), package them with Inno Setup 6 into target/installer/pms-setup-VERSION.exe,
                       and with --install install it silently over the installed copy
  pgo [SECONDS]        the whole PGO pass: pgo-instrument, pgo-train, pgo-merge, pgo-build (a few minutes)
  pgo-instrument       the instrumented build, POOM's too, into target/pgo-instrumented
  pgo-train [SECONDS]  runs it on this taskbar for SECONDS (default 180, and about 120 at least) with pgo/train.toml:
                       every kind of meter, the TVs, the hot-mic alert, notifications, Freedoom fetched and played in
                       POOM. A running daemon is stopped first and started again after. Leave the PC alone meanwhile.
  pgo-merge            merge what it recorded into pgo/pms.profdata and pms-poom/pgo/poom.profdata (needs: rustup
                       component add llvm-tools)
  pgo-build            the optimised release build from those: target/release
  art                  render the icons and the installer's pictures (pms-art): assets/icon, installer/art
  help                 this
";

/// What the installer ships, from target/release.
const SHIPPED: &[&str] = &["pmsd.exe", "pms.exe", "pms-gui.exe", "pms_tap.dll", "pms-poom.exe", "pms-twatch.exe"];

type Result<T = ()> = std::result::Result<T, String>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let result = match args.first().map(String::as_str) {
        Some("build") => build(),
        Some("check") => check(),
        Some("art") => art(),
        Some("installer") => installer(rest.contains(&"--no-build"), rest.contains(&"--pgo"), rest.contains(&"--install")),
        Some("pgo") => seconds(&rest).and_then(pgo),
        Some("pgo-instrument") => pgo_instrument(),
        Some("pgo-train") => seconds(&rest).and_then(pgo_train),
        Some("pgo-merge") => pgo_merge(),
        Some("pgo-build") => pgo_build(),
        Some("help" | "--help" | "-h") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("no task {other:?}; cargo xtask help lists them")),
    };
    if let Err(e) = result {
        eprintln!("xtask: {e}");
        std::process::exit(1);
    }
}

/// The workspace root, wherever cargo xtask was run from.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lies in the workspace").to_path_buf()
}

fn cargo() -> Command {
    let mut c = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    c.current_dir(root());
    c
}

/// Runs `cmd`, its output shown as it comes, and fails if it does.
fn run(cmd: &mut Command) -> Result {
    let shown = format!("{} {}", cmd.get_program().to_string_lossy(), cmd.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    println!("> {shown}");
    let status = cmd.status().map_err(|e| format!("{shown}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{shown}: {status}")) }
}

fn step(name: &str) {
    println!("\n== {name}");
}

/// A running exe can't be written over on Windows (a daemon started from target/release, say), but it can be
/// renamed: such an exe moves aside, to `.old`, and keeps running from there.
fn free() {
    let dir = root().join("target").join("release");
    for name in SHIPPED.iter().filter(|n| n.ends_with(".exe")) {
        let path = dir.join(name);
        let old = path.with_extension("old");
        let _ = std::fs::remove_file(&old);
        let locked = path.exists() && std::fs::OpenOptions::new().write(true).open(&path).is_err();
        if locked && std::fs::rename(&path, &old).is_ok() {
            println!("{} is running; moved it aside to {}", path.display(), old.display());
        }
    }
}

/// The logo (assets/icon) and the installer's box and pictures (installer/art), rendered from SVG by pms-art. The
/// results are kept with the source, so a build needs no rendering.
fn art() -> Result {
    step("art");
    run(cargo().args(["run", "--release", "-p", "pms-art", "--", "--render"]).arg(root()))
}

fn build() -> Result {
    step("release build");
    free();
    run(cargo().args(["build", "--release", "-p", "pms", "-p", "pms-gui", "-p", "pms-tap", "-p", "pms-twatch"]))?;
    doom(false)
}

/// pms-poom: GPL, and a workspace of its own, so that nothing of PimpMyShell's links it. Built apart and put with the rest. For a
/// release (`release`), as the rest is: fat LTO, and the profile kept for it if there is one.
fn doom(release: bool) -> Result {
    step("pms-poom");
    let target = root().join("target").join("poom");
    let mut cmd = cargo();
    cmd.current_dir(root().join("pms-poom")).args(["build", "--release", "--target-dir"]).arg(&target);
    if release {
        release_profile(&mut cmd);
        let kept = kept_poom_profile();
        if kept.exists() {
            check_compiler(&kept)?;
            cmd.env("RUSTFLAGS", format!("-Cprofile-use={}", kept.display()));
        } else {
            println!("warning: no {}: POOM is built with fat LTO but no profile", kept.display());
        }
    }
    run(&mut cmd)?;
    let to = root().join("target").join("release").join("pms-poom.exe");
    std::fs::copy(target.join("release").join("pms-poom.exe"), &to).map_err(|e| format!("{}: {e}", to.display()))?;
    shipped_built()
}

/// Every build ends with all that ships: one left out of a build fails here, not in a release's packaging.
fn shipped_built() -> Result {
    let dir = root().join("target").join("release");
    match SHIPPED.iter().find(|n| !dir.join(n).exists()) {
        Some(missing) => Err(format!("{missing} wasn't built: add its package to the build")),
        None => Ok(()),
    }
}

fn check() -> Result {
    step("clippy");
    run(cargo().args(["clippy", "--release", "--workspace", "--", "-D", "warnings"]))?;
    step("tests");
    run(cargo().args(["test", "--release", "--workspace"]))
}

/// The app's version: the workspace's (`[workspace.package]` in Cargo.toml), which every crate takes.
fn version() -> Result<String> {
    let toml = std::fs::read_to_string(root().join("Cargo.toml")).map_err(|e| e.to_string())?;
    toml.lines()
        .skip_while(|l| l.trim() != "[workspace.package]")
        .find_map(|l| l.strip_prefix("version = \"").and_then(|v| v.strip_suffix('"')))
        .map(str::to_string)
        .ok_or("no version in Cargo.toml's [workspace.package]".into())
}

/// Inno Setup's compiler, where a per-user or machine-wide install puts it.
fn iscc() -> Result<PathBuf> {
    let mut places = Vec::new();
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        places.push(PathBuf::from(l).join("Programs").join("Inno Setup 6").join("ISCC.exe"));
    }
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(p) = std::env::var_os(var) {
            places.push(PathBuf::from(p).join("Inno Setup 6").join("ISCC.exe"));
        }
    }
    places.into_iter().find(|p| p.exists()).ok_or("Inno Setup 6 not found; winget install --id JRSoftware.InnoSetup --scope user".into())
}

fn installer(no_build: bool, pgo: bool, install: bool) -> Result {
    if pgo {
        pgo_build()?;
    } else if !no_build {
        build()?;
    }
    step("installer");
    let v = version()?;
    let dir = root().join("target").join("release");
    if let Some(missing) = SHIPPED.iter().find(|n| !dir.join(n).exists()) {
        return Err(format!("{missing} isn't built; run cargo xtask installer without --no-build"));
    }
    run(Command::new(iscc()?).current_dir(root().join("installer")).args(["/Q", &format!("/DAppVersion={v}"), "pms.iss"]))?;
    let setup = root().join("target").join("installer").join(format!("pms-setup-{v}.exe"));
    println!("built {}", setup.display());
    if install {
        step("install");
        // No wizard and no questions; start at sign-in stays as it is, and what the installer stopped is started again.
        let log = std::env::temp_dir().join("pms-setup.log");
        println!("installing (log: {})", log.display());
        run(Command::new(&setup).args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]).arg(format!("/LOG={}", log.display())))?;
        let installed = std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Programs").join("PimpMyShell")).unwrap_or_default();
        println!("installed into {}", installed.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Profile-guided optimisation

fn seconds(rest: &[&str]) -> Result<u64> {
    rest.first().map_or(Ok(180), |s| s.parse().map_err(|_| format!("{s:?}: the training's length, in seconds")))
}

/// A release takes every last percent: fat LTO over one codegen unit, and no incremental compilation.
fn release_profile(cmd: &mut Command) -> &mut Command {
    cmd.env("CARGO_PROFILE_RELEASE_LTO", "fat").env("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "1").env("CARGO_PROFILE_RELEASE_INCREMENTAL", "false")
}

fn profiles_dir() -> PathBuf {
    root().join("target").join("pgo-profiles")
}

/// POOM's recordings, apart from the rest: its profile is kept apart too.
fn poom_profiles_dir() -> PathBuf {
    root().join("target").join("pgo-profiles-poom")
}

fn instrumented() -> PathBuf {
    root().join("target").join("pgo-instrumented").join("release")
}

fn kept_profile() -> PathBuf {
    root().join("pgo").join("pms.profdata")
}

/// POOM's profile, with its source: it's what running a GPL program recorded, and stays on that side.
fn kept_poom_profile() -> PathBuf {
    root().join("pms-poom").join("pgo").join("poom.profdata")
}

/// The compiler's own version line, which the profile's note records.
fn compiler() -> Result<String> {
    let out = Command::new("rustc").arg("-V").current_dir(root()).output().map_err(|e| format!("rustc: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Says so when `kept` was recorded with another compiler than this one, which it may not fit.
fn check_compiler(kept: &Path) -> Result {
    let recorded = std::fs::read_to_string(kept.with_extension("profdata.txt")).map(|s| s.trim().to_string()).unwrap_or_else(|_| "unknown".into());
    let compiler = compiler()?;
    if recorded != compiler {
        println!("warning: {} was recorded with '{recorded}', this build is '{compiler}': it may not apply", kept.display());
    }
    Ok(())
}

/// `dir`, empty.
fn fresh(dir: &Path) -> Result {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

fn pgo_instrument() -> Result {
    step("PGO: instrumented build");
    // `pms_pgo` gives the TAP its pgo-dump command (it lives in explorer, which never exits to write it), and has the daemon let
    // go of the console rather than end it.
    let flags = format!("-Cprofile-generate={} --cfg pms_pgo", profiles_dir().display());
    // The exes and the TAP, then what the training does things with (naming an example builds only that): those record into the
    // same profile, which is how fetching a game, in the library, gets measured.
    for targets in [&["-p", "pms", "-p", "pms-tap"][..], &["-p", "pms", "--example", "hold_mic", "--example", "pgo_poke"]] {
        let mut cmd = cargo();
        release_profile(&mut cmd).env("RUSTFLAGS", &flags).args(["build", "--release", "--target-dir", "target/pgo-instrumented"]).args(targets);
        run(&mut cmd)?;
    }
    // POOM, a workspace of its own, recording into a folder of its own, and put beside the daemon that starts it.
    let poom = root().join("target").join("pgo-instrumented-poom");
    let mut cmd = cargo();
    release_profile(&mut cmd)
        .current_dir(root().join("pms-poom"))
        .env("RUSTFLAGS", format!("-Cprofile-generate={} --cfg pms_pgo", poom_profiles_dir().display()))
        .args(["build", "--release", "--target-dir"])
        .arg(&poom);
    run(&mut cmd)?;
    let to = instrumented().join("pms-poom.exe");
    std::fs::copy(poom.join("release").join("pms-poom.exe"), &to).map_err(|e| format!("{}: {e}", to.display()))?;
    // The notifications come from toast_lab, which isn't what's measured: an ordinary build.
    run(cargo().args(["build", "--release", "-p", "pms", "--example", "toast_lab"]))?;
    // Build scripts and proc macros were instrumented too, and ran: what they recorded is no use.
    fresh(&profiles_dir())?;
    fresh(&poom_profiles_dir())
}

/// Runs a PimpMyShell exe, waiting for it, ignoring how it went (some steps are only courtesy).
fn quietly(exe: &Path, args: &[&str]) -> bool {
    Command::new(exe).args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Whether toast_lab's app id is registered (by an earlier `toast_lab register`, the training's or somebody's own).
fn toast_lab_registered() -> bool {
    quietly(Path::new("reg"), &["query", r"HKCU\Software\Classes\AppUserModelId\PimpMyShell.ToastLab"])
}

/// Waits for every process of `image` to end, for up to `patience`: whether they did.
fn gone(image: &str, patience: std::time::Duration) -> bool {
    let until = std::time::Instant::now() + patience;
    loop {
        let out = Command::new("tasklist").args(["/FI", &format!("IMAGENAME eq {image}"), "/NH"]).output();
        if out.map_or(true, |o| !String::from_utf8_lossy(&o.stdout).contains(image)) {
            return true;
        }
        if std::time::Instant::now() >= until {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// What the training does, and when (seconds from the daemon starting). It takes about two minutes whatever's asked for; the time
/// asked for beyond that is everyday running, which is most of what people do.
fn pgo_train(seconds: u64) -> Result {
    use std::time::{Duration, Instant};
    let dir = instrumented();
    let (daemon, cli) = (dir.join("pmsd.exe"), dir.join("pms.exe"));
    let (hold, poke) = (dir.join("examples").join("hold_mic.exe"), dir.join("examples").join("pgo_poke.exe"));
    let lab = root().join("target").join("release").join("examples").join("toast_lab.exe");
    if !daemon.exists() || !poke.exists() {
        return Err(format!("no instrumented build at {}; cargo xtask pgo-instrument first", dir.display()));
    }
    step(&format!("PGO: training on this taskbar for {seconds} s"));
    // The training's own files: its config, and beside it the TVs' state and the console's home, game and saves. Yours are left
    // alone. Each run starts the same way: the TVs as new.
    let home = root().join("target").join("pgo-train");
    let poom_home = home.join("poom");
    std::fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
    let config = home.join("pms.toml");
    let settings = std::fs::read_to_string(root().join("pgo").join("train.toml")).map_err(|e| format!("pgo/train.toml: {e}"))?;
    std::fs::write(&config, &settings).map_err(|e| format!("{}: {e}", config.display()))?;
    let _ = std::fs::remove_file(home.join("tv.txt"));
    // The game, fetched each time as the settings fetch it (through your proxy, if you have one), so that's measured too. If that
    // fails three times (GitHub has its moments), the one from last time does.
    let fetched = (1..=3).any(|attempt| {
        if attempt > 1 {
            std::thread::sleep(Duration::from_secs(10));
        }
        println!("fetching Freedoom ({attempt} of 3)");
        Command::new(&poke).arg("wad").env("PMS_POOM_HOME", &poom_home).status().is_ok_and(|s| s.success())
    });
    let wads = poom_home.join("wads");
    let has_wad = |dir: &Path| std::fs::read_dir(dir).is_ok_and(|d| d.flatten().any(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("wad"))));
    // With none from before either, a copy of one of yours, so that POOM is trained all the same (the fetching isn't).
    if !fetched && !has_wad(&wads) {
        let yours = std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("PimpMyShell").join("vidya").join("poom").join("wads"));
        let first = yours.and_then(|y| std::fs::read_dir(y).ok()).and_then(|d| d.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wad"))));
        if let Some(wad) = first
            && std::fs::create_dir_all(&wads).is_ok()
            && std::fs::copy(&wad, wads.join(wad.file_name().unwrap())).is_ok()
        {
            println!("couldn't fetch Freedoom; training with a copy of {}", wad.display());
        }
    }
    let game = fetched || has_wad(&wads);
    if !game {
        println!("warning: no game, so POOM isn't trained (and keeps the profile it had)");
    }
    // Whichever daemon runs now (ours, the installed one) steps aside, and comes back after.
    let running = running_daemon();
    let was_running = quietly(&cli, &["daemon", "stop"]);
    if was_running {
        println!("stopped the running daemon; it starts again after the training");
        std::thread::sleep(Duration::from_secs(3));
    }
    // The instrumented TAP goes into explorer in place of the one there, with no daemon running meanwhile.
    if !quietly(&cli, &["inject"]) {
        return Err("couldn't inject the instrumented TAP into explorer".into());
    }
    println!("injected the instrumented TAP");
    // Notifications come from toast_lab's app id, registered for the training if it isn't already, and only then unregistered after.
    let registered = toast_lab_registered();
    let lab_ok = lab.exists() && (registered || quietly(&lab, &["register"]));
    if !lab_ok {
        println!("warning: toast_lab isn't there to send notifications, so toasts aren't trained");
    }
    let mut d = Command::new(&daemon).arg(&config).env("PMS_POOM_HOME", &poom_home).spawn().map_err(|e| format!("{}: {e}", daemon.display()))?;
    let start = Instant::now();
    let until = |t: u64| std::thread::sleep((start + Duration::from_secs(t)).saturating_duration_since(Instant::now()));
    let pause = |ms: u64| std::thread::sleep(Duration::from_millis(ms));
    let poke = |args: &[&str]| quietly(&poke, args);
    let mic = |s: &str| hold.exists() && quietly(&hold, &[s]);

    // 0 s: both TVs flip up through their channels, from 1295 round to the console on 6, which comes on and plays its demos.
    // 8 s: the microphone: a long hold (the broad warning, then the label), then quick ones (the label's "closed" path).
    until(8);
    println!("holding the microphone");
    mic("12");
    for _ in 0..3 {
        pause(1500);
        mic("1");
    }
    // 30 s: notifications, one kind after another, then a pile of them at once.
    until(30);
    if lab_ok {
        println!("notifications");
        quietly(&lab, &["--gap", "2", "plain", "long", "hero", "buttons", "progress", "header"]);
        quietly(&lab, &["plain", "logo", "circle", "inline", "attribution"]);
    }
    // 50 s: the microphone's label moved before the meters (the config changing as the settings change it), held there, and back.
    until(50);
    println!("the microphone's label at the start");
    let _ = std::fs::write(&config, settings.replace("position = \"tray\"", "position = \"start\""));
    pause(2000);
    mic("6");
    let _ = std::fs::write(&config, &settings);
    // 62 s: POOM in its window: a new game, walking, turning, shooting, the map; closed (which pauses it), and opened again.
    until(62);
    if game {
        println!("playing POOM");
        poke(&["click", "poom"]);
        pause(2000);
        poke(&[
            "keys", "esc", "wait:600", "enter", "wait:600", "enter", "wait:600", "enter", "wait:1500", "up:2500", "right:700", "up:2000", "fire", "fire", "fire", "use", "left:900",
            "up:1500", "tab", "wait:1500", "tab", "fire:1000",
        ]);
        poke(&["close"]);
        pause(2000);
        poke(&["click", "poom"]);
        pause(4000);
        poke(&["close"]);
    }
    // 95 s: the second set shuffles, every 5 s; the first goes up the channels, and back to the console.
    until(95);
    println!("the TVs' remotes");
    poke(&["press", "TV 2", "shuffle"]);
    for _ in 0..3 {
        poke(&["press", "TV", "up"]);
        pause(3000);
    }
    poke(&["press", "TV", "game"]);
    // Then everyday running, for the rest of the time asked for.
    until(seconds);

    println!("writing the profiles");
    if !quietly(&cli, &["pgo-dump"]) {
        println!("warning: the TAP didn't write its profile (was it the instrumented one?)");
    }
    quietly(&cli, &["daemon", "stop"]);
    let _ = d.wait();
    // The daemon let go of the console, which ends with it, writing its profile as it goes.
    if game && !gone("pms-poom.exe", Duration::from_secs(15)) {
        println!("warning: POOM didn't end with the daemon, so it may not have written its profile");
    }
    if lab_ok && !registered {
        quietly(&lab, &["unregister"]);
    }
    if was_running {
        // Its own TAP goes back into explorer before it starts again, in place of the instrumented one (it would swap its own in by
        // itself, but only once it noticed).
        let normal = running.unwrap_or_else(|| root().join("target").join("release").join("pmsd.exe"));
        if !quietly(&normal.with_file_name("pms.exe"), &["inject"]) {
            println!("warning: couldn't put the usual TAP back into explorer; the daemon swaps it in when it starts");
        }
        // On its own: nothing of this console's, which would keep whatever reads it waiting, and fail its writes once that's gone.
        let alone = || std::process::Stdio::null();
        match Command::new(&normal).stdin(alone()).stdout(alone()).stderr(alone()).spawn() {
            Ok(_) => println!("started {} again", normal.display()),
            Err(e) => println!("couldn't start {} again: {e}", normal.display()),
        }
    } else {
        // With no daemon to drive it, the instrumented TAP lets go of the taskbar; the next daemon puts its own in.
        quietly(&cli, &["detach"]);
    }
    Ok(())
}

/// The running daemon's exe, if one runs (to start the same one again after the training).
fn running_daemon() -> Option<PathBuf> {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", "(Get-Process pmsd -ErrorAction SilentlyContinue | Select-Object -First 1).Path"])
        .output()
        .ok()?;
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    // One a build moved aside (free) comes back as the build that took its place.
    let path = if path.extension().is_some_and(|x| x == "old") { path.with_extension("exe") } else { path };
    path.is_file().then_some(path)
}

/// llvm-profdata, from the toolchain's llvm-tools.
fn profdata() -> Result<PathBuf> {
    let sysroot = Command::new("rustc").args(["--print", "sysroot"]).current_dir(root()).output().map_err(|e| format!("rustc: {e}"))?;
    let sysroot = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim());
    let host = Command::new("rustc").arg("-vV").current_dir(root()).output().map_err(|e| format!("rustc: {e}"))?;
    let host = String::from_utf8_lossy(&host.stdout).lines().find_map(|l| l.strip_prefix("host: ")).map(str::to_string).ok_or("rustc names no host")?;
    let profdata = sysroot.join("lib").join("rustlib").join(&host).join("bin").join("llvm-profdata.exe");
    if profdata.exists() { Ok(profdata) } else { Err("llvm-profdata not found; run: rustup component add llvm-tools".into()) }
}

/// Whether anything was recorded into `profiles`.
fn recorded(profiles: &Path) -> bool {
    std::fs::read_dir(profiles).map(|d| d.filter_map(|e| e.ok()).any(|e| e.path().extension().is_some_and(|x| x == "profraw"))).unwrap_or(false)
}

/// What's in `profiles`, merged into `kept`, beside a note of the compiler that recorded it.
fn merge(profdata: &Path, profiles: &Path, kept: &Path) -> Result {
    std::fs::create_dir_all(kept.parent().unwrap()).map_err(|e| e.to_string())?;
    run(Command::new(profdata).arg("merge").arg("-o").arg(kept).arg(profiles))?;
    let compiler = compiler()?;
    std::fs::write(kept.with_extension("profdata.txt"), format!("{compiler}\n")).map_err(|e| e.to_string())?;
    println!("kept the profile: {} (recorded with {compiler})", kept.display());
    Ok(())
}

fn pgo_merge() -> Result {
    step("PGO: merging the profiles");
    let profdata = profdata()?;
    if !recorded(&profiles_dir()) {
        return Err(format!("nothing recorded in {}; cargo xtask pgo-train first", profiles_dir().display()));
    }
    merge(&profdata, &profiles_dir(), &kept_profile())?;
    if recorded(&poom_profiles_dir()) {
        merge(&profdata, &poom_profiles_dir(), &kept_poom_profile())
    } else {
        println!("warning: POOM recorded nothing; {} stays as it was", kept_poom_profile().display());
        Ok(())
    }
}

fn pgo_build() -> Result {
    step("PGO: optimised build");
    let kept = kept_profile();
    if !kept.exists() {
        return Err(format!("no profile at {}; cargo xtask pgo first", kept.display()));
    }
    check_compiler(&kept)?;
    free();
    let mut cmd = cargo();
    release_profile(&mut cmd)
        .env("RUSTFLAGS", format!("-Cprofile-use={}", kept.display()))
        .args(["build", "--release", "-p", "pms", "-p", "pms-gui", "-p", "pms-tap", "-p", "pms-twatch"]);
    run(&mut cmd)?;
    doom(true)
}

fn pgo(seconds: u64) -> Result {
    pgo_instrument()?;
    pgo_train(seconds)?;
    pgo_merge()?;
    pgo_build()
}
