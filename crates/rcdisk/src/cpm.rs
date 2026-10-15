//! CP/M disks: the CPCEMU `.dsk` of the Amstrad CPC (AMSDOS) and the ZX Spectrum +3 (+3DOS).
//!
//! The file system is `dsk::cpm`. This layer adds what CP/M lacks: files carry an AMSDOS or +3DOS
//! header with their exact length and addresses, which listings show and extraction strips (unless
//! asked not to), and which `add --header` writes.

use crate::{AddOptions, Check, Disk, Exported, Extracted, FileRow, ReadOptions};
use dsk::cpm::header::{
    self, AMSDOS_BASIC, AMSDOS_BINARY, Amsdos, HEADER_SIZE, PLUS3_CODE, PLUS3_PROGRAM, Plus3Dos,
};
use dsk::cpm::{Cpm, Error, Layout};
use std::collections::HashSet;

pub struct CpmDisk {
    cpm: Cpm,
    layout: &'static str,
}

fn layout_name(g: &dsk::cpm::Geometry) -> &'static str {
    match (g.first_sector, g.sectors_per_track, g.reserved_tracks) {
        (0xC1, 9, 0) => "CPC data",
        (0x41, 9, 2) => "CPC system",
        (0x01, 8, 1) => "CPC IBM",
        (0x01, 9, 1) => "Spectrum +3",
        _ => "custom",
    }
}

fn err(e: Error) -> String {
    e.to_string()
}

/// `3:NAME.EXT` -> `(3, "NAME.EXT")`.
pub fn split_user(name: &str) -> (u8, &str) {
    match name.split_once(':') {
        Some((u, rest)) => match u.parse::<u8>() {
            Ok(n) if n <= 15 => (n, rest),
            _ => (0, name),
        },
        None => (0, name),
    }
}

/// A name CP/M can hold: upper case, `_` for anything else, 8.3, made unique among `taken`.
pub fn sanitize_name(name: &str, taken: &mut HashSet<String>) -> String {
    let (user, bare) = split_user(name);
    let clean = |s: &str, max: usize| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "$#@!%&'()-_^~".contains(c) {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .take(max)
            .collect()
    };
    let (stem, ext) = bare.rsplit_once('.').unwrap_or((bare, ""));
    let stem = if stem.is_empty() {
        "FILE".to_string()
    } else {
        clean(stem, 8)
    };
    let ext = clean(ext, 3);
    let mut candidate = if ext.is_empty() {
        stem.clone()
    } else {
        format!("{stem}.{ext}")
    };
    let mut n = 1;
    while !taken.insert(format!("{user}:{candidate}")) {
        let tag = format!("~{n}");
        let keep: String = stem.chars().take(8 - tag.len()).collect();
        candidate = if ext.is_empty() {
            format!("{keep}{tag}")
        } else {
            format!("{keep}{tag}.{ext}")
        };
        n += 1;
    }
    if user == 0 {
        candidate
    } else {
        format!("{user}:{candidate}")
    }
}

/// A `*` / `?` wildcard match on `NAME.EXT` (case insensitive); `*` matches any run of characters.
fn glob(pattern: &str, name: &str) -> bool {
    fn go(p: &[u8], n: &[u8]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some(b'*') => (0..=n.len()).any(|i| go(&p[1..], &n[i..])),
            Some(b'?') => !n.is_empty() && go(&p[1..], &n[1..]),
            Some(c) => n.first().is_some_and(|x| x.eq_ignore_ascii_case(c)) && go(&p[1..], &n[1..]),
        }
    }
    go(pattern.as_bytes(), name.as_bytes())
}

/// What a file's first 128 bytes say.
enum Kind {
    Amsdos(Amsdos),
    Plus3(Plus3Dos),
    Plain,
}

fn classify(data: &[u8]) -> Kind {
    if let Some(h) = header::parse_amsdos(data) {
        return Kind::Amsdos(h);
    }
    if let Some(h) = header::parse_plus3dos(data) {
        return Kind::Plus3(h);
    }
    Kind::Plain
}

/// The file's content: header stripped and cut to the length it records.
fn payload(data: &[u8]) -> (Vec<u8>, Kind) {
    match classify(data) {
        Kind::Amsdos(h) => {
            let end = (HEADER_SIZE + h.length as usize).min(data.len());
            (data[HEADER_SIZE..end].to_vec(), Kind::Amsdos(h))
        }
        Kind::Plus3(h) => {
            let end = (h.total as usize).clamp(HEADER_SIZE, data.len());
            (data[HEADER_SIZE..end].to_vec(), Kind::Plus3(h))
        }
        Kind::Plain => (data.to_vec(), Kind::Plain),
    }
}

impl CpmDisk {
    fn matching(&self, pattern: &str) -> Result<Vec<dsk::cpm::FileInfo>, String> {
        let (user, pat) = match pattern.split_once(':') {
            Some((u, p)) if u.parse::<u8>().is_ok_and(|n| n <= 15) => {
                (Some(u.parse::<u8>().unwrap()), p)
            }
            _ => (None, pattern),
        };
        let found: Vec<_> = self
            .cpm
            .files()
            .map_err(err)?
            .into_iter()
            .filter(|f| user.is_none_or(|u| f.user == u) && glob(pat, &f.name))
            .collect();
        if found.is_empty() {
            return Err(format!("no file matching \"{pattern}\""));
        }
        Ok(found)
    }

    fn shown_name(f: &dsk::cpm::FileInfo) -> String {
        if f.user == 0 {
            f.name.clone()
        } else {
            format!("{}:{}", f.user, f.name)
        }
    }
}

impl Disk for CpmDisk {
    fn format(&self) -> String {
        "dsk".to_string()
    }

    fn description(&self) -> String {
        let g = self.cpm.geometry();
        format!(
            "{} {}-track CP/M disk ({} KB, {}{})",
            self.layout,
            g.tracks,
            g.blocks() * g.block_bytes() / 1024,
            if self.cpm.disk().is_extended() {
                "extended DSK"
            } else {
                "standard DSK"
            },
            if self.cpm.disk().heads() > 1 {
                ", side 0 only"
            } else {
                ""
            }
        )
    }

    fn info(&self) -> Vec<(String, String)> {
        let g = self.cpm.geometry();
        vec![
            ("format".to_string(), "dsk".to_string()),
            ("description".to_string(), self.description()),
            ("layout".to_string(), self.layout.to_string()),
            ("tracks".to_string(), g.tracks.to_string()),
            (
                "sectors per track".to_string(),
                format!(
                    "{} x {} bytes (IDs from {:#04x})",
                    g.sectors_per_track,
                    g.sector_bytes(),
                    g.first_sector
                ),
            ),
            ("reserved tracks".to_string(), g.reserved_tracks.to_string()),
            (
                "block size".to_string(),
                format!("{} bytes", g.block_bytes()),
            ),
            (
                "directory".to_string(),
                format!("{} entries", g.dir_entries()),
            ),
            (
                "files".to_string(),
                self.cpm.files().map_or(0, |f| f.len()).to_string(),
            ),
            (
                "free".to_string(),
                format!("{} KB", self.free_blocks() * g.block_bytes() as u64 / 1024),
            ),
        ]
    }

    fn files(&self) -> Vec<FileRow> {
        let Ok(files) = self.cpm.files() else {
            return Vec::new();
        };
        files
            .into_iter()
            .map(|f| {
                let mut flags = Vec::new();
                if f.read_only {
                    flags.push("read-only".to_string());
                }
                if f.system {
                    flags.push("system".to_string());
                }
                let mut size = Some(f.bytes);
                let mut kind = "ASC".to_string();
                if let Ok(raw) = self.cpm.read(f.user, &f.name) {
                    let (data, k) = payload(&raw);
                    match k {
                        Kind::Amsdos(h) => {
                            size = Some(data.len() as u64);
                            kind = match h.kind & 0x0E {
                                0 => "BAS".to_string(),
                                2 => "BIN".to_string(),
                                _ => format!("T{}", h.kind),
                            };
                            if h.kind & 0x0E == 2 {
                                flags.push(format!("load=&{:04X}", h.load));
                                flags.push(format!("entry=&{:04X}", h.entry));
                            }
                        }
                        Kind::Plus3(h) => {
                            size = Some(data.len() as u64);
                            kind = match h.kind {
                                0 => "P3-PRG",
                                1 => "P3-NUM",
                                2 => "P3-CHR",
                                3 => "P3-COD",
                                _ => "P3-???",
                            }
                            .to_string();
                            if h.kind == 3 {
                                flags.push(format!("load={:#06X}", h.param1));
                            }
                        }
                        Kind::Plain => {}
                    }
                }
                let name = Self::shown_name(&f);
                FileRow {
                    name_hex: f.raw_name.iter().map(|b| format!("{b:02X}")).collect(),
                    name,
                    kind,
                    blocks: f.blocks as u32,
                    size,
                    flags,
                }
            })
            .collect()
    }

    fn free_blocks(&self) -> u64 {
        self.cpm.free_blocks() as u64
    }

    fn read(&self, pattern: &str, opts: &ReadOptions) -> Result<Vec<Extracted>, String> {
        let mut out = Vec::new();
        for f in self.matching(pattern)? {
            let raw = self.cpm.read(f.user, &f.name).map_err(err)?;
            let data = if opts.keep_header {
                raw
            } else {
                payload(&raw).0
            };
            let name = Self::shown_name(&f);
            out.push(Extracted {
                file_name: name.replace(':', "-"),
                name,
                data,
            });
        }
        Ok(out)
    }

    fn add(&mut self, host: &str, data: &[u8], opts: &AddOptions) -> Result<String, String> {
        let host_file = host.rsplit(['/', '\\']).next().unwrap_or(host);
        let wanted = opts
            .name
            .clone()
            .unwrap_or_else(|| host_file.to_ascii_uppercase());
        let (name_user, bare) = split_user(&wanted);
        let user = if name_user != 0 { name_user } else { opts.user };
        let bare = bare.to_ascii_uppercase();
        let raw_name = dsk::cpm::parse_name(&bare).map_err(err)?;
        let kind_text = opts.kind.as_deref().map(str::to_ascii_lowercase);
        let body: Vec<u8> = match opts
            .header
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("none") => data.to_vec(),
            Some("amsdos") => {
                let basic = matches!(kind_text.as_deref(), Some("bas") | Some("basic"));
                let load = opts
                    .load
                    .or(basic.then_some(0x170))
                    .ok_or("a binary with an AMSDOS header needs --load")?;
                let h = Amsdos {
                    user,
                    name: raw_name,
                    kind: if basic { AMSDOS_BASIC } else { AMSDOS_BINARY },
                    load,
                    entry: opts.entry.unwrap_or(if basic { 0 } else { load }),
                    length: u32::try_from(data.len())
                        .ok()
                        .filter(|l| *l < 1 << 24)
                        .ok_or("file too long for an AMSDOS header")?,
                };
                let mut out = header::amsdos(&h).to_vec();
                out.extend_from_slice(data);
                out
            }
            Some("plus3dos") | Some("plus3") => {
                let basic = matches!(kind_text.as_deref(), Some("bas") | Some("basic"));
                let load = opts.load.unwrap_or(0);
                if !basic && opts.load.is_none() {
                    return Err("a code file with a +3DOS header needs --load".to_string());
                }
                let h = Plus3Dos {
                    kind: if basic { PLUS3_PROGRAM } else { PLUS3_CODE },
                    length: data.len().min(0xFFFF) as u16,
                    param1: if basic { 0x8000 } else { load },
                    param2: if basic {
                        data.len().min(0xFFFF) as u16
                    } else {
                        0x8000
                    },
                    total: (data.len() + HEADER_SIZE) as u32,
                };
                let mut out = header::plus3dos(&h).to_vec();
                out.extend_from_slice(data);
                out
            }
            Some(other) => {
                return Err(format!(
                    "unknown header \"{other}\" (none, amsdos or plus3dos)"
                ));
            }
        };
        self.cpm
            .write(user, &bare, &body, opts.replace)
            .map_err(err)?;
        Ok(if user == 0 {
            bare
        } else {
            format!("{user}:{bare}")
        })
    }

    fn remove(&mut self, pattern: &str) -> Result<Vec<String>, String> {
        let mut removed = Vec::new();
        for f in self.matching(pattern)? {
            self.cpm.delete(f.user, &f.name).map_err(err)?;
            removed.push(Self::shown_name(&f));
        }
        Ok(removed)
    }

    fn rename(&mut self, old: &str, new: &str) -> Result<(), String> {
        let found = self.matching(old)?;
        if found.len() != 1 {
            return Err(format!("\"{old}\" matches {} files; name one", found.len()));
        }
        let f = &found[0];
        self.cpm
            .rename(f.user, &f.name, &new.to_ascii_uppercase())
            .map_err(err)
    }

    fn check(&self) -> Check {
        let mut check = Check {
            files: self.cpm.files().map_or(0, |f| f.len()),
            free_blocks: self.free_blocks(),
            ..Check::default()
        };
        for p in self.cpm.check() {
            if p.error {
                check.errors.push(p.message)
            } else {
                check.warnings.push(p.message)
            }
        }
        // A header that promises more than the file holds.
        for f in self.cpm.files().unwrap_or_default() {
            if let Ok(raw) = self.cpm.read(f.user, &f.name) {
                match classify(&raw) {
                    Kind::Amsdos(h)
                        if h.length as usize > raw.len().saturating_sub(HEADER_SIZE) =>
                    {
                        check.warnings.push(format!(
                            "{}: the AMSDOS header says {} bytes, the file holds {}",
                            Self::shown_name(&f),
                            h.length,
                            raw.len().saturating_sub(HEADER_SIZE)
                        ));
                    }
                    Kind::Plus3(h) if h.total as usize > raw.len() => {
                        check.warnings.push(format!(
                            "{}: the +3DOS header says {} bytes, the file holds {}",
                            Self::shown_name(&f),
                            h.total,
                            raw.len()
                        ));
                    }
                    _ => {}
                }
            }
        }
        check
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.cpm.disk().to_bytes()
    }

    fn label(&self) -> (String, String) {
        (String::new(), String::new())
    }

    fn export(&self) -> (Vec<Exported>, Vec<String>) {
        let mut files = Vec::new();
        let mut skipped = Vec::new();
        for f in self.cpm.files().unwrap_or_default() {
            match self.cpm.read(f.user, &f.name) {
                Ok(raw) => files.push(Exported {
                    name: Self::shown_name(&f),
                    kind: "seq".to_string(),
                    data: payload(&raw).0,
                }),
                Err(e) => skipped.push(format!("\"{}\": {e}", f.name)),
            }
        }
        (files, skipped)
    }
}

pub fn open(bytes: &[u8]) -> Result<Box<dyn Disk>, String> {
    let disk = dsk::parse(bytes).map_err(|e| e.to_string())?;
    let cpm = Cpm::open(disk).map_err(err)?;
    let layout = layout_name(cpm.geometry());
    Ok(Box::new(CpmDisk { cpm, layout }))
}

pub fn create(format: &str) -> Result<Box<dyn Disk>, String> {
    let layout = match format {
        "dsk" | "dsk-data" => Layout::CpcData,
        "dsk-system" => Layout::CpcSystem,
        "dsk-ibm" => Layout::CpcIbm,
        "dsk-plus3" => Layout::Plus3,
        other => return Err(format!("cannot create a \"{other}\" image")),
    };
    let cpm = Cpm::format(layout);
    let name = layout_name(cpm.geometry());
    Ok(Box::new(CpmDisk { cpm, layout: name }))
}
