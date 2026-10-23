//! Arbitrary bytes disassembled as 68000 code, and the source assembled again: it must give the bytes back (bar the
//! pad byte after an odd last byte).

#![no_main]

use libfuzzer_sys::fuzz_target;
use rcasm::m68k::{M68kOptions, assemble_m68k};
use rcdis::m68k::{M68kDisOptions, disassemble_m68k};

struct NoFiles;

impl rcasm::Files for NoFiles {
    fn read(&self, path: &std::path::Path) -> Result<Vec<u8>, String> {
        Err(format!("no file {}", path.display()))
    }
}

fuzz_target!(|data: &[u8]| {
    let data = &data[..data.len().min(4096)];
    let d = disassemble_m68k(
        data,
        &M68kDisOptions {
            org: 0x1000,
            comments: false,
        },
    )
    .expect("an even origin in range");
    let again = assemble_m68k("fuzz.s", &d.source, &M68kOptions::default(), &NoFiles)
        .expect("the source assembles")
        .image();
    assert!(again.len() >= data.len() && again[..data.len()] == *data);
});
