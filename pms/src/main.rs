//! `pms` CLI.
//!
//!   pms inject                   detach any live TAP, inject a fresh copy of pms_tap.dll
//!   pms                          interactive REPL against the TAP
//!   pms <command..>              one-shot command (`pms help` for the list)
//!   pms meters <file>            run just the perf-counter sampler that feeds the meter strip
//!   pms demo <gpu>[+<gpu>..]     the meters with GPUs this PC doesn't have (intel, radeon, nvidia, snapdragon)
//!   pms daemon [file]            keep the taskbar configured per pms.toml (foreground)
//!   pms daemon stop              stop a running daemon (restores the stock taskbar)
//!   pms gui [file]               settings window (edits the config live)
//!   pms autostart on [file]|off|status
//!                                     run the windowless daemon (pmsd.exe) at logon

use std::io::{BufRead, Write};
use std::time::Duration;

use pms::{config, daemon, quote, send};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const NO_TAP: &str = "no TAP listening; run `pms inject` (or `pms daemon`)";

/// One connection per line, so the REPL never blocks the daemon out of the single-instance pipe.
fn repl() -> Result<()> {
    send("ping").map_err(|e| format!("{NO_TAP} ({e})"))?;
    let stdin = std::io::stdin();
    loop {
        print!("pms> ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            return Ok(());
        }
        match line.trim() {
            "" => continue,
            "exit" | "quit" => return Ok(()),
            line => match send(line) {
                Ok(resp) => println!("{}", resp.trim_end()),
                Err(e) => println!("({e})"),
            },
        }
    }
}

/// Poll the snapshot; it carries a version, so only real changes get printed.
fn watch_layout() -> Result<()> {
    let mut last = String::new();
    loop {
        let snap = send("layout").map_err(|e| format!("{NO_TAP} ({e})"))?;
        if snap != last {
            println!("{}", snap.trim_end());
            println!();
            last = snap;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// `pms demo intel+radeon`: the meters as they'd look with GPUs this PC doesn't have, on a throwaway config, until
/// Ctrl+C. A daemon that's running steps aside, and comes back after.
fn demo(which: Option<&str>) -> Result<()> {
    let Some(which) = which else {
        let names: Vec<&str> = pms::meters::DEMO_GPUS.iter().map(|(key, ..)| *key).collect();
        println!("pms demo <gpu>[+<gpu>..]: pretend this PC has other GPUs, until Ctrl+C. Nothing is changed.\n  GPUs: {}\n  For example: pms demo intel+radeon", names.join(", "));
        return Ok(());
    };
    let gpus = pms::meters::demo(which)?;
    let was_running = daemon::running();
    if was_running {
        daemon::stop()?;
        while daemon::running() {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    let path = std::env::temp_dir().join("pms-demo.toml");
    std::fs::write(&path, "config_version = 2\nupdate_check = false\n")?;
    println!("Pretending this PC has: {}. Press Ctrl+C to stop.", gpus.join(", "));
    let result = daemon::run(&path);
    let _ = std::fs::remove_file(&path);
    if was_running {
        std::process::Command::new(std::env::current_exe()?.with_file_name("pmsd.exe")).spawn()?;
    }
    Ok(result?)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    match arg(0) {
        None => repl(),
        Some("inject") => pms::inject().map(|s| println!("{s}")).map_err(Into::into),
        Some("layout") if arg(1) == Some("watch") => watch_layout(),
        Some("meters") if args.len() == 2 && std::path::Path::new(&args[1]).is_file() => pms::meters::run(&args[1]).map_err(|e| pms::problem::Problem::of(std::path::Path::new(&args[1]), &e).words().into()),
        Some("restart-explorer") => pms::explorer::ask().map_err(Into::into),
        Some("daemon") if arg(1) == Some("stop") => daemon::stop().map(|s| println!("{s}")).map_err(Into::into),
        Some("demo") => demo(arg(1)),
        Some("daemon") => {
            daemon::migrate();
            let path = config::given(arg(1).map(Into::into));
            if config::is_default(&path) {
                config::ensure(&path)?;
            }
            if !path.is_file() {
                return Err(format!("no config at {}; copy pms.example.toml there or pass a path", path.display()).into());
            }
            Ok(daemon::run(&path)?)
        }
        Some("autostart") => daemon::autostart(&args[1..]).map(|s| println!("{s}")).map_err(Into::into),
        Some("gui") => {
            let exe = std::env::current_exe()?.with_file_name("pms-gui.exe");
            std::process::Command::new(&exe).args(&args[1..]).spawn().map_err(|e| format!("{}: {e}", exe.display()))?;
            Ok(())
        }
        Some(_) => {
            let line = args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
            println!("{}", send(&line).map_err(|e| format!("{NO_TAP} ({e})"))?.trim_end());
            Ok(())
        }
    }
}
