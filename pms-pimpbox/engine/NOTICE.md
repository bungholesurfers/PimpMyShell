# The engine

This is `room`, a Rust port of the Doom engine by Markus Mayer and contributors, under the GPL version 2 (see `../COPYING`):
<https://github.com/sunsided/room>, revision `526ed91ac356dc3c72f576cdb2f96093dc42a21f`. It descends from doomgeneric, Chocolate
Doom, and the Doom source code id Software released under the GPL.

Changed from that revision, on 2026-11-06:

- The library alone is kept: `room/src`, without `main.rs`, `gpu.rs` and `platform/` (its window, which `pms-pimpbox` replaces)
  or the regression tests in `doom/c_tests`, which need the original C.
- `Cargo.toml` is new: it leaves out `winit`, `wgpu` and the other dependencies of that window.
- `doom/i_system.rs` declares `strcasecmp` itself, for Windows, which `libc` doesn't have it on.
- `doom/i_sound.rs` honours `-nosound`: without this, the engine always opened the audio device.

Changed on 2026-11-07:

- `doom/tables.rs` and `doom/r_segs.rs`: a wall's texture column read `finetangent` past its end (an index of 4096 or more),
  which panicked. It now reads on into `finesine`, as the original's memory layout made it do (`finetangent_overflowing`).

Changed on 2026-11-09:

- `doom/m_controls.rs`, `doom/m_config.rs` and `doom/g_game.rs`: a second key for moving, strafing and using (`key_up2`, `key_down2`,
  `key_strafeleft2`, `key_straferight2`, `key_use2`), W, S, A, D and E by default, saved with the extended settings. The
  arrows, comma, period and space still work as they always did.

Copyright of what this descends from:

- The Doom source code: Copyright (C) 1993-1996 id Software, Inc., released under the GPL.
- Chocolate Doom: Copyright (C) 2005-2014 Simon Howard and contributors.
- doomgeneric: ozkl and contributors.
- room: Markus Mayer and contributors.

The source of `pms-pimpbox.exe` as it's built and released is in PimpMyShell's repository, in `pms-pimpbox/`, at the release's tag:
<https://github.com/bungholesurfers/PimpMyShell>. Each release on GitHub also carries it as an archive.

No game data is here. An IWAD comes from elsewhere.
