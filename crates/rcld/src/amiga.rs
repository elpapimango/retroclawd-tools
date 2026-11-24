//! The Amiga target: one relocatable 68000 program, written as a hunk executable or put on an ADF disk.
//!
//! The layout names the sources (`[[input]] asm = [...]`, assembled together with sections `.text`, `.data` and `.bss`)
//! and an output with no extension or `.exe` (the hunk file) or `.adf` (a disk). The text becomes the `CODE` hunk, the
//! data the `DATA` hunk (when there is any), the bss the `BSS` hunk. The assembler numbers addresses as one space (text,
//! then data, then bss) and records each long that holds an address; here each such long is pointed into the hunk its
//! value falls in, and listed in that hunk's `RELOC32` block. A program starts at the first byte of its first hunk, so
//! `entry` is refused unless it is the first label.
//!
//! `[options]`: `memory` (`any`, `chip`, `fast`: where the hunks load; `chip` for code that feeds the custom chips),
//! `hunk_symbols` (write `HUNK_SYMBOL` blocks with the labels), and for `.adf`: `fs` (`ofs` or `ffs`), `bootable`
//! (the standard boot block, default true), `disk_name`, `file_name` (the program's name on the disk), `startup` (the text
//! of `s/startup-sequence`, default the program's name); `[[file]]` entries add files (`name` may include directories).

use crate::Build;
use crate::layout::Layout;
use amiga::adf::{self, Adf, Fs};
use amiga::hunk::{Executable, Hunk, Kind, Memory, Reloc};
use rcasm::m68k::{M68kOptions, assemble_m68k, symbols_text};
use std::path::{Path, PathBuf};

fn resolve(base: &Path, p: &str) -> PathBuf {
    if Path::new(p).is_absolute() {
        PathBuf::from(p)
    } else {
        base.join(p)
    }
}

/// Assembles the layout's sources as one relocatable 68000 program.
pub(crate) fn assemble_68000(
    layout: &Layout,
    base: &Path,
    cli_defines: &[(String, i64)],
    who: &str,
) -> Result<rcasm::m68k::M68kAssembly, String> {
    if !layout.region.is_empty() || !layout.reserve.is_empty() {
        return Err(format!(
            "regions and reserves are for machines with a fixed memory layout; the {who} loader places the program"
        ));
    }
    let items = crate::linked_items(layout, base);
    if !items.is_empty() {
        if layout
            .input
            .iter()
            .any(|i| i.asm.is_some() || i.bin.is_some() || i.prg.is_some())
        {
            return Err(format!(
                "the {who} target takes either `asm` sources or `src` / `obj` / `lib` inputs, not both"
            ));
        }
        let mut defines: Vec<(String, i64)> =
            layout.define.iter().map(|(k, v)| (k.clone(), *v)).collect();
        defines.extend(cli_defines.iter().cloned());
        return crate::m68klink::link_relocatable(layout, base, &items, &defines, who);
    }
    let mut sources = Vec::new();
    for (n, input) in layout.input.iter().enumerate() {
        match (&input.asm, &input.bin, &input.prg, &input.symbols) {
            (Some(asm), None, None, None) => sources.extend(asm.list()),
            _ => {
                return Err(format!(
                    "input {}: the {who} target takes assembler sources only (a raw binary cannot say which of its longs are addresses)",
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
    let mut text = String::new();
    for s in &sources {
        text.push_str(&format!(
            "    .include \"{}\"\n",
            resolve(base, s)
                .display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        ));
    }
    let opts = M68kOptions {
        relocatable: true,
        object: false,
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
    Ok(asm)
}

/// The program as hunks.
fn hunks_of(
    asm: &rcasm::m68k::M68kAssembly,
    memory: Memory,
    with_symbols: bool,
) -> Result<Executable, String> {
    let text_len = asm.text.len() as u32;
    let data_len = asm.data.len() as u32;
    // hunk numbers and the bases of the three sections in the assembler's single address space
    let has_data = data_len > 0;
    let has_bss = asm.bss > 0;
    let data_hunk = 1u32;
    let bss_hunk = if has_data { 2 } else { 1 };
    let sections: Vec<(u32, u32, u32)> = {
        let mut v = vec![(0, 0, text_len)];
        if has_data {
            v.push((data_hunk, text_len, text_len + data_len));
        }
        if has_bss {
            v.push((bss_hunk, text_len + data_len, text_len + data_len + asm.bss));
        }
        v
    };
    // which hunk an address value points into (an address one past the end of text counts as the next section)
    let target_of = |v: u32| -> Option<(u32, u32)> {
        for (i, &(hunk, start, end)) in sections.iter().enumerate() {
            let last = i + 1 == sections.len();
            if v >= start && (v < end || (last && v == end)) {
                return Some((hunk, start));
            }
        }
        None
    };
    let mut text = asm.text.clone();
    let mut data = asm.data.clone();
    let mut relocs: [Vec<(u32, Vec<u32>)>; 2] = [Vec::new(), Vec::new()];
    for &r in &asm.relocs {
        let (buf, which, local) = if r < text_len {
            (&mut text, 0usize, r)
        } else {
            (&mut data, 1usize, r - text_len)
        };
        let at = local as usize;
        let Some(slot) = buf.get(at..at + 4) else {
            return Err(format!("a relocation at {r} is outside the program"));
        };
        let v = u32::from_be_bytes([slot[0], slot[1], slot[2], slot[3]]);
        let Some((hunk, start)) = target_of(v) else {
            return Err(format!(
                "the address ${v:X} at offset {r} points outside the program"
            ));
        };
        buf[at..at + 4].copy_from_slice(&(v - start).to_be_bytes());
        match relocs[which].iter_mut().find(|(h, _)| *h == hunk) {
            Some((_, offsets)) => offsets.push(local),
            None => relocs[which].push((hunk, vec![local])),
        }
    }
    let mk =
        |kind: Kind, bytes: Vec<u8>, size: u32, rel: &Vec<(u32, Vec<u32>)>, lo: u32, hi: u32| {
            let mut h = match kind {
                Kind::Code => Hunk::code(bytes),
                Kind::Data => Hunk::data(bytes),
                Kind::Bss => Hunk::bss(size),
            };
            h.memory = memory;
            h.relocs = rel
                .iter()
                .map(|(target, offsets)| Reloc {
                    target: *target,
                    offsets: offsets.clone(),
                })
                .collect();
            if with_symbols {
                h.symbols = asm
                    .symbols
                    .iter()
                    .filter(|s| s.label && s.value >= lo as i64 && s.value < hi as i64)
                    .map(|s| (s.name.clone(), (s.value - lo as i64) as u32))
                    .collect();
            }
            h
        };
    let mut hunks = vec![mk(Kind::Code, text, text_len, &relocs[0], 0, text_len)];
    if has_data {
        hunks.push(mk(
            Kind::Data,
            data,
            data_len,
            &relocs[1],
            text_len,
            text_len + data_len,
        ));
    }
    if has_bss {
        hunks.push(mk(
            Kind::Bss,
            Vec::new(),
            asm.bss,
            &Vec::new(),
            text_len + data_len,
            text_len + data_len + asm.bss,
        ));
    }
    Ok(Executable { hunks })
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
    if !matches!(ext.as_str(), "" | "exe" | "adf") {
        return Err(format!(
            "{}: an Amiga program is a hunk file (no extension or .exe) or a disk (.adf), not .{ext}",
            output.display()
        ));
    }
    let o = &layout.options;
    let memory = match o.memory.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("any") => Memory::Any,
        Some("chip") => Memory::Chip,
        Some("fast") => Memory::Fast,
        Some(other) => return Err(format!("options.memory \"{other}\": any, chip or fast")),
    };
    let asm = assemble_68000(layout, base, cli_defines, "Amiga")?;
    if let Some(e) = &layout.entry {
        // the program starts at the first byte: only a label at 0 is honest
        let at = match e {
            crate::layout::Value::Int(a) => Some(*a),
            crate::layout::Value::Text(t) => asm
                .symbols
                .iter()
                .find(|s| s.name == *t)
                .map(|s| s.value)
                .or_else(|| rc_common::parse_number(t).ok()),
        };
        if at != Some(0) {
            return Err(format!(
                "entry: an Amiga program starts at the first byte of its first hunk; \"{}\" is not there (put the entry code first, or start with a branch to it)",
                match e {
                    crate::layout::Value::Int(a) => format!("{a:#X}"),
                    crate::layout::Value::Text(t) => t.clone(),
                }
            ));
        }
    }
    let exe = hunks_of(&asm, memory, o.hunk_symbols == Some(true))?;
    let program = exe.to_bytes().map_err(|e| e.to_string())?;
    let mut notes = Vec::new();
    let mut map = format!(
        "{} hunk(s): code {} bytes, data {} bytes, bss {} bytes, {} relocations\n",
        exe.hunks.len(),
        asm.text.len(),
        asm.data.len(),
        asm.bss,
        asm.relocs.len()
    );
    let bytes = if ext == "adf" {
        let name = o
            .file_name
            .clone()
            .or_else(|| layout.name.as_ref().map(|n| n.to_ascii_lowercase()))
            .unwrap_or_else(|| "program".to_string());
        let fs = match o.fs.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None | Some("ofs") => Fs::Ofs,
            Some("ffs") => Fs::Ffs,
            Some(other) => return Err(format!("options.fs \"{other}\": ofs or ffs")),
        };
        let bootable = o.bootable != Some(false);
        let mut disk = Adf::format(&adf::Format {
            name: o
                .disk_name
                .clone()
                .or_else(|| layout.name.clone())
                .unwrap_or_else(|| "Program".to_string()),
            fs,
            intl: true,
            high_density: false,
            boot_code: bootable.then(|| adf::STANDARD_BOOT_CODE.to_vec()),
        })
        .map_err(|e| e.to_string())?;
        disk.add_file(&name, &program)
            .map_err(|e| format!("{name}: {e}"))?;
        disk.mkdir("s").map_err(|e| e.to_string())?;
        let startup = o.startup.clone().unwrap_or_else(|| name.clone());
        disk.add_file("s/startup-sequence", format!("{startup}\n").as_bytes())
            .map_err(|e| e.to_string())?;
        for f in &layout.file {
            let data = std::fs::read(resolve(base, &f.path))
                .map_err(|e| format!("cannot read {}: {e}", f.path))?;
            let target = f.name.clone().unwrap_or_else(|| {
                Path::new(&f.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            // make the directories of the target
            let parts: Vec<&str> = target.split('/').filter(|p| !p.is_empty()).collect();
            for i in 1..parts.len() {
                let dir = parts[..i].join("/");
                if disk.list(&dir).is_err() {
                    disk.mkdir(&dir).map_err(|e| format!("{dir}: {e}"))?;
                }
            }
            disk.add_file(&target, &data)
                .map_err(|e| format!("{target}: {e}"))?;
        }
        disk.verify().map_err(|e| e.to_string())?;
        map.push_str(&format!(
            "disk \"{}\" ({:?}): {} blocks free\n",
            disk.name(),
            fs,
            disk.free_blocks()
        ));
        if !bootable {
            notes.push(
                "the disk has no boot code: it starts only from a Workbench or a boot menu"
                    .to_string(),
            );
        }
        disk.to_bytes()
    } else {
        program
    };
    Ok(Build {
        bytes,
        notes,
        map,
        symbols_text: symbols_text(&asm, false),
    })
}
