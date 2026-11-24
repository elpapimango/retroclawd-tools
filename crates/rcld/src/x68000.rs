//! The Sharp X68000 target: one relocatable 68000 program, written as a Human68k `.X` executable.
//!
//! The layout names the sources (`[[input]] asm = [...]`, assembled together with sections `.text`, `.data` and `.bss`)
//! and an output with the extension `.x`. `entry` (a label or an offset) sets the execution address, the offset from the
//! program's start (default 0). Each long that holds an address of the program is recorded in the relocation table the
//! loader walks. With the extension `.xdf` the program goes into a 2HD floppy image with a Human68k filesystem (`human68k`:
//! the program as `NAME.X`, the `[[file]]` entries, `options.disk_name` as the label, `options.ipl` as the boot sector).

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
    if ext != "x" && ext != "xdf" {
        return Err(format!(
            "{}: an X68000 program is written as .x or on a floppy as .xdf, not .{ext}",
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
    let (bytes, disk_note) = if ext == "xdf" {
        let resolve = |p: &str| {
            if Path::new(p).is_absolute() {
                std::path::PathBuf::from(p)
            } else {
                base.join(p)
            }
        };
        let name = match (&layout.options.file_name, &layout.name) {
            (Some(n), _) | (None, Some(n)) => n.clone(),
            _ => "PROGRAM".to_string(),
        };
        let name = if name.contains('.') {
            name
        } else {
            format!("{name}.X")
        };
        let mut files = vec![crate::human68k::File { name, data: bytes }];
        for f in &layout.file {
            let path = resolve(&f.path);
            let data =
                std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let name = f.name.clone().unwrap_or_else(|| {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            files.push(crate::human68k::File { name, data });
        }
        let ipl = match &layout.options.ipl {
            Some(p) => {
                let path = resolve(p);
                Some(
                    std::fs::read(&path)
                        .map_err(|e| format!("cannot read {}: {e}", path.display()))?,
                )
            }
            None => None,
        };
        let n = files.len();
        let disk = crate::human68k::build_xdf(
            &files,
            layout.options.disk_name.as_deref(),
            ipl.as_deref(),
        )?;
        (
            disk,
            format!("floppy: {n} file(s) on a Human68k 2HD disk\n"),
        )
    } else {
        (bytes, String::new())
    };
    let map = format!(
        "{disk_note}text {} bytes, data {} bytes, bss {} bytes, {} relocations, entry ${entry:X}\n",
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
