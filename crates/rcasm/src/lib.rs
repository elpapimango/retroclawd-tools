//! `rcasm`: a two-pass assembler for the 6502 and the Z80 with one shared syntax.
//!
//! See `docs/rcasm.md` in the repository for the language; [`assemble`] is the entry point, the rest
//! turns an [`Assembly`] into files: a flat binary or `.prg`, a symbol file the debugger and `rcdis`
//! can read, and a listing.

mod asm;
pub mod cpu;
pub mod expr;
pub mod text;

pub use asm::{Assembly, DiskFiles, Files, ListEntry, Options, Segment, Symbol, assemble};
pub use cpu::Cpu;

use std::fmt::Write;

/// All segments as one image from the lowest to the highest address, gaps filled with `fill`.
pub fn flat_image(asm: &Assembly, fill: u8) -> Result<(u16, Vec<u8>), String> {
    let (Some(first), Some(last)) = (asm.segments.first(), asm.segments.last()) else {
        return Err("the program produced no bytes".to_string());
    };
    let start = first.start;
    let end = last.end();
    let mut image = vec![fill; (end - start as u32) as usize];
    for seg in &asm.segments {
        let at = (seg.start - start) as usize;
        image[at..at + seg.bytes.len()].copy_from_slice(&seg.bytes);
    }
    Ok((start, image))
}

/// A C64-style `.prg`: the load address (little endian) followed by the flat image.
pub fn prg_image(asm: &Assembly, fill: u8) -> Result<Vec<u8>, String> {
    let (start, image) = flat_image(asm, fill)?;
    let mut out = start.to_le_bytes().to_vec();
    out.extend(image);
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolStyle {
    /// `NAME = $1234`, read by `retroclawd --symbols`, `rcdis --symbols` and `rcasm` itself.
    Rc,
    /// `al C:1234 .NAME`, VICE monitor labels.
    Vice,
}

fn symbol_name(name: &str) -> String {
    name.chars()
        .map(|c| if c == '@' || c == '#' { '_' } else { c })
        .collect()
}

/// The symbol file. Constants (`=`, `equ`, `-D`) are left out unless `all`.
pub fn symbols_text(asm: &Assembly, all: bool, style: SymbolStyle) -> String {
    let mut out = String::new();
    for s in &asm.symbols {
        if !(all || s.label) || !(0..=0xFFFF).contains(&s.value) {
            continue;
        }
        let name = symbol_name(&s.name);
        match style {
            SymbolStyle::Rc => writeln!(out, "{name} = ${:04X}", s.value),
            SymbolStyle::Vice => writeln!(out, "al C:{:04x} .{name}", s.value),
        }
        .expect("writing to a String");
    }
    out
}

/// `ADDR  BYTES  source`, four bytes per row, continuation rows for longer statements.
pub fn listing_text(asm: &Assembly) -> String {
    let mut out = String::new();
    for entry in &asm.listing {
        let addr = entry
            .addr
            .filter(|_| !entry.bytes.is_empty())
            .map_or("    ".to_string(), |a| format!("{a:04X}"));
        let mut rows = entry.bytes.chunks(4);
        let first: String = rows
            .next()
            .map(|c| c.iter().map(|b| format!("{b:02X} ")).collect())
            .unwrap_or_default();
        writeln!(out, "{addr}  {first:<12} {}", entry.source).expect("writing to a String");
        let mut at = entry.addr.unwrap_or(0) as u32 + 4;
        for chunk in rows {
            let bytes: String = chunk.iter().map(|b| format!("{b:02X} ")).collect();
            writeln!(out, "{:04X}  {bytes:<12}", at & 0xFFFF).expect("writing to a String");
            at += 4;
        }
    }
    out
}
