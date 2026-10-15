//! Turning one instruction (a mnemonic and an operand text with expressions in it) into bytes.
//!
//! Both back ends reuse the instruction knowledge of the CPU crates instead of keeping a second table:
//! the 6502 renders the evaluated operand as text for `mos6502::disassemble::assemble_one`; the Z80
//! builds its encoding table once by *enumerating the disassembler* (`z80::disassemble`), so every
//! instruction the debugger can show -- undocumented ones included -- assembles, and the two cannot
//! drift apart.

use crate::expr::{self, EvalError, Scope};
use crate::text::{matching_paren, split_top};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cpu {
    Mos6502,
    Z80,
}

impl Cpu {
    pub fn parse(name: &str) -> Option<Cpu> {
        match name.trim().to_ascii_lowercase().as_str() {
            "6502" | "6510" | "8502" | "mos6502" | "nmos6502" => Some(Cpu::Mos6502),
            "z80" | "z-80" | "zilog-z80" => Some(Cpu::Z80),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Cpu::Mos6502 => "6502",
            Cpu::Z80 => "z80",
        }
    }

    /// Whether `word` (any case) is an instruction of this CPU.
    pub fn is_mnemonic(self, word: &str) -> bool {
        let upper = word.trim_start_matches('*').to_ascii_uppercase();
        match self {
            Cpu::Mos6502 => mos_mnemonics().contains(upper.as_str()),
            Cpu::Z80 => z80_mnemonics().contains(upper.as_str()),
        }
    }
}

/// Where an instruction sits and what it may use.
pub struct Site<'a> {
    pub addr: u16,
    pub scope: &'a dyn Scope,
    /// Final pass: an undefined symbol is an error. First pass: it reads as 0 and is reported back.
    pub strict: bool,
    /// Allow the undocumented 6502 opcodes.
    pub illegal: bool,
    /// The first pass decided this operand had to be 16 bits wide (6502 only).
    pub force_wide: bool,
}

/// What came out of encoding one instruction.
pub struct Encoded {
    pub bytes: Vec<u8>,
    /// Some symbol was not defined yet (first pass only).
    pub unresolved: bool,
}

pub fn encode(cpu: Cpu, mnemonic: &str, operand: &str, site: &Site) -> Result<Encoded, String> {
    match cpu {
        Cpu::Mos6502 => encode_6502(mnemonic, operand, site),
        Cpu::Z80 => encode_z80(mnemonic, operand, site),
    }
}

/// Evaluates operand expressions; in the first pass unknown symbols read as 0 and set `unresolved`.
struct Eval<'a> {
    site: &'a Site<'a>,
    unresolved: Cell<bool>,
}

struct Lenient<'a> {
    inner: &'a dyn Scope,
    flag: &'a Cell<bool>,
}

impl Scope for Lenient<'_> {
    fn symbol(&self, name: &str) -> Option<i64> {
        self.inner.symbol(name).or_else(|| {
            self.flag.set(true);
            Some(0)
        })
    }
    fn pc(&self) -> i64 {
        self.inner.pc()
    }
}

impl<'a> Eval<'a> {
    fn new(site: &'a Site<'a>) -> Self {
        Self {
            site,
            unresolved: Cell::new(false),
        }
    }

    fn value(&self, text: &str) -> Result<i64, String> {
        let expr = expr::parse(text)?;
        let result = if self.site.strict {
            expr.eval(self.site.scope)
        } else {
            expr.eval(&Lenient {
                inner: self.site.scope,
                flag: &self.unresolved,
            })
        };
        result.map_err(|e| match e {
            EvalError::Undefined(name) => format!("undefined symbol \"{name}\""),
            EvalError::Other(m) => m,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// 6502

fn mos_mnemonics() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        (0..=255u8)
            .map(|op| mos6502::disassemble::opcode_info(op).0)
            .collect()
    })
}

fn mos_is_branch(mnemonic: &str) -> bool {
    (0..=255u8).any(|op| {
        let (name, mode, _) = mos6502::disassemble::opcode_info(op);
        name == mnemonic && mode == mos6502::disassemble::Mode::Rel
    })
}

/// A literal written with three or more hex digits (`$0010`, `0x0010`) says "absolute", whatever its
/// value -- the convention of `mos6502::disassemble`, which prints absolute operands that way.
fn is_wide_literal(text: &str) -> bool {
    let t = text.trim();
    let digits = t
        .strip_prefix('$')
        .or_else(|| t.strip_prefix("0x"))
        .or_else(|| t.strip_prefix("0X"));
    digits.is_some_and(|d| d.len() >= 3 && d.chars().all(|c| c.is_ascii_hexdigit()))
}

fn encode_6502(mnemonic: &str, operand: &str, site: &Site) -> Result<Encoded, String> {
    let starred = mnemonic.starts_with('*');
    let mn = mnemonic.trim_start_matches('*').to_ascii_uppercase();
    if !mos_mnemonics().contains(mn.as_str()) {
        return Err(format!("unknown 6502 instruction \"{mnemonic}\""));
    }
    let ev = Eval::new(site);
    let op = operand.trim();
    let wide = |v: i64, force: bool| -> Result<String, String> {
        if !(0..=0xFFFF).contains(&v) {
            return Err(format!("address {v} is outside 0..65535"));
        }
        if v <= 0xFF && !force {
            Ok(format!("${v:02X}"))
        } else {
            Ok(format!("${v:04X}"))
        }
    };
    let zero_page = |v: i64| -> Result<String, String> {
        if !(0..=0xFF).contains(&v) {
            return Err(format!("{v} does not fit in a zero-page byte"));
        }
        Ok(format!("${v:02X}"))
    };
    let text = if op.is_empty() {
        mn.clone()
    } else if op.eq_ignore_ascii_case("A") {
        format!("{mn} A")
    } else if let Some(imm) = op.strip_prefix('#') {
        let v = ev.value(imm)?;
        if !(-128..=255).contains(&v) {
            return Err(format!("immediate value {v} does not fit in a byte"));
        }
        format!("{mn} #${:02X}", v & 0xFF)
    } else {
        // (zp,X) / (zp),Y / (abs), or an address expression with an optional ,X / ,Y.
        let indirect = if op.starts_with('(') {
            matching_paren(op).map(|m| (&op[1..m], op[m + 1..].trim()))
        } else {
            None
        };
        match indirect {
            Some((inner, "")) => {
                let parts = split_top(inner);
                if parts.len() == 2 && parts[1].eq_ignore_ascii_case("X") {
                    format!("{mn} ({},X)", zero_page(ev.value(parts[0])?)?)
                } else if parts.len() == 1 {
                    let v = ev.value(parts[0])?;
                    if !(0..=0xFFFF).contains(&v) {
                        return Err(format!("address {v} is outside 0..65535"));
                    }
                    format!("{mn} (${v:04X})")
                } else {
                    return Err(format!("bad indirect operand \"{op}\""));
                }
            }
            Some((inner, rest)) if rest.eq_ignore_ascii_case(",Y") => {
                format!("{mn} ({}),Y", zero_page(ev.value(inner)?)?)
            }
            _ => {
                let parts = split_top(op);
                let (target, index) = match parts.as_slice() {
                    [t] => (*t, ""),
                    [t, i] if i.eq_ignore_ascii_case("X") => (*t, ",X"),
                    [t, i] if i.eq_ignore_ascii_case("Y") => (*t, ",Y"),
                    _ => return Err(format!("bad operand \"{op}\"")),
                };
                let mut v = ev.value(target)?;
                let is_branch = mos_is_branch(&mn);
                let known = !ev.unresolved.get();
                if is_branch && !known {
                    // First pass, label not defined yet: any in-range placeholder will do.
                    v = site.addr as i64 + 2;
                }
                // A symbol that was unknown in the first pass makes the operand 16 bits wide in both
                // passes, so the instruction keeps its size when the symbol turns out to be small.
                let force = site.force_wide || !known || is_branch || is_wide_literal(target);
                format!("{mn} {}{index}", wide(v, force)?)
            }
        }
    };
    let mut bytes = mos6502::disassemble::assemble_one(&text, site.addr)
        .map_err(|e| format!("{mnemonic} {operand}: {e}"))?;
    if starred {
        // `*` asks for the undocumented encoding of an instruction that also has a documented one.
        let (name, mode, undocumented) = mos6502::disassemble::opcode_info(bytes[0]);
        if !undocumented
            && let Some(twin) = (0..=255u8).find(|&op| {
                let (n, m, u) = mos6502::disassemble::opcode_info(op);
                u && n == name && m == mode
            })
        {
            bytes[0] = twin;
        }
    }
    if mos6502::disassemble::opcode_info(bytes[0]).2 && !site.illegal {
        return Err(format!(
            "{mn} {op} is an undocumented instruction; use --illegal (or .illegal)"
        ));
    }
    Ok(Encoded {
        bytes,
        unresolved: ev.unresolved.get(),
    })
}

// ---------------------------------------------------------------------------------------------
// Z80

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// One immediate byte (also an I/O port).
    N,
    /// A 16-bit value, low byte first.
    Nn,
    /// An index displacement.
    D,
    /// A relative jump target, given as an absolute address.
    E,
}

#[derive(Debug)]
struct Template {
    bytes: Vec<u8>,
    /// In the order the slots appear in the instruction text.
    slots: Vec<(Slot, usize)>,
}

const SCAN_ADDR: u16 = 0x8000;
const FILL: [u8; 3] = [0x11, 0x22, 0x33];

fn z80_table() -> &'static HashMap<String, Template> {
    static TABLE: OnceLock<HashMap<String, Template>> = OnceLock::new();
    TABLE.get_or_init(build_z80_table)
}

fn z80_mnemonics() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| {
        z80_table()
            .keys()
            .filter_map(|k| k.split_whitespace().next())
            .map(|m| &*Box::leak(m.to_string().into_boxed_str()))
            .collect()
    })
}

/// Every instruction the Z80 disassembler can print, as `pattern -> bytes`, canonical encodings first.
fn build_z80_table() -> HashMap<String, Template> {
    let mut candidates: Vec<(Vec<u8>, bool)> = Vec::new();
    for op in 0..=255u8 {
        candidates.push(([&[op][..], &FILL].concat(), false));
    }
    for prefix in [0xCBu8, 0xED, 0xDD, 0xFD] {
        for op in 0..=255u8 {
            candidates.push(([&[prefix, op][..], &FILL].concat(), false));
        }
    }
    // DD CB d op: the (IX+d) forms with op & 7 == 6 are the documented encodings, so they go first.
    for prefix in [0xDDu8, 0xFD] {
        let documented = (0..=255u8).filter(|op| op & 7 == 6);
        let undocumented = (0..=255u8).filter(|op| op & 7 != 6);
        for op in documented.chain(undocumented) {
            candidates.push((vec![prefix, 0xCB, FILL[0], op], true));
        }
    }

    let mut table = HashMap::new();
    for (bytes, indexed_cb) in candidates {
        let (text, len) = z80::disassemble::disassemble_at(&bytes, SCAN_ADDR);
        if text.starts_with("DB ") || len == 0 || len > bytes.len() {
            continue;
        }
        let prefixed_by_index = !indexed_cb && matches!(bytes[0], 0xDD | 0xFD);
        if prefixed_by_index && (text.starts_with("JR") || text.starts_with("DJNZ")) {
            continue; // the prefix is ignored; the disassembler shows the raw displacement
        }
        let mut key = String::new();
        let mut slots: Vec<(Slot, usize)> = Vec::new(); // (slot, tail index)
        let mut rest = text.as_str();
        let target_token = SCAN_ADDR as u32 + 2 + FILL[0] as u32;
        while let Some(at) = rest.find("0x") {
            key.push_str(&rest[..at]);
            let digits = rest[at + 2..]
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .count();
            let value = u32::from_str_radix(&rest[at + 2..at + 2 + digits], 16).unwrap_or(0);
            let after_plus = rest[..at].ends_with('+');
            let nn = ((FILL[1] as u32) << 8) | FILL[0] as u32;
            let slot = if value == nn {
                Some((Slot::Nn, 0))
            } else if value == FILL[0] as u32 {
                Some((if after_plus { Slot::D } else { Slot::N }, 0))
            } else if value == FILL[1] as u32 {
                Some((Slot::N, 1))
            } else if value == target_token {
                Some((Slot::E, 0))
            } else {
                None
            };
            match slot {
                Some((s, tail)) => {
                    key.push_str(match s {
                        Slot::N => "n",
                        Slot::Nn => "nn",
                        Slot::D => "d",
                        Slot::E => "e",
                    });
                    slots.push((s, tail));
                }
                None => key.push_str(&rest[at..at + 2 + digits]),
            }
            rest = &rest[at + 2 + digits..];
        }
        key.push_str(rest);

        let tail_len = if indexed_cb {
            0
        } else {
            slots
                .iter()
                .map(|(s, _)| if *s == Slot::Nn { 2 } else { 1 })
                .sum::<usize>()
        };
        if tail_len > len || (!indexed_cb && bytes[len - tail_len..len] != FILL[..tail_len]) {
            continue;
        }
        let mut template = bytes[..len].to_vec();
        let mut placed = Vec::new();
        for (slot, tail) in slots {
            let at = if indexed_cb { 2 } else { len - tail_len + tail };
            template[at] = 0;
            if slot == Slot::Nn {
                template[at + 1] = 0;
            }
            placed.push((slot, at));
        }
        table.entry(key).or_insert(Template {
            bytes: template,
            slots: placed,
        });
    }
    table
}

const REGISTERS: &[&str] = &[
    "A", "B", "C", "D", "E", "H", "L", "I", "R", "F", "AF", "AF'", "BC", "DE", "HL", "SP", "IX",
    "IY", "IXH", "IXL", "IYH", "IYL", "NZ", "Z", "NC", "PO", "PE", "P", "M",
];

/// One way an operand can be read: its text in the table key and the value it carries, if any.
struct Alt {
    text: String,
    value: Option<(Slot, i64)>,
}

fn z80_alternatives(part: &str, ev: &Eval) -> Result<Vec<Alt>, String> {
    let part = part.trim();
    let upper = part.to_ascii_uppercase();
    if REGISTERS.contains(&upper.as_str()) {
        return Ok(vec![Alt {
            text: upper,
            value: None,
        }]);
    }
    if part.starts_with('(') && matching_paren(part) == Some(part.len() - 1) {
        let inner = part[1..part.len() - 1].trim();
        let iu = inner.to_ascii_uppercase();
        if matches!(iu.as_str(), "HL" | "BC" | "DE" | "SP" | "IX" | "IY" | "C") {
            return Ok(vec![Alt {
                text: format!("({iu})"),
                value: None,
            }]);
        }
        if (iu.starts_with("IX") || iu.starts_with("IY"))
            && iu[2..].trim_start().starts_with(['+', '-'])
        {
            let v = ev.value(&inner[2..])?;
            return Ok(vec![Alt {
                text: format!("({}+d)", &iu[..2]),
                value: Some((Slot::D, v)),
            }]);
        }
        let v = ev.value(inner)?;
        return Ok(vec![
            Alt {
                text: "(nn)".to_string(),
                value: Some((Slot::Nn, v)),
            },
            Alt {
                text: "(n)".to_string(),
                value: Some((Slot::N, v)),
            },
        ]);
    }
    let v = ev.value(part)?;
    Ok(vec![
        Alt {
            text: "n".to_string(),
            value: Some((Slot::N, v)),
        },
        Alt {
            text: "nn".to_string(),
            value: Some((Slot::Nn, v)),
        },
        Alt {
            text: "e".to_string(),
            value: Some((Slot::E, v)),
        },
        // BIT/RES/SET bit numbers and IM modes are printed as plain digits, RST as hex.
        Alt {
            text: format!("{v}"),
            value: None,
        },
        Alt {
            text: format!("0x{v:02X}"),
            value: None,
        },
    ])
}

fn encode_z80(mnemonic: &str, operand: &str, site: &Site) -> Result<Encoded, String> {
    let mut mn = mnemonic.to_ascii_uppercase();
    if mn == "SL1" {
        mn = "SLL".to_string();
    }
    let ev = Eval::new(site);
    let parts = split_top(operand);
    let mut variants: Vec<Vec<&str>> = vec![parts.clone()];
    // Zilog allows leaving out the accumulator for the arithmetic group, and writes IN F,(C) for IN (C).
    match (mn.as_str(), parts.as_slice()) {
        ("ADD" | "ADC" | "SBC", [_]) => {
            let mut v = vec!["A"];
            v.extend(&parts);
            variants.push(v);
        }
        ("SUB" | "AND" | "XOR" | "OR" | "CP", [a, rest @ ..]) if a.eq_ignore_ascii_case("A") => {
            variants.push(rest.to_vec());
        }
        ("IN", [f, c]) if f.eq_ignore_ascii_case("F") => variants.push(vec![*c]),
        _ => {}
    }
    let table = z80_table();
    for variant in &variants {
        let alts: Vec<Vec<Alt>> = variant
            .iter()
            .map(|p| z80_alternatives(p, &ev))
            .collect::<Result<_, _>>()?;
        let mut choice = vec![0usize; alts.len()];
        loop {
            let operands: Vec<&str> = choice
                .iter()
                .zip(&alts)
                .map(|(&i, a)| a[i].text.as_str())
                .collect();
            let key = if operands.is_empty() {
                mn.clone()
            } else {
                format!("{mn} {}", operands.join(","))
            };
            if let Some(template) = table.get(&key) {
                let values: Vec<(Slot, i64)> = choice
                    .iter()
                    .zip(&alts)
                    .filter_map(|(&i, a)| a[i].value)
                    .collect();
                // Slots in the key that no operand supplied a value for (cannot happen: the key's
                // placeholders come only from alternatives that carry one).
                if values.len() == template.slots.len() {
                    return fill_z80(template, &values, site, ev.unresolved.get());
                }
            }
            // Next combination (odometer over the alternatives of each operand).
            let mut i = choice.len();
            let mut wrapped = true;
            while i > 0 {
                i -= 1;
                choice[i] += 1;
                if choice[i] < alts[i].len() {
                    wrapped = false;
                    break;
                }
                choice[i] = 0;
            }
            if wrapped {
                break;
            }
        }
    }
    Err(format!("not a Z80 instruction: {mnemonic} {operand}")
        .trim()
        .to_string())
}

fn fill_z80(
    template: &Template,
    values: &[(Slot, i64)],
    site: &Site,
    unresolved: bool,
) -> Result<Encoded, String> {
    let mut bytes = template.bytes.clone();
    for ((slot, at), (given, v)) in template.slots.iter().zip(values) {
        debug_assert_eq!(slot, given);
        let v = *v;
        match slot {
            Slot::N => {
                if !(-128..=255).contains(&v) {
                    return Err(format!("{v} does not fit in a byte"));
                }
                bytes[*at] = (v & 0xFF) as u8;
            }
            Slot::Nn => {
                if !(-32768..=65535).contains(&v) {
                    return Err(format!("{v} does not fit in 16 bits"));
                }
                let w = (v & 0xFFFF) as u16;
                bytes[*at] = (w & 0xFF) as u8;
                bytes[*at + 1] = (w >> 8) as u8;
            }
            Slot::D => {
                if !(-128..=127).contains(&v) {
                    return Err(format!("displacement {v} is outside -128..127"));
                }
                bytes[*at] = v as i8 as u8;
            }
            Slot::E => {
                let offset = if unresolved && !site.strict {
                    0
                } else {
                    v - (site.addr as i64 + 2)
                };
                if !(-128..=127).contains(&offset) {
                    return Err(format!(
                        "relative jump target ${v:04X} is out of range from ${:04X} (offset {offset})",
                        site.addr
                    ));
                }
                bytes[*at] = offset as i8 as u8;
            }
        }
    }
    Ok(Encoded { bytes, unresolved })
}

#[cfg(test)]
mod z80_table_tests {
    use super::*;

    #[test]
    fn the_table_is_complete_enough_to_hold_every_documented_instruction() {
        let t = z80_table();
        for key in [
            "NOP",
            "LD A,n",
            "LD HL,nn",
            "LD (nn),A",
            "LD A,(IX+d)",
            "LD (IX+d),n",
            "JR e",
            "JR NZ,e",
            "DJNZ e",
            "JP nn",
            "JP (HL)",
            "CALL NZ,nn",
            "RET",
            "RET PO",
            "RST 0x38",
            "IM 1",
            "BIT 3,A",
            "BIT 7,(IX+d)",
            "SET 0,(HL)",
            "RLC (IX+d)",
            "OUT (n),A",
            "OUT (C),A",
            "IN A,(n)",
            "EX AF,AF'",
            "EX (SP),HL",
            "LD A,IXH",
            "LDIR",
            "NEG",
            "ADD A,n",
            "ADD HL,BC",
            "SBC HL,DE",
            "LD (nn),BC",
            "LD BC,(nn)",
            "LD I,A",
            "SLL B",
            "RLC (IX+d),B",
            "PUSH IX",
            "ADD IX,IX",
            "DEC IY",
            "INC (HL)",
            "CP n",
            "SUB B",
        ] {
            assert!(t.contains_key(key), "missing pattern \"{key}\"");
        }
        assert!(t.len() > 1000, "only {} patterns", t.len());
    }

    #[test]
    fn documented_index_bit_instructions_use_the_documented_encoding() {
        let t = z80_table();
        assert_eq!(t["BIT 0,(IX+d)"].bytes, [0xDD, 0xCB, 0x00, 0x46]);
        assert_eq!(t["SET 7,(IY+d)"].bytes, [0xFD, 0xCB, 0x00, 0xFE]);
        assert_eq!(t["RLC (IX+d)"].bytes, [0xDD, 0xCB, 0x00, 0x06]);
        assert_eq!(t["NEG"].bytes, [0xED, 0x44]);
        assert_eq!(t["JR e"].bytes, [0x18, 0x00]);
        assert_eq!(t["LD (nn),A"].bytes, [0x32, 0x00, 0x00]);
    }
}
