//! The GEMDOS executable format (Atari Compendium chapter 2): header, segments, fixup table.

use rc_common::prg::{self, Prg, PrgError};

fn sample() -> Prg {
    Prg {
        text: (0..400u32).map(|i| i as u8).collect(),
        data: vec![1, 2, 3, 4, 5, 6, 7, 8],
        bss: 1024,
        symbols: Vec::new(),
        flags: 0,
        fixups: vec![2, 6, 300, 404],
    }
}

#[test]
fn round_trip() {
    let p = sample();
    let bytes = p.to_bytes();
    assert_eq!(&bytes[..2], &[0x60, 0x1A]);
    assert_eq!(prg::parse(&bytes).unwrap(), p);
    // And with symbols and flags.
    let mut q = p.clone();
    q.symbols = vec![0xAA; 28];
    q.flags = 0x17;
    assert_eq!(prg::parse(&q.to_bytes()).unwrap(), q);
}

#[test]
fn the_header_fields_are_where_the_document_puts_them() {
    let bytes = sample().to_bytes();
    assert_eq!(
        u32::from_be_bytes(bytes[2..6].try_into().unwrap()),
        400,
        "text size at $02"
    );
    assert_eq!(
        u32::from_be_bytes(bytes[6..10].try_into().unwrap()),
        8,
        "data size at $06"
    );
    assert_eq!(
        u32::from_be_bytes(bytes[10..14].try_into().unwrap()),
        1024,
        "bss size at $0A"
    );
    assert_eq!(
        u32::from_be_bytes(bytes[14..18].try_into().unwrap()),
        0,
        "symbol table size at $0E"
    );
    assert_eq!(&bytes[0x1A..0x1C], &[0, 0], "ABSFLAG 0: there are fixups");
    assert_eq!(bytes[0x1C], 0, "the text starts at $1C");
    assert_eq!(bytes[0x1D], 1);
}

/// "1: advance 254 bytes"; "2-254 (even): advance this many bytes and fixup the longword there".
#[test]
fn fixup_table_with_1_bytes_adds_254() {
    let mut p = Prg {
        text: vec![0; 1000],
        fixups: vec![4, 4 + 254 * 2 + 6],
        ..Prg::default()
    };
    let bytes = p.to_bytes();
    let table = &bytes[bytes.len() - 8..];
    assert_eq!(&table[..4], &[0, 0, 0, 4], "the first fixup offset");
    assert_eq!(
        &table[4..],
        &[1, 1, 6, 0],
        "two advances of 254, then a step of 6, then the end"
    );
    assert_eq!(prg::parse(&bytes).unwrap().fixups, vec![4, 518]);
    // A distance of exactly 254 is one step, not an advance.
    p.fixups = vec![2, 256];
    let bytes = p.to_bytes();
    assert_eq!(
        &bytes[bytes.len() - 2..],
        &[254, 0],
        "one 254 step then the end"
    );
    assert_eq!(prg::parse(&bytes).unwrap().fixups, vec![2, 256]);
}

#[test]
fn a_fixup_adds_the_load_address() {
    let p = Prg {
        text: vec![9, 9, 0, 0, 0, 0x10, 0xFF, 0xFF, 0xFF, 0xFE, 9, 9],
        fixups: vec![2, 6],
        ..Prg::default()
    };
    let loaded = p.relocated(0x0001_0000);
    assert_eq!(&loaded[..2], &[9, 9]);
    assert_eq!(&loaded[2..6], &[0, 1, 0, 0x10]);
    assert_eq!(
        &loaded[6..10],
        &[0, 0, 0xFF, 0xFE],
        "wraps like the 32-bit add"
    );
    assert_eq!(&loaded[10..], &[9, 9]);
}

#[test]
fn a_program_without_fixups_has_a_zero_long() {
    let p = Prg {
        text: vec![0x4E, 0x75],
        ..Prg::default()
    };
    let bytes = p.to_bytes();
    assert_eq!(&bytes[bytes.len() - 4..], &[0, 0, 0, 0]);
    assert_eq!(prg::parse(&bytes).unwrap(), p);
}

#[test]
fn a_fixup_beyond_the_text_is_an_error() {
    let mut bytes = Prg {
        text: vec![0; 16],
        fixups: vec![2],
        ..Prg::default()
    }
    .to_bytes();
    // The first fixup offset: 14 leaves no room for a longword in a 16-byte image.
    let at = bytes.len() - 5;
    bytes[at..at + 4].copy_from_slice(&14u32.to_be_bytes());
    assert_eq!(
        prg::parse(&bytes),
        Err(PrgError::FixupBeyondImage { offset: 14 })
    );
    // And a step that leaves the image.
    let mut bytes = Prg {
        text: vec![0; 16],
        fixups: vec![2, 8],
        ..Prg::default()
    }
    .to_bytes();
    let n = bytes.len();
    bytes[n - 2] = 20;
    assert_eq!(
        prg::parse(&bytes),
        Err(PrgError::FixupBeyondImage { offset: 22 })
    );
}

#[test]
fn a_run_of_advances_past_the_image_is_an_error_not_a_loop() {
    let mut bytes = Prg {
        text: vec![0; 16],
        fixups: vec![2],
        ..Prg::default()
    }
    .to_bytes();
    bytes.pop();
    bytes.extend(std::iter::repeat_n(1u8, 100_000));
    bytes.push(0);
    assert!(matches!(
        prg::parse(&bytes),
        Err(PrgError::FixupBeyondImage { .. })
    ));
}

#[test]
fn symbol_table_beyond_the_file_is_an_error() {
    let mut bytes = Prg {
        text: vec![0; 16],
        symbols: vec![0; 14],
        ..Prg::default()
    }
    .to_bytes();
    bytes[14..18].copy_from_slice(&0x0100_0000u32.to_be_bytes());
    assert_eq!(prg::parse(&bytes), Err(PrgError::SymbolTableBeyondFile));
}

#[test]
fn sizes_beyond_the_file_are_an_error_before_anything_is_allocated() {
    let mut bytes = vec![0u8; 40];
    bytes[..2].copy_from_slice(&[0x60, 0x1A]);
    bytes[2..6].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(matches!(
        prg::parse(&bytes),
        Err(PrgError::SizesBeyondFile { .. })
    ));
    bytes[2..6].copy_from_slice(&0u32.to_be_bytes());
    bytes[6..10].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(matches!(
        prg::parse(&bytes),
        Err(PrgError::SizesBeyondFile { .. })
    ));
}

#[test]
fn malformed_files_say_what_is_wrong() {
    assert_eq!(prg::parse(&[]), Err(PrgError::TooShort));
    assert_eq!(prg::parse(&[0x60; 20]), Err(PrgError::TooShort));
    let mut b = vec![0u8; 28];
    assert_eq!(prg::parse(&b), Err(PrgError::BadMagic(0)));
    b[0] = 0x60;
    b[1] = 0x1A;
    assert_eq!(prg::parse(&b), Err(PrgError::FixupTableMissing));
    b[0x1A] = 1;
    assert!(
        prg::parse(&b).is_ok(),
        "an absolute program may stop after its segments"
    );
    let mut odd = Prg {
        text: vec![0; 16],
        fixups: vec![2, 6],
        ..Prg::default()
    }
    .to_bytes();
    let n = odd.len();
    odd[n - 2] = 3;
    assert_eq!(prg::parse(&odd), Err(PrgError::BadFixupStep { byte: 3 }));
    let mut open = Prg {
        text: vec![0; 16],
        fixups: vec![2, 6],
        ..Prg::default()
    }
    .to_bytes();
    open.pop();
    assert_eq!(prg::parse(&open), Err(PrgError::FixupListUnterminated));
}

/// A deterministic spread of random and mutated inputs: nothing panics, and what parses writes back and parses
/// again to the same thing.
#[test]
fn parse_of_random_bytes_never_panics() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let good = sample().to_bytes();
    for round in 0..20_000 {
        let len = (next() % 600) as usize;
        let mut bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        if round % 2 == 0 {
            // Mutations of a good file reach deeper than noise.
            bytes = good.clone();
            for _ in 0..(1 + next() % 6) {
                let i = (next() % bytes.len() as u64) as usize;
                bytes[i] = next() as u8;
            }
            if round % 7 == 0 {
                bytes.truncate((next() % bytes.len() as u64) as usize);
            }
        } else if len >= 2 {
            bytes[0] = 0x60;
            bytes[1] = 0x1A;
        }
        if let Ok(p) = prg::parse(&bytes) {
            let again = prg::parse(&p.to_bytes());
            assert_eq!(again.as_ref(), Ok(&p), "round {round}");
        }
    }
}
