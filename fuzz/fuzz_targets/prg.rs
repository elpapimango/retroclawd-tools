//! A GEMDOS executable from arbitrary bytes: refused or parsed, never a panic; what parses writes back and parses again.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(p) = rc_common::prg::parse(data) {
        let again = rc_common::prg::parse(&p.to_bytes()).expect("what we wrote parses");
        assert_eq!(again, p);
        let _ = p.relocated(0x1_0000);
    }
});
