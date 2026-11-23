//! The HuC6280 back end: everything the core's disassembler prints assembles back, and the instruction forms
//! that matter (block transfers, bit branches, mapper and register instructions, zero page versus absolute)
//! give the documented bytes.

use rcasm::{Cpu, DiskFiles, Options, assemble};

fn asm(source: &str) -> Result<Vec<u8>, String> {
    let opts = Options {
        cpu: Some(Cpu::Huc6280),
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
    asm(&format!(".org $E000\n{line}\n")).unwrap_or_else(|e| panic!("{line}: {e}"))
}

#[test]
fn every_opcode_assembles_back() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for op in 0..=255u8 {
        for operand in [[0x10u8, 0x00], [0x34, 0x12], [0xFE, 0xFF], [0x00, 0x20]] {
            let addr = 0xE000u16;
            let bytes = [op, operand[0], operand[1], 0x00, 0x00, 0x00, 0x00];
            let (text, len) = huc6280::disassemble::disassemble(&bytes, addr);
            checked += 1;
            // a relative target out of reach cannot come from a real encoding; skip those
            match asm(&format!(".org $E000\n{text}\n")) {
                Err(e) if e.contains("branch target") => {}
                Err(e) => failures.push(format!("{bytes:02X?}: \"{text}\" -> {e}")),
                Ok(again) => {
                    let (text2, len2) = huc6280::disassemble::disassemble(&again, addr);
                    if text2 != text || len2 != len || again.len() != len {
                        failures.push(format!("{bytes:02X?}: \"{text}\" -> {again:02X?} \"{text2}\""));
                    }
                }
            }
        }
    }
    assert!(checked > 1000);
    assert!(
        failures.is_empty(),
        "{} of {checked} failed:\n{}",
        failures.len(),
        failures.iter().take(20).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn mapper_and_register_instructions() {
    assert_eq!(at("tam #$01"), [0x53, 0x01]);
    assert_eq!(at("tma #$80"), [0x43, 0x80]);
    assert_eq!(at("st0 #$05"), [0x03, 0x05]);
    assert_eq!(at("st1 #$06"), [0x13, 0x06]);
    assert_eq!(at("st2 #$07"), [0x23, 0x07]);
    assert_eq!(at("csh"), [0xD4]);
    assert_eq!(at("csl"), [0x54]);
    assert_eq!(at("set"), [0xF4]);
    assert_eq!(at("clx"), [0x82]);
    assert_eq!(at("cly"), [0xC2]);
    assert_eq!(at("cla"), [0x62]);
    assert_eq!(at("sax"), [0x22]);
    assert_eq!(at("say"), [0x42]);
    assert_eq!(at("sxy"), [0x02]);
}

#[test]
fn block_transfers_take_three_words() {
    assert_eq!(at("tii $4000,$5000,$0100"), [0x73, 0x00, 0x40, 0x00, 0x50, 0x00, 0x01]);
    assert_eq!(at("tdd $4000,$5000,$0100"), [0xC3, 0x00, 0x40, 0x00, 0x50, 0x00, 0x01]);
    assert_eq!(at("tin $4000,$5000,$0100"), [0xD3, 0x00, 0x40, 0x00, 0x50, 0x00, 0x01]);
    assert_eq!(at("tia $4000,$5000,$0100"), [0xE3, 0x00, 0x40, 0x00, 0x50, 0x00, 0x01]);
    assert_eq!(at("tai $4000,$5000,$0100"), [0xF3, 0x00, 0x40, 0x00, 0x50, 0x00, 0x01]);
}

#[test]
fn block_transfer_with_later_symbols_keeps_its_size() {
    let src = ".org $E000\n tii src,dst,len\n nop\nsrc = $10\ndst = $20\nlen = $30\n";
    assert_eq!(asm(src).unwrap(), [0x73, 0x10, 0x00, 0x20, 0x00, 0x30, 0x00, 0xEA]);
}

#[test]
fn bit_instructions_and_branches() {
    assert_eq!(at("smb0 $20"), [0x87, 0x20]);
    assert_eq!(at("rmb4 $20"), [0x47, 0x20]);
    // BBR3 at $E000: opcode $3F, zero page, offset to $E010 is $E010-$E003.
    assert_eq!(asm(".org $E000\n bbr3 $20,there\n.org $E010\nthere: nop\n").unwrap()[..3], [0x3F, 0x20, 0x0D]);
    assert_eq!(asm(".org $E000\n bbs7 $20,$E000\n").unwrap(), [0xFF, 0x20, 0xFD]);
    assert_eq!(at("tsb $20"), [0x04, 0x20]);
    assert_eq!(at("trb $2000"), [0x1C, 0x00, 0x20]);
}

#[test]
fn a_bit_branch_out_of_range_names_the_distance() {
    let err = asm(".org $E000\n bbr0 $20,far\n.org $E200\nfar: nop\n").unwrap_err();
    assert!(err.contains("branch target"), "{err}");
}

#[test]
fn branches_and_calls() {
    assert_eq!(asm(".org $E000\n bra there\n nop\nthere: nop\n").unwrap()[..2], [0x80, 0x01]);
    assert_eq!(asm(".org $E000\n bsr there\n nop\nthere: rts\n").unwrap()[..2], [0x44, 0x01]);
    assert_eq!(at("jsr $E100"), [0x20, 0x00, 0xE1]);
    assert_eq!(at("jmp ($E100,x)"), [0x7C, 0x00, 0xE1]);
    assert_eq!(at("stz $2000,x"), [0x9E, 0x00, 0x20]);
}

#[test]
fn test_instructions_choose_zero_page_or_absolute() {
    assert_eq!(at("tst #$12,$20"), [0x83, 0x12, 0x20]);
    assert_eq!(at("tst #$12,$2000"), [0x93, 0x12, 0x00, 0x20]);
    assert_eq!(at("tst #$12,$20,x"), [0xA3, 0x12, 0x20]);
    assert_eq!(at("tst #$12,$2000,x"), [0xB3, 0x12, 0x00, 0x20]);
}

#[test]
fn zero_page_versus_absolute_like_the_6502() {
    assert_eq!(at("lda $20"), [0xA5, 0x20]);
    assert_eq!(at("lda $0020"), [0xAD, 0x20, 0x00]);
    assert_eq!(at("lda $2000"), [0xAD, 0x00, 0x20]);
    assert_eq!(at("lda ($20)"), [0xB2, 0x20]);
    assert_eq!(at("lda ($20,x)"), [0xA1, 0x20]);
    assert_eq!(at("lda ($20),y"), [0xB1, 0x20]);
    assert_eq!(at("asl a"), [0x0A]);
}

#[test]
fn unknown_instructions_are_errors() {
    assert!(asm(".org $E000\n tya x\n").is_err());
    assert!(asm(".org $E000\n xyzzy\n").is_err());
}

fn banked(source: &str) -> Vec<(u16, u16, Vec<u8>)> {
    let opts = Options {
        cpu: Some(Cpu::Huc6280),
        ..Options::default()
    };
    assemble("t.asm", source, &opts, &DiskFiles)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .segments
        .into_iter()
        .map(|s| (s.bank, s.start, s.bytes))
        .collect()
}

#[test]
fn banks_keep_same_addresses_apart() {
    let segs = banked(".bank 0\n.org $E000\n nop\n.bank 1\n.org $6000\n clx\n.bank 2\n.org $6000\n cly\n");
    assert_eq!(
        segs,
        vec![
            (0, 0xE000, vec![0xEA]),
            (1, 0x6000, vec![0x82]),
            (2, 0x6000, vec![0xC2]),
        ]
    );
}

#[test]
fn overlap_is_per_bank_and_bank_needs_the_huc6280() {
    assert!(asm(".bank 1\n.org $6000\n nop\n.bank 1\n.org $6000\n nop\n").is_err());
    let z80 = Options {
        cpu: Some(Cpu::Z80),
        ..Options::default()
    };
    assert!(assemble("t.asm", ".bank 1\n", &z80, &DiskFiles).is_err());
}
