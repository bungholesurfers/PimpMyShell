## Download

### ⬇️ [Windows installer (x64)](https://github.com/{{REPO}}/releases/download/{{TAG}}/pms-setup-{{VERSION}}.exe)

The easy way. Installs PimpMyShell for your user alone (no administrator needed), starts it when you sign in, and uninstalls cleanly from Settings > Apps.

### 📦 [Portable zip (x64)](https://github.com/{{REPO}}/releases/download/{{TAG}}/pms-portable-{{VERSION}}.zip)

`pmsd.exe` (run it, and the taskbar's dressed), `pms-gui.exe` (the settings), `pms.exe` (the command line), `pms_tap.dll`, `pms-poom.exe` (the console, under the GPL: `pms-poom-COPYING.txt` and `pms-poom-NOTICE.md`) and `pms-twatch.exe` (Twatch TV's player), to unzip anywhere. No Start menu entry and no self-updating (it tells you when a new version is out); start at sign-in is in its tray menu.

---

- **Exclusively for Windows 11** (on a 64-bit Intel or AMD PC).
- **{{SIGNED}}**
- Built with fat LTO, and profile-guided optimisation when the release had a profile. The SHA-256 checksums of every file are in `SHA256SUMS.txt` below.
- PimpMyShell uses the network to check GitHub for updates (every day or two if you agree when it asks, or whenever you ask), to fetch a game when you ask for one (Freedoom from GitHub, the Doom shareware episode from Debian), and for Twatch TV, which looks at Twitch for your favorites and plays them (through Streamlink, from GitHub, once you get it). It sends nothing but the requests themselves, and installs nothing without asking you. Any of it can go through a SOCKS5 proxy (Settings > Network says what goes where).
- POOM (`pms-poom.exe`) is under the GNU GPL version 2. Its source, as built for this release, is in this tag's `pms-poom/` folder and in the source archives below.