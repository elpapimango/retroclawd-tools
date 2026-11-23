//! Arbitrary text assembled as HuC6280 source: errors are fine, a panic or a hang is not.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rcasm::{Cpu, DiskFiles, Options, assemble};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(&data[..data.len().min(4096)]);
    let opts = Options {
        cpu: Some(Cpu::Huc6280),
        ..Options::default()
    };
    let _ = assemble("fuzz.asm", &text, &opts, &DiskFiles);
});
