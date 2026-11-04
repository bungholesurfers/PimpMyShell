//! Shows PimpMyShell's dialogs as the daemon would, and prints what was chosen. Nothing is kept or changed: the
//! config isn't touched, and no daemon is asked to do anything.
//!
//!   cargo run --example dialogs -- update     the question software update asks once (OK, or closing it, is yes)
//!   cargo run --example dialogs -- access     the explanation behind the notice that notification access is off
//!   cargo run --example dialogs -- config     the one behind a pms.toml that didn't load (a made-up error)

use pms::notices;

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("update") => println!("update_check = {}", pms::update_question()),
        Some("access") => println!("{:?}", notices::choose(&notices::notification_access())),
        Some("config") => {
            let path = pms::config::default_path();
            let error = format!(
                "{}: TOML parse error at line 12, column 1\n   |\n12 | interval_ms = \"soon\"\n   | ^^^^^^^^^^^\ninvalid type: string \"soon\", expected u64",
                path.display()
            );
            println!("{:?}", notices::choose(&notices::config(&path, &error, true)));
        }
        _ => eprintln!("usage: dialogs update|access|config"),
    }
}
