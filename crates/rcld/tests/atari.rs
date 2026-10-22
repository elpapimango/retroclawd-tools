//! The Atari ST target: a layout with one 68000 source becomes a relocatable GEMDOS executable.

use rc_common::prg;
use rcld::layout;
use std::path::{Path, PathBuf};

struct Project(PathBuf);

impl Project {
    fn new(tag: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("rcld-atari-{tag}-{}", std::process::id()));
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

const SOURCE: &str = "
    .text
start:
    pea     msg(pc)          ; position independent: no fixup
    move.w  #9,-(sp)
    trap    #1
    addq.l  #6,sp
    move.l  #msg,d0          ; an address: a fixup
    lea     table,a0         ; an address: a fixup
forever:
    bra.s   forever
    .data
msg:
    dc.b    \"HELLO\",13,10,0
    .even
table:
    dc.l    msg              ; an address in the data: a fixup
    dc.l    msg-start        ; a distance: not
    .bss
buffer:
    ds.b    100
";

const LAYOUT: &str = "machine = \"atari-st\"\n[[input]]\nasm = [\"hello.s\"]\n";

#[test]
fn a_layout_becomes_a_relocatable_prg() {
    let p = Project::new("build");
    p.file("hello.s", SOURCE);
    let built = p.build(LAYOUT, "hello.prg").unwrap();
    let prg = prg::parse(&built.bytes).unwrap();
    assert_eq!(prg.bss, 100);
    assert!(prg.symbols.is_empty());
    // pea (4) move.w (4) trap (2) addq (2) move.l #msg (6: long at 14) lea table (6: long at 20) bra.s (2).
    assert_eq!(prg.text.len(), 4 + 4 + 2 + 2 + 6 + 6 + 2);
    let text = prg.text.len() as u32;
    // The data: "HELLO\r\n\0" is 8 bytes, `table` is at text + 8: its first long is the fixup in the data.
    assert_eq!(prg.fixups, vec![14, 20, text + 8]);
    // Loaded at $20000 the image holds the real addresses.
    let image = prg.relocated(0x20000);
    let long = |at: usize| u32::from_be_bytes(image[at..at + 4].try_into().unwrap());
    assert_eq!(long(14), 0x20000 + text, "move.l #msg");
    assert_eq!(long(20), 0x20000 + text + 8, "lea table");
    assert_eq!(
        long(text as usize + 8),
        0x20000 + text,
        "the pointer in the data"
    );
    assert_eq!(
        long(text as usize + 12),
        text,
        "a distance stays what it is"
    );
    assert!(
        built.map.contains("2 relocations") || built.map.contains("3 relocations"),
        "{}",
        built.map
    );
    assert!(
        built.symbols_text.contains("msg = $"),
        "{}",
        built.symbols_text
    );
}

#[test]
fn the_same_program_builds_to_the_same_bytes() {
    let p = Project::new("same");
    p.file("hello.s", SOURCE);
    let a = p.build(LAYOUT, "a.prg").unwrap().bytes;
    let b = p.build(LAYOUT, "b.prg").unwrap().bytes;
    assert_eq!(a, b);
}

#[test]
fn several_sources_are_one_program() {
    let p = Project::new("many");
    p.file("a.s", "    .text\nstart:\n    bsr.s sub\n    rts\n")
        .file("b.s", "sub:\n    nop\n    rts\n");
    let built = p
        .build(
            "machine = \"atari-st\"\n[[input]]\nasm = [\"a.s\", \"b.s\"]\n",
            "x.prg",
        )
        .unwrap();
    let prg = prg::parse(&built.bytes).unwrap();
    assert_eq!(
        prg.text,
        vec![0x61, 0x02, 0x4E, 0x75, 0x4E, 0x71, 0x4E, 0x75]
    );
    assert!(prg.fixups.is_empty());
}

#[test]
fn things_an_atari_program_cannot_have_are_refused() {
    let p = Project::new("refuse");
    p.file("hello.s", SOURCE).file("blob.bin", "x");
    let e = p.build(LAYOUT, "hello.bin").unwrap_err();
    assert!(e.contains(".prg"), "{e}");
    let e = p
        .build(
            "machine = \"atari-st\"\nentry = \"start\"\n[[input]]\nasm = [\"hello.s\"]\n",
            "h.prg",
        )
        .unwrap_err();
    assert!(e.contains("first byte of its text"), "{e}");
    let e = p
        .build(
            "machine = \"atari-st\"\n[[input]]\nbin = \"blob.bin\"\nat = 0\n",
            "h.prg",
        )
        .unwrap_err();
    assert!(e.contains("assembler sources only"), "{e}");
    let e = p
        .build("machine = \"atari-st\"\n[[region]]\nname = \"ram\"\nstart = 0\nend = 100\n[[input]]\nasm = [\"hello.s\"]\n", "h.prg")
        .unwrap_err();
    assert!(e.contains("regions"), "{e}");
    let e = p.build("machine = \"atari-st\"\n", "h.prg").unwrap_err();
    assert!(e.contains("no program"), "{e}");
}

#[test]
fn an_error_in_the_source_names_the_file_and_line() {
    let p = Project::new("error");
    p.file("hello.s", "    .text\n    bogus d0\n");
    let e = p.build(LAYOUT, "hello.prg").unwrap_err();
    assert!(e.contains("hello.s:2:") && e.contains("bogus"), "{e}");
    let _ = p.path();
}

#[test]
fn the_template_is_a_layout_that_parses() {
    let text = rcld::templates::template("atari-st").expect("a template");
    let layout = layout::parse(text).unwrap();
    assert_eq!(layout.machine, layout::Machine::AtariSt);
}
