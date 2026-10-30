//! pms-release: makes the updater's release key, and writes and signs each release's manifest (`signing.rs`).
//!
//!   pms-release keygen SECRET_FILE
//!       Makes a new Ed25519 key: writes its SECRET half, in hex, to SECRET_FILE and prints its PUBLIC half, for
//!       signing.rs's PUBLIC_KEYS. It never writes over a file that exists, nor into a source tree (a folder with a
//!       .git or Cargo.toml above it). Keep the secret somewhere safe: whoever holds it can publish updates every
//!       installed PimpMyShell will take.
//!
//!   pms-release pubkey [SECRET_FILE]
//!       Prints the PUBLIC half of the secret in SECRET_FILE (or, without one, in PMS_UPDATE_KEY), and whether
//!       signing.rs's PUBLIC_KEYS lists it. Writes nothing.
//!
//!   pms-release manifest INSTALLER VERSION OUT_DIR
//!       Writes OUT_DIR\pms-update.json for INSTALLER, which must be named for VERSION, and signs it into
//!       OUT_DIR\pms-update.json.sig with the secret in the environment variable PMS_UPDATE_KEY. It checks
//!       the signature against signing.rs's PUBLIC_KEYS, so a key the app doesn't know fails the release.

use ed25519_dalek::SigningKey;
use pms::{signing, version};

const USAGE: &str = "usage:
  pms-release keygen SECRET_FILE                  make a new release key (never overwrites a file)
  pms-release pubkey [SECRET_FILE]                print a secret key's public half (default: PMS_UPDATE_KEY)
  pms-release manifest INSTALLER VERSION OUT_DIR  write and sign a release's manifest (with PMS_UPDATE_KEY)";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["keygen", file] => keygen(file),
        ["pubkey"] => pubkey(None),
        ["pubkey", file] => pubkey(Some(file)),
        ["manifest", installer, version, out] => manifest(installer, version, out),
        _ => Err(USAGE.into()),
    };
    if let Err(e) = result {
        eprintln!("pms-release: {e}");
        std::process::exit(1);
    }
}

fn keygen(file: &str) -> Result<(), String> {
    use std::io::Write;
    let path = std::path::absolute(file).map_err(|e| format!("{file}: {e}"))?;
    if path.exists() {
        return Err(format!("{} already exists; it may be a key that signs releases, so it is left alone. Name a new file.", path.display()));
    }
    // A secret inside a source tree is one `git add .` away from being published.
    if let Some(tree) = path.ancestors().skip(1).find(|dir| dir.join(".git").exists() || dir.join("Cargo.toml").exists()) {
        return Err(format!("{} is inside a source tree ({}); keep the secret key outside any repository.", path.display(), tree.display()));
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| format!("no randomness: {e}"))?;
    let key = SigningKey::from_bytes(&seed);
    // create_new too, should the file appear between the check above and here.
    let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    writeln!(out, "{}", signing::hex(&key.to_bytes())).map_err(|e| format!("{}: {e}", path.display()))?;
    drop(out);
    // Read-only, a second guard against writing over it by hand.
    if let Ok(meta) = std::fs::metadata(&path) {
        let mut permissions = meta.permissions();
        permissions.set_readonly(true);
        let _ = std::fs::set_permissions(&path, permissions);
    }
    println!("SECRET key written to {} (read-only).", path.display());
    println!("  Keep it private and backed up. It goes in one place only: GitHub's PMS_UPDATE_KEY secret.");
    println!();
    println!("PUBLIC key (this one goes into PUBLIC_KEYS in pms/src/signing.rs):");
    println!("{}", signing::hex(&key.verifying_key().to_bytes()));
    Ok(())
}

fn pubkey(file: Option<&str>) -> Result<(), String> {
    let (key, from) = match file {
        Some(file) => (secret(&std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?, file)?, file.to_string()),
        None => (secret(&std::env::var("PMS_UPDATE_KEY").map_err(|_| "name a SECRET_FILE, or set PMS_UPDATE_KEY")?, "PMS_UPDATE_KEY")?, "PMS_UPDATE_KEY".to_string()),
    };
    let public = signing::hex(&key.verifying_key().to_bytes());
    println!("PUBLIC key of the secret in {from} (this one goes into PUBLIC_KEYS in pms/src/signing.rs):");
    println!("{public}");
    if signing::PUBLIC_KEYS.contains(&public.as_str()) {
        println!("  This build's signing.rs lists it: releases signed with it update this build.");
    } else {
        println!("  This build's signing.rs does NOT list it: releases signed with it would fail.");
    }
    Ok(())
}

/// The secret key in `text` (hex, as keygen writes it); `from` names where it came from, for errors.
fn secret(text: &str, from: &str) -> Result<SigningKey, String> {
    let bytes: [u8; 32] = signing::unhex(text.trim()).and_then(|b| b.try_into().ok()).ok_or(format!("{from} does not hold a secret key (32 bytes of hex)"))?;
    Ok(SigningKey::from_bytes(&bytes))
}

fn manifest(installer: &str, version: &str, out: &str) -> Result<(), String> {
    use ed25519_dalek::Signer;
    use sha2::{Digest, Sha256};
    let version = version.strip_prefix('v').unwrap_or(version);
    if version::parse(version).is_none() {
        return Err(format!("{version:?} is not a version"));
    }
    let path = std::path::Path::new(installer);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if name != signing::installer_name(version) {
        return Err(format!("the installer {name:?} is not named for {version}"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{installer}: {e}"))?;
    let m = signing::Manifest { version: version.into(), installer: name.into(), size: bytes.len() as u64, sha256: signing::hex(&Sha256::digest(&bytes)) };
    let json = signing::manifest_json(&m);
    let key = secret(&std::env::var("PMS_UPDATE_KEY").map_err(|_| "PMS_UPDATE_KEY is not set")?, "PMS_UPDATE_KEY")?;
    let signature = signing::hex(&key.sign(json.as_bytes()).to_bytes());
    signing::verify(json.as_bytes(), &signature, signing::PUBLIC_KEYS)
        .map_err(|e| format!("{e} (is the key's public half, {}, in signing.rs?)", signing::hex(&key.verifying_key().to_bytes())))?;
    let out = std::path::Path::new(out);
    std::fs::write(out.join(signing::MANIFEST), &json).map_err(|e| e.to_string())?;
    std::fs::write(out.join(signing::SIGNATURE), &signature).map_err(|e| e.to_string())?;
    println!("{json}");
    Ok(())
}
