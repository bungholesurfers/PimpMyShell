//! Shows PimpMyShell's dialogs as the daemon would, and prints what was chosen. Nothing is kept or changed: the
//! config isn't touched, and no daemon is asked to do anything.
//!
//!   cargo run --example dialogs -- update     the question software update asks once (OK, or closing it, is yes)
//!   cargo run --example dialogs -- access     the explanation behind the notice that notification access is off
//!   cargo run --example dialogs -- config     the one behind a pms.toml that didn't load (a made-up error)
//!   cargo run --example dialogs -- replaced [name]   the one for a taskbar that isn't Windows'
//!   cargo run --example dialogs -- click      the question before a notification opens something (a made-up one)
//!   cargo run --example dialogs -- problem    the one that asks for a report (a made-up error)
//!   cargo run --example dialogs -- post       what a saved report says about posting it

use pms::notices;

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("update") => println!("update_check = {}", pms::update_question(false)),
        Some("access") => println!("{:?}", notices::choose(&notices::notification_access())),
        Some("config") => {
            let path = pms::config::default_path();
            let error = format!(
                "{}: TOML parse error at line 12, column 1\n   |\n12 | interval_ms = \"soon\"\n   | ^^^^^^^^^^^\ninvalid type: string \"soon\", expected u64",
                path.display()
            );
            println!("{:?}", notices::choose(&notices::config(&path, &error, true)));
        }
        Some("replaced") => println!("{:?}", notices::choose(&notices::taskbar_replaced(std::env::args().nth(2).as_deref()))),
        Some("click") => {
            let long = (0..30).map(|i| format!("key{i}: a value that goes on for a while")).collect::<Vec<_>>().join("\n");
            let rows = [("Program".to_string(), r"C:\Program Files\Telegram\Telegram.exe".to_string()), ("Arguments".to_string(), long)];
            let a = pms::ask_click("Someone sent you a message", "Open Telegram?", &rows);
            println!("yes = {}, quiet = {}", a.yes, a.quiet);
        }
        Some("problem") => println!("{:?}", notices::choose(&notices::problem("The meters couldn't start", "error: no meter #3"))),
        Some("post") => println!("post = {}", notices::wants_to_post()),
        _ => eprintln!("usage: dialogs update|access|config|replaced [name]|click|problem|post"),
    }
}
