# Game data

`pms/src/wads.rs` fetches what a TV's console can play, only when someone asks in Settings, and keeps it only if its SHA-256 is
exactly what's expected. None of it is in this repository or in the installer.

- **Freedoom** comes from its own release, <https://github.com/freedoom/freedoom/releases>. It's free content under the 3-clause
  BSD licence, which is kept beside it.
- **The Doom shareware episode** comes unmodified from Debian's archive (`doom-wad-shareware`), once its licence has been read.

`doom-shareware-license.txt` here is id Software's licence for that episode, as it came with it, which the settings show first.

Neither Freedoom nor PimpMyShell is affiliated with id Software, Bethesda or Microsoft, and "Doom" is their trademark.
