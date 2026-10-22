//! Arbitrary text as 68000 source, flat and relocatable: errors, never a panic.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rcasm::m68k::{M68kOptions, assemble_m68k};

struct NoFiles;

impl rcasm::Files for NoFiles {
    fn read(&self, path: &std::path::Path) -> Result<Vec<u8>, String> {
        Err(format!("no file {}", path.display()))
    }
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    for relocatable in [false, true] {
        let opts = M68kOptions {
            relocatable,
            ..M68kOptions::default()
        };
        let _ = assemble_m68k("fuzz.s", &text, &opts, &NoFiles);
    }
});
