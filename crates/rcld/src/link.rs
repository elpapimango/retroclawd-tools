//! The linker: relocatable objects and libraries in, placed pieces and symbol addresses out.
//!
//! 1. Objects are taken as given; a library member is added when it defines a name that is still undefined
//!    (repeatedly, so members that need other members work).
//! 2. Sections with the same name are placed one after another (in input order) following a [`Rule`]: a region
//!    to live in, or a start address. A section with a fixed address stays where it was assembled.
//! 3. Every symbol gets its address; every import is looked up among the exports.
//! 4. Every relocation's expression is evaluated with the final addresses and written into the section's bytes,
//!    checked for fit.
//!
//! Errors name the object, the section, the symbol and the source line, and are collected rather than stopping
//! at the first one.

use crate::place::Piece;
use rc_common::obj::{self, Archive, Class, Kind, Object};
use rcasm::expr::{self, Scope};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;

/// Where the sections called `name` go.
#[derive(Clone, Debug, Default)]
pub struct Rule {
    pub name: String,
    /// The region (from the layout) the section must lie in.
    pub region: Option<String>,
    /// Start here instead of continuing after the previous section.
    pub start: Option<u32>,
    /// Alignment of the first section of this name (in addition to each section's own).
    pub align: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct RegionSpec {
    pub name: String,
    pub start: u32,
    /// Inclusive.
    pub end: u32,
}

/// An object as the linker sees it: the file it came from and its contents.
pub struct LinkObject {
    pub file: String,
    pub object: Object,
}

/// A placed run of bytes with a 32-bit address (what [`Linked::pieces`] cannot hold above 64 KB).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WidePiece {
    pub name: String,
    pub start: u32,
    pub bytes: Vec<u8>,
}

/// A section as placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacedSection {
    pub name: String,
    pub addr: u32,
    pub size: u32,
    pub bss: bool,
}

/// How the linker is asked to work.
#[derive(Clone, Copy, Debug)]
pub struct LinkOptions {
    /// The address space: 0x10000 for the 8-bit CPUs, 0x100_0000 for the 68000.
    pub limit: u64,
    /// Find the fields that hold an address of the program (for a loader that relocates them): they go in
    /// [`Linked::address_fields`], and an address in a field that is not four bytes wide is an error.
    pub track_addresses: bool,
}

impl Default for LinkOptions {
    fn default() -> Self {
        LinkOptions { limit: 0x10000, track_addresses: false }
    }
}

#[derive(Debug, Default)]
pub struct Linked {
    /// The placed bytes with 16-bit addresses (empty when the address space is larger: see `wide`).
    pub pieces: Vec<Piece>,
    pub wide: Vec<WidePiece>,
    /// Every placed section that is not empty, in address order.
    pub sections: Vec<PlacedSection>,
    /// The addresses of the four-byte big-endian fields that hold an address of the program (`track_addresses`).
    pub address_fields: Vec<u32>,
    /// Globals and labels, by name (non-global labels as `file:name` when two objects use the same one).
    pub symbols: Vec<(String, i64)>,
    pub map: String,
    pub notes: Vec<String>,
    /// The labels in the debugger's symbol file format (`name = $ADDR`).
    pub symbols_text: String,
}

struct Placed {
    object: usize,
    section: usize,
    addr: u32,
}

/// What the symbol `#n` of one object means while a relocation is evaluated.
struct Ctx<'a> {
    values: &'a [i64],
    /// Which symbols are addresses, and how far they are moved (to see which values depend on an address).
    addr: &'a [bool],
    shift: i64,
    pc: i64,
}

impl Scope for Ctx<'_> {
    fn symbol(&self, name: &str) -> Option<i64> {
        let i: usize = name.strip_prefix('#')?.parse().ok()?;
        let v = self.values.get(i).copied()?;
        Some(if self.addr.get(i).copied().unwrap_or(false) { v + self.shift } else { v })
    }
    fn pc(&self) -> i64 {
        self.pc
    }
}

fn align_up(v: u32, a: u32) -> u32 {
    let a = a.max(1);
    v.div_ceil(a) * a
}

/// Renders a relocation's expression with the object's symbol names, for messages.
fn show(e: &obj::Expr, o: &Object) -> String {
    match e {
        obj::Expr::Num(n) => {
            if *n >= 0 && *n < 10 {
                n.to_string()
            } else {
                format!("${n:X}")
            }
        }
        obj::Expr::Sym(i) => o
            .symbols
            .get(*i as usize)
            .map_or("?".to_string(), |s| s.name.clone()),
        obj::Expr::Pc => "$".to_string(),
        obj::Expr::Unary(op, a) => format!("{}{}", *op as char, show(a, o)),
        obj::Expr::Binary(op, a, b) => {
            let sym = [
                "*", "/", "%", "+", "-", "<<", ">>", "<", "<=", ">", ">=", "==", "!=", "&", "^",
                "|", "&&", "||",
            ];
            format!(
                "{}{}{}",
                show(a, o),
                sym.get(*op as usize).copied().unwrap_or("?"),
                show(b, o)
            )
        }
    }
}

fn fits(value: i64, width: u8, signed_only: bool) -> bool {
    let bits = width as u32 * 8;
    if signed_only {
        let half = 1i64 << (bits - 1);
        (-half..half).contains(&value)
    } else {
        (-(1i64 << (bits - 1))..(1i64 << bits)).contains(&value)
    }
}

pub fn link(
    objects: Vec<LinkObject>,
    libs: &[(String, Archive)],
    rules: &[Rule],
    regions: &[RegionSpec],
) -> Result<Linked, Vec<String>> {
    link_with(objects, libs, rules, regions, LinkOptions::default())
}

pub fn link_with(
    objects: Vec<LinkObject>,
    libs: &[(String, Archive)],
    rules: &[Rule],
    regions: &[RegionSpec],
    options: LinkOptions,
) -> Result<Linked, Vec<String>> {
    let wide = options.limit > 0x10000;
    let digits = if wide { 6 } else { 4 };
    let mut errors: Vec<String> = Vec::new();
    let mut objects = objects;

    // ---- pull in library members ------------------------------------------------------------
    let mut defined: HashMap<String, usize> = HashMap::new();
    let mut wanted: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut taken: HashSet<(usize, usize)> = HashSet::new();
    let mut scanned = 0;
    loop {
        // register what newly added objects define and want
        while scanned < objects.len() {
            let o = &objects[scanned].object;
            for name in o.exports() {
                match defined.get(name) {
                    Some(&first) => errors.push(format!(
                        "\"{name}\" is defined in both {} and {}",
                        objects[first].file, objects[scanned].file
                    )),
                    None => {
                        defined.insert(name.to_string(), scanned);
                    }
                }
            }
            for name in o.imports() {
                wanted.entry(name.to_string()).or_default().push(scanned);
            }
            scanned += 1;
        }
        let missing: Vec<String> = wanted
            .keys()
            .filter(|n| !defined.contains_key(*n))
            .cloned()
            .collect();
        let mut added = false;
        for name in missing {
            for (li, (lname, lib)) in libs.iter().enumerate() {
                let found = match lib.find_export(&name) {
                    Ok(f) => f,
                    Err(e) => {
                        errors.push(format!("{lname}: {e}"));
                        None
                    }
                };
                if let Some(m) = found {
                    if taken.insert((li, m)) {
                        match lib.member(m) {
                            Ok(object) => {
                                let member = lib.names().nth(m).unwrap_or("?");
                                objects.push(LinkObject {
                                    file: format!("{lname}({member})"),
                                    object,
                                });
                                added = true;
                            }
                            Err(e) => errors.push(format!("{lname}: {e}")),
                        }
                    }
                    break;
                }
            }
        }
        if !added && scanned == objects.len() {
            break;
        }
    }
    if let Some(first) = objects.first() {
        for o in &objects {
            if o.object.cpu != first.object.cpu {
                errors.push(format!(
                    "{} is for the {}, {} for the {}",
                    first.file, first.object.cpu, o.file, o.object.cpu
                ));
                break;
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    // ---- placement --------------------------------------------------------------------------
    // section names in order of first appearance
    let mut names: Vec<String> = Vec::new();
    for o in &objects {
        for s in &o.object.sections {
            if !names.contains(&s.name) {
                names.push(s.name.clone());
            }
        }
    }
    let mut placed: Vec<Placed> = Vec::new();
    let mut region_cursor: HashMap<&str, u32> =
        regions.iter().map(|r| (r.name.as_str(), r.start)).collect();
    let mut cursor: Option<u32> = None;
    let mut map = String::new();
    for name in &names {
        let rule = rules.iter().find(|r| r.name == *name);
        let sizes: Vec<(usize, usize)> = objects
            .iter()
            .enumerate()
            .flat_map(|(oi, o)| {
                o.object
                    .sections
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.name == *name)
                    .map(move |(si, _)| (oi, si))
            })
            .collect();
        // fixed sections
        let (fixed, floating): (Vec<_>, Vec<_>) = sizes
            .into_iter()
            .partition(|(oi, si)| objects[*oi].object.sections[*si].addr.is_some());
        for (oi, si) in fixed {
            let s = &objects[oi].object.sections[si];
            placed.push(Placed {
                object: oi,
                section: si,
                addr: s.addr.unwrap_or(0),
            });
        }
        if floating
            .iter()
            .all(|(oi, si)| objects[*oi].object.sections[*si].size == 0)
        {
            for (oi, si) in floating {
                placed.push(Placed {
                    object: oi,
                    section: si,
                    addr: 0,
                });
            }
            continue;
        }
        // where do they start
        let region = match rule.and_then(|r| r.region.as_deref()) {
            Some(rn) => match regions.iter().find(|r| r.name == rn) {
                Some(r) => Some(r),
                None => {
                    errors.push(format!(
                        "segment \"{name}\" is to go in region \"{rn}\", which the layout does not define"
                    ));
                    continue;
                }
            },
            None => None,
        };
        let start_at = match (rule.and_then(|r| r.start), region) {
            (Some(s), _) => s,
            (None, Some(r)) => region_cursor
                .get(r.name.as_str())
                .copied()
                .unwrap_or(r.start),
            (None, None) => match cursor {
                Some(c) => c,
                None => {
                    // the first region that has room, else an error
                    let first_floating = &objects[floating[0].0].object.sections[floating[0].1];
                    if first_floating.zeropage {
                        errors.push(format!(
                            "zero-page segment \"{name}\" needs a rule: [[segment]] name = \"{name}\" start = ..."
                        ));
                        continue;
                    }
                    match regions.first() {
                        Some(r) => region_cursor
                            .get(r.name.as_str())
                            .copied()
                            .unwrap_or(r.start),
                        None => {
                            errors.push(format!(
                                "segment \"{name}\" has no address: give it a rule with start = ... or a region (the layout has none)"
                            ));
                            continue;
                        }
                    }
                }
            },
        };
        let mut at = align_up(start_at, rule.and_then(|r| r.align).unwrap_or(1));
        let seg_start = at;
        for (oi, si) in floating {
            let s = &objects[oi].object.sections[si];
            at = align_up(at, s.align);
            placed.push(Placed {
                object: oi,
                section: si,
                addr: at,
            });
            at += s.size;
        }
        // the region it should fit in: the named one, or any region containing the start
        let limit = region.or_else(|| {
            regions
                .iter()
                .find(|r| (r.start..=r.end).contains(&seg_start))
        });
        if let Some(r) = limit {
            if seg_start < r.start || at > r.end + 1 {
                errors.push(format!(
                    "segment \"{name}\" (${seg_start:04X}-${:04X}, {} bytes) does not fit in region \"{}\" (${:04X}-${:04X})",
                    at.max(1) - 1,
                    at - seg_start,
                    r.name,
                    r.start,
                    r.end
                ));
            }
            region_cursor.insert(r.name.as_str(), at);
        }
        cursor = Some(at);
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut base: Vec<Vec<Option<u32>>> = objects
        .iter()
        .map(|o| vec![None; o.object.sections.len()])
        .collect();
    for p in &placed {
        base[p.object][p.section] = Some(p.addr);
        let o = &objects[p.object];
        let s = &o.object.sections[p.section];
        if s.size > 0 {
            let _ = writeln!(
                map,
                "{:<10} ${:0digits$X}-${:0digits$X} {:>6} bytes  {}{}",
                s.name,
                p.addr,
                p.addr + s.size - 1,
                s.size,
                o.file,
                if s.bss { "  (bss)" } else { "" }
            );
        }
        if p.addr as u64 + s.size as u64 > options.limit {
            errors.push(format!(
                "segment \"{}\" of {} runs past the end of memory (${:0digits$X} + {} bytes)",
                s.name, o.file, p.addr, s.size
            ));
        }
    }

    // ---- symbol values ----------------------------------------------------------------------
    let mut exports: HashMap<&str, (i64, bool)> = HashMap::new();
    let mut values: Vec<Vec<i64>> = Vec::new();
    let mut is_addr: Vec<Vec<bool>> = Vec::new();
    for (oi, o) in objects.iter().enumerate() {
        let mut v = Vec::with_capacity(o.object.symbols.len());
        let mut a = Vec::with_capacity(o.object.symbols.len());
        for s in &o.object.symbols {
            let value = match s.class {
                Class::Label => base[oi][s.section as usize].map_or(0, |b| b as i64) + s.value,
                Class::Const => s.value,
                Class::Import => 0,
            };
            if s.global && s.class != Class::Import {
                exports.insert(s.name.as_str(), (value, s.class == Class::Label));
            }
            v.push(value);
            a.push(s.class == Class::Label);
        }
        values.push(v);
        is_addr.push(a);
    }
    for (oi, o) in objects.iter().enumerate() {
        for (si, s) in o.object.symbols.iter().enumerate() {
            if s.class == Class::Import {
                match exports.get(s.name.as_str()) {
                    Some((v, addr)) => {
                        values[oi][si] = *v;
                        is_addr[oi][si] = *addr;
                    }
                    None => {
                        let who: Vec<&str> = objects
                            .iter()
                            .filter(|x| x.object.imports().any(|n| n == s.name))
                            .map(|x| x.file.as_str())
                            .collect();
                        let msg = format!(
                            "undefined symbol \"{}\" (wanted by {})",
                            s.name,
                            who.join(", ")
                        );
                        if !errors.contains(&msg) {
                            errors.push(msg);
                        }
                    }
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    // ---- relocations ------------------------------------------------------------------------
    let mut images: Vec<Vec<Vec<u8>>> = objects
        .iter()
        .map(|o| o.object.sections.iter().map(|s| s.bytes.clone()).collect())
        .collect();
    let mut address_fields: Vec<u32> = Vec::new();
    for (oi, o) in objects.iter().enumerate() {
        for r in &o.object.relocs {
            let section = &o.object.sections[r.section as usize];
            let sec_base = base[oi][r.section as usize].unwrap_or(0) as i64;
            let instr = sec_base + r.offset as i64 - r.instr_back as i64;
            let at = format!(
                "{}:{}",
                o.object
                    .files
                    .get(r.file as usize)
                    .map_or(o.file.as_str(), String::as_str),
                r.line
            );
            let parsed = match expr::from_object(&r.expr) {
                Ok(e) => e,
                Err(e) => {
                    errors.push(format!("{at}: {e}"));
                    continue;
                }
            };
            let ctx = Ctx {
                values: &values[oi],
                addr: &is_addr[oi],
                shift: 0,
                pc: instr,
            };
            let value = match parsed.eval(&ctx) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("{at}: {e}"));
                    continue;
                }
            };
            if options.track_addresses && r.kind == Kind::Abs {
                const MOVE: i64 = 1 << 30;
                let moved = Ctx {
                    values: &values[oi],
                    addr: &is_addr[oi],
                    shift: MOVE,
                    pc: instr + MOVE,
                };
                match parsed.eval(&moved).map(|v| v - value) {
                    Ok(0) => {}
                    Ok(MOVE) if r.width == 4 && r.big_endian => {
                        address_fields.push((sec_base + r.offset as i64) as u32);
                    }
                    Ok(MOVE) => errors.push(format!(
                        "{at}: {} holds an address of the program, which the loader can only relocate in a four-byte field (segment \"{}\" of {})",
                        show(&r.expr, &o.object),
                        section.name,
                        o.file
                    )),
                    _ => errors.push(format!(
                        "{at}: {} is neither an address nor a constant: it cannot be relocated (segment \"{}\" of {})",
                        show(&r.expr, &o.object),
                        section.name,
                        o.file
                    )),
                }
            }
            let (field, ok) = match r.kind {
                Kind::Abs => (value, fits(value, r.width, false)),
                Kind::Rel => {
                    let d = value - (instr + r.bias);
                    (d, fits(d, r.width, true))
                }
            };
            if !ok {
                let what = match r.kind {
                    Kind::Abs => format!("value {value} (${value:X})"),
                    Kind::Rel => format!("branch distance {field} (the target is ${value:X})"),
                };
                errors.push(format!(
                    "{at}: {} = {what} does not fit in {} byte(s) (segment \"{}\" of {})",
                    show(&r.expr, &o.object),
                    r.width,
                    section.name,
                    o.file
                ));
                continue;
            }
            let bytes = &mut images[oi][r.section as usize];
            for i in 0..r.width as usize {
                let shift = if r.big_endian {
                    (r.width as usize - 1 - i) * 8
                } else {
                    i * 8
                };
                bytes[r.offset as usize + i] = ((field >> shift) & 0xFF) as u8;
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    // ---- output -----------------------------------------------------------------------------
    let mut pieces: Vec<Piece> = Vec::new();
    let mut wide_pieces: Vec<WidePiece> = Vec::new();
    let mut sections: Vec<PlacedSection> = Vec::new();
    let mut order: Vec<&Placed> = placed.iter().collect();
    order.sort_by_key(|p| p.addr);
    for p in order {
        let s = &objects[p.object].object.sections[p.section];
        if s.size > 0 {
            sections.push(PlacedSection {
                name: s.name.clone(),
                addr: p.addr,
                size: s.size,
                bss: s.bss,
            });
        }
        if s.bss || s.bytes.is_empty() {
            continue;
        }
        let bytes = images[p.object][p.section].clone();
        let name = format!("{} {}", objects[p.object].file, s.name);
        if !wide {
            pieces.push(Piece {
                name: name.clone(),
                start: p.addr as u16,
                bytes: bytes.clone(),
            });
        }
        wide_pieces.push(WidePiece {
            name,
            start: p.addr,
            bytes,
        });
    }
    address_fields.sort_unstable();
    address_fields.dedup();
    let mut symbols: Vec<(String, i64)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (oi, o) in objects.iter().enumerate() {
        for (si, s) in o.object.symbols.iter().enumerate() {
            if s.class == Class::Const && !s.global {
                continue;
            }
            let name = if s.global || seen.insert(s.name.clone()) {
                s.name.clone()
            } else {
                format!("{}:{}", o.file, s.name)
            };
            if s.class == Class::Import {
                continue;
            }
            symbols.push((name, values[oi][si]));
        }
    }
    let mut symbols_text = String::new();
    for (oi, o) in objects.iter().enumerate() {
        for (si, s) in o.object.symbols.iter().enumerate() {
            let v = values[oi][si];
            if s.class == Class::Label && (0..options.limit as i64).contains(&v) {
                let name: String = s
                    .name
                    .chars()
                    .map(|c| if c == '@' || c == '#' { '_' } else { c })
                    .collect();
                let _ = writeln!(symbols_text, "{name} = ${v:0digits$X}");
            }
        }
    }
    Ok(Linked {
        pieces,
        wide: wide_pieces,
        sections,
        address_fields,
        symbols,
        map,
        notes: Vec::new(),
        symbols_text,
    })
}
