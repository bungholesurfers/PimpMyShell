# Building, installing and releasing PimpMyShell

Everything runs through `cargo xtask` (the `xtask` crate; `.cargo/config.toml` holds the alias), as Nagare does it.

| task | what it does |
| --- | --- |
| `cargo xtask build` | release build of `pmsd.exe`, `pms.exe`, `pms-gui.exe` and `pms_tap.dll` into `target\release`. If one of those exes is running (a daemon started from there, say), it's moved aside to `.old` first and keeps running from there. |
| `cargo xtask check` | clippy with warnings as errors, then every test. |
| `cargo xtask art` | renders the logo (`assets\icon`: `pms.ico` and a PNG per size, which the build puts in every exe) and the installer's box and pictures (`installer\art`) from SVG, with pms-art. The crab is Scout's public-domain "Red Crab" from Openclipart (`art\openclipart-192996`). The results are kept with the source, so a build needs no rendering. |
| `cargo xtask installer` | builds, then packs `target\installer\pms-setup-VERSION.exe` with Inno Setup 6 (`winget install --id JRSoftware.InnoSetup --scope user`). `--no-build` packs what's already built; `--pgo` packs the optimised build (below); `--install` then installs it silently, as an update would. |
| `cargo xtask pgo [SECONDS]` | profile-guided optimisation, as Nagare's releases have it: an instrumented build, a training run of SECONDS (default 60) on this very taskbar (meters, the hot-mic alert, toasts; a running daemon steps aside and comes back), the profile merged into `pgo\pms.profdata`, and the optimised build from it with fat LTO over one codegen unit. The steps also run alone: `pgo-instrument`, `pgo-train`, `pgo-merge`, `pgo-build`. |

VERSION is the workspace's version, in the root `Cargo.toml` (`[workspace.package]`), which every crate takes. Everyday release builds take no LTO, so they rebuild in seconds; only the PGO build takes fat LTO.

## What the installer does

`pms.iss` installs for the current user alone, with no administrator, into `%LOCALAPPDATA%\Programs\PimpMyShell`:

- the three exes, the TAP DLL and `pms.example.toml`;
- a Start menu shortcut, **PimpMyShell**, to the settings (and a desktop one, if ticked);
- an entry in Settings > Apps, under a fixed AppId, so each version installs over the last;
- the task *Start PimpMyShell when I sign in*, ticked by default: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `PimpMyShell`, which is also what the settings' **start at sign-in** writes.

Before copying files it asks a running daemon to stop (which puts the stock taskbar back), then closes any that are left, and the settings window too. A silent install (an update) starts the daemon again if it had been running.

The config, `%LOCALAPPDATA%\PimpMyShell\pms.toml`, belongs to the user: the daemon writes the example there the first time, if there's none, and the uninstaller leaves it. Uninstalling also removes the Run value (whichever of them wrote it) and Task Manager's on/off mark for it.

A development build (run from `target\release`) and an installed copy can live side by side, but only one daemon at a time. If autostart was switched on from a development build, installing with the task ticked points it at the installed copy.

## Updates

The settings' General page has **Check for updates**. From an installed copy it downloads the newer installer, checks it, and offers **Install and restart**, which runs it silently. A development build only says whether a newer release is out.

What a copy updates to:

- **stable**: the tag in `update/stable.txt` on the repository's main branch, e.g. `v0.2.0`;
- each release carries `pms-setup-VERSION.exe`, `pms-update.json` (a manifest of the installer's name, version, size and SHA-256) and `pms-update.json.sig` (an Ed25519 signature of the manifest).

The updater believes a manifest only if one of the keys in `pms/src/signing.rs` (`PUBLIC_KEYS`) signed it and it names that very version, and keeps the installer only if its size and hash match. It talks to GitHub's hosts alone, over HTTPS, every redirect checked, with Mozilla's roots bundled and a cap on every response. The installer is hashed once more just before it runs (it sat in `%TEMP%\pms-update` meanwhile). Before running it, the updater notes the version in `tried.txt`: if that install didn't take, the next check says so rather than installing it again and again. Once PimpMyShell is that version, the daemon clears the folder away. See `pms/src/update.rs`.

### One-time: the release key

```
cargo run --release --bin pms-release -- keygen C:\somewhere\safe\pms-release.key
```

That writes the secret half to the file and prints the public half: put it in `PUBLIC_KEYS`. Until there is one, the updater refuses every manifest. Keep the secret out of the repository: whoever holds it can ship updates every installed copy will take.

### Each release

1. Bump `version` in the root `Cargo.toml`'s `[workspace.package]`.
2. `cargo xtask check`, then `cargo xtask pgo` (or `pgo-build` alone, if the kept profile still fits the code) and `cargo xtask installer --no-build`.
3. Sign it:
   ```
   set PMS_UPDATE_KEY=<the secret, in hex>
   cargo run --release --bin pms-release -- manifest target\installer\pms-setup-VERSION.exe VERSION target\installer
   ```
4. Publish a release tagged `vVERSION` with the three files from `target\installer`.
5. Once it's proved itself, put `vVERSION` in `update/stable.txt`.

### Trying it without publishing

Set `PMS_UPDATE_SOURCE` to a folder holding `stable.txt`, the installer, its manifest and signature, side by side, and start the settings from that environment: the updater reads the folder instead of GitHub, and does everything else the same.
