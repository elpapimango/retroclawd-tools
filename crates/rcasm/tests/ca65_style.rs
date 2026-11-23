//! The ca65-style conveniences: unnamed labels, `.repeat`, `.addr`, `.dbyt`, `.setcpu` and the housekeeping directives.

use rcasm::{Cpu, DiskFiles, Options, assemble};

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
fn unnamed_labels_look_forward_and_back() {
    let src = "\
.org $1000
        ldx #3
:       dex
        bne :-
        beq :+
        nop
:       rts
";
    // ldx #3; dex; bne back to dex; beq forward to rts (over the nop); nop; rts
    assert_eq!(
        asm(src).unwrap(),
        [0xA2, 3, 0xCA, 0xD0, 0xFD, 0xF0, 0x01, 0xEA, 0x60]
    );
    // two back, two ahead
    let src = ".org $1000\n:  nop\n:  nop\n   jmp :--\n   jmp :++\n:  nop\n:  nop\n";
    let b = asm(src).unwrap();
    assert_eq!(b[2..5], [0x4C, 0x00, 0x10]);
    assert_eq!(b[5..8], [0x4C, 0x09, 0x10]);
}

#[test]
fn repeat_expands_with_a_counter() {
    assert_eq!(
        asm(".org $1000\n.repeat 3\n nop\n.endrep\n").unwrap(),
        [0xEA; 3]
    );
    assert_eq!(
        asm(".org $1000\n.repeat 4, i\n .byte i*2\n.endrep\n").unwrap(),
        [0, 2, 4, 6]
    );
    // nested
    assert_eq!(
        asm(".org $1000\n.repeat 2, a\n.repeat 2, b\n .byte a*10+b\n.endrep\n.endrep\n").unwrap(),
        [0, 1, 10, 11]
    );
    assert!(
        asm(".org $1000\n.repeat 2\n nop\n")
            .unwrap_err()
            .contains("endrep")
    );
}

#[test]
fn addr_dbyt_and_cpu_directives() {
    assert_eq!(
        asm(".org $1000\nhere: .addr here, here+1\n").unwrap(),
        [0x00, 0x10, 0x01, 0x10]
    );
    assert_eq!(
        asm(".org $1000\n.dbyt $1234, 2\n").unwrap(),
        [0x12, 0x34, 0x00, 0x02]
    );
    assert_eq!(
        asm(".setcpu \"65816\"\n.org $808000\n.a16\n lda #$1234\n").unwrap(),
        [0xA9, 0x34, 0x12]
    );
    assert_eq!(
        asm(".p816\n.org $8000\n rep #$30\n lda #$1234\n").unwrap(),
        [0xC2, 0x30, 0xA9, 0x34, 0x12]
    );
    assert_eq!(
        asm(".feature c_comments\n.macpack generic\n.list off\n.org $1000\n nop\n").unwrap(),
        [0xEA]
    );
}
