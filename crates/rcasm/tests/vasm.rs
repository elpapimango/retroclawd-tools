//! vasm's `oldstyle` and `std` syntaxes (`--syntax`), checked against what vasm's manual says each form means.

use rcasm::{Cpu, DiskFiles, Options, Syntax, assemble};

fn asm(syntax: Syntax, cpu: Cpu, source: &str) -> Result<Vec<u8>, String> {
    let opts = Options {
        cpu: Some(cpu),
        syntax,
        ..Options::default()
    };
    assemble("t.asm", source, &opts, &DiskFiles)
        .map(|a| a.segments.into_iter().flat_map(|s| s.bytes).collect())
        .map_err(|e| {
            e.iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })
}

fn old(source: &str) -> Result<Vec<u8>, String> {
    asm(Syntax::Oldstyle, Cpu::Mos6502, source)
}

fn std(source: &str) -> Result<Vec<u8>, String> {
    asm(Syntax::Std, Cpu::Mos6502, source)
}

#[test]
fn oldstyle_labels_comments_and_numbers() {
    let src = "\
* a comment in the first column
        org     $1000
start   lda     #@17            ; octal
        ldx     #%101
        ldy     #4#13           ; base 4
        lda     #12d
        lda     #17q
        lda     #101b
        lda     #'A
.loop   dex
        bne     .loop
skip$   jmp     skip$
";
    assert_eq!(
        old(src).unwrap(),
        [
            0xA9, 0o17, 0xA2, 5, 0xA0, 7, 0xA9, 12, 0xA9, 0o17, 0xA9, 5, 0xA9, 0x41, 0xCA, 0xD0,
            0xFD, 0x4C, 0x11, 0x10
        ]
    );
}

#[test]
fn oldstyle_data_space_and_alignment() {
    let src = "        org     $1000
        byt     1,\"AB\"
        dw      $1234
        dl      $123456         ; 24 bits on the 6502
        fcc     /hi/
        fcs     \"ok\"
        asciiz  \"x\",\"y\"
        str     \"z\"
        blk     2,$EE
        dsw     1,$ABCD
        align   2               ; to a multiple of 4
        byte
        even
";
    assert_eq!(
        old(src).unwrap(),
        [
            1,
            b'A',
            b'B',
            0x34,
            0x12,
            0x56,
            0x34,
            0x12,
            b'h',
            b'i',
            b'o',
            b'k' | 0x80,
            b'x',
            0,
            b'y',
            0,
            b'z',
            13,
            0xEE,
            0xEE,
            0xCD,
            0xAB,
            0,
            0,
            0,
            0
        ]
    );
}

#[test]
fn oldstyle_symbols_conditions_and_repeats() {
    let src = "        org     $1000
SIZE    equ     3
n       set     1
n       set     n+1
        if      SIZE=3          ; a single = compares
        byte    n
        else
        byte    $FF
        endif
        ifeq    SIZE-3
        byte    $10
        endc
        ifgt    SIZE-5
        byte    $FF
        endif
        ifd     SIZE
        byte    $20
        endif
        rept    2
        byte    $30
        endr
";
    assert_eq!(old(src).unwrap(), [2, 0x10, 0x20, 0x30, 0x30]);
}

#[test]
fn oldstyle_macros_take_numbered_and_named_arguments() {
    let src = "        org     $1000
poke    macro
        lda     #\\1
        sta     \\2
        endm
wait    macro   count
        ldx     #\\count
loop\\@ dex
        bne     loop\\@
        endm
        poke    5,$D020
        wait    3
        wait    4
";
    assert_eq!(
        old(src).unwrap(),
        [
            0xA9, 5, 0x8D, 0x20, 0xD0, 0xA2, 3, 0xCA, 0xD0, 0xFD, 0xA2, 4, 0xCA, 0xD0, 0xFD
        ]
    );
}

#[test]
fn oldstyle_unsupported_directives_are_named() {
    let e = old("        org $1000\n        abyte 1,2\n").unwrap_err();
    assert!(e.contains("abyte") && e.contains("t.asm:2"), "{e}");
    let e = old("        org $1000\n        inline\n").unwrap_err();
    assert!(e.contains("inline"), "{e}");
    // an unclosed structure or a stray endstruct is named too
    let e = old("        org $1000\n        endstruct\n").unwrap_err();
    assert!(e.contains("endstruct"), "{e}");
}

#[test]
fn oldstyle_z80_uses_at_for_binary_and_ampersand_for_hex() {
    let src = "        org     $8000
        ld      a,@101
        ld      b,&1F
        ei
";
    assert_eq!(
        asm(Syntax::Oldstyle, Cpu::Z80, src).unwrap(),
        [0x3E, 5, 0x06, 0x1F, 0xFB]
    );
}

#[test]
fn std_labels_numbers_and_data() {
    let src = "\
        .org    0x1000
start:  lda     #017            ; octal
        ldx     #0b101
        ldy     #0x10
1:      dex
        bne     1b
        beq     1f
        nop
1:      rts
        .byte   1, 2
        .short  0x1234
        .long   0x12345678
        .asciz  \"a\"
        .space  2, 0xEE
        .align  2
        .balign 2
";
    assert_eq!(
        std(src).unwrap(),
        [
            0xA9, 0o17, 0xA2, 5, 0xA0, 0x10, 0xCA, 0xD0, 0xFD, 0xF0, 0x01, 0xEA, 0x60, 1, 2, 0x34,
            0x12, 0x78, 0x56, 0x34, 0x12, b'a', 0, 0xEE, 0xEE, 0, 0, 0
        ]
    );
}

#[test]
fn std_symbols_conditions_and_macros() {
    let src = "\
        .org    0x1000
        .equ    two, 2
        .set    n, 1
        .set    n, n+two
        .if n == 3
        .byte   n
        .elseif n == 4
        .byte   0xFF
        .else
        .byte   0xFE
        .endif
        .macro  put v
        .byte   \\v, \\v
        .endm
        put     7
        .rept   2
        .byte   9
        .endr
        .byte   6!1             ; or-not: 6 | ~1
";
    assert_eq!(std(src).unwrap(), [3, 7, 7, 9, 9, 0xFE]);
    let e = std("        .org 0x1000\n        .section .data\n").unwrap_err();
    assert!(e.contains(".section"), "{e}");
}

#[test]
fn native_syntax_is_untouched() {
    // a `*` line is a native statement, `.loop` would be a directive there
    let src = "* = $1000\n        lda #1\n";
    assert_eq!(asm(Syntax::Native, Cpu::Mos6502, src).unwrap(), [0xA9, 1]);
}

#[test]
fn oldstyle_offset_sections_character_operands_and_spaced_operands() {
    let src = "        .dsect
        .org $0010
ptr:    .wrd
count:  .byt
        .dend
        .org $1000
        lda #\"-\"
        ldx #\"A
        lda (ptr), y
        sta count
";
    // `.dsect` gives `ptr` $10 and `count` $12 and writes nothing
    assert_eq!(
        old(src).unwrap(),
        [0xA9, b'-', 0xA2, b'A', 0xB1, 0x10, 0x85, 0x12]
    );
    let e = old("        .dsect\n        .byt\n").unwrap_err();
    assert!(e.contains(".dsect without .dend"), "{e}");
}

#[test]
fn oldstyle_rorg_places_labels_elsewhere_than_the_bytes() {
    let src = "        org     $1000
        rorg    $8000
start   jmp     start
        rend
here    jmp     here
";
    assert_eq!(old(src).unwrap(), [0x4C, 0x00, 0x80, 0x4C, 0x03, 0x10]);
}

#[test]
fn oldstyle_structures_define_offsets_sizes_and_instances() {
    let src = "        org     $1000
        struct  point
x       db      4
y       db      5,6
z       ds      2
        endstruct
p1      point
p2      point   1,,9
        lda     #point.y
        lda     #point
";
    // offsets x 0, y 1, z 3, size 5; an empty value keeps the default, a reserved field takes no value
    assert_eq!(
        old(src).unwrap(),
        [4, 5, 6, 0, 0, 1, 5, 6, 0, 0, 0xA9, 1, 0xA9, 5]
    );
}

#[test]
fn std_irp_irpc_and_ifc() {
    let src = r"        .org    0x1000
        .irp    v, 1, 2, 3
        .byte   \v
        .endr
        .irpc   c, ab
        .byte   '\c'
        .endr
        .macro  same a
        .ifc    \a, yes
        .byte   0xAA
        .else
        .byte   0xBB
        .endif
        .endm
        same    yes
        same    no
        .text
        .byte   9
";
    assert_eq!(std(src).unwrap(), [1, 2, 3, b'a', b'b', 0xAA, 0xBB, 9]);
}

#[test]
fn sections_are_segments_in_an_object_and_an_error_in_an_image() {
    let e = std("        .org 0x1000\n        .section foo\n").unwrap_err();
    assert!(e.contains("object"), "{e}");
    let opts = Options {
        cpu: Some(Cpu::Mos6502),
        syntax: Syntax::Std,
        ..Options::default()
    };
    let obj = rcasm::assemble_object(
        "t.asm",
        "        .text\n        nop\n        .data\n        .byte 1\n",
        &opts,
        &DiskFiles,
    )
    .unwrap();
    let names: Vec<&str> = obj.sections.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"CODE") && names.contains(&"DATA"),
        "{names:?}"
    );
}
