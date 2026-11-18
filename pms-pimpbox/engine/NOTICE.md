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

Changed on 2026-11-18:

- `doom/z_zone.rs`: `Z_FreeTags` stopped after walking 2000 blocks, which the C original doesn't, and printed a line each time. A
  level with more (Freedoom's have) kept the rest when it ended, so every level change leaked until the zone ran out. It now
  walks them all, quietly. `Z_CheckHeapQuiet`'s guard against a looped list was also 2000 blocks, which a big level has: it's
  now as many blocks as the zone could hold.
- `doom/g_game.rs`: `G_CheckSpot` worked out the teleport fog's fine angle as `0x10000000 * (angle / 45)`, unshifted, where
  Chocolate Doom has `(ANG45 >> ANGLETOFINESHIFT) * (angle / 45)`: every spawn spot facing anything but east was "an
  unexpected angle", and the game quit when a player respawned at one. Its error now says which angle. `spawn_fine_angle` is
  public, for POOM's test of it.
- `doom/g_game.rs`: fast monsters (nightmare, `-fast`) are a state, not a step. `G_InitNew`, as vanilla's, halved the demons'
  state tics on every new game with `-fast` and doubled them only when leaving nightmare, so `-fast` demos played one after
  another took them from 2 to 1 to 0, and a demon with 0-tic states looped forever. `G_SetFastMonsters` now remembers whether
  the tables are halved and does nothing when asked for what's already so; `G_InitNew` asks for `fastparm || nightmare`.

Copyright of what this descends from:

- The Doom source code: Copyright (C) 1993-1996 id Software, Inc., released under the GPL.
- Chocolate Doom: Copyright (C) 2005-2014 Simon Howard and contributors.
- doomgeneric: ozkl and contributors.
- room: Markus Mayer and contributors.

Changed on 2026-11-16:

- `doom/d_main.rs`: `-demos <file>...`, up to four demos the title loop plays in place of the game's own (as `DEMO1` to
  `DEMO4`). Like `-playdemo`'s, they don't count as modifying the game, so the shareware version takes them.
- `doom/g_game.rs`: a demo being played ends where its data does, as well as at its end marker, rather than reading on past
  its buffer; one too short for its header isn't played.
- `doom/p_pspr.rs`: `weapon_firings`, a count of every time a weapon's fired, read by `pms-poom --check`.
- `Cargo.toml` and `audio/mod.rs`: a `sound` feature (on by default) for the sound device, so that a build without it has
  nothing of cpal or COM.
- Otherwise nothing in the engine. The game, which plays it (`pms-poom.exe`, from `pms-pimpbox/poom`), is a program of its own, apart from
  the console (`pms-pimpbox.exe`).

The source of `pms-pimpbox.exe` and `pms-poom.exe` as they're built and released is in PimpMyShell's repository, in `pms-pimpbox/`, at the release's tag:
<https://github.com/bungholesurfers/PimpMyShell>. Each release on GitHub also carries it as an archive.

No game data is here. An IWAD comes from elsewhere.
