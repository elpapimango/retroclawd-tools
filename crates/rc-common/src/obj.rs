//! Relocatable object files (`.rco`) and libraries (`.rca`). The byte layout is specified in `docs/formats.md`.
//!
//! Both readers treat their input as hostile: every count and length is checked against the bytes that are left
//! before anything is allocated, every index is checked against its table, and nothing panics. A bad file is an
//! `Err` that says where it went wrong.

use std::collections::BTreeMap;

pub const OBJECT_MAGIC: &[u8; 4] = b"RCO1";
pub const ARCHIVE_MAGIC: &[u8; 4] = b"RCA1";

/// The largest file read (64 MiB) and the largest tables an object may hold.
pub const MAX_FILE: usize = 64 << 20;
pub const MAX_SECTIONS: usize = 65_535;
pub const MAX_SYMBOLS: usize = 1 << 20;
pub const MAX_RELOCS: usize = 4 << 20;
pub const MAX_SECTION_SIZE: u32 = 1 << 28;

const BSS: u8 = 1;
const ZEROPAGE: u8 = 2;
const FIXED: u8 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    /// Uninitialised: it has a size and no bytes.
    pub bss: bool,
    /// Lives in the CPU's zero page (the linker puts it in a zero-page region).
    pub zeropage: bool,
    /// Alignment in bytes (a power of two, at least 1).
    pub align: u32,
    pub size: u32,
    /// A fixed address (an absolute section); `None` for a section the linker places.
    pub addr: Option<u32>,
    /// `size` bytes, or empty for a bss section.
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// An address inside `section`, `value` bytes from its start.
    Label,
    /// A constant.
    Const,
    /// Defined by another object.
    Import,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub class: Class,
    /// Visible to other objects (an export, or an import).
    pub global: bool,
    /// Zero page: a reference may use the short addressing mode.
    pub zeropage: bool,
    /// For a label: the section index.
    pub section: u32,
    /// A label's offset in its section, or a constant's value.
    pub value: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The field holds the expression's value.
    Abs,
    /// The field holds the expression's value minus (the address of the instruction plus `bias`).
    Rel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reloc {
    pub section: u32,
    /// Where the field starts in the section.
    pub offset: u32,
    /// 1 to 4 bytes.
    pub width: u8,
    pub big_endian: bool,
    pub kind: Kind,
    /// For `Rel`: the field is `value - (instruction address + bias)`, the instruction's address being the
    /// section's address plus `offset - instr_back`.
    pub bias: i64,
    pub instr_back: u32,
    /// The expression, in the assembler's own syntax; its symbols are this object's names.
    pub expr: String,
    /// Index into `Object::files` and a line, for messages.
    pub file: u32,
    pub line: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Object {
    /// `6502`, `z80`, `huc6280` or `68000`.
    pub cpu: String,
    /// The source the object came from.
    pub name: String,
    pub files: Vec<String>,
    pub sections: Vec<Section>,
    pub symbols: Vec<Symbol>,
    pub relocs: Vec<Reloc>,
}

// ---------------------------------------------------------------------------------------------
// writing

fn put_u(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn put_i(out: &mut Vec<u8>, v: i64) {
    put_u(out, ((v << 1) ^ (v >> 63)) as u64);
}

fn put_s(out: &mut Vec<u8>, s: &str) {
    put_u(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

impl Object {
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::new();
        o.extend_from_slice(OBJECT_MAGIC);
        put_s(&mut o, &self.cpu);
        put_s(&mut o, &self.name);
        put_u(&mut o, self.files.len() as u64);
        for f in &self.files {
            put_s(&mut o, f);
        }
        put_u(&mut o, self.sections.len() as u64);
        for s in &self.sections {
            put_s(&mut o, &s.name);
            let mut flags = 0;
            if s.bss {
                flags |= BSS;
            }
            if s.zeropage {
                flags |= ZEROPAGE;
            }
            if s.addr.is_some() {
                flags |= FIXED;
            }
            o.push(flags);
            put_u(&mut o, s.align as u64);
            put_u(&mut o, s.size as u64);
            if let Some(a) = s.addr {
                put_u(&mut o, a as u64);
            }
            if !s.bss {
                o.extend_from_slice(&s.bytes);
            }
        }
        put_u(&mut o, self.symbols.len() as u64);
        for s in &self.symbols {
            put_s(&mut o, &s.name);
            let class = match s.class {
                Class::Label => 0u8,
                Class::Const => 1,
                Class::Import => 2,
            };
            o.push(class | (s.global as u8) << 4 | (s.zeropage as u8) << 5);
            if s.class == Class::Label {
                put_u(&mut o, s.section as u64);
            }
            if s.class != Class::Import {
                put_i(&mut o, s.value);
            }
        }
        put_u(&mut o, self.relocs.len() as u64);
        for r in &self.relocs {
            put_u(&mut o, r.section as u64);
            put_u(&mut o, r.offset as u64);
            let kind = matches!(r.kind, Kind::Rel) as u8;
            o.push(r.width | kind << 4 | (r.big_endian as u8) << 5);
            if r.kind == Kind::Rel {
                put_i(&mut o, r.bias);
                put_u(&mut o, r.instr_back as u64);
            }
            put_s(&mut o, &r.expr);
            put_u(&mut o, r.file as u64);
            put_u(&mut o, r.line as u64);
        }
        o
    }
}

// ---------------------------------------------------------------------------------------------
// reading

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn fail<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} (at byte {})", self.at))
    }

    fn left(&self) -> usize {
        self.data.len() - self.at
    }

    fn byte(&mut self) -> Result<u8, String> {
        match self.data.get(self.at) {
            Some(b) => {
                self.at += 1;
                Ok(*b)
            }
            None => self.fail("the file ends in the middle of a value"),
        }
    }

    fn uint(&mut self) -> Result<u64, String> {
        let mut v = 0u64;
        for shift in (0..70).step_by(7) {
            let b = self.byte()?;
            if shift == 63 && b > 1 {
                return self.fail("a number is larger than 64 bits");
            }
            v |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        self.fail("a number is longer than 10 bytes")
    }

    fn int(&mut self) -> Result<i64, String> {
        let u = self.uint()?;
        Ok(((u >> 1) as i64) ^ -((u & 1) as i64))
    }

    fn small(&mut self, what: &str, max: u64) -> Result<u32, String> {
        let v = self.uint()?;
        if v > max {
            return self.fail(&format!("{what} {v} is larger than {max}"));
        }
        Ok(v as u32)
    }

    /// A count of items that each take at least `min` bytes.
    fn count(&mut self, what: &str, max: usize, min: usize) -> Result<usize, String> {
        let n = self.uint()?;
        if n > max as u64 {
            return self.fail(&format!("{n} {what} is more than the {max} allowed"));
        }
        if n as usize * min > self.left() {
            return self.fail(&format!("{n} {what} cannot fit in the rest of the file"));
        }
        Ok(n as usize)
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        if n > self.left() {
            return self.fail(&format!("{n} bytes are claimed, {} are left", self.left()));
        }
        let s = &self.data[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }

    fn string(&mut self) -> Result<String, String> {
        let n = self.uint()?;
        if n > self.left() as u64 {
            return self.fail(&format!("a string of {n} bytes does not fit"));
        }
        let b = self.bytes(n as usize)?;
        match std::str::from_utf8(b) {
            Ok(s) => Ok(s.to_string()),
            Err(_) => self.fail("a string is not UTF-8"),
        }
    }
}

impl Object {
    pub fn decode(data: &[u8]) -> Result<Object, String> {
        if data.len() > MAX_FILE {
            return Err(format!("object larger than {MAX_FILE} bytes"));
        }
        if data.len() < 4 || &data[..4] != OBJECT_MAGIC {
            return Err("not an rcasm object (the file does not start with RCO1)".to_string());
        }
        let mut r = Reader { data, at: 4 };
        let cpu = r.string()?;
        let name = r.string()?;
        let nfiles = r.count("files", 1 << 20, 1)?;
        let mut files = Vec::with_capacity(nfiles);
        for _ in 0..nfiles {
            files.push(r.string()?);
        }
        let nsec = r.count("sections", MAX_SECTIONS, 4)?;
        let mut sections = Vec::with_capacity(nsec);
        for _ in 0..nsec {
            let name = r.string()?;
            let flags = r.byte()?;
            if flags & !(BSS | ZEROPAGE | FIXED) != 0 {
                return r.fail("a section has unknown flags");
            }
            let align = r.small("alignment", 1 << 16)?;
            if align == 0 || !align.is_power_of_two() {
                return r.fail(&format!("alignment {align} is not a power of two"));
            }
            let size = r.small("section size", MAX_SECTION_SIZE as u64)?;
            let addr = if flags & FIXED != 0 {
                Some(r.small("section address", u32::MAX as u64)?)
            } else {
                None
            };
            let bss = flags & BSS != 0;
            let bytes = if bss {
                Vec::new()
            } else {
                r.bytes(size as usize)?.to_vec()
            };
            sections.push(Section {
                name,
                bss,
                zeropage: flags & ZEROPAGE != 0,
                align,
                size,
                addr,
                bytes,
            });
        }
        let nsym = r.count("symbols", MAX_SYMBOLS, 2)?;
        let mut symbols = Vec::with_capacity(nsym);
        for _ in 0..nsym {
            let name = r.string()?;
            let tag = r.byte()?;
            if tag & !0x33 != 0 {
                return r.fail("a symbol has unknown flags");
            }
            let class = match tag & 0x0F {
                0 => Class::Label,
                1 => Class::Const,
                2 => Class::Import,
                _ => return r.fail("a symbol has an unknown class"),
            };
            let section = if class == Class::Label {
                let s = r.small("section index", MAX_SECTIONS as u64)?;
                if s as usize >= sections.len() {
                    return r.fail(&format!(
                        "symbol \"{name}\" is in section {s}, which does not exist"
                    ));
                }
                s
            } else {
                0
            };
            let value = if class == Class::Import { 0 } else { r.int()? };
            if class == Class::Label {
                let size = sections[section as usize].size as i64;
                if value < 0 || value > size {
                    return r.fail(&format!(
                        "label \"{name}\" is {value} bytes into a section of {size}"
                    ));
                }
            }
            symbols.push(Symbol {
                name,
                class,
                global: tag & 0x10 != 0,
                zeropage: tag & 0x20 != 0,
                section,
                value,
            });
        }
        let nrel = r.count("relocations", MAX_RELOCS, 5)?;
        let mut relocs = Vec::with_capacity(nrel);
        for _ in 0..nrel {
            let section = r.small("section index", MAX_SECTIONS as u64)?;
            if section as usize >= sections.len() {
                return r.fail(&format!(
                    "a relocation is in section {section}, which does not exist"
                ));
            }
            let offset = r.small("offset", u32::MAX as u64)?;
            let tag = r.byte()?;
            let width = tag & 0x0F;
            if !(1..=4).contains(&width) || tag & !0x3F != 0 {
                return r.fail("a relocation has a bad width or flags");
            }
            let kind = if tag & 0x10 != 0 {
                Kind::Rel
            } else {
                Kind::Abs
            };
            let (bias, instr_back) = if kind == Kind::Rel {
                (r.int()?, r.small("instruction offset", 64)?)
            } else {
                (0, 0)
            };
            let sec = &sections[section as usize];
            if sec.bss || offset as u64 + width as u64 > sec.size as u64 {
                return r.fail(&format!(
                    "a relocation of {width} bytes at {offset} is outside section \"{}\"",
                    sec.name
                ));
            }
            let expr = r.string()?;
            let file = r.small("file index", u32::MAX as u64)?;
            let line = r.small("line", u32::MAX as u64)?;
            relocs.push(Reloc {
                section,
                offset,
                width,
                big_endian: tag & 0x20 != 0,
                kind,
                bias,
                instr_back,
                expr,
                file,
                line,
            });
        }
        if r.left() != 0 {
            return r.fail(&format!("{} bytes follow the object", r.left()));
        }
        Ok(Object {
            cpu,
            name,
            files,
            sections,
            symbols,
            relocs,
        })
    }

    /// The names this object offers other objects.
    pub fn exports(&self) -> impl Iterator<Item = &str> {
        self.symbols
            .iter()
            .filter(|s| s.global && s.class != Class::Import)
            .map(|s| s.name.as_str())
    }

    /// The names this object needs from others.
    pub fn imports(&self) -> impl Iterator<Item = &str> {
        self.symbols
            .iter()
            .filter(|s| s.class == Class::Import)
            .map(|s| s.name.as_str())
    }
}

// ---------------------------------------------------------------------------------------------
// archives

/// A library: objects by name, and an index from each exported name to the member that defines it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Archive {
    members: Vec<(String, Vec<u8>)>,
}

impl Archive {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an object, replacing a member of the same name.
    pub fn add(&mut self, name: &str, object: &Object) {
        self.add_bytes(name, object.encode());
    }

    pub fn add_bytes(&mut self, name: &str, bytes: Vec<u8>) {
        match self.members.iter_mut().find(|(n, _)| n == name) {
            Some(m) => m.1 = bytes,
            None => self.members.push((name.to_string(), bytes)),
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.members.len();
        self.members.retain(|(n, _)| n != name);
        self.members.len() != before
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.members.iter().map(|(n, _)| n.as_str())
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub fn member_bytes(&self, i: usize) -> Option<&[u8]> {
        self.members.get(i).map(|(_, b)| b.as_slice())
    }

    pub fn member(&self, i: usize) -> Result<Object, String> {
        let (name, bytes) = self
            .members
            .get(i)
            .ok_or_else(|| format!("no member {i}"))?;
        Object::decode(bytes).map_err(|e| format!("member {name}: {e}"))
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.members.iter().position(|(n, _)| n == name)
    }

    /// The index: exported name -> member number (the first member that defines it).
    pub fn index(&self) -> Result<BTreeMap<String, usize>, String> {
        let mut index = BTreeMap::new();
        for i in 0..self.members.len() {
            let o = self.member(i)?;
            for name in o.exports() {
                index.entry(name.to_string()).or_insert(i);
            }
        }
        Ok(index)
    }

    /// The member that defines `name`.
    pub fn find_export(&self, name: &str) -> Result<Option<usize>, String> {
        Ok(self.index()?.get(name).copied())
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let index = self.index()?;
        let mut o = Vec::new();
        o.extend_from_slice(ARCHIVE_MAGIC);
        put_u(&mut o, self.members.len() as u64);
        for (name, bytes) in &self.members {
            put_s(&mut o, name);
            put_u(&mut o, bytes.len() as u64);
            o.extend_from_slice(bytes);
        }
        put_u(&mut o, index.len() as u64);
        for (name, member) in &index {
            put_s(&mut o, name);
            put_u(&mut o, *member as u64);
        }
        Ok(o)
    }

    pub fn decode(data: &[u8]) -> Result<Archive, String> {
        if data.len() > MAX_FILE {
            return Err(format!("archive larger than {MAX_FILE} bytes"));
        }
        if data.len() < 4 || &data[..4] != ARCHIVE_MAGIC {
            return Err("not an rcar archive (the file does not start with RCA1)".to_string());
        }
        let mut r = Reader { data, at: 4 };
        let n = r.count("members", 1 << 20, 2)?;
        let mut members = Vec::with_capacity(n);
        for _ in 0..n {
            let name = r.string()?;
            let len = r.uint()?;
            if len > r.left() as u64 {
                return r.fail(&format!("member \"{name}\" claims {len} bytes"));
            }
            let bytes = r.bytes(len as usize)?.to_vec();
            members.push((name, bytes));
        }
        let archive = Archive { members };
        let stored_n = r.count("index entries", MAX_SYMBOLS, 2)?;
        let mut stored = BTreeMap::new();
        for _ in 0..stored_n {
            let name = r.string()?;
            let member = r.uint()?;
            stored.insert(name, member as usize);
        }
        if r.left() != 0 {
            return r.fail(&format!("{} bytes follow the archive", r.left()));
        }
        let actual = archive.index()?;
        if stored != actual {
            return Err("the archive's index does not match its members".to_string());
        }
        Ok(archive)
    }
}
