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
use rcasm::m68k::{M68kOptions, assemble_m68k, symbols_text};
use std::path::{Path, PathBuf};

fn read_source(base: &Path, p: &str) -> Result<(String, String), String> {
    let path = if Path::new(p).is_absolute() {
        PathBuf::from(p)
    } else {
        base.join(p)
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok((
        path.display().to_string(),
        String::from_utf8_lossy(&bytes).into_owned(),
    ))
}

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
    let mut sources = Vec::new();
    for (n, input) in layout.input.iter().enumerate() {
        match (&input.asm, &input.bin, &input.prg, &input.symbols) {
            (Some(asm), None, None, None) => sources.extend(asm.list()),
            _ => {
                return Err(format!(
                    "input {}: the Atari ST target takes assembler sources only (a raw binary cannot say which of its longs are addresses)",
                    n + 1
                ));
            }
        }
    }
    if sources.is_empty() {
        return Err("no program: give [[input]] asm = [\"main.s\"]".to_string());
    }
    let mut defines: Vec<(String, i64)> =
        layout.define.iter().map(|(k, v)| (k.clone(), *v)).collect();
    defines.extend(cli_defines.iter().cloned());
    // One source assembled from the sources in order: an include of each.
    let mut text = String::new();
    for s in &sources {
        let (path, _) = read_source(base, s)?;
        text.push_str(&format!(
            "    .include \"{}\"\n",
            path.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
    let opts = M68kOptions {
        relocatable: true,
        include_dirs: vec![base.to_path_buf()],
        defines,
    };
    let asm = assemble_m68k("layout", &text, &opts, &rcasm::DiskFiles).map_err(|errors| {
        errors
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    if asm.text.is_empty() {
        return Err("the program has no text".to_string());
    }
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
