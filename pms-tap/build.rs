//! The TAP's version information and icon (pms-res): it's what explorer has loaded, as Task Manager and the like
//! show it.

fn main() {
    pms_res::embed("../assets/icon/pms.ico", &[(pms_res::Target::Dll("pms_tap"), "PimpMyShell, in the taskbar")]);
}