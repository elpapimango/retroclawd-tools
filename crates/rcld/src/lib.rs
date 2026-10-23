//! `rcld`: place assembled programs in memory and emit a runnable image.
//!
//! There are no object files. A layout file ([`layout`]) names the inputs (assembler sources that are assembled
//! together, raw binaries at fixed addresses, `.prg` files, symbol files), the regions they may occupy and what
//! the machine and the output are. [`build`] checks that nothing overlaps or leaves its region and then writes
//! the image the output's extension asks for: a `.prg`, a disk, a cartridge, a tape or a snapshot.

pub mod atari;
pub mod basic;
pub mod emit;
pub mod layout;
pub mod place;
pub mod templates;
pub mod xex;

use layout::{Layout, Machine, Value};
use place::Piece;
use std::path::{Path, PathBuf};

/// What the inputs add up to, before the output format puts its own pieces (a loader, vectors) in.
#[derive(Debug)]
pub struct Program {
    pub pieces: Vec<Piece>,
    /// Every symbol the sources and symbol files define.
    pub symbols: Vec<(String, i64)>,
    /// Labels of the assembled sources, in the debugger's symbol file format.
    pub symbols_text: String,
    pub notes: Vec<String>,
}

/// A finished image and what to say about it.
#[derive(Debug)]
pub struct Build {
    pub bytes: Vec<u8>,
    pub notes: Vec<String>,
    pub map: String,
    pub symbols_text: String,
}

fn resolve(base: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn read(base: &Path, p: &str) -> Result<Vec<u8>, String> {
    let path = resolve(base, p);
    std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Symbol files: `NAME = $1234` (or `equ`, or `NAME 1234` in hex) and VICE's `al C:1234 .NAME`.
pub fn parse_symbols(text: &str) -> Result<Vec<(String, i64)>, String> {
    let mut out = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split(';').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let bad = || format!("line {}: cannot read a symbol from \"{line}\"", n + 1);
        let words: Vec<&str> = line.split_whitespace().collect();
        let (name, value) = if words[0] == "al" && words.len() >= 3 {
            let addr = words[1].rsplit(':').next().unwrap_or(words[1]);
            (
                words[2].trim_start_matches('.'),
                i64::from_str_radix(addr, 16).map_err(|_| bad())?,
            )
        } else if words.len() >= 3 && (words[1] == "=" || words[1].eq_ignore_ascii_case("equ")) {
            (
                words[0],
                rc_common::parse_number(&words[2..].join("")).map_err(|_| bad())?,
            )
        } else if words.len() == 2 {
            (
                words[0],
                i64::from_str_radix(words[1], 16).map_err(|_| bad())?,
            )
        } else {
            return Err(bad());
        };
        out.push((name.to_string(), value));
    }
    Ok(out)
}

fn cpu_of(layout: &Layout) -> Result<rcasm::Cpu, String> {
    match (layout.machine, layout.cpu.as_deref()) {
        (Machine::C64 | Machine::Atari, _) => Ok(rcasm::Cpu::Mos6502),
        (Machine::Spectrum | Machine::Cpc, _) => Ok(rcasm::Cpu::Z80),
        (Machine::AtariSt, _) => {
            Err("the Atari ST is a 68000: it is built by `rcld::atari`".to_string())
        }
        (Machine::Generic, Some("6502" | "6510" | "8502")) => Ok(rcasm::Cpu::Mos6502),
        (Machine::Generic, Some("z80")) => Ok(rcasm::Cpu::Z80),
        (Machine::Generic, Some(other)) => Err(format!("cpu \"{other}\": 6502 or z80")),
        (Machine::Generic, None) => {
            Err("machine = \"generic\" needs cpu = \"6502\" or \"z80\"".to_string())
        }
    }
}

/// Reads and assembles the inputs.
pub fn load(
    layout: &Layout,
    base: &Path,
    cli_defines: &[(String, i64)],
) -> Result<Program, String> {
    let mut defines: Vec<(String, i64)> =
        layout.define.iter().map(|(k, v)| (k.clone(), *v)).collect();
    let mut pieces = Vec::new();
    let mut symbols: Vec<(String, i64)> = Vec::new();
    let mut symbols_text = String::new();
    let mut sources: Vec<PathBuf> = Vec::new();
    for (n, input) in layout.input.iter().enumerate() {
        let kinds = [
            input.asm.is_some(),
            input.bin.is_some(),
            input.prg.is_some(),
            input.symbols.is_some(),
        ];
        if kinds.iter().filter(|&&k| k).count() != 1 {
            return Err(format!(
                "input {}: give exactly one of asm, bin, prg or symbols",
                n + 1
            ));
        }
        if input.at.is_some() && input.bin.is_none() {
            return Err(format!("input {}: \"at\" goes with \"bin\"", n + 1));
        }
        if let Some(asm) = &input.asm {
            sources.extend(asm.list().iter().map(|p| resolve(base, p)));
        } else if let Some(bin) = &input.bin {
            let at = input.at.as_ref().ok_or_else(|| {
                format!("input {}: bin \"{bin}\" needs \"at\" (its address)", n + 1)
            })?;
            let start = layout::address(at, &format!("input {} at", n + 1))?;
            let bytes = read(base, bin)?;
            if start as usize + bytes.len() > 0x10000 {
                return Err(format!(
                    "{bin}: {} bytes at ${start:04X} run past the end of memory",
                    bytes.len()
                ));
            }
            pieces.push(Piece {
                name: bin.clone(),
                start,
                bytes,
            });
        } else if let Some(prg) = &input.prg {
            let bytes = read(base, prg)?;
            if bytes.len() < 3 {
                return Err(format!(
                    "{prg}: a .prg needs the 2-byte load address and at least one byte"
                ));
            }
            let start = u16::from_le_bytes([bytes[0], bytes[1]]);
            if start as usize + bytes.len() - 2 > 0x10000 {
                return Err(format!(
                    "{prg}: {} bytes at ${start:04X} run past the end of memory",
                    bytes.len() - 2
                ));
            }
            pieces.push(Piece {
                name: prg.clone(),
                start,
                bytes: bytes[2..].to_vec(),
            });
        } else if let Some(sym) = &input.symbols {
            let text =
                String::from_utf8(read(base, sym)?).map_err(|_| format!("{sym}: not text"))?;
            let list = parse_symbols(&text).map_err(|e| format!("{sym}: {e}"))?;
            defines.extend(list.iter().cloned());
            symbols.extend(list);
        }
    }
    defines.extend(cli_defines.iter().cloned());
    let mut notes = Vec::new();
    if !sources.is_empty() {
        // A Windows path has backslashes, which are escapes in an assembler string.
        let quote = |p: &Path| {
            p.display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        };
        let source: String = sources
            .iter()
            .map(|p| format!(".include \"{}\"\n", quote(p)))
            .collect();
        let opts = rcasm::Options {
            cpu: Some(cpu_of(layout)?),
            defines,
            include_dirs: vec![base.to_path_buf()],
            ..rcasm::Options::default()
        };
        let asm =
            rcasm::assemble("layout", &source, &opts, &rcasm::DiskFiles).map_err(|diags| {
                diags
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            })?;
        for s in &asm.segments {
            pieces.push(Piece {
                name: "assembled code".to_string(),
                start: s.start,
                bytes: s.bytes.clone(),
            });
        }
        symbols.extend(asm.symbols.iter().map(|s| (s.name.clone(), s.value)));
        symbols_text = rcasm::symbols_text(&asm, false, rcasm::SymbolStyle::Rc);
    } else if layout.input.iter().all(|i| i.symbols.is_some()) {
        notes.push("no program inputs".to_string());
    }
    Ok(Program {
        pieces,
        symbols,
        symbols_text,
        notes,
    })
}

/// The entry address: a number, or a symbol of the sources or symbol files. `None` when the layout names none.
pub fn entry(layout: &Layout, program: &Program) -> Result<Option<(u16, String)>, String> {
    let Some(v) = &layout.entry else {
        return Ok(None);
    };
    let (n, how) = match v {
        Value::Int(i) => (*i, format!("{i:#X}")),
        Value::Text(t) => match rc_common::parse_number(t) {
            Ok(n) => (n, t.clone()),
            Err(_) => {
                let found = program
                    .symbols
                    .iter()
                    .find(|(name, _)| name == t)
                    .map(|(_, v)| *v);
                (
                    found.ok_or_else(|| format!("entry \"{t}\" is not a symbol of the inputs"))?,
                    t.clone(),
                )
            }
        },
    };
    u16::try_from(n)
        .map(|a| Some((a, how)))
        .map_err(|_| format!("entry {n:#X} does not fit in 16 bits"))
}

/// Links: loads the inputs, checks them, and writes the image `output`'s extension asks for. `base` is the
/// directory relative paths in the layout are taken from.
pub fn build(
    layout: &Layout,
    base: &Path,
    output: &Path,
    cli_defines: &[(String, i64)],
) -> Result<Build, String> {
    if layout.machine == Machine::AtariSt {
        return atari::build(layout, base, output, cli_defines);
    }
    let program = load(layout, base, cli_defines)?;
    emit::emit(layout, base, output, program)
}
