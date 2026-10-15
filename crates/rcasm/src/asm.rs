//! The two-pass assembler.
//!
//! The same walk over the source runs twice. The first pass learns every label's address (an
//! instruction's size never depends on a value, except the 6502 choosing zero page or absolute, which
//! the first pass pins: an operand whose symbol was not yet known is absolute in both passes); the
//! second pass evaluates everything strictly and produces the bytes. Includes, macros and conditionals
//! are expanded during the walk, so both passes see exactly the same statements.

use crate::cpu::{self, Cpu, Site};
use crate::expr::{self, EvalError, Scope};
use crate::text::{as_string_literal, split_top, strip_comment};
use rc_common::Diag;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Where the assembler reads files from (`.include`, `.incbin`).
pub trait Files {
    fn read(&self, path: &Path) -> Result<Vec<u8>, String>;
}

pub struct DiskFiles;

impl Files for DiskFiles {
    fn read(&self, path: &Path) -> Result<Vec<u8>, String> {
        std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub cpu: Option<Cpu>,
    /// Allow the undocumented 6502 opcodes.
    pub illegal: bool,
    pub include_dirs: Vec<PathBuf>,
    /// `-D NAME=VALUE`
    pub defines: Vec<(String, i64)>,
}

/// A run of bytes at an address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: u16,
    pub bytes: Vec<u8>,
}

impl Segment {
    pub fn end(&self) -> u32 {
        self.start as u32 + self.bytes.len() as u32
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub value: i64,
    /// A label (an address) rather than a constant from `=` / `equ` / `-D`.
    pub label: bool,
}

/// One statement of the listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListEntry {
    pub file: String,
    pub line: usize,
    pub addr: Option<u16>,
    pub bytes: Vec<u8>,
    pub source: String,
}

#[derive(Debug)]
pub struct Assembly {
    pub cpu: Cpu,
    /// Sorted by address, touching segments merged.
    pub segments: Vec<Segment>,
    /// Sorted by value, then name. Local `@labels` appear as `scope@name`.
    pub symbols: Vec<Symbol>,
    pub listing: Vec<ListEntry>,
}

#[derive(Clone)]
struct LineSrc {
    text: String,
    file: Rc<str>,
    line: usize,
}

struct Macro {
    params: Vec<String>,
    body: Vec<LineSrc>,
}

#[derive(Clone)]
struct Sym {
    value: i64,
    label: bool,
}

struct Cond {
    /// This branch of the conditional is being assembled.
    active: bool,
    /// Some branch already was (so a later `.else` is not).
    taken: bool,
    /// The enclosing region is active.
    parent_active: bool,
    seen_else: bool,
}

const MAX_ERRORS: usize = 50;
const MAX_DEPTH: usize = 32;

struct Asm<'f> {
    opts: Options,
    files: &'f dyn Files,
    cpu: Option<Cpu>,
    illegal: bool,
    pass: u8,
    symbols: HashMap<String, Sym>,
    macros: HashMap<String, Macro>,
    /// Instruction number -> "the first pass found an unknown symbol in the operand".
    wide: HashMap<usize, bool>,
    // per pass
    pc: Option<u32>,
    segments: Vec<Segment>,
    global_scope: String,
    macro_scope: Vec<usize>,
    macro_counter: usize,
    instr_seq: usize,
    conds: Vec<Cond>,
    listing: Vec<ListEntry>,
    errors: Vec<Diag>,
    stop: bool,
}

struct View<'a> {
    symbols: &'a HashMap<String, Sym>,
    scope_prefix: String,
    pc: i64,
}

impl Scope for View<'_> {
    fn symbol(&self, name: &str) -> Option<i64> {
        let key = if name.starts_with('@') {
            format!("{}{name}", self.scope_prefix)
        } else {
            name.to_string()
        };
        self.symbols.get(&key).map(|s| s.value)
    }
    fn pc(&self) -> i64 {
        self.pc
    }
}

/// Assembles `source` (named `name` in messages).
pub fn assemble(
    name: &str,
    source: &str,
    opts: &Options,
    files: &dyn Files,
) -> Result<Assembly, Vec<Diag>> {
    let mut asm = Asm {
        opts: opts.clone(),
        files,
        cpu: opts.cpu,
        illegal: opts.illegal,
        pass: 1,
        symbols: HashMap::new(),
        macros: HashMap::new(),
        wide: HashMap::new(),
        pc: None,
        segments: Vec::new(),
        global_scope: String::new(),
        macro_scope: Vec::new(),
        macro_counter: 0,
        instr_seq: 0,
        conds: Vec::new(),
        listing: Vec::new(),
        errors: Vec::new(),
        stop: false,
    };
    for (n, v) in &opts.defines {
        asm.symbols.insert(
            n.clone(),
            Sym {
                value: *v,
                label: false,
            },
        );
    }
    let root: Rc<str> = Rc::from(name);
    let lines = split_lines(source, &root);
    for pass in 1..=2u8 {
        asm.begin_pass(pass);
        asm.run(lines.clone(), 0);
        if !asm.conds.is_empty() {
            let at = Diag::new(name, 0, "unterminated .if (missing .endif)");
            asm.errors.push(at);
        }
        if !asm.errors.is_empty() {
            return Err(asm.errors);
        }
    }
    asm.finish(name)
}

fn split_lines(source: &str, file: &Rc<str>) -> Vec<LineSrc> {
    source
        .lines()
        .enumerate()
        .map(|(i, l)| LineSrc {
            text: l.to_string(),
            file: Rc::clone(file),
            line: i + 1,
        })
        .collect()
}

/// What a source line is made of.
enum Stmt<'a> {
    Empty,
    /// `labels` then an optional `word operands`.
    Body {
        labels: Vec<&'a str>,
        word: &'a str,
        operand: &'a str,
    },
    Assign {
        labels: Vec<&'a str>,
        name: &'a str,
        value: &'a str,
    },
    Org {
        value: &'a str,
    },
    Labels(Vec<&'a str>),
}

fn take_symbol(text: &str) -> (&str, &str) {
    let end = text
        .char_indices()
        .find(|(_, c)| !expr::is_symbol_char(*c))
        .map_or(text.len(), |(i, _)| i);
    text.split_at(end)
}

fn take_word(text: &str) -> (&str, &str) {
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    (&text[..end], text[end..].trim_start())
}

const DIRECTIVES: &[&str] = &[
    "org", "byte", "db", "defb", "byt", "word", "dw", "defw", "text", "ascii", "asciiz", "str",
    "fill", "ds", "defs", "res", "align", "equ", "set", "include", "incbin", "macro", "endm",
    "endmacro", "if", "ifdef", "ifndef", "else", "endif", "cpu", "illegal", "error", "assert",
    "end",
];

/// Names that are directives even without a leading dot (Z80 assemblers write them bare).
const BARE_DIRECTIVES: &[&str] = &[
    "org", "db", "dw", "defb", "defw", "defs", "ds", "equ", "end", "include", "incbin",
];

fn directive_name(word: &str) -> Option<String> {
    let lower = word.to_ascii_lowercase();
    if let Some(bare) = lower.strip_prefix('.') {
        return DIRECTIVES.contains(&bare).then(|| bare.to_string());
    }
    BARE_DIRECTIVES.contains(&lower.as_str()).then_some(lower)
}

impl<'f> Asm<'f> {
    fn begin_pass(&mut self, pass: u8) {
        self.pass = pass;
        self.pc = None;
        self.segments.clear();
        self.global_scope.clear();
        self.macro_scope.clear();
        self.macro_counter = 0;
        self.instr_seq = 0;
        self.conds.clear();
        self.listing.clear();
        self.stop = false;
        self.illegal = self.opts.illegal;
        self.cpu = self.opts.cpu;
    }

    fn error(&mut self, src: &LineSrc, message: impl Into<String>) {
        if self.errors.len() < MAX_ERRORS {
            self.errors
                .push(Diag::new(&*src.file, src.line, message.into()));
        }
    }

    fn active(&self) -> bool {
        self.conds.last().is_none_or(|c| c.active)
    }

    fn scope_prefix(&self) -> String {
        match self.macro_scope.last() {
            Some(id) => format!("{}#{id}|", self.global_scope),
            None => format!("{}|", self.global_scope),
        }
    }

    fn view(&self) -> View<'_> {
        View {
            symbols: &self.symbols,
            scope_prefix: self.scope_prefix(),
            pc: self.pc.unwrap_or(0) as i64,
        }
    }

    fn key(&self, name: &str) -> String {
        if name.starts_with('@') {
            format!("{}{name}", self.scope_prefix())
        } else {
            name.to_string()
        }
    }

    /// Evaluates an expression that must be fully known right now, in either pass.
    fn eval_now(&self, text: &str) -> Result<i64, String> {
        let e = expr::parse(text)?;
        e.eval(&self.view()).map_err(|err| match err {
            EvalError::Undefined(n) => {
                format!(
                    "\"{n}\" must be defined before this line (its value decides sizes or flow)"
                )
            }
            EvalError::Other(m) => m,
        })
    }

    /// Evaluates data: in the first pass an unknown symbol reads as 0.
    fn eval_data(&self, text: &str) -> Result<i64, String> {
        let e = expr::parse(text)?;
        let view = self.view();
        let result = if self.pass == 1 {
            e.eval(&Lenient { inner: &view })
        } else {
            e.eval(&view)
        };
        result.map_err(|err| err.to_string())
    }

    fn define(&mut self, src: &LineSrc, name: &str, value: i64, label: bool) {
        let key = self.key(name);
        match self.symbols.get(&key) {
            Some(old) if self.pass == 1 => {
                let _ = old;
                self.error(src, format!("\"{name}\" is already defined"));
            }
            Some(old) if old.value != value => {
                let msg = format!(
                    "phase error: \"{name}\" is ${:04X} in the first pass and ${:04X} in the second",
                    old.value, value
                );
                self.error(src, msg);
            }
            _ => {
                self.symbols.insert(key, Sym { value, label });
            }
        }
    }

    fn run(&mut self, lines: Vec<LineSrc>, depth: usize) {
        if depth > MAX_DEPTH {
            if let Some(first) = lines.first() {
                self.error(first, "includes / macros are nested too deeply");
            }
            return;
        }
        let mut i = 0;
        while i < lines.len() && !self.stop {
            if self.errors.len() >= MAX_ERRORS {
                return;
            }
            let src = lines[i].clone();
            i += 1;
            let stripped = strip_comment(&src.text).to_string();
            let stmt = self.parse(&stripped);
            let (labels, kind) = match stmt {
                Stmt::Empty => {
                    self.note(&src, None, &[]);
                    continue;
                }
                Stmt::Labels(l) => (l, None),
                Stmt::Org { value } => (Vec::new(), Some(Kind::Org(value.to_string()))),
                Stmt::Assign {
                    labels,
                    name,
                    value,
                } => (
                    labels,
                    Some(Kind::Assign(name.to_string(), value.to_string())),
                ),
                Stmt::Body {
                    labels,
                    word,
                    operand,
                } => (
                    labels,
                    Some(Kind::Body(word.to_string(), operand.to_string())),
                ),
            };

            // Conditionals and macro definitions are handled even in inactive regions.
            if let Some(Kind::Body(word, operand)) = &kind {
                match directive_name(word).as_deref() {
                    Some("if") | Some("ifdef") | Some("ifndef") => {
                        let dir = directive_name(word).unwrap();
                        self.begin_if(&src, &dir, operand);
                        continue;
                    }
                    Some("else") => {
                        self.do_else(&src);
                        continue;
                    }
                    Some("endif") => {
                        if self.conds.pop().is_none() {
                            self.error(&src, ".endif without .if");
                        }
                        continue;
                    }
                    Some("macro") => {
                        let (body, next) = self.collect_macro(&lines, i);
                        if self.active() {
                            self.define_macro(&src, operand, body);
                        }
                        i = next;
                        continue;
                    }
                    _ => {}
                }
            }
            if !self.active() {
                continue;
            }

            let start_pc = self.pc;
            let before_len = self.current_len();
            for label in &labels {
                self.define_label(&src, label);
            }
            let mut addr = self.pc.map(|p| p as u16);
            match kind {
                None => {}
                Some(Kind::Org(value)) => self.do_org(&src, &value),
                Some(Kind::Assign(name, value)) => self.do_assign(&src, &name, &value),
                Some(Kind::Body(word, operand)) => {
                    let found = self.macros.get(&word.to_ascii_lowercase()).map(m_clone);
                    if let Some(m) = found {
                        let expansion = self.expand_macro(&src, &word, m, &operand);
                        if let Some(lines) = expansion {
                            let saved = self.global_scope.clone();
                            self.macro_counter += 1;
                            self.macro_scope.push(self.macro_counter);
                            self.run(lines, depth + 1);
                            self.macro_scope.pop();
                            self.global_scope = saved;
                        }
                        continue;
                    }
                    match directive_name(&word) {
                        Some(dir) => {
                            if dir == "include" {
                                self.do_include(&src, &operand, depth);
                                continue;
                            }
                            addr = self.pc.map(|p| p as u16);
                            self.directive(&src, &dir, &operand);
                        }
                        None => {
                            addr = self.pc.map(|p| p as u16);
                            self.instruction(&src, &word, &operand);
                        }
                    }
                }
            }
            let emitted = self.emitted_since(start_pc, before_len);
            self.note(&src, addr, &emitted);
        }
    }

    fn current_len(&self) -> usize {
        self.segments.iter().map(|s| s.bytes.len()).sum()
    }

    /// The bytes emitted by the statement that just ran (second pass listing).
    fn emitted_since(&self, start_pc: Option<u32>, before_len: usize) -> Vec<u8> {
        let now = self.current_len();
        if now <= before_len {
            return Vec::new();
        }
        let (Some(start), Some(seg)) = (start_pc, self.segments.last()) else {
            return Vec::new();
        };
        let _ = start;
        let n = now - before_len;
        seg.bytes[seg.bytes.len().saturating_sub(n)..].to_vec()
    }

    fn note(&mut self, src: &LineSrc, addr: Option<u16>, bytes: &[u8]) {
        if self.pass == 2 {
            self.listing.push(ListEntry {
                file: src.file.to_string(),
                line: src.line,
                addr,
                bytes: bytes.to_vec(),
                source: src.text.clone(),
            });
        }
    }

    fn parse<'a>(&self, line: &'a str) -> Stmt<'a> {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            return Stmt::Empty;
        }
        let mut col0 = !trimmed.starts_with(char::is_whitespace);
        let mut rest = trimmed.trim_start();
        let mut labels = Vec::new();
        if let Some(after) = rest.strip_prefix('*') {
            let after = after.trim_start();
            if after.starts_with('=') && !after.starts_with("==") {
                return Stmt::Org {
                    value: after[1..].trim(),
                };
            }
        }
        loop {
            let (word, after) = take_symbol(rest);
            if word.is_empty() {
                break;
            }
            if let Some(next) = after.strip_prefix(':') {
                labels.push(word);
                rest = next.trim_start();
                col0 = false;
                continue;
            }
            let a2 = after.trim_start();
            if a2.starts_with('=') && !a2.starts_with("==") {
                return Stmt::Assign {
                    labels,
                    name: word,
                    value: a2[1..].trim(),
                };
            }
            let (w2, a3) = take_word(a2);
            if (w2.eq_ignore_ascii_case("equ") || w2.eq_ignore_ascii_case(".equ"))
                && !directive_is_word(word)
            {
                return Stmt::Assign {
                    labels,
                    name: word,
                    value: a3.trim(),
                };
            }
            if col0 && !word.starts_with('.') && !self.is_known_word(word) {
                labels.push(word);
                rest = a2;
                col0 = false;
                continue;
            }
            break;
        }
        if rest.is_empty() {
            return Stmt::Labels(labels);
        }
        let (word, operand) = take_word(rest);
        Stmt::Body {
            labels,
            word,
            operand: operand.trim(),
        }
    }

    fn is_known_word(&self, word: &str) -> bool {
        directive_name(word).is_some()
            || self.macros.contains_key(&word.to_ascii_lowercase())
            || self.cpu.is_some_and(|c| c.is_mnemonic(word))
    }

    fn define_label(&mut self, src: &LineSrc, name: &str) {
        let Some(pc) = self.pc else {
            self.error(
                src,
                format!("label \"{name}\" needs an address: start with .org"),
            );
            return;
        };
        if !name.starts_with('@') {
            self.global_scope = name.to_string();
        }
        self.define(src, name, pc as i64, true);
    }

    fn do_org(&mut self, src: &LineSrc, value: &str) {
        match self.eval_now(value) {
            Ok(v) if (0..=0xFFFF).contains(&v) => {
                // A new segment, unless this continues exactly where the last one ended.
                let continues = self.segments.last().is_some_and(|s| s.end() == v as u32)
                    && self.pc == Some(v as u32);
                self.pc = Some(v as u32);
                if !continues {
                    self.segments.push(Segment {
                        start: v as u16,
                        bytes: Vec::new(),
                    });
                }
            }
            Ok(v) => self.error(src, format!(".org {v} is outside 0..65535")),
            Err(e) => self.error(src, e),
        }
    }

    fn do_assign(&mut self, src: &LineSrc, name: &str, value: &str) {
        if self.pass == 1 {
            // A constant may refer to something defined later; then it is defined in the second pass.
            match self.eval_data(value) {
                Ok(v) => {
                    let unresolved = self.mentions_unknown(value);
                    if !unresolved {
                        self.define(src, name, v, false);
                    }
                }
                Err(e) => self.error(src, e),
            }
        } else {
            match self.eval_data(value) {
                Ok(v) => self.define(src, name, v, false),
                Err(e) => self.error(src, e),
            }
        }
    }

    fn mentions_unknown(&self, text: &str) -> bool {
        let Ok(e) = expr::parse(text) else {
            return false;
        };
        let mut names = Vec::new();
        e.symbols(&mut names);
        let view = self.view();
        names.iter().any(|n| view.symbol(n).is_none())
    }

    fn ensure_pc(&mut self, src: &LineSrc) -> Option<u32> {
        if self.pc.is_none() {
            self.error(src, "no address yet: start with .org");
        }
        self.pc
    }

    fn emit(&mut self, src: &LineSrc, bytes: &[u8]) {
        let Some(pc) = self.ensure_pc(src) else {
            return;
        };
        if pc as usize + bytes.len() > 0x10000 {
            self.error(src, "the program counter would pass $FFFF");
            return;
        }
        if self.segments.is_empty() {
            self.segments.push(Segment {
                start: pc as u16,
                bytes: Vec::new(),
            });
        }
        self.segments
            .last_mut()
            .expect("a segment exists")
            .bytes
            .extend_from_slice(bytes);
        self.pc = Some(pc + bytes.len() as u32);
    }

    fn instruction(&mut self, src: &LineSrc, word: &str, operand: &str) {
        let Some(cpu) = self.cpu else {
            self.error(src, "no CPU selected: use --cpu or a .cpu directive");
            return;
        };
        if !cpu.is_mnemonic(word) {
            let hint = if word.starts_with('.') {
                "unknown directive"
            } else {
                "unknown instruction"
            };
            self.error(src, format!("{hint} \"{word}\""));
            return;
        }
        let Some(pc) = self.ensure_pc(src) else {
            return;
        };
        if pc > 0xFFFF {
            self.error(src, "the program counter passed $FFFF");
            return;
        }
        self.instr_seq += 1;
        let id = self.instr_seq;
        let force_wide = self.wide.get(&id).copied().unwrap_or(false);
        let view = self.view();
        let site = Site {
            addr: pc as u16,
            scope: &view,
            strict: self.pass == 2,
            illegal: self.illegal,
            force_wide,
        };
        let result = cpu::encode(cpu, word, operand, &site);
        match result {
            Ok(enc) => {
                if self.pass == 1 {
                    self.wide.insert(id, enc.unresolved);
                }
                self.emit(src, &enc.bytes);
            }
            Err(e) => {
                self.error(src, e);
                // Keep the addresses sane for the rest of the pass.
                self.pc = Some(pc + 1);
            }
        }
    }

    fn directive(&mut self, src: &LineSrc, dir: &str, operand: &str) {
        match dir {
            "org" => self.do_org(src, operand),
            "end" => self.stop = true,
            "byte" | "db" | "defb" | "byt" | "text" | "ascii" | "str" | "asciiz" => {
                let mut out = Vec::new();
                for part in split_top(operand) {
                    if let Some(lit) = as_string_literal(part) {
                        match lit {
                            Ok(b) => out.extend(b),
                            Err(e) => {
                                self.error(src, e);
                                return;
                            }
                        }
                        continue;
                    }
                    match self.eval_data(part) {
                        Ok(v) if (-128..=255).contains(&v) => out.push((v & 0xFF) as u8),
                        Ok(v) => {
                            self.error(src, format!("{v} does not fit in a byte"));
                            return;
                        }
                        Err(e) => {
                            self.error(src, e);
                            return;
                        }
                    }
                }
                if out.is_empty() {
                    self.error(src, format!(".{dir} needs at least one value"));
                    return;
                }
                if dir == "asciiz" {
                    out.push(0);
                }
                self.emit(src, &out);
            }
            "word" | "dw" | "defw" => {
                let mut out = Vec::new();
                let parts = split_top(operand);
                if parts.is_empty() {
                    self.error(src, ".word needs at least one value");
                    return;
                }
                for part in parts {
                    match self.eval_data(part) {
                        Ok(v) if (-32768..=65535).contains(&v) => {
                            out.extend_from_slice(&((v & 0xFFFF) as u16).to_le_bytes())
                        }
                        Ok(v) => {
                            self.error(src, format!("{v} does not fit in 16 bits"));
                            return;
                        }
                        Err(e) => {
                            self.error(src, e);
                            return;
                        }
                    }
                }
                self.emit(src, &out);
            }
            "fill" | "ds" | "defs" | "res" => {
                let parts = split_top(operand);
                if parts.is_empty() || parts.len() > 2 {
                    self.error(src, format!(".{dir} takes a count and an optional value"));
                    return;
                }
                let count = match self.eval_now(parts[0]) {
                    Ok(c) if (0..=0x10000).contains(&c) => c as usize,
                    Ok(c) => {
                        self.error(src, format!("fill count {c} is outside 0..65536"));
                        return;
                    }
                    Err(e) => {
                        self.error(src, e);
                        return;
                    }
                };
                let value = match parts.get(1).map(|p| self.eval_data(p)) {
                    None => 0,
                    Some(Ok(v)) if (-128..=255).contains(&v) => (v & 0xFF) as u8,
                    Some(Ok(v)) => {
                        self.error(src, format!("fill value {v} does not fit in a byte"));
                        return;
                    }
                    Some(Err(e)) => {
                        self.error(src, e);
                        return;
                    }
                };
                self.emit(src, &vec![value; count]);
            }
            "align" => {
                let parts = split_top(operand);
                let Some(first) = parts.first() else {
                    self.error(src, ".align needs a boundary");
                    return;
                };
                let (boundary, fill) = match (
                    self.eval_now(first),
                    parts.get(1).map_or(Ok(0), |p| self.eval_data(p)),
                ) {
                    (Ok(b), Ok(f)) if b > 0 && (-128..=255).contains(&f) => (b as u32, f as u8),
                    (Ok(b), Ok(_)) if b <= 0 => {
                        self.error(src, ".align boundary must be positive");
                        return;
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        self.error(src, e);
                        return;
                    }
                    _ => {
                        self.error(src, ".align fill value does not fit in a byte");
                        return;
                    }
                };
                let Some(pc) = self.ensure_pc(src) else {
                    return;
                };
                let pad = (boundary - pc % boundary) % boundary;
                self.emit(src, &vec![fill; pad as usize]);
            }
            "equ" | "set" => {
                let parts = split_top(operand);
                if let [name, value] = parts.as_slice() {
                    self.do_assign(src, name, value);
                } else {
                    self.error(src, format!(".{dir} takes a name and a value"));
                }
            }
            "incbin" => self.do_incbin(src, operand),
            "cpu" => match Cpu::parse(operand) {
                Some(c) => {
                    if self.opts.cpu.is_some_and(|o| o != c) {
                        self.error(src, format!(".cpu {operand} conflicts with --cpu"));
                    } else {
                        self.cpu = Some(c);
                    }
                }
                None => self.error(src, format!("unknown CPU \"{operand}\" (6502 or z80)")),
            },
            "illegal" => match operand.trim().to_ascii_lowercase().as_str() {
                "" | "on" => self.illegal = true,
                "off" => self.illegal = false,
                other => self.error(src, format!(".illegal takes on or off, not \"{other}\"")),
            },
            "error" => {
                let msg = as_string_literal(operand)
                    .and_then(|r| r.ok())
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_else(|| operand.to_string());
                self.error(src, msg);
            }
            "assert" => {
                let parts = split_top(operand);
                let Some(cond) = parts.first() else {
                    self.error(src, ".assert needs a condition");
                    return;
                };
                match self.eval_data(cond) {
                    Ok(0) if self.pass == 2 => {
                        let msg = parts
                            .get(1)
                            .and_then(|p| as_string_literal(p))
                            .and_then(|r| r.ok())
                            .map(|b| String::from_utf8_lossy(&b).into_owned())
                            .unwrap_or_else(|| format!("assertion failed: {cond}"));
                        self.error(src, msg);
                    }
                    Ok(_) => {}
                    Err(e) => self.error(src, e),
                }
            }
            "macro" | "endm" | "endmacro" | "if" | "ifdef" | "ifndef" | "else" | "endif"
            | "include" => unreachable!("handled by the walk"),
            other => self.error(src, format!("unknown directive .{other}")),
        }
    }

    fn resolve_path(&self, src: &LineSrc, name: &str) -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        let from = Path::new(&*src.file).parent().unwrap_or(Path::new(""));
        candidates.push(from.join(name));
        for dir in &self.opts.include_dirs {
            candidates.push(dir.join(name));
        }
        candidates
    }

    fn read_file(&self, src: &LineSrc, name: &str) -> Result<(PathBuf, Vec<u8>), String> {
        let mut last = String::new();
        for path in self.resolve_path(src, name) {
            match self.files.read(&path) {
                Ok(bytes) => return Ok((path, bytes)),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn file_arg(&self, operand: &str) -> Result<String, String> {
        match as_string_literal(operand) {
            Some(Ok(b)) => Ok(String::from_utf8_lossy(&b).into_owned()),
            Some(Err(e)) => Err(e),
            None => Err("expected a quoted file name".to_string()),
        }
    }

    fn do_include(&mut self, src: &LineSrc, operand: &str, depth: usize) {
        let name = match self.file_arg(operand) {
            Ok(n) => n,
            Err(e) => {
                self.error(src, e);
                return;
            }
        };
        match self.read_file(src, &name) {
            Ok((path, bytes)) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let file: Rc<str> = Rc::from(path.to_string_lossy().as_ref());
                let lines = split_lines(&text, &file);
                self.run(lines, depth + 1);
            }
            Err(e) => self.error(src, e),
        }
    }

    fn do_incbin(&mut self, src: &LineSrc, operand: &str) {
        let parts = split_top(operand);
        let Some(first) = parts.first() else {
            self.error(src, ".incbin needs a file name");
            return;
        };
        let name = match self.file_arg(first) {
            Ok(n) => n,
            Err(e) => {
                self.error(src, e);
                return;
            }
        };
        let bytes = match self.read_file(src, &name) {
            Ok((_, b)) => b,
            Err(e) => {
                self.error(src, e);
                return;
            }
        };
        let skip = match parts.get(1).map(|p| self.eval_now(p)) {
            None => 0,
            Some(Ok(v)) if v >= 0 => v as usize,
            Some(Ok(v)) => {
                self.error(src, format!("skip {v} is negative"));
                return;
            }
            Some(Err(e)) => {
                self.error(src, e);
                return;
            }
        };
        if skip > bytes.len() {
            self.error(src, format!("skip {skip} is past the end of {name}"));
            return;
        }
        let len = match parts.get(2).map(|p| self.eval_now(p)) {
            None => bytes.len() - skip,
            Some(Ok(v)) if v >= 0 && skip + v as usize <= bytes.len() => v as usize,
            Some(Ok(v)) => {
                self.error(src, format!("length {v} does not fit in {name}"));
                return;
            }
            Some(Err(e)) => {
                self.error(src, e);
                return;
            }
        };
        self.emit(src, &bytes[skip..skip + len]);
    }

    // ---- conditionals ----

    fn begin_if(&mut self, src: &LineSrc, dir: &str, operand: &str) {
        let parent_active = self.active();
        let value = if !parent_active {
            false
        } else {
            match dir {
                "ifdef" | "ifndef" => {
                    let defined = self.view().symbol(operand.trim()).is_some();
                    defined == (dir == "ifdef")
                }
                _ => match self.eval_now(operand) {
                    Ok(v) => v != 0,
                    Err(e) => {
                        self.error(src, e);
                        false
                    }
                },
            }
        };
        self.conds.push(Cond {
            active: parent_active && value,
            taken: value,
            parent_active,
            seen_else: false,
        });
    }

    fn do_else(&mut self, src: &LineSrc) {
        match self.conds.last_mut() {
            None => self.error(src, ".else without .if"),
            Some(c) if c.seen_else => self.error(src, "second .else in one .if"),
            Some(c) => {
                c.seen_else = true;
                c.active = c.parent_active && !c.taken;
                c.taken = true;
            }
        }
    }

    // ---- macros ----

    /// Collects a macro body: the lines after the `.macro` line up to its matching `.endm`.
    fn collect_macro(&mut self, lines: &[LineSrc], mut i: usize) -> (Vec<LineSrc>, usize) {
        let mut depth = 1;
        let mut body = Vec::new();
        while i < lines.len() {
            let stripped = strip_comment(&lines[i].text);
            let word = stripped.split_whitespace().next().unwrap_or("");
            match directive_name(word).as_deref() {
                Some("macro") => depth += 1,
                Some("endm") | Some("endmacro") => {
                    depth -= 1;
                    if depth == 0 {
                        return (body, i + 1);
                    }
                }
                _ => {}
            }
            body.push(lines[i].clone());
            i += 1;
        }
        if let Some(last) = lines.last() {
            let l = last.clone();
            self.error(&l, "macro without .endm");
        }
        (body, i)
    }

    fn define_macro(&mut self, src: &LineSrc, header: &str, body: Vec<LineSrc>) {
        let (name, params) = take_word(header);
        if name.is_empty() {
            self.error(src, ".macro needs a name");
            return;
        }
        let key = name.to_ascii_lowercase();
        if self.pass == 1 && self.macros.contains_key(&key) {
            self.error(src, format!("macro \"{name}\" is already defined"));
            return;
        }
        let params: Vec<String> = split_top(params).into_iter().map(String::from).collect();
        self.macros.insert(key, Macro { params, body });
    }

    fn expand_macro(
        &mut self,
        src: &LineSrc,
        name: &str,
        m: MacroCopy,
        operand: &str,
    ) -> Option<Vec<LineSrc>> {
        let args: Vec<String> = split_top(operand).into_iter().map(String::from).collect();
        if args.len() > m.params.len() {
            self.error(
                src,
                format!(
                    "macro {name} takes {} argument(s), got {}",
                    m.params.len(),
                    args.len()
                ),
            );
            return None;
        }
        Some(
            m.body
                .iter()
                .map(|l| LineSrc {
                    text: substitute(&l.text, &m.params, &args),
                    file: Rc::clone(&l.file),
                    line: l.line,
                })
                .collect(),
        )
    }

    fn finish(self, name: &str) -> Result<Assembly, Vec<Diag>> {
        let Some(cpu) = self.cpu.or(self.opts.cpu) else {
            return Err(vec![Diag::new(
                name,
                0,
                "no CPU selected: use --cpu or a .cpu directive",
            )]);
        };
        let mut segments: Vec<Segment> = self
            .segments
            .into_iter()
            .filter(|s| !s.bytes.is_empty())
            .collect();
        segments.sort_by_key(|s| s.start);
        let mut merged: Vec<Segment> = Vec::new();
        for seg in segments {
            if let Some(last) = merged.last_mut() {
                if (seg.start as u32) < last.end() {
                    return Err(vec![Diag::new(
                        name,
                        0,
                        format!(
                            "segments overlap: ${:04X}-${:04X} and ${:04X}-${:04X}",
                            last.start,
                            last.end() - 1,
                            seg.start,
                            seg.end() - 1
                        ),
                    )]);
                }
                if seg.start as u32 == last.end() {
                    last.bytes.extend(seg.bytes);
                    continue;
                }
            }
            merged.push(seg);
        }
        let mut symbols: Vec<Symbol> = self
            .symbols
            .into_iter()
            .map(|(name, s)| Symbol {
                name: display_name(&name),
                value: s.value,
                label: s.label,
            })
            .collect();
        symbols.sort_by(|a, b| (a.value, &a.name).cmp(&(b.value, &b.name)));
        Ok(Assembly {
            cpu,
            segments: merged,
            symbols,
            listing: self.listing,
        })
    }
}

/// `scope|@name` -> `scope@name`; scopes of macro expansions keep their `#n`.
fn display_name(key: &str) -> String {
    match key.split_once('|') {
        Some((scope, local)) => format!("{scope}{local}"),
        None => key.to_string(),
    }
}

enum Kind {
    Org(String),
    Assign(String, String),
    Body(String, String),
}

struct MacroCopy {
    params: Vec<String>,
    body: Vec<LineSrc>,
}

fn m_clone(m: &Macro) -> MacroCopy {
    MacroCopy {
        params: m.params.clone(),
        body: m.body.clone(),
    }
}

fn directive_is_word(word: &str) -> bool {
    directive_name(word).is_some()
}

/// Replaces whole-word occurrences of each parameter by its argument.
fn substitute(line: &str, params: &[String], args: &[String]) -> String {
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if expr::is_symbol_start(c) || c == '\\' {
            let start = i;
            i += 1;
            while i < chars.len() && expr::is_symbol_char(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let bare = word.strip_prefix('\\').unwrap_or(&word);
            match params.iter().position(|p| p == bare) {
                Some(n) => out.push_str(args.get(n).map_or("", String::as_str)),
                None => out.push_str(&word),
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

struct Lenient<'a> {
    inner: &'a dyn Scope,
}

impl Scope for Lenient<'_> {
    fn symbol(&self, name: &str) -> Option<i64> {
        Some(self.inner.symbol(name).unwrap_or(0))
    }
    fn pc(&self) -> i64 {
        self.inner.pc()
    }
}
