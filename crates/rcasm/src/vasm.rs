//! vasm's `oldstyle` and `std` syntax modules for the 8-bit CPUs (6502, HuC6280, Z80): every source line is rewritten
//! into rcasm's own syntax before the two passes see it (`--syntax oldstyle|std`).
//!
//! Written from vasm's manual (`syntax_oldstyle.texi`, `syntax_std.texi`, the 6502 and Z80 chapters), not its code.
//! A directive this layer does not support becomes an `.error` naming it, at its line, so a source fails loudly
//! instead of assembling differently. What is translated:
//!
//! * labels: `oldstyle` at the first column with or without `:`, `std` with `:`; local labels `.name` / `name$`
//!   (rcasm's `@name`), `std`'s digit labels `1:` with `1b` / `1f`; `:` anonymous labels as rcasm has them;
//! * numbers: `@17` (octal; binary on the Z80), `4#123` (any base 2-9), `12d`, `17o`, `17q`, `101b`, `std`'s `017`
//!   (octal), the Z80's `&FF`; `'A` without the closing quote;
//! * operators: `oldstyle`'s `=` as equality in conditions, `std`'s `a!b` (or-not);
//! * data, space, alignment, origin, include, conditional, macro (`\1`..`\9`, `\name`, `\@`, `\()`) and repeat
//!   directives under all the names the manual lists, `set` (a symbol that may be redefined), `.elseif`;
//! * `rorg`/`phase` ... `rend`/`dephase` (rcasm's `.phase`), oldstyle `struct` (field offsets `name.field`, the size `name`,
//!   instances `label name values...` with the defaults), sections (rcasm's `.segment` in object mode; in an image `.text`,
//!   `.data`, `.rodata` and `.bss` go on in one piece and a named section is an error), std's `.irp` / `.irpc` (the block is
//!   repeated with `\name` replaced) and `.ifc` / `.ifnc` (rcasm's: the two texts after macro arguments are put in).

use crate::Cpu;
use crate::text::{split_top, strip_comment};

/// Which source syntax rcasm reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Syntax {
    /// rcasm's own (with the ca65 directives).
    #[default]
    Native,
    /// vasm's `oldstyle` module.
    Oldstyle,
    /// vasm's `std` module (GNU-as style).
    Std,
}

impl Syntax {
    pub fn parse(name: &str) -> Option<Syntax> {
        match name.to_ascii_lowercase().as_str() {
            "native" | "rcasm" => Some(Syntax::Native),
            "oldstyle" | "vasm-oldstyle" => Some(Syntax::Oldstyle),
            "std" | "vasm-std" => Some(Syntax::Std),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Syntax::Native => "rcasm",
            Syntax::Oldstyle => "vasm oldstyle",
            Syntax::Std => "vasm std",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cond {
    NonZero,
    Zero,
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    /// Bytes (strings allowed).
    Bytes,
    /// Values of N bytes, little endian.
    Data(u8),
    AsciiZ,
    StrCr,
    Fcc,
    Fcs,
    /// `n[,fill]` units of N bytes.
    Space(u8),
    /// `n`: zero bytes.
    Zeros,
    /// `.align` as a power of two (`oldstyle` `align`, `std` `.align` / `.p2align`).
    AlignBits,
    /// `.balign`.
    AlignBytes,
    Even,
    Org,
    Include,
    Incbin,
    Equ,
    Set,
    Equiv,
    Defc,
    If(Cond),
    Ifdef(bool),
    Ifused,
    Ifblank(bool),
    Elseif,
    Else,
    Endif,
    Macro,
    Endm,
    Rept,
    Endr,
    End,
    Assert,
    Error,
    Global,
    Dsect,
    Dend,
    Phase,
    Dephase,
    Struct,
    EndStruct,
    /// `section NAME`
    Section,
    /// `.text`, `.data`, `.rodata`, `.bss`: rcasm's segment of that kind.
    Seg(&'static str),
    Ifc(bool),
    /// `.irp` (false) or `.irpc` (true).
    Irp(bool),
    /// `abyte offset, values`: bytes with the offset added to each.
    Abyte,
    /// `roffs offset[, fill]`: pad up to the address `offset` past the last `org`.
    Roffs,
    /// `inline` / `einline`: a block whose local labels are its own.
    Inline,
    Einline,
    /// `incdir path`: another folder to look for include files in.
    Incdir,
    /// `exitmacro`: leave the macro expansion here.
    Exitmacro,
    /// Accepted, no effect (listing control, debug records).
    Ignore,
    Unsupported,
}

fn directive(word: &str, syntax: Syntax, cpu: Option<Cpu>) -> Option<Dir> {
    let w = word.to_ascii_lowercase();
    let z80 = cpu == Some(Cpu::Z80);
    let long = if z80 { 4 } else { 3 };
    match syntax {
        Syntax::Native => None,
        Syntax::Oldstyle => {
            let w = w.strip_prefix('.').unwrap_or(&w);
            Some(match w {
                "byt" | "byte" | "db" | "dfb" | "defb" | "fcb" | "data" | "di8" | "ascii"
                | "asc" => Dir::Bytes,
                "word" | "wor" | "wrd" | "dw" | "dfw" | "defw" | "fdb" | "di16" | "addr" | "da"
                | "defp" => Dir::Data(2),
                "dl" | "defl" => Dir::Data(long),
                "di24" => Dir::Data(3),
                "di32" => Dir::Data(4),
                "asciiz" | "string" => Dir::AsciiZ,
                "str" => Dir::StrCr,
                "fcc" | "defm" | "text" => Dir::Fcc,
                "fcs" => Dir::Fcs,
                "blk" | "bsz" | "ds" | "dsb" | "defs" | "zmb" | "dc" | "ds8" => Dir::Space(1),
                "rmb" if !matches!(cpu, Some(Cpu::Mos6502 | Cpu::Huc6280 | Cpu::W65816)) => {
                    Dir::Space(1)
                }
                "blkw" | "dsw" | "ds16" => Dir::Space(2),
                "blkl" | "dsl" => Dir::Space(long),
                "ds24" => Dir::Space(3),
                "ds32" => Dir::Space(4),
                "fill" | "reserve" | "spc" | "bss" => Dir::Zeros,
                "align" => Dir::AlignBits,
                "even" => Dir::Even,
                "org" => Dir::Org,
                "include" => Dir::Include,
                "incbin" | "binary" | "mdat" => Dir::Incbin,
                "equ" | "eq" => Dir::Equ,
                "set" if cpu != Some(Cpu::Huc6280) => Dir::Set,
                "defc" => Dir::Defc,
                "if" | "cond" | "ifne" => Dir::If(Cond::NonZero),
                "ifeq" => Dir::If(Cond::Zero),
                "ifgt" => Dir::If(Cond::Gt),
                "ifge" => Dir::If(Cond::Ge),
                "iflt" => Dir::If(Cond::Lt),
                "ifle" => Dir::If(Cond::Le),
                "ifdef" | "ifd" => Dir::Ifdef(true),
                "ifndef" | "ifnd" => Dir::Ifdef(false),
                "ifused" => Dir::Ifused,
                "ifblank" => Dir::Ifblank(true),
                "ifnblank" => Dir::Ifblank(false),
                "else" | "el" => Dir::Else,
                "endif" | "endc" | "fi" => Dir::Endif,
                "ei" if !z80 => Dir::Endif,
                "macro" | "mac" => Dir::Macro,
                "endm" | "endmac" | "endmacro" => Dir::Endm,
                "rept" | "repeat" => Dir::Rept,
                "endr" | "endrep" | "endrepeat" => Dir::Endr,
                "end" => Dir::End,
                "assert" => Dir::Assert,
                "fail" => Dir::Error,
                "global" | "xdef" | "xref" | "xlib" | "extern" => Dir::Global,
                "list" | "nolist" => Dir::Ignore,
                "dsect" => Dir::Dsect,
                "dend" => Dir::Dend,
                "rorg" | "phase" => Dir::Phase,
                "rend" | "dephase" => Dir::Dephase,
                "struct" | "structure" => Dir::Struct,
                "endstruct" | "endstructure" => Dir::EndStruct,
                "section" => Dir::Section,
                "abyte" => Dir::Abyte,
                "roffs" => Dir::Roffs,
                "inline" => Dir::Inline,
                "einline" => Dir::Einline,
                "incdir" => Dir::Incdir,
                "exitmacro" | "exitm" | "mexit" => Dir::Exitmacro,
                "di64" => Dir::Data(8),
                "ds64" => Dir::Space(8),
                // object-file hints with no effect on what is assembled
                "local" | "symdepend" | "needs" => Dir::Ignore,
                "weak" => Dir::Unsupported,
                _ => return None,
            })
        }
        Syntax::Std => {
            let w = w.strip_prefix('.')?;
            Some(match w {
                "byte" | "ascii" => Dir::Bytes,
                "short" | "half" | "word" | "2byte" | "uahalf" | "uashort" | "int" => Dir::Data(2),
                "long" | "4byte" | "ualong" | "uaword" => Dir::Data(4),
                "asciiz" | "asciz" | "string" => Dir::AsciiZ,
                "space" | "skip" | "zero" => Dir::Space(1),
                "align" | "p2align" => Dir::AlignBits,
                "balign" => Dir::AlignBytes,
                "org" => Dir::Org,
                "include" => Dir::Include,
                "incbin" => Dir::Incbin,
                "equ" | "set" => Dir::Set,
                "equiv" => Dir::Equiv,
                "if" | "ifne" => Dir::If(Cond::NonZero),
                "ifeq" => Dir::If(Cond::Zero),
                "ifgt" => Dir::If(Cond::Gt),
                "ifge" => Dir::If(Cond::Ge),
                "iflt" => Dir::If(Cond::Lt),
                "ifle" => Dir::If(Cond::Le),
                "ifdef" => Dir::Ifdef(true),
                "ifndef" => Dir::Ifdef(false),
                "ifb" => Dir::Ifblank(true),
                "ifnb" => Dir::Ifblank(false),
                "elseif" => Dir::Elseif,
                "else" => Dir::Else,
                "endif" => Dir::Endif,
                "macro" => Dir::Macro,
                "endm" => Dir::Endm,
                "rept" => Dir::Rept,
                "endr" => Dir::Endr,
                "end" => Dir::End,
                "err" | "abort" => Dir::Error,
                "global" | "globl" | "extern" => Dir::Global,
                "list" | "nolist" | "file" | "size" | "type" | "stabs" | "stabn" | "stabd" => {
                    Dir::Ignore
                }
                "text" => Dir::Seg("CODE"),
                "data" => Dir::Seg("DATA"),
                "rodata" => Dir::Seg("RODATA"),
                "bss" => Dir::Seg("BSS"),
                "section" => Dir::Section,
                "irp" => Dir::Irp(false),
                "irpc" => Dir::Irp(true),
                "ifc" => Dir::Ifc(true),
                "ifnc" => Dir::Ifc(false),
                "8byte" | "quad" | "uaquad" | "double" | "float" | "single" | "comm" | "lcomm"
                | "fail" | "hidden" | "internal" | "protected" | "local" | "weak" | "incdir"
                | "pushsection" | "popsection" | "sdata" | "sbss" | "sdata2" | "dpage" | "tocd"
                | "stab" | "stabstr" | "swbeg" | "balignw" | "balignl" | "p2alignw"
                | "p2alignl" => Dir::Unsupported,
                _ => return None,
            })
        }
    }
}

/// One field of an oldstyle `struct`: the size of a unit, and its default data or a reserved count.
#[derive(Clone, Debug)]
pub struct Field {
    unit: u8,
    kind: FieldKind,
}

#[derive(Clone, Debug)]
enum FieldKind {
    /// Data with default values (operands already in rcasm's terms).
    Data(Vec<String>),
    /// `count` units of `fill`.
    Space(String, String),
}

/// The oldstyle structures defined so far, shared by all the files of a program.
pub type Structs = std::collections::HashMap<String, Vec<Field>>;

/// A `.irp` / `.irpc` block being collected.
struct Irp {
    name: String,
    values: Vec<String>,
    chars: bool,
    body: Vec<String>,
    depth: usize,
}

/// Rewrites the lines of one source file.
pub struct Rewriter<'s> {
    syntax: Syntax,
    cpu: Option<Cpu>,
    /// Building an object (sections become segments) rather than an image.
    object: bool,
    structs: &'s mut Structs,
    /// Inside a `struct`: its name, its fields so far and the offset expression of the next field.
    struct_def: Option<(String, Vec<Field>, String)>,
    irp: Option<Irp>,
    /// Inside a macro definition: its parameter names (`_p1`.. for positional ones).
    macro_params: Option<Vec<String>>,
    /// For each open conditional, how many extra `.endif`s its `.elseif`s need.
    ifs: Vec<usize>,
    /// `std` digit labels: how many of each digit were defined so far.
    digits: [u32; 10],
    /// Open `inline` blocks (innermost last), each with its number: local labels inside belong to it alone.
    inline: Vec<u32>,
    inlines: u32,
    /// The last `org` address (an rcasm expression): `roffs` counts from it.
    origin: String,
}

impl<'s> Rewriter<'s> {
    pub fn new(syntax: Syntax, cpu: Option<Cpu>, object: bool, structs: &'s mut Structs) -> Self {
        Rewriter {
            syntax,
            cpu,
            object,
            structs,
            struct_def: None,
            irp: None,
            macro_params: None,
            ifs: Vec::new(),
            digits: [0; 10],
            inline: Vec::new(),
            inlines: 0,
            origin: "0".to_string(),
        }
    }

    /// The rcasm lines for one source line (one line in, usually one out).
    pub fn line(&mut self, text: &str) -> Vec<String> {
        if self.syntax == Syntax::Native {
            return vec![text.to_string()];
        }
        if self.irp.is_some() {
            return self.irp_line(text);
        }
        let text = if self.macro_params.is_some() {
            self.macro_text(text)
        } else {
            text.to_string()
        };
        // `*` in the first column: a comment, unless it sets the address (`*=`)
        if self.syntax == Syntax::Oldstyle
            && let Some(rest) = text.strip_prefix('*')
            && !rest.trim_start().starts_with('=')
        {
            return vec![String::new()];
        }
        let code = strip_comment(&text).trim_end();
        if code.trim().is_empty() {
            return vec![String::new()];
        }
        let (labels, rest) = self.split_labels(code);
        let rest = rest.trim();
        let (word, operand) = match rest.find(char::is_whitespace) {
            Some(i) => (&rest[..i], rest[i..].trim()),
            None => (rest, ""),
        };
        // `name = expr`, `name equ expr` with the name in the label field
        let mut out = Vec::new();
        if word == "=" || (word.starts_with('=') && !word.starts_with("==")) {
            let value = if word == "=" {
                operand.to_string()
            } else {
                format!("{} {operand}", &word[1..])
            };
            return self.assign(&labels, &value, false);
        }
        let dir = directive(word, self.syntax, self.cpu);
        if self.struct_def.is_some() {
            return self.struct_line(&labels, dir, word, operand);
        }
        if dir.is_none()
            && let Some(fields) = self.structs.get(word).cloned()
        {
            return self.struct_instance(&labels, &fields, operand);
        }
        let Some(dir) = dir else {
            // an instruction or a macro call
            let mut line = self.label_text(&labels);
            if !rest.is_empty() {
                let mnemonic = self.cpu.is_some_and(|c| c.is_mnemonic(word));
                let op = if mnemonic {
                    self.operand(operand)
                } else {
                    self.expr(operand)
                };
                line.push_str(&format!("        {word} {op}"));
            }
            return vec![line];
        };
        let label = labels.last().cloned();
        let lead = |me: &Self| me.label_text(&labels);
        let quoted = |s: &str| {
            let s = s.trim();
            if s.starts_with('"') || s.starts_with('\'') {
                format!("\"{}\"", s.trim_matches(|c| c == '"' || c == '\''))
            } else {
                format!("\"{s}\"")
            }
        };
        let ops = split_top(operand);
        match dir {
            Dir::Equ | Dir::Set | Dir::Equiv | Dir::Defc
                if label.is_some() && matches!(dir, Dir::Equ | Dir::Set) =>
            {
                let mut l = labels.clone();
                l.pop();
                if !l.is_empty() {
                    out.push(self.label_text(&l));
                }
                out.extend(self.assign(&[label.unwrap()], operand, dir == Dir::Set));
                return out;
            }
            Dir::Set | Dir::Equiv => {
                // std: `.set name, value`
                if let [name, value] = ops.as_slice() {
                    out.push(lead(self));
                    out.extend(self.assign(&[name.to_string()], value, dir == Dir::Set));
                    return out;
                }
                out.push(self.unsupported(word, "takes a name and a value"));
            }
            Dir::Defc => match operand.split_once('=') {
                Some((name, value)) => {
                    out.extend(self.assign(&[name.trim().to_string()], value, false))
                }
                None => out.push(self.unsupported(word, "takes name = value")),
            },
            Dir::Equ => out.push(self.unsupported(word, "needs a name in the label field")),
            Dir::Bytes | Dir::Data(_) if operand.is_empty() => {
                let n = if let Dir::Data(n) = dir { n } else { 1 };
                out.push(format!("{}        .fill {n}", lead(self)));
            }
            Dir::Bytes => out.push(format!("{}        .byte {}", lead(self), self.list(&ops))),
            Dir::Data(2) => out.push(format!("{}        .word {}", lead(self), self.list(&ops))),
            Dir::Data(n) => {
                let parts: Vec<String> = ops.iter().map(|e| bytes_of(&self.expr(e), n)).collect();
                out.push(format!("{}        .byte {}", lead(self), parts.join(", ")));
            }
            Dir::AsciiZ | Dir::StrCr => {
                let end = if dir == Dir::AsciiZ { "0" } else { "13" };
                let parts: Vec<String> = ops
                    .iter()
                    .map(|e| format!("{}, {end}", self.expr(e)))
                    .collect();
                out.push(format!("{}        .byte {}", lead(self), parts.join(", ")));
            }
            Dir::Fcc | Dir::Fcs => match delimited(operand) {
                Some(s) if dir == Dir::Fcc => {
                    out.push(format!("{}        .byte {}", lead(self), quote_bytes(&s)));
                }
                Some(s) => {
                    let mut b = s.into_bytes();
                    if let Some(last) = b.last_mut() {
                        *last |= 0x80;
                    }
                    let nums: Vec<String> = b.iter().map(|x| format!("${x:02X}")).collect();
                    out.push(format!("{}        .byte {}", lead(self), nums.join(", ")));
                }
                None => out.push(self.unsupported(word, "wants a delimited string")),
            },
            Dir::Space(unit) => {
                let (n, fill) = match ops.as_slice() {
                    [n] => (self.expr(n), "0".to_string()),
                    [n, f] => (self.expr(n), self.expr(f)),
                    _ => {
                        out.push(self.unsupported(word, "takes a count and a fill value"));
                        return out;
                    }
                };
                if unit == 1 {
                    out.push(format!("{}        .fill {n}, {fill}", lead(self)));
                } else {
                    out.push(lead(self));
                    out.push(format!("        .repeat {n}"));
                    out.push(format!("        .byte {}", bytes_of(&fill, unit)));
                    out.push("        .endrep".to_string());
                }
            }
            Dir::Zeros => out.push(format!(
                "{}        .fill {}",
                lead(self),
                self.expr(operand)
            )),
            Dir::AlignBits | Dir::AlignBytes => {
                let fill = ops
                    .get(1)
                    .map(|f| format!(", {}", self.expr(f)))
                    .unwrap_or_default();
                if ops.len() > 2 {
                    out.push(self.unsupported(word, "with a maximum padding"));
                } else if let Some(n) = ops.first() {
                    let n = self.expr(n);
                    let size = if dir == Dir::AlignBits {
                        format!("1 << ({n})")
                    } else {
                        n
                    };
                    out.push(format!("{}        .align {size}{fill}", lead(self)));
                }
            }
            Dir::Even => out.push(format!("{}        .align 2", lead(self))),
            Dir::Org => {
                let a = ops.first().copied().unwrap_or("").trim_start_matches('#');
                self.origin = self.expr(a);
                out.push(format!("{}        .org {}", lead(self), self.origin));
            }
            Dir::Roffs => {
                // the address `offset` past the last `org`, padded up to
                let fill = ops
                    .get(1)
                    .map(|f| self.expr(f))
                    .unwrap_or_else(|| "0".to_string());
                let off = self.expr(ops.first().copied().unwrap_or("0"));
                out.push(format!(
                    "{}        .fill ({}) + ({off}) - *, {fill}",
                    lead(self),
                    self.origin
                ));
            }
            Dir::Abyte => match ops.split_first() {
                Some((off, values)) if !values.is_empty() => {
                    let off = self.expr(off);
                    let mut parts = Vec::new();
                    for v in values {
                        let v = v.trim();
                        match delimited(v) {
                            Some(s) if v.starts_with('"') || v.starts_with('\'') => parts
                                .extend(s.bytes().map(|b| format!("(${b:02X} + ({off})) & $FF"))),
                            _ => parts.push(format!("(({}) + ({off})) & $FF", self.expr(v))),
                        }
                    }
                    out.push(format!("{}        .byte {}", lead(self), parts.join(", ")));
                }
                _ => out.push(self.unsupported(word, "takes an offset and values")),
            },
            Dir::Inline => {
                self.inlines += 1;
                self.inline.push(self.inlines);
                out.push(lead(self));
            }
            Dir::Einline => {
                if self.inline.pop().is_none() {
                    out.push(self.unsupported(word, "without inline"));
                } else {
                    out.push(lead(self));
                }
            }
            Dir::Incdir => out.push(format!("{}        .incdir {}", lead(self), quoted(operand))),
            Dir::Exitmacro => out.push(format!("{}        .exitmacro", lead(self))),
            Dir::Include => out.push(format!(
                "{}        .include {}",
                lead(self),
                quoted(operand)
            )),
            Dir::Incbin => {
                let mut parts = vec![quoted(ops.first().copied().unwrap_or(""))];
                parts.extend(ops.iter().skip(1).map(|e| self.expr(e)));
                out.push(format!(
                    "{}        .incbin {}",
                    lead(self),
                    parts.join(", ")
                ));
            }
            Dir::If(c) => {
                self.ifs.push(0);
                let e = self.condition(operand);
                let test = match c {
                    Cond::NonZero => format!("({e})"),
                    Cond::Zero => format!("({e}) == 0"),
                    Cond::Gt => format!("({e}) > 0"),
                    Cond::Ge => format!("({e}) >= 0"),
                    Cond::Lt => format!("({e}) < 0"),
                    Cond::Le => format!("({e}) <= 0"),
                };
                out.push(format!("        .if {test}"));
            }
            Dir::Ifdef(yes) => {
                self.ifs.push(0);
                let d = if yes { ".ifdef" } else { ".ifndef" };
                out.push(format!("        {d} {}", self.expr(operand)));
            }
            Dir::Ifused => {
                self.ifs.push(0);
                out.push(format!("        .ifref {}", self.expr(operand)));
            }
            Dir::Ifblank(yes) => {
                self.ifs.push(0);
                let d = if yes { ".ifblank" } else { ".ifnblank" };
                out.push(format!("        {d} {operand}"));
            }
            Dir::Elseif => {
                if let Some(extra) = self.ifs.last_mut() {
                    *extra += 1;
                }
                out.push("        .else".to_string());
                out.push(format!("        .if ({})", self.condition(operand)));
            }
            Dir::Else => out.push("        .else".to_string()),
            Dir::Endif => {
                let extra = self.ifs.pop().unwrap_or(0);
                for _ in 0..=extra {
                    out.push("        .endif".to_string());
                }
            }
            Dir::Macro => {
                // oldstyle: `name macro [args]` or `macro name[,args]`; std: `.macro name arg1 arg2` / `name, arg1`
                let (name, args): (String, Vec<String>) = match (&label, self.syntax) {
                    (Some(l), Syntax::Oldstyle) => {
                        (l.clone(), ops.iter().map(|s| s.to_string()).collect())
                    }
                    _ => {
                        let mut it = operand
                            .split(|c: char| c == ',' || c.is_whitespace())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string());
                        let name = it.next().unwrap_or_default();
                        (name, it.collect())
                    }
                };
                if args.iter().any(|a| a.contains('=')) {
                    out.push(self.unsupported(word, "with default argument values"));
                    return out;
                }
                let params: Vec<String> = if args.is_empty() {
                    (1..=9).map(|i| format!("_p{i}")).collect()
                } else {
                    args
                };
                out.push(format!("        .macro {name} {}", params.join(", ")));
                self.macro_params = Some(params);
            }
            Dir::Endm => {
                self.macro_params = None;
                out.push(format!("{}        .endm", lead(self)));
            }
            Dir::Rept => out.push(format!("{}        .repeat {}", lead(self), self.list(&ops))),
            Dir::Endr => out.push(format!("{}        .endrep", lead(self))),
            Dir::End => out.push(format!("{}        .end", lead(self))),
            Dir::Assert => {
                let msg = ops
                    .get(1)
                    .map(|m| format!(", {}", quoted(m)))
                    .unwrap_or_default();
                let e = self.condition(ops.first().copied().unwrap_or("0"));
                out.push(format!("{}        .assert {e}{msg}", lead(self)));
            }
            Dir::Error => out.push(format!("{}        .error {}", lead(self), quoted(operand))),
            Dir::Global => out.push(format!("{}        .global {}", lead(self), self.list(&ops))),
            Dir::Phase => out.push(format!(
                "{}        .phase {}",
                lead(self),
                self.expr(operand)
            )),
            Dir::Dephase => out.push(format!("{}        .dephase", lead(self))),
            Dir::Struct => {
                // `name struct` or `struct name`
                let name = match (&label, operand.trim()) {
                    (_, op) if !op.is_empty() => op.to_string(),
                    (Some(l), _) => l.clone(),
                    _ => {
                        out.push(self.unsupported(word, "needs a name"));
                        return out;
                    }
                };
                self.struct_def = Some((name, Vec::new(), "0".to_string()));
            }
            Dir::EndStruct => out.push(self.unsupported(word, "without struct")),
            Dir::Section if self.object => {
                let name = ops
                    .first()
                    .map(|s| s.trim().trim_matches('"'))
                    .unwrap_or("");
                out.push(format!("{}        .segment \"{name}\"", lead(self)));
            }
            Dir::Section => out.push(self.unsupported(
                word,
                "needs -f object (an image is one piece); it is not supported",
            )),
            Dir::Seg(name) if self.object => {
                out.push(format!("{}        .segment \"{name}\"", lead(self)))
            }
            Dir::Seg(_) => out.push(lead(self)),
            Dir::Ifc(yes) => {
                self.ifs.push(0);
                let d = if yes { ".ifc" } else { ".ifnc" };
                out.push(format!("        {d} {operand}"));
            }
            Dir::Irp(chars) => match ops.split_first() {
                Some((name, values)) if !name.is_empty() => {
                    self.irp = Some(Irp {
                        name: name.trim().to_string(),
                        values: values.iter().map(|v| v.trim().to_string()).collect(),
                        chars,
                        body: Vec::new(),
                        depth: 0,
                    });
                    out.push(lead(self));
                }
                _ => out.push(self.unsupported(word, "needs a name")),
            },
            Dir::Dsect => out.push(format!("{}        .dsect", lead(self))),
            Dir::Dend => out.push(format!("{}        .dend", lead(self))),
            Dir::Ignore => out.push(lead(self)),
            Dir::Unsupported => out.push(self.unsupported(word, "is not supported")),
        }
        out
    }

    /// The first word of a line after its labels, lower case (to find the end of a `.irp` block).
    fn word_of(&self, text: &str) -> String {
        let code = strip_comment(text);
        let mut rest = code.trim_end();
        if self.syntax == Syntax::Oldstyle && !rest.starts_with(char::is_whitespace) {
            rest = rest.split_once(char::is_whitespace).map_or("", |(_, r)| r);
        }
        let mut t = rest.trim_start();
        while let Some(i) = t.find(':') {
            if i > 0 && t[..i].chars().all(is_ident) {
                t = t[i + 1..].trim_start();
            } else {
                break;
            }
        }
        t.split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    }

    /// A line inside a `.irp` / `.irpc` block: kept until its `.endr`, then the block is put out once for each value.
    fn irp_line(&mut self, text: &str) -> Vec<String> {
        let word = self.word_of(text);
        let w = word.strip_prefix('.').unwrap_or(&word);
        let irp = self.irp.as_mut().expect("in a .irp block");
        match w {
            "irp" | "irpc" | "rept" => irp.depth += 1,
            "endr" if irp.depth > 0 => irp.depth -= 1,
            "endr" => {
                let irp = self.irp.take().expect("in a .irp block");
                let values: Vec<String> = if irp.chars {
                    irp.values
                        .iter()
                        .flat_map(|v| v.chars().map(String::from).collect::<Vec<_>>())
                        .collect()
                } else if irp.values.is_empty() {
                    vec![String::new()]
                } else {
                    irp.values.clone()
                };
                let mut out = Vec::new();
                for v in &values {
                    for line in &irp.body {
                        let line = line
                            .replace(&format!("\\{}\\()", irp.name), v)
                            .replace(&format!("\\{}", irp.name), v);
                        out.extend(self.line(&line));
                    }
                }
                return out;
            }
            _ => {}
        }
        irp.body.push(text.to_string());
        vec![String::new()]
    }

    /// A line inside an oldstyle `struct`: data and space directives are fields (their labels become `struct__label` offsets),
    /// instructions are left out; `endstruct` defines the size.
    fn struct_line(
        &mut self,
        labels: &[String],
        dir: Option<Dir>,
        word: &str,
        operand: &str,
    ) -> Vec<String> {
        let (name, mut fields, offset) = self.struct_def.take().expect("in a struct");
        let ops = split_top(operand);
        let field = match dir {
            Some(Dir::EndStruct) => {
                let size = format!("{name} = {offset}");
                self.structs.insert(name, fields);
                return vec![size];
            }
            Some(Dir::Bytes) => Some((
                1,
                FieldKind::Data(ops.iter().map(|e| self.expr(e)).collect()),
            )),
            Some(Dir::Data(n)) => Some((
                n,
                FieldKind::Data(ops.iter().map(|e| self.expr(e)).collect()),
            )),
            Some(Dir::Space(n)) => {
                let count = ops
                    .first()
                    .map(|e| self.expr(e))
                    .unwrap_or_else(|| "0".to_string());
                let fill = ops
                    .get(1)
                    .map(|e| self.expr(e))
                    .unwrap_or_else(|| "0".to_string());
                Some((n, FieldKind::Space(count, fill)))
            }
            Some(Dir::Zeros) => Some((1, FieldKind::Space(self.expr(operand), "0".to_string()))),
            _ => None,
        };
        let mut out = Vec::new();
        let mut next = offset.clone();
        if let Some((unit, kind)) = field {
            let label = labels.last().filter(|l| *l != ":").cloned();
            if let Some(l) = &label {
                out.push(format!("{name}__{l} = {offset}"));
            }
            let size = match &kind {
                FieldKind::Data(vals) if vals.is_empty() => format!("{unit}"),
                FieldKind::Data(vals) => {
                    let n: usize = vals.iter().map(|v| string_len(v).unwrap_or(1)).sum();
                    format!("{}", n * usize::from(unit))
                }
                FieldKind::Space(count, _) => format!("({count}) * {unit}"),
            };
            next = format!("({offset}) + {size}");
            fields.push(Field { unit, kind });
        } else if !word.is_empty() && dir.is_none() {
            // an instruction in a structure is ignored, as vasm does
        }
        self.struct_def = Some((name, fields, next));
        out
    }

    /// `label name values...`: the structure's fields with the given values (an empty one keeps the default).
    fn struct_instance(
        &mut self,
        labels: &[String],
        fields: &[Field],
        operand: &str,
    ) -> Vec<String> {
        let given = split_top(operand);
        let mut out = vec![self.label_text(labels)];
        for (i, f) in fields.iter().enumerate() {
            let value = given
                .get(i)
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .map(|v| self.expr(v));
            match &f.kind {
                FieldKind::Data(defaults) => {
                    let vals: Vec<String> = match &value {
                        Some(v) => vec![v.clone()],
                        None if defaults.is_empty() => vec!["0".to_string()],
                        None => defaults.clone(),
                    };
                    out.push(match f.unit {
                        1 => format!("        .byte {}", vals.join(", ")),
                        2 => format!("        .word {}", vals.join(", ")),
                        n => format!(
                            "        .byte {}",
                            vals.iter()
                                .map(|v| bytes_of(v, n))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    });
                }
                FieldKind::Space(count, fill) => {
                    if f.unit == 1 {
                        out.push(format!("        .fill {count}, {fill}"));
                    } else {
                        out.push(format!("        .repeat {count}"));
                        out.push(format!("        .byte {}", bytes_of(fill, f.unit)));
                        out.push("        .endrep".to_string());
                    }
                }
            }
        }
        out
    }

    fn unsupported(&self, word: &str, why: &str) -> String {
        format!(
            "        .error \"{}: {word} {why} by rcasm\"",
            self.syntax.name()
        )
    }

    fn assign(&self, names: &[String], value: &str, redefinable: bool) -> Vec<String> {
        let name = self.symbol(names.last().map(|s| s.as_str()).unwrap_or(""));
        let value = self.expr(value);
        if redefinable {
            vec![format!("        .set {name}, {value}")]
        } else {
            vec![format!("{name} = {value}")]
        }
    }

    /// The label field: (labels, the rest of the line).
    fn split_labels(&mut self, code: &str) -> (Vec<String>, String) {
        let mut labels = Vec::new();
        let mut rest = code;
        match self.syntax {
            Syntax::Oldstyle => {
                if !rest.starts_with(char::is_whitespace) {
                    if let Some(after) = rest.strip_prefix(':')
                        && (after.is_empty() || after.starts_with(char::is_whitespace))
                    {
                        labels.push(":".to_string());
                        rest = after;
                    } else {
                        let end = rest
                            .find(|c: char| c.is_whitespace() || c == ':' || c == '=')
                            .unwrap_or(rest.len());
                        let name = &rest[..end];
                        if !name.is_empty() && !name.starts_with('*') {
                            labels.push(name.to_string());
                            rest = &rest[end..];
                            rest = rest.strip_prefix(':').unwrap_or(rest);
                        }
                    }
                } else {
                    // an indented `name:` is a label too
                    let t = rest.trim_start();
                    if let Some(i) = t.find(':')
                        && i > 0
                        && t[..i].chars().all(is_ident)
                        && t[i + 1..].chars().next().is_none_or(char::is_whitespace)
                    {
                        labels.push(t[..i].to_string());
                        rest = &t[i + 1..];
                    }
                }
            }
            Syntax::Std | Syntax::Native => loop {
                let t = rest.trim_start();
                let Some(i) = t.find(':') else { break };
                let name = &t[..i];
                if name.is_empty()
                    || !name.chars().all(is_ident)
                    || name.starts_with(|c: char| c.is_ascii_digit())
                        && !name.chars().all(|c| c.is_ascii_digit())
                {
                    break;
                }
                if name.len() == 1 && name.chars().all(|c| c.is_ascii_digit()) {
                    let d = (name.as_bytes()[0] - b'0') as usize;
                    self.digits[d] += 1;
                    labels.push(format!("__d{d}_{}", self.digits[d]));
                } else {
                    labels.push(name.to_string());
                }
                rest = &t[i + 1..];
            },
        }
        (labels, rest.to_string())
    }

    fn label_text(&self, labels: &[String]) -> String {
        labels
            .iter()
            .map(|l| {
                if l == ":" {
                    ":".to_string()
                } else {
                    format!("{}:", self.symbol(l))
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A symbol name in rcasm's terms (local labels become `@` labels; inside an `inline` block, names of that block alone).
    fn symbol(&self, name: &str) -> String {
        let name = name.trim();
        let local = if let Some(rest) = name.strip_prefix('.')
            && !rest.is_empty()
        {
            rest.to_string()
        } else if let Some(stem) = name.strip_suffix('$')
            && !stem.is_empty()
            && stem.chars().all(is_ident)
        {
            format!("{stem}_d")
        } else {
            return name.to_string();
        };
        match self.inline.last() {
            // an `@` label (a global one would start a new scope for the `@` labels after the block)
            Some(n) => format!("@__inline{n}_{local}"),
            None => format!("@{local}"),
        }
    }

    fn list(&self, ops: &[&str]) -> String {
        ops.iter()
            .map(|e| self.expr(e))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// An instruction's operand: a quoted single character is a character constant (`#"A"`, `#"A`), and blanks
    /// around commas go (`(ptr), y`).
    fn operand(&self, text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut s = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '"'
                && let Some(&c) = chars.get(i + 1)
            {
                s.push('\'');
                s.push(c);
                s.push('\'');
                i += 2;
                if chars.get(i) == Some(&'"') {
                    i += 1;
                }
                continue;
            }
            s.push(chars[i]);
            i += 1;
        }
        let e = self.expr(&s);
        let mut out = String::new();
        let mut quote = false;
        for c in e.chars() {
            if c == '\'' {
                quote = !quote;
            }
            if !quote && c.is_whitespace() && (out.ends_with(',') || out.is_empty()) {
                continue;
            }
            if !quote && c == ',' {
                while out.ends_with(char::is_whitespace) {
                    out.pop();
                }
            }
            out.push(c);
        }
        out
    }

    /// A condition: `oldstyle`'s single `=` is equality.
    fn condition(&self, e: &str) -> String {
        let e = self.expr(e);
        if self.syntax != Syntax::Oldstyle {
            return e;
        }
        let b = e.as_bytes();
        let mut out = String::new();
        let mut quote = None;
        for (i, c) in e.char_indices() {
            if let Some(q) = quote {
                if c == q {
                    quote = None;
                }
            } else if c == '"' {
                quote = Some(c);
            } else if c == '='
                && !matches!(b.get(i.wrapping_sub(1)), Some(b'=' | b'!' | b'<' | b'>'))
                && b.get(i + 1) != Some(&b'=')
            {
                out.push_str("==");
                continue;
            }
            out.push(c);
        }
        out
    }

    /// An expression or operand in rcasm's terms.
    fn expr(&self, text: &str) -> String {
        let z80 = self.cpu == Some(Cpu::Z80);
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        // the last character that is not a space: decides whether `&`/`!` are operators
        let operand_before = |out: &str| {
            out.trim_end()
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == ')' || c == '_')
        };
        while i < chars.len() {
            let c = chars[i];
            match c {
                '"' => {
                    let start = i;
                    i += 1;
                    while i < chars.len() && chars[i] != '"' {
                        if chars[i] == '\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                    i = (i + 1).min(chars.len());
                    out.extend(&chars[start..i]);
                    continue;
                }
                '\'' if !operand_before(&out) => {
                    // `'A'` or `'A`
                    if let Some(&ch) = chars.get(i + 1) {
                        out.push('\'');
                        out.push(ch);
                        out.push('\'');
                        i += 2;
                        if chars.get(i) == Some(&'\'') {
                            i += 1;
                        }
                        continue;
                    }
                }
                '@' if self.syntax == Syntax::Oldstyle
                    && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) =>
                {
                    let start = i + 1;
                    let mut j = start;
                    while j < chars.len() && chars[j].is_ascii_alphanumeric() {
                        j += 1;
                    }
                    let digits: String = chars[start..j].iter().collect();
                    let radix = if z80 { 2 } else { 8 };
                    match i64::from_str_radix(&digits, radix) {
                        Ok(v) => out.push_str(&v.to_string()),
                        Err(_) => out.push_str(&format!("@{digits}")),
                    }
                    i = j;
                    continue;
                }
                '&' if z80
                    && self.syntax == Syntax::Oldstyle
                    && !operand_before(&out)
                    && chars.get(i + 1).is_some_and(|c| c.is_ascii_hexdigit()) =>
                {
                    out.push('$');
                    i += 1;
                    continue;
                }
                '!' if self.syntax == Syntax::Std
                    && operand_before(&out)
                    && chars.get(i + 1) != Some(&'=') =>
                {
                    out.push_str("|~");
                    i += 1;
                    continue;
                }
                _ => {}
            }
            if c.is_ascii_digit()
                && !out
                    .chars()
                    .last()
                    .is_some_and(|p| p.is_ascii_alphanumeric() || p == '_' || p == '$' || p == '%')
            {
                let start = i;
                let mut j = i;
                while j < chars.len()
                    && (chars[j].is_ascii_alphanumeric() || chars[j] == '#' || chars[j] == '_')
                {
                    j += 1;
                }
                let tok: String = chars[start..j].iter().collect();
                out.push_str(&self.number(&tok));
                i = j;
                continue;
            }
            if c == '.' || c.is_ascii_alphabetic() || c == '_' {
                let start = i;
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '$' {
                    j += 1;
                }
                let tok: String = chars[start..j].iter().collect();
                // `name.field` of an oldstyle structure
                if chars.get(j) == Some(&'.') && self.structs.contains_key(&tok) {
                    let mut k = j + 1;
                    while k < chars.len() && (chars[k].is_ascii_alphanumeric() || chars[k] == '_') {
                        k += 1;
                    }
                    if k > j + 1 {
                        let field: String = chars[j + 1..k].iter().collect();
                        out.push_str(&format!("{tok}__{field}"));
                        i = k;
                        continue;
                    }
                }
                out.push_str(&if tok == "." {
                    tok.clone()
                } else {
                    self.symbol(&tok)
                });
                i = j;
                continue;
            }
            out.push(c);
            i += 1;
        }
        out
    }

    /// A number token (`std` digit-label references `1b` / `1f` too).
    fn number(&self, tok: &str) -> String {
        let lower = tok.to_ascii_lowercase();
        if self.syntax == Syntax::Std && tok.len() == 2 && tok.as_bytes()[0].is_ascii_digit() {
            let d = (tok.as_bytes()[0] - b'0') as usize;
            match &lower[1..] {
                "b" => return format!("__d{d}_{}", self.digits[d]),
                "f" => return format!("__d{d}_{}", self.digits[d] + 1),
                _ => {}
            }
        }
        if let Some((base, digits)) = lower.split_once('#')
            && let Ok(b) = base.parse::<u32>()
            && (2..=9).contains(&b)
            && let Ok(v) = i64::from_str_radix(digits, b)
        {
            return v.to_string();
        }
        if lower.starts_with("0x") || lower.ends_with('h') {
            return tok.to_string();
        }
        if let Some(rest) = lower.strip_prefix("0b")
            && rest.chars().all(|c| c == '0' || c == '1')
            && !rest.is_empty()
        {
            return tok.to_string();
        }
        let (body, radix) = match lower.chars().last() {
            Some('d') => (&lower[..lower.len() - 1], 10),
            Some('o') | Some('q') => (&lower[..lower.len() - 1], 8),
            Some('b') => (&lower[..lower.len() - 1], 2),
            _ if self.syntax == Syntax::Std && lower.len() > 1 && lower.starts_with('0') => {
                (&lower[1..], 8)
            }
            _ => return tok.to_string(),
        };
        match i64::from_str_radix(body, radix) {
            Ok(v) => v.to_string(),
            Err(_) => tok.to_string(),
        }
    }

    /// Inside a macro body: `\1`..`\9` and `\name` are the parameters, `name\@` a label of its own per expansion,
    /// `\()` separates.
    fn macro_text(&self, text: &str) -> String {
        let params = self.macro_params.as_ref().expect("in a macro");
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' {
                let next = chars.get(i + 1).copied();
                match next {
                    Some(d) if d.is_ascii_digit() && d != '0' => {
                        out.push_str(
                            &params
                                .get((d as u8 - b'1') as usize)
                                .cloned()
                                .unwrap_or_default(),
                        );
                        i += 2;
                        continue;
                    }
                    Some('@') => {
                        // the label the `\@` is attached to becomes an `@` label (one per expansion)
                        let stem_len = out.chars().rev().take_while(|c| is_ident(*c)).count();
                        let stem: String =
                            out.chars().skip(out.chars().count() - stem_len).collect();
                        for _ in 0..stem_len {
                            out.pop();
                        }
                        out.push('@');
                        out.push_str(&stem);
                        out.push_str("_m");
                        i += 2;
                        continue;
                    }
                    Some('(') if chars.get(i + 2) == Some(&')') => {
                        i += 3;
                        continue;
                    }
                    Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                        let mut j = i + 1;
                        while j < chars.len()
                            && (chars[j].is_ascii_alphanumeric() || chars[j] == '_')
                        {
                            j += 1;
                        }
                        let name: String = chars[i + 1..j].iter().collect();
                        if params.contains(&name) {
                            out.push_str(&name);
                            i = j;
                            continue;
                        }
                    }
                    _ => {}
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$' || c == '@'
}

/// `e` as `n` little-endian bytes, for `.byte`.
fn bytes_of(e: &str, n: u8) -> String {
    (0..n)
        .map(|k| {
            if k == 0 {
                format!("({e}) & $FF")
            } else {
                format!("(({e}) >> {}) & $FF", 8 * k)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The byte count of a `"..."` operand, `None` for an expression.
fn string_len(v: &str) -> Option<usize> {
    let v = v.trim();
    let body = v.strip_prefix('"')?.strip_suffix('"')?;
    crate::text::unescape(body).ok().map(|b| b.len())
}

/// `fcc`'s string: any printable delimiter.
fn delimited(operand: &str) -> Option<String> {
    let mut it = operand.trim().chars();
    let d = it.next()?;
    let rest: String = it.collect();
    let end = rest.find(d)?;
    Some(rest[..end].to_string())
}

fn quote_bytes(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
