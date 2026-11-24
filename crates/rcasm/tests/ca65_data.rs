//! ca65's `.define`, `.struct`, `.enum`, macro `.local` names and `.ifblank`.

use rcasm::{Cpu, DiskFiles, Options, assemble};

fn asm(source: &str) -> Result<Vec<u8>, String> {
    let opts = Options {
        cpu: Some(Cpu::Mos6502),
        ..Options::default()
    };
    assemble("t.asm", source, &opts, &DiskFiles)
        .map(|a| a.segments.into_iter().flat_map(|s| s.bytes).collect::<Vec<u8>>())
        .map_err(|e| e.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("; "))
}

#[test]
fn define_replaces_names_and_function_like_macros_outside_strings() {
    let src = "\
.org $1000
.define SCREEN $0400
.define LO(x) <(x)
.define TWICE(a) ((a)*2)
        lda #LO(SCREEN)
        sta SCREEN
        lda #TWICE(3+1)
        .byte \"SCREEN\"
.undef SCREEN
";
    assert_eq!(
        asm(src).unwrap(),
        [0xA9, 0x00, 0x8D, 0x00, 0x04, 0xA9, 8, b'S', b'C', b'R', b'E', b'E', b'N']
    );
    assert!(asm(".org $1000\n.define X 1\n.undef X\n lda #X\n").unwrap_err().contains("X"));
}

#[test]
fn ifdef_sees_a_define() {
    let src = ".org $1000\n.define DEBUG 1\n.ifdef DEBUG\n nop\n.endif\n.ifndef DEBUG\n brk\n.endif\n";
    assert_eq!(asm(src).unwrap(), [0xEA]);
}

#[test]
fn struct_offsets_and_sizeof() {
    let src = "\
.struct Sprite
        xpos    .word
        ypos    .byte
        flags   .byte
        name    .res 4
        extra   .tag Point
.endstruct
.struct Point
.endstruct
";
    // Point is empty here and defined after: .tag needs it first
    assert!(asm(src).unwrap_err().contains("not a struct"));
    let src = "\
.org $1000
.struct Point
        px      .word
        py      .word
.endstruct
.struct Sprite
        xpos    .word
        ypos    .byte
        flags   .byte
        name    .res 4
        pos     .tag Point
.endstruct
        lda #Sprite::ypos
        lda #Sprite::name
        lda #Sprite::pos + Point::py
        lda #.sizeof(Sprite)
        lda #.sizeof(Point)
";
    assert_eq!(
        asm(src).unwrap(),
        [0xA9, 2, 0xA9, 4, 0xA9, 8 + 2, 0xA9, 12, 0xA9, 4]
    );
}

#[test]
fn enum_counts_up_and_jumps_with_equals() {
    let src = "\
.org $1000
.enum Color
        Black
        Red
        Green = 5
        Blue
.endenum
.enum
        First = 10
        Second
.endenum
        lda #Color::Red
        lda #Color::Blue
        lda #Second
";
    assert_eq!(asm(src).unwrap(), [0xA9, 1, 0xA9, 6, 0xA9, 11]);
}

#[test]
fn local_names_are_private_to_each_expansion() {
    let src = "\
.org $1000
.macro wait n
        .local again
        ldx #n
again:  dex
        bne again
.endmacro
        wait 3
        wait 5
";
    // two expansions of the same label would be \"already defined\" without .local
    assert_eq!(
        asm(src).unwrap(),
        [0xA2, 3, 0xCA, 0xD0, 0xFD, 0xA2, 5, 0xCA, 0xD0, 0xFD]
    );
    assert!(asm(".local x\n").unwrap_err().contains("macro"));
}

#[test]
fn ifblank_tests_a_macro_argument() {
    let src = "\
.org $1000
.macro load v
        .ifblank v
        lda #0
        .else
        lda #v
        .endif
.endmacro
        load
        load 7
";
    assert_eq!(asm(src).unwrap(), [0xA9, 0, 0xA9, 7]);
}
