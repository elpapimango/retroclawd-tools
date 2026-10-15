//! Cartridges: C64 `.crt` and CPC Plus / GX4000 `.cpr`, and the raw ROM images they wrap.

use c64_cartridge::{Chip, ChipKind, Crt};
use cpc_cartridge::{Cartridge, CartridgePage, PAGE_SIZE};

pub fn crt_type_name(id: u16) -> String {
    let name = match id {
        0 => "Normal cartridge",
        1 => "Action Replay",
        2 => "KCS Power Cartridge",
        3 => "Final Cartridge III",
        4 => "Simons' BASIC",
        5 => "Ocean",
        6 => "Expert Cartridge",
        7 => "Fun Play / Power Play",
        8 => "Super Games",
        9 => "Atomic Power",
        10 => "Epyx FastLoad",
        11 => "Westermann Learning",
        12 => "Rex Utility",
        13 => "Final Cartridge I",
        14 => "Magic Formel",
        15 => "C64 Game System / System 3",
        16 => "WarpSpeed",
        17 => "Dinamic",
        18 => "Zaxxon / Super Zaxxon",
        19 => "Magic Desk / Domark / HES Australia",
        32 => "EasyFlash",
        _ => return format!("type {id}"),
    };
    format!("{name} (type {id})")
}

pub fn chip_kind(k: ChipKind) -> &'static str {
    match k {
        ChipKind::Rom => "ROM",
        ChipKind::Ram => "RAM",
        ChipKind::Flash => "flash",
    }
}

/// The cartridge lines as they read in the header: low (false) is active.
pub fn line_name(level: bool) -> &'static str {
    if level {
        "high (inactive)"
    } else {
        "low (active)"
    }
}

/// A plain cartridge from a raw ROM image: 8K at 0x8000 (EXROM active), 16K at 0x8000 (EXROM and GAME active).
/// Other sizes are an error: they would need a bank-switching scheme the file cannot guess.
pub fn crt_from_raw(name: &str, raw: &[u8]) -> Result<Crt, String> {
    let (game_level, load) = match raw.len() {
        0x2000 => (true, 0x8000),
        0x4000 => (false, 0x8000),
        n => {
            return Err(format!(
                "a plain cartridge is 8192 or 16384 bytes, this is {n}; bigger ones need a bank-switching type"
            ));
        }
    };
    Ok(Crt {
        version: 0x0100,
        hardware_type: 0,
        exrom_level: false,
        game_level,
        name: name.to_string(),
        chips: vec![Chip {
            kind: ChipKind::Rom,
            bank: 0,
            load_address: load,
            data: raw.to_vec(),
        }],
    })
}

/// The ROM of a plain cartridge as one image: the chips in bank order, sorted by load address within a bank.
pub fn raw_from_crt(crt: &Crt) -> Result<Vec<u8>, String> {
    if crt.hardware_type != 0 {
        return Err(format!(
            "{} is banked; there is no single raw image (use extract to get its chips)",
            crt_type_name(crt.hardware_type)
        ));
    }
    let mut chips: Vec<&Chip> = crt.chips.iter().collect();
    chips.sort_by_key(|c| (c.bank, c.load_address));
    Ok(chips.iter().flat_map(|c| c.data.iter().copied()).collect())
}

/// A `.cpr` from raw 16K pages (the last one padded with zeros).
pub fn cpr_from_raw(raw: &[u8]) -> Result<(Cartridge, Option<String>), String> {
    if raw.is_empty() {
        return Err("the image is empty".to_string());
    }
    if raw.len() > 32 * PAGE_SIZE {
        return Err(format!("{} bytes do not fit in 32 pages of 16K", raw.len()));
    }
    let pages = raw
        .chunks(PAGE_SIZE)
        .enumerate()
        .map(|(i, c)| CartridgePage {
            bank: i as u8,
            data: c.to_vec(),
        })
        .collect();
    let note = (!raw.len().is_multiple_of(PAGE_SIZE))
        .then(|| format!("the last page was padded with zeros to {PAGE_SIZE} bytes"));
    Ok((Cartridge { pages }, note))
}

/// The pages of a `.cpr` back to back, in page order. Missing page numbers are an error (a gap would shift
/// everything after it).
pub fn raw_from_cpr(cart: &Cartridge) -> Result<Vec<u8>, String> {
    let mut pages: Vec<&CartridgePage> = cart.pages.iter().collect();
    pages.sort_by_key(|p| p.bank);
    let mut out = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        if p.bank as usize != i {
            return Err(format!(
                "page {i} is missing (pages present: {}); a raw image cannot have gaps",
                pages
                    .iter()
                    .map(|p| p.bank.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let mut d = p.data.clone();
        d.resize(PAGE_SIZE, 0);
        out.extend(d);
    }
    Ok(out)
}
