//! Atari ST `.st` floppies: FAT12 per Microsoft's *FAT: General Overview of On-Disk Format* (v1.03), with the BPB values
//! of *Atari ST Internals* p.178 and the executable boot sector rule of p.179.

use rcdisk::{AddOptions, ReadOptions};

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn add(disk: &mut dyn rcdisk::Disk, name: &str, data: &[u8]) -> Result<String, String> {
    disk.add(name, data, &AddOptions::default())
}

#[test]
fn new_720k_has_the_internals_bpb() {
    let disk = rcdisk::create("st", None, None).unwrap();
    let b = disk.to_bytes();
    assert_eq!(b.len(), 737_280);
    assert_eq!(le16(&b, 11), 512, "bytes per sector");
    assert_eq!(b[13], 2, "sectors per cluster");
    assert_eq!(le16(&b, 14), 1, "reserved sectors");
    assert_eq!(b[16], 2, "FATs");
    assert_eq!(le16(&b, 17), 112, "directory entries");
    assert_eq!(le16(&b, 19), 1440, "sectors");
    assert_eq!(b[21], 0xF9, "media");
    assert_eq!(le16(&b, 22), 5, "sectors per FAT");
    assert_eq!(le16(&b, 24), 9, "sectors per track");
    assert_eq!(le16(&b, 26), 2, "sides");
    // FAT[0] holds the media byte with the other bits set, FAT[1] an end-of-chain mark, in both FATs.
    for fat in [512, 512 * 6] {
        assert_eq!(&b[fat..fat + 3], &[0xF9, 0xFF, 0xFF]);
    }
    assert_eq!(
        disk.free_blocks(),
        711,
        "clusters free (Internals: numcl 711)"
    );
    let ss = rcdisk::create("st-ss", None, None).unwrap().to_bytes();
    assert_eq!(ss.len(), 368_640);
    assert_eq!((le16(&ss, 19), ss[21], le16(&ss, 26)), (720, 0xF8, 1));
}

#[test]
fn add_then_list_and_extract_round_trip() {
    let mut disk = rcdisk::create("st", None, None).unwrap();
    let big: Vec<u8> = (0..3000).map(|i| (i * 7) as u8).collect();
    assert_eq!(
        add(disk.as_mut(), "hello.txt", b"Hello, ST\r\n").unwrap(),
        "HELLO.TXT"
    );
    assert_eq!(add(disk.as_mut(), "big.bin", &big).unwrap(), "BIG.BIN");
    add(disk.as_mut(), "empty", b"").unwrap();
    // Open the bytes again, as a fresh image would be.
    let disk = rcdisk::open("x.st", &disk.to_bytes()).unwrap();
    let names: Vec<String> = disk.files().into_iter().map(|f| f.name).collect();
    assert_eq!(names, ["HELLO.TXT", "BIG.BIN", "EMPTY"]);
    let got = disk.read("BIG.BIN", &ReadOptions::default()).unwrap();
    assert_eq!(got[0].data, big);
    assert_eq!(
        disk.read("hello.*", &ReadOptions::default()).unwrap()[0].data,
        b"Hello, ST\r\n"
    );
    assert!(
        disk.read("EMPTY", &ReadOptions::default()).unwrap()[0]
            .data
            .is_empty()
    );
    assert_eq!(
        disk.free_blocks(),
        711 - 1 - 3,
        "3000 bytes take three 1024-byte clusters"
    );
    assert!(disk.check().errors.is_empty(), "{:?}", disk.check());
}

#[test]
fn boot_code_makes_the_sector_executable() {
    let disk = rcdisk::create("st", None, None).unwrap();
    let code = [0x60u8, 0xFE];
    let b = rcdisk::st::with_boot_code(&disk.to_bytes(), &code).unwrap();
    let sum = b[..512].chunks(2).fold(0u16, |s, w| {
        s.wrapping_add(u16::from_be_bytes([w[0], w[1]]))
    });
    assert_eq!(sum, 0x1234);
    assert_eq!(&b[0..2], &[0x60, 0x1C], "BRA.S to offset 30");
    assert_eq!(&b[30..32], &code);
    assert_eq!(le16(&b, 11), 512, "the BPB is kept");
    assert!(
        rcdisk::st::with_boot_code(&disk.to_bytes(), &[0; 500]).is_err(),
        "too long"
    );
}

#[test]
fn disk_full_is_an_error() {
    let mut disk = rcdisk::create("st-ss", None, None).unwrap();
    let free = disk.free_blocks() as usize;
    let err = add(disk.as_mut(), "huge.bin", &vec![0; (free + 1) * 1024]).unwrap_err();
    assert!(err.contains("full") || err.contains("space"), "{err}");
    assert_eq!(disk.free_blocks() as usize, free, "nothing half-written");
}

/// A file whose first cluster is `start` and whose FAT entries are written by `chain`.
fn hostile(start: u16, size: u32, chain: &[(u16, u16)]) -> Vec<u8> {
    let mut disk = rcdisk::create("st", None, None).unwrap();
    add(disk.as_mut(), "a.bin", &[1; 10]).unwrap();
    let mut b = disk.to_bytes();
    let dir = 512 * 11;
    b[dir + 26..dir + 28].copy_from_slice(&start.to_le_bytes());
    b[dir + 28..dir + 32].copy_from_slice(&size.to_le_bytes());
    for &(n, value) in chain {
        for fat in [512usize, 512 * 6] {
            let at = fat + n as usize + n as usize / 2;
            let word = u16::from_le_bytes([b[at], b[at + 1]]);
            let word = if n & 1 == 1 {
                (word & 0x000F) | (value << 4)
            } else {
                (word & 0xF000) | (value & 0x0FFF)
            };
            b[at..at + 2].copy_from_slice(&word.to_le_bytes());
        }
    }
    b
}

#[test]
fn cyclic_chain_is_an_error() {
    let b = hostile(2, 5000, &[(2, 3), (3, 2)]);
    let disk = rcdisk::open("x.st", &b).unwrap();
    assert!(disk.read("A.BIN", &ReadOptions::default()).is_err());
    assert!(!disk.check().errors.is_empty());
}

#[test]
fn out_of_range_chain_is_an_error() {
    let b = hostile(2, 5000, &[(2, 4000)]);
    let disk = rcdisk::open("x.st", &b).unwrap();
    assert!(disk.read("A.BIN", &ReadOptions::default()).is_err());
    let b = hostile(900, 10, &[]);
    assert!(
        rcdisk::open("x.st", &b)
            .unwrap()
            .read("A.BIN", &ReadOptions::default())
            .is_err()
    );
}

#[test]
fn size_beyond_chain_is_an_error() {
    let b = hostile(2, 5000, &[(2, 0xFFF)]);
    let disk = rcdisk::open("x.st", &b).unwrap();
    let err = disk.read("A.BIN", &ReadOptions::default()).unwrap_err();
    assert!(err.contains("5000") || err.contains("size"), "{err}");
}

#[test]
fn bad_names_are_refused() {
    let mut disk = rcdisk::create("st", None, None).unwrap();
    assert!(add(disk.as_mut(), "toolongname.txt", b"x").is_err());
    assert!(add(disk.as_mut(), "a.text", b"x").is_err());
    assert!(add(disk.as_mut(), "a+b.txt", b"x").is_err());
    assert_eq!(
        add(disk.as_mut(), "Mixed.Txt", b"x").unwrap(),
        "MIXED.TXT",
        "upper case on disk"
    );
    assert!(
        add(disk.as_mut(), "MIXED.TXT", b"y").is_err(),
        "names are unique"
    );
}

#[test]
fn st_images_are_recognised() {
    let disk = rcdisk::create("st", None, None).unwrap();
    assert_eq!(rcdisk::sniff("game.st", &disk.to_bytes()), Ok("st"));
    assert_eq!(
        rcdisk::open("game.st", &disk.to_bytes()).unwrap().format(),
        "st"
    );
    assert!(rcdisk::open("game.st", &[0u8; 1000]).is_err());
}
