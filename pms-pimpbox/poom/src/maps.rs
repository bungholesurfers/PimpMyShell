//! Which maps a WAD has, by its own directory, as the speed demo archive names them (E1M1, MAP07): for the daemon to tell what
//! game a WAD of any name might be (`pms-poom --maps`). Only the directory's read, every number in it checked against the file;
//! nothing's played.

use std::io::{Read, Seek, SeekFrom};

/// The most lumps a directory may list: Freedoom, the biggest there is, has a few thousand.
const MOST_LUMPS: u32 = 65_536;

/// The maps in the WAD `f`, in the directory's order, each once; None if it isn't a WAD, or its directory doesn't fit in it.
pub fn in_wad(mut f: impl Read + Seek) -> Option<Vec<String>> {
    let size = f.seek(SeekFrom::End(0)).ok()?;
    f.seek(SeekFrom::Start(0)).ok()?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).ok()?;
    if &head[..4] != b"IWAD" && &head[..4] != b"PWAD" {
        return None;
    }
    let count = u32::from_le_bytes(head[4..8].try_into().ok()?);
    let at = u32::from_le_bytes(head[8..12].try_into().ok()?);
    if count > MOST_LUMPS || u64::from(at) + u64::from(count) * 16 > size {
        return None;
    }
    f.seek(SeekFrom::Start(u64::from(at))).ok()?;
    let mut dir = vec![0u8; count as usize * 16];
    f.read_exact(&mut dir).ok()?;
    let mut maps: Vec<String> = Vec::new();
    for entry in dir.as_chunks::<16>().0 {
        let name: String = entry[8..16].iter().take_while(|b| **b != 0).map(|b| char::from(b.to_ascii_uppercase())).collect();
        if is_map(&name) && !maps.contains(&name) {
            maps.push(name);
        }
    }
    Some(maps)
}

/// Whether a lump's name is a map's: ExMy (episodes 1 to 9, maps 1 to 9) or MAPxx (01 to 99).
pub fn is_map(name: &str) -> bool {
    let b = name.as_bytes();
    let episode = b.len() == 4 && b[0] == b'E' && b[2] == b'M' && (b'1'..=b'9').contains(&b[1]) && (b'1'..=b'9').contains(&b[3]);
    let numbered = b.len() == 5 && name.starts_with("MAP") && b[3..].iter().all(u8::is_ascii_digit) && &name[3..] != "00";
    episode || numbered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wad(magic: &[u8; 4], names: &[&str]) -> Vec<u8> {
        let mut w = magic.to_vec();
        w.extend((names.len() as u32).to_le_bytes());
        w.extend(12u32.to_le_bytes());
        for n in names {
            w.extend([0u8; 8]);
            let mut name = [0u8; 8];
            name[..n.len()].copy_from_slice(n.as_bytes());
            w.extend(name);
        }
        w
    }

    #[test]
    fn the_maps_are_the_directorys_own() {
        let maps = in_wad(std::io::Cursor::new(wad(b"IWAD", &["PLAYPAL", "E1M1", "THINGS", "e1m2", "E1M1", "MAP07", "MAP00", "E1M10", "EXMY"]))).unwrap();
        assert_eq!(maps, ["E1M1", "E1M2", "MAP07"]);
    }

    #[test]
    fn a_directory_that_doesnt_fit_is_no_wad() {
        let mut w = wad(b"IWAD", &["E1M1"]);
        w[4..8].copy_from_slice(&2u32.to_le_bytes());
        assert!(in_wad(std::io::Cursor::new(&w)).is_none(), "more lumps than the file has room for");
        assert!(in_wad(std::io::Cursor::new(wad(b"ZWAD", &["E1M1"]))).is_none());
        assert!(in_wad(std::io::Cursor::new(b"IWAD".to_vec())).is_none());
        let mut huge = wad(b"IWAD", &[]);
        huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(in_wad(std::io::Cursor::new(huge)).is_none());
    }
}
