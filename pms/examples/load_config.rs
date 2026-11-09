//! Loads a settings file as PimpMyShell does, and says how it went: `load_config <pms.toml>`. It writes nothing but what loading
//! itself does: favorites and network settings still in the file are moved to their own files beside it.
fn main() {
    let path = std::path::PathBuf::from(std::env::args().nth(1).expect("load_config <pms.toml>"));
    match pms::config::load(&path) {
        Ok(c) => {
            println!("loaded: {} meters, {} setting(s) that couldn't be used", c.meters.len(), c.issues.len());
            for e in &c.issues {
                println!("  {}", pms::problem::Problem::of(&path, e).words().replace('\n', " | "));
            }
            println!("network: {}", c.network.map_or("none".into(), |n| format!("proxy set: {}, streams {}, isolate {}", !n.proxy.is_empty(), n.streams, n.isolate)));
        }
        Err(e) => println!("didn't load: {e}"),
    }
}
