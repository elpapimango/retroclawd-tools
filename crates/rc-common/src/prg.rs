//! The GEMDOS executable (`.PRG`, `.TOS`, `.TTP`): a 28-byte header, the text and data segments, an optional symbol
//! table, and a fixup table that makes the program relocatable.
//!
//! The format is from the Atari Compendium, chapter 2 "GEMDOS executable files" (the program file header;
//! <http://bus-error.nokturnal.pl/atari_compendium/html/chap2.htm>, SHA-256 of the page
//! `6941f8b1af4f32561703335fc5644944c929febf569c34724cc09a79b62a6fd9`), and nowhere else:
//!
//! | Offset | Contents |
//! |---|---|
//! | `$00` | word, `$601A` |
//! | `$02`, `$06`, `$0A`, `$0E` | longs: sizes of the text, data and bss segments and of the symbol table |
//! | `$12` | long, reserved |
//! | `$16` | long, `PRGFLAGS` |
//! | `$1A` | word, `ABSFLAG`: non-zero when the program has no fixups |
//! | `$1C` | the text segment, then the data segment, then the symbol table |
//! | after them | a long, the offset of the first longword to fix up (0: none); then bytes: 0 ends the list, 1 advances 254 bytes, an even value 2-254 advances that many bytes and fixes up the longword there |
//!
//! A fixup adds the load address of the text segment to the longword. The document says the first offset is "from the
//! beginning" of the executable; [H?] it is read as the offset from the start of the text segment, which is what a
//! loader that relocates the image in place does (the text, data and the fixups of both share one offset space).
//!
//! Every size read from a file is checked against what is there before anything is allocated, and the fixup walk is
//! bounded by the bytes of the table: a hostile file can make [`parse`] fail, never loop or allocate much.

use std::fmt;

pub const MAGIC: u16 = 0x601A;
pub const HEADER_LEN: usize = 28;
/// The largest executable read: a 16 MiB file is far beyond anything an ST can load (it has at most 4 MiB of RAM).
pub const MAX_FILE: usize = 16 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrgError {
    /// Fewer bytes than a header.
    TooShort,
    BadMagic(u16),
    TooLarge(usize),
    /// The sizes in the header add up to more than the file holds.
    SizesBeyondFile {
        needed: u64,
        have: usize,
    },
    /// The symbol table runs past the end of the file.
    SymbolTableBeyondFile,
    /// A fixup (or the first fixup offset) is not inside the text and data segments with room for a longword.
    FixupBeyondImage {
        offset: u64,
    },
    /// A fixup step that is odd, or not a step at all.
    BadFixupStep {
        byte: u8,
    },
    /// The fixup list ends without its terminating zero.
    FixupListUnterminated,
    /// The first fixup offset is not there.
    FixupTableMissing,
}

impl fmt::Display for PrgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrgError::TooShort => write!(f, "shorter than the 28-byte program header"),
            PrgError::BadMagic(m) => write!(
                f,
                "not a GEMDOS executable (starts with ${m:04X}, not $601A)"
            ),
            PrgError::TooLarge(n) => {
                write!(f, "{n} bytes is larger than the {MAX_FILE} byte limit")
            }
            PrgError::SizesBeyondFile { needed, have } => write!(
                f,
                "the header's segment sizes need {needed} bytes, the file has {have}"
            ),
            PrgError::SymbolTableBeyondFile => {
                write!(f, "the symbol table runs past the end of the file")
            }
            PrgError::FixupBeyondImage { offset } => write!(
                f,
                "a fixup at offset {offset} is outside the text and data segments"
            ),
            PrgError::BadFixupStep { byte } => {
                write!(f, "fixup step {byte} is not 1 or an even 2-254")
            }
            PrgError::FixupListUnterminated => write!(f, "the fixup list has no terminating zero"),
            PrgError::FixupTableMissing => write!(f, "the file ends before the fixup table"),
        }
    }
}

impl std::error::Error for PrgError {}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Prg {
    pub text: Vec<u8>,
    pub data: Vec<u8>,
    /// The size of the zeroed bss segment the loader adds.
    pub bss: u32,
    /// The symbol table as the file holds it (its layout is the toolchain's; the format document leaves it).
    pub symbols: Vec<u8>,
    pub flags: u32,
    /// Offsets from the start of the text segment of the longwords a loader adds its load address to, ascending.
    pub fixups: Vec<u32>,
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Reads an executable.
pub fn parse(bytes: &[u8]) -> Result<Prg, PrgError> {
    if bytes.len() > MAX_FILE {
        return Err(PrgError::TooLarge(bytes.len()));
    }
    if bytes.len() < HEADER_LEN {
        return Err(PrgError::TooShort);
    }
    let magic = u16::from_be_bytes([bytes[0], bytes[1]]);
    if magic != MAGIC {
        return Err(PrgError::BadMagic(magic));
    }
    let (tsize, dsize, bsize, ssize) = (
        be32(bytes, 2) as u64,
        be32(bytes, 6) as u64,
        be32(bytes, 10),
        be32(bytes, 14) as u64,
    );
    let flags = be32(bytes, 22);
    let image_end = HEADER_LEN as u64 + tsize + dsize;
    if image_end > bytes.len() as u64 {
        return Err(PrgError::SizesBeyondFile {
            needed: image_end,
            have: bytes.len(),
        });
    }
    let symbols_end = image_end + ssize;
    if symbols_end > bytes.len() as u64 {
        return Err(PrgError::SymbolTableBeyondFile);
    }
    let (image_end, symbols_end) = (image_end as usize, symbols_end as usize);
    let text_end = HEADER_LEN + tsize as usize;
    let image_len = tsize + dsize;
    let mut prg = Prg {
        text: bytes[HEADER_LEN..text_end].to_vec(),
        data: bytes[text_end..image_end].to_vec(),
        bss: bsize,
        symbols: bytes[image_end..symbols_end].to_vec(),
        flags,
        fixups: Vec::new(),
    };
    let table = &bytes[symbols_end..];
    if table.len() < 4 {
        // A file that stops after the symbols has no fixup table at all: the header's ABSFLAG says whether that is
        // all right, and an absolute program is allowed to omit it.
        return if table.is_empty() && u16::from_be_bytes([bytes[0x1A], bytes[0x1B]]) != 0 {
            Ok(prg)
        } else {
            Err(PrgError::FixupTableMissing)
        };
    }
    let first = be32(table, 0) as u64;
    if first == 0 {
        return Ok(prg);
    }
    let in_image = |offset: u64| offset + 4 <= image_len;
    if !in_image(first) {
        return Err(PrgError::FixupBeyondImage { offset: first });
    }
    prg.fixups.push(first as u32);
    let mut at = first;
    let mut terminated = false;
    for &step in &table[4..] {
        match step {
            0 => {
                terminated = true;
                break;
            }
            1 => at += 254,
            s if s % 2 == 0 && s >= 2 => {
                at += s as u64;
                if !in_image(at) {
                    return Err(PrgError::FixupBeyondImage { offset: at });
                }
                prg.fixups.push(at as u32);
            }
            s => return Err(PrgError::BadFixupStep { byte: s }),
        }
        // Steps of 1 move on without a fixup: a run of them past the image is already out of it.
        if at > image_len + 254 {
            return Err(PrgError::FixupBeyondImage { offset: at });
        }
    }
    if !terminated {
        return Err(PrgError::FixupListUnterminated);
    }
    Ok(prg)
}

impl Prg {
    /// The file: header, text, data, symbols, fixups. Fixups must be ascending, 2 or more bytes apart and even
    /// distances from the first (`debug_assert`ed; the assembler makes them so).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            HEADER_LEN + self.text.len() + self.data.len() + self.symbols.len() + 16,
        );
        out.extend(MAGIC.to_be_bytes());
        out.extend((self.text.len() as u32).to_be_bytes());
        out.extend((self.data.len() as u32).to_be_bytes());
        out.extend(self.bss.to_be_bytes());
        out.extend((self.symbols.len() as u32).to_be_bytes());
        out.extend(0u32.to_be_bytes());
        out.extend(self.flags.to_be_bytes());
        // The document: "it is better to represent a program having no fixups with 0 here and placing a 0 longword as
        // the fixup offset".
        out.extend(0u16.to_be_bytes());
        out.extend(&self.text);
        out.extend(&self.data);
        out.extend(&self.symbols);
        match self.fixups.split_first() {
            None => out.extend(0u32.to_be_bytes()),
            Some((&first, rest)) => {
                debug_assert!(first != 0, "a first fixup at offset 0 would read as none");
                out.extend(first.to_be_bytes());
                let mut at = first;
                for &next in rest {
                    debug_assert!(
                        next > at && (next - at) % 2 == 0,
                        "fixups ascend, an even distance apart"
                    );
                    let mut d = next - at;
                    while d > 254 {
                        out.push(1);
                        d -= 254;
                    }
                    out.push(d as u8);
                    at = next;
                }
                out.push(0);
            }
        }
        out
    }

    /// The text and data segments as they are in memory when loaded at `base`: every fixup has `base` added.
    pub fn relocated(&self, base: u32) -> Vec<u8> {
        let mut image: Vec<u8> = self.text.iter().chain(&self.data).copied().collect();
        for &f in &self.fixups {
            let at = f as usize;
            if let Some(long) = image.get_mut(at..at + 4) {
                let v = u32::from_be_bytes([long[0], long[1], long[2], long[3]]).wrapping_add(base);
                long.copy_from_slice(&v.to_be_bytes());
            }
        }
        image
    }
}
