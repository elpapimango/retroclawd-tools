//! The Mega-CD target: the system area fields and the ISO 9660 volume.

use rcld::layout;
use std::path::PathBuf;

struct Project(PathBuf);

impl Project {
    fn new(tag: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("rcld-segacd-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Project(dir)
    }
    fn file(&self, name: &str, text: &[u8]) -> &Project {
        std::fs::write(self.0.join(name), text).unwrap();
        self
    }
    fn build(&self, layout_text: &str) -> Result<rcld::Build, String> {
        let layout = layout::parse(layout_text)?;
        rcld::build(&layout, &self.0, &self.0.join("disc.iso"), &[])
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const IP: &str = "        .org $FF0584\nstart:  bra.s start\n";
const SP: &str = "        .org $6000\n        dc.b \"MAIN-TEST\",0,0,0\n        dc.b 0,1,0,0\n        dc.l 0,0,$20,0\nentries: dc.w init-entries,0\ninit:   rts\n";
const LAYOUT: &str = "machine = \"segacd\"\nname = \"testdisc\"\n[options]\nregion = \"E\"\n[[input]]\nasm = [\"ip.s\"]\n[[input]]\nasm = [\"sp.s\"]\n";

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

#[test]
fn the_system_area_has_the_header_the_ip_and_the_sp() {
    let p = Project::new("sys");
    p.file("ip.s", IP.as_bytes()).file("sp.s", SP.as_bytes());
    let b = p.build(LAYOUT).unwrap();
    let d = &b.bytes;
    assert_eq!(&d[..16], b"SEGADISCSYSTEM  ");
    assert_eq!(&d[0x10..0x1B], b"TESTDISC   ");
    assert_eq!(&d[0x100..0x110], b"SEGA GENESIS    ");
    assert_eq!(d[0x1F0], b'E');
    // the IP starts at $200 and runs to the sector end, the SP follows
    assert_eq!(be32(d, 0x30), 0x200);
    assert_eq!(be32(d, 0x34), 0x600);
    assert_eq!(be32(d, 0x40), 0x800);
    assert_eq!(be32(d, 0x44), 0x800);
    // bra.s start = 60FE at $FF0584 -> disc offset $784
    assert_eq!(&d[0x784..0x786], &[0x60, 0xFE]);
    // the SP module header and its first word
    assert_eq!(&d[0x800..0x808], b"MAIN-TES");
    assert!(
        b.notes.iter().any(|n| n.contains("security")),
        "{:?}",
        b.notes
    );
    assert_eq!(d.len() % 2048, 0);
}

#[test]
fn files_go_in_an_iso9660_volume() {
    let p = Project::new("iso");
    p.file("ip.s", IP.as_bytes()).file("sp.s", SP.as_bytes());
    p.file("level1.dat", &[7u8; 5000]).file("a.txt", b"hello");
    let b = p
        .build(&format!(
            "{LAYOUT}[[file]]\npath = \"level1.dat\"\n[[file]]\npath = \"a.txt\"\n"
        ))
        .unwrap();
    let d = &b.bytes;
    let sector = |n: usize| &d[n * 2048..(n + 1) * 2048];
    assert_eq!(&sector(16)[1..6], b"CD001");
    assert_eq!(sector(16)[0], 1);
    assert_eq!(sector(17)[0], 255);
    // the root directory record in the volume descriptor points at the directory
    let root = u32::from_le_bytes(sector(16)[158..162].try_into().unwrap()) as usize;
    let dir = sector(root);
    // records: ., .., then A.TXT;1 and LEVEL1.DAT;1 (sorted)
    let mut at = 0;
    let mut names = Vec::new();
    while dir[at] != 0 {
        let len = dir[at] as usize;
        let n = dir[at + 32] as usize;
        let extent = u32::from_le_bytes(dir[at + 2..at + 6].try_into().unwrap()) as usize;
        let size = u32::from_le_bytes(dir[at + 10..at + 14].try_into().unwrap()) as usize;
        names.push((
            String::from_utf8_lossy(&dir[at + 33..at + 33 + n]).into_owned(),
            extent,
            size,
        ));
        at += len;
    }
    assert_eq!(names.len(), 4, "{names:?}");
    assert_eq!(names[2].0, "A.TXT;1");
    assert_eq!(names[3].0, "LEVEL1.DAT;1");
    let (_, e, s) = &names[2];
    assert_eq!(&d[e * 2048..e * 2048 + *s], b"hello");
    let (_, e, s) = &names[3];
    assert_eq!(*s, 5000);
    assert!(d[e * 2048..e * 2048 + 5000].iter().all(|b| *b == 7));
    assert_eq!(d.len(), (names[3].1 + 3) * 2048);
}

#[test]
fn mistakes_are_named() {
    let p = Project::new("bad");
    p.file("ip.s", IP.as_bytes()).file("sp.s", SP.as_bytes());
    p.file("low.s", b"        .org $FF0000\n        nop\n");
    let e = p
        .build("machine = \"segacd\"\n[[input]]\nasm = [\"low.s\"]\n[[input]]\nasm = [\"sp.s\"]\n")
        .unwrap_err();
    assert!(e.contains("security code"), "{e}");
    let e = p
        .build("machine = \"segacd\"\n[[input]]\nasm = [\"sp.s\"]\n")
        .unwrap_err();
    assert!(e.contains("no initial program"), "{e}");
    let e = p
        .build("machine = \"segacd\"\n[[input]]\nasm = [\"ip.s\"]\n")
        .unwrap_err();
    assert!(e.contains("no sub program"), "{e}");
    p.file("sec.bin", &[1u8; 10]);
    let e = p
        .build(&format!(
            "{}\n",
            LAYOUT.replace("region = \"E\"", "region = \"E\"\nsecurity = \"sec.bin\"")
        ))
        .unwrap_err();
    assert!(e.contains("security code is"), "{e}");
    p.file("big.s", b"        .org $FF0584\n        ds.b 40000\n");
    let e = p
        .build("machine = \"segacd\"\n[[input]]\nasm = [\"big.s\"]\n[[input]]\nasm = [\"sp.s\"]\n")
        .unwrap_err();
    assert!(e.contains("system area"), "{e}");
    p.file("bad name.dat", b"x");
    let e = p
        .build(&format!("{LAYOUT}[[file]]\npath = \"bad name.dat\"\n"))
        .unwrap_err();
    assert!(e.contains("ISO 9660"), "{e}");
}

#[test]
fn the_security_code_is_placed_when_given() {
    let p = Project::new("sec");
    p.file("ip.s", IP.as_bytes()).file("sp.s", SP.as_bytes());
    let code: Vec<u8> = (0..0x584usize).map(|i| (i % 251) as u8).collect();
    p.file("sec.bin", &code);
    let b = p
        .build(&LAYOUT.replace("region = \"E\"", "region = \"E\"\nsecurity = \"sec.bin\""))
        .unwrap();
    assert_eq!(&b.bytes[0x200..0x784], &code[..]);
    assert!(!b.notes.iter().any(|n| n.contains("security")));
}

#[test]
fn the_security_code_can_come_from_a_disc_image() {
    let p = Project::new("secdisc");
    p.file("ip.s", IP.as_bytes()).file("sp.s", SP.as_bytes());
    let code: Vec<u8> = (0..0x584usize).map(|i| (i % 249) as u8).collect();
    p.file("sec.bin", &code);
    let with = LAYOUT.replace("region = \"E\"", "region = \"E\"\nsecurity = \"sec.bin\"");
    let iso = p.build(&with).unwrap().bytes;
    // a 2048-byte sector image (.iso)
    p.file("game.iso", &iso);
    let b = p
        .build(&LAYOUT.replace("region = \"E\"", "region = \"E\"\nsecurity = \"game.iso\""))
        .unwrap();
    assert_eq!(&b.bytes[0x200..0x784], &code[..]);
    // a raw 2352-byte sector image (.bin): sync, header, the 2048 bytes, error correction
    let mut raw = vec![0u8; 2352 * 2];
    raw[0] = 0;
    raw[1..11].fill(0xFF);
    raw[15] = 1;
    raw[16..16 + 2048].copy_from_slice(&iso[..2048]);
    p.file("game.bin", &raw);
    let b = p
        .build(&LAYOUT.replace("region = \"E\"", "region = \"E\"\nsecurity = \"game.bin\""))
        .unwrap();
    assert_eq!(&b.bytes[0x200..0x784], &code[..]);
    // a file that is neither a code nor a Mega-CD disc
    p.file("other.bin", &vec![0u8; 4096]);
    let e = p
        .build(&LAYOUT.replace("region = \"E\"", "region = \"E\"\nsecurity = \"other.bin\""))
        .unwrap_err();
    assert!(e.contains("SEGADISCSYSTEM"), "{e}");
}
