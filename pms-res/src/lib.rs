//! For build scripts: puts PimpMyShell's icon (assets/icon/pms.ico, which pms-art renders) and version information
//! into executables, as Nagare's build does - written here as a compiled resource file per executable (each of the
//! .ico's images an RT_ICON, an RT_GROUP_ICON listing them, and an RT_VERSION), which the MSVC linker takes as it is,
//! so no resource compiler (rc.exe, from the Windows SDK) is needed.
//!
//! Explorer shows the description as the file's, Task Manager as the process's name, and a notification from an
//! exe without an app id of its own carries it.

use std::path::{Path, PathBuf};

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const LANGUAGE: u16 = 0x0409;

/// What a build makes: an exe (`Bin`, by its target name) or the crate's DLL.
pub enum Target<'a> {
    Bin(&'a str),
    Dll(&'a str),
}

/// Puts the icon and version information into each of `targets`, with its description. `ico` is the workspace's
/// icon, relative to the crate (`../assets/icon/pms.ico`).
pub fn embed(ico: &str, targets: &[(Target, &str)]) {
    println!("cargo:rerun-if-changed={ico}");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if !windows || !msvc {
        return;
    }
    // The empty resource every .res file begins with, then the icon, which every executable shares.
    let mut res = Vec::new();
    resource(&mut res, 0, 0, 0, &[]);
    match std::fs::read(Path::new(ico)) {
        Ok(ico) => icon(&mut res, &ico),
        Err(_) => println!("cargo:warning=no {ico}: the executables get no icon (run cargo xtask art)"),
    }
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    for (target, description) in targets {
        let (name, file, link) = match target {
            Target::Bin(b) => (*b, format!("{b}.exe"), format!("cargo:rustc-link-arg-bin={b}=")),
            Target::Dll(d) => (*d, format!("{d}.dll"), "cargo:rustc-link-arg-cdylib=".to_string()),
        };
        let mut own = res.clone();
        resource(&mut own, RT_VERSION, 1, 0x0030, &version_info(name, &file, description, &version));
        let path = out.join(format!("{name}.res"));
        std::fs::write(&path, own).expect("write the executable's resource file");
        println!("{link}{}", path.display());
    }
}

/// A resource's header, its type and name numbers, then its data, padded to four bytes.
fn resource(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&[0xff, 0xff]);
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&[0xff, 0xff]);
    out.extend_from_slice(&name.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&LANGUAGE.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// The .ico's images, each an RT_ICON, and the RT_GROUP_ICON listing them (resource number 1: the exe's icon).
fn icon(res: &mut Vec<u8>, ico: &[u8]) {
    let u16_at = |i: usize| u16::from_le_bytes([ico[i], ico[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([ico[i], ico[i + 1], ico[i + 2], ico[i + 3]]);
    let count = u16_at(4) as usize;
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(count as u16).to_le_bytes());
    for i in 0..count {
        let entry = 6 + 16 * i;
        let (size, offset) = (u32_at(entry + 8) as usize, u32_at(entry + 12) as usize);
        let id = i as u16 + 1;
        resource(res, RT_ICON, id, 0x1010, &ico[offset..offset + size]);
        // The group's entry: the .ico's, its offset replaced by the image's resource number.
        group.extend_from_slice(&ico[entry..entry + 12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    resource(res, RT_GROUP_ICON, 1, 0x1030, &group);
}

/// A version resource's node: its length, its value's length (`value_len`: in characters for text, else bytes),
/// whether the value is text, its key, then its value and its children, each on a four-byte boundary.
fn node(key: &str, value: &[u8], value_len: u16, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let pad = |b: &mut Vec<u8>| {
        while !b.len().is_multiple_of(4) {
            b.push(0);
        }
    };
    let mut b = vec![0, 0];
    b.extend_from_slice(&value_len.to_le_bytes());
    b.extend_from_slice(&u16::from(text).to_le_bytes());
    b.extend(key.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
    pad(&mut b);
    b.extend_from_slice(value);
    for c in children {
        pad(&mut b);
        b.extend_from_slice(c);
    }
    let len = b.len() as u16;
    b[..2].copy_from_slice(&len.to_le_bytes());
    b
}

/// A version resource (VS_VERSIONINFO) for `file`, described as `description`, of PimpMyShell `version`.
fn version_info(name: &str, file: &str, description: &str, version: &str) -> Vec<u8> {
    let n: Vec<u16> = version.split('.').map(|p| p.parse().unwrap_or(0)).chain([0; 4]).take(4).collect();
    let (ms, ls) = ((u32::from(n[0]) << 16) | u32::from(n[1]), (u32::from(n[2]) << 16) | u32::from(n[3]));
    // VS_FIXEDFILEINFO: its signature and layout's version, the file's and the product's version, no flags, a
    // Windows (NT, 32-bit API) application (or DLL).
    let kind = if file.ends_with(".dll") { 2 } else { 1 };
    let fixed: Vec<u8> = [0xFEEF_04BD, 0x0001_0000, ms, ls, ms, ls, 0x3F, 0, 0x0004_0004, kind, 0, 0, 0]
        .iter()
        .flat_map(|v: &u32| v.to_le_bytes())
        .collect();
    let text = |key: &str, value: &str| {
        let v: Vec<u8> = value.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        node(key, &v, (v.len() / 2) as u16, true, &[])
    };
    let strings = [
        text("CompanyName", "PimpMyShell"),
        text("FileDescription", description),
        text("FileVersion", version),
        text("InternalName", name),
        text("OriginalFilename", file),
        text("ProductName", "PimpMyShell (P.M.S.)"),
        text("ProductVersion", version),
    ];
    // US English, Unicode.
    let table = node("040904B0", &[], 0, true, &strings);
    let string_info = node("StringFileInfo", &[], 0, true, &[table]);
    let translation = node("Translation", &[0x09, 0x04, 0xB0, 0x04], 4, false, &[]);
    let var_info = node("VarFileInfo", &[], 0, true, &[translation]);
    node("VS_VERSION_INFO", &fixed, fixed.len() as u16, false, &[string_info, var_info])
}
