//! Object mode: the assembler writes a relocatable `.rco` object instead of placed segments.
//!
//! The walk over the source is the same as in flat mode. What changes is where bytes go and what a symbol is:
//! code goes into named sections (`.segment`), a label is an offset into its section, an `.import`ed name is
//! unknown until link time. An operand that depends on a label of a relocatable section, on an import or on `$`
//! is *relocatable*: the encoder is given a placeholder and the object records "these bytes hold this
//! expression". The encoders are not modified: the bytes of the field are found by running the encoder again
//! with the operand's value moved (`cpu::Probe`) and, to tell a relative branch from an absolute address, with the
//! instruction's address moved.

use super::*;
use crate::cpu::{Operand, Probe};
use crate::expr::{Dep, Expr, Group};
use rc_common::obj;
use std::cell::RefCell;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SymKind {
    Const,
    /// An address in section n.
    Label(usize),
    Import,
}

pub(super) struct OSection {
    pub name: String,
    pub bss: bool,
    pub zp: bool,
    pub align: u32,
    pub addr: Option<u32>,
    pub bytes: Vec<u8>,
    pub size: u32,
}

impl OSection {
    fn new(name: &str) -> Self {
        OSection {
            name: name.to_string(),
            bss: false,
            zp: false,
            align: 1,
            addr: None,
            bytes: Vec::new(),
            size: 0,
        }
    }

    pub fn base(&self) -> u32 {
        self.addr.unwrap_or(0)
    }

    pub fn end(&self) -> u32 {
        self.base() + self.size
    }
}

pub(super) struct PendingReloc {
    section: usize,
    offset: u32,
    width: u8,
    kind: obj::Kind,
    bias: i64,
    back: u32,
    expr: Expr,
    file: String,
    line: usize,
}

#[derive(Default)]
pub(super) struct ObjState {
    pub on: bool,
    pub sections: Vec<OSection>,
    pub cur: usize,
    relocs: Vec<PendingReloc>,
    exports: Vec<(String, bool)>,
    globals: Vec<String>,
}

impl ObjState {
    pub fn new(on: bool) -> Self {
        ObjState {
            on,
            ..ObjState::default()
        }
    }

    pub fn begin_pass(&mut self) {
        if self.on {
            self.sections = vec![OSection::new("CODE")];
            self.cur = 0;
            self.relocs.clear();
        }
    }
}

impl View<'_> {
    pub(super) fn key_of(&self, name: &str) -> String {
        if name.starts_with('@') {
            format!("{}{name}", self.scope_prefix)
        } else {
            name.to_string()
        }
    }

    fn group_of(&self, name: &str) -> Option<Group> {
        let ob = self.ob?;
        let key = self.key_of(name);
        match self.symbols.get(&key)?.kind {
            SymKind::Const => None,
            // A section that does not exist yet in this pass (a forward reference) is a placed one.
            SymKind::Label(i) => ob
                .sections
                .get(i)
                .is_none_or(|s| s.addr.is_none())
                .then_some(Group::Section(i)),
            SymKind::Import => Some(Group::Import(key)),
        }
    }

    fn group_zp(&self, g: &Group) -> bool {
        let Some(ob) = self.ob else { return false };
        match g {
            Group::Section(i) => ob.sections.get(*i).is_some_and(|s| s.zp),
            Group::Import(key) => self.symbols.get(key).is_some_and(|s| s.zp),
        }
    }

    fn label_zp(&self, name: &str) -> bool {
        self.symbols
            .get(&self.key_of(name))
            .is_some_and(|s| s.zp || matches!(s.kind, SymKind::Label(_)) && self.group_zp_of(name))
    }

    fn group_zp_of(&self, name: &str) -> bool {
        self.group_of(name).is_some_and(|g| self.group_zp(&g))
    }
}

/// A scope with one group's values moved by `delta`; unknown names read as 0.
struct Shifted<'a, 'b> {
    view: &'a View<'b>,
    group: &'a Group,
    delta: i64,
}

impl Scope for Shifted<'_, '_> {
    fn symbol(&self, name: &str) -> Option<i64> {
        let v = self.view.symbol(name).unwrap_or(0);
        Some(if self.view.group_of(name).as_ref() == Some(self.group) {
            v + self.delta
        } else {
            v
        })
    }
    fn pc(&self) -> i64 {
        let ob = self.view.ob;
        match (ob, self.group) {
            (Some(o), Group::Section(i)) if *i == o.cur => self.view.pc + self.delta,
            _ => self.view.pc,
        }
    }
}

struct Zero<'a, 'b>(&'a View<'b>);

impl Scope for Zero<'_, '_> {
    fn symbol(&self, name: &str) -> Option<i64> {
        Some(self.0.symbol(name).unwrap_or(0))
    }
    fn pc(&self) -> i64 {
        self.0.pc
    }
}

impl View<'_> {
    pub(super) fn dependency_of(&self, e: &Expr) -> Option<Dep> {
        let ob = self.ob?;
        let mut names = Vec::new();
        e.symbols(&mut names);
        let mut groups: Vec<Group> = Vec::new();
        for n in &names {
            if let Some(g) = self.group_of(n)
                && !groups.contains(&g)
            {
                groups.push(g);
            }
        }
        if e.has_pc() && ob.sections[ob.cur].addr.is_none() {
            let g = Group::Section(ob.cur);
            if !groups.contains(&g) {
                groups.push(g);
            }
        }
        if groups.is_empty() {
            return None;
        }
        let base = e.eval(&Zero(self)).ok()?;
        let mut dependent = Vec::new();
        for g in groups {
            let moves = [1i64, 0x100].iter().any(|&delta| {
                let shifted = Shifted {
                    view: self,
                    group: &g,
                    delta,
                };
                e.eval(&shifted).is_ok_and(|v| v != base)
            });
            if moves {
                dependent.push(g);
            }
        }
        if dependent.is_empty() {
            return None;
        }
        // Zero page when everything the value moves with is declared zero page.
        let zeropage = dependent.iter().all(|g| match g {
            Group::Section(_) => self.group_zp(g),
            Group::Import(_) => self.group_zp(g),
        }) && names
            .iter()
            .filter(|n| self.group_of(n).is_some())
            .all(|n| self.label_zp(n) || self.group_zp_of(n));
        Some(Dep {
            groups: dependent,
            zeropage,
        })
    }
}

fn parse_section_options(parts: &[&str]) -> Result<(bool, bool, u32, Option<u32>), String> {
    let (mut bss, mut zp, mut align, mut addr) = (false, false, 1u32, None);
    for p in parts {
        let lower = p.to_ascii_lowercase();
        if lower == "bss" {
            bss = true;
        } else if lower == "zeropage" || lower == "zp" {
            zp = true;
        } else if let Some(v) = lower.strip_prefix("align") {
            let v = v.trim().trim_start_matches('=').trim();
            let n = rc_common::parse_number(v).map_err(|e| format!("align: {e}"))?;
            if !(1..=1 << 16).contains(&n) || (n as u64).count_ones() != 1 {
                return Err(format!("align {n} is not a power of two up to 65536"));
            }
            align = n as u32;
        } else if let Some(v) = lower.strip_prefix("at") {
            let v = v.trim().trim_start_matches('=').trim();
            let n = rc_common::parse_number(v).map_err(|e| format!("at: {e}"))?;
            if !(0..=0xFFFF).contains(&n) {
                return Err(format!("at {n:#X} is outside 0..65535"));
            }
            addr = Some(n as u32);
        } else {
            return Err(format!(
                "unknown segment option \"{p}\" (bss, zeropage, align=N, at=ADDR)"
            ));
        }
    }
    Ok((bss, zp, align, addr))
}

impl Asm<'_> {
    pub(super) fn do_segment(&mut self, src: &LineSrc, operand: &str) {
        if !self.ob.on {
            self.error(
                src,
                ".segment needs object mode (rcasm -c, or assemble_object)",
            );
            return;
        }
        let parts = split_top(operand);
        let Some(first) = parts.first() else {
            self.error(src, ".segment needs a name");
            return;
        };
        let name = match self.file_arg(first) {
            Ok(n) if !n.is_empty() => n,
            Ok(_) => {
                self.error(src, "a segment name cannot be empty");
                return;
            }
            Err(e) => {
                self.error(src, e);
                return;
            }
        };
        let (bss, zp, align, addr) = match parse_section_options(&parts[1..]) {
            Ok(o) => o,
            Err(e) => {
                self.error(src, e);
                return;
            }
        };
        match self.ob.sections.iter().position(|s| s.name == name) {
            Some(i) => {
                let s = &self.ob.sections[i];
                let same = (parts.len() == 1)
                    || (s.bss == bss && s.zp == zp && s.align == align && s.addr == addr);
                if !same {
                    self.error(
                        src,
                        format!("segment \"{name}\" was started with other options"),
                    );
                    return;
                }
                self.ob.cur = i;
            }
            None => {
                let mut s = OSection::new(&name);
                s.bss = bss;
                s.zp = zp;
                s.align = align;
                s.addr = addr;
                self.ob.sections.push(s);
                self.ob.cur = self.ob.sections.len() - 1;
                if self.ob.sections.len() > obj::MAX_SECTIONS {
                    self.error(src, "too many segments");
                    return;
                }
            }
        }
        self.pc = Some(self.ob.sections[self.ob.cur].end());
    }

    /// `.import a, b`, `.importzp a`, `.export a`, `.exportzp a`, `.global a`.
    pub(super) fn do_linkage(&mut self, src: &LineSrc, dir: &str, operand: &str) {
        if !self.ob.on {
            self.error(
                src,
                format!(".{dir} needs object mode (rcasm -c, or assemble_object)"),
            );
            return;
        }
        let names = split_top(operand);
        if names.is_empty() {
            self.error(src, format!(".{dir} needs at least one name"));
            return;
        }
        for name in names {
            let valid = name.chars().next().is_some_and(expr::is_symbol_start)
                && name.chars().all(expr::is_symbol_char)
                && !name.starts_with('@');
            if !valid {
                self.error(
                    src,
                    format!("\"{name}\" is not a name that can be shared between objects"),
                );
                continue;
            }
            match dir {
                "import" | "importzp" => {
                    let zp = dir == "importzp";
                    match self.symbols.get(name) {
                        None => {
                            self.symbols.insert(
                                name.to_string(),
                                Sym {
                                    value: 0,
                                    label: false,
                                    kind: SymKind::Import,
                                    zp,
                                },
                            );
                        }
                        Some(s) if s.kind == SymKind::Import => {}
                        Some(_) => self.error(src, format!("\"{name}\" is already defined here")),
                    }
                }
                "export" | "exportzp" => {
                    let zp = dir == "exportzp";
                    if !self.ob.exports.iter().any(|(n, _)| n == name) {
                        self.ob.exports.push((name.to_string(), zp));
                    }
                }
                _ => {
                    if !self.ob.globals.iter().any(|n| n == name) {
                        self.ob.globals.push(name.to_string());
                    }
                }
            }
        }
    }

    /// Between the passes: names declared `.global` that nothing defined are imports; `.exportzp` marks zero page.
    pub(super) fn settle_links(&mut self) {
        if !self.ob.on {
            return;
        }
        let globals = self.ob.globals.clone();
        for g in globals {
            self.symbols.entry(g).or_insert(Sym {
                value: 0,
                label: false,
                kind: SymKind::Import,
                zp: false,
            });
        }
        let exports = self.ob.exports.clone();
        for (name, zp) in exports {
            if zp && let Some(s) = self.symbols.get_mut(&name) {
                s.zp = true;
            }
        }
    }

    pub(super) fn file_name(&self, src: &LineSrc) -> String {
        src.file.to_string()
    }

    /// Emits the bytes of an object-mode instruction and records its relocations.
    pub(super) fn object_instruction(
        &mut self,
        src: &LineSrc,
        cpu: Cpu,
        word: &str,
        operand: &str,
    ) {
        let pc = self.pc.unwrap_or(0);
        if pc > 0xFFFF {
            self.error(src, "the section is larger than 64 KB");
            return;
        }
        self.instr_seq += 1;
        let id = self.instr_seq;
        let force_wide = self.wide.get(&id).copied().unwrap_or(false);
        let strict = self.pass == 2;
        let illegal = self.illegal;
        let (m16, x16) = (self.m16, self.x16);
        let rec: RefCell<Vec<Operand>> = RefCell::new(Vec::new());
        let view = self.view();
        let wide = std::cell::Cell::new(force_wide);
        let attempt = |addr: u32, probe: Probe| {
            rec.borrow_mut().clear();
            let site = Site {
                addr,
                scope: &view,
                strict,
                illegal,
                force_wide: wide.get(),
                m16: m16,
                x16: x16,
                rec: Some(&rec),
                probe,
            };
            let r = cpu::encode(cpu, word, operand, &site);
            (r, rec.borrow().clone())
        };
        let addr = pc;
        let (mut first, mut ops) = attempt(addr, Probe::default());
        // An address the linker decides is 16 bits wide unless it was declared zero page.
        if !wide.get()
            && ops
                .iter()
                .any(|o| o.dep.as_ref().is_some_and(|d| !d.zeropage))
        {
            wide.set(true);
            (first, ops) = attempt(addr, Probe::default());
        }
        let mut near = None;
        let mut result = first;
        let mut operands = ops;
        let first_error = match &result {
            Err(e) if operands.iter().any(|o| o.dep.is_some()) => Some(e.clone()),
            _ => None,
        };
        if first_error.is_some() {
            // The placeholder offset may be out of range (a branch to another section): try one near the instruction.
            let probe = Probe {
                near: Some(pc as i64),
                shift: None,
            };
            let (r, o) = attempt(addr, probe);
            if r.is_ok() {
                near = probe.near;
                result = r;
                operands = o;
            }
        }
        let enc = match result {
            Ok(e) => e,
            Err(e) => {
                self.error(src, e);
                self.pc = Some(pc + 1);
                return;
            }
        };
        if self.pass == 1 {
            self.wide.insert(id, enc.unresolved || wide.get());
            self.emit(src, &enc.bytes);
            return;
        }
        let cur = self.ob.cur;
        let mut pending: Vec<(u32, u8, obj::Kind, i64, u32, Expr)> = Vec::new();
        for (k, op) in operands.iter().enumerate() {
            let Some(dep) = &op.dep else { continue };
            let base = &enc.bytes;
            let mut changed = vec![false; base.len()];
            for delta in [1i64, 0x100, 0x10000] {
                let probe = Probe {
                    near,
                    shift: Some((k, delta)),
                };
                if let (Ok(e), _) = attempt(addr, probe)
                    && e.bytes.len() == base.len()
                {
                    for (i, c) in changed.iter_mut().enumerate() {
                        *c |= e.bytes[i] != base[i];
                    }
                }
            }
            let Some(first_byte) = changed.iter().position(|c| *c) else {
                continue;
            };
            let last_byte = changed.iter().rposition(|c| *c).unwrap_or(first_byte);
            let width = last_byte - first_byte + 1;
            if width > 4 {
                self.error(
                    src,
                    format!("cannot find the address field of \"{word} {operand}\""),
                );
                return;
            }
            // Relative: the field moves with the instruction's own address.
            let mut rel = false;
            if let Some(moved) = addr.checked_add(0x10) {
                let probe = Probe { near, shift: None };
                if let (Ok(e), _) = attempt(moved, probe)
                    && e.bytes.len() == base.len()
                {
                    rel = (first_byte..=last_byte).any(|i| e.bytes[i] != base[i]);
                }
            }
            let only_here = dep.groups.iter().all(|g| *g == Group::Section(cur));
            if rel && only_here {
                if let Some(e) = &first_error {
                    // The target is in this very section and still out of range: a real error.
                    self.error(src, e.clone());
                    return;
                }
                continue; // a distance inside one section is a constant
            }
            if rel {
                let mut field = 0i64;
                for i in (0..width).rev() {
                    field = (field << 8) | base[first_byte + i] as i64;
                }
                if width < 8 && field & (1 << (width * 8 - 1)) != 0 {
                    field -= 1 << (width * 8);
                }
                let bias = op.value - addr as i64 - field;
                if !(0..=16).contains(&bias) {
                    self.error(
                        src,
                        format!("cannot relocate the branch \"{word} {operand}\""),
                    );
                    return;
                }
                pending.push((
                    first_byte as u32,
                    width as u8,
                    obj::Kind::Rel,
                    bias,
                    first_byte as u32,
                    op.expr.clone(),
                ));
            } else {
                pending.push((
                    first_byte as u32,
                    width as u8,
                    obj::Kind::Abs,
                    0,
                    first_byte as u32,
                    op.expr.clone(),
                ));
            }
        }
        let start = self.ob.sections[cur].size;
        for (at, width, kind, bias, back, expr) in pending {
            self.ob.relocs.push(PendingReloc {
                section: cur,
                offset: start + at,
                width,
                kind,
                bias,
                back,
                expr,
                file: self.file_name(src),
                line: src.line,
            });
        }
        self.emit(src, &enc.bytes);
        self.track_widths(word, operand);
    }

    /// One `.byte` / `.word` value in object mode: the placeholder to write and, when the linker decides the
    /// value, the relocation of the `width` bytes that will sit `at` bytes into this statement's output.
    pub(super) fn object_data(
        &mut self,
        src: &LineSrc,
        part: &str,
        width: u8,
        at: usize,
    ) -> Result<i64, String> {
        let e = expr::parse(part)?;
        let v = self.eval_data(part)?;
        let view = self.view();
        let Some(_dep) = view.dependency_of(&e) else {
            return Ok(v);
        };
        let named = e.map_names(&|n| view.key_of(n));
        if self.pass == 2 {
            let cur = self.ob.cur;
            let offset = self.ob.sections[cur].size + at as u32;
            self.ob.relocs.push(PendingReloc {
                section: cur,
                offset,
                width,
                kind: obj::Kind::Abs,
                bias: 0,
                back: at as u32,
                expr: named,
                file: self.file_name(src),
                line: src.line,
            });
        }
        Ok(v & if width == 1 { 0xFF } else { 0xFFFF })
    }

    pub(super) fn finish_object(self, name: &str) -> Result<obj::Object, Vec<Diag>> {
        let Some(cpu) = self.cpu.or(self.opts.cpu) else {
            return Err(vec![Diag::new(
                name,
                0,
                "no CPU selected: use --cpu or a .cpu directive",
            )]);
        };
        let mut errors = Vec::new();
        let exported: HashSet<&str> = self
            .ob
            .exports
            .iter()
            .map(|(n, _)| n.as_str())
            .chain(self.ob.globals.iter().map(String::as_str))
            .collect();
        for (n, _) in &self.ob.exports {
            match self.symbols.get(n) {
                Some(s) if s.kind != SymKind::Import => {}
                _ => errors.push(Diag::new(
                    name,
                    0,
                    format!("\"{n}\" is exported but never defined"),
                )),
            }
        }
        let mut keys: Vec<&String> = self.symbols.keys().collect();
        keys.sort();
        let mut index: HashMap<&str, u32> = HashMap::new();
        let mut symbols = Vec::new();
        for key in keys {
            let s = &self.symbols[key];
            let is_export = exported.contains(key.as_str());
            if s.kind == SymKind::Const && !is_export {
                continue;
            }
            index.insert(key.as_str(), symbols.len() as u32);
            let (class, section, value) = match s.kind {
                SymKind::Const => (obj::Class::Const, 0, s.value),
                SymKind::Label(i) => (
                    obj::Class::Label,
                    i as u32,
                    s.value - self.ob.sections[i].base() as i64,
                ),
                SymKind::Import => (obj::Class::Import, 0, 0),
            };
            let zeropage = s.zp || matches!(s.kind, SymKind::Label(i) if self.ob.sections[i].zp);
            symbols.push(obj::Symbol {
                name: display_name(key),
                class,
                global: is_export || s.kind == SymKind::Import,
                zeropage,
                section,
                value,
            });
        }
        let mut files: Vec<String> = vec![name.to_string()];
        let mut relocs = Vec::new();
        for r in &self.ob.relocs {
            let file = match files.iter().position(|f| *f == r.file) {
                Some(i) => i,
                None => {
                    files.push(r.file.clone());
                    files.len() - 1
                }
            };
            match to_obj(&r.expr, &index, &self.symbols) {
                Ok(expr) => relocs.push(obj::Reloc {
                    section: r.section as u32,
                    offset: r.offset,
                    width: r.width,
                    big_endian: false,
                    kind: r.kind,
                    bias: r.bias,
                    instr_back: r.back,
                    expr,
                    file: file as u32,
                    line: r.line as u32,
                }),
                Err(e) => errors.push(Diag::new(&r.file, r.line, e)),
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let sections = self
            .ob
            .sections
            .iter()
            .map(|s| obj::Section {
                name: s.name.clone(),
                bss: s.bss,
                zeropage: s.zp,
                align: s.align,
                size: s.size,
                addr: s.addr,
                bytes: if s.bss { Vec::new() } else { s.bytes.clone() },
            })
            .collect();
        Ok(obj::Object {
            cpu: cpu.name().to_string(),
            name: name.to_string(),
            files,
            sections,
            symbols,
            relocs,
        })
    }
}

fn to_obj(
    e: &Expr,
    index: &HashMap<&str, u32>,
    symbols: &HashMap<String, Sym>,
) -> Result<obj::Expr, String> {
    Ok(match e {
        Expr::Num(n) => obj::Expr::Num(*n),
        Expr::Pc => obj::Expr::Pc,
        Expr::Sym(key) => match symbols.get(key) {
            Some(s) if s.kind == SymKind::Const => obj::Expr::Num(s.value),
            Some(_) => match index.get(key.as_str()) {
                Some(i) => obj::Expr::Sym(*i),
                None => return Err(format!("\"{key}\" is not in the symbol table")),
            },
            None => return Err(format!("undefined symbol \"{}\"", display_name(key))),
        },
        Expr::Unary(op, a) => obj::Expr::Unary(*op as u8, Box::new(to_obj(a, index, symbols)?)),
        Expr::Binary(op, a, b) => obj::Expr::Binary(
            op.code(),
            Box::new(to_obj(a, index, symbols)?),
            Box::new(to_obj(b, index, symbols)?),
        ),
    })
}
