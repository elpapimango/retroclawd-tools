//! `.proc` / `.scope` blocks and `name::symbol` references.

use rcasm::{Cpu, DiskFiles, Options, assemble, assemble_object};

fn asm(source: &str) -> Result<Vec<u8>, String> {
    let opts = Options {
        cpu: Some(Cpu::Mos6502),
        ..Options::default()
    };
    assemble("t.asm", source, &opts, &DiskFiles)
        .map(|a| {
            a.segments
                .into_iter()
                .flat_map(|s| s.bytes)
                .collect::<Vec<u8>>()
        })
        .map_err(|e| {
            e.iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })
}

#[test]
fn a_proc_is_a_label_and_a_scope() {
    // main calls lib::init; inside lib, `loop` is lib::loop and `init` is found without the prefix
    let src = "\
.org $1000
main:   jsr lib::init
        jmp main
.proc lib
init:   ldx #0
loop:   inx
        bne loop
        rts
.endproc
";
    let b = asm(src).unwrap();
    // jsr $1006; jmp $1000; ldx #0; inx; bne -3; rts
    assert_eq!(b[..3], [0x20, 0x06, 0x10]);
    assert_eq!(b[3..6], [0x4C, 0x00, 0x10]);
    assert_eq!(b[6..], [0xA2, 0x00, 0xE8, 0xD0, 0xFD, 0x60]);
}

#[test]
fn the_same_name_in_two_scopes_is_two_symbols() {
    let src = "\
.org $1000
.scope a
x:      .byte 1
.endscope
.scope b
x:      .byte 2
y:      .word x, a::x, ::top
.endscope
top:    nop
";
    let bytes = asm(src).unwrap();
    // a::x at $1000, b::x at $1001; the .word in b: x = $1001, a::x = $1000, ::top = $1007
    assert_eq!(bytes[..2], [1, 2]);
    assert_eq!(bytes[2..8], [0x01, 0x10, 0x00, 0x10, 0x08, 0x10]);
}

#[test]
fn an_outer_name_is_visible_inside_and_unclosed_or_unmatched_ends_are_errors() {
    let src = ".org $1000\nouter = 5\n.scope s\n lda #outer\n.endscope\n";
    assert_eq!(asm(src).unwrap(), [0xA9, 0x05]);
    assert!(
        asm(".org $1000\n.endproc\n")
            .unwrap_err()
            .contains("without")
    );
    assert!(
        asm(".org $1000\n.proc\n")
            .unwrap_err()
            .contains("needs a name")
    );
}

#[test]
fn scoped_names_work_in_objects() {
    let opts = Options {
        cpu: Some(Cpu::Mos6502),
        ..Options::default()
    };
    let src = ".export start\nstart: jsr util::run\n.proc util\nrun: rts\n.endproc\n";
    let o = assemble_object("t.asm", src, &opts, &DiskFiles).unwrap();
    assert!(o.symbols.iter().any(|s| s.name == "util::run"));
    assert!(o.relocs.is_empty() || o.relocs.iter().all(|r| r.width == 2));
}
