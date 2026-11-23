//! The PC Engine / TurboGrafx-16 target: a HuCard image.
//!
//! The layout names HuC6280 sources (`[[input]] asm = [...]`, assembled together; `.bank n` before an `.org` puts the
//! code in 8 KB ROM bank `n`) and raw binaries (`bin`, `at`, optional `bank`). Bank 0 is what the CPU sees at
//! `$E000-$FFFF` after reset, so code in bank 0 has to be assembled there. A piece in bank `n` sits at file offset
//! `n * 8192 + (address & $1FFF)`; a piece may run on into the following banks. The image is padded to `options.size`
//! (default: the next power of two, at least 128 KB) with the `fill` byte (default 0).
//!
//! The interrupt vectors are the last ten bytes of bank 0 (`$FFF6` IRQ2/BRK, `$FFF8` IRQ1, `$FFFA` timer, `$FFFC` NMI,
//! `$FFFE` reset). Unless the sources write those bytes themselves, they come from the labels `irq2`, `irq1`, `timer`,
//! `nmi` and `reset` (or the layout's `entry`, or `start`); a vector without a label points at the reset code.

use crate::Build;
use crate::layout::{self, Layout};
use rcasm::{Cpu, SymbolStyle};
use std::fmt::Write;
use std::path::{Path, PathBuf};

const BANK: usize = 0x2000;
const MIN_SIZE: usize = 0x20000;
/// Sizes HuCards come in beyond the powers of two.
const OTHER_SIZES: [usize; 2] = [0x60000, 0x280000];
const MAX_SIZE: usize = 0x400000;

struct Piece {
    name: String,
    offset: usize,
    bytes: Vec<u8>,
    bank: u16,
    start: u16,
}

fn resolve(base: &Path, p: &str) -> PathBuf {
    if Path::new(p).is_absolute() {
        PathBuf::from(p)
    } else {
        base.join(p)
    }
}

fn offset_of(bank: u16, start: u16, what: &str) -> Result<usize, String> {
    if bank == 0 && start < 0xE000 {
        return Err(format!(
            "{what}: bank 0 is mapped at $E000-$FFFF when the card starts; ${start:04X} is below that"
        ));
    }
    Ok(bank as usize * BANK + (start as usize & (BANK - 1)))
}

fn valid_size(size: usize) -> bool {
    size.is_multiple_of(BANK)
        && (OTHER_SIZES.contains(&size) || (size.is_power_of_two() && size >= BANK))
        && size <= MAX_SIZE
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
    if !matches!(ext.as_str(), "pce" | "tg16") {
        return Err(format!(
            "{}: a HuCard is written as .pce (or .tg16), not .{ext}",
            output.display()
        ));
    }
    if !layout.region.is_empty() || !layout.reserve.is_empty() {
        return Err("regions and reserves are for machines with one flat memory; a HuCard is banks of ROM".to_string());
    }
    let mut sources = Vec::new();
    let mut pieces: Vec<Piece> = Vec::new();
    for (n, input) in layout.input.iter().enumerate() {
        match (&input.asm, &input.bin, &input.prg, &input.symbols) {
            (Some(asm), None, None, None) => {
                if input.bank.is_some() || input.at.is_some() {
                    return Err(format!(
                        "input {}: assembler sources choose their banks with .bank and .org",
                        n + 1
                    ));
                }
                sources.extend(asm.list());
            }
            (None, Some(bin), None, None) => {
                let at = input.at.as_ref().ok_or_else(|| {
                    format!("input {}: bin \"{bin}\" needs \"at\" (its address)", n + 1)
                })?;
                let start = layout::address(at, &format!("input {} at", n + 1))?;
                let bank = input.bank.unwrap_or(0);
                let path = resolve(base, bin);
                let bytes = std::fs::read(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                pieces.push(Piece {
                    name: bin.clone(),
                    offset: offset_of(bank, start, bin)?,
                    bytes,
                    bank,
                    start,
                });
            }
            _ => {
                return Err(format!(
                    "input {}: the PC Engine target takes asm and bin inputs",
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
                resolve(base, s)
                    .display()
                    .to_string()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
            ));
        }
        let opts = rcasm::Options {
            cpu: Some(Cpu::Huc6280),
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
                offset: offset_of(s.bank, s.start, "the program")?,
                bytes: s.bytes.clone(),
                bank: s.bank,
                start: s.start,
            });
        }
        symbols = asm.symbols.iter().map(|s| (s.name.clone(), s.value)).collect();
        symbols_text = rcasm::symbols_text(&asm, false, SymbolStyle::Rc);
    }
    if pieces.is_empty() {
        return Err("no program: give [[input]] asm = [\"main.asm\"]".to_string());
    }

    // Overlaps in the file.
    pieces.sort_by_key(|p| p.offset);
    for pair in pieces.windows(2) {
        let end = pair[0].offset + pair[0].bytes.len();
        if pair[1].offset < end {
            return Err(format!(
                "{} (file offset ${:X}-${:X}) overlaps {} (from ${:X})",
                pair[0].name,
                pair[0].offset,
                end - 1,
                pair[1].name,
                pair[1].offset
            ));
        }
    }

    // Size.
    let used = pieces
        .iter()
        .map(|p| p.offset + p.bytes.len())
        .max()
        .unwrap_or(0)
        .max(BANK);
    let size = match &layout.options.size {
        Some(v) => {
            let n = layout::number(v).map_err(|e| format!("options.size: {e}"))?;
            usize::try_from(n).map_err(|_| format!("options.size: {n} is not a size"))?
        }
        None => used.next_power_of_two().max(MIN_SIZE),
    };
    if !valid_size(size) {
        return Err(format!(
            "options.size {size}: a HuCard is a multiple of 8 KB that is a power of two, 384 KB or 2.5 MB (at most 4 MB)"
        ));
    }
    if used > size {
        return Err(format!(
            "the program needs {used} bytes (up to file offset ${:X}) but options.size is {size}",
            used - 1
        ));
    }
    let mut image = vec![layout.fill.unwrap_or(0); size];
    for p in &pieces {
        image[p.offset..p.offset + p.bytes.len()].copy_from_slice(&p.bytes);
    }

    // Vectors in the last ten bytes of bank 0.
    let mut notes = Vec::new();
    let vectors = 0x1FF6..0x2000usize;
    let written_by_sources = pieces
        .iter()
        .any(|p| p.offset < vectors.end && p.offset + p.bytes.len() > vectors.start);
    if !written_by_sources {
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
            "no reset vector: define the label `reset` (or `start`), set `entry`, or write the vectors at $FFF6 yourself".to_string()
        })?;
        let reset = u16::try_from(reset)
            .ok()
            .filter(|a| *a >= 0xE000)
            .ok_or_else(|| {
                format!("the reset code at {reset:#X} is not in bank 0 ($E000-$FFFF)")
            })?;
        for (slot, names) in [
            (0usize, ["irq2", "brk"].as_slice()),
            (2, ["irq1"].as_slice()),
            (4, ["timer"].as_slice()),
            (6, ["nmi"].as_slice()),
        ] {
            let target = match find(names) {
                Some(v) => u16::try_from(v)
                    .map_err(|_| format!("vector {} is not a 16-bit address", names[0]))?,
                None => {
                    notes.push(format!(
                        "no label `{}`: the vector at ${:04X} points at the reset code",
                        names[0],
                        0xFFF6 + slot
                    ));
                    reset
                }
            };
            image[0x1FF6 + slot..0x1FF8 + slot].copy_from_slice(&target.to_le_bytes());
        }
        image[0x1FFE..0x2000].copy_from_slice(&reset.to_le_bytes());
    }

    let mut map = String::new();
    for p in &pieces {
        let _ = writeln!(
            map,
            "bank {:3} ${:04X}-${:04X}  file ${:06X}  {}",
            p.bank,
            p.start,
            p.start as usize + p.bytes.len().saturating_sub(1),
            p.offset,
            p.name
        );
    }
    let _ = writeln!(map, "{size} bytes ({} banks)", size / BANK);
    Ok(Build {
        bytes: image,
        notes,
        map,
        symbols_text,
    })
}
