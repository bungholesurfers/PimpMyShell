//! Prints the size a screen is being shown at, for trying display scales: `cargo run --release -p pms-screen --example wanted -- TV`.

fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| "TV".to_string());
    match pms_screen::Screen::open(&name) {
        Ok(screen) => println!("{:?}", screen.wanted()),
        Err(e) => println!("{e}"),
    }
}
