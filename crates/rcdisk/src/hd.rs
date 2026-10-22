//! Atari ST hard disk images: an AHDI root sector with its partitions, and FAT16 file systems inside them.
//!
//! Sources: the Atari Compendium, chapter 4 "The Hard Disk" (hard disk boot sector, partition header, extended
//! partitions; <http://bus-error.nokturnal.pl/atari_compendium/html/chap4.htm>, page SHA-256
//! `36b88a35bf8858582348cc8f31bb75bf216f5302aca95214b04bf5d58ac2fff3`):
//!
//! | Offset | Contents |
//! |---|---|
//! | `$1C2` | `hd_siz`, a Motorola long: the number of 512-byte sectors on the device |
//! | `$1C6`, `$1D2`, `$1DE`, `$1EA` | four 12-byte partition headers |
//! | `$1F6`, `$1FA` | `bst_st` and `bst_cnt`: the bad sector list (a sector offset and a count of sectors) |
//!
//! A partition header: a flag byte (bit 0 the partition exists, bit 7 it is bootable), the three-byte id (`GEM` a
//! regular partition under 16 MB, `BGM` a big one, `XGM` an extended partition), `p_st` and `p_size` as Motorola longs in
//! 512-byte sectors. An extended partition starts with a root sector of its own: its first header describes a partition
//! (`p_st` from the start of the extended partition), a second header holding `XGM` links the next extended partition
//! (`p_st` again from the start of the first extended partition, [H?]: the page says "an offset to the next extended
//! partition"). The file system inside a partition is FAT16 with a BPB in its first sector like a floppy's (media `$F8`
//! for a hard disk); a partition of 32 MB or more has logical sectors of more than 512 bytes so that its sector count
//! fits the BPB's 16-bit field.
//!
//! The image is untrusted: every offset and size is checked against the image, partition chains are followed under a
//! depth limit with loop detection, FAT chains under a step limit, and nothing is allocated from a size in the file.

use std::collections::BTreeSet;

pub const SECTOR: usize = 512;
/// The most an image may be (the machine takes at most this much too).
pub const MAX_IMAGE: u64 = 512 << 20;
const MAX_EXTENDED_DEPTH: usize = 16;
const ROOT_ENTRIES: usize = 512;
/// A 16-bit FAT holds at most this many clusters (the top values mean end of chain, free, bad).
const MAX_CLUSTERS: usize = 65_524;
/// Fewer clusters than this makes a FAT12 file system by the usual rule: too small for this tool.
const MIN_CLUSTERS: usize = 4_096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partition {
    pub flag: u8,
    pub id: [u8; 3],
    /// First 512-byte sector of the partition on the disk.
    pub start: u32,
    pub size: u32,
}

impl Partition {
    pub fn id_text(&self) -> String {
        String::from_utf8_lossy(&self.id).into_owned()
    }
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn le16(b: &[u8], at: usize) -> usize {
    u16::from_le_bytes([b[at], b[at + 1]]) as usize
}

#[derive(Debug)]
pub struct HardDisk {
    data: Vec<u8>,
}

/// A directory entry of the root directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// `NAME.EXT` as shown (no padding).
    pub name: String,
    pub size: u32,
    pub directory: bool,
    pub first_cluster: u16,
}

/// What a FAT16 file system's BPB says, checked.
struct Geometry {
    /// Offset of the partition's first byte in the image.
    base: usize,
    bps: usize,
    spc: usize,
    reserved: usize,
    fats: usize,
    spf: usize,
    root_start: usize,
    root_sectors: usize,
    data_start: usize,
    clusters: usize,
}

impl HardDisk {
    /// A disk image: a multiple of 512 bytes, at least two sectors, at most [`MAX_IMAGE`]. The root sector's contents
    /// are checked when the partitions are read.
    pub fn parse(data: Vec<u8>) -> Result<Self, String> {
        let n = data.len() as u64;
        if n > MAX_IMAGE {
            return Err(format!(
                "{n} bytes is larger than the {MAX_IMAGE} byte limit"
            ));
        }
        if n < 2 * SECTOR as u64 || !n.is_multiple_of(SECTOR as u64) {
            return Err(format!(
                "{n} bytes is not a hard disk image (whole 512-byte sectors, at least two)"
            ));
        }
        Ok(Self { data })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    fn sectors(&self) -> u64 {
        (self.data.len() / SECTOR) as u64
    }

    /// A new disk of `total` sectors with one partition of `partition` sectors starting at sector 1, formatted FAT16.
    /// A partition under 16 MB is `GEM`, from 16 MB `BGM`; a size the format cannot hold is an error naming the limit.
    pub fn create(total: u64, partition: u64) -> Result<Self, String> {
        if total * SECTOR as u64 > MAX_IMAGE {
            return Err(format!(
                "a disk of {total} sectors is larger than {MAX_IMAGE} bytes"
            ));
        }
        if partition == 0 || partition + 1 > total {
            return Err("the partition must fit on the disk after the root sector".to_string());
        }
        let g = format_params(partition)?;
        let mut data = vec![0u8; total as usize * SECTOR];
        // The root sector.
        {
            let root = &mut data[..SECTOR];
            root[0x1C2..0x1C6].copy_from_slice(&(total as u32).to_be_bytes());
            let id: &[u8; 3] = if partition * (SECTOR as u64) >= 16 << 20 {
                b"BGM"
            } else {
                b"GEM"
            };
            root[0x1C6] = 0x81; // exists and bootable
            root[0x1C7..0x1CA].copy_from_slice(id);
            root[0x1CA..0x1CE].copy_from_slice(&1u32.to_be_bytes());
            root[0x1CE..0x1D2].copy_from_slice(&(partition as u32).to_be_bytes());
            // The checksum of the root sector says it is not executable: it is data.
        }
        let mut disk = HardDisk { data };
        disk.format_fat16(1, partition as usize, &g);
        Ok(disk)
    }

    /// The partitions: the standard ones and those reached through extended partitions.
    pub fn partitions(&self) -> Result<Vec<Partition>, String> {
        let total = self.sectors();
        let root = &self.data[..SECTOR];
        let mut out = Vec::new();
        let mut extended: Vec<u32> = Vec::new();
        for i in 0..4 {
            let p = read_header(root, 0x1C6 + 12 * i);
            if p.flag & 1 == 0 {
                continue;
            }
            check_inside(&p, total)?;
            match &p.id {
                b"XGM" => extended.push(p.start),
                _ => out.push(p),
            }
        }
        if out.is_empty() && extended.is_empty() {
            return Err("the root sector has no partition".to_string());
        }
        // Each extended partition: its sector holds a partition and a link to the next one.
        let first_extended = extended.first().copied();
        let mut seen = BTreeSet::new();
        let mut queue: Vec<(u32, usize)> = extended.into_iter().map(|s| (s, 1)).collect();
        while let Some((sector, depth)) = queue.pop() {
            if depth > MAX_EXTENDED_DEPTH {
                return Err("extended partitions nest too deep".to_string());
            }
            if !seen.insert(sector) {
                return Err("extended partitions link back to one already seen".to_string());
            }
            let at = (sector as usize)
                .checked_mul(SECTOR)
                .filter(|&at| at + SECTOR <= self.data.len())
                .ok_or("an extended partition starts beyond the disk")?;
            let ext = &self.data[at..at + SECTOR];
            let base = first_extended.unwrap_or(sector);
            let mut used = 0;
            for i in 0..4 {
                let mut p = read_header(ext, 0x1C6 + 12 * i);
                if p.flag & 1 == 0 {
                    continue;
                }
                used += 1;
                if used > 2 {
                    return Err("an extended partition uses more than two headers".to_string());
                }
                p.start = p
                    .start
                    .checked_add(if p.id == *b"XGM" { base } else { sector })
                    .ok_or("a partition start overflows")?;
                check_inside(&p, total)?;
                if p.id == *b"XGM" {
                    queue.push((p.start, depth + 1));
                } else {
                    out.push(p);
                }
            }
        }
        Ok(out)
    }

    fn geometry(&self, p: &Partition) -> Result<Geometry, String> {
        let base = p.start as usize * SECTOR;
        let end = base
            .checked_add(p.size as usize * SECTOR)
            .filter(|&e| e <= self.data.len())
            .ok_or("the partition is beyond the disk")?;
        let boot = &self.data[base..base + SECTOR];
        let bps = le16(boot, 11);
        let spc = boot[13] as usize;
        let reserved = le16(boot, 14);
        let fats = boot[16] as usize;
        let root_entries = le16(boot, 17);
        let total_sectors = le16(boot, 19);
        let spf = le16(boot, 22);
        if !(512..=8192).contains(&bps)
            || !bps.is_power_of_two()
            || spc == 0
            || !spc.is_power_of_two()
        {
            return Err("the partition's boot sector has no usable BPB (bytes per sector, sectors per cluster)".to_string());
        }
        if reserved == 0 || fats == 0 || fats > 2 || spf == 0 || root_entries == 0 {
            return Err("the partition's BPB is not a FAT file system".to_string());
        }
        let _fits = total_sectors
            .checked_mul(bps)
            .filter(|&b| b > 0 && base + b <= end)
            .ok_or("the BPB's size is beyond the partition")?;
        let root_start = reserved + fats * spf;
        let root_sectors = (root_entries * 32).div_ceil(bps);
        let data_start = root_start + root_sectors;
        if data_start >= total_sectors {
            return Err("the BPB leaves no room for data".to_string());
        }
        let clusters = (total_sectors - data_start) / spc;
        if clusters < MIN_CLUSTERS {
            return Err(format!(
                "{clusters} clusters: a FAT12 file system, which this tool does not read"
            ));
        }
        if clusters > MAX_CLUSTERS || spf * bps < (clusters + 2) * 2 {
            return Err("the FAT is too small for its clusters, or there are too many".to_string());
        }
        Ok(Geometry {
            base,
            bps,
            spc,
            reserved,
            fats,
            spf,
            root_start,
            root_sectors,
            data_start,
            clusters,
        })
    }

    /// The first FAT of partition `p`.
    fn fat<'a>(&'a self, g: &Geometry) -> &'a [u8] {
        let at = g.base + g.reserved * g.bps;
        &self.data[at..at + g.spf * g.bps]
    }

    fn fat_get(&self, g: &Geometry, cluster: usize) -> u16 {
        let f = self.fat(g);
        u16::from_le_bytes([f[cluster * 2], f[cluster * 2 + 1]])
    }

    fn fat_set(&mut self, g: &Geometry, cluster: usize, value: u16) {
        for copy in 0..g.fats {
            let at = g.base + (g.reserved + copy * g.spf) * g.bps + cluster * 2;
            self.data[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }
    }

    fn cluster_offset(g: &Geometry, c: usize) -> usize {
        g.base + (g.data_start + (c - 2) * g.spc) * g.bps
    }

    /// The clusters of the chain starting at `first`: a bounded walk that fails on a cycle or a bad link.
    fn chain(&self, g: &Geometry, first: usize) -> Result<Vec<usize>, String> {
        let mut out = Vec::new();
        let mut c = first;
        while (2..g.clusters + 2).contains(&c) {
            if out.len() > g.clusters {
                return Err("a FAT chain is longer than the disk: it loops".to_string());
            }
            out.push(c);
            let next = self.fat_get(g, c) as usize;
            if next >= 0xFFF8 {
                return Ok(out);
            }
            if next < 2 || next >= g.clusters + 2 {
                return Err(format!("a FAT chain runs into cluster {next}"));
            }
            c = next;
        }
        Err(format!(
            "a FAT chain starts at cluster {first}, which does not exist"
        ))
    }

    /// The files in the root directory of partition `index`.
    pub fn list(&self, index: usize) -> Result<Vec<Entry>, String> {
        let p = self.partition(index)?;
        let g = self.geometry(&p)?;
        let at = g.base + g.root_start * g.bps;
        let root = &self.data[at..at + g.root_sectors * g.bps];
        let mut out = Vec::new();
        for e in root.chunks(32) {
            match e[0] {
                0 => break,
                0xE5 => continue,
                _ => {}
            }
            if e[11] == 0x0F || e[11] & 0x08 != 0 {
                continue; // a long name piece or the volume label
            }
            let stem = String::from_utf8_lossy(&e[..8]).trim_end().to_string();
            let ext = String::from_utf8_lossy(&e[8..11]).trim_end().to_string();
            if stem == "." || stem == ".." {
                continue;
            }
            out.push(Entry {
                name: if ext.is_empty() {
                    stem
                } else {
                    format!("{stem}.{ext}")
                },
                size: u32::from_le_bytes([e[28], e[29], e[30], e[31]]),
                directory: e[11] & 0x10 != 0,
                first_cluster: u16::from_le_bytes([e[26], e[27]]),
            });
        }
        Ok(out)
    }

    fn partition(&self, index: usize) -> Result<Partition, String> {
        self.partitions()?
            .into_iter()
            .nth(index)
            .ok_or_else(|| format!("there is no partition {index}"))
    }

    /// A file of the root directory (case-insensitive name).
    pub fn read(&self, index: usize, name: &str) -> Result<Vec<u8>, String> {
        let p = self.partition(index)?;
        let g = self.geometry(&p)?;
        let entry = self
            .list(index)?
            .into_iter()
            .find(|e| e.name.eq_ignore_ascii_case(name) && !e.directory)
            .ok_or_else(|| format!("{name} is not in the root directory"))?;
        let mut out = Vec::new();
        if entry.size == 0 {
            return Ok(out);
        }
        let cluster_bytes = g.spc * g.bps;
        for c in self.chain(&g, entry.first_cluster as usize)? {
            let at = Self::cluster_offset(&g, c);
            out.extend_from_slice(&self.data[at..at + cluster_bytes]);
        }
        if out.len() < entry.size as usize {
            return Err(format!(
                "{name}'s chain holds {} bytes, its size says {}",
                out.len(),
                entry.size
            ));
        }
        out.truncate(entry.size as usize);
        Ok(out)
    }

    /// Puts `data` into the root directory of partition `index` as `name` (8.3, upper-cased). An existing file of the
    /// name is an error unless `replace`.
    pub fn add(
        &mut self,
        index: usize,
        name: &str,
        data: &[u8],
        replace: bool,
    ) -> Result<(), String> {
        let short = short_name(name)?;
        let p = self.partition(index)?;
        let g = self.geometry(&p)?;
        if self
            .list(index)?
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case(&short_text(&short)))
        {
            if !replace {
                return Err(format!("{name} is already on the disk"));
            }
            self.remove(index, name)?;
        }
        let cluster_bytes = g.spc * g.bps;
        let need = data.len().div_ceil(cluster_bytes);
        // Free clusters, scanning the FAT.
        let free: Vec<usize> = (2..g.clusters + 2)
            .filter(|&c| self.fat_get(&g, c) == 0)
            .take(need)
            .collect();
        if free.len() < need {
            return Err(format!(
                "the disk is full: {} bytes need {need} clusters, {} are free",
                data.len(),
                free.len()
            ));
        }
        let root_at = g.base + g.root_start * g.bps;
        let slot = (0..ROOT_ENTRIES.min(g.root_sectors * g.bps / 32))
            .find(|i| {
                let b = self.data[root_at + i * 32];
                b == 0 || b == 0xE5
            })
            .ok_or("the root directory is full")?;
        for (i, &c) in free.iter().enumerate() {
            let next = free.get(i + 1).map_or(0xFFFF, |&n| n as u16);
            self.fat_set(&g, c, next);
            let at = Self::cluster_offset(&g, c);
            let chunk = &data[i * cluster_bytes..data.len().min((i + 1) * cluster_bytes)];
            self.data[at..at + chunk.len()].copy_from_slice(chunk);
            self.data[at + chunk.len()..at + cluster_bytes].fill(0);
        }
        let mut e = [0u8; 32];
        e[..11].copy_from_slice(&short);
        e[11] = 0x20;
        e[24..26].copy_from_slice(&((2026u16 - 1980) << 9 | 1 << 5 | 1).to_le_bytes());
        e[26..28].copy_from_slice(&(free.first().copied().unwrap_or(0) as u16).to_le_bytes());
        e[28..32].copy_from_slice(&(data.len() as u32).to_le_bytes());
        let at = root_at + slot * 32;
        self.data[at..at + 32].copy_from_slice(&e);
        Ok(())
    }

    /// Deletes a root directory file and frees its clusters.
    pub fn remove(&mut self, index: usize, name: &str) -> Result<(), String> {
        let p = self.partition(index)?;
        let g = self.geometry(&p)?;
        let want = short_text(&short_name(name)?);
        let root_at = g.base + g.root_start * g.bps;
        for i in 0..g.root_sectors * g.bps / 32 {
            let at = root_at + i * 32;
            let e = self.data[at..at + 32].to_vec();
            if e[0] == 0 {
                break;
            }
            if e[0] == 0xE5 || e[11] & 0x18 != 0 {
                continue;
            }
            let stem = String::from_utf8_lossy(&e[..8]).trim_end().to_string();
            let ext = String::from_utf8_lossy(&e[8..11]).trim_end().to_string();
            let shown = if ext.is_empty() {
                stem
            } else {
                format!("{stem}.{ext}")
            };
            if shown.eq_ignore_ascii_case(&want) {
                let first = u16::from_le_bytes([e[26], e[27]]) as usize;
                if first >= 2 {
                    for c in self.chain(&g, first)? {
                        self.fat_set(&g, c, 0);
                    }
                }
                self.data[at] = 0xE5;
                return Ok(());
            }
        }
        Err(format!("{name} is not on the disk"))
    }

    /// Writes the boot sector, both FATs and an empty root directory of a FAT16 file system over `size` sectors from
    /// sector `start`.
    fn format_fat16(&mut self, start: usize, size: usize, g: &FormatParams) {
        let base = start * SECTOR;
        let boot = &mut self.data[base..base + SECTOR];
        let le = |b: &mut [u8], at: usize, v: usize| {
            b[at..at + 2].copy_from_slice(&(v as u16).to_le_bytes())
        };
        boot[0..2].copy_from_slice(&0x601Cu16.to_be_bytes());
        boot[2..8].copy_from_slice(b"RCDISK");
        boot[8..11].copy_from_slice(&[0x52, 0x43, 0x44]);
        le(boot, 11, g.bps);
        boot[13] = g.spc as u8;
        le(boot, 14, 1);
        boot[16] = 2;
        le(boot, 17, ROOT_ENTRIES);
        le(boot, 19, size * SECTOR / g.bps);
        boot[21] = 0xF8;
        le(boot, 22, g.spf);
        le(boot, 24, 16);
        le(boot, 26, 2);
        for copy in 0..2 {
            let at = base + (1 + copy * g.spf) * g.bps;
            self.data[at..at + 2].copy_from_slice(&0xFFF8u16.to_le_bytes());
            self.data[at + 2..at + 4].copy_from_slice(&0xFFFFu16.to_le_bytes());
        }
    }
}

struct FormatParams {
    bps: usize,
    spc: usize,
    spf: usize,
}

/// The cluster size, logical sector size and FAT size for a partition of `sectors` 512-byte sectors.
fn format_params(sectors: u64) -> Result<FormatParams, String> {
    let bytes = sectors * SECTOR as u64;
    // Logical sectors grow until the count fits the BPB's 16-bit field.
    let bps = [512usize, 1024, 2048, 4096]
        .into_iter()
        .find(|&b| bytes / b as u64 <= 65_535)
        .ok_or_else(|| {
            format!("{bytes} bytes is more than a FAT16 partition with this BPB holds (256 MiB)")
        })?;
    let logical = (bytes / bps as u64) as usize;
    for spc in [1usize, 2, 4, 8, 16, 32, 64] {
        // The FAT takes `spf` sectors of each copy; iterate to a fixed point.
        let mut spf = 1;
        loop {
            let overhead = 1 + 2 * spf + (ROOT_ENTRIES * 32).div_ceil(bps);
            if overhead >= logical {
                break;
            }
            let clusters = (logical - overhead) / spc;
            let need = ((clusters + 2) * 2).div_ceil(bps);
            if need <= spf {
                if (MIN_CLUSTERS..=MAX_CLUSTERS).contains(&clusters) {
                    return Ok(FormatParams { bps, spc, spf });
                }
                break;
            }
            spf = need;
        }
    }
    Err(format!(
        "a partition of {sectors} sectors cannot be FAT16: it needs at least {MIN_CLUSTERS} clusters (about 2 MiB)"
    ))
}

fn read_header(sector: &[u8], at: usize) -> Partition {
    Partition {
        flag: sector[at],
        id: [sector[at + 1], sector[at + 2], sector[at + 3]],
        start: be32(sector, at + 4),
        size: be32(sector, at + 8),
    }
}

fn check_inside(p: &Partition, total: u64) -> Result<(), String> {
    let end = u64::from(p.start) + u64::from(p.size);
    if p.size == 0 || end > total {
        return Err(format!(
            "the {} partition at sector {} of {} sectors is beyond the disk's {total}",
            p.id_text(),
            p.start,
            p.size
        ));
    }
    if !matches!(&p.id, b"GEM" | b"BGM" | b"XGM") {
        return Err(format!(
            "partition id {:?} is not GEM, BGM or XGM",
            p.id_text()
        ));
    }
    Ok(())
}

/// `NAME.EXT` as the 11 bytes of a directory entry.
fn short_name(name: &str) -> Result<[u8; 11], String> {
    let bad = || format!("\"{name}\" is not an 8.3 file name");
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let ok = |s: &str, max: usize| {
        s.len() <= max
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-$~!#%&@^".contains(&b))
    };
    if stem.is_empty() || !ok(stem, 8) || !ok(ext, 3) {
        return Err(bad());
    }
    let mut out = [b' '; 11];
    for (i, b) in stem.bytes().enumerate() {
        out[i] = b.to_ascii_uppercase();
    }
    for (i, b) in ext.bytes().enumerate() {
        out[8 + i] = b.to_ascii_uppercase();
    }
    Ok(out)
}

fn short_text(short: &[u8; 11]) -> String {
    let stem = String::from_utf8_lossy(&short[..8]).trim_end().to_string();
    let ext = String::from_utf8_lossy(&short[8..]).trim_end().to_string();
    if ext.is_empty() {
        stem
    } else {
        format!("{stem}.{ext}")
    }
}

/// A size such as `8M`, `512K`, `2G` or a plain number of bytes, in bytes.
pub fn parse_size(text: &str) -> Result<u64, String> {
    let t = text.trim();
    let (digits, mult) = match t.chars().last() {
        Some('k' | 'K') => (&t[..t.len() - 1], 1u64 << 10),
        Some('m' | 'M') => (&t[..t.len() - 1], 1 << 20),
        Some('g' | 'G') => (&t[..t.len() - 1], 1 << 30),
        _ => (t, 1),
    };
    digits
        .trim()
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(mult))
        .ok_or_else(|| format!("\"{text}\" is not a size (a number with K, M or G)"))
}

/// A file name from the command line that is used as a path to write: no separators, no `..`, not absolute. The names
/// stored on a disk are 8.3 names and cannot hold any of these, so this guards what the user types.
pub fn safe_file_name(name: &str) -> Result<&str, String> {
    let separator = |c: char| c == '/' || c == '\\' || c == ':';
    if name.is_empty() || name == "." || name == ".." || name.contains(separator) {
        return Err(format!("\"{name}\" is not a plain file name"));
    }
    Ok(name)
}
