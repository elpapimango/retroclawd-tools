//! What rcdis writes, rcasm must turn back into the same bytes -- for hand-made programs, for random
//! bytes (which reach every opcode, twin and prefix combination) and, when the ROM images are around,
//! for real ROMs.

use machine::SymbolTable;
use rcasm::{Cpu, DiskFiles, Options as AsmOptions, assemble};
use rcdis::{Options, disassemble};

fn assemble_source(source: &str) -> Result<(u16, Vec<u8>), String> {
    let asm = assemble("out.asm", source, &AsmOptions::default(), &DiskFiles).map_err(|e| {
        e.iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    let (start, image) = rcasm::flat_image(&asm, 0x00)?;
    Ok((start, image))
}

fn round_trip(opts: &Options, bytes: &[u8]) -> String {
    let dis = disassemble(bytes, opts).expect("disassembles");
    let (start, again) = assemble_source(&dis.source)
        .unwrap_or_else(|e| panic!("the output does not assemble:\n{e}\n---\n{}", dis.source));
    assert_eq!(start, opts.org, "start address");
    assert_eq!(again.len(), bytes.len(), "length");
    if again != bytes {
        let at = again.iter().zip(bytes).position(|(a, b)| a != b).unwrap();
        panic!(
            "first difference at ${:04X}: assembled {:02X}, original {:02X}\n{}",
            opts.org as usize + at,
            again[at],
            bytes[at],
            dis.source
                .lines()
                .skip_while(|l| !l.contains(&format!("{:04X}", opts.org as usize + at)))
                .take(1)
                .collect::<String>()
        );
    }
    dis.source
}

/// A small deterministic generator (xorshift) so the "random" tests are repeatable.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn random_bytes_round_trip_on_the_6502() {
    for seed in 1..=8 {
        let bytes = noise(seed, 20_000);
        round_trip(&Options::new(Cpu::Mos6502, 0x2000), &bytes);
    }
}

fn random_bytes_round_trip_on_the_z80() {
    for seed in 1..=8 {
        let bytes = noise(seed, 20_000);
        round_trip(&Options::new(Cpu::Z80, 0x4000), &bytes);
    }
}

fn random_bytes_round_trip_in_flow_mode_too() {
    for seed in 1..=4 {
        let bytes = noise(seed * 7, 8_000);
        for cpu in [Cpu::Mos6502, Cpu::Z80] {
            let mut opts = Options::new(cpu, 0x8000);
            opts.entries = vec![0x8000, 0x8100, 0x9000];
            round_trip(&opts, &bytes);
        }
    }
}
/// Merged (every check kept): `random_bytes_round_trip_on_the_6502`, `random_bytes_round_trip_on_the_z80`, `random_bytes_round_trip_in_flow_mode_too`.
#[test]
fn random_bytes_round_trips() {
    random_bytes_round_trip_on_the_6502();
    random_bytes_round_trip_on_the_z80();
    random_bytes_round_trip_in_flow_mode_too();
}

fn flow_mode_keeps_data_between_routines_as_data() {
    // jsr sub / rts / .byte "HI" / sub: lda #1 / rts
    let program = [
        0x20, 0x06, 0xC0, // C000 jsr $C006
        0x60, // C003 rts
        0x48, 0x49, // C004 "HI" (data)
        0xA9, 0x01, // C006 lda #1
        0x60, // C008 rts
    ];
    let mut opts = Options::new(Cpu::Mos6502, 0xC000);
    opts.entries = vec![0xC000];
    let source = round_trip(&opts, &program);
    assert!(source.contains("JSR L_C006"), "{source}");
    assert!(source.contains("L_C006:"), "{source}");
    assert!(source.contains(".byte $48,$49"), "{source}");
    assert!(source.contains("following flow from $C000"), "{source}");
    // Linear mode decodes the same bytes as instructions.
    let linear = round_trip(&Options::new(Cpu::Mos6502, 0xC000), &program);
    assert!(!linear.contains(".byte $48"), "{linear}");
}

fn z80_flow_follows_jumps_calls_and_relative_branches() {
    let program = [
        0xCD, 0x08, 0x80, // 8000 call 8008
        0x18, 0x03, // 8003 jr 8008 (skips the data)
        0xDE, 0xAD, 0xBE, // 8005 data
        0x3E, 0x07, // 8008 ld a,7
        0xC9, // 800A ret
    ];
    let mut opts = Options::new(Cpu::Z80, 0x8000);
    opts.entries = vec![0x8000];
    let source = round_trip(&opts, &program);
    assert!(source.contains("CALL L_8008"), "{source}");
    assert!(source.contains("JR L_8008"), "{source}");
    assert!(source.contains(".byte $DE,$AD,$BE"), "{source}");
}
/// Merged (every check kept): `flow_mode_keeps_data_between_routines_as_data`, `z80_flow_follows_jumps_calls_and_relative_branches`.
#[test]
fn flow_mode() {
    flow_mode_keeps_data_between_routines_as_data();
    z80_flow_follows_jumps_calls_and_relative_branches();
}

fn symbol_files_name_addresses_inside_and_outside_the_image() {
    let program = [
        0x20, 0xD2, 0xFF, // jsr $FFD2
        0xA5, 0xC6, // lda $C6
        0xAD, 0x20, 0xD0, // lda $D020
        0x8D, 0x10, 0x00, // sta $0010 (absolute, value < 256)
        0x4C, 0x00, 0xC0, // jmp $C000
    ];
    let mut opts = Options::new(Cpu::Mos6502, 0xC000);
    opts.symbols =
        SymbolTable::parse("CHROUT = $FFD2\nNDX = $C6\nBORDER = $D020\nZP10 = $10\nmain = $C000\n")
            .unwrap();
    let source = round_trip(&opts, &program);
    assert!(source.contains("JSR CHROUT"), "{source}");
    assert!(source.contains("LDA NDX"), "{source}");
    assert!(source.contains("LDA BORDER"), "{source}");
    assert!(source.contains("main:"), "{source}");
    assert!(source.contains("JMP main"), "{source}");
    // An absolute operand with a small value keeps its four digits: naming it could make it zero page.
    assert!(source.contains("STA $0010"), "{source}");
    assert!(source.contains("CHROUT = $FFD2"), "{source}");
    assert!(source.contains("NDX = $00C6"), "{source}");
}

#[test]
fn an_encoding_the_assembler_cannot_reproduce_becomes_data() {
    // The six implied *NOPs print identically; only one can be written back as text.
    let bytes = [0x1A, 0x3A, 0x5A, 0x7A, 0xDA, 0xFA];
    let source = round_trip(&Options::new(Cpu::Mos6502, 0xC000), &bytes);
    assert!(source.contains(".byte"), "{source}");
    // A DD prefix in front of an instruction that ignores it.
    let source = round_trip(&Options::new(Cpu::Z80, 0x8000), &[0xDD, 0x00, 0x00]);
    assert!(source.contains(".byte"), "{source}");
}

fn data_ranges_and_address_comments() {
    let bytes = [0xA9, 0x01, 0xEA, 0xEA, 0x60];
    let mut opts = Options::new(Cpu::Mos6502, 0x1000);
    opts.data = vec![(0x1002, 0x1003)];
    opts.comments = true;
    let source = round_trip(&opts, &bytes);
    assert!(source.contains(".byte $EA,$EA"), "{source}");
    assert!(source.contains("; 1000: A9 01"), "{source}");
}

fn bad_input_is_an_error() {
    let mut opts = Options::new(Cpu::Mos6502, 0xFFF0);
    assert!(disassemble(&[0; 32], &opts).unwrap_err().contains("$FFFF"));
    opts = Options::new(Cpu::Mos6502, 0xC000);
    opts.entries = vec![0x1000];
    assert!(disassemble(&[0; 4], &opts).unwrap_err().contains("outside"));
}
/// Merged (every check kept): `symbol_files_name_addresses_inside_and_outside_the_image`, `data_ranges_and_address_comments`, `bad_input_is_an_error`.
#[test]
fn symbols_ranges_and_bad_input() {
    symbol_files_name_addresses_inside_and_outside_the_image();
    data_ranges_and_address_comments();
    bad_input_is_an_error();
}

// ---- real ROMs, when the machine repositories are checked out ----

fn rom(relative: &str) -> Option<Vec<u8>> {
    let base = std::env::var_os("RC_ROMS")?;
    let path = std::path::Path::new(&base).join(relative);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(e) => {
            eprintln!("skipping: {}: {e}", path.display());
            None
        }
    }
}

#[test]
fn real_roms_round_trip_byte_for_byte() {
    let roms: [(&str, Cpu, u16); 7] = [
        ("sinclair/roms/zx-spectrum/spec48.rom", Cpu::Z80, 0x0000),
        ("sinclair/roms/zx81/zx81.rom", Cpu::Z80, 0x0000),
        ("amstrad/roms/cpc/cpc464.rom", Cpu::Z80, 0x0000),
        (
            "commodore/roms/c64/kernal-901227-03.bin",
            Cpu::Mos6502,
            0xE000,
        ),
        (
            "commodore/roms/c64/basic-901226-01.bin",
            Cpu::Mos6502,
            0xA000,
        ),
        (
            "commodore/roms/c64/kernal-251104-04.bin",
            Cpu::Mos6502,
            0xE000,
        ),
        (
            "commodore/roms/1541/dos1541-325302-01+901229-05.bin",
            Cpu::Mos6502,
            0xC000,
        ),
    ];
    let mut tested = 0;
    for (name, cpu, org) in roms {
        let Some(bytes) = rom(name) else { continue };
        tested += 1;
        let mut linear = Options::new(cpu, org);
        round_trip(&linear, &bytes);
        // Following the reset vector region as well.
        linear.entries = vec![org];
        round_trip(&linear, &bytes);
    }
    if std::env::var_os("RC_ROMS").is_some() {
        assert!(tested > 0, "RC_ROMS is set but no ROM was found");
    }
}
