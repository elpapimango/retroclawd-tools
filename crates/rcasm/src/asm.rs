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
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[path = "object.rs"]
mod object;
use object::{ObjState, SymKind};

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
    /// Write a relocatable object (use [`assemble_object`]).
    pub object: bool,
    /// The source syntax: rcasm's own, or vasm's `oldstyle` / `std` (rewritten line by line, see [`crate::vasm`]).
    pub syntax: crate::Syntax,
}

/// A run of bytes at an address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: u32,
    pub bytes: Vec<u8>,
    /// The ROM bank (`.bank n`, HuC6280): segments of different banks may use the same addresses. 0 elsewhere.
    pub bank: u16,
}

impl Segment {
    pub fn end(&self) -> u32 {
        self.start + self.bytes.len() as u32
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
    pub addr: Option<u32>,
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
    kind: SymKind,
    zp: bool,
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
    /// ca65 `.define`s: the name and what it stands for.
    defines: HashMap<String, Define>,
    /// `.charmap`: what each byte of a string literal becomes in `.byte`, `.text`, `.asciiz` and the like.
    charmap: [u8; 256],
    /// `.ifconst` results in the order the first pass decided them; the second pass replays them, so a label defined
    /// later stays "not constant" in both passes and the sizes agree.
    ifconst: Vec<bool>,
    ifconst_at: usize,
    /// The symbols expressions have used so far in this pass (keys), for `.ifref`; its results are replayed like
    /// `.ifconst`'s.
    refs: RefCell<HashSet<String>>,
    ifref: Vec<bool>,
    /// Symbols made with `.set`: they may be given a new value at any time.
    vars: HashSet<String>,
    /// Inside `.dsect`: the address to return to at `.dend`; addresses advance, nothing is written.
    dsect: Option<Option<u32>>,
    /// Where the next `.dsect` starts (where the last one ended).
    dsect_pc: u32,
    ifref_at: usize,
    /// How many macro expansions have renamed their `.local` names.
    local_counter: usize,
    /// Instruction number -> "the first pass found an unknown symbol in the operand".
    wide: HashMap<usize, bool>,
    // per pass
    pc: Option<u32>,
    /// The bank `.bank` selected (0 until then).
    bank: u16,
    /// `.proc` / `.scope` nesting.
    scopes: Vec<String>,
    /// How many unnamed labels (`:`) this pass has defined.
    anon: i64,
    /// 65816: register widths (`.a16`, `.i16`, `REP`/`SEP` in smart mode).
    m16: bool,
    x16: bool,
    smart: bool,
    segments: Vec<Segment>,
    global_scope: String,
    macro_scope: Vec<usize>,
    macro_counter: usize,
    instr_seq: usize,
    conds: Vec<Cond>,
    listing: Vec<ListEntry>,
    errors: Vec<Diag>,
    stop: bool,
    ob: ObjState,
}

struct View<'a> {
    ob: Option<&'a ObjState>,
    symbols: &'a HashMap<String, Sym>,
    scope_prefix: String,
    /// The names of the `.proc`/`.scope` blocks around the statement, outermost first.
    scopes: &'a [String],
    pc: i64,
    /// Where uses of symbols are recorded (`.ifref`).
    refs: &'a RefCell<HashSet<String>>,
}

impl Scope for View<'_> {
    fn symbol(&self, name: &str) -> Option<i64> {
        let key = self.key_of(name);
        let v = self.symbols.get(&key).map(|s| s.value);
        self.refs.borrow_mut().insert(key);
        v
    }
    fn pc(&self) -> i64 {
        self.pc
    }
    fn dependency(&self, e: &expr::Expr) -> Option<expr::Dep> {
        self.dependency_of(e)
    }
    fn canonical(&self, name: &str) -> String {
        self.key_of(name)
    }
}

/// Assembles `source` (named `name` in messages).
pub fn assemble(
    name: &str,
    source: &str,
    opts: &Options,
    files: &dyn Files,
) -> Result<Assembly, Vec<Diag>> {
    let asm = run_passes(name, source, opts, files, false)?;
    asm.finish(name)
}

/// Assembles `source` into a relocatable object (sections, imports, exports, relocations).
pub fn assemble_object(
    name: &str,
    source: &str,
    opts: &Options,
    files: &dyn Files,
) -> Result<rc_common::obj::Object, Vec<Diag>> {
    let asm = run_passes(name, source, opts, files, true)?;
    asm.finish_object(name)
}

fn run_passes<'f>(
    name: &str,
    source: &str,
    opts: &Options,
    files: &'f dyn Files,
    object: bool,
) -> Result<Asm<'f>, Vec<Diag>> {
    let mut asm = Asm {
        opts: opts.clone(),
        files,
        cpu: opts.cpu,
        illegal: opts.illegal,
        pass: 1,
        symbols: HashMap::new(),
        macros: HashMap::new(),
        defines: HashMap::new(),
        charmap: std::array::from_fn(|i| i as u8),
        ifconst: Vec::new(),
        ifconst_at: 0,
        refs: RefCell::new(HashSet::new()),
        ifref: Vec::new(),
        vars: HashSet::new(),
        dsect: None,
        dsect_pc: 0,
        ifref_at: 0,
        local_counter: 0,
        wide: HashMap::new(),
        pc: None,
        bank: 0,
        scopes: Vec::new(),
        anon: 0,
        m16: false,
        x16: false,
        smart: true,
        segments: Vec::new(),
        global_scope: String::new(),
        macro_scope: Vec::new(),
        macro_counter: 0,
        instr_seq: 0,
        conds: Vec::new(),
        listing: Vec::new(),
        errors: Vec::new(),
        stop: false,
        ob: ObjState::new(object || opts.object),
    };
    for (n, v) in &opts.defines {
        asm.symbols.insert(
            n.clone(),
            Sym {
                value: *v,
                label: false,
                kind: SymKind::Const,
                zp: false,
            },
        );
    }
    let root: Rc<str> = Rc::from(name);
    let lines = split_lines(source, &root, opts);
    for pass in 1..=2u8 {
        asm.begin_pass(pass);
        asm.run(lines.clone(), 0);
        if !asm.conds.is_empty() {
            let at = Diag::new(name, 0, "unterminated .if (missing .endif)");
            asm.errors.push(at);
        }
        if asm.dsect.is_some() {
            asm.errors.push(Diag::new(name, 0, ".dsect without .dend"));
        }
        if !asm.errors.is_empty() {
            return Err(asm.errors);
        }
        if pass == 1 {
            asm.settle_links();
        }
    }
    Ok(asm)
}

fn split_lines(source: &str, file: &Rc<str>, opts: &Options) -> Vec<LineSrc> {
    let mut rw = crate::vasm::Rewriter::new(opts.syntax, opts.cpu);
    source
        .lines()
        .enumerate()
        .flat_map(|(i, l)| {
            rw.line(l).into_iter().map(move |text| LineSrc {
                text,
                file: Rc::clone(file),
                line: i + 1,
            })
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
    "org",
    "byte",
    "db",
    "defb",
    "byt",
    "word",
    "dw",
    "defw",
    "text",
    "ascii",
    "asciiz",
    "str",
    "fill",
    "ds",
    "defs",
    "res",
    "align",
    "equ",
    "set",
    "include",
    "incbin",
    "macro",
    "endm",
    "endmacro",
    "if",
    "ifdef",
    "ifndef",
    "else",
    "endif",
    "cpu",
    "illegal",
    "error",
    "assert",
    "end",
    "bank",
    "segment",
    "import",
    "importzp",
    "export",
    "exportzp",
    "global",
    "a8",
    "a16",
    "i8",
    "i16",
    "smart",
    "proc",
    "endproc",
    "scope",
    "endscope",
    "addr",
    "dbyt",
    "setcpu",
    "p02",
    "pc02",
    "psc02",
    "p816",
    "feature",
    "list",
    "nolist",
    "macpack",
    "listbytes",
    "autoimport",
    "case",
    "repeat",
    "endrep",
    "define",
    "undef",
    "undefine",
    "struct",
    "endstruct",
    "enum",
    "endenum",
    "local",
    "ifblank",
    "ifnblank",
    "union",
    "endunion",
    "ifconst",
    "ifref",
    "ifnref",
    "dsect",
    "dend",
    "charmap",
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
        self.charmap = std::array::from_fn(|i| i as u8);
        self.ifconst_at = 0;
        self.ifref_at = 0;
        self.refs.borrow_mut().clear();
        self.dsect = None;
        self.dsect_pc = 0;
        self.pc = None;
        self.bank = 0;
        self.scopes.clear();
        self.anon = 0;
        self.m16 = false;
        self.x16 = false;
        self.smart = true;
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
        if self.ob.on {
            self.ob.begin_pass();
            self.pc = Some(0);
        }
    }

    fn error(&mut self, src: &LineSrc, message: impl Into<String>) {
        if self.errors.len() < MAX_ERRORS {
            self.errors
                .push(Diag::new(&*src.file, src.line, message.into()));
        }
    }

    /// The size of the CPU's address space (65536 until a CPU is chosen).
    fn limit(&self) -> u32 {
        1 << self.cpu.map_or(16, |c| c.addr_bits())
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
            ob: self.ob.on.then_some(&self.ob),
            symbols: &self.symbols,
            scope_prefix: self.scope_prefix(),
            scopes: &self.scopes,
            pc: self.pc.unwrap_or(0) as i64,
            refs: &self.refs,
        }
    }

    /// The key a definition of `name` has here: inside `.proc a` / `.scope b` it is `a::b::name`.
    fn key(&self, name: &str) -> String {
        if name.starts_with('@') {
            format!("{}{name}", self.scope_prefix())
        } else if self.scopes.is_empty() || name.contains("::") {
            name.strip_prefix("::").unwrap_or(name).to_string()
        } else {
            format!("{}::{name}", self.scopes.join("::"))
        }
    }

    /// Evaluates an expression that must be fully known right now, in either pass.
    fn eval_now(&self, text: &str) -> Result<i64, String> {
        let e = expr::parse(text)?;
        if self.ob.on && self.view().dependency_of(&e).is_some() {
            return Err(format!(
                "\"{text}\" depends on an address the linker chooses; it cannot decide sizes or flow here"
            ));
        }
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
        if self.vars.contains(&key) {
            self.symbols.insert(
                key,
                Sym {
                    value,
                    label,
                    kind: SymKind::Const,
                    zp: false,
                },
            );
            return;
        }
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
                let kind = match (label && self.ob.on).then_some(self.ob.cur) {
                    Some(i) => SymKind::Label(i),
                    None => SymKind::Const,
                };
                self.symbols.insert(
                    key,
                    Sym {
                        value,
                        label,
                        kind,
                        zp: false,
                    },
                );
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
            let stripped = self.anonymous_labels(strip_comment(&src.text));
            let stripped = if self.defines.is_empty() && !stripped.contains(".sizeof") {
                stripped
            } else {
                self.apply_defines(&stripped)
            };
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
                    Some("if") | Some("ifdef") | Some("ifndef") | Some("ifblank")
                    | Some("ifnblank") | Some("ifconst") | Some("ifref") | Some("ifnref") => {
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
                    Some(dir @ ("struct" | "union" | "enum")) => {
                        let close = match dir {
                            "struct" => "endstruct",
                            "union" => "endunion",
                            _ => "endenum",
                        };
                        let (body, next) = self.collect_block(&lines, i, dir, close);
                        if self.active() {
                            match dir {
                                "struct" => self.do_struct(&src, operand, body, false),
                                "union" => self.do_struct(&src, operand, body, true),
                                _ => self.do_enum(&src, operand, body),
                            }
                        }
                        i = next;
                        continue;
                    }
                    Some("repeat") => {
                        let (body, next) = self.collect_block(&lines, i, "repeat", "endrep");
                        if self.active() {
                            self.do_repeat(&src, operand, body, depth);
                        }
                        i = next;
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
            let mut addr = self.pc;
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
                            addr = self.pc;
                            self.directive(&src, &dir, &operand);
                        }
                        None => {
                            addr = self.pc;
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
        if self.ob.on {
            return self.ob.sections.iter().map(|s| s.bytes.len()).sum();
        }
        self.segments.iter().map(|s| s.bytes.len()).sum()
    }

    /// The bytes emitted by the statement that just ran (second pass listing).
    fn emitted_since(&self, start_pc: Option<u32>, before_len: usize) -> Vec<u8> {
        let now = self.current_len();
        if now <= before_len {
            return Vec::new();
        }
        if self.ob.on {
            let bytes = &self.ob.sections[self.ob.cur].bytes;
            return bytes[bytes.len().saturating_sub(now - before_len)..].to_vec();
        }
        let (Some(start), Some(seg)) = (start_pc, self.segments.last()) else {
            return Vec::new();
        };
        let _ = start;
        let n = now - before_len;
        seg.bytes[seg.bytes.len().saturating_sub(n)..].to_vec()
    }

    fn note(&mut self, src: &LineSrc, addr: Option<u32>, bytes: &[u8]) {
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
        if self.ob.on {
            self.error(src, ".org is not used in object mode: the linker places sections (a fixed address: .segment \"NAME\", at=ADDR)");
            return;
        }
        match self.eval_now(value) {
            Ok(v) if (0..self.limit() as i64).contains(&v) => {
                // A new segment, unless this continues exactly where the last one ended.
                let continues = self
                    .segments
                    .last()
                    .is_some_and(|s| s.end() == v as u32 && s.bank == self.bank)
                    && self.pc == Some(v as u32);
                self.pc = Some(v as u32);
                if !continues {
                    self.segments.push(Segment {
                        start: v as u32,
                        bytes: Vec::new(),
                        bank: self.bank,
                    });
                }
            }
            Ok(v) => {
                let max = self.limit() - 1;
                self.error(src, format!(".org {v} is outside 0..{max}"))
            }
            Err(e) => self.error(src, e),
        }
    }

    /// `.bank n`: the following `.org` starts a segment in ROM bank n (HuC6280 only).
    fn do_bank(&mut self, src: &LineSrc, value: &str) {
        if !matches!(self.cpu, Some(Cpu::Huc6280 | Cpu::Mos6502)) {
            self.error(
                src,
                ".bank is for the HuC6280 and the NES (.cpu huc6280, or 6502 for a NES cartridge)",
            );
            return;
        }
        match self.eval_now(value) {
            Ok(v) if (0..=0xFFF).contains(&v) => {
                self.bank = v as u16;
                self.pc = None; // an `.org` has to follow
            }
            Ok(v) => self.error(src, format!(".bank {v} is outside 0..4095")),
            Err(e) => self.error(src, e),
        }
    }

    fn do_assign(&mut self, src: &LineSrc, name: &str, value: &str) {
        if self.ob.on
            && let Ok(e) = expr::parse(value)
            && self.view().dependency_of(&e).is_some()
        {
            self.error(
                        src,
                        format!(
                            "\"{name}\" would depend on an address the linker chooses; define a label instead"
                        ),
                    );
            return;
        }
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
        if self.dsect.is_some() {
            // an offset section: the labels get addresses, nothing is written
            if pc as usize + bytes.len() > self.limit() as usize {
                self.error(src, "the .dsect passes the end of the address space");
            }
            self.pc = Some(pc + bytes.len() as u32);
            return;
        }
        if self.ob.on {
            let cur = self.ob.cur;
            let limit = self.limit() as usize;
            let sec = &mut self.ob.sections[cur];
            if sec.bss {
                let name = sec.name.clone();
                self.error(
                    src,
                    format!("segment \"{name}\" is uninitialised (bss): reserve space with .res"),
                );
                return;
            }
            if sec.size as usize + bytes.len() > limit {
                let name = sec.name.clone();
                self.error(src, format!("segment \"{name}\" is larger than 64 KB"));
                return;
            }
            sec.bytes.extend_from_slice(bytes);
            sec.size += bytes.len() as u32;
            self.pc = Some(sec.end());
            return;
        }
        if pc as usize + bytes.len() > self.limit() as usize {
            self.error(src, "the program counter would pass $FFFF");
            return;
        }
        if self.segments.is_empty() {
            self.segments.push(Segment {
                start: pc,
                bytes: Vec::new(),
                bank: self.bank,
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
        if pc >= self.limit() {
            self.error(src, "the program counter passed $FFFF");
            return;
        }
        if self.ob.on {
            self.object_instruction(src, cpu, word, operand);
            return;
        }
        self.instr_seq += 1;
        let id = self.instr_seq;
        let force_wide = self.wide.get(&id).copied().unwrap_or(false);
        let view = self.view();
        let site = Site {
            addr: pc,
            scope: &view,
            strict: self.pass == 2,
            illegal: self.illegal,
            force_wide,
            m16: self.m16,
            x16: self.x16,
            rec: None,
            probe: Default::default(),
        };
        let result = cpu::encode(cpu, word, operand, &site);
        match result {
            Ok(enc) => {
                if self.pass == 1 {
                    self.wide.insert(id, enc.unresolved);
                }
                self.emit(src, &enc.bytes);
                self.track_widths(word, operand);
            }
            Err(e) => {
                self.error(src, e);
                // Keep the addresses sane for the rest of the pass.
                self.pc = Some(pc + 1);
            }
        }
    }

    /// 65816 smart mode: `REP` and `SEP` with a constant change the immediate widths that follow.
    pub(super) fn track_widths(&mut self, word: &str, operand: &str) {
        if self.cpu != Some(Cpu::W65816) || !self.smart {
            return;
        }
        let up = word.to_ascii_uppercase();
        let up = up.split('.').next().unwrap_or("");
        if up != "REP" && up != "SEP" {
            return;
        }
        if let Ok(v) = self.eval_now(operand.trim().trim_start_matches('#')) {
            let set = up == "SEP";
            if v & 0x20 != 0 {
                self.m16 = !set;
            }
            if v & 0x10 != 0 {
                self.x16 = !set;
            }
        }
    }

    fn directive(&mut self, src: &LineSrc, dir: &str, operand: &str) {
        match dir {
            "org" if self.dsect.is_some() => match self.eval_now(operand) {
                Ok(v) if (0..self.limit() as i64).contains(&v) => self.pc = Some(v as u32),
                Ok(v) => self.error(src, format!(".org {v} is outside the address space")),
                Err(e) => self.error(src, e),
            },
            "org" => self.do_org(src, operand),
            "dsect" if self.ob.on => self.error(
                src,
                ".dsect is for images; in object mode use a bss segment",
            ),
            "dsect" if self.dsect.is_some() => self.error(src, ".dsect is already open"),
            "dsect" => {
                self.dsect = Some(self.pc);
                self.pc = Some(self.dsect_pc);
            }
            "dend" => match self.dsect.take() {
                Some(back) => {
                    self.dsect_pc = self.pc.unwrap_or(0);
                    self.pc = back;
                }
                None => self.error(src, ".dend without .dsect"),
            },
            "bank" => self.do_bank(src, operand),
            "segment" => self.do_segment(src, operand),
            "import" | "importzp" | "export" | "exportzp" | "global" => {
                self.do_linkage(src, dir, operand)
            }
            "end" => self.stop = true,
            "byte" | "db" | "defb" | "byt" | "text" | "ascii" | "str" | "asciiz" => {
                let mut out = Vec::new();
                for part in split_top(operand) {
                    if let Some(lit) = as_string_literal(part) {
                        match lit {
                            Ok(b) => out.extend(b.iter().map(|c| self.charmap[usize::from(*c)])),
                            Err(e) => {
                                self.error(src, e);
                                return;
                            }
                        }
                        continue;
                    }
                    let value = if self.ob.on {
                        self.object_data(src, part, 1, out.len())
                    } else {
                        self.eval_data(part)
                    };
                    match value {
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
            "dbyt" => {
                let mut out = Vec::new();
                for part in split_top(operand) {
                    match self.eval_data(part) {
                        Ok(v) if (-32768..=65535).contains(&v) => {
                            out.extend_from_slice(&((v & 0xFFFF) as u16).to_be_bytes())
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
            "setcpu" | "p02" | "pc02" | "psc02" | "p816" => {
                let name = match dir {
                    "setcpu" => as_string_literal(operand)
                        .and_then(|r| r.ok())
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .unwrap_or_else(|| operand.trim().to_string()),
                    "p816" => "65816".to_string(),
                    _ => "6502".to_string(),
                };
                match Cpu::parse(&name) {
                    Some(c) => {
                        self.cpu = Some(c);
                    }
                    None => self.error(src, format!("unknown CPU \"{name}\"")),
                }
            }
            // listing and ca65 housekeeping directives that change nothing here
            "feature" | "list" | "nolist" | "macpack" | "listbytes" | "autoimport" | "case" => {}
            "word" | "dw" | "defw" | "addr" => {
                let mut out = Vec::new();
                let parts = split_top(operand);
                if parts.is_empty() {
                    self.error(src, ".word needs at least one value");
                    return;
                }
                for part in parts {
                    let value = if self.ob.on {
                        self.object_data(src, part, 2, out.len())
                    } else {
                        self.eval_data(part)
                    };
                    match value {
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
                    Ok(c) if (0..=self.limit() as i64).contains(&c) => c as usize,
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
                if self.ob.on && self.ob.sections[self.ob.cur].bss {
                    let cur = self.ob.cur;
                    let limit = self.limit() as usize;
                    let sec = &mut self.ob.sections[cur];
                    if sec.size as usize + count > limit {
                        self.error(src, "the segment would be larger than 64 KB");
                        return;
                    }
                    sec.size += count as u32;
                    self.pc = Some(sec.end());
                    return;
                }
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
                    if dir == "set" {
                        let key = self.key(name);
                        self.vars.insert(key);
                    }
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
            "a8" | "a16" | "i8" | "i16" => {
                if self.cpu != Some(Cpu::W65816) {
                    self.error(src, format!(".{dir} is for the 65816 (.cpu 65816)"));
                    return;
                }
                match dir {
                    "a8" => self.m16 = false,
                    "a16" => self.m16 = true,
                    "i8" => self.x16 = false,
                    _ => self.x16 = true,
                }
            }
            "proc" | "scope" => {
                let name = operand.trim();
                if name.is_empty()
                    || !name.chars().all(expr::is_symbol_char)
                    || name.starts_with('@')
                {
                    self.error(src, format!(".{dir} needs a name"));
                    return;
                }
                if dir == "proc" {
                    self.define_label(src, name);
                }
                self.scopes.push(name.to_string());
            }
            "endproc" | "endscope" => {
                if self.scopes.pop().is_none() {
                    self.error(src, format!(".{dir} without .proc or .scope"));
                }
            }
            "smart" => match operand.trim().to_ascii_lowercase().as_str() {
                "" | "on" => self.smart = true,
                "off" => self.smart = false,
                other => self.error(src, format!(".smart takes on or off, not \"{other}\"")),
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
            "define" => self.do_define(src, operand),
            "undef" | "undefine" => {
                if self.defines.remove(operand.trim()).is_none() && self.pass == 1 {
                    self.error(
                        src,
                        format!(".{dir}: \"{}\" is not a .define", operand.trim()),
                    );
                }
            }
            "local" => self.error(src, ".local is only valid inside a macro"),
            "charmap" => self.do_charmap(src, operand),
            "endstruct" | "endunion" | "endenum" => {
                self.error(src, format!(".{dir} without its opening directive"))
            }
            "macro" | "endm" | "endmacro" | "if" | "ifdef" | "ifndef" | "ifblank" | "ifnblank"
            | "ifconst" | "ifref" | "ifnref" | "else" | "endif" | "include" | "struct"
            | "union" | "enum" => {
                unreachable!("handled by the walk")
            }
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
                let lines = split_lines(&text, &file, &self.opts);
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
                    // a test, not a use: `.ifref` does not see it
                    let defined = self
                        .symbols
                        .contains_key(&self.view().canonical(operand.trim()))
                        || self.defines.contains_key(operand.trim());
                    defined == (dir == "ifdef")
                }
                "ifblank" | "ifnblank" => operand.trim().is_empty() == (dir == "ifblank"),
                "ifref" | "ifnref" => {
                    let at = self.ifref_at;
                    self.ifref_at += 1;
                    let used = match self.ifref.get(at) {
                        Some(v) if self.pass == 2 => *v,
                        _ => {
                            let key = self.view().canonical(operand.trim());
                            let v = self.refs.borrow().contains(&key);
                            self.ifref.truncate(at);
                            self.ifref.push(v);
                            v
                        }
                    };
                    used == (dir == "ifref")
                }
                "ifconst" => {
                    let at = self.ifconst_at;
                    self.ifconst_at += 1;
                    match self.ifconst.get(at) {
                        Some(v) if self.pass == 2 => *v,
                        _ => {
                            let v = self.eval_now(operand).is_ok();
                            self.ifconst.truncate(at);
                            self.ifconst.push(v);
                            v
                        }
                    }
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
    /// Like `collect_macro` for any `.open` ... `.close` block.
    fn collect_block(
        &mut self,
        lines: &[LineSrc],
        mut i: usize,
        open: &str,
        close: &str,
    ) -> (Vec<LineSrc>, usize) {
        let mut depth = 1;
        let mut body = Vec::new();
        while i < lines.len() {
            let stripped = strip_comment(&lines[i].text);
            let word = stripped.split_whitespace().next().unwrap_or("");
            match directive_name(word).as_deref() {
                Some(w) if w == open => depth += 1,
                Some(w) if w == close => {
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
            self.error(&l, format!(".{open} without .{close}"));
        }
        (body, i)
    }

    /// `.repeat COUNT [, VAR]` ... `.endrep`: the body COUNT times, VAR counting from 0.
    fn do_repeat(&mut self, src: &LineSrc, operand: &str, body: Vec<LineSrc>, depth: usize) {
        let parts = split_top(operand);
        let Some(count) = parts.first() else {
            self.error(src, ".repeat needs a count");
            return;
        };
        let n = match self.eval_now(count) {
            Ok(n) if (0..=65536).contains(&n) => n,
            Ok(n) => {
                self.error(src, format!(".repeat {n} is outside 0..65536"));
                return;
            }
            Err(e) => {
                self.error(src, e);
                return;
            }
        };
        let var: Vec<String> = parts
            .get(1)
            .map(|v| vec![v.trim().to_string()])
            .unwrap_or_default();
        for k in 0..n {
            let lines: Vec<LineSrc> = body
                .iter()
                .map(|l| LineSrc {
                    text: if var.is_empty() {
                        l.text.clone()
                    } else {
                        substitute(&l.text, &var, &[k.to_string()])
                    },
                    file: Rc::clone(&l.file),
                    line: l.line,
                })
                .collect();
            self.run(lines, depth + 1);
        }
    }

    /// ca65's unnamed labels: a line that starts with `:` defines one; `:+`, `:++` the next ones, `:-`, `:--` the
    /// previous ones, written as names that cannot clash.
    fn anonymous_labels(&mut self, line: &str) -> String {
        let t = line.trim_start();
        let mut text = line.to_string();
        if t.starts_with(':') && !t.starts_with("::") && !t[1..].starts_with(['+', '-']) {
            self.anon += 1;
            let indent = line.len() - t.len();
            text = format!("{}__anon_{}:{}", &line[..indent], self.anon, &t[1..]);
        }
        if !text.contains(":+") && !text.contains(":-") {
            return text;
        }
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            let prev_ok = i == 0
                || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_' || chars[i - 1] == ':');
            if chars[i] == ':'
                && prev_ok
                && i + 1 < chars.len()
                && (chars[i + 1] == '+' || chars[i + 1] == '-')
            {
                let sign = chars[i + 1];
                let mut n = 0;
                while i + 1 + n < chars.len() && chars[i + 1 + n] == sign {
                    n += 1;
                }
                let target = if sign == '+' {
                    self.anon + n as i64
                } else {
                    self.anon - n as i64 + 1
                };
                out.push_str(&format!("__anon_{target}"));
                i += 1 + n;
                continue;
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

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
        // `.local a, b`: names that belong to this expansion alone
        let mut locals: Vec<String> = Vec::new();
        let mut body: Vec<&LineSrc> = Vec::new();
        for l in &m.body {
            let text = strip_comment(&l.text);
            let (word, rest) = take_word(text.trim());
            if word.eq_ignore_ascii_case(".local") {
                locals.extend(
                    split_top(rest)
                        .into_iter()
                        .map(|n| n.trim().to_string())
                        .filter(|n| !n.is_empty()),
                );
            } else {
                body.push(l);
            }
        }
        self.local_counter += 1;
        let mut names = m.params.clone();
        let mut values = args;
        values.resize(m.params.len(), String::new());
        for l in &locals {
            names.push(l.clone());
            values.push(format!("{l}__L{}", self.local_counter));
        }
        Some(
            body.iter()
                .map(|l| LineSrc {
                    text: substitute(&l.text, &names, &values),
                    file: Rc::clone(&l.file),
                    line: l.line,
                })
                .collect(),
        )
    }

    // ---- ca65: .define, .struct, .enum ----

    /// `.define NAME text` or `.define NAME(a, b) text`.
    fn do_define(&mut self, src: &LineSrc, operand: &str) {
        let text = operand.trim();
        let end = text
            .find(|c: char| !expr::is_symbol_char(c))
            .unwrap_or(text.len());
        let name = &text[..end];
        if name.is_empty() || name.starts_with('.') && name.len() == 1 {
            self.error(src, ".define needs a name");
            return;
        }
        let rest = &text[end..];
        let (params, body) = match rest.strip_prefix('(') {
            Some(after) => match after.find(')') {
                Some(close) => (
                    Some(
                        split_top(&after[..close])
                            .into_iter()
                            .map(|p| p.trim().to_string())
                            .collect(),
                    ),
                    after[close + 1..].trim(),
                ),
                None => {
                    self.error(src, ".define: the parameter list has no )");
                    return;
                }
            },
            None => (None, rest.trim()),
        };
        self.defines.insert(
            name.to_string(),
            Define {
                params,
                body: body.to_string(),
            },
        );
    }

    /// Replaces `.define`d names (object-like and function-like) and `.sizeof(Struct)` in a line, outside strings.
    fn apply_defines(&self, line: &str) -> String {
        let first = line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(
            first.as_str(),
            ".define" | ".undef" | ".undefine" | ".ifdef" | ".ifndef"
        ) {
            return line.to_string();
        }
        let mut text = line.to_string();
        for _ in 0..8 {
            let next = self.apply_defines_once(&text);
            if next == text {
                break;
            }
            text = next;
        }
        text
    }

    fn apply_defines_once(&self, line: &str) -> String {
        let chars: Vec<char> = line.chars().collect();
        let mut out = String::with_capacity(line.len());
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '"' || c == '\'' {
                out.push(c);
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        out.push(chars[i]);
                    } else if chars[i] == c {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            if !expr::is_symbol_start(c) {
                out.push(c);
                i += 1;
                continue;
            }
            let start = i;
            i += 1;
            while i < chars.len() && expr::is_symbol_char(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let qualified = start > 0 && chars[start - 1] == ':';
            if word.eq_ignore_ascii_case(".sizeof")
                && chars.get(i) == Some(&'(')
                && let Some(close) = chars[i..].iter().position(|c| *c == ')')
            {
                let inner: String = chars[i + 1..i + close].iter().collect();
                out.push_str(&format!("{}::__size", inner.trim()));
                i += close + 1;
                continue;
            }
            match self.defines.get(&word) {
                Some(d) if !qualified => match &d.params {
                    None => out.push_str(&d.body),
                    Some(params) => {
                        if chars.get(i) != Some(&'(') {
                            out.push_str(&word);
                            continue;
                        }
                        // the arguments: up to the matching )
                        let mut depth = 0;
                        let mut end = None;
                        for (k, ch) in chars.iter().enumerate().skip(i) {
                            match ch {
                                '(' => depth += 1,
                                ')' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        end = Some(k);
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        let Some(end) = end else {
                            out.push_str(&word);
                            continue;
                        };
                        let inner: String = chars[i + 1..end].iter().collect();
                        let args: Vec<String> = split_top(&inner)
                            .into_iter()
                            .map(|a| a.trim().to_string())
                            .collect();
                        out.push_str(&substitute(&d.body, params, &args));
                        i = end + 1;
                    }
                },
                _ => out.push_str(&word),
            }
        }
        out
    }

    /// `.struct Name` ... `.endstruct`: `Name::member` is the member's offset, `.sizeof(Name)` the size. A `.union`
    /// (`union`) puts every member at 0 and is as large as its largest member.
    fn do_struct(&mut self, src: &LineSrc, operand: &str, body: Vec<LineSrc>, union: bool) {
        let what = if union { ".union" } else { ".struct" };
        let name = operand.trim();
        if name.is_empty() || !name.chars().all(|c| expr::is_symbol_char(c) || c == ':') {
            self.error(src, format!("{what} needs a name"));
            return;
        }
        let mut off: i64 = 0;
        let mut largest: i64 = 0;
        for l in &body {
            let text = strip_comment(&l.text).trim().to_string();
            if text.is_empty() {
                continue;
            }
            let (first, rest) = take_word(&text);
            let (member, decl) = if first.starts_with('.') {
                (None, text.as_str())
            } else {
                (Some(first.trim_end_matches(':')), rest)
            };
            let (d, operand) = take_word(decl);
            let size = match d.to_ascii_lowercase().as_str() {
                ".byte" => Ok(1),
                ".word" | ".addr" => Ok(2),
                ".faraddr" => Ok(3),
                ".dword" => Ok(4),
                ".res" => self.eval_now(operand),
                ".tag" => self
                    .view()
                    .symbol(&format!("{}::__size", operand.trim()))
                    .ok_or_else(|| format!(".tag: \"{}\" is not a struct", operand.trim())),
                other => Err(format!(
                    "{other} is not allowed in a {what} (.byte .word .addr .dword .res .tag)"
                )),
            };
            let size = match size {
                Ok(n) if n >= 0 => n,
                Ok(n) => {
                    self.error(l, format!("a member cannot have the size {n}"));
                    continue;
                }
                Err(e) => {
                    self.error(l, e);
                    continue;
                }
            };
            if let Some(m) = member {
                self.define(l, &format!("{name}::{m}"), off, false);
            }
            if union {
                largest = largest.max(size);
            } else {
                off += size;
            }
        }
        let size = if union { largest } else { off };
        self.define(src, &format!("{name}::__size"), size, false);
    }

    /// `.charmap INDEX, VALUE`: the byte `INDEX` of a string literal becomes `VALUE` from here on.
    fn do_charmap(&mut self, src: &LineSrc, operand: &str) {
        let parts = split_top(operand);
        if parts.len() != 2 {
            self.error(src, ".charmap needs an index and a value");
            return;
        }
        let mut get = |p: &str| match self.eval_now(p) {
            Ok(v) if (0..=255).contains(&v) => Some(v as u8),
            Ok(v) => {
                self.error(src, format!(".charmap: {v} is not a byte (0-255)"));
                None
            }
            Err(e) => {
                self.error(src, e);
                None
            }
        };
        if let (Some(index), Some(value)) = (get(parts[0]), get(parts[1])) {
            self.charmap[usize::from(index)] = value;
        }
    }

    /// `.enum [Name]` ... `.endenum`: names counting up from 0 (or from the last `= value`), as `Name::A` for a named
    /// enum and `A` for an unnamed one.
    fn do_enum(&mut self, src: &LineSrc, operand: &str, body: Vec<LineSrc>) {
        let name = operand.trim();
        let mut next: i64 = 0;
        for l in &body {
            let text = strip_comment(&l.text).trim().to_string();
            for item in split_top(&text) {
                let item = item.trim();
                if item.is_empty() {
                    continue;
                }
                let (n, value) = match item.split_once('=') {
                    Some((n, v)) => (n.trim(), Some(v.trim())),
                    None => (item, None),
                };
                if n.is_empty() || !n.chars().all(expr::is_symbol_char) {
                    self.error(l, format!("\"{item}\" is not an enum member"));
                    continue;
                }
                if let Some(v) = value {
                    match self.eval_now(v) {
                        Ok(v) => next = v,
                        Err(e) => {
                            self.error(l, e);
                            continue;
                        }
                    }
                }
                let full = if name.is_empty() {
                    n.to_string()
                } else {
                    format!("{name}::{n}")
                };
                self.define(l, &full, next, false);
                next += 1;
            }
        }
        let _ = src;
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
        segments.sort_by_key(|s| (s.bank, s.start));
        let mut merged: Vec<Segment> = Vec::new();
        for seg in segments {
            if let Some(last) = merged.last_mut() {
                if seg.bank == last.bank && seg.start < last.end() {
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
                if seg.bank == last.bank && seg.start == last.end() {
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

/// A ca65 `.define`: object-like (`params` is `None`) or function-like.
struct Define {
    params: Option<Vec<String>>,
    body: String,
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
