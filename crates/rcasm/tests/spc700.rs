//! The SPC700 back end: everything the core's disassembler prints assembles back, and the documented forms give the
//! documented bytes.

use rcasm::{Cpu, DiskFiles, Options, assemble};

fn asm(source: &str) -> Result<Vec<u8>, String> {
    let opts = Options {
        cpu: Some(Cpu::Spc700),
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
    asm(&format!(".org $0200\n{line}\n")).unwrap_or_else(|e| panic!("{line}: {e}"))
}

#[test]
fn every_opcode_assembles_back() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for op in 0..=255u8 {
        for operand in [
            [0x10u8, 0x00, 0x00],
            [0x34, 0x12, 0x56],
            [0xFE, 0xFF, 0x7E],
            [0x00, 0x20, 0x01],
            [0x05, 0x40, 0xA0],
        ] {
            let bytes = [op, operand[0], operand[1], operand[2]];
            let (text, len) = spc700::disassemble::disassemble(&bytes, 0x0200);
            checked += 1;
            match asm(&format!(".org $0200\n{text}\n")) {
                Err(e) if e.contains("branch target") => {}
                Err(e) => failures.push(format!("{bytes:02X?}: \"{text}\" -> {e}")),
                Ok(again) => {
                    let (text2, len2) = spc700::disassemble::disassemble(&again, 0x0200);
                    if text2 != text || len2 != len || again.len() != len {
                        failures.push(format!(
                            "{bytes:02X?}: \"{text}\" -> {again:02X?} \"{text2}\""
                        ));
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
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn moves_and_addressing() {
    assert_eq!(at("mov a,#$12"), [0xE8, 0x12]);
    assert_eq!(at("mov a,$12"), [0xE4, 0x12]);
    assert_eq!(at("mov a,!$1234"), [0xE5, 0x34, 0x12]);
    assert_eq!(at("mov a,$1234"), [0xE5, 0x34, 0x12]);
    assert_eq!(at("mov a,$12+x"), [0xF4, 0x12]);
    assert_eq!(at("mov a,!$1234+x"), [0xF5, 0x34, 0x12]);
    assert_eq!(at("mov a,!$1234+y"), [0xF6, 0x34, 0x12]);
    assert_eq!(at("mov a,[$12+x]"), [0xE7, 0x12]);
    assert_eq!(at("mov a,[$12]+y"), [0xF7, 0x12]);
    assert_eq!(at("mov a,(x)"), [0xE6]);
    assert_eq!(at("mov a,(x)+"), [0xBF]);
    assert_eq!(at("mov (x)+,a"), [0xAF]);
    assert_eq!(at("mov x,#$12"), [0xCD, 0x12]);
    assert_eq!(at("mov y,$12+x"), [0xFB, 0x12]);
    assert_eq!(at("mov $12,a"), [0xC4, 0x12]);
    assert_eq!(at("mov !$1234,a"), [0xC5, 0x34, 0x12]);
    assert_eq!(at("mov $12,#$34"), [0x8F, 0x34, 0x12]);
    // dd,ds: the destination is written first and stored second
    assert_eq!(at("mov $22,$11"), [0xFA, 0x11, 0x22]);
    assert_eq!(at("movw ya,$12"), [0xBA, 0x12]);
    assert_eq!(at("movw $12,ya"), [0xDA, 0x12]);
    assert_eq!(at("mov x,sp"), [0x9D]);
    assert_eq!(at("mov sp,x"), [0xBD]);
}

#[test]
fn arithmetic_and_logic() {
    assert_eq!(at("adc a,#$05"), [0x88, 0x05]);
    assert_eq!(at("adc $22,$11"), [0x89, 0x11, 0x22]);
    assert_eq!(at("adc $22,#$11"), [0x98, 0x11, 0x22]);
    assert_eq!(at("adc (x),(y)"), [0x99]);
    assert_eq!(at("or a,$12"), [0x04, 0x12]);
    assert_eq!(at("cmp x,#$10"), [0xC8, 0x10]);
    assert_eq!(at("cmp y,!$1234"), [0x5E, 0x34, 0x12]);
    assert_eq!(at("addw ya,$12"), [0x7A, 0x12]);
    assert_eq!(at("mul ya"), [0xCF]);
    assert_eq!(at("div ya,x"), [0x9E]);
    assert_eq!(at("xcn a"), [0x9F]);
    assert_eq!(at("inc $12+x"), [0xBB, 0x12]);
    assert_eq!(at("asl a"), [0x1C]);
}

#[test]
fn bit_operations() {
    assert_eq!(at("set1 $12.3"), [0x62, 0x12]);
    assert_eq!(at("clr1 $12.7"), [0xF2, 0x12]);
    assert_eq!(at("or1 c,$1234.5"), [0x0A, 0x34, 0xB2]);
    assert_eq!(at("or1 c,/$0234.5"), [0x2A, 0x34, 0xA2]);
    assert_eq!(at("mov1 c,$0123.0"), [0xAA, 0x23, 0x01]);
    assert_eq!(at("mov1 $0123.4,c"), [0xCA, 0x23, 0x81]);
    assert_eq!(at("not1 $0100.7"), [0xEA, 0x00, 0xE1]);
    assert!(
        asm(".org $200\n or1 c,$2000.1\n")
            .unwrap_err()
            .contains("8 KB")
    );
}

#[test]
fn branches_and_calls() {
    assert_eq!(
        asm(".org $0200\n bra there\n nop\nthere: nop\n").unwrap()[..2],
        [0x2F, 0x01]
    );
    assert_eq!(
        asm(".org $0200\n bbs $12.3,there\n nop\nthere: nop\n").unwrap()[..3],
        [0x63, 0x12, 0x01]
    );
    assert_eq!(
        asm(".org $0200\n cbne $12,there\n nop\nthere: nop\n").unwrap()[..3],
        [0x2E, 0x12, 0x01]
    );
    assert_eq!(
        asm(".org $0200\n dbnz y,there\n nop\nthere: nop\n").unwrap()[..2],
        [0xFE, 0x01]
    );
    assert_eq!(at("call !$1234"), [0x3F, 0x34, 0x12]);
    assert_eq!(at("jmp !$1234"), [0x5F, 0x34, 0x12]);
    assert_eq!(at("jmp [!$1234+x]"), [0x1F, 0x34, 0x12]);
    assert_eq!(at("tcall 5"), [0x51]);
    assert_eq!(at("tcall 15"), [0xF1]);
    assert_eq!(at("pcall $f0"), [0x4F, 0xF0]);
    assert_eq!(at("ret"), [0x6F]);
    assert_eq!(at("reti"), [0x7F]);
    assert_eq!(at("brk"), [0x0F]);
    let e = asm(".org $200\n bne far\n.org $400\nfar: nop\n").unwrap_err();
    assert!(e.contains("branch target"), "{e}");
    assert!(asm(".org $200\n tcall 16\n").is_err());
}

#[test]
fn forward_references_keep_their_size() {
    // `later` is a direct page address defined below: the first pass assumed absolute, the second must agree
    let src = ".org $0200\n mov a,later\n nop\nlater = $12\n";
    let b = asm(src).unwrap();
    assert_eq!(b[..4], [0xE5, 0x12, 0x00, 0x00]);
}

#[test]
fn unknown_instructions_are_errors() {
    assert!(asm(".org $200\n xyzzy\n").is_err());
    assert!(asm(".org $200\n mov a\n").is_err());
    assert!(asm(".org $200\n mov a,#$100\n").is_err());
}
