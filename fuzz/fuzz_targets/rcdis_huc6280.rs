//! Arbitrary bytes disassembled as HuC6280 code, and the source assembled again: it must give the bytes back.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rcasm::{Cpu, DiskFiles, Options as AsmOptions, assemble};
use rcdis::{Options, disassemble};

fuzz_target!(|data: &[u8]| {
    let data = &data[..data.len().min(4096)];
    let d = disassemble(data, &Options::new(Cpu::Huc6280, 0xE000)).expect("disassembles");
    let asm = assemble("fuzz.asm", &d.source, &AsmOptions::default(), &DiskFiles)
        .expect("the source assembles");
    let (_, again) = rcasm::flat_image(&asm, 0).expect("an image");
    assert_eq!(again, data);
});
