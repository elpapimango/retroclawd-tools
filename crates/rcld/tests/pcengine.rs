//! The PC Engine target: banked HuC6280 sources become a HuCard image.

use rcld::layout;
use std::path::{Path, PathBuf};

struct Project(PathBuf);

impl Project {
    fn new(tag: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("rcld-pce-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Project(dir)
    }
    fn file(&self, name: &str, text: &str) -> &Project {
        std::fs::write(self.0.join(name), text).unwrap();
        self
    }
    fn path(&self) -> &Path {
        &self.0
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

const LAYOUT: &str = r#"
machine = "pcengine"
entry = "start"
[[input]]
asm = ["main.asm"]
"#;

const ONE_BANK: &str = "
    .org $E000
start:
    sei
    csh
    nop
loop:
    bra loop
irq:
    rti
";

#[test]
fn a_one_bank_program_becomes_a_128_kb_card_with_its_reset_vector() {
    let p = Project::new("one");
    p.file("main.asm", ONE_BANK);
    let built = p.build(LAYOUT, "game.pce").unwrap();
    assert_eq!(built.bytes.len(), 0x20000);
    assert_eq!(&built.bytes[..5], &[0x78, 0xD4, 0xEA, 0x80, 0xFE]);
    // reset vector at $FFFE = file offset $1FFE, pointing at `start` ($E000)
    assert_eq!(&built.bytes[0x1FFE..0x2000], &[0x00, 0xE0]);
    // vectors without a label point at the reset code, and say so
    assert_eq!(
        &built.bytes[0x1FF6..0x1FFE],
        &[0x00, 0xE0, 0x00, 0xE0, 0x00, 0xE0, 0x00, 0xE0]
    );
    assert!(
        built.notes.iter().any(|n| n.contains("irq1")),
        "{:?}",
        built.notes
    );
}

#[test]
fn labelled_vectors_are_written() {
    let p = Project::new("vec");
    p.file(
        "main.asm",
        "    .org $E000\nstart:\n    nop\nirq1:\n    rti\ntimer:\n    rti\nnmi:\n    rti\nirq2:\n    rti\n",
    );
    let built = p.build(LAYOUT, "game.pce").unwrap();
    let w = |at: usize| u16::from_le_bytes([built.bytes[at], built.bytes[at + 1]]);
    assert_eq!(w(0x1FFE), 0xE000);
    assert_eq!(w(0x1FF8), 0xE001); // irq1
    assert_eq!(w(0x1FFA), 0xE002); // timer
    assert_eq!(w(0x1FFC), 0xE003); // nmi
    assert_eq!(w(0x1FF6), 0xE004); // irq2
}

#[test]
fn sources_that_write_the_vectors_keep_them() {
    let p = Project::new("own");
    p.file(
        "main.asm",
        "    .org $E000\nstart:\n    nop\n    .org $FFF6\n    .word $E111,$E222,$E333,$E444,$E000\n",
    );
    let built = p.build(LAYOUT, "game.pce").unwrap();
    assert_eq!(
        &built.bytes[0x1FF6..0x2000],
        &[0x11, 0xE1, 0x22, 0xE2, 0x33, 0xE3, 0x44, 0xE4, 0x00, 0xE0]
    );
    assert!(built.notes.is_empty());
}

#[test]
fn bank_n_lands_at_n_times_8192() {
    let p = Project::new("banks");
    p.file(
        "main.asm",
        "    .org $E000\nstart:\n    nop\n    .bank 1\n    .org $6000\n    clx\n    .bank 2\n    .org $6000\n    cly\n    .bank 3\n    .org $4000\n    cla\n",
    );
    let built = p.build(LAYOUT, "game.pce").unwrap();
    assert_eq!(built.bytes[0x2000], 0x82);
    assert_eq!(built.bytes[0x4000], 0xC2);
    assert_eq!(built.bytes[0x6000], 0x62);
}

#[test]
fn a_binary_goes_in_its_bank() {
    let p = Project::new("bin");
    p.file("main.asm", ONE_BANK);
    std::fs::write(p.path().join("gfx.bin"), [1u8, 2, 3, 4]).unwrap();
    let layout = format!("{LAYOUT}\n[[input]]\nbin = \"gfx.bin\"\nbank = 5\nat = 0x4010\n");
    let built = p.build(&layout, "game.pce").unwrap();
    assert_eq!(&built.bytes[5 * 0x2000 + 0x10..][..4], &[1, 2, 3, 4]);
}

#[test]
fn bank_0_below_e000_is_refused() {
    let p = Project::new("low");
    p.file("main.asm", "    .org $4000\nstart:\n    nop\n");
    let err = p.build(LAYOUT, "game.pce").unwrap_err();
    assert!(err.contains("bank 0"), "{err}");
}

#[test]
fn overlapping_pieces_are_named() {
    let p = Project::new("overlap");
    p.file(
        "main.asm",
        "    .org $E000\nstart:\n    nop\n    .bank 1\n    .org $6000\n    nop\n",
    );
    std::fs::write(p.path().join("a.bin"), [0u8; 16]).unwrap();
    let layout = format!("{LAYOUT}\n[[input]]\nbin = \"a.bin\"\nbank = 1\nat = 0x6000\n");
    let err = p.build(&layout, "game.pce").unwrap_err();
    assert!(err.contains("overlaps"), "{err}");
}

#[test]
fn sizes_are_checked() {
    let p = Project::new("size");
    p.file("main.asm", ONE_BANK);
    let with = |size: &str| format!("{LAYOUT}\n[options]\nsize = {size}\n");
    assert_eq!(
        p.build(&with("393216"), "g.pce").unwrap().bytes.len(),
        393216
    );
    assert_eq!(
        p.build(&with("2621440"), "g.pce").unwrap().bytes.len(),
        2621440
    );
    assert!(p.build(&with("100000"), "g.pce").is_err());
    assert!(p.build(&with("8192"), "g.pce").is_ok());
    assert!(p.build(&with("65536"), "g.pce").is_ok());
}

#[test]
fn a_program_larger_than_the_size_says_so() {
    let p = Project::new("big");
    p.file(
        "main.asm",
        "    .org $E000\nstart:\n    nop\n    .bank 40\n    .org $6000\n    nop\n",
    );
    let err = p
        .build(&format!("{LAYOUT}\n[options]\nsize = 131072\n"), "g.pce")
        .unwrap_err();
    assert!(err.contains("options.size"), "{err}");
}

#[test]
fn the_extension_must_be_pce() {
    let p = Project::new("ext");
    p.file("main.asm", ONE_BANK);
    assert!(p.build(LAYOUT, "game.bin").is_err());
}

#[test]
fn no_reset_label_is_an_error() {
    let p = Project::new("noreset");
    p.file("main.asm", "    .org $E000\nfoo:\n    nop\n");
    let err = p
        .build(
            "machine = \"pcengine\"\n[[input]]\nasm = [\"main.asm\"]\n",
            "g.pce",
        )
        .unwrap_err();
    assert!(err.contains("reset"), "{err}");
}
