//! `rcld`: place assembled programs in memory and emit a runnable image.
//!
//! There are no object files. A layout file ([`layout`]) names the inputs (assembler sources that are assembled
//! together, raw binaries at fixed addresses, `.prg` files, symbol files), the regions they may occupy and what
//! the machine and the output are. [`build`] checks that nothing overlaps or leaves its region and then writes
//! the image the output's extension asks for: a `.prg`, a disk, a cartridge, a tape or a snapshot.

pub mod amiga;
pub mod atari;
pub mod basic;
pub mod cfg;
pub mod emit;
pub mod genesis;
pub mod human68k;
pub mod layout;
pub mod link;
pub mod m68klink;
pub mod neogeo;
pub mod nes;
pub mod pcengine;
pub mod place;
pub mod segacd;
pub mod snes;
pub mod templates;
pub mod x68000;
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
        (Machine::PcEngine, _) => Ok(rcasm::Cpu::Huc6280),
        (Machine::Nes, _) => Ok(rcasm::Cpu::Mos6502),
        (Machine::Snes, _) => Ok(rcasm::Cpu::W65816),
        (
            Machine::Amiga | Machine::X68000 | Machine::Genesis | Machine::NeoGeo | Machine::SegaCd,
            _,
        ) => Err(
            "the Amiga and the X68000 are 68000 machines: they are built by their own modules"
                .to_string(),
        ),
        (Machine::AtariSt, _) => {
            Err("the Atari ST is a 68000: it is built by `rcld::atari`".to_string())
        }
        (Machine::Generic, Some("6502" | "6510" | "8502")) => Ok(rcasm::Cpu::Mos6502),
        (Machine::Generic, Some("z80")) => Ok(rcasm::Cpu::Z80),
        (Machine::Generic, Some("huc6280")) => Ok(rcasm::Cpu::Huc6280),
        (Machine::Generic, Some(other)) => Err(format!("cpu \"{other}\": 6502 or z80")),
        (Machine::Generic, None) => {
            Err("machine = \"generic\" needs cpu = \"6502\" or \"z80\"".to_string())
        }
    }
}

/// The layouts of the 68000 machines: their sources are assembled by the 68000 back end.
pub(crate) fn is_68k(layout: &Layout) -> bool {
    matches!(
        layout.machine,
        Machine::AtariSt
            | Machine::Amiga
            | Machine::X68000
            | Machine::Genesis
            | Machine::NeoGeo
            | Machine::SegaCd
    ) || (layout.machine == Machine::Generic
        && matches!(layout.cpu.as_deref(), Some("68000" | "m68000" | "68k")))
}

/// The inputs that go through the linker (`src`, `obj`, `lib`), in order.
pub(crate) fn linked_items(layout: &Layout, base: &Path) -> Vec<Item> {
    let mut items = Vec::new();
    for input in &layout.input {
        for (list, make) in [
            (&input.src, Item::Src as fn(PathBuf) -> Item),
            (&input.obj, Item::Obj as fn(PathBuf) -> Item),
            (&input.lib, Item::Lib as fn(PathBuf) -> Item),
        ] {
            if let Some(list) = list {
                items.extend(list.list().into_iter().map(|p| make(resolve(base, &p))));
            }
        }
    }
    items
}

/// An input that goes through the linker.
pub(crate) enum Item {
    Src(PathBuf),
    Obj(PathBuf),
    Lib(PathBuf),
}

/// Assembles the `src` files to objects, reads the `obj` and `lib` files and links them by the layout's rules.
pub(crate) fn link_inputs(
    layout: &Layout,
    base: &Path,
    items: &[Item],
    defines: &[(String, i64)],
    options: link::LinkOptions,
    extra_rules: &[link::Rule],
) -> Result<link::Linked, String> {
    let m68k = is_68k(layout);
    let cpu = if m68k { None } else { Some(cpu_of(layout)?) };
    let mut objects = Vec::new();
    let mut libs = Vec::new();
    for item in items {
        match item {
            Item::Src(path) => {
                let text = std::fs::read(path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                let text = String::from_utf8_lossy(&text).into_owned();
                let name = path.to_string_lossy().into_owned();
                let object = if m68k {
                    let opts = rcasm::m68k::M68kOptions {
                        defines: defines.to_vec(),
                        include_dirs: vec![base.to_path_buf()],
                        ..rcasm::m68k::M68kOptions::default()
                    };
                    rcasm::m68k::assemble_m68k_object(&name, &text, &opts, &rcasm::DiskFiles)
                } else {
                    let opts = rcasm::Options {
                        cpu,
                        defines: defines.to_vec(),
                        include_dirs: vec![base.to_path_buf()],
                        ..rcasm::Options::default()
                    };
                    rcasm::assemble_object(&name, &text, &opts, &rcasm::DiskFiles)
                }
                .map_err(|diags| {
                    diags
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join(
                            "
",
                        )
                })?;
                objects.push(link::LinkObject { file: name, object });
            }
            Item::Obj(path) => {
                let bytes = std::fs::read(path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                let object = rc_common::obj::Object::decode(&bytes)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                objects.push(link::LinkObject {
                    file: path.to_string_lossy().into_owned(),
                    object,
                });
            }
            Item::Lib(path) => {
                let bytes = std::fs::read(path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                let lib = rc_common::obj::Archive::decode(&bytes)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                libs.push((path.to_string_lossy().into_owned(), lib));
            }
        }
    }
    let mut rules = Vec::new();
    for r in &layout.segment {
        rules.push(link::Rule {
            name: r.name.clone(),
            region: r.region.clone(),
            start: match &r.start {
                Some(v) => {
                    Some(layout::address(v, &format!("segment \"{}\" start", r.name))? as u32)
                }
                None => None,
            },
            align: r.align,
        });
    }
    let regions: Vec<link::RegionSpec> = place::regions(layout)?
        .into_iter()
        .map(|r| link::RegionSpec {
            name: r.name,
            start: r.start as u32,
            end: r.end as u32,
        })
        .collect();
    rules.extend(extra_rules.iter().cloned());
    link::link_with(objects, &libs, &rules, &regions, options).map_err(|e| {
        e.join(
            "
",
        )
    })
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
    let mut linked: Vec<Item> = Vec::new();
    for (n, input) in layout.input.iter().enumerate() {
        let kinds = [
            input.asm.is_some(),
            input.bin.is_some(),
            input.prg.is_some(),
            input.symbols.is_some(),
            input.src.is_some(),
            input.obj.is_some(),
            input.lib.is_some(),
        ];
        if kinds.iter().filter(|&&k| k).count() != 1 {
            return Err(format!(
                "input {}: give exactly one of asm, src, obj, lib, bin, prg or symbols",
                n + 1
            ));
        }
        if let Some(list) = &input.src {
            linked.extend(
                list.list()
                    .into_iter()
                    .map(|p| Item::Src(resolve(base, &p))),
            );
            continue;
        }
        if let Some(list) = &input.obj {
            linked.extend(
                list.list()
                    .into_iter()
                    .map(|p| Item::Obj(resolve(base, &p))),
            );
            continue;
        }
        if let Some(list) = &input.lib {
            linked.extend(
                list.list()
                    .into_iter()
                    .map(|p| Item::Lib(resolve(base, &p))),
            );
            continue;
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
    if !linked.is_empty() {
        let l = link_inputs(
            layout,
            base,
            &linked,
            &defines,
            link::LinkOptions::default(),
            &[],
        )?;
        pieces.extend(l.pieces);
        symbols.extend(l.symbols);
        symbols_text.push_str(&l.symbols_text);
        notes.extend(l.notes);
    }
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
                start: s.start as u16,
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
    if let Some(path) = &layout.cfg {
        let full = resolve(base, path);
        let text = std::fs::read_to_string(&full)
            .map_err(|e| format!("cannot read {}: {e}", full.display()))?;
        let cfg = cfg::parse(&text).map_err(|e| format!("{}: {e}", full.display()))?;
        let mut merged = layout.clone();
        merged.cfg = None;
        merged.region.extend(cfg.regions);
        merged.segment.extend(cfg.segments);
        return build(&merged, base, output, cli_defines);
    }
    if layout.machine == Machine::AtariSt {
        return atari::build(layout, base, output, cli_defines);
    }
    if layout.machine == Machine::PcEngine {
        return pcengine::build(layout, base, output, cli_defines);
    }
    if layout.machine == Machine::SegaCd {
        return segacd::build(layout, base, output, cli_defines);
    }
    if layout.machine == Machine::NeoGeo {
        return neogeo::build(layout, base, output, cli_defines);
    }
    if layout.machine == Machine::Genesis {
        return genesis::build(layout, base, output, cli_defines);
    }
    if layout.machine == Machine::Amiga {
        return amiga::build(layout, base, output, cli_defines);
    }
    if layout.machine == Machine::X68000 {
        return x68000::build(layout, base, output, cli_defines);
    }
    if layout.machine == Machine::Nes {
        return nes::build(layout, base, output, cli_defines);
    }
    if layout.machine == Machine::Snes {
        return snes::build(layout, base, output, cli_defines);
    }
    let program = load(layout, base, cli_defines)?;
    emit::emit(layout, base, output, program)
}
