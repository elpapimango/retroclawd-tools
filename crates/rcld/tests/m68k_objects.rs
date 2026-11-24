//! 68000 objects through the linker: the same program built from assembler sources and from objects must come out
//! the same, and the objects must be what makes several files and a library possible.

use rc_common::prg;
use rcld::layout;
use std::path::{Path, PathBuf};

struct Project(PathBuf);

impl Project {
    fn new(tag: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("rcld-m68kobj-{tag}-{}", std::process::id()));
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
    pea     msg(pc)
    move.w  #9,-(sp)
    trap    #1
    addq.l  #6,sp
    move.l  #msg,d0
    lea     table,a0
forever:
    bra.s   forever
    .data
msg:
    dc.b    \"HELLO\",13,10,0
    .even
table:
    dc.l    msg
    dc.l    msg-start
    .bss
buffer:
    ds.b    100
";

#[test]
fn an_object_links_to_the_program_the_assembler_makes() {
    let p = Project::new("same");
    p.file("hello.s", SOURCE);
    let direct = p
        .build(
            "machine = \"atari-st\"\n[[input]]\nasm = [\"hello.s\"]\n",
            "a.prg",
        )
        .unwrap();
    let linked = p
        .build(
            "machine = \"atari-st\"\n[[input]]\nsrc = [\"hello.s\"]\n",
            "b.prg",
        )
        .unwrap();
    let (a, b) = (
        prg::parse(&direct.bytes).unwrap(),
        prg::parse(&linked.bytes).unwrap(),
    );
    assert_eq!(a.text, b.text);
    assert_eq!(a.data, b.data);
    assert_eq!(a.bss, b.bss);
    assert_eq!(a.fixups, b.fixups);
}

#[test]
fn two_files_and_a_library_link_and_the_imports_are_filled_in() {
    let p = Project::new("two");
    p.file(
        "main.s",
        "
    xref    double
    xdef    start
    .text
start:
    move.l  #21,d0
    bsr.w   double          ; another file, another section distance: left to the linker
    jsr     double          ; an absolute long: a fixup
    lea     answer,a0
    rts
    .data
answer:
    dc.l    0
",
    );
    p.file(
        "lib.s",
        "
    xdef    double
    .text
double:
    add.l   d0,d0
    rts
",
    );
    let built = p
        .build(
            "machine = \"atari-st\"\n[[input]]\nsrc = [\"main.s\", \"lib.s\"]\n",
            "x.prg",
        )
        .unwrap();
    let prg = prg::parse(&built.bytes).unwrap();
    // main: moveq-size move.l #21 (6) bsr.w (4) jsr (6) lea (6) rts (2) = 24; lib's `double` follows at 24
    assert_eq!(prg.text.len(), 24 + 4);
    let word = |at: usize| u16::from_be_bytes([prg.text[at], prg.text[at + 1]]);
    assert_eq!(word(6), 0x6100, "bsr.w");
    assert_eq!(word(8), 24 - 8, "displacement from the extension word");
    let image = prg.relocated(0x1000);
    let long = |at: usize| u32::from_be_bytes(image[at..at + 4].try_into().unwrap());
    assert_eq!(long(12), 0x1000 + 24, "jsr double");
    assert!(prg.fixups.contains(&12), "{:?}", prg.fixups);
    assert!(prg.fixups.contains(&18), "lea answer: {:?}", prg.fixups);
    assert_eq!(prg.fixups.len(), 2);
}

#[test]
fn a_member_of_a_library_is_linked_only_when_something_needs_it() {
    let p = Project::new("lib");
    p.file(
        "main.s",
        "    xref used\n    .text\n    jsr used\n    rts\n",
    );
    p.file("used.s", "    xdef used\n    .text\nused:\n    rts\n");
    p.file(
        "unused.s",
        "    xdef unused\n    .text\nunused:\n    nop\n    nop\n    rts\n",
    );
    let layout = |libs: &str| {
        format!(
            "machine = \"atari-st\"\n[[input]]\nsrc = [\"main.s\"]\n[[input]]\nlib = [\"{libs}\"]\n"
        )
    };
    // build the library with the tools' own pieces
    let asm = |name: &str| {
        let src = std::fs::read_to_string(p.path().join(name)).unwrap();
        rcasm::m68k::assemble_m68k_object(
            name,
            &src,
            &rcasm::m68k::M68kOptions::default(),
            &rcasm::DiskFiles,
        )
        .unwrap()
    };
    let mut lib = rc_common::obj::Archive::new();
    for n in ["used.s", "unused.s"] {
        lib.add(n, &asm(n));
    }
    std::fs::write(p.path().join("lib.rca"), lib.encode().unwrap()).unwrap();
    let built = p.build(&layout("lib.rca"), "x.prg").unwrap();
    let prg = prg::parse(&built.bytes).unwrap();
    // jsr (6) + rts (2) + used: rts (2): `unused` stays out
    assert_eq!(prg.text.len(), 6 + 2 + 2);
}

#[test]
fn an_import_nobody_defines_names_the_symbol_and_the_file() {
    let p = Project::new("undef");
    p.file(
        "main.s",
        "    xref nowhere\n    .text\n    jsr nowhere\n    rts\n",
    );
    let err = p
        .build(
            "machine = \"atari-st\"\n[[input]]\nsrc = [\"main.s\"]\n",
            "x.prg",
        )
        .unwrap_err();
    assert!(err.contains("nowhere") && err.contains("main.s"), "{err}");
}

#[test]
fn an_address_in_a_word_is_refused_with_its_line() {
    let p = Project::new("word");
    p.file(
        "main.s",
        "    .text\nstart:\n    nop\n    .data\nd:\n    dc.w   start\n",
    );
    let err = p
        .build(
            "machine = \"atari-st\"\n[[input]]\nsrc = [\"main.s\"]\n",
            "x.prg",
        )
        .unwrap_err();
    assert!(
        err.contains("main.s:6") && err.contains("four-byte"),
        "{err}"
    );
}

#[test]
fn a_branch_out_of_range_after_linking_is_an_error() {
    let p = Project::new("far");
    p.file("main.s", "    xref far\n    .text\n    bra.w far\n");
    p.file(
        "far.s",
        "    xdef far\n    .text\n    ds.b 40000\nfar:\n    rts\n",
    );
    let err = p
        .build(
            "machine = \"atari-st\"\n[[input]]\nsrc = [\"main.s\", \"far.s\"]\n",
            "x.prg",
        )
        .unwrap_err();
    assert!(
        err.contains("main.s:3") && err.contains("does not fit"),
        "{err}"
    );
}

#[test]
fn a_cartridge_places_text_at_zero_and_bss_in_ram() {
    let p = Project::new("flat");
    p.file(
        "main.s",
        "
    xdef    start
    .text
start:
    lea     counter,a0         ; work RAM
    move.l  d0,(a0)
    bra.s   start
    .bss
counter:
    ds.l    1
",
    );
    let layout = layout::parse("machine = \"genesis\"\n[[input]]\nsrc = [\"main.s\"]\n").unwrap();
    let built = rcld::build(&layout, p.path(), &p.path().join("x.md"), &[]).unwrap();
    let rom = &built.bytes;
    let long = |at: usize| u32::from_be_bytes(rom[at..at + 4].try_into().unwrap());
    assert_eq!(
        long(4),
        0x200,
        "the generated reset vector: start follows the vectors and the header"
    );
    // lea counter,a0 -> absolute long in work RAM
    assert_eq!(
        long(0x202),
        0xFF_0000,
        "counter is the first thing in the work RAM"
    );
}
