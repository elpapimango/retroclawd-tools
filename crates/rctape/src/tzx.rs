//! The TZX block model: a tape is a list of blocks, each an ID byte and a body. Unlike `zx-tape`, which flattens
//! a tape into pulses for playback, this keeps the blocks as they are, so a file written back is byte-for-byte
//! what was read. (Provisional adapter: if the main repositories get a block-level model, this goes away.)
//!
//! Format: <https://worldofspectrum.net/TZXformat.html> (revision 1.20).

use std::fmt;

pub const MAGIC: &[u8; 8] = b"ZXTape!\x1A";

/// One block: its ID and everything after the ID byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub id: u8,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tape {
    pub major: u8,
    pub minor: u8,
    pub blocks: Vec<Block>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    NotATzx,
    Truncated { block: usize, offset: usize },
    UnknownBlock { id: u8, block: usize, offset: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotATzx => write!(f, "not a TZX file (no \"ZXTape!\" signature)"),
            Error::Truncated { block, offset } => write!(
                f,
                "block {block} (at byte {offset}) runs past the end of the file"
            ),
            Error::UnknownBlock { id, block, offset } => write!(
                f,
                "block {block} (at byte {offset}) has the unknown ID 0x{id:02X}, so its length is not known"
            ),
        }
    }
}

impl std::error::Error for Error {}

fn le16(b: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]) as usize)
}
fn le24(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, 0]) as usize)
}
fn le32(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]) as usize)
}

/// The body length of the block with this ID, from the bytes that follow the ID. `Some(None)`: known ID whose
/// length fields are not all there yet (truncated); `None`: unknown ID.
fn body_len(id: u8, b: &[u8]) -> Option<Option<usize>> {
    let l = match id {
        0x10 => le16(b, 2).map(|n| 4 + n),
        0x11 => le24(b, 15).map(|n| 18 + n),
        0x12 => Some(4),
        0x13 => b.first().map(|&n| 1 + 2 * n as usize),
        0x14 => le24(b, 7).map(|n| 10 + n),
        0x15 => le24(b, 5).map(|n| 8 + n),
        0x16 | 0x17 => le32(b, 0).map(|n| 4 + n),
        0x18 | 0x19 => le32(b, 0).map(|n| 4 + n),
        0x20 => Some(2),
        0x21 => b.first().map(|&n| 1 + n as usize),
        0x22 | 0x25 | 0x27 => Some(0),
        0x23 | 0x24 => Some(2),
        0x26 => le16(b, 0).map(|n| 2 + 2 * n),
        0x28 => le16(b, 0).map(|n| 2 + n),
        0x2A => Some(4),
        0x2B => Some(5),
        0x30 => b.first().map(|&n| 1 + n as usize),
        0x31 => b.get(1).map(|&n| 2 + n as usize),
        0x32 => le16(b, 0).map(|n| 2 + n),
        0x33 => b.first().map(|&n| 1 + 3 * n as usize),
        0x34 => Some(8),
        0x35 => le32(b, 16).map(|n| 20 + n),
        0x40 => le24(b, 1).map(|n| 4 + n),
        0x5A => Some(9),
        _ => return None,
    };
    Some(l)
}

impl Tape {
    pub fn new() -> Tape {
        Tape {
            major: 1,
            minor: 20,
            blocks: Vec::new(),
        }
    }

    pub fn parse(bytes: &[u8]) -> Result<Tape, Error> {
        if bytes.len() < 10 || &bytes[..8] != MAGIC {
            return Err(Error::NotATzx);
        }
        let mut tape = Tape {
            major: bytes[8],
            minor: bytes[9],
            blocks: Vec::new(),
        };
        let mut at = 10;
        while at < bytes.len() {
            let id = bytes[at];
            let rest = &bytes[at + 1..];
            let n = match body_len(id, rest) {
                None => {
                    return Err(Error::UnknownBlock {
                        id,
                        block: tape.blocks.len() + 1,
                        offset: at,
                    });
                }
                Some(None) => None,
                Some(Some(n)) => Some(n),
            };
            let Some(n) = n.filter(|&n| n <= rest.len()) else {
                return Err(Error::Truncated {
                    block: tape.blocks.len() + 1,
                    offset: at,
                });
            };
            tape.blocks.push(Block {
                id,
                body: rest[..n].to_vec(),
            });
            at += 1 + n;
        }
        Ok(tape)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.push(self.major);
        out.push(self.minor);
        for b in &self.blocks {
            out.push(b.id);
            out.extend_from_slice(&b.body);
        }
        out
    }
}

impl Default for Tape {
    fn default() -> Self {
        Tape::new()
    }
}

impl Block {
    /// A standard-speed data block (ID 0x10).
    pub fn standard(data: &[u8], pause_ms: u16) -> Block {
        let mut body = pause_ms.to_le_bytes().to_vec();
        body.extend_from_slice(&(data.len() as u16).to_le_bytes());
        body.extend_from_slice(data);
        Block { id: 0x10, body }
    }

    pub fn pause(ms: u16) -> Block {
        Block {
            id: 0x20,
            body: ms.to_le_bytes().to_vec(),
        }
    }

    /// A text description (ID 0x30), at most 255 bytes of it.
    pub fn text(text: &str) -> Block {
        let bytes = &text.as_bytes()[..text.len().min(255)];
        let mut body = vec![bytes.len() as u8];
        body.extend_from_slice(bytes);
        Block { id: 0x30, body }
    }

    pub fn kind(&self) -> &'static str {
        match self.id {
            0x10 => "Standard speed data",
            0x11 => "Turbo speed data",
            0x12 => "Pure tone",
            0x13 => "Pulse sequence",
            0x14 => "Pure data",
            0x15 => "Direct recording",
            0x16 => "C64 ROM data",
            0x17 => "C64 turbo data",
            0x18 => "CSW recording",
            0x19 => "Generalized data",
            0x20 => "Pause",
            0x21 => "Group start",
            0x22 => "Group end",
            0x23 => "Jump",
            0x24 => "Loop start",
            0x25 => "Loop end",
            0x26 => "Call sequence",
            0x27 => "Return",
            0x28 => "Select",
            0x2A => "Stop if 48K",
            0x2B => "Set signal level",
            0x30 => "Text description",
            0x31 => "Message",
            0x32 => "Archive info",
            0x33 => "Hardware type",
            0x34 => "Emulation info",
            0x35 => "Custom info",
            0x40 => "Snapshot",
            0x5A => "Glue",
            _ => "Unknown",
        }
    }

    /// The bytes a loader receives from a data block (0x10, 0x11, 0x14): flag, data and checksum for a ROM block.
    pub fn payload(&self) -> Option<&[u8]> {
        match self.id {
            0x10 => self.body.get(4..),
            0x11 => self.body.get(18..),
            0x14 => self.body.get(10..),
            _ => None,
        }
    }

    /// Replaces the payload of a data block, fixing its length field. False for any other block.
    pub fn set_payload(&mut self, data: &[u8]) -> bool {
        let (head, len_at, width) = match self.id {
            0x10 => (4, 2, 2),
            0x11 => (18, 15, 3),
            0x14 => (10, 7, 3),
            _ => return false,
        };
        self.body.truncate(head);
        self.body.extend_from_slice(data);
        let n = (data.len() as u32).to_le_bytes();
        self.body[len_at..len_at + width].copy_from_slice(&n[..width]);
        true
    }

    /// The pause after the block, in milliseconds, for the blocks that carry one.
    pub fn pause_ms(&self) -> Option<u16> {
        let at = match self.id {
            0x10 | 0x20 => 0,
            0x11 => 13,
            0x14 => 5,
            _ => return None,
        };
        Some(le16(&self.body, at)? as u16)
    }

    pub fn set_pause_ms(&mut self, ms: u16) -> bool {
        let at = match self.id {
            0x10 | 0x20 => 0,
            0x11 => 13,
            0x14 => 5,
            _ => return false,
        };
        self.body[at..at + 2].copy_from_slice(&ms.to_le_bytes());
        true
    }

    /// Text, message, archive info, hardware, custom info and glue: nothing the tape plays.
    pub fn is_info(&self) -> bool {
        matches!(self.id, 0x30..=0x35 | 0x5A)
    }

    pub fn is_data(&self) -> bool {
        matches!(self.id, 0x10 | 0x11 | 0x14)
    }

    /// The text of a text description or message block.
    pub fn text_of(&self) -> Option<String> {
        let (len_at, from) = match self.id {
            0x30 => (0, 1),
            0x31 => (1, 2),
            _ => return None,
        };
        let n = *self.body.get(len_at)? as usize;
        let bytes = self.body.get(from..from + n)?;
        Some(bytes.iter().map(|&b| printable(b)).collect())
    }

    /// Approximate playing time in T-states (3.5 MHz), `None` for blocks whose time is not known from the block
    /// alone (loops, jumps, recordings and the rest).
    pub fn cycles(&self) -> Option<u64> {
        let ms = |ms: usize| ms as u64 * 3500;
        match self.id {
            0x10 => {
                let data = self.payload()?;
                let pilot = if data.first().is_some_and(|&f| f < 0x80) {
                    8063
                } else {
                    3223
                };
                Some(
                    pulses_cycles(2168, pilot, 667, 735, 855, 1710, data, 8)
                        + ms(self.pause_ms()? as usize),
                )
            }
            0x11 => {
                let b = &self.body;
                let data = self.payload()?;
                let used = *b.get(12)? as usize;
                Some(
                    pulses_cycles(
                        le16(b, 0)? as u64,
                        le16(b, 10)? as u64,
                        le16(b, 2)? as u64,
                        le16(b, 4)? as u64,
                        le16(b, 6)? as u64,
                        le16(b, 8)? as u64,
                        data,
                        used.clamp(1, 8),
                    ) + ms(self.pause_ms()? as usize),
                )
            }
            0x12 => Some(le16(&self.body, 0)? as u64 * le16(&self.body, 2)? as u64),
            0x13 => {
                let n = *self.body.first()? as usize;
                let mut total = 0;
                for i in 0..n {
                    total += le16(&self.body, 1 + 2 * i)? as u64;
                }
                Some(total)
            }
            0x14 => {
                let b = &self.body;
                let data = self.payload()?;
                let used = *b.get(4)? as usize;
                Some(
                    pulses_cycles(
                        0,
                        0,
                        0,
                        0,
                        le16(b, 0)? as u64,
                        le16(b, 2)? as u64,
                        data,
                        used.clamp(1, 8),
                    ) + ms(self.pause_ms()? as usize),
                )
            }
            0x20 => Some(ms(self.pause_ms()? as usize)),
            _ => None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn pulses_cycles(
    pilot: u64,
    pilot_n: u64,
    sync1: u64,
    sync2: u64,
    zero: u64,
    one: u64,
    data: &[u8],
    last_bits: usize,
) -> u64 {
    let mut t = pilot * pilot_n + sync1 + sync2;
    for (i, &byte) in data.iter().enumerate() {
        let bits = if i + 1 == data.len() { last_bits } else { 8 };
        for k in 0..bits {
            let one_bit = byte & (0x80 >> k) != 0;
            t += 2 * if one_bit { one } else { zero };
        }
    }
    t
}

/// ASCII as is, the Spectrum's block graphics and tokens as `?`.
pub fn printable(b: u8) -> char {
    if (0x20..0x7F).contains(&b) {
        b as char
    } else {
        '?'
    }
}
