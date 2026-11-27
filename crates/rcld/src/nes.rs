//! The NES / Famicom target: an iNES cartridge file.
//!
//! The layout names 6502 sources (`[[input]] asm = [...]`, assembled together) and, in `[options]`, the cartridge:
//! `mapper`, `submapper`, `mirroring` (`horizontal`, `vertical`, `four`), `battery`, `chr` (a `.chr` file: the CHR ROM),
//! `chr_ram`, `prg_ram`, `nes2`.
//!
//! **Flat** (the sources never use `.bank`, `options.banked` is not set): the program lives in `$8000-$FFFF` and the
//! PRG ROM is that 32 KB, or the 16 KB `$C000-$FFFF` when nothing is below `$C000` (NROM-128).
//! **Banked** (`.bank n` before an `.org`, or `banked = true`): PRG is 16 KB banks, bank `n` at file offset
//! `n * 16384 + (address & $3FFF)`; the last bank is the one fixed at `$C000` and carries the vectors. The size is the
//! next power of two of the banks used (or `options.size`).
//!
//! The vectors (`$FFFA` NMI, `$FFFC` reset, `$FFFE` IRQ/BRK) are the last six bytes of the PRG. Unless the sources
//! write those bytes, they come from the labels `nmi`, `reset` (or the layout's `entry`, or `start`) and `irq`
//! (or `brk`); a vector without a label points at the reset code, with a note.

use crate::Build;
use crate::layout::{self, Layout};
use ines::{Ines, Mirroring};
use rcasm::{Cpu, SymbolStyle};
use std::fmt::Write;
use std::path::{Path, PathBuf};

const BANK: usize = 0x4000;

struct Piece {
    name: String,
    bank: u16,
    start: u16,
    offset: usize,
    bytes: Vec<u8>,
}

fn resolve(base: &Path, p: &str) -> PathBuf {
    if Path::new(p).is_absolute() {
        PathBuf::from(p)
    } else {
        base.join(p)
    }
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
    if ext != "nes" {
        return Err(format!(
            "{}: a NES cartridge is written as .nes, not .{ext}",
            output.display()
        ));
    }
    if !layout.region.is_empty() || !layout.reserve.is_empty() {
        return Err("regions and reserves are for machines with one flat memory; a NES cartridge is banks of ROM".to_string());
    }
    let o = &layout.options;
    let mut sources: Vec<String> = Vec::new();
    let mut raw: Vec<Piece> = Vec::new();
    for (n, input) in layout.input.iter().enumerate() {
        match (&input.asm, &input.bin, &input.prg, &input.symbols) {
            (Some(asm), None, None, None) => {
                if input.bank.is_some() || input.at.is_some() {
                    return Err(format!(
                        "input {}: assembler sources choose their banks with .bank and .org",
                        n + 1
                    ));
                }
                sources.extend(
                    asm.list()
                        .iter()
                        .map(|s| resolve(base, s).display().to_string()),
                );
            }
            (None, Some(bin), None, None) => {
                let at = input.at.as_ref().ok_or_else(|| {
                    format!("input {}: bin \"{bin}\" needs \"at\" (its address)", n + 1)
                })?;
                let start = layout::address(at, &format!("input {} at", n + 1))?;
                let path = resolve(base, bin);
                let bytes = std::fs::read(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                raw.push(Piece {
                    name: bin.clone(),
                    bank: input.bank.unwrap_or(0),
                    start,
                    offset: 0,
                    bytes,
                });
            }
            _ => {
                return Err(format!(
                    "input {}: the NES target takes asm and bin inputs",
                    n + 1
                ));
            }
        }
    }
    let mut defines: Vec<(String, i64)> =
        layout.define.iter().map(|(k, v)| (k.clone(), *v)).collect();
    defines.extend(cli_defines.iter().cloned());
    let mut symbols: Vec<(String, i64)> = Vec::new();
    let mut symbols_text = String::new();
    let mut pieces = raw;
    if !sources.is_empty() {
        let mut text = String::new();
        for s in &sources {
            text.push_str(&format!(
                "    .include \"{}\"\n",
                s.replace('\\', "\\\\").replace('"', "\\\"")
            ));
        }
        let opts = rcasm::Options {
            cpu: Some(Cpu::Mos6502),
            syntax: crate::syntax_of(layout)?,
            defines,
            include_dirs: vec![base.to_path_buf()],
            ..rcasm::Options::default()
        };
        let asm = rcasm::assemble("layout", &text, &opts, &rcasm::DiskFiles).map_err(|errors| {
            errors
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        })?;
        for s in &asm.segments {
            pieces.push(Piece {
                name: format!("code at bank {} ${:04X}", s.bank, s.start),
                bank: s.bank,
                start: s.start as u16,
                offset: 0,
                bytes: s.bytes.clone(),
            });
        }
        symbols = asm
            .symbols
            .iter()
            .map(|s| (s.name.clone(), s.value))
            .collect();
        symbols_text = rcasm::symbols_text(&asm, false, SymbolStyle::Rc);
    }
    if pieces.is_empty() {
        return Err("no program: give [[input]] asm = [\"main.asm\"]".to_string());
    }
    for p in &pieces {
        if p.start < 0x8000 {
            return Err(format!(
                "{}: ${:04X} is below $8000, where the cartridge's PRG ROM is mapped",
                p.name, p.start
            ));
        }
        if p.start as usize + p.bytes.len() > 0x10000 {
            return Err(format!("{}: runs past $FFFF", p.name));
        }
    }

    let banked = o.banked == Some(true) || pieces.iter().any(|p| p.bank != 0);
    let mut notes = Vec::new();
    let prg_len;
    if banked {
        let banks = pieces
            .iter()
            .map(|p| {
                p.bank as usize
                    + (p.start as usize & (BANK - 1))
                        .saturating_add(p.bytes.len())
                        .div_ceil(BANK)
                        .max(1)
                    - 1
            })
            .max()
            .unwrap_or(0)
            + 1;
        prg_len = match &o.size {
            Some(v) => {
                let n = layout::number(v).map_err(|e| format!("options.size: {e}"))?;
                usize::try_from(n).map_err(|_| format!("options.size: {n} is not a size"))?
            }
            None => (banks * BANK).next_power_of_two(),
        };
        if prg_len % BANK != 0 || prg_len < banks * BANK {
            return Err(format!(
                "options.size {prg_len}: the PRG ROM is a multiple of 16 KB and the banks used need {} bytes",
                banks * BANK
            ));
        }
        for p in &mut pieces {
            p.offset = p.bank as usize * BANK + (p.start as usize & (BANK - 1));
        }
    } else {
        let lowest = pieces.iter().map(|p| p.start).min().unwrap_or(0x8000);
        prg_len = match &o.size {
            Some(v) => {
                let n = layout::number(v).map_err(|e| format!("options.size: {n}", n = e))?;
                usize::try_from(n).map_err(|_| format!("options.size: {n} is not a size"))?
            }
            None if lowest >= 0xC000 => 0x4000,
            None => 0x8000,
        };
        if !matches!(prg_len, 0x4000 | 0x8000) {
            return Err(format!(
                "options.size {prg_len}: a flat cartridge is 16 or 32 KB (use .bank for more)"
            ));
        }
        let window = 0x10000 - prg_len;
        for p in &mut pieces {
            if (p.start as usize) < window {
                return Err(format!(
                    "{}: ${:04X} is below ${window:04X}, the start of a {} KB PRG ROM",
                    p.name,
                    p.start,
                    prg_len / 1024
                ));
            }
            p.offset = p.start as usize - window;
        }
    }
    pieces.sort_by_key(|p| p.offset);
    for pair in pieces.windows(2) {
        let end = pair[0].offset + pair[0].bytes.len();
        if pair[1].offset < end {
            return Err(format!(
                "{} (PRG offset ${:X}-${:X}) overlaps {} (from ${:X})",
                pair[0].name,
                pair[0].offset,
                end - 1,
                pair[1].name,
                pair[1].offset
            ));
        }
    }
    let used = pieces
        .iter()
        .map(|p| p.offset + p.bytes.len())
        .max()
        .unwrap_or(0);
    if used > prg_len {
        return Err(format!(
            "the program needs {used} bytes of PRG ROM but options.size is {prg_len}"
        ));
    }
    let mut prg = vec![layout.fill.unwrap_or(0xFF); prg_len];
    for p in &pieces {
        prg[p.offset..p.offset + p.bytes.len()].copy_from_slice(&p.bytes);
    }

    // vectors: the last six bytes of the PRG
    let vectors = prg_len - 6..prg_len;
    let written = pieces
        .iter()
        .any(|p| p.offset < vectors.end && p.offset + p.bytes.len() > vectors.start);
    if !written {
        let find = |names: &[&str]| -> Option<i64> {
            names.iter().find_map(|n| {
                symbols
                    .iter()
                    .find(|(s, _)| s.eq_ignore_ascii_case(n))
                    .map(|(_, v)| *v)
            })
        };
        let entry = match &layout.entry {
            Some(layout::Value::Int(a)) => Some(*a),
            Some(layout::Value::Text(t)) => match find(&[t.as_str()]) {
                Some(v) => Some(v),
                None => Some(rc_common::parse_number(t).map_err(|_| {
                    format!("entry \"{t}\" is neither a label of the sources nor an address")
                })?),
            },
            None => find(&["reset", "start"]),
        };
        let reset = entry.ok_or_else(|| {
            "no reset vector: define the label `reset` (or `start`), set `entry`, or write the vectors at $FFFA yourself".to_string()
        })?;
        let reset = u16::try_from(reset)
            .ok()
            .filter(|a| *a >= 0x8000)
            .ok_or_else(|| format!("the reset code at {reset:#X} is not in $8000-$FFFF"))?;
        let mut vector = |names: &[&str], what: &str| -> Result<u16, String> {
            match find(names) {
                Some(v) => {
                    u16::try_from(v).map_err(|_| format!("vector {what} is not a 16-bit address"))
                }
                None => {
                    notes.push(format!(
                        "no label `{}`: the {what} vector points at the reset code",
                        names[0]
                    ));
                    Ok(reset)
                }
            }
        };
        let nmi = vector(&["nmi"], "NMI")?;
        let irq = vector(&["irq", "brk"], "IRQ")?;
        prg[prg_len - 6..prg_len - 4].copy_from_slice(&nmi.to_le_bytes());
        prg[prg_len - 4..prg_len - 2].copy_from_slice(&reset.to_le_bytes());
        prg[prg_len - 2..prg_len].copy_from_slice(&irq.to_le_bytes());
    }

    // CHR
    let chr = match &o.chr {
        Some(p) => {
            let path = resolve(base, p);
            let bytes =
                std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            if bytes.is_empty() || bytes.len() % 0x2000 != 0 {
                return Err(format!(
                    "{}: CHR ROM is a multiple of 8 KB, this is {} bytes",
                    path.display(),
                    bytes.len()
                ));
            }
            bytes
        }
        None => Vec::new(),
    };
    let mut cart = Ines::new(o.mapper.unwrap_or(0), prg, chr);
    cart.nes2 =
        o.nes2.unwrap_or(false) || o.mapper.is_some_and(|m| m > 255) || o.submapper.is_some();
    cart.submapper = o.submapper.unwrap_or(0);
    cart.mirroring = match o
        .mirroring
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("horizontal") => Mirroring::Horizontal,
        Some("vertical") => Mirroring::Vertical,
        Some("four") | Some("four-screen") => Mirroring::FourScreen,
        Some(other) => {
            return Err(format!(
                "options.mirroring \"{other}\": horizontal, vertical or four"
            ));
        }
    };
    cart.battery = o.battery.unwrap_or(false);
    let ram = |v: &Option<layout::Value>, what: &str| -> Result<u32, String> {
        match v {
            Some(v) => {
                let n = layout::number(v).map_err(|e| format!("options.{what}: {e}"))?;
                u32::try_from(n).map_err(|_| format!("options.{what}: {n} is not a size"))
            }
            None => Ok(0),
        }
    };
    let prg_ram = ram(&o.prg_ram, "prg_ram")?;
    if cart.battery {
        cart.prg_nvram = if prg_ram > 0 { prg_ram } else { 8192 };
    } else {
        cart.prg_ram = prg_ram;
    }
    cart.chr_ram = ram(&o.chr_ram, "chr_ram")?;
    if cart.chr_rom.is_empty() && cart.chr_ram == 0 {
        cart.chr_ram = 8192;
    }
    let bytes = cart.to_bytes().map_err(|e| e.to_string())?;
    let mut map = String::new();
    for p in &pieces {
        let _ = writeln!(
            map,
            "bank {:3} ${:04X}-${:04X}  prg ${:06X}  {}",
            p.bank,
            p.start,
            p.start as usize + p.bytes.len().saturating_sub(1),
            p.offset,
            p.name
        );
    }
    let _ = writeln!(
        map,
        "mapper {}, PRG {} KB, CHR {} KB{}",
        cart.mapper,
        cart.prg_rom.len() / 1024,
        cart.chr_rom.len() / 1024,
        if banked { ", banked" } else { "" }
    );
    Ok(Build {
        bytes,
        notes,
        map,
        symbols_text,
    })
}
