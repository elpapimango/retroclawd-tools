//! The Atari ST target: one relocatable 68000 program, written as a GEMDOS executable.
//!
//! The layout names the sources (`[[input]] asm = [...]`, assembled together as one program with sections `.text`,
//! `.data` and `.bss`) and an output with the extension `.prg`, `.tos` or `.ttp`. There are no regions or reserves
//! (the loader places the program wherever it finds room), no `entry` (a GEMDOS program starts at the first byte of
//! its text), and no raw binaries: a long that holds an address of the program is recorded in the fixup table the
//! loader walks, and a binary cannot say which of its longs those are.

use crate::Build;
use crate::layout::Layout;
use rc_common::prg::Prg;
use rcasm::m68k::symbols_text;
use std::path::Path;

pub fn build(
    layout: &Layout,
    base: &Path,
    output: &Path,
    cli_defines: &[(String, i64)],
) -> Result<Build, String> {
    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(ext.as_str(), "prg" | "tos" | "ttp") {
        return Err(format!(
            "{}: an Atari ST program is written as .prg, .tos or .ttp, not .{ext}",
            output.display()
        ));
    }
    if layout.entry.is_some() {
        return Err(
            "entry: a GEMDOS program starts at the first byte of its text; remove `entry`"
                .to_string(),
        );
    }
    if !layout.region.is_empty() || !layout.reserve.is_empty() {
        return Err("regions and reserves are for machines with a fixed memory layout; the ST loader places the program".to_string());
    }
    let asm = crate::amiga::assemble_68000(layout, base, cli_defines, "Atari ST")?;
    let prg = Prg {
        text: asm.text.clone(),
        data: asm.data.clone(),
        bss: asm.bss,
        symbols: Vec::new(),
        flags: 0,
        fixups: asm.relocs.clone(),
    };
    let bytes = prg.to_bytes();
    let map = format!(
        "text {} bytes at 0\ndata {} bytes at {}\nbss {} bytes at {}\n{} relocations\n",
        asm.text.len(),
        asm.data.len(),
        asm.text.len(),
        asm.bss,
        asm.text.len() + asm.data.len(),
        asm.relocs.len()
    );
    Ok(Build {
        bytes,
        notes: Vec::new(),
        map,
        symbols_text: symbols_text(&asm, false),
    })
}
