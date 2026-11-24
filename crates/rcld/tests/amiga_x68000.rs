//! The Amiga and X68000 targets: the 68000 assembler's relocatable program written as a hunk file / ADF / .X, and read
//! back through the format crates.

use amiga::adf::Adf;
use amiga::hunk::{Executable, Kind, Memory};
use rcld::layout;
use std::path::{Path, PathBuf};

struct Project(PathBuf);

impl Project {
    fn new(tag: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("rcld-68k-{tag}-{}", std::process::id()));
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

/// Text with a pointer to data, a pointer to bss and a pointer back into the text; data holding a pointer to text.
const PROGRAM: &str = "\
        .text
start:  lea     table,a0
        move.l  #counter,a1
        move.l  #start,d0
        rts
        .data
table:  dc.l    start
        dc.l    counter
        dc.b    1,2,3,4
        .bss
counter: ds.l   1
";

const LAYOUT: &str = "machine = \"amiga\"\nname = \"DEMO\"\n[[input]]\nasm = \"main.s\"\n";

#[test]
fn a_hunk_executable_has_a_hunk_per_section_and_relocations_into_the_right_ones() {
    let p = Project::new("hunk");
    p.file("main.s", PROGRAM);
    let b = p.build(LAYOUT, "demo").unwrap();
    let exe = Executable::parse(&b.bytes).unwrap();
    assert_eq!(exe.hunks.len(), 3);
    assert_eq!(exe.hunks[0].kind, Kind::Code);
    assert_eq!(exe.hunks[1].kind, Kind::Data);
    assert_eq!(exe.hunks[2].kind, Kind::Bss);
    assert_eq!(exe.hunks[2].size, 4);
    // text: `lea table,a0` is pc-relative here (no relocation); the two `move.l #label` hold absolute addresses:
    // one into bss (hunk 2, offset 0), one into text (hunk 0, offset 0)
    let targets: Vec<(u32, usize)> = exe.hunks[0]
        .relocs
        .iter()
        .map(|r| (r.target, r.offsets.len()))
        .collect();
    assert!(targets.contains(&(2, 1)), "{targets:?}");
    assert!(targets.contains(&(0, 1)), "{targets:?}");
    // data: dc.l start -> text, dc.l counter -> bss
    let data_targets: Vec<(u32, usize)> = exe.hunks[1]
        .relocs
        .iter()
        .map(|r| (r.target, r.offsets.len()))
        .collect();
    assert!(
        data_targets.contains(&(0, 1)) && data_targets.contains(&(2, 1)),
        "{data_targets:?}"
    );
    // loading adds each hunk's base: the data's pointer to counter is base(bss) + 0
    let bases = [0x1000, 0x2000, 0x3000];
    let loaded = exe.load(&bases).unwrap();
    assert_eq!(&loaded[1][0..4], &0x1000u32.to_be_bytes());
    assert_eq!(&loaded[1][4..8], &0x3000u32.to_be_bytes());
    assert!(b.symbols_text.contains("start"), "{}", b.symbols_text);
}

#[test]
fn memory_symbols_and_entry_options() {
    let p = Project::new("opts");
    p.file("main.s", PROGRAM);
    let b = p
        .build(
            &format!("{LAYOUT}[options]\nmemory = \"chip\"\nhunk_symbols = true\n"),
            "demo",
        )
        .unwrap();
    let exe = Executable::parse(&b.bytes).unwrap();
    assert!(exe.hunks.iter().all(|h| h.memory == Memory::Chip));
    assert!(
        exe.hunks[0]
            .symbols
            .iter()
            .any(|(n, v)| n == "start" && *v == 0)
    );
    // an entry that is not the first byte is refused
    p.file("two.s", "        .text\n        nop\nmain:   rts\n");
    let e = p
        .build(
            "machine = \"amiga\"\nentry = \"main\"\n[[input]]\nasm = \"two.s\"\n",
            "demo",
        )
        .unwrap_err();
    assert!(e.contains("first byte"), "{e}");
    let e = p
        .build(
            "machine = \"amiga\"\n[[input]]\nasm = \"two.s\"\n",
            "demo.tap",
        )
        .unwrap_err();
    assert!(e.contains(".adf"), "{e}");
}

#[test]
fn an_adf_holds_the_program_a_startup_sequence_and_extra_files() {
    let p = Project::new("adf");
    p.file("main.s", PROGRAM).file("readme.txt", "hello");
    let b = p
        .build(
            &format!(
                "{LAYOUT}[options]\nfs = \"ffs\"\nfile_name = \"demo\"\n[[file]]\npath = \"readme.txt\"\nname = \"docs/README\"\n"
            ),
            "demo.adf",
        )
        .unwrap();
    let disk = Adf::parse(&b.bytes).unwrap();
    disk.verify().unwrap();
    assert_eq!(disk.name(), "DEMO");
    assert_eq!(disk.fs(), amiga::adf::Fs::Ffs);
    assert_eq!(disk.read_file("s/startup-sequence").unwrap(), b"demo\n");
    assert_eq!(disk.read_file("docs/README").unwrap(), b"hello");
    let exe = Executable::parse(&disk.read_file("demo").unwrap()).unwrap();
    assert_eq!(exe.hunks.len(), 3);
    // bootable: the standard boot code is in the boot block
    assert_eq!(&b.bytes[12..16], &[0x43, 0xFA, 0x00, 0x18]);
    // not bootable on request
    let b = p
        .build(
            &format!("{LAYOUT}[options]\nbootable = false\n"),
            "demo.adf",
        )
        .unwrap();
    assert_eq!(&b.bytes[12..16], &[0, 0, 0, 0]);
    assert!(b.notes.iter().any(|n| n.contains("boot")));
    assert!(p.path().exists());
}

#[test]
fn an_x68000_executable_has_relocations_and_an_entry() {
    let p = Project::new("x");
    p.file(
        "main.s",
        &format!(
            "        .text\n        bra.s   start\n        nop\n{}",
            PROGRAM.replacen("        .text\n", "", 1)
        ),
    );
    let b = p
        .build(
            "machine = \"x68000\"\nentry = \"start\"\n[[input]]\nasm = \"main.s\"\n",
            "demo.x",
        )
        .unwrap();
    let exe = x68k::exe::Executable::parse(&b.bytes).unwrap();
    assert_eq!(&b.bytes[..2], b"HU");
    assert_eq!(exe.entry, 4);
    assert_eq!(exe.bss, 4);
    assert!(!exe.relocs.is_empty());
    // relocating at $6000 turns the data's first long (the address of start, 4) into $6004
    let image = exe.relocated(0x6000);
    let at = exe.text.len();
    assert_eq!(&image[at..at + 4], &0x6004u32.to_be_bytes());
    // errors
    let e = p
        .build(
            "machine = \"x68000\"\n[[input]]\nasm = \"main.s\"\n",
            "demo.exe",
        )
        .unwrap_err();
    assert!(e.contains(".x"), "{e}");
    let e = p
        .build(
            "machine = \"x68000\"\nentry = 9999\n[[input]]\nasm = \"main.s\"\n",
            "demo.x",
        )
        .unwrap_err();
    assert!(e.contains("outside the text"), "{e}");
}

#[test]
fn an_x68000_floppy_holds_the_program_and_extra_files() {
    let p = Project::new("xdf");
    p.file("main.s", "        .text\nstart:  rts\n");
    p.file("data.bin", "0123456789");
    let b = p
        .build(
            "machine = \"x68000\"\nname = \"demo\"\n[options]\ndisk_name = \"MYDISK\"\n[[input]]\nasm = \"main.s\"\n[[file]]\npath = \"data.bin\"\nname = \"DATA.BIN\"\n",
            "demo.xdf",
        )
        .unwrap();
    assert_eq!(b.bytes.len(), 1_261_568);
    // the volume label, then the files in name order: DATA.BIN, DEMO.X
    let root = &b.bytes[5 * 1024..];
    assert_eq!(&root[..6], b"MYDISK");
    assert_eq!(&root[32..40], b"DATA    ");
    assert_eq!(&root[64..72], b"DEMO    ");
    assert_eq!(&root[72..75], b"X  ");
    let size = u32::from_le_bytes(root[32 + 28..32 + 32].try_into().unwrap());
    assert_eq!(size, 10);
    assert!(b.map.contains("2 file(s)"), "{}", b.map);
}
