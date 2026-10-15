//! The assembler language, end to end: source text in, bytes and symbols out.

use rcasm::{Assembly, Cpu, Files, Options, assemble};
use std::collections::HashMap;
use std::path::Path;

struct Mem(HashMap<String, Vec<u8>>);

impl Files for Mem {
    fn read(&self, path: &Path) -> Result<Vec<u8>, String> {
        let key = path.to_string_lossy().replace('\\', "/");
        let key = key.strip_prefix("./").unwrap_or(&key).to_string();
        self.0
            .get(&key)
            .cloned()
            .ok_or_else(|| format!("no such file {key}"))
    }
}

fn run(cpu: Cpu, source: &str) -> Result<Assembly, Vec<String>> {
    run_with(cpu, source, &[], false)
}

fn run_with(
    cpu: Cpu,
    source: &str,
    files: &[(&str, &[u8])],
    illegal: bool,
) -> Result<Assembly, Vec<String>> {
    let opts = Options {
        cpu: Some(cpu),
        illegal,
        ..Options::default()
    };
    let mem = Mem(files
        .iter()
        .map(|(n, b)| (n.to_string(), b.to_vec()))
        .collect());
    assemble("t.asm", source, &opts, &mem).map_err(|e| e.iter().map(|d| d.to_string()).collect())
}

fn bytes(cpu: Cpu, source: &str) -> Vec<u8> {
    let asm = run(cpu, source).unwrap_or_else(|e| panic!("{e:#?}"));
    assert_eq!(asm.segments.len(), 1, "{:?}", asm.segments);
    asm.segments[0].bytes.clone()
}

fn symbol(asm: &Assembly, name: &str) -> i64 {
    asm.symbols
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no symbol {name}: {:?}", asm.symbols))
        .value
}

fn errors(cpu: Cpu, source: &str) -> Vec<String> {
    run(cpu, source).expect_err("expected an error")
}

#[test]
fn a_6502_program_with_forward_and_backward_labels() {
    let asm = run(
        Cpu::Mos6502,
        "        .org $C000
start:  ldx #0
loop:   lda text,x
        beq done
        jsr $FFD2
        inx
        bne loop
done:   rts
text:   .text \"HI\"
        .byte 0
",
    )
    .unwrap();
    assert_eq!(
        asm.segments[0].bytes,
        [
            0xA2, 0x00, // ldx #0
            0xBD, 0x0E, 0xC0, // lda text,x
            0xF0, 0x06, // beq done
            0x20, 0xD2, 0xFF, // jsr $FFD2
            0xE8, // inx
            0xD0, 0xF5, // bne loop
            0x60, // rts
            b'H', b'I', 0,
        ]
    );
    assert_eq!(symbol(&asm, "start"), 0xC000);
    assert_eq!(symbol(&asm, "loop"), 0xC002);
    assert_eq!(symbol(&asm, "done"), 0xC00D);
    assert_eq!(symbol(&asm, "text"), 0xC00E);
}

#[test]
fn zero_page_versus_absolute_follows_the_symbol_value_and_stays_stable() {
    // `zp` is defined before use and small: zero-page. `later` is defined after use: absolute even
    // though its value is small, in both passes (the size must not change between them).
    let b = bytes(
        Cpu::Mos6502,
        "        .org $1000
zp      = $10
        lda zp
        lda later
        lda zp+1,x
        sta ($20),y
        jmp ($0300)
later   = $12
",
    );
    assert_eq!(
        b,
        [
            0xA5, 0x10, // lda zp
            0xAD, 0x12, 0x00, // lda later (absolute)
            0xB5, 0x11, // lda zp+1,x
            0x91, 0x20, // sta ($20),y
            0x6C, 0x00, 0x03, // jmp ($0300)
        ]
    );
}

#[test]
fn six_five_o_two_addressing_modes() {
    let b = bytes(
        Cpu::Mos6502,
        ".org $0800
        asl
        asl a
        lda #<$1234
        lda #>$1234
        lda ($10,x)
        lda ($10),y
        lda $1234,y
        ldx $10,y
        inc $1234,x
        cmp #'A'
        clc
",
    );
    assert_eq!(
        b,
        [
            0x0A, 0x0A, 0xA9, 0x34, 0xA9, 0x12, 0xA1, 0x10, 0xB1, 0x10, 0xB9, 0x34, 0x12, 0xB6,
            0x10, 0xFE, 0x34, 0x12, 0xC9, 0x41, 0x18
        ]
    );
}

#[test]
fn branches_take_absolute_targets_and_are_range_checked() {
    let b = bytes(Cpu::Mos6502, ".org $C000\nloop: dex\n bne loop\n");
    assert_eq!(b, [0xCA, 0xD0, 0xFD]);
    let e = errors(Cpu::Mos6502, ".org $C000\n bne far\n .fill 200\nfar: nop\n");
    assert!(e[0].contains("t.asm:2") && e[0].contains("range"), "{e:?}");
}

#[test]
fn undocumented_6502_opcodes_need_the_flag() {
    let e = errors(Cpu::Mos6502, ".org 0\n lax $10\n");
    assert!(e[0].contains("undocumented"), "{e:?}");
    // `*nop` is the undocumented implied NOP (the first of its six encodings), plain `nop` the real one.
    let asm = run_with(Cpu::Mos6502, ".org 0\n lax $10\n *nop\n nop\n", &[], true).unwrap();
    assert_eq!(asm.segments[0].bytes, [0xA7, 0x10, 0x1A, 0xEA]);
    // The directive does the same as the flag, and can be switched off again.
    let asm = run(
        Cpu::Mos6502,
        ".org 0\n.illegal\n lax $10\n.illegal off\n nop\n",
    )
    .unwrap();
    assert_eq!(asm.segments[0].bytes, [0xA7, 0x10, 0xEA]);
}

#[test]
fn a_z80_program() {
    let b = bytes(
        Cpu::Z80,
        "        org 0x8000
start:  ld hl,msg
        ld b,len
next:   ld a,(hl)
        rst 0x10
        inc hl
        djnz next
        ret
msg:    db \"HELLO\"
len     equ $ - msg
",
    );
    assert_eq!(
        b,
        [
            0x21, 0x0B, 0x80, // ld hl,msg
            0x06, 0x05, // ld b,len
            0x7E, // ld a,(hl)
            0xD7, // rst 0x10
            0x23, // inc hl
            0x10, 0xFB, // djnz next
            0xC9, // ret
            b'H', b'E', b'L', b'L', b'O',
        ]
    );
}

#[test]
fn z80_operand_forms() {
    let b = bytes(
        Cpu::Z80,
        ".org 0x4000
        ld a,5
        ld a,-1
        ld (ix+5),0x12
        ld a,(iy-3)
        ld (0x5C78),a
        ld hl,(0x5C78)
        ld (0x5C78),bc
        add a,b
        add b
        sub 3
        and a
        cp (hl)
        jp (hl)
        jp nz,0x1234
        call 0x0556
        ret nz
        out (0xFE),a
        in a,(0xFE)
        out (c),a
        im 1
        bit 3,(ix+2)
        set 7,(iy-1)
        rlc (ix+4)
        res 0,h
        ex af,af'
        ld sp,hl
        push ix
        ld a,ixh
        sbc hl,de
",
    );
    assert_eq!(
        b,
        [
            0x3E, 0x05, // ld a,5
            0x3E, 0xFF, // ld a,-1
            0xDD, 0x36, 0x05, 0x12, // ld (ix+5),0x12
            0xFD, 0x7E, 0xFD, // ld a,(iy-3)
            0x32, 0x78, 0x5C, // ld (0x5C78),a
            0x2A, 0x78, 0x5C, // ld hl,(0x5C78)
            0xED, 0x43, 0x78, 0x5C, // ld (0x5C78),bc
            0x80, // add a,b
            0x80, // add b
            0xD6, 0x03, // sub 3
            0xA7, // and a
            0xBE, // cp (hl)
            0xE9, // jp (hl)
            0xC2, 0x34, 0x12, // jp nz,0x1234
            0xCD, 0x56, 0x05, // call 0x0556
            0xC0, // ret nz
            0xD3, 0xFE, // out (0xFE),a
            0xDB, 0xFE, // in a,(0xFE)
            0xED, 0x79, // out (c),a
            0xED, 0x56, // im 1
            0xDD, 0xCB, 0x02, 0x5E, // bit 3,(ix+2)
            0xFD, 0xCB, 0xFF, 0xFE, // set 7,(iy-1)
            0xDD, 0xCB, 0x04, 0x06, // rlc (ix+4)
            0xCB, 0x84, // res 0,h
            0x08, // ex af,af'
            0xF9, // ld sp,hl
            0xDD, 0xE5, // push ix
            0xDD, 0x7C, // ld a,ixh
            0xED, 0x52, // sbc hl,de
        ]
    );
}

#[test]
fn z80_relative_jumps_take_absolute_targets() {
    let b = bytes(
        Cpu::Z80,
        ".org 0x8000\nloop: jr loop\n jr nz,loop\n jr fwd\n nop\nfwd: djnz loop\n",
    );
    assert_eq!(b, [0x18, 0xFE, 0x20, 0xFC, 0x18, 0x01, 0x00, 0x10, 0xF7]);
    let e = errors(Cpu::Z80, ".org 0x8000\n jr far\n .fill 200\nfar: nop\n");
    assert!(e[0].contains("out of range"), "{e:?}");
}

#[test]
fn data_directives() {
    let b = bytes(
        Cpu::Mos6502,
        ".org $2000
        .byte 1, 2+3, 'A', \"hi\", $FF
        .word $1234, label
        .fill 3, $EA
        .fill 2
        .align 8, $FF
label:  .asciiz \"ok\"
",
    );
    assert_eq!(
        b,
        [
            1, 5, 0x41, b'h', b'i', 0xFF, // .byte
            0x34, 0x12, 0x10, 0x20, // .word (label = $2010)
            0xEA, 0xEA, 0xEA, 0, 0,    // fill
            0xFF, // align to $2010
            b'o', b'k', 0
        ]
    );
}

#[test]
fn org_starts_segments_and_continuing_org_merges() {
    let asm = run(
        Cpu::Mos6502,
        ".org $1000\n nop\n.org $1001\n nop\n.org $3000\n nop\n",
    )
    .unwrap();
    assert_eq!(asm.segments.len(), 2);
    assert_eq!(asm.segments[0].start, 0x1000);
    assert_eq!(asm.segments[0].bytes, [0xEA, 0xEA]);
    assert_eq!(asm.segments[1].start, 0x3000);

    let e = errors(Cpu::Mos6502, ".org $1000\n .fill 16\n.org $1008\n nop\n");
    assert!(e[0].contains("overlap"), "{e:?}");
    // `* = addr` is .org.
    let b = bytes(Cpu::Mos6502, "* = $C000\n here: jmp here\n");
    assert_eq!(b, [0x4C, 0x00, 0xC0]);
}

#[test]
fn local_labels_are_scoped_to_the_last_global_label() {
    let b = bytes(
        Cpu::Z80,
        ".org 0x100
one:    ld b,3
@loop:  djnz @loop
two:    ld b,4
@loop:  djnz @loop
",
    );
    assert_eq!(b, [0x06, 0x03, 0x10, 0xFE, 0x06, 0x04, 0x10, 0xFE]);
    let e = errors(Cpu::Z80, ".org 0\na: nop\n@x: nop\n@x: nop\n");
    assert!(e[0].contains("already defined"), "{e:?}");
}

#[test]
fn macros_with_arguments_and_their_own_local_labels() {
    let b = bytes(
        Cpu::Mos6502,
        ".org $C000
.macro delay count
        ldx #count
@l:     dex
        bne @l
.endm
        delay 3
        delay 5
        rts
",
    );
    assert_eq!(
        b,
        [0xA2, 3, 0xCA, 0xD0, 0xFD, 0xA2, 5, 0xCA, 0xD0, 0xFD, 0x60]
    );
}

#[test]
fn conditionals_and_defines() {
    let src = ".org 0
.if DEBUG
        .byte 1
.else
        .byte 2
.endif
.ifdef EXTRA
        .byte 3
.endif
.ifndef EXTRA
        .byte 4
.endif
";
    let mut opts = Options {
        cpu: Some(Cpu::Mos6502),
        ..Options::default()
    };
    opts.defines = vec![("DEBUG".into(), 0)];
    let asm = assemble("t.asm", src, &opts, &Mem(HashMap::new())).unwrap();
    assert_eq!(asm.segments[0].bytes, [2, 4]);
    opts.defines = vec![("DEBUG".into(), 1), ("EXTRA".into(), 1)];
    let asm = assemble("t.asm", src, &opts, &Mem(HashMap::new())).unwrap();
    assert_eq!(asm.segments[0].bytes, [1, 3]);
    let e = errors(Cpu::Mos6502, ".if 1\n nop\n");
    assert!(e.iter().any(|m| m.contains("endif")), "{e:?}");
    let e = errors(Cpu::Mos6502, ".endif\n");
    assert!(e[0].contains("without"), "{e:?}");
}

#[test]
fn include_and_incbin_read_files() {
    let asm = run_with(
        Cpu::Mos6502,
        ".org $1000\n.include \"defs.inc\"\n lda #VALUE\n.incbin \"blob.bin\", 1, 2\n",
        &[("defs.inc", b"VALUE = 7\n"), ("blob.bin", &[9, 8, 7, 6])],
        false,
    )
    .unwrap();
    assert_eq!(asm.segments[0].bytes, [0xA9, 7, 8, 7]);
    let e = run_with(Cpu::Mos6502, ".include \"nope.inc\"\n", &[], false).unwrap_err();
    assert!(e[0].contains("nope.inc"), "{e:?}");
}

#[test]
fn the_current_address_and_expressions() {
    let b = bytes(
        Cpu::Z80,
        ".org 0x9000
        ld hl,$ + 3
        ld de,* + 6
        ld bc,(1 << 4) | 3
",
    );
    assert_eq!(b, [0x21, 0x03, 0x90, 0x11, 0x09, 0x90, 0x01, 0x13, 0x00]);
}

#[test]
fn assert_and_error_directives() {
    assert!(run(Cpu::Mos6502, ".org 0\n.assert 1 == 1\n").is_ok());
    let e = errors(
        Cpu::Mos6502,
        ".org 0\nx = 3\n.assert x == 4, \"x must be 4\"\n",
    );
    assert!(e[0].contains("x must be 4"), "{e:?}");
    let e = errors(Cpu::Mos6502, ".error \"stop here\"\n");
    assert!(e[0].contains("stop here"), "{e:?}");
}

#[test]
fn diagnostics_name_file_and_line() {
    let e = errors(Cpu::Mos6502, ".org 0\n nop\n bogus $10\n lda #300\n");
    assert!(
        e[0].starts_with("t.asm:3:") && e[0].contains("bogus"),
        "{e:?}"
    );
    assert!(
        e[1].starts_with("t.asm:4:") && e[1].contains("300"),
        "{e:?}"
    );
    let e = errors(Cpu::Mos6502, ".org 0\n lda missing\n");
    assert!(e[0].contains("undefined symbol \"missing\""), "{e:?}");
    let e = errors(Cpu::Z80, "start: nop\n");
    assert!(e[0].contains(".org"), "{e:?}");
    let e = errors(Cpu::Z80, ".org 0\nx = 1\nx = 2\n");
    assert!(e[0].contains("already defined"), "{e:?}");
}

#[test]
fn the_cpu_can_come_from_the_source() {
    let opts = Options::default();
    let asm = assemble(
        "t.asm",
        ".cpu z80\n.org 0\n ld a,1\n",
        &opts,
        &Mem(HashMap::new()),
    )
    .unwrap();
    assert_eq!(asm.cpu, Cpu::Z80);
    assert_eq!(asm.segments[0].bytes, [0x3E, 1]);
    let e = assemble("t.asm", ".org 0\n nop\n", &opts, &Mem(HashMap::new())).unwrap_err();
    assert!(e[0].to_string().contains("no CPU selected"), "{e:?}");
}

#[test]
fn symbol_and_listing_output() {
    let asm = run(
        Cpu::Mos6502,
        ".org $C000\nSIZE = 4\nstart: lda #SIZE\n rts\n",
    )
    .unwrap();
    let syms = rcasm::symbols_text(&asm, false, rcasm::SymbolStyle::Rc);
    assert_eq!(syms, "start = $C000\n");
    let all = rcasm::symbols_text(&asm, true, rcasm::SymbolStyle::Vice);
    assert!(
        all.contains("al C:c000 .start") && all.contains("al C:0004 .SIZE"),
        "{all}"
    );
    // What the assembler writes, the debugger's symbol reader accepts.
    let table = machine::SymbolTable::parse(&syms).unwrap();
    assert_eq!(table.name(0xC000), Some("start"));
    let listing = rcasm::listing_text(&asm);
    assert!(listing.contains("C000  A9 04"), "{listing}");
    assert!(listing.contains("C002  60"), "{listing}");
}
