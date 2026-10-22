//! Atari ST hard disk images: AHDI partitions and FAT16 (Atari Compendium chapter 4 for the root sector).

use rcdisk::hd::{HardDisk, SECTOR, parse_size};

const MIB: u64 = 1 << 20;

fn disk(total_mib: u64, partition_mib: u64) -> HardDisk {
    HardDisk::create(
        total_mib * MIB / SECTOR as u64,
        partition_mib * MIB / SECTOR as u64,
    )
    .unwrap()
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

#[test]
fn a_new_disk_has_a_valid_ahdi_root() {
    let d = disk(8, 7);
    let root = &d.bytes()[..SECTOR];
    assert_eq!(be32(root, 0x1C2) as u64, 8 * MIB / 512, "hd_siz");
    assert_eq!(root[0x1C6] & 0x81, 0x81, "p_flg: exists and bootable");
    assert_eq!(
        &root[0x1C7..0x1CA],
        b"GEM",
        "a partition under 16 MB is GEM"
    );
    assert_eq!(be32(root, 0x1CA), 1, "p_st");
    assert_eq!(be32(root, 0x1CE) as u64, 7 * MIB / 512, "p_size");
    for i in 1..4 {
        assert_eq!(root[0x1C6 + 12 * i] & 1, 0, "partition {i} does not exist");
    }
    let parts = d.partitions().unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].id_text(), "GEM");
    // The partition's boot sector: a BPB with media $F8.
    let boot = &d.bytes()[SECTOR..2 * SECTOR];
    assert_eq!(u16::from_le_bytes([boot[11], boot[12]]), 512);
    assert_eq!(boot[21], 0xF8, "a hard disk's media byte");
    assert_eq!(boot[16], 2, "two FATs");
    assert!(d.list(0).unwrap().is_empty());
}

#[test]
fn a_big_partition_is_bgm_with_larger_logical_sectors() {
    let d = disk(40, 40 - 1);
    let parts = d.partitions().unwrap();
    assert_eq!(parts[0].id_text(), "BGM", "16 MB or more");
    let boot = &d.bytes()[SECTOR..2 * SECTOR];
    let bps = u16::from_le_bytes([boot[11], boot[12]]);
    let sectors = u16::from_le_bytes([boot[19], boot[20]]) as u64;
    assert!(
        bps > 512,
        "{bps}: a 39 MiB partition does not fit 16 bits of 512-byte sectors"
    );
    assert_eq!(sectors * bps as u64, 39 * MIB);
    let mut d = d;
    d.add(0, "big.txt", &vec![7u8; 100_000], false).unwrap();
    assert_eq!(d.read(0, "BIG.TXT").unwrap(), vec![7u8; 100_000]);
}

#[test]
fn add_list_extract_round_trip_fat16() {
    let mut d = disk(8, 7);
    let hello: Vec<u8> = (0..3000u32).map(|i| (i * 13) as u8).collect();
    d.add(0, "hello.txt", b"Hello from rcdisk.\r\n", false)
        .unwrap();
    d.add(0, "Data.BIN", &hello, false).unwrap();
    let names: Vec<_> = d
        .list(0)
        .unwrap()
        .into_iter()
        .map(|e| (e.name, e.size))
        .collect();
    assert_eq!(
        names,
        vec![
            ("HELLO.TXT".to_string(), 20),
            ("DATA.BIN".to_string(), 3000)
        ]
    );
    assert_eq!(d.read(0, "hello.txt").unwrap(), b"Hello from rcdisk.\r\n");
    assert_eq!(d.read(0, "data.bin").unwrap(), hello);
    // The image survives being written and read back.
    let again = HardDisk::parse(d.bytes().to_vec()).unwrap();
    assert_eq!(again.read(0, "DATA.BIN").unwrap(), hello);
    assert!(d.read(0, "missing.txt").is_err());
}

#[test]
fn replacing_and_removing_free_the_clusters() {
    let mut d = disk(8, 7);
    let before = d.bytes().to_vec();
    d.add(0, "a.bin", &vec![1u8; 50_000], false).unwrap();
    assert!(
        d.add(0, "a.bin", b"x", false)
            .unwrap_err()
            .contains("already")
    );
    d.add(0, "a.bin", b"small", true).unwrap();
    assert_eq!(d.read(0, "a.bin").unwrap(), b"small");
    d.remove(0, "a.bin").unwrap();
    assert!(d.list(0).unwrap().is_empty());
    // The FATs are empty again (the cleared directory entry and the data stay as they were left).
    let fat = SECTOR + SECTOR; // reserved sector, then the first FAT
    assert_eq!(&d.bytes()[fat..fat + 4096], &before[fat..fat + 4096]);
}

#[test]
fn a_full_disk_is_an_error_and_changes_nothing() {
    let mut d = disk(4, 3);
    let size = d.bytes().len();
    let err = d.add(0, "huge.bin", &vec![0u8; size], false).unwrap_err();
    assert!(err.contains("full"), "{err}");
    assert!(d.list(0).unwrap().is_empty());
}

#[test]
fn names_that_are_not_8_3_are_refused() {
    let mut d = disk(8, 7);
    for bad in [
        "",
        ".x",
        "toolongname.txt",
        "a.toolong",
        "sp ace.txt",
        "a/b.txt",
        "..",
        "a\\b",
    ] {
        assert!(d.add(0, bad, b"x", false).is_err(), "{bad:?}");
    }
}

#[test]
fn an_extract_name_with_dotdot_is_refused() {
    for bad in ["../x", "..\\x", "/etc/passwd", "C:\\x", "a/b", ".."] {
        assert!(rcdisk::hd::safe_file_name(bad).is_err(), "{bad:?}");
    }
    assert_eq!(
        rcdisk::hd::safe_file_name("HELLO.TXT").unwrap(),
        "HELLO.TXT"
    );
}

/// A root sector whose extended partition links to itself, and one whose chain is endless.
#[test]
fn an_xgm_chain_loop_is_an_error() {
    let mut raw = vec![0u8; 64 * SECTOR];
    raw[0x1C2..0x1C6].copy_from_slice(&64u32.to_be_bytes());
    // A standard partition and an extended one at sector 10.
    let mut header = |sector: usize, slot: usize, id: &[u8; 3], start: u32, size: u32| {
        let at = sector * SECTOR + 0x1C6 + 12 * slot;
        raw[at] = 1;
        raw[at + 1..at + 4].copy_from_slice(id);
        raw[at + 4..at + 8].copy_from_slice(&start.to_be_bytes());
        raw[at + 8..at + 12].copy_from_slice(&size.to_be_bytes());
    };
    header(0, 0, b"GEM", 1, 8);
    header(0, 1, b"XGM", 10, 20);
    // Sector 10: a partition and a link to sector 10 again (offset 0 from the first extended partition).
    header(10, 0, b"GEM", 1, 4);
    header(10, 1, b"XGM", 0, 20);
    let d = HardDisk::parse(raw.clone()).unwrap();
    let err = d.partitions().unwrap_err();
    assert!(err.contains("link back"), "{err}");
    // A chain that never repeats but is too deep is refused too: sector 10 + n links onward.
    let mut long = vec![0u8; 100 * SECTOR];
    long[0x1C2..0x1C6].copy_from_slice(&100u32.to_be_bytes());
    let mut header = |sector: usize, slot: usize, id: &[u8; 3], start: u32, size: u32| {
        let at = sector * SECTOR + 0x1C6 + 12 * slot;
        long[at] = 1;
        long[at + 1..at + 4].copy_from_slice(id);
        long[at + 4..at + 8].copy_from_slice(&start.to_be_bytes());
        long[at + 8..at + 12].copy_from_slice(&size.to_be_bytes());
    };
    header(0, 0, b"GEM", 1, 8);
    header(0, 1, b"XGM", 10, 80);
    for k in 0..40usize {
        header(10 + k, 0, b"GEM", 1, 1);
        header(10 + k, 1, b"XGM", k as u32 + 1, 1);
    }
    let err = HardDisk::parse(long).unwrap().partitions().unwrap_err();
    assert!(err.contains("too deep"), "{err}");
}

#[test]
fn a_partition_beyond_the_disk_is_an_error() {
    let mut d = disk(8, 7);
    let mut raw = d.bytes().to_vec();
    raw[0x1CE..0x1D2].copy_from_slice(&u32::MAX.to_be_bytes());
    let err = HardDisk::parse(raw).unwrap().partitions().unwrap_err();
    assert!(err.contains("beyond the disk"), "{err}");
    // A start that overflows, a size of zero, an unknown id.
    for (at, bytes) in [(0x1CA, u32::MAX.to_be_bytes()), (0x1CE, 0u32.to_be_bytes())] {
        let mut raw = d.bytes().to_vec();
        raw[at..at + 4].copy_from_slice(&bytes);
        assert!(HardDisk::parse(raw).unwrap().partitions().is_err());
    }
    let mut raw = d.bytes().to_vec();
    raw[0x1C7..0x1CA].copy_from_slice(b"???");
    assert!(HardDisk::parse(raw).unwrap().partitions().is_err());
    d.add(0, "x.txt", b"x", false).unwrap();
}

#[test]
fn a_cyclic_fat16_chain_is_an_error() {
    let mut d = disk(8, 7);
    d.add(0, "loop.bin", &vec![1u8; 5000], false).unwrap();
    let mut raw = d.bytes().to_vec();
    // Cluster 2's FAT entry points to 3 and 3's back to 2, in the first FAT (after the reserved sector).
    let fat = SECTOR + SECTOR;
    raw[fat + 4..fat + 6].copy_from_slice(&3u16.to_le_bytes());
    raw[fat + 6..fat + 8].copy_from_slice(&2u16.to_le_bytes());
    let err = HardDisk::parse(raw)
        .unwrap()
        .read(0, "LOOP.BIN")
        .unwrap_err();
    assert!(err.contains("loops"), "{err}");
}

#[test]
fn sizes_the_format_cannot_hold_are_errors() {
    assert!(
        HardDisk::create(100, 99).unwrap_err().contains("at least"),
        "tiny"
    );
    assert!(
        HardDisk::create(10, 10).is_err(),
        "no room for the root sector"
    );
    assert!(HardDisk::create(0, 0).is_err());
    assert!(
        HardDisk::create(2 * 512 * 1024 * 1024 / 512, 1000).is_err(),
        "over the image limit"
    );
    assert!(HardDisk::parse(vec![0; 100]).is_err());
    assert!(HardDisk::parse(vec![0; 513]).is_err());
    assert_eq!(parse_size("8M").unwrap(), 8 * MIB);
    assert_eq!(parse_size("512k").unwrap(), 512 << 10);
    assert_eq!(parse_size("1G").unwrap(), 1 << 30);
    assert_eq!(parse_size("4096").unwrap(), 4096);
    assert!(parse_size("x").is_err() && parse_size("").is_err());
}

/// Corrupt images: nothing panics, whatever the damage.
#[test]
fn hostile_images_never_panic() {
    let mut state = 0x0123_4567_89AB_CDEFu64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut good = disk(8, 7);
    good.add(0, "a.txt", &vec![9u8; 4000], false).unwrap();
    good.add(0, "b.txt", &[8u8; 100], false).unwrap();
    let image = good.into_bytes();
    for round in 0..600
        * std::env::var("RC_SWEEP")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1)
    {
        let mut raw = image.clone();
        for _ in 0..(1 + next() % 8) {
            // Mostly the root sector, the boot sector, the FATs and the directory: where the structure is.
            let region = match next() % 4 {
                0 => 0..SECTOR,
                1 => SECTOR..2 * SECTOR,
                2 => 2 * SECTOR..40 * SECTOR,
                _ => 0..raw.len(),
            };
            let at = region.start + (next() as usize % region.len());
            raw[at] = next() as u8;
        }
        let Ok(d) = HardDisk::parse(raw) else {
            continue;
        };
        if let Ok(parts) = d.partitions() {
            for i in 0..parts.len() {
                let _ = d.list(i);
                let _ = d.read(i, "A.TXT");
                let mut d2 = HardDisk::parse(d.bytes().to_vec()).unwrap();
                let _ = d2.add(i, "new.bin", &vec![1u8; 3000], true);
                let _ = d2.remove(i, "B.TXT");
            }
        }
        let _ = round;
    }
}
