//! The Super Nintendo / Super Famicom target: a `.sfc` cartridge image.
//!
//! The layout names 65816 sources (`[[input]] asm = [...]`, assembled together; the addresses written by `.org` are the
//! 24-bit addresses the CPU uses, such as `$808000`) and raw binaries (`bin` with a 24-bit `at`). `[options]`:
//! `mapping` (`lorom` default, `hirom`, `exhirom`), `fast`, `title`, `cart_type`, `ram_size`, `country`, `version`,
//! `size` (the ROM size, default the next power of two, at least the mapping's minimum), `fix_checksum`.
//!
//! An address maps to a ROM offset by the cartridge's mapping:
//!
//! | mapping | addresses | ROM offset |
//! |---|---|---|
//! | LoROM | banks `$00-$7D` / `$80-$FF`, `$8000-$FFFF` | `(bank & $7F) * $8000 + (address & $7FFF)` |
//! | HiROM | banks `$C0-$FF`, any address | `(bank - $C0) * $10000 + address16` |
//! | HiROM | banks `$00-$3F` / `$80-$BF`, `$8000-$FFFF` | `(bank & $3F) * $10000 + address16` |
//! | ExHiROM | banks `$C0-$FF` / `$40-$7D` | `(bank - $C0) * $10000 + address16` / `$400000 + (bank - $40) * $10000 + address16` |
//!
//! The internal header (title, map mode, type, sizes, vectors) is written from `[options]` and the labels `reset`
//! (or the layout's `entry`, or `start`), `nmi`, `irq`, `cop`, `brk`, `abort`; a vector without a label points at the
//! reset code, with a note. If the sources write the header bytes themselves (the 32 bytes at the mapping's header
//! address), the header is left alone and only the checksum is fixed.

use crate::Build;
use crate::layout::{self, Layout};
use rcasm::{Cpu, SymbolStyle};
use sfc::{BuildOptions, Mapping, Sfc, Vectors};
use std::fmt::Write;
use std::path::{Path, PathBuf};

struct Piece {
    name: String,
    addr: u32,
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

/// The ROM offset of the 24-bit address `addr` under `mapping`.
fn rom_offset(mapping: Mapping, addr: u32, len: usize, name: &str) -> Result<usize, String> {
    let bank = (addr >> 16) as usize;
    let low = (addr & 0xFFFF) as usize;
    let bad = |why: &str| {
        Err(format!(
            "{name}: ${addr:06X} is {why} in {} (see the table in the rcld docs)",
            match mapping {
                Mapping::LoRom => "LoROM",
                Mapping::HiRom => "HiROM",
                Mapping::ExHiRom => "ExHiROM",
            }
        ))
    };
    let (offset, limit) = match mapping {
        Mapping::LoRom => {
            if !((0x00..=0x7D).contains(&bank) || (0x80..=0xFF).contains(&bank)) || low < 0x8000 {
                return bad("not in the ROM window ($8000-$FFFF of banks $00-$7D and $80-$FF)");
            }
            (
                (bank & 0x7F) * 0x8000 + (low & 0x7FFF),
                0x8000 - (low & 0x7FFF),
            )
        }
        Mapping::HiRom => {
            if bank >= 0xC0 {
                ((bank - 0xC0) * 0x10000 + low, 0x10000 - low)
            } else if (bank & 0x7F) < 0x40 && low >= 0x8000 {
                ((bank & 0x3F) * 0x10000 + low, 0x10000 - low)
            } else {
                return bad(
                    "not in the ROM window (banks $C0-$FF, or $8000-$FFFF of $00-$3F / $80-$BF)",
                );
            }
        }
        Mapping::ExHiRom => {
            if bank >= 0xC0 {
                ((bank - 0xC0) * 0x10000 + low, 0x10000 - low)
            } else if (0x40..=0x7D).contains(&bank) {
                (0x40_0000 + (bank - 0x40) * 0x10000 + low, 0x10000 - low)
            } else if bank < 0x40 && low >= 0x8000 {
                (0x40_0000 + bank * 0x10000 + low, 0x10000 - low)
            } else if (0x80..0xC0).contains(&bank) && low >= 0x8000 {
                ((bank - 0x80) * 0x10000 + low, 0x10000 - low)
            } else {
                return bad("not in the ROM window");
            }
        }
    };
    let _ = limit;
    let _ = len;
    Ok(offset)
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
    if !matches!(ext.as_str(), "sfc" | "smc") {
        return Err(format!(
            "{}: a Super Nintendo cartridge is written as .sfc (or .smc), not .{ext}",
            output.display()
        ));
    }
    if !layout.region.is_empty() || !layout.reserve.is_empty() {
        return Err("regions and reserves are for machines with one flat memory; a Super Nintendo cartridge is banks of ROM".to_string());
    }
    let o = &layout.options;
    let mapping = match o.mapping.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("lorom") => Mapping::LoRom,
        Some("hirom") => Mapping::HiRom,
        Some("exhirom") => Mapping::ExHiRom,
        Some(other) => {
            return Err(format!(
                "options.mapping \"{other}\": lorom, hirom or exhirom"
            ));
        }
    };
    let mut sources: Vec<String> = Vec::new();
    let mut pieces: Vec<Piece> = Vec::new();
    for (n, input) in layout.input.iter().enumerate() {
        match (&input.asm, &input.bin, &input.prg, &input.symbols) {
            (Some(asm), None, None, None) => {
                if input.bank.is_some() || input.at.is_some() {
                    return Err(format!(
                        "input {}: assembler sources choose their addresses with .org",
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
                    format!(
                        "input {}: bin \"{bin}\" needs \"at\" (its 24-bit address)",
                        n + 1
                    )
                })?;
                let addr = layout::number(at).map_err(|e| format!("input {} at: {e}", n + 1))?;
                let addr = u32::try_from(addr)
                    .ok()
                    .filter(|a| *a <= 0xFF_FFFF)
                    .ok_or_else(|| {
                        format!("input {} at: {addr:#X} is outside 0..16777215", n + 1)
                    })?;
                let path = resolve(base, bin);
                let bytes = std::fs::read(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                pieces.push(Piece {
                    name: bin.clone(),
                    addr,
                    offset: 0,
                    bytes,
                });
            }
            _ => {
                return Err(format!(
                    "input {}: the Super Nintendo target takes asm and bin inputs",
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
    if !sources.is_empty() {
        let mut text = String::new();
        for s in &sources {
            text.push_str(&format!(
                "    .include \"{}\"\n",
                s.replace('\\', "\\\\").replace('"', "\\\"")
            ));
        }
        let opts = rcasm::Options {
            cpu: Some(Cpu::W65816),
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
                name: format!("code at ${:06X}", s.start),
                addr: s.start,
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
    for p in &mut pieces {
        p.offset = rom_offset(mapping, p.addr, p.bytes.len(), &p.name)?;
        // a piece must stay in one bank's ROM window: the last byte maps contiguously
        if !p.bytes.is_empty() {
            let last = rom_offset(mapping, p.addr + p.bytes.len() as u32 - 1, 1, &p.name);
            match last {
                Ok(l) if l == p.offset + p.bytes.len() - 1 => {}
                _ => {
                    return Err(format!(
                        "{} (${:06X}, {} bytes) crosses the end of a bank's ROM window; start the next part with its own .org",
                        p.name,
                        p.addr,
                        p.bytes.len()
                    ));
                }
            }
        }
    }
    pieces.sort_by_key(|p| p.offset);
    for pair in pieces.windows(2) {
        let end = pair[0].offset + pair[0].bytes.len();
        if pair[1].offset < end {
            return Err(format!(
                "{} (ROM offset ${:X}-${:X}) overlaps {} (from ${:X})",
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
    let fill = layout.fill.unwrap_or(0xFF);
    let mut rom = vec![fill; used];
    for p in &pieces {
        rom[p.offset..p.offset + p.bytes.len()].copy_from_slice(&p.bytes);
    }
    if let Some(v) = &o.size {
        let n = layout::number(v).map_err(|e| format!("options.size: {e}"))?;
        let n = usize::try_from(n).map_err(|_| format!("options.size: {n} is not a size"))?;
        if !n.is_power_of_two() || n < used {
            return Err(format!(
                "options.size {n}: a power of two of at least the {used} bytes the program needs"
            ));
        }
        rom.resize(n, fill);
    }

    let header_at = mapping.header_offset();
    let header_written = pieces
        .iter()
        .any(|p| p.offset < header_at + 32 && p.offset + p.bytes.len() > header_at);
    let mut notes = Vec::new();
    let sfc = if header_written {
        let min = match mapping {
            Mapping::LoRom => 0x8000,
            Mapping::HiRom => 0x10000,
            Mapping::ExHiRom => 0x41_0000,
        };
        rom.resize(rom.len().max(min).next_power_of_two(), fill);
        let mut s = Sfc::parse(&rom).map_err(|e| format!("the header the sources write: {e}"))?;
        if o.fix_checksum != Some(false) {
            s.fix_checksum();
        }
        s
    } else {
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
            "no reset vector: define the label `reset` (or `start`), set `entry`, or write the header yourself".to_string()
        })?;
        let reset16 = u16::try_from(reset & 0xFFFF).unwrap_or(0);
        if reset & 0xFFFF < 0x8000 {
            return Err(format!(
                "the reset code at {reset:#X} is not in the upper half of a bank ($8000-$FFFF): the CPU starts in bank 0 with the vector's 16-bit address"
            ));
        }
        let mut vec16 = |names: &[&str], what: &str| -> u16 {
            match find(names) {
                Some(v) => (v & 0xFFFF) as u16,
                None => {
                    notes.push(format!(
                        "no label `{}`: the {what} vector points at the reset code",
                        names[0]
                    ));
                    reset16
                }
            }
        };
        let nmi = vec16(&["nmi"], "NMI");
        let irq = vec16(&["irq"], "IRQ");
        let cop = vec16(&["cop"], "COP");
        let brk = vec16(&["brk"], "BRK");
        let abort = vec16(&["abort"], "ABORT");
        let opts = BuildOptions {
            mapping,
            title: o.title.clone().unwrap_or_else(|| {
                layout
                    .name
                    .clone()
                    .unwrap_or_else(|| "RETROCLAWD".to_string())
                    .to_ascii_uppercase()
            }),
            fast: o.fast.unwrap_or(false),
            cart_type: o.cart_type.unwrap_or(0),
            ram_size: o.ram_size.unwrap_or(0),
            country: o.country.unwrap_or(0),
            version: o.version.unwrap_or(0),
            vectors: Vectors {
                native_cop: cop,
                native_brk: brk,
                native_abort: abort,
                native_nmi: nmi,
                native_irq: irq,
                cop,
                abort,
                nmi,
                reset: reset16,
                irq,
            },
            fill,
        };
        Sfc::build(rom, &opts).map_err(|e| e.to_string())?
    };
    let mut map = String::new();
    for p in &pieces {
        let _ = writeln!(
            map,
            "${:06X}-${:06X}  rom ${:06X}  {}",
            p.addr,
            p.addr as usize + p.bytes.len().saturating_sub(1),
            p.offset,
            p.name
        );
    }
    let _ = writeln!(
        map,
        "{} KB, {:?}, header \"{}\"{}",
        sfc.rom.len() / 1024,
        sfc.mapping,
        sfc.header.title,
        if header_written {
            " (written by the sources)"
        } else {
            ""
        }
    );
    Ok(Build {
        bytes: sfc.to_bytes(),
        notes,
        map,
        symbols_text,
    })
}
