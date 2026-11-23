//! The HuC6280 (PC Engine): a 65C02 with the Rockwell bit instructions, block transfers, `TST`, `ST0`-`ST2`,
//! the register swaps and the mapper instructions.
//!
//! Like the Z80 back end, the encoding table is built once by *enumerating the disassembler*
//! (`huc6280::disassemble`), so every instruction the debugger shows assembles and the two cannot drift apart.
//! Operands are matched by shape: `Z` a zero-page byte, `W` a 16-bit address, `N` an immediate byte, `R` a
//! relative target written as an address.

use crate::cpu::{Encoded, Eval, Site};
use crate::text::{matching_paren, split_top};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// An immediate byte or a zero-page byte.
    Byte,
    /// A 16-bit value, low byte first.
    Word,
    /// A relative target, given as an absolute address.
    Rel,
}

#[derive(Debug)]
struct Template {
    opcode: u8,
    /// Slots in operand order with the byte index each fills.
    slots: Vec<(Slot, usize)>,
    len: usize,
}

const FILL: [u8; 6] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];

fn is_relative(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "BPL" | "BMI" | "BVC" | "BVS" | "BCC" | "BCS" | "BNE" | "BEQ" | "BRA" | "BSR"
    )
}

fn is_bit_branch(mnemonic: &str) -> bool {
    (mnemonic.starts_with("BBR") || mnemonic.starts_with("BBS")) && mnemonic.len() == 4
}

/// `LDA $11,X` -> (`LDA Z,X`, slots); numbers in the text become shape letters.
fn shape_of(text: &str) -> (String, Vec<(Slot, usize)>) {
    let (mnemonic, rest) = text.split_once(' ').unwrap_or((text, ""));
    let mut shape = String::from(mnemonic);
    let mut slots = Vec::new();
    if rest.is_empty() {
        return (shape, slots);
    }
    shape.push(' ');
    let rel_here = is_relative(mnemonic);
    let bit_branch = is_bit_branch(mnemonic);
    let mut index = 1usize;
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    let mut operand_no = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '$' {
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && chars[end].is_ascii_hexdigit() {
                end += 1;
            }
            let digits = end - start;
            let after_hash = i > 0 && chars[i - 1] == '#';
            if after_hash {
                shape.push('N');
                slots.push((Slot::Byte, index));
                index += 1;
            } else if rel_here || (bit_branch && operand_no == 1) {
                shape.push('R');
                slots.push((Slot::Rel, index));
                index += 1;
            } else if digits == 2 {
                shape.push('Z');
                slots.push((Slot::Byte, index));
                index += 1;
            } else {
                shape.push('W');
                slots.push((Slot::Word, index));
                index += 2;
            }
            i = end;
            operand_no += 1;
            continue;
        }
        shape.push(c);
        i += 1;
    }
    (shape, slots)
}

fn table() -> &'static HashMap<String, Template> {
    static TABLE: OnceLock<HashMap<String, Template>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = HashMap::new();
        for op in 0..=255u8 {
            let bytes = [&[op][..], &FILL].concat();
            let (text, len) = huc6280::disassemble::disassemble(&bytes, 0x8000);
            if len == 0
                || text.starts_with("???")
                || text.starts_with("DB")
                || text.starts_with(".DB")
            {
                continue;
            }
            let (shape, slots) = shape_of(&text);
            table.entry(shape).or_insert(Template {
                opcode: op,
                slots,
                len,
            });
        }
        // The undefined opcodes act as NOP and print as such; the documented one is $EA.
        if let Some(t) = table.get_mut("NOP") {
            t.opcode = 0xEA;
        }
        table
    })
}

fn mnemonics() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        table()
            .keys()
            .filter_map(|k| k.split_whitespace().next())
            .map(str::to_string)
            .collect()
    })
}

pub fn is_mnemonic(word: &str) -> bool {
    mnemonics().contains(&word.to_ascii_uppercase())
}

/// One written operand: its shape letter and value expression, if it has one.
struct Part {
    /// Text of the expression.
    expr: String,
    /// `N` immediate, `A` an address that may be zero page or absolute, `R` relative.
    kind: char,
}

pub fn encode(mnemonic: &str, operand: &str, site: &Site) -> Result<Encoded, String> {
    let mn = mnemonic.to_ascii_uppercase();
    if !is_mnemonic(&mn) {
        return Err(format!("unknown HuC6280 instruction \"{mnemonic}\""));
    }
    let ev = Eval::new(site);
    let op = operand.trim();
    // Shape template with `{}` where an address goes, and the written parts in order.
    let mut parts: Vec<Part> = Vec::new();
    let template: String = if op.is_empty() || op.eq_ignore_ascii_case("A") {
        mn.clone()
    } else if is_relative(&mn) {
        parts.push(Part {
            expr: op.to_string(),
            kind: 'R',
        });
        format!("{mn} R")
    } else if is_bit_branch(&mn) {
        let p = split_top(op);
        if p.len() != 2 {
            return Err(format!("{mn} takes a zero-page address and a target"));
        }
        parts.push(Part {
            expr: p[0].to_string(),
            kind: 'Z',
        });
        parts.push(Part {
            expr: p[1].to_string(),
            kind: 'R',
        });
        format!("{mn} Z,R")
    } else if let Some(rest) = op.strip_prefix('#') {
        let p = split_top(rest);
        if p.is_empty() {
            return Err(format!("bad operand \"{op}\""));
        }
        parts.push(Part {
            expr: p[0].to_string(),
            kind: 'N',
        });
        let mut shape = format!("{mn} #N");
        match p.len() {
            1 => {}
            2 => {
                parts.push(Part {
                    expr: p[1].to_string(),
                    kind: 'A',
                });
                shape.push_str(",{}");
            }
            3 if p[2].eq_ignore_ascii_case("X") => {
                parts.push(Part {
                    expr: p[1].to_string(),
                    kind: 'A',
                });
                shape.push_str(",{},X");
            }
            _ => return Err(format!("bad operand \"{op}\"")),
        }
        shape
    } else if op.starts_with('(') {
        let Some(close) = matching_paren(op) else {
            return Err(format!("bad operand \"{op}\""));
        };
        let inner = split_top(&op[1..close]);
        let tail = op[close + 1..].trim();
        match (inner.as_slice(), tail) {
            ([e, x], "") if x.eq_ignore_ascii_case("X") => {
                parts.push(Part {
                    expr: e.to_string(),
                    kind: 'A',
                });
                format!("{mn} ({{}},X)")
            }
            ([e], "") => {
                parts.push(Part {
                    expr: e.to_string(),
                    kind: 'A',
                });
                format!("{mn} ({{}})")
            }
            ([e], y) if y.eq_ignore_ascii_case(",Y") => {
                parts.push(Part {
                    expr: e.to_string(),
                    kind: 'A',
                });
                format!("{mn} ({{}}),Y")
            }
            _ => return Err(format!("bad operand \"{op}\"")),
        }
    } else {
        let p = split_top(op);
        match p.as_slice() {
            [t] => {
                parts.push(Part {
                    expr: t.to_string(),
                    kind: 'A',
                });
                format!("{mn} {{}}")
            }
            [t, i] if i.eq_ignore_ascii_case("X") || i.eq_ignore_ascii_case("Y") => {
                parts.push(Part {
                    expr: t.to_string(),
                    kind: 'A',
                });
                format!("{mn} {{}},{}", i.to_ascii_uppercase())
            }
            [a, b, c] => {
                for e in [a, b, c] {
                    parts.push(Part {
                        expr: e.to_string(),
                        kind: 'A',
                    });
                }
                format!("{mn} {{}},{{}},{{}}")
            }
            _ => return Err(format!("bad operand \"{op}\"")),
        }
    };

    // Values.
    let mut values = Vec::new();
    for p in &parts {
        values.push(ev.value(&p.expr)?);
    }
    let known = !ev.unresolved.get();

    // Choose zero page or absolute for each address, then look the shape up.
    let addr_idx: Vec<usize> = parts
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == 'A')
        .map(|(i, _)| i)
        .collect();
    let zp_ok: Vec<bool> = addr_idx
        .iter()
        .map(|&i| {
            known
                && !site.force_wide
                && (0..=0xFF).contains(&values[i])
                && !is_wide_literal(&parts[i].expr)
        })
        .collect();
    let build = |zero_page: &[bool]| -> String {
        let mut out = String::new();
        let mut rest = template.as_str();
        let mut n = 0;
        while let Some(at) = rest.find("{}") {
            out.push_str(&rest[..at]);
            out.push(if zero_page[n] { 'Z' } else { 'W' });
            n += 1;
            rest = &rest[at + 2..];
        }
        out.push_str(rest);
        out
    };
    let mut attempts = vec![zp_ok.clone(), vec![false; zp_ok.len()]];
    if zp_ok.iter().any(|z| !z) && zp_ok.iter().any(|z| *z) {
        attempts.insert(1, zp_ok.iter().map(|_| false).collect());
    }
    let mut found = None;
    for a in &attempts {
        let key = build(a);
        if let Some(t) = table().get(&key) {
            found = Some((key, t));
            break;
        }
    }
    let Some((_, t)) = found else {
        return Err(format!("{mnemonic} {operand}: no such instruction form"));
    };
    let mut bytes = vec![0u8; t.len];
    bytes[0] = t.opcode;
    for ((slot, at), (part, value)) in t.slots.iter().zip(parts.iter().zip(values.iter().copied()))
    {
        match slot {
            Slot::Byte => {
                if part.kind == 'N' {
                    if !(-128..=255).contains(&value) {
                        return Err(format!("immediate value {value} does not fit in a byte"));
                    }
                } else if !(0..=0xFF).contains(&value) {
                    return Err(format!("{value} does not fit in a zero-page byte"));
                }
                bytes[*at] = (value & 0xFF) as u8;
            }
            Slot::Word => {
                if !(0..=0xFFFF).contains(&value) {
                    return Err(format!("address {value} is outside 0..65535"));
                }
                bytes[*at] = (value & 0xFF) as u8;
                bytes[*at + 1] = (value >> 8) as u8;
            }
            Slot::Rel => {
                let target = if known {
                    value
                } else {
                    site.addr as i64 + t.len as i64
                };
                let offset = target - (site.addr as i64 + t.len as i64);
                if !(-128..=127).contains(&offset) {
                    return Err(format!(
                        "branch target is {offset} bytes away; a branch reaches -128..127"
                    ));
                }
                bytes[*at] = offset as i8 as u8;
            }
        }
    }
    Ok(Encoded {
        bytes,
        unresolved: ev.unresolved.get(),
    })
}

/// A literal of three or more hex digits (`$0010`) says "absolute", as for the 6502.
fn is_wide_literal(text: &str) -> bool {
    let t = text.trim();
    let digits = t
        .strip_prefix('$')
        .or_else(|| t.strip_prefix("0x"))
        .or_else(|| t.strip_prefix("0X"));
    digits.is_some_and(|d| d.len() >= 3 && d.chars().all(|c| c.is_ascii_hexdigit()))
}
