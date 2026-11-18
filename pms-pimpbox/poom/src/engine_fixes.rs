//! The engine's own tests don't build here (they need the original C), so what we changed in it is tested from POOM's program,
//! which links it with the C functions it needs. Each of these took the game down, with Freedoom or with speed demos, before
//! it was fixed (engine/NOTICE.md, 2026-11-18).

use std::sync::Mutex;

use room::doom::g_game::{G_SetFastMonsters, spawn_fine_angle};
use room::doom::info::states;
use room::doom::statenum::{S_SARG_PAIN2, S_SARG_RUN1};
use room::doom::z_zone::{PU_LEVEL, PU_PURGELEVEL, Z_CheckHeapQuiet, Z_FreeMemory, Z_FreeTags, Z_Init, Z_Malloc};

/// The engine's tables and zone are globals: one test at a time touches them.
static ENGINE: Mutex<()> = Mutex::new(());

fn demon_tics() -> Vec<i32> {
    (S_SARG_RUN1 as usize..=S_SARG_PAIN2 as usize).map(|i| unsafe { (*std::ptr::addr_of!(states))[i].tics }).collect()
}

/// A title loop of speed demos starts a new game for each, some with -fast and some without. However many come in a row, the
/// demons' states are halved once and restored once: never ground down to 0 tics, which looped a demon forever (E1M6, after
/// demo 93960, a -fast one, had played twice).
#[test]
fn fast_monsters_are_a_state_not_a_step() {
    let _one = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let usual = demon_tics();
    assert!(usual.iter().all(|&t| t > 0), "{usual:?}");
    unsafe {
        G_SetFastMonsters(1);
        let fast = demon_tics();
        G_SetFastMonsters(1);
        G_SetFastMonsters(1);
        assert_eq!(demon_tics(), fast, "fast again is the same fast");
        assert!(fast.iter().zip(&usual).all(|(f, u)| *f == u >> 1));
        G_SetFastMonsters(0);
        assert_eq!(demon_tics(), usual, "and back");
        G_SetFastMonsters(0);
        assert_eq!(demon_tics(), usual, "back again is the same");
    }
}

/// Every facing a map spot can have is one `G_CheckSpot` knows: east, north, west and south straight into the fine tables, and
/// the diagonals and the back half as vanilla's overflow read them. (It was "an unexpected angle" for all but east.)
#[test]
fn every_spawn_facing_is_one_g_checkspot_knows() {
    let known = [0, 1024, 2048, 3072, 4096, 5120, 6144, 7168];
    for (step, angle) in (0..360i16).step_by(45).enumerate() {
        assert_eq!(spawn_fine_angle(angle), known[step], "{angle} degrees");
    }
}

/// A level's blocks all go when it ends, however many it has: Freedoom's levels have thousands, and a cap on how many were
/// walked kept the rest, a level at a time, until the zone was full.
#[test]
fn a_level_with_thousands_of_blocks_is_freed_whole() {
    let _one = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        Z_Init();
        let empty = Z_FreeMemory();
        for _ in 0..5000 {
            assert!(!Z_Malloc(64, PU_LEVEL, std::ptr::null_mut()).is_null());
        }
        assert!(Z_FreeMemory() < empty);
        Z_FreeTags(PU_LEVEL, PU_PURGELEVEL - 1);
        assert_eq!(Z_FreeMemory(), empty, "every block of the level is free again");
        assert!(Z_CheckHeapQuiet(), "and the zone's in one piece");
    }
}
