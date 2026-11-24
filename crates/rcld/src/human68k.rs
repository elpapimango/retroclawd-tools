//! A Human68k floppy (2HD, `.xdf`: 77 cylinders, 2 heads, 8 sectors of 1024 bytes) with a FAT12 filesystem.
//!
//! Layout `[H?]` (written from memory of the format, not checked against a Human68k machine): sector 0 is the boot sector (an
//! IPL, or zeros with the BPB), then two FATs of 2 sectors, a root directory of 192 entries (6 sectors) and the data area with
//! one sector per cluster, cluster 2 first. The BPB in the boot sector is big endian (bytes per sector at `$12`, sectors per
//! cluster `$14`, FATs `$15`, reserved sectors `$16`, root entries `$18`, total sectors `$1A`, media `$1C`, sectors per FAT
//! `$1D`); the FAT and the directory are little endian as in MS-DOS. Names are written as 8.3 (Human68k's longer names are not).

const SECTOR: usize = 1024;
const TOTAL_SECTORS: usize = 1232;
const FAT_SECTORS: usize = 2;
const ROOT_ENTRIES: usize = 192;
const ROOT_SECTORS: usize = ROOT_ENTRIES * 32 / SECTOR;
const FAT1: usize = 1;
const ROOT: usize = FAT1 + 2 * FAT_SECTORS;
const DATA: usize = ROOT + ROOT_SECTORS;
pub const XDF_SIZE: usize = TOTAL_SECTORS * SECTOR;

pub struct File {
    /// `NAME.EXT`: up to 8 and 3 characters.
    pub name: String,
    pub data: Vec<u8>,
}

/// Splits a name into the space-padded 8 and 3 characters of a directory entry.
fn split_name(name: &str) -> Result<([u8; 8], [u8; 3]), String> {
    let upper = name.to_ascii_uppercase();
    let (stem, ext) = upper.rsplit_once('.').unwrap_or((&upper, ""));
    let ok = |s: &str, max: usize| {
        s.len() <= max
            && s.bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || b"_-$~!#%&@^".contains(&c))
    };
    if stem.is_empty() || !ok(stem, 8) || !ok(ext, 3) {
        return Err(format!(
            "\"{name}\": a file name on this disk is up to 8 letters, digits or _ - $ ~ ! # % & @ ^ and an extension of up to 3"
        ));
    }
    let mut a = [b' '; 8];
    let mut b = [b' '; 3];
    a[..stem.len()].copy_from_slice(stem.as_bytes());
    b[..ext.len()].copy_from_slice(ext.as_bytes());
    Ok((a, b))
}

fn put12(fat: &mut [u8], cluster: usize, value: u16) {
    let at = cluster * 3 / 2;
    if cluster % 2 == 0 {
        fat[at] = value as u8;
        fat[at + 1] = (fat[at + 1] & 0xF0) | (value >> 8) as u8 & 0x0F;
    } else {
        fat[at] = (fat[at] & 0x0F) | ((value << 4) as u8 & 0xF0);
        fat[at + 1] = (value >> 4) as u8;
    }
}

/// One 32-byte directory entry at slot `*entry`.
fn add_entry(
    root: &mut [u8],
    entry: &mut usize,
    name: [u8; 8],
    ext: [u8; 3],
    attr: u8,
    cluster: u16,
    size: u32,
) {
    let e = &mut root[*entry * 32..*entry * 32 + 32];
    e[..8].copy_from_slice(&name);
    e[8..11].copy_from_slice(&ext);
    e[11] = attr;
    // 1993-10-23 00:00:00
    e[22..24].copy_from_slice(&0u16.to_le_bytes());
    e[24..26].copy_from_slice(&(((1993 - 1980) << 9) | (10 << 5) | 23u16).to_le_bytes());
    e[26..28].copy_from_slice(&cluster.to_le_bytes());
    e[28..32].copy_from_slice(&size.to_le_bytes());
    *entry += 1;
}

/// The floppy image. `ipl` is the boot sector's code (at most 1024 bytes, its BPB is overwritten); without it the boot sector
/// is a branch and the BPB, so the disk is a data disk. `label` is the volume label.
pub fn build_xdf(
    files: &[File],
    label: Option<&str>,
    ipl: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let mut disk = vec![0u8; XDF_SIZE];
    // the boot sector
    match ipl {
        Some(code) => {
            if code.len() > SECTOR {
                return Err(format!(
                    "the boot code is {} bytes: a boot sector holds {SECTOR}",
                    code.len()
                ));
            }
            disk[..code.len()].copy_from_slice(code);
        }
        None => {
            disk[0] = 0x60;
            disk[1] = 0x1E;
            disk[2..18].copy_from_slice(b"Hudson soft 2.00");
        }
    }
    disk[0x12..0x14].copy_from_slice(&(SECTOR as u16).to_be_bytes());
    disk[0x14] = 1;
    disk[0x15] = 2;
    disk[0x16..0x18].copy_from_slice(&1u16.to_be_bytes());
    disk[0x18..0x1A].copy_from_slice(&(ROOT_ENTRIES as u16).to_be_bytes());
    disk[0x1A..0x1C].copy_from_slice(&(TOTAL_SECTORS as u16).to_be_bytes());
    disk[0x1C] = 0xFE;
    disk[0x1D] = FAT_SECTORS as u8;
    // the FAT and the directory
    let mut fat = vec![0u8; FAT_SECTORS * SECTOR];
    fat[..3].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
    let mut root = vec![0u8; ROOT_SECTORS * SECTOR];
    let mut entry = 0;
    if let Some(l) = label {
        let upper = l.to_ascii_uppercase();
        if upper.len() > 11 {
            return Err(format!(
                "the volume label \"{l}\" is longer than 11 characters"
            ));
        }
        let mut n = [b' '; 8];
        let mut x = [b' '; 3];
        for (i, c) in upper.bytes().enumerate() {
            if i < 8 {
                n[i] = c;
            } else {
                x[i - 8] = c;
            }
        }
        add_entry(&mut root, &mut entry, n, x, 0x08, 0, 0);
    }
    let clusters = TOTAL_SECTORS - DATA;
    let mut next = 2usize;
    let mut sorted: Vec<&File> = files.iter().collect();
    sorted.sort_by(|a, b| {
        a.name
            .to_ascii_uppercase()
            .cmp(&b.name.to_ascii_uppercase())
    });
    for f in sorted {
        let (name, ext) = split_name(&f.name)?;
        if entry >= ROOT_ENTRIES {
            return Err(format!("the root directory holds {ROOT_ENTRIES} entries"));
        }
        if root[..entry * 32]
            .chunks(32)
            .any(|e| e[..8] == name && e[8..11] == ext)
        {
            return Err(format!(
                "two files are named {}",
                f.name.to_ascii_uppercase()
            ));
        }
        let n = f.data.len().div_ceil(SECTOR);
        if next - 2 + n > clusters {
            return Err(format!(
                "the files do not fit: the disk has {clusters} sectors of data, {} is the file that does not fit",
                f.name
            ));
        }
        let first = if n == 0 { 0 } else { next };
        for k in 0..n {
            let c = next + k;
            put12(&mut fat, c, if k + 1 == n { 0xFFF } else { (c + 1) as u16 });
            let at = (DATA + c - 2) * SECTOR;
            let chunk = &f.data[k * SECTOR..((k + 1) * SECTOR).min(f.data.len())];
            disk[at..at + chunk.len()].copy_from_slice(chunk);
        }
        next += n;
        add_entry(
            &mut root,
            &mut entry,
            name,
            ext,
            0x20,
            first as u16,
            f.data.len() as u32,
        );
    }
    for k in 0..2 {
        let at = (FAT1 + k * FAT_SECTORS) * SECTOR;
        disk[at..at + fat.len()].copy_from_slice(&fat);
    }
    disk[ROOT * SECTOR..DATA * SECTOR].copy_from_slice(&root);
    Ok(disk)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads a file back with its own FAT walk.
    fn read(disk: &[u8], name: &str) -> Option<Vec<u8>> {
        let (n, e) = split_name(name).ok()?;
        let root = &disk[ROOT * SECTOR..DATA * SECTOR];
        let ent = root.chunks(32).find(|c| c[..8] == n && c[8..11] == e)?;
        let mut cluster = u16::from_le_bytes([ent[26], ent[27]]) as usize;
        let size = u32::from_le_bytes(ent[28..32].try_into().ok()?) as usize;
        let fat = &disk[FAT1 * SECTOR..(FAT1 + FAT_SECTORS) * SECTOR];
        let mut out = Vec::new();
        while cluster >= 2 && cluster < 0xFF8 && out.len() < size + SECTOR {
            let at = (DATA + cluster - 2) * SECTOR;
            out.extend_from_slice(&disk[at..at + SECTOR]);
            let i = cluster * 3 / 2;
            let v = u16::from_le_bytes([fat[i], fat[i + 1]]);
            cluster = if cluster % 2 == 0 { v & 0xFFF } else { v >> 4 } as usize;
        }
        out.truncate(size);
        Some(out)
    }

    #[test]
    fn files_round_trip_through_the_fat() {
        let big: Vec<u8> = (0..5000).map(|i| (i % 253) as u8).collect();
        let disk = build_xdf(
            &[
                File {
                    name: "prog.x".into(),
                    data: big.clone(),
                },
                File {
                    name: "readme".into(),
                    data: b"hi".to_vec(),
                },
                File {
                    name: "empty.dat".into(),
                    data: Vec::new(),
                },
            ],
            Some("mydisk"),
            None,
        )
        .unwrap();
        assert_eq!(disk.len(), 1_261_568);
        assert_eq!(read(&disk, "PROG.X").unwrap(), big);
        assert_eq!(read(&disk, "readme").unwrap(), b"hi");
        assert_eq!(read(&disk, "EMPTY.DAT").unwrap(), Vec::<u8>::new());
        // the BPB
        assert_eq!(u16::from_be_bytes([disk[0x12], disk[0x13]]), 1024);
        assert_eq!(disk[0x1C], 0xFE);
        // the two FATs agree and start with the media byte
        assert_eq!(&disk[1024..1027], &[0xFE, 0xFF, 0xFF]);
        assert_eq!(disk[1024..3072], disk[3072..5120]);
        // the label is an entry with the volume attribute
        assert_eq!(&disk[ROOT * SECTOR..ROOT * SECTOR + 6], b"MYDISK");
        assert_eq!(disk[ROOT * SECTOR + 11], 0x08);
    }

    #[test]
    fn mistakes_are_named() {
        assert!(
            build_xdf(
                &[File {
                    name: "toolongname.x".into(),
                    data: vec![]
                }],
                None,
                None
            )
            .is_err()
        );
        assert!(
            build_xdf(
                &[File {
                    name: "a b.x".into(),
                    data: vec![]
                }],
                None,
                None
            )
            .is_err()
        );
        let huge = vec![0u8; 1_300_000];
        let e = build_xdf(
            &[File {
                name: "huge.bin".into(),
                data: huge,
            }],
            None,
            None,
        )
        .unwrap_err();
        assert!(e.contains("do not fit"), "{e}");
        let dup = [
            File {
                name: "a.x".into(),
                data: vec![],
            },
            File {
                name: "A.X".into(),
                data: vec![],
            },
        ];
        assert!(
            build_xdf(&dup, None, None)
                .unwrap_err()
                .contains("two files")
        );
    }

    #[test]
    fn twelve_bit_entries_pack_two_to_three_bytes() {
        let mut fat = vec![0u8; 6];
        put12(&mut fat, 2, 0xABC);
        put12(&mut fat, 3, 0x123);
        assert_eq!(&fat[3..6], &[0xBC, 0x3A, 0x12]);
    }
}
