# Building, installing and releasing PimpMyShell

Everything runs through `cargo xtask` (the `xtask` crate; `.cargo/config.toml` holds the alias), as Nagare does it.

| task | what it does |
| --- | --- |
| `cargo xtask build` | release build of `pmsd.exe`, `pms.exe`, `pms-gui.exe` and `pms_tap.dll` into `target\release`. If one of those exes is running (a daemon started from there, say), it's moved aside to `.old` first and keeps running from there. |
| `cargo xtask check` | clippy with warnings as errors, then every test. |
| `cargo xtask art` | renders the logo (`assets\icon`: `pms.ico` and a PNG per size, which the build puts in every exe) and the installer's box and pictures (`installer\art`) from SVG, with pms-art. The crab is Scout's public-domain "Red Crab" from Openclipart (`art\openclipart-192996`). The results are kept with the source, so a build needs no rendering. |
| `cargo xtask installer` | builds, then packs `target\installer\pms-setup-VERSION.exe` with Inno Setup 6 (`winget install --id JRSoftware.InnoSetup --scope user`). `--no-build` packs what's already built; `--pgo` packs the optimised build (below); `--install` then installs it silently, as an update would. |
| `cargo xtask pgo [SECONDS]` | profile-guided optimisation, as Nagare's releases have it: an instrumented build, a training run of SECONDS (default 60) on this very taskbar (meters, the hot-mic alert, toasts; a running daemon steps aside and comes back), the profile merged into `pgo\pms.profdata`, and the optimised build from it with fat LTO over one codegen unit. The steps also run alone: `pgo-instrument`, `pgo-train`, `pgo-merge`, `pgo-build`. |

VERSION is the workspace's version, in the root `Cargo.toml` (`[workspace.package]`), which every crate takes. Everyday release builds take no LTO, so they rebuild in seconds; only the PGO build (and the release workflow's build, when there's no profile) takes fat LTO.

## What the installer does

`pms.iss` installs for the current user alone, with no administrator, into `%LOCALAPPDATA%\Programs\PimpMyShell`:

- the three exes, the TAP DLL, `pms.example.toml` and `LICENSE.txt`;
- a Start menu shortcut, **PimpMyShell**, to the settings (and a desktop one, if ticked);
- an entry in Settings > Apps, under a fixed AppId, so each version installs over the last;
- the task *Start PimpMyShell when I sign in*, ticked by default: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `PimpMyShell`, which is also what the settings' **start at sign-in** writes. A silent update leaves it as it is, and the wizard shows how it is now.

The wizard is dark whatever Windows is set to. Behind every page is the settings' tile of bills, with the box on it, which `cargo xtask art` draws into `installer/art`, from vectors, once for each display scale at the exact size Inno Setup's help gives (any other size gets stretched on the way to the screen); beside the buttons is the notice it needs, from `pms-gui/src/legal.txt`, which the settings show too.

Before copying files it asks a running daemon to stop (which puts the stock taskbar back), then closes any that are left, and the settings window too. A silent install (an update) starts the daemon again if it had been running, and the settings if they were open. Settings that close themselves to install an update pass `/SETTINGS=1`, because the installer can't see them by then.

The config, `%APPDATA%\PimpMyShell\pms.toml` (only what the user changed, on top of the shipped defaults; roaming; before 0.1.3 it was in `%LOCALAPPDATA%\PimpMyShell`, and one there moves over the first time), belongs to the user: the daemon writes the example there the first time, if there's none, and the uninstaller leaves it. The logs and the machine's own notes stay in `%LOCALAPPDATA%\PimpMyShell`. Uninstalling also removes the Run value (whichever of them wrote it) and Task Manager's on/off mark for it.

A development build (run from `target\release`) and an installed copy can live side by side, but only one daemon at a time. If autostart was switched on from a development build, installing with the task ticked points it at the installed copy.

## Updates

The settings' General page has **Check for updates**. From an installed copy it downloads the newer installer, checks it, and offers **Install X and restart**, which runs it silently. A portable or development copy only says whether a newer release is out.

What a copy updates to (`update_channel` in `pms.toml`):

- **stable** (the default): the tag in `update/stable.txt` on the repository's main branch, e.g. `v0.2.0`;
- **latest**: the newest published release, its tag read from where GitHub's `/releases/latest` redirects.

Each release carries `pms-setup-VERSION.exe`, `pms-update.json` (a manifest of the installer's name, version, size and SHA-256) and `pms-update.json.sig` (an Ed25519 signature of the manifest).

The updater believes a manifest only if one of the keys in `pms/src/signing.rs` (`PUBLIC_KEYS`) signed it and it names that very version, and keeps the installer only if its size and hash match. It talks to GitHub's hosts alone, over HTTPS, every redirect checked, with Mozilla's roots bundled and a cap on every response. The installer is hashed once more just before it runs (it sat in `%TEMP%\pms-update` meanwhile). Before running it, the updater notes the version in `tried.txt`: if that install didn't take, the next check by itself says so (once a run) rather than installing it again and again; a check you ask for downloads it again. Once PimpMyShell is that version, the daemon clears the folder away. See `pms/src/update.rs`.

### One-time: the release key

```
cargo run --release --bin pms-release -- keygen C:\somewhere\safe\pms-release.key
```

That writes the secret half to the file and prints the public half: put it in `PUBLIC_KEYS`. Until there is one, the updater refuses every manifest. Keep the secret out of the repository: whoever holds it can ship updates every installed copy will take.

### Each release

1. Bump `version` in the root `Cargo.toml`'s `[workspace.package]`.
2. `cargo xtask check`; if the code changed much since the kept profile, `cargo xtask pgo` and commit `pgo\pms.profdata`.
3. Push the tag `vVERSION`. The release workflow (`.github/workflows/release.yml`) builds with the kept profile (or
   fat LTO without one), packs the installer and a portable zip, signs the manifest with the repository secret
   `PMS_UPDATE_KEY` (the secret, in hex), adds `SHA256SUMS.txt`, and drafts the release.
4. Publish the draft.
5. Once it's proved itself, put `vVERSION` in `update/stable.txt` (see `update/README.md`).

A release can also be made by hand from a local build: `cargo xtask installer --pgo`, then

```
set PMS_UPDATE_KEY=<the secret, in hex>
cargo run --release --bin pms-release -- manifest target\installer\pms-setup-VERSION.exe VERSION target\installer
```

and a release tagged `vVERSION` with the three files from `target\installer` (pushing that tag also starts the
workflow, which would then try to draft the same release).

### Trying it without publishing

Set `PMS_UPDATE_SOURCE` to a folder holding `stable.txt` (and `latest.txt`, for the latest channel), the installer, its manifest and signature, side by side, and start the settings or the daemon from that environment: the updater reads the folder instead of GitHub, and does everything else the same (so only an installed copy downloads).
