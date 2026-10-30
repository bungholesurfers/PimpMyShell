//! The icon and version information of PimpMyShell's executables (pms-res).

use pms_res::Target::Bin;

fn main() {
    pms_res::embed(
        "../assets/icon/pms.ico",
        &[(Bin("pmsd"), "PimpMyShell"), (Bin("pms"), "PimpMyShell command line"), (Bin("pms-release"), "PimpMyShell release tool")],
    );
}