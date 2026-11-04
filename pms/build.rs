//! The icon and version information of PimpMyShell's executables (pms-res), and the daemon's manifest.

use pms_res::Target::Bin;

fn main() {
    pms_res::embed(
        "../assets/icon/pms.ico",
        &[(Bin("pmsd"), "PimpMyShell"), (Bin("pms"), "PimpMyShell command line"), (Bin("pms-release"), "PimpMyShell release tool")],
    );
    // The daemon's dialogs (software update's question, the notices' explanations) take version 6 of the common
    // controls: task dialogs, with buttons of their own, and the current look. The linker writes the manifest into
    // the exe. The examples get it too, for dialogs.rs.
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if msvc {
        for target in ["-bin=pmsd", "-examples"] {
            println!("cargo:rustc-link-arg{target}=/MANIFEST:EMBED");
            println!(
                "cargo:rustc-link-arg{target}=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' \
                 version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
            );
        }
    }
}
