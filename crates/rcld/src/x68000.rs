//! The Sharp X68000 target: one relocatable 68000 program, written as a Human68k `.X` executable.
//!
//! The layout names the sources (`[[input]] asm = [...]`, assembled together with sections `.text`, `.data` and `.bss`)
//! and an output with the extension `.x`. `entry` (a label or an offset) sets the execution address, the offset from the
//! program's start (default 0). Each long that holds an address of the program is recorded in the relocation table the
//! loader walks. A floppy image with a Human68k filesystem is not written yet (the filesystem is added with the
//! machine, where it can be checked).

use crate::Build;
use crate::amiga::assemble_68000;
use crate::layout::{Layout, Value};
use rcasm::m68k::symbols_text;
use std::path::Path;
use x68k::exe::Executable;

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
    if ext != "x" {
        return Err(format!(
            "{}: an X68000 program is written as .x, not .{ext}",
            output.display()
        ));
    }
    let asm = assemble_68000(layout, base, cli_defines, "X68000")?;
    let entry = match &layout.entry {
        None => 0,
        Some(Value::Int(a)) => {
            u32::try_from(*a).map_err(|_| format!("entry {a:#X} is not an offset"))?
        }
        Some(Value::Text(t)) => match asm.symbols.iter().find(|s| s.name == *t) {
            Some(s) => {
                u32::try_from(s.value).map_err(|_| format!("entry \"{t}\" is not an offset"))?
            }
            None => rc_common::parse_number(t)
                .ok()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| {
                    format!("entry \"{t}\" is neither a label of the sources nor an offset")
                })?,
        },
    };
    if entry as usize >= asm.text.len() {
        return Err(format!(
            "entry ${entry:X} is outside the text ({} bytes)",
            asm.text.len()
        ));
    }
    let exe = Executable {
        load_mode: 0,
        base: 0,
        entry,
        text: asm.text.clone(),
        data: asm.data.clone(),
        bss: asm.bss,
        relocs: asm.relocs.clone(),
        tail: Vec::new(),
        symbol_size: 0,
        scd: [0; 3],
    };
    let bytes = exe.to_bytes().map_err(|e| e.to_string())?;
    let map = format!(
        "text {} bytes, data {} bytes, bss {} bytes, {} relocations, entry ${entry:X}\n",
        asm.text.len(),
        asm.data.len(),
        asm.bss,
        asm.relocs.len()
    );
    Ok(Build {
        bytes,
        notes: Vec::new(),
        map,
        symbols_text: symbols_text(&asm, false),
    })
}
