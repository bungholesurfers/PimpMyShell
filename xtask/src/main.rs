//! `cargo xtask`: PimpMyShell's build tasks, in Rust rather than shell scripts, so that they run wherever Cargo does.
//! Run from the workspace; `cargo xtask help` lists them.
//!
//! Everyday release builds are for working on PimpMyShell: no LTO, so a change rebuilds in seconds. Releases can take
//! every last percent with profile-guided optimisation, as Nagare's do, in steps or all at once (`pgo`): an
//! instrumented build (`pgo-instrument`), a training run with it on this very taskbar (`pgo-train`), what it
//! recorded merged into `pgo/pms.profdata` beside a note of the compiler (`pgo-merge`), and the optimised build
//! from that, with fat LTO over one codegen unit (`pgo-build`). A profile fits only the compiler that recorded it
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
  pgo-instrument       the instrumented build, into target/pgo-instrumented
  pgo-train [SECONDS]  runs it on this taskbar for SECONDS (default 60): meters, the hot-mic alert, toasts.
                       A running daemon is stopped first and started again after.
  pgo-merge            merge what it recorded into pgo/pms.profdata (needs: rustup component add llvm-tools)
  pgo-build            the optimised release build from pgo/pms.profdata: target/release
  art                  render the icons and the installer's pictures (pms-art): assets/icon, installer/art
  help                 this
";

/// What the installer ships, from target/release.
const SHIPPED: &[&str] = &["pmsd.exe", "pms.exe", "pms-gui.exe", "pms_tap.dll"];

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
    run(cargo().args(["build", "--release", "-p", "pms", "-p", "pms-gui", "-p", "pms-tap"]))
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
        // No wizard and no questions; the tasks are those chosen last time (start at sign-in, ticked to begin with),
        // and a daemon the installer stopped is started again.
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
    rest.first().map_or(Ok(60), |s| s.parse().map_err(|_| format!("{s:?}: the training's length, in seconds")))
}

/// A release takes every last percent: fat LTO over one codegen unit, and no incremental compilation.
fn release_profile(cmd: &mut Command) -> &mut Command {
    cmd.env("CARGO_PROFILE_RELEASE_LTO", "fat").env("CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "1").env("CARGO_PROFILE_RELEASE_INCREMENTAL", "false")
}

fn profiles_dir() -> PathBuf {
    root().join("target").join("pgo-profiles")
}

fn instrumented() -> PathBuf {
    root().join("target").join("pgo-instrumented").join("release")
}

fn kept_profile() -> PathBuf {
    root().join("pgo").join("pms.profdata")
}

/// The compiler's own version line, which the profile's note records.
fn compiler() -> Result<String> {
    let out = Command::new("rustc").arg("-V").current_dir(root()).output().map_err(|e| format!("rustc: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn pgo_instrument() -> Result {
    step("PGO: instrumented build");
    let profiles = profiles_dir();
    let _ = std::fs::remove_dir_all(&profiles);
    std::fs::create_dir_all(&profiles).map_err(|e| format!("{}: {e}", profiles.display()))?;
    // `pms_pgo` gives the TAP its pgo-dump command (it lives in explorer, which never exits to write it).
    let flags = format!("-Cprofile-generate={} --cfg pms_pgo", profiles.display());
    // The exes and the TAP, then the training's microphone holder (naming an example builds only that).
    for targets in [&["-p", "pms", "-p", "pms-tap"][..], &["-p", "pms", "--example", "hold_mic"]] {
        let mut cmd = cargo();
        release_profile(&mut cmd).env("RUSTFLAGS", &flags).args(["build", "--release", "--target-dir", "target/pgo-instrumented"]).args(targets);
        run(&mut cmd)?;
    }
    // Build scripts and proc macros were instrumented too, and ran: what they recorded is no use.
    let _ = std::fs::remove_dir_all(&profiles);
    std::fs::create_dir_all(&profiles).map_err(|e| format!("{}: {e}", profiles.display()))
}

/// Runs a PimpMyShell exe, waiting for it, ignoring how it went (some steps are only courtesy).
fn quietly(exe: &Path, args: &[&str]) -> bool {
    Command::new(exe).args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

fn pgo_train(seconds: u64) -> Result {
    use std::time::{Duration, Instant};
    let dir = instrumented();
    let (daemon, cli, hold) = (dir.join("pmsd.exe"), dir.join("pms.exe"), dir.join("examples").join("hold_mic.exe"));
    if !daemon.exists() {
        return Err(format!("no instrumented build at {}; cargo xtask pgo-instrument first", dir.display()));
    }
    step(&format!("PGO: training on this taskbar for {seconds} s"));
    // Whichever daemon runs now (ours, the installed one) steps aside, and comes back after.
    let was_running = quietly(&cli, &["daemon", "stop"]);
    if was_running {
        println!("stopped the running daemon; it starts again after the training");
        std::thread::sleep(Duration::from_secs(3));
    }
    // The bundled example exercises nearly everything: every meter style, the backdrop, the hot mic, toasts.
    let config = std::env::temp_dir().join("pms-pgo.toml");
    std::fs::copy(root().join("pms.example.toml"), &config).map_err(|e| e.to_string())?;
    let mut d = Command::new(&daemon).arg(&config).spawn().map_err(|e| format!("{}: {e}", daemon.display()))?;
    let start = Instant::now();
    let until = |t: u64| {
        let at = start + Duration::from_secs(t);
        std::thread::sleep(at.saturating_duration_since(Instant::now()));
    };
    // The microphone: a long hold (the broad warning, then the label), then quick ones (the label's "closed" path).
    until(8);
    if hold.exists() && seconds >= 40 {
        println!("holding the microphone");
        quietly(&hold, &["12"]);
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(1500));
            quietly(&hold, &["1"]);
        }
    }
    until(seconds);
    println!("writing the profiles");
    if !quietly(&cli, &["pgo-dump"]) {
        println!("warning: the TAP didn't write its profile (was it the instrumented one?)");
    }
    quietly(&cli, &["daemon", "stop"]);
    let _ = d.wait();
    let _ = std::fs::remove_file(&config);
    if was_running {
        let normal = root().join("target").join("release").join("pmsd.exe");
        match Command::new(&normal).spawn() {
            Ok(_) => println!("started {} again", normal.display()),
            Err(e) => println!("couldn't start {} again: {e}", normal.display()),
        }
    }
    Ok(())
}

fn pgo_merge() -> Result {
    step("PGO: merging the profiles");
    let sysroot = Command::new("rustc").args(["--print", "sysroot"]).current_dir(root()).output().map_err(|e| format!("rustc: {e}"))?;
    let sysroot = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim());
    let host = Command::new("rustc").arg("-vV").current_dir(root()).output().map_err(|e| format!("rustc: {e}"))?;
    let host = String::from_utf8_lossy(&host.stdout).lines().find_map(|l| l.strip_prefix("host: ")).map(str::to_string).ok_or("rustc names no host")?;
    let profdata = sysroot.join("lib").join("rustlib").join(&host).join("bin").join("llvm-profdata.exe");
    if !profdata.exists() {
        return Err("llvm-profdata not found; run: rustup component add llvm-tools".into());
    }
    let profiles = profiles_dir();
    let recorded = std::fs::read_dir(&profiles).map(|d| d.filter_map(|e| e.ok()).any(|e| e.path().extension().is_some_and(|x| x == "profraw"))).unwrap_or(false);
    if !recorded {
        return Err(format!("nothing recorded in {}; cargo xtask pgo-train first", profiles.display()));
    }
    let kept = kept_profile();
    std::fs::create_dir_all(kept.parent().unwrap()).map_err(|e| e.to_string())?;
    run(Command::new(&profdata).arg("merge").arg("-o").arg(&kept).arg(&profiles))?;
    let compiler = compiler()?;
    std::fs::write(kept.with_extension("profdata.txt"), format!("{compiler}\n")).map_err(|e| e.to_string())?;
    println!("kept the profile: {} (recorded with {compiler})", kept.display());
    Ok(())
}

fn pgo_build() -> Result {
    step("PGO: optimised build");
    let kept = kept_profile();
    if !kept.exists() {
        return Err(format!("no profile at {}; cargo xtask pgo first", kept.display()));
    }
    let recorded = std::fs::read_to_string(kept.with_extension("profdata.txt")).map(|s| s.trim().to_string()).unwrap_or_else(|_| "unknown".into());
    let compiler = compiler()?;
    if recorded != compiler {
        println!("warning: the profile was recorded with '{recorded}', this build is '{compiler}': it may not apply");
    }
    free();
    let mut cmd = cargo();
    release_profile(&mut cmd)
        .env("RUSTFLAGS", format!("-Cprofile-use={}", kept.display()))
        .args(["build", "--release", "-p", "pms", "-p", "pms-gui", "-p", "pms-tap"]);
    run(&mut cmd)
}

fn pgo(seconds: u64) -> Result {
    pgo_instrument()?;
    pgo_train(seconds)?;
    pgo_merge()?;
    pgo_build()
}