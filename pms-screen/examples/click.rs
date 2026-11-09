//! Clicks the screen `doom`, as the taskbar would: `cargo run --release -p pms-screen --example click`.

fn main() {
    let screen = pms_screen::Screen::open("doom").expect("screen");
    println!("clicked; drawn by process {}", screen.click());
}
