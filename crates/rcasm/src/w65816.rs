//! The 65C816 (Super Nintendo, Apple IIGS): direct page, absolute and long addressing, stack-relative modes, block moves,
//! and immediates whose size follows the `M` and `X` flags.
//!
//! The encoding table is built once by enumerating the disassembler (`w65c816::disassemble`) in the four width states,
//! so every instruction the debugger shows assembles and the two cannot drift apart. Operand shapes: `N` an 8-bit
//! immediate, `I` a 16-bit one, `Z` a direct page byte, `W` a 16-bit address, `L` a 24-bit one, `R` an 8-bit relative
//! target, `Q` a 16-bit one.
//!
//! Which size an address takes: the digits of a literal (`$12` direct page, `$1234` absolute, `$123456` long), else
//! the value when it is known (`<= $FF` direct page, `<= $FFFF` absolute, else long), else absolute; a `.b`, `.w` or
//! `.l` suffix on the mnemonic decides outright. The size of an immediate is the accumulator's (`M`) or the index
//! registers' (`X`) width: `.a8`/`.a16`/`.i8`/`.i16` set them and, in `.smart` mode (the default), so do `REP` and `SEP`.

use crate::cpu::{Encoded, Eval, Site};
use crate::text::{matching_paren, split_top};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Byte,
    Word,
    Long,
    Rel8,
    Rel16,
}

#[derive(Debug)]
struct Template {
    opcode: u8,
    slots: Vec<(Slot, usize)>,
    len: usize,
}

const FILL: [u8; 6] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];

fn is_rel8(mn: &str) -> bool {
    matches!(
        mn,
        "BPL" | "BMI" | "BVC" | "BVS" | "BCC" | "BCS" | "BNE" | "BEQ" | "BRA"
    )
}

fn is_rel16(mn: &str) -> bool {
    matches!(mn, "BRL" | "PER")
}

/// The byte index (1-based) a value printed from `FILL` came from.
fn index_of(value: u32, digits: usize) -> Option<usize> {
    let bytes = digits / 2;
    (1..=FILL.len() - bytes + 1)
        .find(|&i| (0..bytes).fold(0u32, |a, k| a | (FILL[i - 1 + k] as u32) << (8 * k)) == value)
}

/// `LDA #$11` -> (`LDA #N`, slots); numbers in the text become shape letters.
fn shape_of(text: &str) -> (String, Vec<(Slot, usize)>) {
    let (mnemonic, rest) = text.split_once(' ').unwrap_or((text, ""));
    let mut shape = String::from(mnemonic);
    let mut slots = Vec::new();
    if rest.is_empty() {
        return (shape, slots);
    }
    shape.push(' ');
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && chars[end].is_ascii_hexdigit() {
                end += 1;
            }
            let digits = end - start;
            let value =
                u32::from_str_radix(&chars[start..end].iter().collect::<String>(), 16).unwrap_or(0);
            let after_hash = i > 0 && chars[i - 1] == '#';
            if is_rel8(mnemonic) {
                shape.push('R');
                slots.push((Slot::Rel8, 1));
            } else if is_rel16(mnemonic) {
                shape.push('Q');
                slots.push((Slot::Rel16, 1));
            } else {
                let at = index_of(value, digits).unwrap_or(1);
                match (digits, after_hash) {
                    (2, true) => {
                        shape.push('N');
                        slots.push((Slot::Byte, at));
                    }
                    (4, true) => {
                        shape.push('I');
                        slots.push((Slot::Word, at));
                    }
                    (2, false) => {
                        shape.push('Z');
                        slots.push((Slot::Byte, at));
                    }
                    (4, false) => {
                        shape.push('W');
                        slots.push((Slot::Word, at));
                    }
                    _ => {
                        shape.push('L');
                        slots.push((Slot::Long, at));
                    }
                }
            }
            i = end;
            continue;
        }
        shape.push(chars[i]);
        i += 1;
    }
    (shape, slots)
}

struct Table {
    shapes: HashMap<String, Template>,
    /// Mnemonics whose immediate follows `M`, and those whose follows `X`.
    imm_m: HashSet<String>,
    imm_x: HashSet<String>,
    mnemonics: HashSet<String>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut shapes = HashMap::new();
        let (mut imm_m, mut imm_x) = (HashSet::new(), HashSet::new());
        for (m16, x16) in [(false, false), (true, false), (false, true), (true, true)] {
            for op in 0..=255u8 {
                let bytes = [&[op][..], &FILL].concat();
                let (text, len) = w65c816::disassemble::disassemble(&bytes, 0x8000, m16, x16);
                let (shape, slots) = shape_of(&text);
                if shape.contains("#I") {
                    let mn = shape.split(' ').next().unwrap_or("").to_string();
                    if m16 && !x16 {
                        imm_m.insert(mn);
                    } else if x16 && !m16 {
                        imm_x.insert(mn);
                    }
                }
                shapes.entry(shape).or_insert(Template {
                    opcode: op,
                    slots,
                    len,
                });
            }
        }
        let mnemonics = shapes
            .keys()
            .filter_map(|k| k.split_whitespace().next())
            .map(str::to_string)
            .collect();
        Table {
            shapes,
            imm_m,
            imm_x,
            mnemonics,
        }
    })
}

/// A mnemonic with an optional `.b`/`.w`/`.l` suffix.
fn split_suffix(word: &str) -> (String, Option<char>) {
    let upper = word.to_ascii_uppercase();
    if upper.len() > 2 {
        if let Some((mn, sfx)) = upper.split_at_checked(upper.len() - 2) {
            if let Some(c) = sfx.strip_prefix('.').and_then(|s| s.chars().next()) {
                if matches!(c, 'B' | 'W' | 'L') && table().mnemonics.contains(mn) {
                    return (mn.to_string(), Some(c.to_ascii_lowercase()));
                }
            }
        }
    }
    (upper, None)
}

pub fn is_mnemonic(word: &str) -> bool {
    table().mnemonics.contains(&split_suffix(word).0)
}

struct Part {
    expr: String,
    /// `N` immediate (byte or word by width), `A` an address with a size to choose, `R`/`Q` relative.
    kind: char,
}

/// Whether the literal says "at least absolute" (three or more hex digits) or "long" (five or more).
fn literal_digits(text: &str) -> Option<usize> {
    let t = text.trim();
    let digits = t
        .strip_prefix('$')
        .or_else(|| t.strip_prefix("0x"))
        .or_else(|| t.strip_prefix("0X"))?;
    (digits.chars().all(|c| c.is_ascii_hexdigit()) && !digits.is_empty()).then_some(digits.len())
}

pub fn encode(mnemonic: &str, operand: &str, site: &Site) -> Result<Encoded, String> {
    let (mn, suffix) = split_suffix(mnemonic);
    let t = table();
    if !t.mnemonics.contains(&mn) {
        return Err(format!("unknown 65816 instruction \"{mnemonic}\""));
    }
    let ev = Eval::new(site);
    let op = operand.trim();
    let mut parts: Vec<Part> = Vec::new();
    // the template with `{}` where an address size goes
    let template: String = if op.is_empty() {
        mn.clone()
    } else if op.eq_ignore_ascii_case("A") {
        format!("{mn} A")
    } else if is_rel8(&mn) || is_rel16(&mn) {
        parts.push(Part {
            expr: op.to_string(),
            kind: if is_rel8(&mn) { 'R' } else { 'Q' },
        });
        format!("{mn} {}", if is_rel8(&mn) { 'R' } else { 'Q' })
    } else if let Some(rest) = op.strip_prefix('#') {
        parts.push(Part {
            expr: rest.trim().to_string(),
            kind: 'N',
        });
        let wide = if t.imm_m.contains(&mn) {
            site.m16
        } else if t.imm_x.contains(&mn) {
            site.x16
        } else {
            false
        };
        let wide = match suffix {
            Some('w') => true,
            Some('b') => false,
            _ => wide,
        };
        format!("{mn} #{}", if wide { 'I' } else { 'N' })
    } else if mn == "MVN" || mn == "MVP" {
        let p = split_top(op);
        if p.len() != 2 {
            return Err(format!("{mn} takes a source bank and a destination bank"));
        }
        for e in p {
            parts.push(Part {
                expr: e.to_string(),
                kind: 'A',
            });
        }
        format!("{mn} {{}},{{}}")
    } else if op.starts_with('(') || op.starts_with('[') {
        let long = op.starts_with('[');
        let close = if long {
            matching_bracket(op)
        } else {
            matching_paren(op)
        };
        let Some(close) = close else {
            return Err(format!("bad operand \"{op}\""));
        };
        let inner = split_top(&op[1..close]);
        let tail = op[close + 1..].trim();
        let (open, shut) = if long { ('[', ']') } else { ('(', ')') };
        match (inner.as_slice(), tail) {
            ([e], "") => {
                parts.push(Part {
                    expr: e.to_string(),
                    kind: 'A',
                });
                format!("{mn} {open}{{}}{shut}")
            }
            ([e, x], "") if !long && x.eq_ignore_ascii_case("X") => {
                parts.push(Part {
                    expr: e.to_string(),
                    kind: 'A',
                });
                format!("{mn} ({{}},X)")
            }
            ([e, s], y) if !long && s.eq_ignore_ascii_case("S") && y.eq_ignore_ascii_case(",Y") => {
                parts.push(Part {
                    expr: e.to_string(),
                    kind: 'A',
                });
                format!("{mn} ({{}},S),Y")
            }
            ([e], y) if y.eq_ignore_ascii_case(",Y") => {
                parts.push(Part {
                    expr: e.to_string(),
                    kind: 'A',
                });
                format!("{mn} {open}{{}}{shut},Y")
            }
            _ => return Err(format!("bad operand \"{op}\"")),
        }
    } else {
        let p = split_top(op);
        match p.as_slice() {
            [e] => {
                parts.push(Part {
                    expr: e.to_string(),
                    kind: 'A',
                });
                format!("{mn} {{}}")
            }
            [e, i] if ["X", "Y", "S"].iter().any(|r| i.eq_ignore_ascii_case(r)) => {
                parts.push(Part {
                    expr: e.to_string(),
                    kind: 'A',
                });
                format!("{mn} {{}},{}", i.to_ascii_uppercase())
            }
            _ => return Err(format!("bad operand \"{op}\"")),
        }
    };

    let mut values = Vec::new();
    for p in &parts {
        values.push(ev.value(&p.expr)?);
    }
    let known = !ev.unresolved.get();

    // sizes to try for each address part, best first
    let candidates = |idx: usize| -> Vec<char> {
        if mn == "MVN" || mn == "MVP" {
            return vec!['Z'];
        }
        if let Some(s) = suffix {
            return vec![match s {
                'b' => 'Z',
                'w' => 'W',
                _ => 'L',
            }];
        }
        let digits = literal_digits(&parts[idx].expr);
        let v = values[idx];
        if let Some(d) = digits {
            return match d {
                0..=2 if known && !site.force_wide => vec!['Z', 'W', 'L'],
                0..=2 => vec!['W', 'L', 'Z'],
                3..=4 => vec!['W', 'L'],
                _ => vec!['L'],
            };
        }
        if !known || site.force_wide {
            return vec!['W', 'L', 'Z'];
        }
        if (0..=0xFF).contains(&v) {
            vec!['Z', 'W', 'L']
        } else if (0..=0xFFFF).contains(&v) {
            vec!['W', 'L']
        } else {
            vec!['L']
        }
    };
    let addr_parts: Vec<usize> = parts
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == 'A')
        .map(|(i, _)| i)
        .collect();
    let fill = |sizes: &[char]| -> String {
        let mut out = String::new();
        let mut rest = template.as_str();
        let mut n = 0;
        while let Some(at) = rest.find("{}") {
            out.push_str(&rest[..at]);
            out.push(sizes[n]);
            n += 1;
            rest = &rest[at + 2..];
        }
        out.push_str(rest);
        out
    };
    let mut found = None;
    if addr_parts.is_empty() {
        if let Some(tpl) = t.shapes.get(&template) {
            found = Some(tpl);
        }
    } else if addr_parts.len() == 1 {
        for c in candidates(addr_parts[0]) {
            if let Some(tpl) = t.shapes.get(&fill(&[c])) {
                found = Some(tpl);
                break;
            }
        }
    } else {
        found = t.shapes.get(&fill(&vec!['Z'; addr_parts.len()]));
    }
    let Some(tpl) = found else {
        let hint = if matches!(mn.as_str(), "JMP" | "JSR")
            && values.first().is_some_and(|v| *v > 0xFFFF)
        {
            format!(
                " ({} takes a 16-bit address in the program bank; for a 24-bit one use {})",
                mn,
                if mn == "JMP" { "JML" } else { "JSL" }
            )
        } else {
            String::new()
        };
        return Err(format!(
            "{mnemonic} {operand}: no such instruction form{hint}"
        ));
    };
    if tpl.slots.len() != parts.len() {
        return Err(format!("{mnemonic} {operand}: no such instruction form"));
    }
    let mut bytes = vec![0u8; tpl.len];
    bytes[0] = tpl.opcode;
    for ((slot, at), (part, value)) in tpl
        .slots
        .iter()
        .zip(parts.iter().zip(values.iter().copied()))
    {
        match slot {
            Slot::Byte => {
                let ok = if part.kind == 'N' {
                    (-128..=255).contains(&value)
                } else {
                    (0..=0xFF).contains(&value)
                };
                if !ok {
                    let hint = if part.kind == 'N' && (256..=0xFFFF).contains(&value) {
                        " (the register is 8 bits wide here: .a16/.i16 or REP makes it 16)"
                    } else {
                        ""
                    };
                    return Err(format!("{value} does not fit in a byte{hint}"));
                }
                bytes[*at] = value as u8;
            }
            Slot::Word => {
                if !(-32768..=0xFFFF).contains(&value) {
                    return Err(format!(
                        "{value:#X} does not fit in 16 bits (use a long address, or the .l suffix)"
                    ));
                }
                bytes[*at] = value as u8;
                bytes[*at + 1] = (value >> 8) as u8;
            }
            Slot::Long => {
                if !(0..=0xFF_FFFF).contains(&value) {
                    return Err(format!("address {value:#X} is outside 0..16777215"));
                }
                bytes[*at] = value as u8;
                bytes[*at + 1] = (value >> 8) as u8;
                bytes[*at + 2] = (value >> 16) as u8;
            }
            Slot::Rel8 | Slot::Rel16 => {
                let next = site.addr as i64 + tpl.len as i64;
                let target = if known { value } else { next };
                if (target >> 16) != (next >> 16) && known && target > 0xFFFF {
                    return Err(format!(
                        "branch target ${target:06X} is in another bank (the branch stays in bank ${:02X})",
                        next >> 16
                    ));
                }
                if *slot == Slot::Rel8 {
                    let offset = (target & 0xFFFF) - (next & 0xFFFF);
                    if !(-128..=127).contains(&offset) {
                        return Err(format!(
                            "branch target is {offset} bytes away; a branch reaches -128..127"
                        ));
                    }
                    bytes[*at] = offset as i8 as u8;
                } else {
                    let offset = ((target & 0xFFFF) - (next & 0xFFFF)) & 0xFFFF;
                    bytes[*at] = offset as u8;
                    bytes[*at + 1] = (offset >> 8) as u8;
                }
            }
        }
    }
    Ok(Encoded {
        bytes,
        unresolved: ev.unresolved.get(),
    })
}

fn matching_bracket(text: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in text.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}
