//! The HuC6280 disassembler output assembles back to the same bytes, in linear and in flow mode.

use rcasm::{DiskFiles, Options as AsmOptions, assemble};
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

fn round_trip(opts: &Options, bytes: &[u8]) -> String {
    let dis = disassemble(bytes, opts).expect("disassembles");
    let asm = assemble("out.asm", &dis.source, &AsmOptions::default(), &DiskFiles)
        .unwrap_or_else(|e| panic!("the output does not assemble: {e:?}\n{}", dis.source));
    let (start, again) = rcasm::flat_image(&asm, 0x00).unwrap();
    assert_eq!(start, opts.org);
    assert_eq!(again, bytes, "\n{}", dis.source);
    dis.source
}

#[test]
fn random_bytes_round_trip() {
    for seed in 1..=8 {
        round_trip(
            &Options::new(rcasm::Cpu::Huc6280, 0xE000),
            &noise(seed, 8_000),
        );
    }
}

#[test]
fn flow_mode_follows_branches_calls_and_bit_branches() {
    let program = [
        0x44, 0x08, // E000 bsr $E00A
        0x0F, 0x20, 0x02, // E002 bbr0 $20,$E007
        0xEA, // E005 nop
        0xEA, // E006 nop
        0x60, // E007 rts
        0x48, 0x49, // E008 data
        0x73, 0x00, 0x40, 0x00, 0x50, 0x10, 0x00, // E00A tii $4000,$5000,$0010
        0x60, // E011 rts
    ];
    let mut opts = Options::new(rcasm::Cpu::Huc6280, 0xE000);
    opts.entries = vec![0xE000];
    let source = round_trip(&opts, &program);
    assert!(source.contains("BSR L_E00A"), "{source}");
    assert!(source.contains("L_E00A:"), "{source}");
    assert!(source.contains("TII"), "{source}");
    assert!(source.contains(".byte $48,$49"), "{source}");
}
