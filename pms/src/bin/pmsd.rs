//! The daemon without a console window, for autostart. Logs to %LOCALAPPDATA%\PimpMyShell\daemon.log.
//!
//!   pmsd [config]      (default %LOCALAPPDATA%\PimpMyShell\pms.toml)

#![windows_subsystem = "windows"]

use std::path::PathBuf;

fn main() {
    pms::log_to_file();
    pms::daemon::migrate();
    let path = std::env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(pms::config::default_path);
    if let Err(e) = pms::config::ensure(&path) {
        pms::log(format!("no config: {e}"));
    }
    if let Err(e) = pms::daemon::run(&path) {
        pms::log(format!("daemon exited: {e}"));
    }
}
