//! The daemon without a console window, for autostart. Logs to %APPDATA%\PimpMyShell\logs\daemon.log.
//!
//!   pmsd [config]      (default %APPDATA%\PimpMyShell\pms.toml, where one in %LOCALAPPDATA%\PimpMyShell, its
//!                       place before 0.1.3, moves the first time)

#![windows_subsystem = "windows"]

fn main() {
    pms::log_to_file();
    pms::daemon::migrate();
    let path = pms::config::given(std::env::args_os().nth(1));
    if let Err(e) = pms::config::ensure(&path) {
        pms::log(format!("no config: {e}"));
    }
    if let Err(e) = pms::daemon::run(&path) {
        pms::log(format!("daemon exited: {e}"));
    }
}
