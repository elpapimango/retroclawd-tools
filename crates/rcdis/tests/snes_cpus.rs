//! The 65816 and SPC700 disassembler output assembles back to the same bytes.

use rcasm::{Cpu, DiskFiles, Options as AsmOptions, assemble};
use rcdis::w65816::{W65816Options, disassemble as dis65816};
use rcdis::{Options, disassemble};

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

fn reassemble(source: &str) -> (u32, Vec<u8>) {
    let asm = assemble("out.asm", source, &AsmOptions::default(), &DiskFiles)
        .unwrap_or_else(|e| panic!("the output does not assemble: {e:?}\n{source}"));
    rcasm::flat_image(&asm, 0x00).unwrap()
}

#[test]
fn the_65816_round_trips_random_bytes_in_every_starting_width() {
    for (m16, x16) in [(false, false), (true, false), (false, true), (true, true)] {
        for seed in 1..=6 {
            let bytes = noise(seed, 6_000);
            let o = W65816Options {
                org: 0x80_8000,
                m16,
                x16,
                comments: false,
            };
            let dis = dis65816(&bytes, &o).unwrap();
            let (start, again) = reassemble(&dis.source);
            assert_eq!(start, 0x80_8000);
            assert_eq!(
                again, bytes,
                "m16={m16} x16={x16} seed {seed}\n{}",
                dis.source
            );
        }
    }
}

#[test]
fn the_65816_follows_rep_and_sep_and_labels_targets() {
    let program = [
        0x78, // sei
        0x18, 0xFB, // clc, xce
        0xC2, 0x30, // rep #$30
        0xA9, 0x34, 0x12, // lda #$1234
        0xA2, 0x78, 0x56, // ldx #$5678
        0xE2, 0x20, // sep #$20
        0xA9, 0x12, // lda #$12
        0x20, 0x14, 0x80, // jsr $8014
        0xD0,
        0xFB, // bne $8010 (back to the sep... target 0x8013? checked by label text below)
        0x6B, // rtl (at $8016? the sweep decides)
    ];
    let o = W65816Options {
        org: 0x00_8000,
        m16: false,
        x16: false,
        comments: false,
    };
    let dis = dis65816(&program, &o).unwrap();
    assert!(dis.source.contains("LDA #$1234"), "{}", dis.source);
    assert!(dis.source.contains("LDX #$5678"), "{}", dis.source);
    assert!(dis.source.contains("LDA #$12\n"), "{}", dis.source);
    assert!(dis.source.contains("JSR L_"), "{}", dis.source);
    let (_, again) = reassemble(&dis.source);
    assert_eq!(again, program);
}

#[test]
fn spc700_round_trips_random_bytes_and_flow() {
    for seed in 1..=8 {
        let bytes = noise(seed, 6_000);
        let dis = disassemble(&bytes, &Options::new(Cpu::Spc700, 0x0200)).unwrap();
        let (start, again) = reassemble(&dis.source);
        assert_eq!(start, 0x0200);
        assert_eq!(again, bytes, "seed {seed}\n{}", dis.source);
    }
    let program = [
        0x3F, 0x07, 0x02, // call !$0207
        0x2F, 0x03, // bra $0207... (to the ret after data)
        0x11, 0x22, // data
        0x6F, // ret
    ];
    let mut opts = Options::new(Cpu::Spc700, 0x0200);
    opts.entries = vec![0x0200];
    let dis = disassemble(&program, &opts).unwrap();
    let (_, again) = reassemble(&dis.source);
    assert_eq!(again, program, "{}", dis.source);
    assert!(dis.source.contains("CALL"), "{}", dis.source);
}

#[test]
fn the_65816_refuses_the_16_bit_path() {
    let e = disassemble(&[0xEA], &Options::new(Cpu::W65816, 0x8000)).unwrap_err();
    assert!(e.contains("65816"), "{e}");
}
