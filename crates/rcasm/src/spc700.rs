//! The SPC700 (the Super Nintendo's sound CPU).
//!
//! The encoding table is built once by enumerating the disassembler (`spc700::disassemble`), so every instruction it
//! shows assembles. Operand shapes: `N` an immediate byte (`#`), `D` a direct page byte, `A` an absolute address
//! (`!`), `R` a relative target, `M` a memory bit (`$1234.5`: a 13-bit address and a bit number).
//!
//! Syntax: `mov a,#$12`, `mov a,$12` (direct page), `mov a,!$1234` (absolute; a plain operand above `$FF` is absolute
//! too), `mov a,$12+x`, `mov a,[$12+x]`, `mov a,[$12]+y`, `mov a,(x)+`, `or1 c,/$1234.5`, `set1 $12.3`,
//! `bbs $12.3,target`, `tcall 5`, `mov $22,$11` (destination first).

use crate::cpu::{Encoded, Eval, Site};
use crate::text::split_top;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Byte,
    Word,
    Bit,
    Rel,
}

#[derive(Debug)]
struct Template {
    opcode: u8,
    slots: Vec<(Slot, usize)>,
    len: usize,
}

const FILL: [u8; 3] = [0x11, 0x22, 0x33];

fn is_branch(mn: &str) -> bool {
    matches!(
        mn,
        "BPL"
            | "BMI"
            | "BVC"
            | "BVS"
            | "BCC"
            | "BCS"
            | "BNE"
            | "BEQ"
            | "BRA"
            | "BBS"
            | "BBC"
            | "CBNE"
            | "DBNZ"
    )
}

fn shape_of(text: &str, len: usize) -> (String, Vec<(Slot, usize)>) {
    let (mnemonic, rest) = text.split_once(' ').unwrap_or((text, ""));
    let mut shape = String::from(mnemonic);
    let mut slots = Vec::new();
    if rest.is_empty() {
        return (shape, slots);
    }
    shape.push(' ');
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    let rel_at = len - 1;
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
            let prev = if i > 0 { chars[i - 1] } else { ' ' };
            let bit_after = end < chars.len() && chars[end] == '.';
            if is_branch(mnemonic)
                && digits == 4
                && !bit_after
                && prev != '#'
                && (mnemonic != "BBS" && mnemonic != "BBC" || chars[..i].contains(&','))
            {
                shape.push('r');
                slots.push((Slot::Rel, rel_at));
            } else if digits == 4 && bit_after {
                // $1234.5: a memory bit; the index is 1 (always the first operand byte)
                shape.push('m');
                slots.push((Slot::Bit, 1));
                end += 2; // the dot and the bit digit
            } else if digits == 4 {
                let at = (1..=2)
                    .find(|&k| (FILL[k - 1] as u32 | (FILL[k] as u32) << 8) == value)
                    .unwrap_or(1);
                shape.push('a');
                slots.push((Slot::Word, at));
            } else {
                let at = FILL
                    .iter()
                    .position(|&f| f as u32 == value)
                    .map_or(1, |p| p + 1);
                if prev == '#' {
                    shape.push('n');
                } else {
                    shape.push('d');
                }
                slots.push((Slot::Byte, at));
                if bit_after {
                    // $12.3: the bit number is part of the opcode
                    shape.push('.');
                    shape.push(chars[end + 1]);
                    end += 2;
                }
            }
            i = end;
            continue;
        }
        if chars[i] == '!' {
            i += 1; // the shape letter `A` says "absolute"
            continue;
        }
        shape.push(chars[i]);
        i += 1;
    }
    (shape, slots)
}

fn table() -> &'static (HashMap<String, Template>, HashSet<String>) {
    static TABLE: OnceLock<(HashMap<String, Template>, HashSet<String>)> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut shapes = HashMap::new();
        for op in 0..=255u8 {
            let bytes = [&[op][..], &FILL].concat();
            let (text, len) = spc700::disassemble::disassemble(&bytes, 0x8000);
            let (shape, slots) = shape_of(&text, len);
            shapes.entry(shape).or_insert(Template {
                opcode: op,
                slots,
                len,
            });
        }
        let mnemonics = shapes
            .keys()
            .filter_map(|k| k.split_whitespace().next())
            .map(str::to_string)
            .collect();
        (shapes, mnemonics)
    })
}

pub fn is_mnemonic(word: &str) -> bool {
    table().1.contains(&word.to_ascii_uppercase())
}

/// One written operand, as the shape text it gives and the expression parts (with kinds) that fill it.
struct Operand {
    /// Shape text with `{}` where a size-choosing address goes, other letters fixed.
    shape: String,
    expr: Option<String>,
    /// `N`, `D` (direct page, may widen to `A`), `A` (absolute), `R`, `M`.
    kind: char,
    /// For `M`: the bit number.
    bit: u8,
}

fn strip_index(text: &str) -> (&str, &'static str) {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    if lower.ends_with("+x") {
        (t[..t.len() - 2].trim(), "+X")
    } else if lower.ends_with("+y") {
        (t[..t.len() - 2].trim(), "+Y")
    } else {
        (t, "")
    }
}

fn split_bit(text: &str) -> Option<(&str, u8)> {
    let (e, b) = text.rsplit_once('.')?;
    let bit: u8 = b.trim().parse().ok()?;
    (bit < 8).then_some((e.trim(), bit))
}

fn parse_operand(mn: &str, idx: usize, count: usize, text: &str) -> Result<Operand, String> {
    let t = text.trim();
    let upper = t.to_ascii_uppercase();
    let fixed = |s: &str| Operand {
        shape: s.to_string(),
        expr: None,
        kind: ' ',
        bit: 0,
    };
    if matches!(
        upper.as_str(),
        "A" | "X" | "Y" | "SP" | "C" | "YA" | "PSW" | "(X)" | "(Y)" | "(X)+"
    ) {
        return Ok(fixed(&upper));
    }
    if let Some(e) = t.strip_prefix('#') {
        return Ok(Operand {
            shape: "#n".to_string(),
            expr: Some(e.trim().to_string()),
            kind: 'N',
            bit: 0,
        });
    }
    if is_branch(mn) && idx + 1 == count {
        return Ok(Operand {
            shape: "r".to_string(),
            expr: Some(t.to_string()),
            kind: 'R',
            bit: 0,
        });
    }
    if let Some(inner) = t.strip_prefix('[') {
        // [d+X] or [d]+Y
        if let Some(body) = inner
            .strip_suffix("]+Y")
            .or_else(|| inner.strip_suffix("]+y"))
        {
            return Ok(Operand {
                shape: "[d]+Y".to_string(),
                expr: Some(body.trim().to_string()),
                kind: 'd',
                bit: 0,
            });
        }
        if let Some(body) = inner.strip_suffix(']') {
            let (e, idx) = strip_index(body);
            if idx == "+X" {
                return Ok(match e.strip_prefix('!') {
                    Some(abs) => Operand {
                        shape: "[a+X]".to_string(),
                        expr: Some(abs.trim().to_string()),
                        kind: 'A',
                        bit: 0,
                    },
                    None => Operand {
                        shape: "[d+X]".to_string(),
                        expr: Some(e.to_string()),
                        kind: 'd',
                        bit: 0,
                    },
                });
            }
        }
        return Err(format!("bad operand \"{t}\""));
    }
    let (complement, body) = match t.strip_prefix('/') {
        Some(rest) => (true, rest.trim()),
        None => (false, t),
    };
    // a memory bit or a direct page bit
    if let Some((e, bit)) = split_bit(body) {
        if matches!(mn, "SET1" | "CLR1" | "BBS" | "BBC") {
            return Ok(Operand {
                shape: format!("d.{bit}"),
                expr: Some(e.to_string()),
                kind: 'D',
                bit,
            });
        }
        return Ok(Operand {
            shape: if complement { "/m" } else { "m" }.to_string(),
            expr: Some(e.to_string()),
            kind: 'M',
            bit,
        });
    }
    if matches!(mn, "NOT1" | "OR1" | "AND1" | "EOR1" | "MOV1") && !body.contains('.') {
        return Err(format!("\"{t}\": a memory bit is written address.bit"));
    }
    if let Some(rest) = body.strip_prefix('!') {
        let (e, idx) = strip_index(rest);
        return Ok(Operand {
            shape: format!("a{idx}"),
            expr: Some(e.to_string()),
            kind: 'A',
            bit: 0,
        });
    }
    let (e, idx) = strip_index(body);
    Ok(Operand {
        shape: format!("d{idx}"),
        expr: Some(e.to_string()),
        kind: 'd',
        bit: 0,
    })
}

pub fn encode(mnemonic: &str, operand: &str, site: &Site) -> Result<Encoded, String> {
    let mn = mnemonic.to_ascii_uppercase();
    let (shapes, mnemonics) = table();
    if !mnemonics.contains(&mn) {
        return Err(format!("unknown SPC700 instruction \"{mnemonic}\""));
    }
    let ev = Eval::new(site);
    let op = operand.trim();
    let texts: Vec<&str> = if op.is_empty() {
        Vec::new()
    } else {
        split_top(op)
    };
    if mn == "TCALL" {
        let n = ev.value(op)?;
        if !(0..=15).contains(&n) {
            return Err(format!("TCALL {n}: the vector number is 0..15"));
        }
        let t = shapes
            .get(&format!("TCALL {n}"))
            .ok_or_else(|| "no such TCALL".to_string())?;
        return Ok(Encoded {
            bytes: vec![t.opcode],
            unresolved: false,
        });
    }
    let mut operands = Vec::new();
    for (i, t) in texts.iter().enumerate() {
        operands.push(parse_operand(&mn, i, texts.len(), t)?);
    }
    let mut values = Vec::new();
    for o in &operands {
        values.push(match &o.expr {
            Some(e) => ev.value(e)?,
            None => 0,
        });
    }
    let known = !ev.unresolved.get();
    // `d`-kind operands choose direct page or absolute by value; try direct page first
    let ambiguous: Vec<usize> = operands
        .iter()
        .enumerate()
        .filter(|(_, o)| o.kind == 'd')
        .map(|(i, _)| i)
        .collect();
    let mut combos: Vec<Vec<char>> = vec![Vec::new()];
    for &i in &ambiguous {
        let small = known && !site.force_wide && (0..=0xFF).contains(&values[i]);
        let order: &[char] = if small { &['d', 'a'] } else { &['a', 'd'] };
        let mut next = Vec::new();
        for c in &combos {
            for &k in order {
                let mut v = c.clone();
                v.push(k);
                next.push(v);
            }
        }
        combos = next;
    }
    let mut found = None;
    for combo in &combos {
        let mut shape_parts = Vec::new();
        let mut k = 0;
        for o in &operands {
            let s = if o.kind == 'd' {
                let letter = combo[k];
                k += 1;
                o.shape.replacen('d', &letter.to_string(), 1)
            } else {
                o.shape.clone()
            };
            shape_parts.push(s);
        }
        let key = if shape_parts.is_empty() {
            mn.clone()
        } else {
            format!("{mn} {}", shape_parts.join(","))
        };
        if let Some(t) = shapes.get(&key) {
            found = Some((t, combo.clone()));
            break;
        }
    }
    let Some((tpl, _)) = found else {
        return Err(format!("{mnemonic} {operand}: no such instruction form"));
    };
    // the slots appear in the shape's text order, which is the operand order
    let value_operands: Vec<usize> = operands
        .iter()
        .enumerate()
        .filter(|(_, o)| o.expr.is_some())
        .map(|(i, _)| i)
        .collect();
    if tpl.slots.len() != value_operands.len() {
        return Err(format!("{mnemonic} {operand}: no such instruction form"));
    }
    let mut bytes = vec![0u8; tpl.len];
    bytes[0] = tpl.opcode;
    for ((slot, at), &oi) in tpl.slots.iter().zip(&value_operands) {
        let v = values[oi];
        match slot {
            Slot::Byte => {
                let imm = operands[oi].kind == 'N';
                if !(if imm {
                    (-128..=255).contains(&v)
                } else {
                    (0..=0xFF).contains(&v)
                }) {
                    return Err(format!("{v} does not fit in a byte"));
                }
                bytes[*at] = v as u8;
            }
            Slot::Word => {
                if !(0..=0xFFFF).contains(&v) {
                    return Err(format!("address {v} is outside 0..65535"));
                }
                bytes[*at] = v as u8;
                bytes[*at + 1] = (v >> 8) as u8;
            }
            Slot::Bit => {
                if !(0..=0x1FFF).contains(&v) {
                    return Err(format!(
                        "a memory bit lives in the first 8 KB: address {v:#X}"
                    ));
                }
                let w = v as u16 | (operands[oi].bit as u16) << 13;
                bytes[*at] = w as u8;
                bytes[*at + 1] = (w >> 8) as u8;
            }
            Slot::Rel => {
                let next = site.addr as i64 + tpl.len as i64;
                let target = if known { v } else { next };
                let offset = target - next;
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
