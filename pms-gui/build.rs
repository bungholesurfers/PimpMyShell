//! The settings' icon and version information (pms-res).

fn main() {
    pms_res::embed("../assets/icon/pms.ico", &[(pms_res::Target::Bin("pms-gui"), "PimpMyShell settings")]);
}