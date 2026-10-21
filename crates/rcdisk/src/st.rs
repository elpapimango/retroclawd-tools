//! Atari ST `.st` floppies: a raw sector dump holding a FAT12 file system.
//!
//! - FAT12: Microsoft, *FAT: General Overview of On-Disk Format* v1.03 (2000): the BPB fields, 12-bit FAT entries at
//!   byte `N + N/2` (an odd cluster's entry in the high 12 bits of the little-endian word), end of chain at `$FF8` and
//!   above, bad cluster `$FF7`, FAT[0] = the media byte with the other bits set, FAT[1] = an end-of-chain mark; 32-byte
//!   directory entries (name 0-10, attribute 11, first cluster 26, size 28; little-endian), `$E5` a free entry, `$00`
//!   the end of the directory; upper-case 8.3 names without the listed illegal bytes.
//! - The ST's layout: *Atari ST Internals* (Abacus 1985) p.178: 512-byte sectors, 2 sectors per cluster, 1 reserved
//!   sector, 2 FATs of 5 sectors, 112 root entries, 9 sectors per track; 1440 sectors and media `$F9` double sided,
//!   720 and `$F8` single sided. The executable boot sector (p.179): the 256 big-endian words sum to `$1234`, the
//!   program starts at offset 30.
//!
//! Every image is untrusted: chains are walked at most cluster-count steps, and a chain that loops, leaves the disk,
//! hits a free or bad cluster, or ends before the file's size is an error.

use crate::{AddOptions, Check, Disk, Exported, Extracted, FileRow, ReadOptions};

const SECTOR: usize = 512;
const ENTRY: usize = 32;
const EOC: u16 = 0xFFF;
const FREE_ENTRY: u8 = 0xE5;
const ATTR_VOLUME_ID: u8 = 0x08;
const ATTR_DIRECTORY: u8 = 0x10;
/// Bytes not allowed in a short name (Microsoft, `DIR_Name`).
const ILLEGAL: &[u8] = &[
    0x22, 0x2A, 0x2B, 0x2C, 0x2E, 0x2F, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F, 0x5B, 0x5C, 0x5D, 0x7C,
];
/// Where an executable boot sector's program starts (*Internals* p.179).
const BOOT_CODE: usize = 30;

/// The BPB fields this adapter uses.
#[derive(Clone, Copy, Debug)]
struct Layout {
    spc: usize,
    reserved: usize,
    fats: usize,
    root_entries: usize,
    total: usize,
    spf: usize,
    media: u8,
}

impl Layout {
    fn fat_start(&self, n: usize) -> usize {
        (self.reserved + n * self.spf) * SECTOR
    }
    fn root_start(&self) -> usize {
        (self.reserved + self.fats * self.spf) * SECTOR
    }
    fn data_sector(&self) -> usize {
        self.reserved + self.fats * self.spf + (self.root_entries * ENTRY).div_ceil(SECTOR)
    }
    fn clusters(&self) -> usize {
        (self.total.saturating_sub(self.data_sector())) / self.spc
    }
    fn cluster_bytes(&self) -> usize {
        self.spc * SECTOR
    }
}

pub struct StDisk {
    bytes: Vec<u8>,
    layout: Layout,
    format: &'static str,
}

fn le16(b: &[u8], at: usize) -> usize {
    u16::from_le_bytes([b[at], b[at + 1]]) as usize
}

fn parse_layout(b: &[u8]) -> Result<Layout, String> {
    if b.len() < SECTOR || !b.len().is_multiple_of(SECTOR) {
        return Err(format!(
            "not a .st image: {} bytes is not a whole number of 512-byte sectors",
            b.len()
        ));
    }
    let l = Layout {
        spc: b[13] as usize,
        reserved: le16(b, 14),
        fats: b[16] as usize,
        root_entries: le16(b, 17),
        total: le16(b, 19),
        spf: le16(b, 22),
        media: b[21],
    };
    let plausible = le16(b, 11) == SECTOR
        && l.spc > 0
        && l.reserved > 0
        && l.fats > 0
        && l.spf > 0
        && l.root_entries > 0
        && l.total * SECTOR == b.len()
        && l.data_sector() < l.total
        && l.clusters() < 4085
        // The FATs must hold an entry for every cluster.
        && l.spf * SECTOR * 2 >= (l.clusters() + 2) * 3;
    if !plausible {
        return Err(format!(
            "no usable FAT12 boot sector (bytes/sector {}, total sectors {}, file {} bytes)",
            le16(b, 11),
            l.total,
            b.len()
        ));
    }
    Ok(l)
}

/// `name` (host or disk spelling) as the 11 bytes of a short name, upper case.
fn short_name(name: &str) -> Result<[u8; 11], String> {
    let upper = name.to_ascii_uppercase();
    let (base, ext) = upper.rsplit_once('.').unwrap_or((&upper, ""));
    let bad = |s: &str| {
        s.bytes()
            .any(|c| !(0x21..=0x7E).contains(&c) || ILLEGAL.contains(&c))
    };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 || bad(base) || bad(ext) {
        return Err(format!(
            "\"{name}\" is not a valid 8.3 name (at most 8 + 3 characters, none of \" * + , . / : ; < = > ? [ \\ ] |)"
        ));
    }
    let mut out = [b' '; 11];
    out[..base.len()].copy_from_slice(base.as_bytes());
    out[8..8 + ext.len()].copy_from_slice(ext.as_bytes());
    Ok(out)
}

fn display_name(raw: &[u8]) -> String {
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim_end().to_string();
    let (base, ext) = (text(&raw[..8]), text(&raw[8..11]));
    if ext.is_empty() {
        base
    } else {
        format!("{base}.{ext}")
    }
}

struct Entry {
    slot: usize,
    raw: [u8; 11],
    start: usize,
    size: usize,
}

impl StDisk {
    fn new(bytes: Vec<u8>, format: &'static str) -> Result<Self, String> {
        let layout = parse_layout(&bytes)?;
        Ok(Self {
            bytes,
            layout,
            format,
        })
    }

    fn fat(&self, n: usize) -> u16 {
        let at = self.layout.fat_start(0) + n + n / 2;
        let word = le16(&self.bytes, at) as u16;
        if n & 1 == 1 { word >> 4 } else { word & 0x0FFF }
    }

    fn set_fat(&mut self, n: usize, value: u16) {
        for f in 0..self.layout.fats {
            let at = self.layout.fat_start(f) + n + n / 2;
            let word = le16(&self.bytes, at) as u16;
            let word = if n & 1 == 1 {
                (word & 0x000F) | (value << 4)
            } else {
                (word & 0xF000) | (value & 0x0FFF)
            };
            self.bytes[at..at + 2].copy_from_slice(&word.to_le_bytes());
        }
    }

    /// The live directory entries (no free slots, volume labels or subdirectories).
    fn entries(&self) -> Vec<Entry> {
        let root = self.layout.root_start();
        let mut out = Vec::new();
        for slot in 0..self.layout.root_entries {
            let e = &self.bytes[root + slot * ENTRY..root + (slot + 1) * ENTRY];
            match e[0] {
                0x00 => break,
                FREE_ENTRY => continue,
                _ => {}
            }
            if e[11] & (ATTR_VOLUME_ID | ATTR_DIRECTORY) != 0 {
                continue;
            }
            let mut raw = [0u8; 11];
            raw.copy_from_slice(&e[..11]);
            out.push(Entry {
                slot,
                raw,
                start: le16(e, 26),
                size: u32::from_le_bytes([e[28], e[29], e[30], e[31]]) as usize,
            });
        }
        out
    }

    /// The clusters of a file, checked against the disk and the file's size.
    fn chain(&self, e: &Entry) -> Result<Vec<usize>, String> {
        let name = display_name(&e.raw);
        let needed = e.size.div_ceil(self.layout.cluster_bytes());
        let max = self.layout.clusters() + 1;
        let mut out = Vec::new();
        if e.size == 0 && e.start == 0 {
            return Ok(out);
        }
        let mut n = e.start;
        loop {
            if !(2..=max).contains(&n) {
                return Err(format!("{name}: cluster {n} is outside the disk (2-{max})"));
            }
            if out.len() > self.layout.clusters() {
                return Err(format!("{name}: its cluster chain loops"));
            }
            out.push(n);
            let next = self.fat(n);
            match next {
                0 => return Err(format!("{name}: its chain runs into the free cluster {n}")),
                0xFF7 => {
                    return Err(format!(
                        "{name}: its chain runs into a bad cluster after {n}"
                    ));
                }
                v if v >= 0xFF8 => break,
                v => n = v as usize,
            }
        }
        if out.len() < needed {
            return Err(format!(
                "{name}: the size says {} bytes, the chain holds {} clusters ({} bytes)",
                e.size,
                out.len(),
                out.len() * self.layout.cluster_bytes()
            ));
        }
        Ok(out)
    }

    fn cluster_offset(&self, n: usize) -> usize {
        (self.layout.data_sector() + (n - 2) * self.layout.spc) * SECTOR
    }

    fn free_clusters(&self) -> Vec<usize> {
        (2..self.layout.clusters() + 2)
            .filter(|&n| self.fat(n) == 0)
            .collect()
    }

    fn find(&self, pattern: &str) -> Vec<Entry> {
        self.entries()
            .into_iter()
            .filter(|e| crate::cpm::glob(pattern, &display_name(&e.raw)))
            .collect()
    }

    fn free_entry(&mut self, slot: usize) -> Result<(), String> {
        let e = self
            .entries()
            .into_iter()
            .find(|e| e.slot == slot)
            .ok_or("no such entry")?;
        for n in self.chain(&e).unwrap_or_default() {
            self.set_fat(n, 0);
        }
        self.bytes[self.layout.root_start() + slot * ENTRY] = FREE_ENTRY;
        Ok(())
    }
}

impl Disk for StDisk {
    fn format(&self) -> String {
        self.format.to_string()
    }

    fn description(&self) -> String {
        format!(
            "Atari ST FAT12, {} sectors, {} clusters of {} bytes",
            self.layout.total,
            self.layout.clusters(),
            self.layout.cluster_bytes()
        )
    }

    fn info(&self) -> Vec<(String, String)> {
        let l = &self.layout;
        vec![
            ("format".into(), self.format.into()),
            ("sectors".into(), l.total.to_string()),
            ("sectors per cluster".into(), l.spc.to_string()),
            ("FATs".into(), format!("{} x {} sectors", l.fats, l.spf)),
            ("root entries".into(), l.root_entries.to_string()),
            ("media".into(), format!("${:02X}", l.media)),
            ("free clusters".into(), self.free_blocks().to_string()),
            (
                "bootable".into(),
                (boot_sum(&self.bytes) == 0x1234).to_string(),
            ),
        ]
    }

    fn files(&self) -> Vec<FileRow> {
        self.entries()
            .iter()
            .map(|e| FileRow {
                name: display_name(&e.raw),
                name_hex: e.raw.iter().map(|b| format!("{b:02x}")).collect(),
                kind: String::new(),
                blocks: e.size.div_ceil(self.layout.cluster_bytes()) as u32,
                size: Some(e.size as u64),
                flags: Vec::new(),
            })
            .collect()
    }

    fn free_blocks(&self) -> u64 {
        self.free_clusters().len() as u64
    }

    fn read(&self, pattern: &str, _: &ReadOptions) -> Result<Vec<Extracted>, String> {
        let found = self.find(pattern);
        if found.is_empty() {
            return Err(format!("no file matches \"{pattern}\""));
        }
        found
            .iter()
            .map(|e| {
                let mut data = Vec::with_capacity(e.size);
                for n in self.chain(e)? {
                    let at = self.cluster_offset(n);
                    data.extend_from_slice(&self.bytes[at..at + self.layout.cluster_bytes()]);
                }
                data.truncate(e.size);
                let name = display_name(&e.raw);
                Ok(Extracted {
                    file_name: name.clone(),
                    name,
                    data,
                })
            })
            .collect()
    }

    fn add(&mut self, host_name: &str, data: &[u8], opts: &AddOptions) -> Result<String, String> {
        let wanted = opts.name.clone().unwrap_or_else(|| {
            std::path::Path::new(host_name)
                .file_name()
                .map_or(host_name.to_string(), |n| n.to_string_lossy().into_owned())
        });
        let raw = short_name(&wanted)?;
        let name = display_name(&raw);
        if let Some(old) = self.entries().into_iter().find(|e| e.raw == raw) {
            if !opts.replace {
                return Err(format!("{name} is already on the disk (use --replace)"));
            }
            self.free_entry(old.slot)?;
        }
        if data.len() > u32::MAX as usize {
            return Err(format!("{name}: too large"));
        }
        let root = self.layout.root_start();
        let slot = (0..self.layout.root_entries)
            .find(|&s| matches!(self.bytes[root + s * ENTRY], 0x00 | FREE_ENTRY))
            .ok_or("the root directory is full")?;
        let needed = data.len().div_ceil(self.layout.cluster_bytes());
        let free = self.free_clusters();
        if free.len() < needed {
            return Err(format!(
                "the disk is full: {name} needs {needed} clusters, {} are free",
                free.len()
            ));
        }
        let clusters = &free[..needed];
        for (i, (&n, chunk)) in clusters
            .iter()
            .zip(data.chunks(self.layout.cluster_bytes()))
            .enumerate()
        {
            let at = self.cluster_offset(n);
            self.bytes[at..at + self.layout.cluster_bytes()].fill(0);
            self.bytes[at..at + chunk.len()].copy_from_slice(chunk);
            let next = clusters.get(i + 1).map_or(EOC, |&m| m as u16);
            self.set_fat(n, next);
        }
        let e = root + slot * ENTRY;
        self.bytes[e..e + ENTRY].fill(0);
        self.bytes[e..e + 11].copy_from_slice(&raw);
        self.bytes[e + 26..e + 28]
            .copy_from_slice(&(clusters.first().copied().unwrap_or(0) as u16).to_le_bytes());
        self.bytes[e + 28..e + 32].copy_from_slice(&(data.len() as u32).to_le_bytes());
        Ok(name)
    }

    fn remove(&mut self, pattern: &str) -> Result<Vec<String>, String> {
        let found = self.find(pattern);
        if found.is_empty() {
            return Err(format!("no file matches \"{pattern}\""));
        }
        let mut names = Vec::new();
        for e in found {
            self.free_entry(e.slot)?;
            names.push(display_name(&e.raw));
        }
        Ok(names)
    }

    fn rename(&mut self, old: &str, new: &str) -> Result<(), String> {
        let raw = short_name(new)?;
        if self.entries().iter().any(|e| e.raw == raw) {
            return Err(format!("{} is already on the disk", display_name(&raw)));
        }
        let mut found = self.find(old);
        if found.len() != 1 {
            return Err(format!("\"{old}\" matches {} files, not one", found.len()));
        }
        let e = found.remove(0);
        let at = self.layout.root_start() + e.slot * ENTRY;
        self.bytes[at..at + 11].copy_from_slice(&raw);
        Ok(())
    }

    fn check(&self) -> Check {
        let mut c = Check {
            free_blocks: self.free_blocks(),
            ..Check::default()
        };
        let mut owner = std::collections::HashMap::new();
        for e in self.entries() {
            c.files += 1;
            match self.chain(&e) {
                Ok(chain) => {
                    for n in chain {
                        if let Some(other) = owner.insert(n, display_name(&e.raw)) {
                            c.errors.push(format!(
                                "cluster {n} belongs to {other} and {}",
                                display_name(&e.raw)
                            ));
                        }
                    }
                }
                Err(err) => c.errors.push(err),
            }
        }
        c
    }

    fn to_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    fn label(&self) -> (String, String) {
        (String::new(), String::new())
    }

    fn export(&self) -> (Vec<Exported>, Vec<String>) {
        let mut files = Vec::new();
        let mut skipped = Vec::new();
        for e in self.entries() {
            let name = display_name(&e.raw);
            match self.read(&name, &ReadOptions::default()) {
                Ok(mut x) => files.push(Exported {
                    name,
                    kind: String::new(),
                    data: x.remove(0).data,
                }),
                Err(err) => skipped.push(err),
            }
        }
        (files, skipped)
    }
}

fn boot_sum(b: &[u8]) -> u16 {
    b[..SECTOR].chunks(2).fold(0u16, |s, w| {
        s.wrapping_add(u16::from_be_bytes([w[0], w[1]]))
    })
}

/// `image` with `code` as its boot program: `BRA.S` to offset 30, the code there, and the last word set so the sector
/// sums to `$1234` (*Internals* p.179). The BPB is kept.
pub fn with_boot_code(image: &[u8], code: &[u8]) -> Result<Vec<u8>, String> {
    parse_layout(image)?;
    if code.len() > SECTOR - 2 - BOOT_CODE {
        return Err(format!(
            "the boot program has {} bytes; at most {} fit between offset 30 and the checksum word",
            code.len(),
            SECTOR - 2 - BOOT_CODE
        ));
    }
    let mut b = image.to_vec();
    b[0..2].copy_from_slice(&[0x60, (BOOT_CODE - 2) as u8]);
    b[BOOT_CODE..BOOT_CODE + code.len()].copy_from_slice(code);
    b[SECTOR - 2..SECTOR].fill(0);
    let fix = 0x1234u16.wrapping_sub(boot_sum(&b));
    b[SECTOR - 2..SECTOR].copy_from_slice(&fix.to_be_bytes());
    Ok(b)
}

pub fn open(bytes: &[u8]) -> Result<Box<dyn Disk>, String> {
    let format = if bytes.len() == 368_640 {
        "st-ss"
    } else {
        "st"
    };
    Ok(Box::new(StDisk::new(bytes.to_vec(), format)?))
}

/// A blank, formatted disk: `st` (80 tracks, 2 sides, 9 sectors) or `st-ss` (one side).
pub fn create(format: &str) -> Result<Box<dyn Disk>, String> {
    let (sides, media, format) = match format {
        "st" => (2usize, 0xF9u8, "st"),
        "st-ss" => (1, 0xF8, "st-ss"),
        other => return Err(format!("unknown Atari ST format \"{other}\" (st, st-ss)")),
    };
    let total = 80 * 9 * sides;
    let mut b = vec![0u8; total * SECTOR];
    let put = |b: &mut [u8], at: usize, v: u16| b[at..at + 2].copy_from_slice(&v.to_le_bytes());
    put(&mut b, 11, SECTOR as u16);
    b[13] = 2;
    put(&mut b, 14, 1);
    b[16] = 2;
    put(&mut b, 17, 112);
    put(&mut b, 19, total as u16);
    b[21] = media;
    put(&mut b, 22, 5);
    put(&mut b, 24, 9);
    put(&mut b, 26, sides as u16);
    let mut disk = StDisk::new(b, format)?;
    // FAT[0]: the media byte with the other bits set; FAT[1]: an end-of-chain mark.
    disk.set_fat(0, 0xF00 | media as u16);
    disk.set_fat(1, EOC);
    Ok(Box::new(disk))
}
