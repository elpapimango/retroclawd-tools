//! `rcdis`: a disassembler for the 6502 and the Z80 that writes source `rcasm` assembles back to the
//! same bytes.
//!
//! The instruction decoding is `mos6502::disassemble` / `z80::disassemble`, the same code the debugger
//! shows. Two modes: *linear* (decode every byte from the start) and *flow* (follow jumps, calls and
//! branches from entry points, so data between routines stays data). Labels come from jump targets,
//! memory operands and symbol files; every instruction is checked to assemble back to its bytes, and
//! an encoding the assembler would not reproduce (the six `*NOP`s, a Z80 prefix that does nothing) is
//! written as `.byte` instead.

pub mod m68k;

use machine::SymbolTable;
use rcasm::Cpu;
use rcasm::cpu::{Site, encode};
use rcasm::expr::Scope;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write;

#[derive(Clone, Debug)]
pub struct Options {
    pub cpu: Cpu,
    /// Address of the first byte.
    pub org: u16,
    /// Entry points. Empty means linear mode.
    pub entries: Vec<u16>,
    /// Names for addresses, inside or outside the image.
    pub symbols: SymbolTable,
    /// Inclusive address ranges that are data, never instructions.
    pub data: Vec<(u16, u16)>,
    /// Append `; ADDR: BYTES` to every line.
    pub comments: bool,
}

impl Options {
    pub fn new(cpu: Cpu, org: u16) -> Self {
        Self {
            cpu,
            org,
            entries: Vec::new(),
            symbols: SymbolTable::new(),
            data: Vec::new(),
            comments: false,
        }
    }
}

#[derive(Debug)]
pub struct Disassembly {
    pub source: String,
    pub instructions: usize,
    pub data_bytes: usize,
    pub labels: usize,
}

/// What an instruction does to the flow of control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Flow {
    /// Execution can continue with the next instruction.
    falls_through: bool,
    target: Option<u16>,
}

struct Insn {
    text: String,
    len: usize,
    flow: Flow,
    /// A memory address the instruction reads or writes (for labelling).
    reference: Option<u16>,
}

struct Empty;
impl Scope for Empty {
    fn symbol(&self, _: &str) -> Option<i64> {
        None
    }
    fn pc(&self) -> i64 {
        0
    }
}

fn parse_hex_token(text: &str) -> Option<u16> {
    let t = text.trim();
    let digits = t
        .strip_prefix('$')
        .or_else(|| t.strip_prefix("0x"))
        .or_else(|| t.strip_prefix("0X"))?;
    u16::from_str_radix(digits, 16).ok()
}

fn decode(cpu: Cpu, image: &[u8], at: usize, addr: u16) -> Option<Insn> {
    let window = &image[at..image.len().min(at + 4)];
    let (text, len) = match cpu {
        Cpu::Mos6502 => mos6502::disassemble::disassemble(window, addr),
        Cpu::Z80 => z80::disassemble::disassemble_at(window, addr),
    };
    if len == 0 || at + len > image.len() {
        return None;
    }
    // The decoder reads missing bytes as zero; an instruction that needs bytes the image does not
    // have is not decoded.
    let (flow, reference) = match cpu {
        Cpu::Mos6502 => flow_6502(&text),
        Cpu::Z80 => flow_z80(&text),
    };
    Some(Insn {
        text,
        len,
        flow,
        reference,
    })
}

fn flow_6502(text: &str) -> (Flow, Option<u16>) {
    let t = text.trim_start_matches('*');
    let (mnemonic, operand) = t.split_once(' ').unwrap_or((t, ""));
    let target = parse_hex_token(operand);
    let go = |falls_through, target| Flow {
        falls_through,
        target,
    };
    match mnemonic {
        "JMP" => (go(false, target), None),
        "JSR" => (go(true, target), None),
        "BCC" | "BCS" | "BEQ" | "BMI" | "BNE" | "BPL" | "BVC" | "BVS" => (go(true, target), None),
        "RTS" | "RTI" | "BRK" | "JAM" => (go(false, None), None),
        _ => {
            // A memory operand: four hex digits, not an immediate.
            let reference = if operand.starts_with('#') {
                None
            } else {
                operand
                    .trim_start_matches('(')
                    .split([',', ')'])
                    .next()
                    .filter(|o| o.len() == 5 && o.starts_with('$'))
                    .and_then(parse_hex_token)
            };
            (go(true, None), reference)
        }
    }
}

fn flow_z80(text: &str) -> (Flow, Option<u16>) {
    let (mnemonic, operand) = text.split_once(' ').unwrap_or((text, ""));
    let last = operand.rsplit(',').next().unwrap_or("");
    let conditional = operand.contains(',');
    let go = |falls_through, target| Flow {
        falls_through,
        target,
    };
    match mnemonic {
        "JP" | "JR" if operand.starts_with('(') => (go(false, None), None),
        "JP" | "JR" => (go(conditional, parse_hex_token(last)), None),
        "DJNZ" | "CALL" => (go(true, parse_hex_token(last)), None),
        "RST" => (go(true, parse_hex_token(operand)), None),
        "RET" => (go(!operand.is_empty(), None), None),
        "RETI" | "RETN" => (go(false, None), None),
        _ => {
            // `(0xNNNN)` memory operands.
            let reference = operand
                .split(['(', ')', ','])
                .find(|p| p.len() == 6 && p.starts_with("0x"))
                .and_then(parse_hex_token)
                .filter(|_| operand.contains('('));
            (go(true, None), reference)
        }
    }
}

/// Whether the assembler turns `text` back into exactly `bytes`.
fn assembles_back(cpu: Cpu, text: &str, addr: u16, bytes: &[u8]) -> bool {
    let (mnemonic, operand) = text.split_once(' ').unwrap_or((text, ""));
    let site = Site {
        addr,
        scope: &Empty,
        strict: true,
        illegal: true,
        force_wide: false,
    };
    encode(cpu, mnemonic, operand, &site).is_ok_and(|e| e.bytes == bytes)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cell {
    Unknown,
    /// Start of an instruction, with its length.
    Code(usize),
    /// Inside an instruction.
    Inside,
    Data,
}

/// Disassembles `image`, which sits at `opts.org`.
pub fn disassemble(image: &[u8], opts: &Options) -> Result<Disassembly, String> {
    let org = opts.org as usize;
    let end = org + image.len();
    if end > 0x10000 {
        return Err(format!(
            "{} bytes at ${:04X} run past $FFFF",
            image.len(),
            opts.org
        ));
    }
    let in_image = |a: u16| (opts.org as usize..end).contains(&(a as usize));
    let forced_data = |a: u16| opts.data.iter().any(|(s, e)| (*s..=*e).contains(&a));

    // 1. Which bytes are instructions.
    let mut cells = vec![Cell::Unknown; image.len()];
    let mut insns: BTreeMap<u16, Insn> = BTreeMap::new();
    let mut take = |addr: u16, cells: &mut Vec<Cell>| -> Option<Flow> {
        let at = addr as usize - org;
        match cells[at] {
            Cell::Code(_) => return None,
            Cell::Inside | Cell::Data => return None,
            Cell::Unknown => {}
        }
        if forced_data(addr) {
            return None;
        }
        let insn = decode(opts.cpu, image, at, addr)?;
        // The whole instruction must be free and not forced data.
        if (0..insn.len)
            .any(|i| cells[at + i] != Cell::Unknown || forced_data(addr.wrapping_add(i as u16)))
        {
            return None;
        }
        if !assembles_back(opts.cpu, &insn.text, addr, &image[at..at + insn.len]) {
            // Keep the bytes as data: the assembler could not write this encoding back.
            return None;
        }
        cells[at] = Cell::Code(insn.len);
        for c in &mut cells[at + 1..at + insn.len] {
            *c = Cell::Inside;
        }
        let flow = insn.flow;
        insns.insert(addr, insn);
        Some(flow)
    };

    let mut targets: BTreeSet<u16> = BTreeSet::new();
    if opts.entries.is_empty() {
        let mut addr = org;
        while addr < end {
            match take(addr as u16, &mut cells) {
                Some(_) => {
                    let len = match cells[addr - org] {
                        Cell::Code(l) => l,
                        _ => 1,
                    };
                    addr += len;
                }
                None => {
                    cells[addr - org] = Cell::Data;
                    addr += 1;
                }
            }
        }
        for insn in insns.values() {
            if let Some(t) = insn.flow.target.filter(|t| in_image(*t)) {
                targets.insert(t);
            }
        }
    } else {
        let mut work: VecDeque<u16> = VecDeque::new();
        for &e in &opts.entries {
            if !in_image(e) {
                return Err(format!(
                    "entry ${e:04X} is outside the image (${:04X}-${:04X})",
                    opts.org,
                    end - 1
                ));
            }
            targets.insert(e);
            work.push_back(e);
        }
        while let Some(start) = work.pop_front() {
            let mut addr = start;
            loop {
                if !in_image(addr) {
                    break;
                }
                let Some(flow) = take(addr, &mut cells) else {
                    break;
                };
                if let Some(t) = flow.target.filter(|t| in_image(*t)) {
                    targets.insert(t);
                    work.push_back(t);
                }
                if !flow.falls_through {
                    break;
                }
                let len = match cells[addr as usize - org] {
                    Cell::Code(l) => l,
                    _ => 1,
                };
                addr = addr.wrapping_add(len as u16);
                if (addr as usize) < org {
                    break;
                }
            }
        }
        for c in &mut cells {
            if *c == Cell::Unknown {
                *c = Cell::Data;
            }
        }
    }
    for insn in insns.values() {
        if let Some(r) = insn.reference.filter(|r| in_image(*r)) {
            targets.insert(r);
        }
    }

    // 2. Names. A label sits only where an instruction starts or data lies.
    let mut labels: BTreeMap<u16, String> = BTreeMap::new();
    let placeable = |a: u16| !matches!(cells[a as usize - org], Cell::Inside);
    for &t in &targets {
        if in_image(t) && placeable(t) {
            let name = match opts.symbols.name(t) {
                Some(n) => n.to_string(),
                None if matches!(cells[t as usize - org], Cell::Code(_)) => format!("L_{t:04X}"),
                None => format!("D_{t:04X}"),
            };
            labels.insert(t, name);
        }
    }
    for a in opts.org as usize..end {
        if let Some(n) = opts.symbols.name(a as u16)
            && placeable(a as u16)
        {
            labels.entry(a as u16).or_insert_with(|| n.to_string());
        }
    }

    // 3. Text.
    let mut externals: BTreeMap<u16, String> = BTreeMap::new();
    let mut body = String::new();
    let mut instructions = 0;
    let mut data_bytes = 0;
    let mut illegal = false;
    let mut addr = org;
    let comment = |body: &mut String, at: usize, len: usize| {
        if opts.comments {
            let bytes: Vec<String> = image[at..at + len]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect();
            let _ = write!(body, "  ; {:04X}: {}", org + at, bytes.join(" "));
        }
    };
    while addr < end {
        let a16 = addr as u16;
        if let Some(name) = labels.get(&a16) {
            let _ = writeln!(body, "{name}:");
        }
        match cells[addr - org] {
            Cell::Code(len) => {
                let insn = &insns[&a16];
                if insn.text.starts_with('*') {
                    illegal = true;
                }
                let text = symbolize(&insn.text, opts, &labels, &in_image, &mut externals);
                let _ = write!(body, "        {text}");
                comment(&mut body, addr - org, len);
                body.push('\n');
                instructions += 1;
                addr += len;
            }
            _ => {
                // A run of data up to eight bytes, ending at a label or the next instruction.
                let mut n = 1;
                while addr + n < end
                    && n < 8
                    && !matches!(cells[addr + n - org], Cell::Code(_))
                    && !labels.contains_key(&((addr + n) as u16))
                {
                    n += 1;
                }
                let list: Vec<String> = image[addr - org..addr - org + n]
                    .iter()
                    .map(|b| format!("${b:02X}"))
                    .collect();
                let _ = write!(body, "        .byte {}", list.join(","));
                comment(&mut body, addr - org, n);
                body.push('\n');
                data_bytes += n;
                addr += n;
            }
        }
    }

    // 4. Header.
    let mut out = String::new();
    let _ = writeln!(
        out,
        "; rcdis: {} bytes at ${:04X}-${:04X}, {}",
        image.len(),
        opts.org,
        end.saturating_sub(1),
        if opts.entries.is_empty() {
            "linear".to_string()
        } else {
            let list: Vec<String> = opts.entries.iter().map(|e| format!("${e:04X}")).collect();
            format!("following flow from {}", list.join(", "))
        }
    );
    let _ = writeln!(out, ".cpu {}", opts.cpu.name());
    if illegal {
        let _ = writeln!(out, ".illegal");
    }
    for (a, name) in &externals {
        let _ = writeln!(out, "{name} = ${a:04X}");
    }
    let _ = writeln!(out, "        .org ${:04X}", opts.org);
    out.push_str(&body);
    Ok(Disassembly {
        source: out,
        instructions,
        data_bytes,
        labels: labels.len(),
    })
}

/// Replaces addresses in an instruction by label and symbol names where that keeps the encoding.
fn symbolize(
    text: &str,
    opts: &Options,
    labels: &BTreeMap<u16, String>,
    in_image: &dyn Fn(u16) -> bool,
    externals: &mut BTreeMap<u16, String>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let (prefix_len, is_hex) = if c == '$' {
            (1, true)
        } else if c == '0' && matches!(chars.get(i + 1), Some('x' | 'X')) {
            (2, true)
        } else {
            (0, false)
        };
        if is_hex && prefix_len > 0 {
            let digits: String = chars[i + prefix_len..]
                .iter()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect();
            let after = chars.get(i + prefix_len + digits.len());
            let immediate = i > 0 && chars[i - 1] == '#';
            let word_edge = i == 0 || !chars[i - 1].is_ascii_alphanumeric();
            if word_edge && after.is_none_or(|c| !c.is_ascii_alphanumeric()) {
                let value = u16::from_str_radix(&digits, 16).ok();
                let replacement = value.and_then(|v| match digits.len() {
                    4 => {
                        // Only the 6502 says "absolute" by writing a small value with four digits;
                        // naming that value could shrink it to zero page.
                        let would_shrink = opts.cpu == Cpu::Mos6502 && v <= 0xFF;
                        if would_shrink {
                            None
                        } else if let Some(n) = labels.get(&v) {
                            Some(n.clone())
                        } else if !in_image(v) {
                            opts.symbols.name(v).map(|n| {
                                externals.insert(v, n.to_string());
                                n.to_string()
                            })
                        } else {
                            None
                        }
                    }
                    2 if opts.cpu == Cpu::Mos6502 && !immediate => opts.symbols.name(v).map(|n| {
                        externals.insert(v, n.to_string());
                        n.to_string()
                    }),
                    _ => None,
                });
                if let Some(name) = replacement {
                    out.push_str(&name);
                    i += prefix_len + digits.len();
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}
