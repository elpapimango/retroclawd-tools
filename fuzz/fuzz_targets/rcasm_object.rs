#![no_main]
use libfuzzer_sys::fuzz_target;
use rcasm::{Cpu, DiskFiles, Options, assemble_object};

fuzz_target!(|data: &[u8]| {
    let Some((&which, rest)) = data.split_first() else {
        return;
    };
    let Ok(source) = std::str::from_utf8(rest) else {
        return;
    };
    let cpu = [Cpu::Mos6502, Cpu::Z80, Cpu::Huc6280][which as usize % 3];
    let opts = Options {
        cpu: Some(cpu),
        ..Options::default()
    };
    if let Ok(o) = assemble_object("fuzz.asm", source, &opts, &DiskFiles) {
        // an object the assembler writes is one the reader accepts
        assert!(rc_common::obj::Object::decode(&o.encode()).is_ok());
    }
});
