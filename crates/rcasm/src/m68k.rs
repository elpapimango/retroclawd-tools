//! The 68000 back end: Motorola syntax, two passes, three sections, and the relocation marks a GEMDOS program needs.
//!
//! It is a module of its own rather than a CPU of the 16-bit assembler (`asm.rs`): addresses here are 32 bits, a program
//! has `text`, `data` and `bss` sections, and a long operand that holds a section address is marked for relocation.
//!
//! Language (lower or upper case mnemonics, labels case-sensitive, `;` comments):
//! - labels: `name:` or `name` at the start of a line; `name equ expr` / `name = expr`; sections `.text` `.data` `.bss`;
//!   `.org expr` (flat programs only, at most 24 bits); `dc.b/w/l` (strings in quotes for `dc.b`), `ds.b/w/l`,
//!   `.even`, `.include "file"`, `.opt autobranch`, `.globl`/`xdef` (ignored);
//! - operands: `d0-d7`, `a0-a7`, `sp` (a7), `(an)`, `(an)+`, `-(an)`, `d(an)`, `d(an,xn.w)`, `d(pc)`, `d(pc,xn.w)`,
//!   `expr.w`, `expr` or `expr.l` (absolute long: never shortened for you), `#expr`, `sr`, `ccr`, `usp`, MOVEM lists
//!   (`d0-d7/a0-a6`);
//! - sizes `.b .w .l` (`.w` when left out, except where the instruction has one size); `bra`, `bsr`, `bcc`, `dbcc` take
//!   `.s` or `.w`, and a branch with no size is an error unless `.opt autobranch` is on (then `.s` when the target is
//!   behind the branch and near, else `.w`);
//! - `add`/`sub`/`cmp`/`and`/`or`/`eor` with an immediate source become the `i` forms, an address register
//!   destination becomes `adda`/`suba`/`cmpa`; `addq`, `subq`, `moveq` are explicit.
//!
//! Every instruction is checked by decoding what was produced with the CPU crate's disassembler: a combination of
//! operands the 68000 does not have comes back as a word that is no instruction, and is refused (the instruction set
//! is the CPU crate's, so this assembler cannot accept what the debugger would not show).

use crate::asm::Files;
use crate::expr::{self, EvalError, Scope};
use crate::text::{matching_paren, split_top, strip_comment};
use rc_common::Diag;
use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

/// What the caller wants.
#[derive(Clone, Debug, Default)]
pub struct M68kOptions {
    /// A relocatable program (a `.PRG`): the text starts at 0, `.org` is an error, and a long that holds an address
    /// is recorded in [`M68kAssembly::relocs`]. Off: a flat image at `.org` (default 0).
    pub relocatable: bool,
    pub include_dirs: Vec<PathBuf>,
    /// `-D NAME=VALUE`
    pub defines: Vec<(String, i64)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct M68kSymbol {
    pub name: String,
    pub value: i64,
    /// An address (a label) rather than a constant.
    pub label: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct M68kListing {
    pub file: String,
    pub line: usize,
    pub addr: u32,
    pub bytes: Vec<u8>,
    pub source: String,
}

#[derive(Clone, Debug, Default)]
pub struct M68kAssembly {
    pub text: Vec<u8>,
    pub data: Vec<u8>,
    /// The size of the zeroed bss section (it follows the data section in memory).
    pub bss: u32,
    /// Where the text starts: 0 for a relocatable program, the `.org` of a flat one.
    pub origin: u32,
    /// Offsets from the start of the text of the longs that hold an address of this program, ascending (a relocatable
    /// program only).
    pub relocs: Vec<u32>,
    pub symbols: Vec<M68kSymbol>,
    pub listing: Vec<M68kListing>,
}

impl M68kAssembly {
    /// Text then data, as they lie in memory.
    pub fn image(&self) -> Vec<u8> {
        self.text.iter().chain(&self.data).copied().collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sz {
    B,
    W,
    L,
}

impl Sz {
    fn bits(self) -> u16 {
        match self {
            Sz::B => 0,
            Sz::W => 1,
            Sz::L => 2,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sec {
    Text = 0,
    Data = 1,
    Bss = 2,
}

#[derive(Clone, Debug)]
enum Def {
    Label(Sec, u32),
    Const(i64),
    /// `equ` with an expression that was not yet computable: evaluated when asked for.
    Lazy(String),
}

#[derive(Clone, Debug)]
enum Op {
    Dn(u8),
    An(u8),
    Ind(u8),
    Post(u8),
    Pre(u8),
    Disp(String, u8),
    /// `d(an,xn.s)`: displacement, base register, index register (8-15 are address registers), long index.
    Idx(String, u8, u8, bool),
    PcDisp(String),
    PcIdx(String, u8, bool),
    AbsW(String),
    AbsL(String),
    Imm(String),
    Sr,
    Ccr,
    Usp,
    /// An expression with no marking: an absolute long as an operand, the target of a branch.
    Bare(String),
}

#[derive(Clone, Debug)]
enum Ext {
    W(u16),
    L(u32, bool),
}

struct Line {
    file: Rc<str>,
    line: usize,
    text: String,
}

struct Asm<'a> {
    opts: &'a M68kOptions,
    strict: bool,
    symbols: HashMap<String, Def>,
    order: Vec<String>,
    sec: Sec,
    off: [u32; 3],
    base: [i64; 3],
    out: [Vec<u8>; 2],
    relocs: Vec<u32>,
    org: Option<u32>,
    autobranch: bool,
    /// Autobranch decisions of pass 1, by order of occurrence; pass 2 follows them.
    branch_short: Vec<bool>,
    branch_seen: usize,
    errors: Vec<Diag>,
    listing: Vec<M68kListing>,
    unresolved: Cell<bool>,
    depth: Cell<u32>,
}

/// Evaluation of expressions at one point of one pass.
struct Sc<'a, 'b> {
    asm: &'b Asm<'a>,
    /// Added to every section address and to the pc: shifting the whole program tells which values are addresses.
    shift: i64,
    pc: i64,
}

impl Scope for Sc<'_, '_> {
    fn symbol(&self, name: &str) -> Option<i64> {
        let a = self.asm;
        match a.symbols.get(name) {
            Some(Def::Label(sec, off)) => Some(a.base[*sec as usize] + *off as i64 + self.shift),
            Some(Def::Const(v)) => Some(*v),
            Some(Def::Lazy(text)) => {
                if a.depth.get() > 32 {
                    return None;
                }
                a.depth.set(a.depth.get() + 1);
                let v = expr::parse(text).ok().and_then(|e| e.eval(self).ok());
                a.depth.set(a.depth.get() - 1);
                v
            }
            None => {
                if a.strict {
                    None
                } else {
                    a.unresolved.set(true);
                    Some(0)
                }
            }
        }
    }

    fn pc(&self) -> i64 {
        self.pc
    }
}

const SHIFT: i64 = 1 << 28;

/// A distance between two addresses as the 68000 sees it: the address bus is 24 bits, so it wraps there.
fn wrap24(d: i64) -> i64 {
    let m = d & 0xFF_FFFF;
    if m >= 0x80_0000 { m - 0x100_0000 } else { m }
}

fn conditions() -> &'static [(&'static str, u16)] {
    &[
        ("t", 0),
        ("f", 1),
        ("hi", 2),
        ("ls", 3),
        ("cc", 4),
        ("hs", 4),
        ("cs", 5),
        ("lo", 5),
        ("ne", 6),
        ("eq", 7),
        ("vc", 8),
        ("vs", 9),
        ("pl", 10),
        ("mi", 11),
        ("ge", 12),
        ("lt", 13),
        ("gt", 14),
        ("le", 15),
    ]
}

fn condition(name: &str) -> Option<u16> {
    conditions()
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
}

fn register(text: &str) -> Option<Op> {
    let t = text.trim().to_ascii_lowercase();
    if t == "sp" {
        return Some(Op::An(7));
    }
    let b = t.as_bytes();
    if b.len() == 2 && (b'0'..=b'7').contains(&b[1]) {
        return match b[0] {
            b'd' => Some(Op::Dn(b[1] - b'0')),
            b'a' => Some(Op::An(b[1] - b'0')),
            _ => None,
        };
    }
    None
}

fn index_register(text: &str) -> Option<(u8, bool)> {
    let t = text.trim().to_ascii_lowercase();
    let (reg, long) = if let Some(r) = t.strip_suffix(".l") {
        (r, true)
    } else if let Some(r) = t.strip_suffix(".w") {
        (r, false)
    } else {
        (t.as_str(), false)
    };
    match register(reg)? {
        Op::Dn(n) => Some((n, long)),
        Op::An(n) => Some((8 + n, long)),
        _ => None,
    }
}

fn parse_op(text: &str) -> Result<Op, String> {
    let t = text.trim();
    if t.is_empty() {
        return Err("missing operand".to_string());
    }
    let lower = t.to_ascii_lowercase();
    match lower.as_str() {
        "sr" => return Ok(Op::Sr),
        "ccr" => return Ok(Op::Ccr),
        "usp" => return Ok(Op::Usp),
        _ => {}
    }
    if let Some(r) = register(t) {
        return Ok(r);
    }
    if let Some(rest) = t.strip_prefix('#') {
        return Ok(Op::Imm(rest.trim().to_string()));
    }
    if let Some(inner) = t.strip_prefix("-(").and_then(|r| r.strip_suffix(')'))
        && let Some(Op::An(n)) = register(inner)
    {
        return Ok(Op::Pre(n));
    }
    if let Some(inner) = t.strip_prefix('(').and_then(|r| r.strip_suffix(")+"))
        && let Some(Op::An(n)) = register(inner)
    {
        return Ok(Op::Post(n));
    }
    if t.ends_with(')') {
        // `prefix(inner)`: the parenthesis group that closes at the end.
        let open = t
            .char_indices()
            .filter(|(_, c)| *c == '(')
            .map(|(i, _)| i)
            .find(|&i| matching_paren(&t[i..]) == Some(t.len() - i - 1));
        if let Some(open) = open {
            let prefix = t[..open].trim();
            let parts = split_top(&t[open + 1..t.len() - 1]);
            let disp = if prefix.is_empty() { "0" } else { prefix };
            let base = parts.first().map(|p| p.trim().to_ascii_lowercase());
            match (base.as_deref(), parts.len()) {
                (Some("pc"), 1) => return Ok(Op::PcDisp(disp.to_string())),
                (Some("pc"), 2) => {
                    let (r, long) = index_register(parts[1]).ok_or_else(|| {
                        format!("\"{}\" is not an index register", parts[1].trim())
                    })?;
                    return Ok(Op::PcIdx(disp.to_string(), r, long));
                }
                (Some(b), n) => {
                    if let Some(Op::An(a)) = register(b) {
                        if n == 1 {
                            return Ok(if prefix.is_empty() {
                                Op::Ind(a)
                            } else {
                                Op::Disp(prefix.to_string(), a)
                            });
                        }
                        if n == 2 {
                            let (r, long) = index_register(parts[1]).ok_or_else(|| {
                                format!("\"{}\" is not an index register", parts[1].trim())
                            })?;
                            return Ok(Op::Idx(disp.to_string(), a, r, long));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(prefix) = lower.strip_suffix(".w") {
        return Ok(Op::AbsW(t[..prefix.len()].to_string()));
    }
    if let Some(prefix) = lower.strip_suffix(".l") {
        return Ok(Op::AbsL(t[..prefix.len()].to_string()));
    }
    Ok(Op::Bare(t.to_string()))
}

/// A MOVEM register list: `d0-d7/a0-a6`, `d1/d3`, a lone register. Bit 0 is d0 ... bit 15 is a7.
fn parse_list(text: &str) -> Option<u16> {
    let mut mask = 0u16;
    for part in text.split('/') {
        let (lo, hi) = match part.split_once('-') {
            Some((a, b)) => (a, b),
            None => (part, part),
        };
        let n = |r: &str| match register(r)? {
            Op::Dn(n) => Some(n),
            Op::An(n) => Some(8 + n),
            _ => None,
        };
        let (a, b) = (n(lo)?, n(hi)?);
        if a > b || (a < 8) != (b < 8) {
            return None;
        }
        for i in a..=b {
            mask |= 1 << i;
        }
    }
    Some(mask)
}

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

impl<'a> Asm<'a> {
    fn new(opts: &'a M68kOptions) -> Self {
        let mut symbols = HashMap::new();
        for (name, v) in &opts.defines {
            symbols.insert(name.clone(), Def::Const(*v));
        }
        Asm {
            opts,
            strict: false,
            symbols,
            order: Vec::new(),
            sec: Sec::Text,
            off: [0; 3],
            base: [0; 3],
            out: [Vec::new(), Vec::new()],
            relocs: Vec::new(),
            org: None,
            autobranch: false,
            branch_short: Vec::new(),
            branch_seen: 0,
            errors: Vec::new(),
            listing: Vec::new(),
            unresolved: Cell::new(false),
            depth: Cell::new(0),
        }
    }

    fn pc(&self) -> i64 {
        self.base[self.sec as usize] + self.off[self.sec as usize] as i64
    }

    /// The value of an expression, and whether it is an address of this program (relocatable output, final pass).
    fn value(&self, text: &str, pc: i64) -> Result<(i64, bool), String> {
        let e = expr::parse(text)?;
        let eval = |shift: i64| {
            e.eval(&Sc {
                asm: self,
                shift,
                pc: pc + shift,
            })
        };
        let fail = |err: EvalError| match err {
            EvalError::Undefined(name) => format!("undefined symbol \"{name}\""),
            EvalError::Other(m) => m,
        };
        let v1 = eval(0).map_err(fail)?;
        if !(self.strict && self.opts.relocatable) {
            return Ok((v1, false));
        }
        let v2 = eval(SHIFT).map_err(fail)?;
        match v2 - v1 {
            0 => Ok((v1, false)),
            SHIFT => Ok((v1, true)),
            _ => Err(format!(
                "\"{text}\" is not an address or a constant: it cannot be relocated"
            )),
        }
    }

    fn number(&self, text: &str, pc: i64) -> Result<i64, String> {
        self.value(text, pc).map(|v| v.0)
    }

    fn err(&mut self, line: &Line, message: impl Into<String>) {
        if self.errors.len() < 50 {
            self.errors
                .push(Diag::new(&*line.file, line.line, message.into()));
        }
    }

    // ---- effective addresses ----

    /// The six-bit mode/register field and the extension words of `op` as an operand of size `sz`. `at` is the
    /// address of the next extension word (a PC-relative operand is relative to it).
    fn ea(&self, op: &Op, sz: Sz, at: i64, ipc: i64) -> Result<(u16, Vec<Ext>), String> {
        // The first pass does not know every value yet: it only needs the size, so it does not range check.
        let lax = !self.strict;
        let word = |v: i64, what: &str| -> Result<u16, String> {
            if lax || (-0x8000..=0xFFFF).contains(&v) {
                Ok(v as u16)
            } else {
                Err(format!("{what} {v} does not fit in 16 bits"))
            }
        };
        let disp16 = |v: i64| -> Result<u16, String> {
            if lax || (-0x8000..=0x7FFF).contains(&v) {
                Ok(v as u16)
            } else {
                Err(format!("displacement {v} does not fit in 16 bits"))
            }
        };
        let brief = |disp: i64, reg: u8, long: bool| -> Result<u16, String> {
            if !lax && !(-128..=127).contains(&disp) {
                return Err(format!("displacement {disp} does not fit in 8 bits"));
            }
            Ok(((reg as u16) << 12) | ((long as u16) << 11) | (disp as u8 as u16))
        };
        let no_reloc = |reloc: bool, what: &str| -> Result<(), String> {
            if reloc {
                Err(format!(
                    "an address of the program cannot go in a 16-bit {what}"
                ))
            } else {
                Ok(())
            }
        };
        Ok(match op {
            Op::Dn(n) => (*n as u16, vec![]),
            Op::An(n) => (8 | *n as u16, vec![]),
            Op::Ind(n) => (0x10 | *n as u16, vec![]),
            Op::Post(n) => (0x18 | *n as u16, vec![]),
            Op::Pre(n) => (0x20 | *n as u16, vec![]),
            Op::Disp(e, n) => {
                let (v, r) = self.value(e, ipc)?;
                no_reloc(r, "displacement")?;
                (0x28 | *n as u16, vec![Ext::W(disp16(v)?)])
            }
            Op::Idx(e, n, reg, long) => {
                let (v, r) = self.value(e, ipc)?;
                no_reloc(r, "displacement")?;
                (0x30 | *n as u16, vec![Ext::W(brief(v, *reg, *long)?)])
            }
            Op::PcDisp(e) => {
                let (target, _) = self.value(e, ipc)?;
                (0x3A, vec![Ext::W(disp16(wrap24(target - at))?)])
            }
            Op::PcIdx(e, reg, long) => {
                let (target, _) = self.value(e, ipc)?;
                (0x3B, vec![Ext::W(brief(wrap24(target - at), *reg, *long)?)])
            }
            Op::AbsW(e) => {
                let (v, r) = self.value(e, ipc)?;
                no_reloc(r, "absolute address")?;
                (0x38, vec![Ext::W(word(v, "address")?)])
            }
            Op::AbsL(e) | Op::Bare(e) => {
                let (v, r) = self.value(e, ipc)?;
                if !lax && !(-0x8000_0000..=0xFFFF_FFFF).contains(&v) {
                    return Err(format!("address {v} does not fit in 32 bits"));
                }
                (0x39, vec![Ext::L(v as u32, r)])
            }
            Op::Imm(e) => {
                let (v, r) = self.value(e, ipc)?;
                match sz {
                    Sz::B => {
                        no_reloc(r, "immediate")?;
                        if !lax && !(-128..=255).contains(&v) {
                            return Err(format!("{v} does not fit in a byte"));
                        }
                        (0x3C, vec![Ext::W(v as u8 as u16)])
                    }
                    Sz::W => {
                        no_reloc(r, "immediate")?;
                        (0x3C, vec![Ext::W(word(v, "immediate")?)])
                    }
                    Sz::L => {
                        if !lax && !(-0x8000_0000..=0xFFFF_FFFF).contains(&v) {
                            return Err(format!("{v} does not fit in 32 bits"));
                        }
                        (0x3C, vec![Ext::L(v as u32, r)])
                    }
                }
            }
            Op::Sr | Op::Ccr | Op::Usp => {
                return Err("this register is not an operand here".to_string());
            }
        })
    }

    /// The size in bytes of the extension words.
    fn ext_len(ext: &[Ext]) -> i64 {
        ext.iter()
            .map(|e| match e {
                Ext::W(_) => 2,
                Ext::L(..) => 4,
            })
            .sum()
    }

    /// One instruction: its opcode word and extension words from the pieces. Extension words of the operands are
    /// computed in the order they are written, each knowing where it lies (PC-relative operands need that).
    fn build(&self, insn: &Insn, pc: i64) -> Result<Vec<Ext>, String> {
        // First the operands' extension words in order, with the pc moving over what is already there.
        let mut words: Vec<Ext> = Vec::new();
        let mut fields: Vec<u16> = Vec::new();
        for item in &insn.parts {
            match item {
                Part::Ea(op, sz) => {
                    let at = pc + 2 + Self::ext_len(&words);
                    let (field, ext) = self.ea(op, *sz, at, pc)?;
                    fields.push(field);
                    words.extend(ext);
                }
                Part::Word(w) => words.push(Ext::W(*w)),
                Part::WordFrom(f) => {
                    let at = pc + 2 + Self::ext_len(&words);
                    words.push(Ext::W(f(self, at)?));
                }
            }
        }
        let opcode = (insn.opcode)(&fields);
        let mut all = vec![Ext::W(opcode)];
        all.extend(words);
        Ok(all)
    }
}

/// Works out the opcode word from the effective-address fields of the operands, in the order they were given.
type OpcodeFn = Box<dyn Fn(&[u16]) -> u16>;
/// Works out a word from the address it will sit at (a displacement).
type WordFn = Box<dyn Fn(&Asm, i64) -> Result<u16, String>>;

/// An instruction before its operands are resolved.
struct Insn {
    parts: Vec<Part>,
    opcode: OpcodeFn,
}

enum Part {
    /// An effective address operand of the given size: contributes its field to the opcode and its extension words.
    Ea(Op, Sz),
    Word(u16),
    /// A word worked out from the address it will sit at (branches, DBcc).
    WordFrom(WordFn),
}

fn fixed(opcode: u16) -> Insn {
    Insn {
        parts: vec![],
        opcode: Box::new(move |_| opcode),
    }
}

fn with_ea(op: Op, sz: Sz, f: impl Fn(u16) -> u16 + 'static) -> Insn {
    Insn {
        parts: vec![Part::Ea(op, sz)],
        opcode: Box::new(move |fs| f(fs[0])),
    }
}

fn with_ea_then(op: Op, sz: Sz, extra: Vec<Part>, f: impl Fn(u16) -> u16 + 'static) -> Insn {
    let mut parts = vec![Part::Ea(op, sz)];
    parts.extend(extra);
    Insn {
        parts,
        opcode: Box::new(move |fs| f(fs[0])),
    }
}

/// The destination field of a MOVE: register in bits 11-9, mode in bits 8-6.
fn swap_ea(field: u16) -> u16 {
    ((field & 7) << 9) | ((field >> 3) << 6)
}

impl Asm<'_> {
    /// The size from the suffix, or `default`.
    fn size(sz: Option<Sz>, default: Sz) -> Sz {
        sz.unwrap_or(default)
    }

    fn data_alterable_or_dn(op: &Op) -> bool {
        matches!(
            op,
            Op::Dn(_)
                | Op::Ind(_)
                | Op::Post(_)
                | Op::Pre(_)
                | Op::Disp(..)
                | Op::Idx(..)
                | Op::AbsW(_)
                | Op::AbsL(_)
                | Op::Bare(_)
        )
    }

    /// The instruction for `mnemonic` (already lower case, suffix removed) with its operand texts.
    fn instruction(
        &mut self,
        mn: &str,
        sz: Option<Sz>,
        raw: &[&str],
        pc: i64,
    ) -> Result<Insn, String> {
        let ops: Vec<Op> = raw.iter().map(|r| parse_op(r)).collect::<Result<_, _>>()?;
        let need = |n: usize| -> Result<(), String> {
            if ops.len() == n {
                Ok(())
            } else {
                Err(format!(
                    "{mn} takes {n} operand{}",
                    if n == 1 { "" } else { "s" }
                ))
            }
        };
        let size_w = |s: Option<Sz>| Self::size(s, Sz::W);
        // Plain no-operand instructions.
        let simple = match mn {
            "nop" => Some(0x4E71),
            "rts" => Some(0x4E75),
            "rte" => Some(0x4E73),
            "rtr" => Some(0x4E77),
            "reset" => Some(0x4E70),
            "trapv" => Some(0x4E76),
            "illegal" => Some(0x4AFC),
            _ => None,
        };
        if let Some(w) = simple {
            need(0)?;
            return Ok(fixed(w));
        }
        match mn {
            "trap" => {
                need(1)?;
                let Op::Imm(e) = &ops[0] else {
                    return Err("trap takes #vector".to_string());
                };
                let v = self.number(e, pc)?;
                if !(0..=15).contains(&v) {
                    return Err(format!("trap vector {v} is not 0-15"));
                }
                return Ok(fixed(0x4E40 | v as u16));
            }
            "stop" => {
                need(1)?;
                let Op::Imm(e) = &ops[0] else {
                    return Err("stop takes #status".to_string());
                };
                let v = self.number(e, pc)?;
                return Ok(Insn {
                    parts: vec![Part::Word(v as u16)],
                    opcode: Box::new(|_| 0x4E72),
                });
            }
            "moveq" => {
                need(2)?;
                let (Op::Imm(e), Op::Dn(d)) = (&ops[0], &ops[1]) else {
                    return Err("moveq takes #value,dn".to_string());
                };
                let v = self.number(e, pc)?;
                if !(-128..=255).contains(&v) {
                    return Err(format!("moveq value {v} does not fit in a byte"));
                }
                return Ok(fixed(0x7000 | ((*d as u16) << 9) | (v as u8 as u16)));
            }
            "swap" => {
                need(1)?;
                let Op::Dn(d) = &ops[0] else {
                    return Err("swap takes a data register".to_string());
                };
                return Ok(fixed(0x4840 | *d as u16));
            }
            "ext" => {
                need(1)?;
                let Op::Dn(d) = &ops[0] else {
                    return Err("ext takes a data register".to_string());
                };
                let base = match sz {
                    Some(Sz::L) => 0x48C0,
                    Some(Sz::W) | None => 0x4880,
                    Some(Sz::B) => return Err("ext is .w or .l".to_string()),
                };
                return Ok(fixed(base | *d as u16));
            }
            "unlk" => {
                need(1)?;
                let Op::An(a) = &ops[0] else {
                    return Err("unlk takes an address register".to_string());
                };
                return Ok(fixed(0x4E58 | *a as u16));
            }
            "link" => {
                need(2)?;
                let (Op::An(a), Op::Imm(e)) = (&ops[0], &ops[1]) else {
                    return Err("link takes an,#displacement".to_string());
                };
                let v = self.number(e, pc)?;
                if !(-0x8000..=0x7FFF).contains(&v) {
                    return Err(format!("link displacement {v} does not fit in 16 bits"));
                }
                return Ok(Insn {
                    parts: vec![Part::Word(v as u16)],
                    opcode: {
                        let a = *a as u16;
                        Box::new(move |_| 0x4E50 | a)
                    },
                });
            }
            "exg" => {
                need(2)?;
                let (x, y, mode) = match (&ops[0], &ops[1]) {
                    (Op::Dn(x), Op::Dn(y)) => (*x, *y, 0x40),
                    (Op::An(x), Op::An(y)) => (*x, *y, 0x48),
                    (Op::Dn(x), Op::An(y)) => (*x, *y, 0x88),
                    (Op::An(y), Op::Dn(x)) => (*x, *y, 0x88),
                    _ => return Err("exg takes two registers".to_string()),
                };
                return Ok(fixed(0xC100 | ((x as u16) << 9) | mode | y as u16));
            }
            "abcd" | "sbcd" => {
                need(2)?;
                let base = if mn == "abcd" { 0xC100 } else { 0x8100 };
                let (rx, ry, m) = match (&ops[0], &ops[1]) {
                    (Op::Dn(y), Op::Dn(x)) => (*x, *y, 0),
                    (Op::Pre(y), Op::Pre(x)) => (*x, *y, 8),
                    _ => return Err(format!("{mn} takes dy,dx or -(ay),-(ax)")),
                };
                return Ok(fixed(base | ((rx as u16) << 9) | m | ry as u16));
            }
            "movep" => {
                need(2)?;
                let long = match sz {
                    Some(Sz::L) => true,
                    Some(Sz::W) | None => false,
                    Some(Sz::B) => return Err("movep is .w or .l".to_string()),
                };
                let (dn, mem, to_mem) = match (&ops[0], &ops[1]) {
                    (Op::Dn(d), m @ (Op::Disp(..) | Op::Ind(_))) => (*d, m, true),
                    (m @ (Op::Disp(..) | Op::Ind(_)), Op::Dn(d)) => (*d, m, false),
                    _ => return Err("movep takes dn,d(an) or d(an),dn".to_string()),
                };
                let (a, disp) = match mem {
                    Op::Disp(e, a) => (*a, e.clone()),
                    Op::Ind(a) => (*a, "0".to_string()),
                    _ => unreachable!(),
                };
                let v = self.number(&disp, pc)?;
                if !(-0x8000..=0x7FFF).contains(&v) {
                    return Err(format!("displacement {v} does not fit in 16 bits"));
                }
                let opmode = 4 | (long as u16) | ((to_mem as u16) << 1);
                return Ok(Insn {
                    parts: vec![Part::Word(v as u16)],
                    opcode: Box::new(move |_| {
                        0x0008 | ((dn as u16) << 9) | (opmode << 6) | a as u16
                    }),
                });
            }
            _ => {}
        }
        // Instructions with an effective address.
        match mn {
            "lea" => {
                need(2)?;
                let Op::An(a) = &ops[1] else {
                    return Err("lea takes ea,an".to_string());
                };
                let a = *a as u16;
                return Ok(with_ea(ops[0].clone(), Sz::L, move |f| {
                    0x41C0 | (a << 9) | f
                }));
            }
            "pea" => {
                need(1)?;
                return Ok(with_ea(ops[0].clone(), Sz::L, |f| 0x4840 | f));
            }
            "jmp" => {
                need(1)?;
                return Ok(with_ea(ops[0].clone(), Sz::L, |f| 0x4EC0 | f));
            }
            "jsr" => {
                need(1)?;
                return Ok(with_ea(ops[0].clone(), Sz::L, |f| 0x4E80 | f));
            }
            "nbcd" => {
                need(1)?;
                return Ok(with_ea(ops[0].clone(), Sz::B, |f| 0x4800 | f));
            }
            "tas" => {
                need(1)?;
                return Ok(with_ea(ops[0].clone(), Sz::B, |f| 0x4AC0 | f));
            }
            "mulu" | "muls" | "divu" | "divs" | "chk" => {
                need(2)?;
                let Op::Dn(d) = &ops[1] else {
                    return Err(format!("{mn} takes ea,dn"));
                };
                let base = match mn {
                    "mulu" => 0xC0C0,
                    "muls" => 0xC1C0,
                    "divu" => 0x80C0,
                    "divs" => 0x81C0,
                    _ => 0x4180,
                };
                let d = *d as u16;
                return Ok(with_ea(ops[0].clone(), Sz::W, move |f| base | (d << 9) | f));
            }
            "clr" | "neg" | "negx" | "not" | "tst" => {
                need(1)?;
                let s = size_w(sz);
                let base = match mn {
                    "clr" => 0x4200,
                    "neg" => 0x4400,
                    "negx" => 0x4000,
                    "not" => 0x4600,
                    _ => 0x4A00,
                };
                return Ok(with_ea(ops[0].clone(), s, move |f| {
                    base | (s.bits() << 6) | f
                }));
            }
            _ => {}
        }
        match mn {
            "move" | "movea" => return self.move_insn(mn, sz, &ops, pc),
            "movem" => return self.movem(sz, &ops, raw),
            "add" | "sub" | "adda" | "suba" | "addi" | "subi" | "addq" | "subq" | "addx"
            | "subx" | "cmp" | "cmpa" | "cmpi" | "cmpm" | "and" | "or" | "eor" | "andi" | "ori"
            | "eori" => {
                return self.arith(mn, sz, &ops, pc);
            }
            "asl" | "asr" | "lsl" | "lsr" | "rol" | "ror" | "roxl" | "roxr" => {
                return self.shift(mn, sz, &ops, pc);
            }
            "btst" | "bchg" | "bclr" | "bset" => return self.bit(mn, &ops, pc),
            _ => {}
        }
        // Branches, DBcc, Scc.
        if let Some(cc) = mn
            .strip_prefix("db")
            .and_then(|c| if c == "ra" { Some(1) } else { condition(c) })
        {
            need(2)?;
            let Op::Dn(d) = &ops[0] else {
                return Err("dbcc takes dn,label".to_string());
            };
            let target = match &ops[1] {
                Op::Bare(e) | Op::AbsL(e) => e.clone(),
                _ => return Err("dbcc takes dn,label".to_string()),
            };
            let d = *d as u16;
            return Ok(Insn {
                parts: vec![Part::WordFrom(Box::new(move |a, at| {
                    a.rel16(&target, at, "dbcc")
                }))],
                opcode: Box::new(move |_| 0x50C8 | (cc << 8) | d),
            });
        }
        if let Some(cc) = mn.strip_prefix('b').and_then(|c| match c {
            "ra" => Some(0),
            "sr" => Some(1),
            _ => condition(c).filter(|&cc| cc > 1),
        }) {
            need(1)?;
            let target = match &ops[0] {
                Op::Bare(e) | Op::AbsL(e) => e.clone(),
                _ => return Err("a branch takes a label or an address".to_string()),
            };
            return self.branch(cc, sz, &target, pc);
        }
        if let Some(cc) = mn.strip_prefix('s').and_then(condition) {
            need(1)?;
            return Ok(with_ea(ops[0].clone(), Sz::B, move |f| {
                0x50C0 | (cc << 8) | f
            }));
        }
        Err(format!("unknown instruction \"{mn}\""))
    }

    /// A word displacement to `target` from the extension word at `at`; in the first pass a target that is not yet
    /// known is not range checked.
    fn rel16(&self, target: &str, at: i64, what: &str) -> Result<u16, String> {
        let before = self.unresolved.replace(false);
        let t = self.number(target, at - 2);
        let unresolved = self.unresolved.replace(before);
        let t = t?;
        if unresolved && !self.strict {
            return Ok(0);
        }
        let disp = wrap24(t - at);
        if !(-0x8000..=0x7FFF).contains(&disp) {
            return Err(format!(
                "the target is {disp} bytes away: too far for {what}"
            ));
        }
        Ok(disp as u16)
    }

    fn branch(&mut self, cc: u16, sz: Option<Sz>, target: &str, pc: i64) -> Result<Insn, String> {
        let known = |a: &Self| {
            let before = a.unresolved.replace(false);
            let t = a.number(target, pc);
            let unresolved = a.unresolved.replace(before);
            (t, unresolved)
        };
        let short = match sz {
            Some(Sz::B) => true,
            Some(Sz::W) => false,
            Some(Sz::L) => return Err("a branch is .s or .w on the 68000".to_string()),
            None if self.autobranch => {
                let idx = self.branch_seen;
                self.branch_seen += 1;
                if self.strict {
                    self.branch_short.get(idx).copied().unwrap_or(false)
                } else {
                    // Pass 1: short only for a target already behind the branch and near.
                    let (t, unresolved) = known(self);
                    let short = !unresolved
                        && t.is_ok_and(|t| {
                            let d = t - (pc + 2);
                            (-128..0).contains(&d)
                        });
                    self.branch_short.push(short);
                    short
                }
            }
            None => {
                return Err(format!(
                    "the size of this branch is not given: write {0}.s or {0}.w, or turn on `.opt autobranch`",
                    ["bra", "bsr"].get(cc as usize).copied().unwrap_or("bcc")
                ));
            }
        };
        if short {
            let (t, unresolved) = known(self);
            let t = t?;
            let disp = wrap24(t - (pc + 2));
            let byte = if unresolved && !self.strict {
                2
            } else if (-128..=127).contains(&disp) && disp != 0 {
                disp as u8 as u16
            } else {
                return Err(format!(
                    "the target is {disp} bytes away: not a short branch"
                ));
            };
            return Ok(fixed(0x6000 | (cc << 8) | byte));
        }
        let target = target.to_string();
        Ok(Insn {
            parts: vec![Part::WordFrom(Box::new(move |a, at| {
                a.rel16(&target, at, "a branch")
            }))],
            opcode: Box::new(move |_| 0x6000 | (cc << 8)),
        })
    }

    fn move_insn(&mut self, mn: &str, sz: Option<Sz>, ops: &[Op], pc: i64) -> Result<Insn, String> {
        if ops.len() != 2 {
            return Err(format!("{mn} takes two operands"));
        }
        let (src, dst) = (&ops[0], &ops[1]);
        // The status register forms.
        match (src, dst) {
            (Op::Sr, d) => {
                return Ok(with_ea(d.clone(), Sz::W, |f| 0x40C0 | f));
            }
            (s, Op::Sr) => {
                return Ok(with_ea(s.clone(), Sz::W, |f| 0x46C0 | f));
            }
            (s, Op::Ccr) => {
                return Ok(with_ea(s.clone(), Sz::W, |f| 0x44C0 | f));
            }
            (Op::An(a), Op::Usp) => {
                return Ok(fixed(0x4E60 | *a as u16));
            }
            (Op::Usp, Op::An(a)) => {
                return Ok(fixed(0x4E68 | *a as u16));
            }
            _ => {}
        }
        let _ = pc;
        let s = size_w_default(sz);
        let size_bits = match s {
            Sz::B => 0x1000,
            Sz::W => 0x3000,
            Sz::L => 0x2000,
        };
        if let Op::An(a) = dst {
            if s == Sz::B {
                return Err("movea is .w or .l".to_string());
            }
            let a = *a as u16;
            return Ok(with_ea(src.clone(), s, move |f| {
                size_bits | (a << 9) | (1 << 6) | f
            }));
        }
        if mn == "movea" {
            return Err("movea takes an address register destination".to_string());
        }
        Ok(Insn {
            parts: vec![Part::Ea(src.clone(), s), Part::Ea(dst.clone(), s)],
            opcode: Box::new(move |fs| size_bits | swap_ea(fs[1]) | fs[0]),
        })
    }

    fn movem(&mut self, sz: Option<Sz>, ops: &[Op], raw: &[&str]) -> Result<Insn, String> {
        if ops.len() != 2 {
            return Err("movem takes a register list and an address".to_string());
        }
        let s = match sz {
            Some(Sz::L) => Sz::L,
            Some(Sz::W) | None => Sz::W,
            Some(Sz::B) => return Err("movem is .w or .l".to_string()),
        };
        let l = (s == Sz::L) as u16;
        let list = |i: usize| parse_list(raw[i].trim());
        let is_list_like =
            |i: usize| matches!(ops[i], Op::Dn(_) | Op::An(_)) || list(i).is_some_and(|_| true);
        let mem = |op: &Op| !matches!(op, Op::Dn(_) | Op::An(_));
        if mem(&ops[1]) && is_list_like(0) {
            let mask = list(0).ok_or("movem takes a register list like d0-d7/a0-a6")?;
            let predec = matches!(ops[1], Op::Pre(_));
            let mask = if predec { mask.reverse_bits() } else { mask };
            return Ok(
                with_ea_then(ops[1].clone(), s, vec![], move |f| 0x4880 | (l << 6) | f)
                    .with_leading_word(mask),
            );
        }
        if mem(&ops[0]) && is_list_like(1) {
            let mask = list(1).ok_or("movem takes a register list like d0-d7/a0-a6")?;
            return Ok(
                with_ea_then(ops[0].clone(), s, vec![], move |f| 0x4C80 | (l << 6) | f)
                    .with_leading_word(mask),
            );
        }
        Err("movem takes a register list and a memory address".to_string())
    }

    fn arith(&mut self, mn: &str, sz: Option<Sz>, ops: &[Op], pc: i64) -> Result<Insn, String> {
        if ops.len() != 2 {
            return Err(format!("{mn} takes two operands"));
        }
        let s = size_w_default(sz);
        let (src, dst) = (&ops[0], &ops[1]);
        // `andi #n,ccr` / `andi #n,sr` and the generic spellings of them.
        if matches!(dst, Op::Ccr | Op::Sr)
            && matches!(mn, "andi" | "ori" | "eori" | "and" | "or" | "eor")
        {
            let Op::Imm(e) = src else {
                return Err(format!(
                    "{mn} to the status register takes an immediate value"
                ));
            };
            let v = self.number(e, pc)?;
            let base = match (mn.trim_end_matches('i'), matches!(dst, Op::Sr)) {
                ("and", false) => 0x023C,
                ("and", true) => 0x027C,
                ("or", false) => 0x003C,
                ("or", true) => 0x007C,
                ("eor", false) => 0x0A3C,
                _ => 0x0A7C,
            };
            if matches!(dst, Op::Ccr) && !(-128..=255).contains(&v) {
                return Err(format!("{v} does not fit in a byte"));
            }
            let w = if matches!(dst, Op::Ccr) {
                v as u8 as u16
            } else {
                v as u16
            };
            return Ok(Insn {
                parts: vec![Part::Word(w)],
                opcode: Box::new(move |_| base),
            });
        }
        let kind = mn.trim_end_matches(['i', 'a', 'q', 'x']);
        let kind = match mn {
            "cmpm" => "cmpm",
            "adda" | "suba" | "cmpa" => &mn[..mn.len() - 1],
            _ => kind,
        };
        let bits = s.bits();
        // Explicit and implied immediate forms.
        let immediate_op = |base: u16| -> Result<Insn, String> {
            Ok(
                with_ea_then(dst.clone(), s, vec![], move |f| base | (bits << 6) | f)
                    .with_leading_ea(src.clone(), s),
            )
        };
        match mn {
            "addq" | "subq" => {
                let Op::Imm(e) = src else {
                    return Err(format!("{mn} takes #1-8,ea"));
                };
                let v = self.number(e, pc)?;
                if !(1..=8).contains(&v) {
                    return Err(format!("{mn} takes 1 to 8, not {v}"));
                }
                if s == Sz::B && matches!(dst, Op::An(_)) {
                    return Err(format!("{mn}.b cannot go to an address register"));
                }
                let q = (v as u16 & 7) << 9;
                let base = if mn == "addq" { 0x5000 } else { 0x5100 };
                return Ok(with_ea(dst.clone(), s, move |f| base | q | (bits << 6) | f));
            }
            "addi" | "subi" | "cmpi" | "andi" | "ori" | "eori" => {
                if !matches!(src, Op::Imm(_)) {
                    return Err(format!("{mn} takes #value,ea"));
                }
                let base = match mn {
                    "addi" => 0x0600,
                    "subi" => 0x0400,
                    "cmpi" => 0x0C00,
                    "andi" => 0x0200,
                    "ori" => 0x0000,
                    _ => 0x0A00,
                };
                return immediate_op(base);
            }
            "addx" | "subx" => {
                let base = if mn == "addx" { 0xD100 } else { 0x9100 };
                let (rx, ry, m) = match (src, dst) {
                    (Op::Dn(y), Op::Dn(x)) => (*x, *y, 0),
                    (Op::Pre(y), Op::Pre(x)) => (*x, *y, 8),
                    _ => return Err(format!("{mn} takes dy,dx or -(ay),-(ax)")),
                };
                return Ok(fixed(
                    base | ((rx as u16) << 9) | (bits << 6) | m | ry as u16,
                ));
            }
            "cmpm" => {
                let (Op::Post(y), Op::Post(x)) = (src, dst) else {
                    return Err("cmpm takes (ay)+,(ax)+".to_string());
                };
                return Ok(fixed(0xB108 | ((*x as u16) << 9) | (bits << 6) | *y as u16));
            }
            _ => {}
        }
        // The generic spellings.
        let family = match kind {
            "add" => (0xD000, 0x0600),
            "sub" => (0x9000, 0x0400),
            "cmp" => (0xB000, 0x0C00),
            "and" => (0xC000, 0x0200),
            "or" => (0x8000, 0x0000),
            "eor" => (0xB000, 0x0A00),
            _ => return Err(format!("unknown instruction \"{mn}\"")),
        };
        let (reg_base, imm_base) = family;
        if let Op::An(a) = dst
            && matches!(kind, "add" | "sub" | "cmp")
        {
            if s == Sz::B {
                return Err(format!("{mn}.b cannot go to an address register"));
            }
            let a = *a as u16;
            let opmode = if s == Sz::W { 3 } else { 7 };
            return Ok(with_ea(src.clone(), s, move |f| {
                reg_base | (a << 9) | (opmode << 6) | f
            }));
        }
        if mn.ends_with('a') && !matches!(dst, Op::An(_)) {
            return Err(format!("{mn} takes an address register destination"));
        }
        if matches!(src, Op::Imm(_)) && !(matches!(dst, Op::Dn(_)) && kind != "eor") {
            return immediate_op(imm_base);
        }
        if let Op::Dn(d) = dst
            && kind != "eor"
        {
            let d = *d as u16;
            return Ok(with_ea(src.clone(), s, move |f| {
                reg_base | (d << 9) | (bits << 6) | f
            }));
        }
        if let Op::Dn(d) = src
            && kind != "cmp"
            && Self::data_alterable_or_dn(dst)
        {
            let d = *d as u16;
            return Ok(with_ea(dst.clone(), s, move |f| {
                reg_base | (d << 9) | (4 << 6) | (bits << 6) | f
            }));
        }
        Err(format!("{mn}: no such combination of operands"))
    }

    fn shift(&mut self, mn: &str, sz: Option<Sz>, ops: &[Op], pc: i64) -> Result<Insn, String> {
        let (stem, left) = mn.split_at(mn.len() - 1);
        let kind = match stem {
            "as" => 0,
            "ls" => 1,
            "rox" => 2,
            _ => 3,
        };
        let dir = (left == "l") as u16;
        if ops.len() == 1 {
            // A word-sized memory shift by one.
            if sz.is_some_and(|s| s != Sz::W) {
                return Err(format!("{mn} of memory is .w"));
            }
            return Ok(with_ea(ops[0].clone(), Sz::W, move |f| {
                0xE0C0 | (kind << 9) | (dir << 8) | f
            }));
        }
        if ops.len() != 2 {
            return Err(format!("{mn} takes a count and a data register"));
        }
        let Op::Dn(d) = &ops[1] else {
            return Err(format!("{mn} takes a count and a data register"));
        };
        let s = size_w_default(sz);
        let (count, by_reg) = match &ops[0] {
            Op::Imm(e) => {
                let v = self.number(e, pc)?;
                if !(1..=8).contains(&v) {
                    return Err(format!("a shift count is 1 to 8, not {v}"));
                }
                (v as u16 & 7, 0)
            }
            Op::Dn(r) => (*r as u16, 1),
            _ => return Err(format!("{mn} takes #count or dn as the count")),
        };
        Ok(fixed(
            0xE000
                | (count << 9)
                | (dir << 8)
                | (s.bits() << 6)
                | (by_reg << 5)
                | (kind << 3)
                | *d as u16,
        ))
    }

    fn bit(&mut self, mn: &str, ops: &[Op], pc: i64) -> Result<Insn, String> {
        if ops.len() != 2 {
            return Err(format!("{mn} takes a bit number and an operand"));
        }
        let t = match mn {
            "btst" => 0,
            "bchg" => 1,
            "bclr" => 2,
            _ => 3,
        };
        match &ops[0] {
            Op::Dn(d) => {
                let d = *d as u16;
                Ok(with_ea(ops[1].clone(), Sz::B, move |f| {
                    0x0100 | (d << 9) | (t << 6) | f
                }))
            }
            Op::Imm(e) => {
                let v = self.number(e, pc)?;
                if !(0..=255).contains(&v) {
                    return Err(format!("bit number {v} does not fit in a byte"));
                }
                Ok(with_ea_then(ops[1].clone(), Sz::B, vec![], move |f| {
                    0x0800 | (t << 6) | f
                })
                .with_leading_word(v as u16))
            }
            _ => Err(format!("{mn} takes #bit or dn as the bit number")),
        }
    }
}

fn size_w_default(sz: Option<Sz>) -> Sz {
    sz.unwrap_or(Sz::W)
}

impl Insn {
    /// A word that comes right after the opcode, before any operand extension words (MOVEM's mask, a static bit
    /// number).
    fn with_leading_word(mut self, w: u16) -> Insn {
        self.parts.insert(0, Part::Word(w));
        // The leading part is not an effective address: the opcode closure reads the EA's field from `fs[0]`, which
        // stays the first EA field.
        self
    }

    /// An effective address operand that comes first (an immediate source: its words precede the destination's).
    fn with_leading_ea(mut self, op: Op, sz: Sz) -> Insn {
        self.parts.insert(0, Part::Ea(op, sz));
        let inner = self.opcode;
        // The destination is now the second EA field.
        self.opcode = Box::new(move |fs| inner(&fs[1..]));
        self
    }
}

/// The statements of one pass.
impl Asm<'_> {
    fn emit(&mut self, bytes: &[u8]) {
        match self.sec {
            Sec::Bss => {}
            s => self.out[s as usize].extend_from_slice(bytes),
        }
        self.off[self.sec as usize] += bytes.len() as u32;
    }

    fn reserve(&mut self, n: u32) {
        match self.sec {
            Sec::Bss => self.off[2] += n,
            s => {
                self.out[s as usize].extend(std::iter::repeat_n(0u8, n as usize));
                self.off[s as usize] += n;
            }
        }
    }

    fn define(&mut self, line: &Line, name: &str, def: Def) {
        if !self.strict {
            if self.symbols.contains_key(name) {
                self.err(line, format!("\"{name}\" is defined twice"));
                return;
            }
            if !self.symbols.contains_key(name) {
                self.order.push(name.to_string());
            }
            self.symbols.insert(name.to_string(), def);
        } else if matches!(def, Def::Lazy(_)) {
            // Settled in pass 1 as far as it could be; in pass 2 everything is known.
            if let Ok(v) = self.number(
                match &def {
                    Def::Lazy(t) => t,
                    _ => unreachable!(),
                },
                self.pc(),
            ) {
                self.symbols.insert(name.to_string(), Def::Const(v));
            }
        }
    }

    fn statement(&mut self, line: &Line) {
        let code = strip_comment(&line.text);
        let mut rest = code.trim_end();
        if rest.trim().is_empty() {
            return;
        }
        let start = self.listing_start();
        // A label starts in column 0.
        if rest.starts_with('*') {
            return;
        }
        if !rest.starts_with(char::is_whitespace) && !rest.starts_with('.') {
            let end = rest
                .find(|c: char| c.is_whitespace() || c == ':' || c == '=')
                .unwrap_or(rest.len());
            let name = rest[..end].to_string();
            let after = rest[end..].trim_start();
            let after = after.strip_prefix(':').unwrap_or(after).trim_start();
            let eq_form = after.strip_prefix('=').map(str::trim).or_else(|| {
                let word_end = after.find(char::is_whitespace).unwrap_or(after.len());
                (after[..word_end].eq_ignore_ascii_case("equ")
                    || after[..word_end].eq_ignore_ascii_case(".equ")
                    || after[..word_end].eq_ignore_ascii_case(".set"))
                .then(|| after[word_end..].trim())
            });
            if let Some(value) = eq_form {
                let def = match self.number(value, self.pc()) {
                    Ok(v) if !self.unresolved.get() => Def::Const(v),
                    _ => {
                        self.unresolved.set(false);
                        Def::Lazy(value.to_string())
                    }
                };
                self.define(line, &name, def);
                return;
            }
            if !name.is_empty() && !name.starts_with('.') {
                self.define(
                    line,
                    &name,
                    Def::Label(self.sec, self.off[self.sec as usize]),
                );
            }
            rest = after;
            if rest.is_empty() {
                return;
            }
        }
        let rest = rest.trim();
        let (word, operands) = match rest.find(char::is_whitespace) {
            Some(i) => (&rest[..i], rest[i..].trim()),
            None => (rest, ""),
        };
        let word_lower = word.to_ascii_lowercase();
        let (stem, suffix) = match word_lower.rsplit_once('.') {
            Some((s, x)) if !s.is_empty() && matches!(x, "b" | "w" | "l" | "s") => {
                (s.to_string(), Some(x.to_string()))
            }
            _ => (word_lower.clone(), None),
        };
        let stem = stem.trim_start_matches('.');
        let result =
            self.directive_or_instruction(line, stem, suffix.as_deref(), operands, &word_lower);
        if let Err(message) = result {
            self.err(line, message);
        }
        if self.strict {
            self.list(line, start);
        }
    }

    fn listing_start(&self) -> (Sec, u32, usize, usize) {
        (
            self.sec,
            self.off[self.sec as usize],
            self.out[0].len(),
            self.out[1].len(),
        )
    }

    fn list(&mut self, line: &Line, start: (Sec, u32, usize, usize)) {
        let (sec, off, t, d) = start;
        let bytes = match sec {
            Sec::Text => self.out[0].get(t..).unwrap_or_default().to_vec(),
            Sec::Data => self.out[1].get(d..).unwrap_or_default().to_vec(),
            Sec::Bss => Vec::new(),
        };
        if sec != self.sec {
            return;
        }
        self.listing.push(M68kListing {
            file: line.file.to_string(),
            line: line.line,
            addr: (self.base[sec as usize] + off as i64) as u32,
            bytes,
            source: line.text.trim_end().to_string(),
        });
    }

    fn directive_or_instruction(
        &mut self,
        line: &Line,
        stem: &str,
        suffix: Option<&str>,
        operands: &str,
        word: &str,
    ) -> Result<(), String> {
        let sz = suffix.and_then(|s| match s {
            "b" | "s" => Some(Sz::B),
            "w" => Some(Sz::W),
            "l" => Some(Sz::L),
            _ => None,
        });
        let pc = self.pc();
        match stem {
            "text" | "data" | "bss" => {
                self.sec = match stem {
                    "text" => Sec::Text,
                    "data" => Sec::Data,
                    _ => Sec::Bss,
                };
                return Ok(());
            }
            "section" => {
                let name = operands.trim().trim_start_matches('.').to_ascii_lowercase();
                self.sec = match name.as_str() {
                    "text" | "code" => Sec::Text,
                    "data" => Sec::Data,
                    "bss" => Sec::Bss,
                    _ => return Err(format!("unknown section \"{name}\": text, data or bss")),
                };
                return Ok(());
            }
            "globl" | "global" | "xdef" | "xref" | "extern" => return Ok(()),
            "even" => {
                if self.off[self.sec as usize] % 2 == 1 {
                    self.reserve(1);
                }
                return Ok(());
            }
            "opt" => {
                for o in operands.split(',').map(|o| o.trim().to_ascii_lowercase()) {
                    match o.as_str() {
                        "autobranch" => self.autobranch = true,
                        "noautobranch" => self.autobranch = false,
                        "" => {}
                        other => return Err(format!("unknown option \"{other}\" (autobranch)")),
                    }
                }
                return Ok(());
            }
            "org" => {
                if self.opts.relocatable {
                    return Err(".org has no meaning in a relocatable program".to_string());
                }
                let v = self.number(operands, pc)?;
                if !(0..=0xFF_FFFF).contains(&v) {
                    return Err(format!(".org {v:#X} is beyond the 24-bit address space"));
                }
                if self.org.is_some() || self.off != [0, 0, 0] {
                    return Err(".org must come first, once".to_string());
                }
                self.org = Some(v as u32);
                if !self.strict {
                    // The first pass needs the origin for addresses it meets before the sizes are settled.
                    self.base = [v, v, v];
                }
                return Ok(());
            }
            "dc" | "ds" => {
                let s = sz.unwrap_or(Sz::W);
                let unit = match s {
                    Sz::B => 1,
                    Sz::W => 2,
                    Sz::L => 4,
                };
                if stem == "ds" {
                    let n = self.number(operands, pc)?;
                    if !(0..=0x100_0000).contains(&n) {
                        return Err(format!("ds count {n} is out of range"));
                    }
                    self.reserve(n as u32 * unit);
                    return Ok(());
                }
                if self.sec == Sec::Bss {
                    return Err("dc in the bss section (only ds reserves space there)".to_string());
                }
                if s != Sz::B && self.off[self.sec as usize] % 2 == 1 {
                    return Err("dc.w/dc.l at an odd address: put .even before it".to_string());
                }
                for item in split_top(operands) {
                    if let Some(lit) = crate::text::as_string_literal(item.trim()) {
                        if s != Sz::B {
                            return Err("a string goes in dc.b".to_string());
                        }
                        let bytes = lit?;
                        self.emit(&bytes);
                        continue;
                    }
                    let at = self.pc();
                    let (v, reloc) = self.value(item, at)?;
                    match s {
                        Sz::B => {
                            if !(-128..=255).contains(&v) || reloc {
                                return Err(format!("{item} does not fit in a byte"));
                            }
                            self.emit(&[v as u8]);
                        }
                        Sz::W => {
                            if !(-0x8000..=0xFFFF).contains(&v) || reloc {
                                return Err(format!("{item} does not fit in a word"));
                            }
                            self.emit(&be16(v as u16));
                        }
                        Sz::L => {
                            if !(-0x8000_0000..=0xFFFF_FFFF).contains(&v) {
                                return Err(format!("{item} does not fit in 32 bits"));
                            }
                            if reloc {
                                self.note_reloc(0);
                            }
                            self.emit(&(v as u32).to_be_bytes());
                        }
                    }
                }
                return Ok(());
            }
            "include" => return self.include(line, operands),
            _ => {}
        }
        if self.sec == Sec::Bss {
            return Err("an instruction in the bss section".to_string());
        }
        if self.off[self.sec as usize] % 2 == 1 {
            return Err("an instruction at an odd address: put .even before it".to_string());
        }
        let _ = word;
        let raw: Vec<&str> = split_top(operands);
        let insn = self.instruction(stem, sz, &raw, pc)?;
        let parts = self.build(&insn, pc)?;
        self.emit_insn(parts, pc)
    }

    fn include(&mut self, _line: &Line, _operands: &str) -> Result<(), String> {
        Err(".include is handled when the lines are read".to_string())
    }

    /// Records that the long about to be written at the end of the current section is an address.
    fn note_reloc(&mut self, delta: u32) {
        if self.strict && self.opts.relocatable {
            let off = self.off[self.sec as usize] + delta;
            let image = match self.sec {
                Sec::Text => off,
                Sec::Data => self.off_text_len() + off,
                Sec::Bss => return,
            };
            self.relocs.push(image);
        }
    }

    /// The length of the text section as pass 1 settled it (the data section follows it in the image).
    fn off_text_len(&self) -> u32 {
        (self.base[1] - self.base[0]) as u32
    }

    fn emit_insn(&mut self, parts: Vec<Ext>, pc: i64) -> Result<(), String> {
        let mut bytes = Vec::new();
        let mut marks = Vec::new();
        for p in &parts {
            match p {
                Ext::W(w) => bytes.extend(be16(*w)),
                Ext::L(v, reloc) => {
                    if *reloc {
                        marks.push(bytes.len() as u32);
                    }
                    bytes.extend(v.to_be_bytes());
                }
            }
        }
        // The decoder must know the word: this is what refuses an operand combination the 68000 does not have.
        let (text, len) = mc68000::disassemble::disassemble(&bytes, pc as u32);
        if text.starts_with("dc.") || len != bytes.len() {
            return Err("this combination of operands is not a 68000 instruction".to_string());
        }
        for m in marks {
            self.note_reloc(m);
        }
        self.emit(&bytes);
        Ok(())
    }
}

/// The source lines with `.include`s expanded.
fn read_lines(
    name: &str,
    source: &str,
    opts: &M68kOptions,
    files: &dyn Files,
    out: &mut Vec<Line>,
    errors: &mut Vec<Diag>,
    depth: u32,
) {
    let file: Rc<str> = Rc::from(name);
    for (i, raw) in source.lines().enumerate() {
        let code = strip_comment(raw).trim();
        let lower = code.to_ascii_lowercase();
        let rest = lower
            .strip_prefix(".include")
            .or_else(|| lower.strip_prefix("include"))
            .filter(|r| r.starts_with([' ', '\t']));
        if let Some(rest) = rest {
            let at = code.len() - rest.len();
            let arg = code[at..].trim();
            let Some(Ok(bytes)) = crate::text::as_string_literal(arg) else {
                errors.push(Diag::new(name, i + 1, ".include takes a quoted file name"));
                continue;
            };
            if depth >= 16 {
                errors.push(Diag::new(
                    name,
                    i + 1,
                    ".include nests too deep (a file includes itself?)",
                ));
                continue;
            }
            let path = String::from_utf8_lossy(&bytes).to_string();
            let mut found = None;
            let candidates: Vec<PathBuf> = std::iter::once(PathBuf::from(&path))
                .chain(opts.include_dirs.iter().map(|d| d.join(&path)))
                .collect();
            for c in candidates {
                if let Ok(b) = files.read(&c) {
                    found = Some((c, b));
                    break;
                }
            }
            match found {
                Some((p, b)) => {
                    let text = String::from_utf8_lossy(&b).to_string();
                    read_lines(
                        &p.display().to_string(),
                        &text,
                        opts,
                        files,
                        out,
                        errors,
                        depth + 1,
                    );
                }
                None => errors.push(Diag::new(name, i + 1, format!("cannot find {path}"))),
            }
            continue;
        }
        out.push(Line {
            file: file.clone(),
            line: i + 1,
            text: raw.to_string(),
        });
    }
}

/// Assembles `source`. Errors come with file and line; at most 50 are collected.
pub fn assemble_m68k(
    name: &str,
    source: &str,
    opts: &M68kOptions,
    files: &dyn Files,
) -> Result<M68kAssembly, Vec<Diag>> {
    let mut lines = Vec::new();
    let mut errors = Vec::new();
    read_lines(name, source, opts, files, &mut lines, &mut errors, 0);
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut asm = Asm::new(opts);
    // Pass 1: sizes and symbols.
    for l in &lines {
        asm.statement(l);
    }
    if !asm.errors.is_empty() {
        return Err(asm.errors);
    }
    let sizes = asm.off;
    let text_len = sizes[0] + (sizes[0] & 1);
    let data_len = sizes[1] + (sizes[1] & 1);
    let origin = asm.org.unwrap_or(0) as i64;
    let bases = [
        origin,
        origin + text_len as i64,
        origin + (text_len + data_len) as i64,
    ];
    // Pass 2: values.
    asm.strict = true;
    asm.base = bases;
    asm.out = [Vec::new(), Vec::new()];
    asm.off = [0; 3];
    asm.sec = Sec::Text;
    asm.branch_seen = 0;
    asm.org = None;
    for l in &lines {
        asm.statement(l);
        if asm.errors.len() >= 50 {
            break;
        }
    }
    if !asm.errors.is_empty() {
        return Err(asm.errors);
    }
    if asm.off != sizes {
        return Err(vec![Diag::new(
            name,
            0,
            "internal error: the sizes of the two passes differ",
        )]);
    }
    let [mut text, mut data] = std::mem::take(&mut asm.out);
    if text.len() % 2 == 1 {
        text.push(0);
    }
    if data.len() % 2 == 1 {
        data.push(0);
    }
    let mut symbols: Vec<M68kSymbol> = Vec::new();
    for n in &asm.order {
        let Some(def) = asm.symbols.get(n) else {
            continue;
        };
        let (value, label) = match def {
            Def::Label(sec, off) => (asm.base[*sec as usize] + *off as i64, true),
            Def::Const(v) => (*v, false),
            Def::Lazy(_) => continue,
        };
        symbols.push(M68kSymbol {
            name: n.clone(),
            value,
            label,
        });
    }
    symbols.sort_by(|a, b| (a.value, &a.name).cmp(&(b.value, &b.name)));
    let mut relocs = std::mem::take(&mut asm.relocs);
    relocs.sort_unstable();
    relocs.dedup();
    Ok(M68kAssembly {
        text,
        data,
        bss: sizes[2] + (sizes[2] & 1),
        origin: origin as u32,
        relocs,
        symbols,
        listing: asm.listing,
    })
}

/// `NAME = $VALUE` (the symbol file `retroclawd --symbols` and `rcdis --symbols` read); constants only with `all`.
pub fn symbols_text(asm: &M68kAssembly, all: bool) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for sym in &asm.symbols {
        if all || sym.label {
            let _ = writeln!(out, "{} = ${:06X}", sym.name, sym.value & 0xFFFF_FFFF);
        }
    }
    out
}

/// `ADDR  BYTES  source`, four bytes per row.
pub fn listing_text(asm: &M68kAssembly) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for entry in &asm.listing {
        let mut rows = entry.bytes.chunks(4);
        let first: String = rows
            .next()
            .map(|c| c.iter().map(|b| format!("{b:02X} ")).collect())
            .unwrap_or_default();
        let addr = if entry.bytes.is_empty() {
            "      ".to_string()
        } else {
            format!("{:06X}", entry.addr)
        };
        let _ = writeln!(out, "{addr}  {first:<12} {}", entry.source);
        let mut at = entry.addr + 4;
        for chunk in rows {
            let bytes: String = chunk.iter().map(|b| format!("{b:02X} ")).collect();
            let _ = writeln!(out, "{at:06X}  {bytes:<12}");
            at += 4;
        }
    }
    out
}
