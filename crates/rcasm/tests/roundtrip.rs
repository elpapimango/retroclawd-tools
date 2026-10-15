//! The disassemblers are the source of truth for instruction encodings. Everything they can print must
//! assemble back to bytes that print the same way. (Twin encodings of one instruction -- the mirrored
//! `NEG`, the undocumented `BIT` aliases, 6502 opcode twins -- may assemble to the canonical twin, so the
//! comparison is of text, not of bytes.)

use rcasm::{Cpu, DiskFiles, Options, assemble};

fn assemble_one(cpu: Cpu, addr: u16, line: &str) -> Result<Vec<u8>, String> {
    let opts = Options {
        cpu: Some(cpu),
        illegal: true,
        ..Options::default()
    };
    let source = format!(".org {addr}\n{line}\n");
    assemble("rt.asm", &source, &opts, &DiskFiles)
        .map(|a| {
            a.segments
                .into_iter()
                .next()
                .map(|s| s.bytes)
                .unwrap_or_default()
        })
        .map_err(|e| {
            e.iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })
}

#[test]
fn every_6502_opcode_assembles_back() {
    let operands: [[u8; 2]; 4] = [[0x10, 0x00], [0x34, 0x12], [0xFE, 0xFF], [0x00, 0x20]];
    let mut failures = Vec::new();
    let mut checked = 0;
    for op in 0..=255u8 {
        for operand in operands {
            let addr = 0xC000u16;
            let bytes = [op, operand[0], operand[1]];
            let (text, len) = mos6502::disassemble::disassemble(&bytes, addr);
            // A branch whose target is out of reach of a signed byte cannot come from a real opcode.
            checked += 1;
            match assemble_one(Cpu::Mos6502, addr, &text) {
                Err(e) => failures.push(format!("{bytes:02X?}: \"{text}\" -> {e}")),
                Ok(again) => {
                    let (text2, len2) = mos6502::disassemble::disassemble(&again, addr);
                    if text2 != text || len2 != len || again.len() != len {
                        failures.push(format!(
                            "{bytes:02X?}: \"{text}\" -> {again:02X?} \"{text2}\""
                        ));
                    }
                }
            }
        }
    }
    failures.sort();
    failures.dedup();
    assert!(
        failures.is_empty(),
        "{} of {checked} do not round-trip:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_z80_instruction_assembles_back() {
    let tail = [0x12u8, 0x34, 0x56];
    let mut candidates: Vec<Vec<u8>> = Vec::new();
    for op in 0..=255u8 {
        candidates.push([&[op][..], &tail].concat());
        for prefix in [0xCBu8, 0xED, 0xDD, 0xFD] {
            candidates.push([&[prefix, op][..], &tail].concat());
        }
        for prefix in [0xDDu8, 0xFD] {
            candidates.push(vec![prefix, 0xCB, 0x05, op]);
            candidates.push(vec![prefix, 0xCB, 0xFB, op]);
        }
    }
    let mut failures = Vec::new();
    let mut checked = 0;
    for bytes in candidates {
        let addr = 0x8000u16;
        let (text, len) = z80::disassemble::disassemble_at(&bytes, addr);
        if text.starts_with("DB ") {
            continue; // an unassigned encoding
        }
        // A DD/FD prefix in front of JR/DJNZ is ignored by the CPU and the disassembler does not
        // convert its displacement.
        if matches!(bytes[0], 0xDD | 0xFD) && (text.starts_with("JR") || text.starts_with("DJNZ")) {
            continue;
        }
        checked += 1;
        match assemble_one(Cpu::Z80, addr, &text) {
            Err(e) => failures.push(format!("{bytes:02X?}: \"{text}\" -> {e}")),
            Ok(again) => {
                let (text2, len2) = z80::disassemble::disassemble_at(&again, addr);
                if text2 != text || len2 != again.len() {
                    failures.push(format!(
                        "{bytes:02X?} (len {len}): \"{text}\" -> {again:02X?} \"{text2}\""
                    ));
                }
            }
        }
    }
    failures.sort();
    failures.dedup();
    assert!(
        failures.is_empty(),
        "{} of {checked} do not round-trip:\n{}",
        failures.len(),
        failures
            .iter()
            .take(60)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
