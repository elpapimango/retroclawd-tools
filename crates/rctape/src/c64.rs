//! Commodore tapes: `.t64` (a directory of programs, which is what rctape can list and extract) and the raw
//! `.tap` pulse recording (which it can only describe: decoding the bits needs the KERNAL's or a turbo loader's
//! timing, and is not done).

use c64_tape::{T64, T64Entry, Tape};

pub const PAL_CLOCK_HZ: f64 = 985_248.0;
pub const NTSC_CLOCK_HZ: f64 = 1_022_730.0;

/// A name from the file's PETSCII: unshifted letters stay upper case, shifted ones become lower case, anything
/// unprintable `?` (the same convention `rcdisk` uses), trailing spaces and shifted spaces removed.
pub fn petscii_name(raw: &[u8]) -> String {
    let s: String = raw
        .iter()
        .map(|&b| match b {
            0x20..=0x40 | 0x5B..=0x5D => b as char,
            0x41..=0x5A => b as char,
            0xC1..=0xDA => (b - 0xC1 + b'a') as char,
            0xA0 => ' ',
            _ => '?',
        })
        .collect();
    s.trim_end().to_string()
}

pub struct T64Row {
    pub index: usize,
    pub name: String,
    pub kind: &'static str,
    pub start: u16,
    pub end: u16,
    pub size: usize,
}

fn entry_kind(e: &T64Entry) -> &'static str {
    match e.entry_type {
        1 => match e.c1541_type & 7 {
            1 => "SEQ",
            2 => "PRG",
            3 => "USR",
            4 => "REL",
            _ => "PRG",
        },
        2 => "FRZ",
        _ => "???",
    }
}

pub fn t64_rows(t: &T64) -> Vec<T64Row> {
    t.entries()
        .iter()
        .enumerate()
        .map(|(i, e)| T64Row {
            index: i + 1,
            name: petscii_name(&e.name),
            kind: entry_kind(e),
            start: e.start_addr,
            end: e.end_addr,
            size: e.data.len(),
        })
        .collect()
}

/// The entry as a `.prg` file: its load address, then its data. `None` for entries without program data.
pub fn t64_prg(e: &T64Entry) -> Option<Vec<u8>> {
    if e.entry_type != 1 || e.data.is_empty() {
        return None;
    }
    let mut v = e.start_addr.to_le_bytes().to_vec();
    v.extend_from_slice(&e.data);
    Some(v)
}

/// A file name for an entry: the name made safe, `.prg`.
pub fn t64_file_name(e: &T64Entry) -> String {
    let stem = crate::spectrum::sanitize(&petscii_name(&e.name));
    format!("{}.prg", if stem.is_empty() { "unnamed" } else { &stem })
}

pub struct TapInfo {
    pub version: u8,
    pub platform: u8,
    pub video: &'static str,
    pub pulses: usize,
    pub seconds: f64,
    pub shortest: u32,
    pub longest: u32,
}

/// Facts about a raw `.tap`; `bytes` is the file (the header carries the version and video standard).
pub fn tap_info(tape: &Tape, bytes: &[u8]) -> TapInfo {
    let pulses = tape.pulses();
    let video_byte = bytes.get(14).copied().unwrap_or(0);
    let clock = if video_byte == 1 {
        NTSC_CLOCK_HZ
    } else {
        PAL_CLOCK_HZ
    };
    let total: u64 = pulses.iter().map(|&p| p as u64).sum();
    TapInfo {
        version: bytes.get(12).copied().unwrap_or(0),
        platform: bytes.get(13).copied().unwrap_or(0),
        video: match video_byte {
            0 => "PAL",
            1 => "NTSC",
            2 => "PAL (C64 variant)",
            3 => "NTSC (C64 variant)",
            _ => "unknown",
        },
        pulses: pulses.len(),
        seconds: total as f64 / clock,
        shortest: pulses.iter().copied().min().unwrap_or(0),
        longest: pulses.iter().copied().max().unwrap_or(0),
    }
}
