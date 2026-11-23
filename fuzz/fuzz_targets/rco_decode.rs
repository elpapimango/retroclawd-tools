#![no_main]
use libfuzzer_sys::fuzz_target;
use rc_common::obj::Object;

fuzz_target!(|data: &[u8]| {
    if let Ok(o) = Object::decode(data) {
        // what decodes must encode back to something that decodes to the same object
        assert_eq!(Object::decode(&o.encode()).as_ref(), Ok(&o));
    }
});
