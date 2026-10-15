//! What a ROM-loadable block holds: the 19-byte header, the data block after it, the XOR checksum, and a lister
//! for tokenised Spectrum BASIC.

use crate::tzx::printable;

/// The XOR of every byte but the last equals the last.
pub fn checksum_ok(data: &[u8]) -> bool {
    match data.split_last() {
        Some((&last, rest)) => rest.iter().fold(0u8, |a, &b| a ^ b) == last,
        None => false,
    }
}

pub fn checksum_of(data_without_checksum: &[u8]) -> u8 {
    data_without_checksum.iter().fold(0, |a, &b| a ^ b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Program,
    NumberArray,
    CharacterArray,
    Code,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    /// The ten name characters with trailing spaces removed.
    pub name: String,
    pub length: u16,
    /// Program: autostart line (>= 32768 means none). Code: start address.
    pub param1: u16,
    /// Program: length of the BASIC part (variables follow). Code: 32768.
    pub param2: u16,
}

/// Decodes a header block's payload (flag 0, 17 bytes, checksum). `None` when it is not one.
pub fn parse_header(data: &[u8]) -> Option<Header> {
    if data.len() != 19 || data[0] != 0 {
        return None;
    }
    let kind = match data[1] {
        0 => Kind::Program,
        1 => Kind::NumberArray,
        2 => Kind::CharacterArray,
        3 => Kind::Code,
        _ => return None,
    };
    let name: String = data[2..12].iter().map(|&b| printable(b)).collect();
    let word = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
    Some(Header {
        kind,
        name: name.trim_end().to_string(),
        length: word(12),
        param1: word(14),
        param2: word(16),
    })
}

/// Builds the 19-byte header payload.
pub fn make_header(kind: Kind, name: &str, length: u16, param1: u16, param2: u16) -> Vec<u8> {
    let mut v = vec![0u8, kind as u8];
    let mut n: Vec<u8> = name.bytes().take(10).collect();
    n.resize(10, b' ');
    v.extend(n);
    v.extend_from_slice(&length.to_le_bytes());
    v.extend_from_slice(&param1.to_le_bytes());
    v.extend_from_slice(&param2.to_le_bytes());
    v.push(checksum_of(&v));
    v
}

impl Header {
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            Kind::Program => "Program",
            Kind::NumberArray => "Number array",
            Kind::CharacterArray => "Character array",
            Kind::Code => "Bytes",
        }
    }

    pub fn describe(&self) -> String {
        match self.kind {
            Kind::Program if self.param1 < 32768 => format!(
                "{}: {} LINE {} ({} bytes)",
                self.kind_name(),
                self.name,
                self.param1,
                self.length
            ),
            Kind::Code => format!(
                "{}: {} start {}, length {}",
                self.kind_name(),
                self.name,
                self.param1,
                self.length
            ),
            _ => format!(
                "{}: {} ({} bytes)",
                self.kind_name(),
                self.name,
                self.length
            ),
        }
    }

    /// A file name for the data that follows this header.
    pub fn file_name(&self) -> String {
        let stem = sanitize(&self.name);
        let stem = if stem.is_empty() {
            "unnamed".to_string()
        } else {
            stem
        };
        let ext = match self.kind {
            Kind::Program => "bas",
            Kind::Code => "bin",
            Kind::NumberArray | Kind::CharacterArray => "arr",
        };
        format!("{stem}.{ext}")
    }
}

/// Keeps a name safe as a file name on every system.
pub fn sanitize(name: &str) -> String {
    let s: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_. ()+".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    s.trim_matches(['.', ' ']).to_string()
}

const TOKENS: [&str; 91] = [
    "RND",
    "INKEY$",
    "PI",
    "FN ",
    "POINT ",
    "SCREEN$ ",
    "ATTR ",
    "AT ",
    "TAB ",
    "VAL$ ",
    "CODE ",
    "VAL ",
    "LEN ",
    "SIN ",
    "COS ",
    "TAN ",
    "ASN ",
    "ACS ",
    "ATN ",
    "LN ",
    "EXP ",
    "INT ",
    "SQR ",
    "SGN ",
    "ABS ",
    "PEEK ",
    "IN ",
    "USR ",
    "STR$ ",
    "CHR$ ",
    "NOT ",
    "BIN ",
    " OR ",
    " AND ",
    "<=",
    ">=",
    "<>",
    " LINE ",
    " THEN ",
    " TO ",
    " STEP ",
    "DEF FN ",
    "CAT ",
    "FORMAT ",
    "MOVE ",
    "ERASE ",
    "OPEN #",
    "CLOSE #",
    "MERGE ",
    "VERIFY ",
    "BEEP ",
    "CIRCLE ",
    "INK ",
    "PAPER ",
    "FLASH ",
    "BRIGHT ",
    "INVERSE ",
    "OVER ",
    "OUT ",
    "LPRINT ",
    "LLIST ",
    "STOP",
    "READ ",
    "DATA ",
    "RESTORE ",
    "NEW",
    "BORDER ",
    "CONTINUE ",
    "DIM ",
    "REM ",
    "FOR ",
    "GO TO ",
    "GO SUB ",
    "INPUT ",
    "LOAD ",
    "LIST ",
    "LET ",
    "PAUSE ",
    "NEXT ",
    "POKE ",
    "PRINT ",
    "PLOT ",
    "RUN ",
    "SAVE ",
    "RANDOMIZE ",
    "IF ",
    "CLS",
    "DRAW ",
    "CLEAR ",
    "RETURN",
    "COPY",
];

/// Lists a tokenised BASIC program (the data of a Program block, without flag and checksum) as text, one line
/// per BASIC line. Stops at the first line that does not fit.
pub fn list_basic(program: &[u8]) -> String {
    let mut out = String::new();
    let mut at = 0;
    while at + 4 <= program.len() {
        let number = u16::from_be_bytes([program[at], program[at + 1]]);
        if number >= 0x4000 {
            break; // the variables area starts where the lines end
        }
        let len = u16::from_le_bytes([program[at + 2], program[at + 3]]) as usize;
        let Some(body) = program.get(at + 4..at + 4 + len) else {
            break;
        };
        out.push_str(&format!("{number} "));
        let mut i = 0;
        while i < body.len() {
            let b = body[i];
            match b {
                0x0D => break,
                0x0E => i += 5, // the hidden floating point form of the number just printed
                0xA5..=0xFF => out.push_str(TOKENS[(b - 0xA5) as usize]),
                0x10..=0x15 => i += 1, // colour control: its argument follows
                0x16 | 0x17 => i += 2,
                0x20..=0x7F => out.push(match b {
                    0x5E => '↑',
                    0x60 => '£',
                    0x7F => '©',
                    _ => b as char,
                }),
                _ => out.push('?'),
            }
            i += 1;
        }
        out.push('\n');
        at += 4 + len;
    }
    out
}
