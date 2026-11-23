//! The 65816 back end: everything the core's disassembler prints assembles back (in each width state), and the
//! documented instruction forms give the documented bytes.

use rcasm::{Cpu, DiskFiles, Options, assemble};

fn asm(source: &str) -> Result<Vec<u8>, String> {
    let opts = Options {
        cpu: Some(Cpu::W65816),
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

fn at(line: &str) -> Vec<u8> {
    asm(&format!(".org $008000\n{line}\n")).unwrap_or_else(|e| panic!("{line}: {e}"))
}

fn state(m16: bool, x16: bool) -> String {
    format!(
        ".{}\n.{}\n",
        if m16 { "a16" } else { "a8" },
        if x16 { "i16" } else { "i8" }
    )
}

#[test]
fn every_opcode_assembles_back_in_each_width_state() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for (m16, x16) in [(false, false), (true, false), (false, true), (true, true)] {
        for op in 0..=255u8 {
            for operand in [
                [0x10u8, 0x00, 0x00],
                [0x34, 0x12, 0x56],
                [0xFE, 0xFF, 0x7E],
                [0x00, 0x20, 0x01],
            ] {
                let bytes = [op, operand[0], operand[1], operand[2]];
                let (text, len) = w65c816::disassemble::disassemble(&bytes, 0x00_8000, m16, x16);
                checked += 1;
                let src = format!("{}.org $008000\n{text}\n", state(m16, x16));
                match asm(&src) {
                    Err(e) if e.contains("branch target") => {}
                    Err(e) => failures.push(format!("{bytes:02X?} {m16} {x16}: \"{text}\" -> {e}")),
                    Ok(again) => {
                        let (text2, len2) =
                            w65c816::disassemble::disassemble(&again, 0x00_8000, m16, x16);
                        if text2 != text || len2 != len || again.len() != len {
                            failures.push(format!(
                                "{bytes:02X?}: \"{text}\" -> {again:02X?} \"{text2}\""
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 4000);
    assert!(
        failures.is_empty(),
        "{} of {checked} failed:\n{}",
        failures.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn immediates_follow_the_register_widths() {
    assert_eq!(at("lda #$12"), [0xA9, 0x12]);
    assert_eq!(
        asm(&format!("{}.org $8000\n lda #$1234\n", state(true, false))).unwrap(),
        [0xA9, 0x34, 0x12]
    );
    assert_eq!(
        asm(&format!("{}.org $8000\n ldx #$1234\n", state(false, true))).unwrap(),
        [0xA2, 0x34, 0x12]
    );
    // an accumulator 16-bit does not widen LDX
    assert_eq!(
        asm(&format!("{}.org $8000\n ldx #$12\n", state(true, false))).unwrap(),
        [0xA2, 0x12]
    );
    // a 16-bit value in an 8-bit register says why
    let e = asm(".org $8000\n lda #$1234\n").unwrap_err();
    assert!(
        e.contains("does not fit in a byte") && e.contains("16"),
        "{e}"
    );
}

#[test]
fn rep_and_sep_track_the_widths_when_smart() {
    assert_eq!(
        asm(".org $8000\n rep #$30\n lda #$1234\n ldx #$5678\n sep #$20\n lda #$12\n").unwrap(),
        [
            0xC2, 0x30, 0xA9, 0x34, 0x12, 0xA2, 0x78, 0x56, 0xE2, 0x20, 0xA9, 0x12
        ]
    );
    // with smart off they do not
    assert_eq!(
        asm(".org $8000\n.smart off\n rep #$30\n lda #$12\n").unwrap(),
        [0xC2, 0x30, 0xA9, 0x12]
    );
    // the suffixes decide outright
    assert_eq!(at("lda.w #$1234"), [0xA9, 0x34, 0x12]);
}

#[test]
fn address_sizes() {
    assert_eq!(at("lda $12"), [0xA5, 0x12]);
    assert_eq!(at("lda $0012"), [0xAD, 0x12, 0x00]);
    assert_eq!(at("lda $1234"), [0xAD, 0x34, 0x12]);
    assert_eq!(at("lda $123456"), [0xAF, 0x56, 0x34, 0x12]);
    assert_eq!(at("lda $7E0012"), [0xAF, 0x12, 0x00, 0x7E]);
    assert_eq!(at("lda $123456,x"), [0xBF, 0x56, 0x34, 0x12]);
    assert_eq!(at("lda $1234,x"), [0xBD, 0x34, 0x12]);
    assert_eq!(at("lda $1234,y"), [0xB9, 0x34, 0x12]);
    assert_eq!(at("lda $12,x"), [0xB5, 0x12]);
    assert_eq!(at("lda.l $12"), [0xAF, 0x12, 0x00, 0x00]);
    assert_eq!(at("lda.w $12"), [0xAD, 0x12, 0x00]);
    assert_eq!(at("lda.b $12"), [0xA5, 0x12]);
    assert_eq!(at("lda ($12)"), [0xB2, 0x12]);
    assert_eq!(at("lda ($12,x)"), [0xA1, 0x12]);
    assert_eq!(at("lda ($12),y"), [0xB1, 0x12]);
    assert_eq!(at("lda [$12]"), [0xA7, 0x12]);
    assert_eq!(at("lda [$12],y"), [0xB7, 0x12]);
    assert_eq!(at("lda $12,s"), [0xA3, 0x12]);
    assert_eq!(at("lda ($12,s),y"), [0xB3, 0x12]);
    assert_eq!(at("asl a"), [0x0A]);
    assert_eq!(at("jmp ($1234)"), [0x6C, 0x34, 0x12]);
    assert_eq!(at("jmp ($1234,x)"), [0x7C, 0x34, 0x12]);
    assert_eq!(at("jml [$1234]"), [0xDC, 0x34, 0x12]);
    assert_eq!(at("jml $123456"), [0x5C, 0x56, 0x34, 0x12]);
    assert_eq!(at("jsl $123456"), [0x22, 0x56, 0x34, 0x12]);
    assert_eq!(at("jsr $1234"), [0x20, 0x34, 0x12]);
    assert_eq!(at("jsr ($1234,x)"), [0xFC, 0x34, 0x12]);
    assert_eq!(at("pea $1234"), [0xF4, 0x34, 0x12]);
    assert_eq!(at("pei ($12)"), [0xD4, 0x12]);
}

#[test]
fn a_24_bit_label_for_jmp_names_the_fix() {
    let e = asm(".org $008000\nthere = $808010\n jmp there\n").unwrap_err();
    assert!(e.contains("JML"), "{e}");
    assert_eq!(at("jml $808010"), [0x5C, 0x10, 0x80, 0x80]);
}

#[test]
fn block_moves_take_source_then_destination_and_store_destination_first() {
    assert_eq!(at("mvn $7E,$7F"), [0x54, 0x7F, 0x7E]);
    assert_eq!(at("mvp $01,$02"), [0x44, 0x02, 0x01]);
}

#[test]
fn branches_and_relative_long() {
    assert_eq!(
        asm(".org $8000\n bra there\n nop\nthere: nop\n").unwrap()[..2],
        [0x80, 0x01]
    );
    assert_eq!(
        asm(".org $8000\n brl there\n nop\nthere: nop\n").unwrap()[..3],
        [0x82, 0x01, 0x00]
    );
    assert_eq!(
        asm(".org $8000\n per there\n nop\nthere: nop\n").unwrap()[..3],
        [0x62, 0x01, 0x00]
    );
    // BRL reaches back across the bank
    assert_eq!(
        asm(".org $8000\nhere: brl here\n").unwrap(),
        [0x82, 0xFD, 0xFF]
    );
    let e = asm(".org $8000\n bne far\n.org $8200\nfar: nop\n").unwrap_err();
    assert!(e.contains("branch target"), "{e}");
    // a branch into another bank
    let e = asm(".org $008000\n bne far\n.org $018010\nfar: nop\n").unwrap_err();
    assert!(e.contains("another bank"), "{e}");
}

#[test]
fn a_forward_reference_to_a_long_label_keeps_its_size() {
    // the label is above $FFFF: lda needs .l; a forward use with the suffix works
    let src = ".org $008000\n lda.l data\n nop\n.org $7E1000\ndata: .byte 1\n";
    let bytes = asm(src).unwrap();
    assert_eq!(bytes[..5], [0xAF, 0x00, 0x10, 0x7E, 0xEA]);
}

#[test]
fn big_addresses_in_a_16_bit_cpu_are_errors() {
    let opts = Options {
        cpu: Some(Cpu::Mos6502),
        ..Options::default()
    };
    assert!(assemble("t", ".org $808000\n", &opts, &DiskFiles).is_err());
    // the 65816 takes it
    assert_eq!(asm(".org $808000\n nop\n").unwrap(), [0xEA]);
}

#[test]
fn unknown_instructions_are_errors() {
    assert!(asm(".org $8000\n xyzzy\n").is_err());
    assert!(asm(".org $8000\n lda\n").is_err());
    assert!(asm(".org $8000\n nop #1\n").is_err());
}
