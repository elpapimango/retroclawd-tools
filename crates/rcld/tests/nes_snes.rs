//! The NES and Super Nintendo targets: built images re-read with the format crates, vectors from labels, banks and
//! mappings placing code where the hardware looks for it.

use rcld::layout;
use sfc::{Coprocessor, Mapping, Sfc};
use std::path::{Path, PathBuf};

struct Project(PathBuf);

impl Project {
    fn new(tag: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("rcld-nes-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Project(dir)
    }
    fn file(&self, name: &str, text: &str) -> &Project {
        std::fs::write(self.0.join(name), text).unwrap();
        self
    }
    fn bytes(&self, name: &str, b: &[u8]) -> &Project {
        std::fs::write(self.0.join(name), b).unwrap();
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

const NES_MAIN: &str = "\
.org $8000
reset:  sei
        cld
        ldx #$40
        stx $4017
forever: jmp forever
nmi:    rti
irq:    rti
";

#[test]
fn a_flat_nes_cartridge_is_32_kb_with_the_vectors_at_the_end() {
    let p = Project::new("flat");
    p.file("main.asm", NES_MAIN);
    let b = p
        .build(
            "machine = \"nes\"\n[[input]]\nasm = \"main.asm\"\n[options]\nmirroring = \"vertical\"\n",
            "game.nes",
        )
        .unwrap();
    let cart = ines::Ines::parse(&b.bytes).unwrap();
    assert_eq!(cart.prg_rom.len(), 0x8000);
    assert_eq!(cart.mapper, 0);
    assert_eq!(cart.mirroring, ines::Mirroring::Vertical);
    assert!(cart.chr_rom.is_empty()); // no CHR file: CHR RAM
    assert_eq!(&cart.prg_rom[..3], &[0x78, 0xD8, 0xA2]);
    // reset = $8000, nmi/irq = the rti's
    let v = &cart.prg_rom[0x8000 - 6..];
    assert_eq!(u16::from_le_bytes([v[2], v[3]]), 0x8000);
    let nmi = u16::from_le_bytes([v[0], v[1]]);
    assert_eq!(cart.prg_rom[nmi as usize - 0x8000], 0x40);
    assert!(b.notes.is_empty(), "{:?}", b.notes);
}

#[test]
fn code_only_in_the_upper_half_makes_a_16_kb_cartridge() {
    let p = Project::new("nrom128");
    p.file(
        "main.asm",
        ".org $C000\nreset: jmp reset\nnmi: rti\nirq: rti\n",
    );
    let b = p
        .build(
            "machine = \"nes\"\n[[input]]\nasm = \"main.asm\"\n",
            "g.nes",
        )
        .unwrap();
    let cart = ines::Ines::parse(&b.bytes).unwrap();
    assert_eq!(cart.prg_rom.len(), 0x4000);
    assert_eq!(&cart.prg_rom[..3], &[0x4C, 0x00, 0xC0]);
}

#[test]
fn a_banked_cartridge_places_banks_and_puts_the_vectors_in_the_last() {
    let p = Project::new("banked");
    p.file(
        "main.asm",
        ".bank 0\n.org $8000\nbank0: .byte $B0\n.bank 1\n.org $8000\nbank1: .byte $B1\n.bank 3\n.org $C000\nreset: jmp reset\nnmi: rti\nirq: rti\n",
    );
    let b = p
        .build(
            "machine = \"nes\"\n[[input]]\nasm = \"main.asm\"\n[options]\nmapper = 2\n",
            "g.nes",
        )
        .unwrap();
    let cart = ines::Ines::parse(&b.bytes).unwrap();
    assert_eq!(cart.mapper, 2);
    assert_eq!(cart.prg_rom.len(), 4 * 0x4000);
    assert_eq!(cart.prg_rom[0], 0xB0);
    assert_eq!(cart.prg_rom[0x4000], 0xB1);
    assert_eq!(
        &cart.prg_rom[3 * 0x4000..3 * 0x4000 + 3],
        &[0x4C, 0x00, 0xC0]
    );
    let v = &cart.prg_rom[4 * 0x4000 - 6..];
    assert_eq!(u16::from_le_bytes([v[2], v[3]]), 0xC000);
}

#[test]
fn chr_and_battery_and_nes2_options() {
    let p = Project::new("chr");
    p.file("main.asm", NES_MAIN)
        .bytes("tiles.chr", &vec![0x5A; 8192]);
    let b = p
        .build(
            "machine = \"nes\"\n[[input]]\nasm = \"main.asm\"\n[options]\nchr = \"tiles.chr\"\nbattery = true\nmapper = 4\nsubmapper = 1\n",
            "g.nes",
        )
        .unwrap();
    let cart = ines::Ines::parse(&b.bytes).unwrap();
    assert!(cart.nes2);
    assert_eq!(cart.submapper, 1);
    assert_eq!(cart.chr_rom.len(), 8192);
    assert!(cart.battery);
    assert_eq!(cart.prg_nvram, 8192);
    assert_eq!(cart.chr_ram, 0);
}

#[test]
fn nes_mistakes_are_named() {
    let p = Project::new("bad");
    p.file("low.asm", ".org $6000\nreset: rts\n")
        .file("novec.asm", ".org $8000\nstart_: rts\n")
        .file(
            "over.asm",
            ".org $8000\nreset: .byte 1\n.org $8000\n.byte 2\n",
        );
    let e = p
        .build("machine = \"nes\"\n[[input]]\nasm = \"low.asm\"\n", "g.nes")
        .unwrap_err();
    assert!(e.contains("below $8000"), "{e}");
    let e = p
        .build(
            "machine = \"nes\"\n[[input]]\nasm = \"novec.asm\"\n",
            "g.nes",
        )
        .unwrap_err();
    assert!(e.contains("no reset vector"), "{e}");
    let e = p
        .build(
            "machine = \"nes\"\n[[input]]\nasm = \"over.asm\"\n",
            "g.nes",
        )
        .unwrap_err();
    assert!(
        e.contains("overlap") || e.contains("segments overlap"),
        "{e}"
    );
    let e = p
        .build("machine = \"nes\"\n[[input]]\nasm = \"low.asm\"\n", "g.prg")
        .unwrap_err();
    assert!(e.contains(".nes"), "{e}");
}

const SNES_MAIN: &str = "\
.org $808000
reset:  sei
        clc
        xce
        rep #$30
        lda #$1234
        jml main
main:   wai
        bra main
nmi:    rti
irq:    rti
";

#[test]
fn a_lorom_cartridge_has_its_header_vectors_and_checksum() {
    let p = Project::new("lorom");
    p.file("main.asm", SNES_MAIN);
    let b = p
        .build(
            "machine = \"snes\"\nname = \"HELLO SNES\"\n[[input]]\nasm = \"main.asm\"\n",
            "game.sfc",
        )
        .unwrap();
    let cart = Sfc::parse(&b.bytes).unwrap();
    assert_eq!(cart.mapping, Mapping::LoRom);
    assert_eq!(cart.rom.len(), 0x8000);
    assert_eq!(cart.header.title, "HELLO SNES");
    assert!(cart.checksum_ok());
    assert_eq!(cart.header.vectors.reset, 0x8000);
    assert_eq!(&cart.rom[..3], &[0x78, 0x18, 0xFB]);
    let nmi = cart.header.vectors.native_nmi as usize;
    assert_eq!(cart.rom[nmi - 0x8000], 0x40);
    assert_eq!(cart.header.coprocessor(), Coprocessor::None);
    // the jml went through the 24-bit assembler: main is $80800C
    assert_eq!(&cart.rom[8..12], &[0x5C, 0x0C, 0x80, 0x80]);
    assert!(b.notes.iter().all(|n| !n.contains("NMI")), "{:?}", b.notes);
    assert!(b.notes.iter().any(|n| n.contains("COP")), "{:?}", b.notes);
}

#[test]
fn banks_land_where_the_mapping_says() {
    let p = Project::new("banks");
    p.file(
        "main.asm",
        ".org $808000\nreset: jmp reset\nnmi: rti\nirq: rti\n.org $828000\nbank2: .byte $B2\n.org $9F9000\nbank3f: .byte $3F\n",
    );
    let b = p
        .build(
            "machine = \"snes\"\n[[input]]\nasm = \"main.asm\"\n[options]\nmapping = \"lorom\"\n",
            "g.sfc",
        )
        .unwrap();
    let cart = Sfc::parse(&b.bytes).unwrap();
    // bank $82 -> ROM offset 2 * $8000; bank $9F -> $1F * $8000 + $1000
    assert_eq!(cart.rom[2 * 0x8000], 0xB2);
    assert_eq!(cart.rom[0x1F * 0x8000 + 0x1000], 0x3F);
    assert_eq!(cart.rom.len(), 0x20 * 0x8000);

    let p = Project::new("hirom");
    p.file(
        "main.asm",
        ".org $C00000\nreset: jmp $8000\nnmi: rti\nirq: rti\n.org $C10000\nsecond: .byte $C1\n",
    );
    let b = p
        .build(
            "machine = \"snes\"\n[[input]]\nasm = \"main.asm\"\n[options]\nmapping = \"hirom\"\nfast = true\n",
            "g.sfc",
        )
        .unwrap_err();
    // the reset code at $C000xx is below $8000 in its bank: the CPU's reset vector is 16 bits in bank 0
    assert!(b.contains("upper half of a bank"), "{b}");
}

#[test]
fn hirom_with_the_code_in_the_mirror_of_bank_c0() {
    let p = Project::new("hirom2");
    p.file(
        "main.asm",
        ".org $C08000\nreset: jmp reset\nnmi: rti\nirq: rti\n.org $C10000\nsecond: .byte $C1\n",
    );
    let b = p
        .build(
            "machine = \"snes\"\n[[input]]\nasm = \"main.asm\"\n[options]\nmapping = \"hirom\"\nfast = true\nsize = 131072\n",
            "g.sfc",
        )
        .unwrap();
    let cart = Sfc::parse(&b.bytes).unwrap();
    assert_eq!(cart.mapping, Mapping::HiRom);
    assert!(cart.header.fast_rom());
    assert_eq!(cart.rom.len(), 0x20000);
    assert_eq!(cart.rom[0x8000], 0x4C);
    assert_eq!(cart.rom[0x10000], 0xC1);
    assert!(cart.checksum_ok());
}

#[test]
fn a_header_the_sources_write_is_kept_and_its_checksum_fixed() {
    let p = Project::new("ownhdr");
    // LoROM header at $FFC0 of bank 0 (ROM offset $7FC0)
    let src = "\
.org $808000
reset: jmp reset
nmi: rti
.org $80FFC0
.byte \"OWN HEADER           \"
.byte $20, $00, $05, $00, $00, $00, $00
.word $0000, $0000
.fill 4, 0
.word 0,0,0,nmi&$FFFF,0,0
.fill 4, 0
.word 0,0,0,nmi&$FFFF,reset&$FFFF,0
";
    p.file("main.asm", src);
    let b = p
        .build(
            "machine = \"snes\"\n[[input]]\nasm = \"main.asm\"\n",
            "g.sfc",
        )
        .unwrap();
    let cart = Sfc::parse(&b.bytes).unwrap();
    assert_eq!(cart.header.title, "OWN HEADER");
    assert!(cart.checksum_ok());
    assert!(b.map.contains("written by the sources"), "{}", b.map);
}

#[test]
fn snes_mistakes_are_named() {
    let p = Project::new("sbad");
    p.file("low.asm", ".org $000100\nreset: rts\n")
        .file("cross.asm", ".org $80FFF0\nreset: .fill 32, 1\n")
        .file(
            "over.asm",
            ".org $808000\nreset: .byte 1\n.org $008000\n.byte 2\n",
        );
    let e = p
        .build(
            "machine = \"snes\"\n[[input]]\nasm = \"low.asm\"\n",
            "g.sfc",
        )
        .unwrap_err();
    assert!(e.contains("not in the ROM window"), "{e}");
    let e = p
        .build(
            "machine = \"snes\"\n[[input]]\nasm = \"cross.asm\"\n",
            "g.sfc",
        )
        .unwrap_err();
    assert!(e.contains("crosses the end of a bank"), "{e}");
    let e = p
        .build(
            "machine = \"snes\"\n[[input]]\nasm = \"over.asm\"\n",
            "g.sfc",
        )
        .unwrap_err();
    assert!(e.contains("overlaps"), "{e}");
    let e = p
        .build(
            "machine = \"snes\"\n[[input]]\nasm = \"low.asm\"\n[options]\nmapping = \"flat\"\n",
            "g.sfc",
        )
        .unwrap_err();
    assert!(e.contains("lorom, hirom or exhirom"), "{e}");
    assert!(p.path().exists());
}
