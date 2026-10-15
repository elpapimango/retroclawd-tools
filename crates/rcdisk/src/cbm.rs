//! Commodore disks: `.d64` (35 or 40 tracks), `.x64`, `.d71`, `.d81`, and `.g64` for reading.

use crate::{AddOptions, Check, Disk, Exported, Extracted, FileRow, ReadOptions};
use cbm_dos::{FileKind, Image};

/// What the three sector-image crates have in common beyond [`Image`].
pub trait Container: Image + Clone {
    fn to_image_bytes(&self) -> Vec<u8>;
    fn disk_name(&self) -> [u8; 16];
    fn disk_id(&self) -> [u8; 2];
    fn dos_type(&self) -> [u8; 2];
    fn track_count(&self) -> u8;
    fn has_error_info(&self) -> bool;
}

macro_rules! container {
    ($t:ty, $tracks:expr) => {
        impl Container for $t {
            fn to_image_bytes(&self) -> Vec<u8> {
                self.to_bytes()
            }
            fn disk_name(&self) -> [u8; 16] {
                <$t>::disk_name(self)
            }
            fn disk_id(&self) -> [u8; 2] {
                <$t>::disk_id(self)
            }
            fn dos_type(&self) -> [u8; 2] {
                <$t>::dos_type(self)
            }
            fn track_count(&self) -> u8 {
                let f: fn(&$t) -> u8 = $tracks;
                f(self)
            }
            fn has_error_info(&self) -> bool {
                <$t>::has_error_info(self)
            }
        }
    };
}

container!(d64::D64, |d| d.variant().track_count());
container!(d71::D71, |_| 70);
container!(d81::D81, |_| 80);

pub struct Cbm<I: Container> {
    image: I,
    format: &'static str,
    /// The image came from a `.g64` (cannot be written back) or must be written as `.x64`.
    read_only: Option<String>,
    x64: bool,
    bad_sectors: usize,
}

fn ascii(raw: &[u8]) -> String {
    let end = raw.iter().position(|&b| b == 0xA0).unwrap_or(raw.len());
    cbm_dos::to_ascii(&raw[..end])
}

fn hex(raw: &[u8]) -> String {
    raw.iter().map(|b| format!("{b:02X}")).collect()
}

/// A host-safe file name.
fn host_name(name: &str, kind: Option<FileKind>) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if "/\\:*?\"<>|".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    out = out.trim().to_string();
    if out.is_empty() {
        out.push('_');
    }
    let ext = match kind {
        Some(FileKind::Prg) => "prg",
        Some(FileKind::Seq) => "seq",
        Some(FileKind::Usr) => "usr",
        Some(FileKind::Rel) => "rel",
        _ => "bin",
    };
    format!("{out}.{ext}")
}

fn kind_from_name(host: &str) -> FileKind {
    match host
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("seq") | Some("txt") => FileKind::Seq,
        Some("usr") => FileKind::Usr,
        _ => FileKind::Prg,
    }
}

impl<I: Container> Cbm<I> {
    fn new(image: I, format: &'static str) -> Self {
        Self {
            image,
            format,
            read_only: None,
            x64: format == "x64",
            bad_sectors: 0,
        }
    }

    fn matching(&self, pattern: &str) -> Result<Vec<cbm_dos::Entry>, String> {
        let pat = cbm_dos::from_ascii(pattern)
            .or_else(|_| {
                if pattern == "*" {
                    Ok(b"*".to_vec())
                } else {
                    Err(cbm_dos::FsError::InvalidName)
                }
            })
            .map_err(|e| format!("\"{pattern}\": {e}"))?;
        let found: Vec<_> = cbm_dos::directory(&self.image)
            .into_iter()
            .filter(|e| cbm_dos::matches_pattern(&pat, e.name_bytes()))
            .collect();
        if found.is_empty() {
            return Err(format!("no file matching \"{pattern}\""));
        }
        Ok(found)
    }

    fn guard(&self) -> Result<(), String> {
        match &self.read_only {
            Some(why) => Err(why.clone()),
            None => Ok(()),
        }
    }
}

impl<I: Container> Disk for Cbm<I> {
    fn format(&self) -> String {
        self.format.to_string()
    }

    fn description(&self) -> String {
        let model = match self.format {
            "d71" => "1571",
            "d81" => "1581",
            _ => "1541",
        };
        format!(
            "{}-track {model} disk image{}",
            self.image.track_count(),
            if self.image.has_error_info() {
                " with error info"
            } else {
                ""
            }
        )
    }

    fn read_only(&self) -> Option<String> {
        self.read_only.clone()
    }

    fn info(&self) -> Vec<(String, String)> {
        let id = self.image.disk_id();
        let dos = self.image.dos_type();
        let mut rows = vec![
            ("format".to_string(), self.format.to_string()),
            ("description".to_string(), self.description()),
            ("name".to_string(), ascii(&self.image.disk_name())),
            ("id".to_string(), cbm_dos::to_ascii(&id)),
            ("dos type".to_string(), cbm_dos::to_ascii(&dos)),
            (
                "files".to_string(),
                cbm_dos::directory(&self.image).len().to_string(),
            ),
            ("blocks free".to_string(), self.free_blocks().to_string()),
            (
                "size".to_string(),
                format!("{} bytes", self.image.to_image_bytes().len()),
            ),
        ];
        if self.bad_sectors > 0 {
            rows.push((
                "unreadable sectors".to_string(),
                format!("{} (left zeroed by the .g64 decode)", self.bad_sectors),
            ));
        }
        rows
    }

    fn files(&self) -> Vec<FileRow> {
        cbm_dos::directory(&self.image)
            .into_iter()
            .map(|e| {
                let mut flags = Vec::new();
                if e.is_locked() {
                    flags.push("locked".to_string());
                }
                if !e.is_closed() {
                    flags.push("unclosed".to_string());
                }
                FileRow {
                    name: ascii(e.name_bytes()),
                    name_hex: hex(e.name_bytes()),
                    kind: e.kind().map_or("???".to_string(), |k| k.name().to_string()),
                    blocks: e.blocks as u32,
                    size: None,
                    flags,
                }
            })
            .collect()
    }

    fn free_blocks(&self) -> u64 {
        cbm_dos::free_blocks(&self.image) as u64
    }

    fn read(&self, pattern: &str, _opts: &ReadOptions) -> Result<Vec<Extracted>, String> {
        let mut out = Vec::new();
        for entry in self.matching(pattern)? {
            let name = ascii(entry.name_bytes());
            let data =
                cbm_dos::read_file(&self.image, &entry).map_err(|e| format!("\"{name}\": {e}"))?;
            out.push(Extracted {
                file_name: host_name(&name, entry.kind()),
                name,
                data,
            });
        }
        Ok(out)
    }

    fn add(&mut self, host: &str, data: &[u8], opts: &AddOptions) -> Result<String, String> {
        self.guard()?;
        let stem = host
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(host)
            .rsplit_once('.')
            .map_or(host, |(s, _)| s);
        let stem = if stem.is_empty() { host } else { stem };
        let wanted = opts.name.as_deref().unwrap_or(stem);
        let raw = cbm_dos::from_ascii(wanted).map_err(|_| {
            format!(
                "\"{wanted}\" is not a usable CBM file name (1 to 16 characters, ASCII); use --name"
            )
        })?;
        let kind = match &opts.kind {
            Some(k) => match FileKind::parse(k) {
                Some(kind @ (FileKind::Prg | FileKind::Seq | FileKind::Usr)) => kind,
                _ => {
                    return Err(format!(
                        "cannot store a file of type \"{k}\" (use prg, seq or usr)"
                    ));
                }
            },
            None => kind_from_name(host),
        };
        cbm_dos::save_file(&mut self.image, &raw, kind, data, opts.replace)
            .map_err(|e| format!("\"{wanted}\": {e}"))?;
        Ok(ascii(&raw))
    }

    fn remove(&mut self, pattern: &str) -> Result<Vec<String>, String> {
        self.guard()?;
        let mut removed = Vec::new();
        for entry in self.matching(pattern)? {
            let name = ascii(entry.name_bytes());
            cbm_dos::scratch(&mut self.image, entry.name_bytes())
                .map_err(|e| format!("\"{name}\": {e}"))?;
            removed.push(name);
        }
        Ok(removed)
    }

    fn rename(&mut self, old: &str, new: &str) -> Result<(), String> {
        self.guard()?;
        let found = self.matching(old)?;
        if found.len() != 1 {
            return Err(format!("\"{old}\" matches {} files; name one", found.len()));
        }
        let new_raw = cbm_dos::from_ascii(new).map_err(|e| format!("\"{new}\": {e}"))?;
        cbm_dos::rename(&mut self.image, found[0].name_bytes(), &new_raw)
            .map_err(|e| format!("\"{old}\": {e}"))
    }

    fn check(&self) -> Check {
        let v = cbm_dos::validate(&self.image);
        let mut out = Check {
            files: v.files,
            free_blocks: v.blocks_free as u64,
            ..Check::default()
        };
        for p in v.problems {
            match p.severity {
                cbm_dos::Severity::Error => out.errors.push(p.message),
                cbm_dos::Severity::Warning => out.warnings.push(p.message),
            }
        }
        if self.bad_sectors > 0 {
            out.warnings.push(format!(
                "{} sector(s) could not be decoded from the .g64",
                self.bad_sectors
            ));
        }
        out
    }

    fn to_bytes(&self) -> Vec<u8> {
        let image = self.image.to_image_bytes();
        if self.x64 {
            let mut out = x64_header(self.image.track_count());
            out.extend(image);
            out
        } else {
            image
        }
    }

    fn label(&self) -> (String, String) {
        (
            ascii(&self.image.disk_name()),
            cbm_dos::to_ascii(&self.image.disk_id()),
        )
    }

    fn export(&self) -> (Vec<Exported>, Vec<String>) {
        let mut files = Vec::new();
        let mut skipped = Vec::new();
        for entry in cbm_dos::directory(&self.image) {
            let name = ascii(entry.name_bytes());
            let kind = match entry.kind() {
                Some(k @ (FileKind::Prg | FileKind::Seq | FileKind::Usr)) => k,
                other => {
                    skipped.push(format!(
                        "\"{name}\": {} files cannot be copied",
                        other.map_or("unknown type", FileKind::name)
                    ));
                    continue;
                }
            };
            match cbm_dos::read_file(&self.image, &entry) {
                Ok(data) => files.push(Exported {
                    name,
                    kind: kind.name().to_ascii_lowercase(),
                    data,
                }),
                Err(e) => skipped.push(format!("\"{name}\": {e}")),
            }
        }
        (files, skipped)
    }
}

/// Opens a Commodore image by format name (see [`crate::sniff`]).
pub fn open(format: &str, bytes: &[u8]) -> Result<Box<dyn Disk>, String> {
    match format {
        "d64" => Ok(Box::new(Cbm::new(
            d64::D64::parse(bytes).map_err(|e| e.to_string())?,
            "d64",
        ))),
        "x64" => Ok(Box::new(Cbm::new(
            d64::D64::parse_x64(bytes).map_err(|e| e.to_string())?,
            "x64",
        ))),
        "d71" => Ok(Box::new(Cbm::new(
            d71::D71::parse(bytes).map_err(|e| e.to_string())?,
            "d71",
        ))),
        "d81" => Ok(Box::new(Cbm::new(
            d81::D81::parse(bytes).map_err(|e| e.to_string())?,
            "d81",
        ))),
        "g64" => {
            let g = g64::parse(bytes).map_err(|e| e.to_string())?;
            let decoded = g.to_d64().map_err(|e| e.to_string())?;
            let mut disk = Cbm::new(decoded.disk, "g64");
            disk.bad_sectors = decoded.bad_sectors;
            disk.read_only = Some(
                "a .g64 cannot be written back (it holds raw tracks); convert it to .d64 first"
                    .to_string(),
            );
            Ok(Box::new(disk))
        }
        other => Err(format!("unknown disk format \"{other}\"")),
    }
}

fn id_bytes(id: Option<&str>) -> Result<[u8; 2], String> {
    let id = id.unwrap_or("00");
    let raw = cbm_dos::from_ascii(id).map_err(|_| format!("disk ID \"{id}\" is not usable"))?;
    match raw.as_slice() {
        [a, b] => Ok([*a, *b]),
        _ => Err(format!("a disk ID is exactly two characters, not \"{id}\"")),
    }
}

/// A blank formatted image.
pub fn create(format: &str, name: Option<&str>, id: Option<&str>) -> Result<Box<dyn Disk>, String> {
    let id = id_bytes(id)?;
    let name = match name.filter(|n| !n.is_empty()) {
        Some(n) => cbm_dos::from_ascii(n)
            .map_err(|_| format!("disk name \"{n}\" is not usable (1 to 16 characters)"))?,
        None => Vec::new(),
    };
    Ok(match format {
        "d64" | "x64" => {
            let mut d = d64::D64::blank_formatted(d64::Variant::Track35, id);
            d.set_disk_name(&name);
            Box::new(Cbm::new(d, if format == "x64" { "x64" } else { "d64" }))
        }
        "d64-40" => {
            let mut d = d64::D64::blank_formatted(d64::Variant::Track40, id);
            d.set_disk_name(&name);
            Box::new(Cbm::new(d, "d64"))
        }
        "d71" => {
            let mut d = d71::D71::blank_formatted(id);
            d.set_disk_name(&name);
            Box::new(Cbm::new(d, "d71"))
        }
        "d81" => {
            let mut d = d81::D81::blank_formatted(id);
            d.set_disk_name(&name);
            Box::new(Cbm::new(d, "d81"))
        }
        other => return Err(format!("cannot create a \"{other}\" image")),
    })
}

fn x64_header(tracks: u8) -> Vec<u8> {
    let mut header = vec![0x43, 0x15, 0x41, 0x64, 1, 2, 0, tracks];
    header.resize(64, 0);
    header
}

/// Copies the files of `source` into a new image of `format`.
pub fn convert(source: &dyn Disk, format: &str) -> Result<(Box<dyn Disk>, Vec<String>), String> {
    let (name, id) = source.label();
    let id = if id.chars().count() == 2 {
        id
    } else {
        "00".to_string()
    };
    // The same sector layout: only the container changes, nothing is lost.
    if matches!(
        (source.format().as_str(), format),
        ("d64", "x64") | ("x64", "d64") | ("g64", "d64")
    ) {
        let bytes = source.to_bytes();
        let sectors = if source.format() == "x64" {
            &bytes[64..]
        } else {
            &bytes[..]
        };
        let copy = if format == "x64" {
            let mut b = x64_header(if sectors.len() >= 196608 { 40 } else { 35 });
            b.extend(sectors);
            open("x64", &b)?
        } else {
            open("d64", sectors)?
        };
        let note = if source.format() == "g64" {
            vec![
                "standard sectors only: copy-protection data in the .g64 is not carried over"
                    .to_string(),
            ]
        } else {
            Vec::new()
        };
        return Ok((copy, note));
    }
    if format == "g64" {
        return Err("writing .g64 images is not supported yet".to_string());
    }
    let mut target = create(format, Some(&name), Some(&id))?;
    let (files, mut skipped) = source.export();
    for f in files {
        let opts = AddOptions {
            name: Some(f.name.clone()),
            kind: Some(f.kind.clone()).filter(|k| !k.is_empty()),
            replace: false,
            ..AddOptions::default()
        };
        if let Err(e) = target.add(&f.name, &f.data, &opts) {
            skipped.push(e);
        }
    }
    Ok((target, skipped))
}
