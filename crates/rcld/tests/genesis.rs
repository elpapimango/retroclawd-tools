//! The Mega Drive target: generated vectors and header, the checksum, hand-written headers kept.

use rcld::layout;
use std::path::PathBuf;

struct Project(PathBuf);

impl Project {
    fn new(tag: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("rcld-md-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Project(dir)
    }
    fn file(&self, name: &str, text: &str) -> &Project {
        std::fs::write(self.0.join(name), text).unwrap();
        self
    }
    fn build(&self, layout_text: &str, out: &str) -> Result<rcld::Build, String> {
        let layout = layout::parse(layout_text)?;
        rcld::build(&layout, &self.0, &self.0.join(out), &[])
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const MAIN: &str = "\
        .org $200
reset:  move.w  #$2700,sr
        bra.s   reset
vint:   rte
";

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[test]
fn vectors_and_header_are_generated_and_the_checksum_is_right() {
    let p = Project::new("gen");
    p.file("main.s", MAIN);
    let b = p
        .build(
            "machine = \"genesis\"\nname = \"HELLO\"\n[[input]]\nasm = \"main.s\"\n",
            "hello.md",
        )
        .unwrap();
    let rom = &b.bytes;
    assert_eq!(rom.len(), 0x20000);
    assert_eq!(be32(rom, 0), 0x0100_0000);
    assert_eq!(be32(rom, 4), 0x200);
    assert_eq!(be32(rom, 30 * 4), 0x206);
    assert_eq!(&rom[0x100..0x110], b"SEGA MEGA DRIVE ");
    assert_eq!(&rom[0x120..0x125], b"HELLO");
    assert_eq!(be32(rom, 0x1A4), 0x1FFFF);
    let mut sum = 0u16;
    for w in rom[0x200..].chunks(2) {
        sum = sum.wrapping_add(u16::from_be_bytes([w[0], w[1]]));
    }
    assert_eq!(u16::from_be_bytes([rom[0x18E], rom[0x18F]]), sum);
    assert!(
        b.notes.iter().any(|n| n.contains("`hint`")),
        "{:?}",
        b.notes
    );
    assert!(!b.notes.iter().any(|n| n.contains("TMSS")));
}

#[test]
fn a_header_the_sources_write_is_kept() {
    let p = Project::new("own");
    let src = "\
        .org 0
        dc.l $00FFFE00, start
        ds.b $F8
        dc.b \"SEGA GENESIS    \"
        ds.b $F0
start:  bra.s start
";
    p.file("main.s", src);
    let b = p
        .build(
            "machine = \"genesis\"\n[[input]]\nasm = \"main.s\"\n",
            "g.bin",
        )
        .unwrap();
    assert_eq!(be32(&b.bytes, 0), 0x00FF_FE00);
    assert_eq!(&b.bytes[0x100..0x10C], b"SEGA GENESIS");
    assert!(b.map.contains("written by the sources"));
}

#[test]
fn genesis_mistakes_are_named() {
    let p = Project::new("bad");
    p.file("novec.s", "        .org $200\nfoo:    rts\n");
    let e = p
        .build(
            "machine = \"genesis\"\n[[input]]\nasm = \"novec.s\"\n",
            "g.md",
        )
        .unwrap_err();
    assert!(e.contains("no reset vector"), "{e}");
    let e = p
        .build(
            "machine = \"genesis\"\n[[input]]\nasm = \"novec.s\"\n",
            "g.prg",
        )
        .unwrap_err();
    assert!(e.contains(".md"), "{e}");
}
