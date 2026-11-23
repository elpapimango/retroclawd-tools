//! ld65 linker configuration files (`.cfg`), read as they are.
//!
//! ```text
//! MEMORY   { ZP: file = "", start = $0002, size = $00FE;  MAIN: file = %O, start = $0801, size = $77FF, fill = yes; }
//! SEGMENTS { ZEROPAGE: load = ZP, type = zp;  CODE: load = MAIN, type = ro;  BSS: load = MAIN, type = bss, align = $100; }
//! ```
//!
//! `MEMORY` areas become regions (`start`, `size`; `fill`, `file`, `type` and `define` are ignored: the layout's own
//! `fill`, `output` and machine decide those). `SEGMENTS` become placement rules: `load` the region, `start` an address,
//! `align`. `run`, `optional`, `define`, `offset` and `type` are accepted and ignored; the blocks `FILES`, `FORMATS`,
//! `SYMBOLS`, `FEATURES` and `STARTADDRESS` are skipped. Values are numbers (`$1000`, `0x1000`, `%101`, decimal) with
//! `+`, `-` and `*`; names of linker symbols (`__STACKSIZE__`) are not known here and are an error.

use crate::layout::{Region, SegmentRule, Value};

#[derive(Debug, Default)]
pub struct Cfg {
    pub regions: Vec<Region>,
    pub segments: Vec<SegmentRule>,
}

struct Lexer<'a> {
    chars: Vec<char>,
    pos: usize,
    text: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Word(String),
    Str(String),
    Sym(char),
}

fn lex(text: &str) -> Result<Vec<Tok>, String> {
    let l = Lexer {
        chars: text.chars().collect(),
        pos: 0,
        text,
    };
    let _ = l.text;
    let mut l = l;
    let mut out = Vec::new();
    while l.pos < l.chars.len() {
        let c = l.chars[l.pos];
        if c.is_whitespace() {
            l.pos += 1;
        } else if c == '#' {
            while l.pos < l.chars.len() && l.chars[l.pos] != '\n' {
                l.pos += 1;
            }
        } else if c == '"' {
            let mut s = String::new();
            l.pos += 1;
            while l.pos < l.chars.len() && l.chars[l.pos] != '"' {
                s.push(l.chars[l.pos]);
                l.pos += 1;
            }
            if l.pos >= l.chars.len() {
                return Err("an unterminated string".to_string());
            }
            l.pos += 1;
            out.push(Tok::Str(s));
        } else if "{};:,=+-*()".contains(c) {
            out.push(Tok::Sym(c));
            l.pos += 1;
        } else {
            let start = l.pos;
            while l.pos < l.chars.len()
                && !l.chars[l.pos].is_whitespace()
                && !"{};:,=+-*()\"#".contains(l.chars[l.pos])
            {
                l.pos += 1;
            }
            if start == l.pos {
                return Err(format!("unexpected \"{c}\""));
            }
            out.push(Tok::Word(l.chars[start..l.pos].iter().collect()));
        }
    }
    Ok(out)
}

/// An expression of numbers with `+ - *`, left to right with `*` first.
fn value(toks: &[Tok]) -> Result<i64, String> {
    let mut terms: Vec<(char, i64)> = Vec::new();
    let mut sign = '+';
    let mut i = 0;
    let mut cur: Option<i64> = None;
    let mut mul: Option<i64> = None;
    let flush = |terms: &mut Vec<(char, i64)>, sign: char, cur: &mut Option<i64>| {
        if let Some(v) = cur.take() {
            terms.push((sign, v));
        }
    };
    while i < toks.len() {
        match &toks[i] {
            Tok::Word(w) => {
                let n = rc_common::parse_number(w).map_err(|_| {
                    format!("\"{w}\" is not a number (linker symbols are not known here)")
                })?;
                cur = Some(match mul.take() {
                    Some(m) => m * n,
                    None => n,
                });
            }
            Tok::Sym('*') => mul = cur.take(),
            Tok::Sym(c @ ('+' | '-')) => {
                flush(&mut terms, sign, &mut cur);
                sign = *c;
            }
            other => return Err(format!("unexpected {other:?} in a value")),
        }
        i += 1;
    }
    flush(&mut terms, sign, &mut cur);
    Ok(terms
        .iter()
        .fold(0, |a, (s, v)| if *s == '+' { a + v } else { a - v }))
}

pub fn parse(text: &str) -> Result<Cfg, String> {
    let toks = lex(text)?;
    let mut cfg = Cfg::default();
    let mut i = 0;
    while i < toks.len() {
        let Tok::Word(block) = &toks[i] else {
            return Err(format!("expected a block name, found {:?}", toks[i]));
        };
        let block = block.to_ascii_uppercase();
        if toks.get(i + 1) != Some(&Tok::Sym('{')) {
            return Err(format!("{block}: expected {{"));
        }
        let mut j = i + 2;
        let mut entries: Vec<&[Tok]> = Vec::new();
        let mut start = j;
        while j < toks.len() && toks[j] != Tok::Sym('}') {
            if toks[j] == Tok::Sym(';') {
                entries.push(&toks[start..j]);
                start = j + 1;
            }
            j += 1;
        }
        if j >= toks.len() {
            return Err(format!("{block}: missing }}"));
        }
        if start < j {
            entries.push(&toks[start..j]); // a last entry without its ;
        }
        i = j + 1;
        if block != "MEMORY" && block != "SEGMENTS" {
            continue;
        }
        for e in entries.into_iter().filter(|e| !e.is_empty()) {
            let Some(Tok::Word(name)) = e.first() else {
                return Err(format!("{block}: an entry starts with a name"));
            };
            if e.get(1) != Some(&Tok::Sym(':')) {
                return Err(format!("{block}: {name}: expected :"));
            }
            // attributes: key = value pairs separated by commas (a value may hold + - *)
            let mut attrs: Vec<(String, Vec<Tok>)> = Vec::new();
            let mut k = 2;
            while k < e.len() {
                let Tok::Word(key) = &e[k] else {
                    return Err(format!("{block}: {name}: expected an attribute name"));
                };
                if e.get(k + 1) != Some(&Tok::Sym('=')) {
                    return Err(format!("{block}: {name}: {key} needs = value"));
                }
                let vs = k + 2;
                let mut ve = vs;
                while ve < e.len() && e[ve] != Tok::Sym(',') {
                    ve += 1;
                }
                attrs.push((key.to_ascii_lowercase(), e[vs..ve].to_vec()));
                k = ve + 1;
            }
            let get = |key: &str| {
                attrs
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.as_slice())
            };
            let word = |key: &str| -> Option<String> {
                match get(key) {
                    Some([Tok::Word(w)]) | Some([Tok::Str(w)]) => Some(w.clone()),
                    _ => None,
                }
            };
            if block == "MEMORY" {
                let start = value(get("start").ok_or_else(|| format!("MEMORY {name}: no start"))?)
                    .map_err(|e| format!("MEMORY {name}: start: {e}"))?;
                let size = value(get("size").ok_or_else(|| format!("MEMORY {name}: no size"))?)
                    .map_err(|e| format!("MEMORY {name}: size: {e}"))?;
                if size <= 0 {
                    return Err(format!("MEMORY {name}: size {size}"));
                }
                cfg.regions.push(Region {
                    name: name.clone(),
                    start: Value::Int(start),
                    end: Value::Int(start + size - 1),
                });
            } else {
                let start = match get("start") {
                    Some(v) => Some(Value::Int(
                        value(v).map_err(|e| format!("SEGMENTS {name}: start: {e}"))?,
                    )),
                    None => None,
                };
                let align = match get("align") {
                    Some(v) => Some(
                        u32::try_from(
                            value(v).map_err(|e| format!("SEGMENTS {name}: align: {e}"))?,
                        )
                        .map_err(|_| format!("SEGMENTS {name}: bad align"))?,
                    ),
                    None => None,
                };
                cfg.segments.push(SegmentRule {
                    name: name.clone(),
                    region: word("load"),
                    start,
                    align,
                });
            }
        }
    }
    Ok(cfg)
}
