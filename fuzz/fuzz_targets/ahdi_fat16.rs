//! A hard disk image from arbitrary bytes: the partitions, the root directory and every file are read; none of it may
//! panic or loop.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(disk) = rcdisk::hd::HardDisk::parse(data.to_vec()) else {
        return;
    };
    let Ok(parts) = disk.partitions() else { return };
    for i in 0..parts.len() {
        if let Ok(entries) = disk.list(i) {
            for e in entries.iter().take(64) {
                let _ = disk.read(i, &e.name);
            }
        }
    }
});
