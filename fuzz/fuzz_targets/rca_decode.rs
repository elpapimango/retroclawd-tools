#![no_main]
use libfuzzer_sys::fuzz_target;
use rc_common::obj::Archive;

fuzz_target!(|data: &[u8]| {
    if let Ok(a) = Archive::decode(data) {
        let bytes = a.encode().expect("a decoded archive encodes");
        assert_eq!(Archive::decode(&bytes).as_ref(), Ok(&a));
    }
});
