//! The 68000 back end in object mode: what becomes a relocation and what the assembler settles itself.

use rc_common::obj::{Class, Kind, Object};
use rcasm::DiskFiles;
use rcasm::m68k::{M68kOptions, assemble_m68k_object};

fn object(src: &str) -> Result<Object, String> {
    assemble_m68k_object("t.s", src, &M68kOptions::default(), &DiskFiles)
        .map_err(|e| e.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n"))
}

#[test]
fn only_what_depends_on_an_address_or_an_import_is_relocated() {
    let o = object(
        "
    xref    ext
    xdef    start
    .text
start:
    move.l  #4,d0           ; a constant
    move.l  #start,d1       ; an address: absolute long
    lea     back(pc),a0     ; the same section: settled here
    bra.s   start           ; the same section: settled here
    bsr.w   ext             ; an import: relative word
    jsr     ext             ; an import: absolute long
back:
    rts
    .data
tab:
    dc.l    start, ext+4
    dc.w    7
",
    )
    .unwrap();
    assert_eq!(o.cpu, "68000");
    let kinds: Vec<(u32, u32, u8, Kind)> = o.relocs.iter().map(|r| (r.section, r.offset, r.width, r.kind)).collect();
    // move.l #start (2 + 4 at offset 6), bsr.w ext (word at 20, relative), jsr ext (long at 24), the two data longs
    assert_eq!(
        kinds,
        vec![
            (0, 8, 4, Kind::Abs),
            (0, 20, 2, Kind::Rel),
            (0, 24, 4, Kind::Abs),
            (1, 0, 4, Kind::Abs),
            (1, 4, 4, Kind::Abs),
        ]
    );
    let names: Vec<(&str, Class, bool)> = o.symbols.iter().map(|s| (s.name.as_str(), s.class, s.global)).collect();
    assert!(names.contains(&("start", Class::Label, true)));
    assert!(names.contains(&("ext", Class::Import, true)));
    assert!(names.contains(&("back", Class::Label, false)));
    // a relative field is measured from the extension word
    let bsr = o.relocs.iter().find(|r| r.kind == Kind::Rel).unwrap();
    assert_eq!((bsr.bias, bsr.instr_back), (2, 2));
}

#[test]
fn a_label_difference_is_a_constant_and_an_alias_stays_an_address() {
    let o = object(
        "
    .text
a:  nop
b:  nop
size    equ b-a
alias   equ b
    moveq   #size,d0
    move.l  #alias,d1
",
    )
    .unwrap();
    assert_eq!(o.relocs.len(), 1, "{:?}", o.relocs);
    assert_eq!(o.relocs[0].width, 4);
    assert_eq!(&o.sections[0].bytes[4..6], &[0x70, 0x02], "moveq #2,d0");
}

#[test]
fn mistakes_are_named() {
    let e = object("    .org $1000\n    nop\n").unwrap_err();
    assert!(e.contains("t.s:1") && e.contains(".org"), "{e}");
    let e = object("    xref ext\n    .text\n    bra.s ext\n").unwrap_err();
    assert!(e.contains("t.s:3") && e.contains("short branch"), "{e}");
    let e = object("    xref ext\n    .text\n    moveq #ext,d0\n").unwrap_err();
    assert!(e.contains("t.s:3") && e.contains("not a constant"), "{e}");
    let e = object("    xdef nothing\n    .text\n    nop\n").unwrap_err();
    assert!(e.contains("nothing") && e.contains("not defined"), "{e}");
    let e = object("    .text\n    jsr missing\n").unwrap_err();
    assert!(e.contains("missing"), "{e}");
}

#[test]
fn a_global_that_is_not_defined_is_an_import() {
    let o = object("    .globl f\n    .globl g\n    .text\ng:  jsr f\n").unwrap();
    let f = o.symbols.iter().find(|s| s.name == "f").unwrap();
    let g = o.symbols.iter().find(|s| s.name == "g").unwrap();
    assert_eq!((f.class, g.class), (Class::Import, Class::Label));
    assert!(g.global);
}
