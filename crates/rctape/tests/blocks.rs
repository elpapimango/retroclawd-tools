//! The block model: files come back byte for byte, conversions keep what they can, the helpers agree with the
//! emulator's own tape player.

use rctape::spectrum::{self, Kind, checksum_ok, make_header, parse_header};
use rctape::{Block, CleanupOptions, Tape};

fn framed(flag: u8, data: &[u8]) -> Vec<u8> {
    let mut v = vec![flag];
    v.extend_from_slice(data);
    v.push(spectrum::checksum_of(&v));
    v
}

fn program(name: &str, data: &[u8]) -> Vec<Block> {
    vec![
        Block::standard(
            &make_header(Kind::Code, name, data.len() as u16, 32768, 32768),
            1000,
        ),
        Block::standard(&framed(0xFF, data), 1000),
    ]
}

/// One block of every kind the format defines, with plausible bodies.
fn every_block() -> Vec<Block> {
    vec![
        Block::standard(&framed(0, &[1, 2, 3]), 500),
        Block {
            id: 0x11,
            body: [
                &[
                    0x78, 0x08, 0x9B, 0x02, 0xDF, 0x02, 0x57, 0x03, 0xAE, 0x06, 0x00, 0x10, 8,
                    0xE8, 0x03, 3, 0, 0,
                ][..],
                &[0xAA, 0xBB, 0xCC],
            ]
            .concat(),
        },
        Block {
            id: 0x12,
            body: vec![0x78, 0x08, 0x10, 0x00],
        },
        Block {
            id: 0x13,
            body: vec![3, 1, 0, 2, 0, 3, 0],
        },
        Block {
            id: 0x14,
            body: [
                &[0x57, 0x03, 0xAE, 0x06, 8, 0x64, 0x00, 2, 0, 0][..],
                &[0x12, 0x34],
            ]
            .concat(),
        },
        Block {
            id: 0x15,
            body: [&[0xE8, 0x03, 79, 0, 8, 2, 0, 0][..], &[0xAB, 0xCD]].concat(),
        },
        Block {
            id: 0x18,
            body: [&[3, 0, 0, 0][..], &[1, 2, 3]].concat(),
        },
        Block {
            id: 0x19,
            body: [&[2, 0, 0, 0][..], &[9, 9]].concat(),
        },
        Block::pause(250),
        Block {
            id: 0x21,
            body: vec![3, b'a', b'b', b'c'],
        },
        Block {
            id: 0x22,
            body: vec![],
        },
        Block {
            id: 0x23,
            body: vec![1, 0],
        },
        Block {
            id: 0x24,
            body: vec![2, 0],
        },
        Block {
            id: 0x25,
            body: vec![],
        },
        Block {
            id: 0x26,
            body: vec![2, 0, 1, 0, 2, 0],
        },
        Block {
            id: 0x27,
            body: vec![],
        },
        Block {
            id: 0x28,
            body: vec![5, 0, 1, 0, 0, 1, b'x'],
        },
        Block {
            id: 0x2A,
            body: vec![0, 0, 0, 0],
        },
        Block {
            id: 0x2B,
            body: vec![1, 0, 0, 0, 1],
        },
        Block::text("a text"),
        Block {
            id: 0x31,
            body: vec![5, 3, b'h', b'i', b'!'],
        },
        Block {
            id: 0x32,
            body: vec![2, 0, 0, 0],
        },
        Block {
            id: 0x33,
            body: vec![1, 0, 1, 1],
        },
        Block {
            id: 0x34,
            body: vec![0; 8],
        },
        Block {
            id: 0x35,
            body: [&b"Custom info stri"[..], &[2, 0, 0, 0][..], &[7, 7]].concat(),
        },
        Block {
            id: 0x40,
            body: vec![1, 2, 0, 0, 5, 6],
        },
        Block {
            id: 0x5A,
            body: b"XTapeX!\x1a\x00".to_vec(),
        },
    ]
}

#[test]
fn every_block_kind_comes_back_byte_for_byte() {
    let tape = Tape {
        major: 1,
        minor: 20,
        blocks: every_block(),
    };
    let bytes = tape.to_bytes();
    let back = Tape::parse(&bytes).expect("parses");
    assert_eq!(back, tape);
    assert_eq!(back.to_bytes(), bytes);
}

#[test]
fn a_tape_with_an_unknown_block_says_where() {
    let mut bytes = Tape::new().to_bytes();
    bytes.extend_from_slice(&[0x10, 0, 0, 1, 0, 0xAA]);
    bytes.push(0x77);
    let e = Tape::parse(&bytes).unwrap_err().to_string();
    assert!(e.contains("block 2") && e.contains("0x77"), "{e}");
}

#[test]
fn a_cut_off_tape_says_which_block() {
    let mut bytes = Tape::new().to_bytes();
    bytes.extend_from_slice(&[0x10, 0, 0, 5, 0, 1, 2]);
    let e = Tape::parse(&bytes).unwrap_err().to_string();
    assert!(e.contains("block 1") && e.contains("past the end"), "{e}");
    assert!(Tape::parse(b"nope nope nope").is_err());
}

#[test]
fn tap_to_tzx_and_back_is_the_identity() {
    let mut tap = Vec::new();
    for b in program("hello", &[1, 2, 3, 4])
        .iter()
        .chain(program("bye", &[9]).iter())
    {
        let p = b.payload().unwrap();
        tap.extend_from_slice(&(p.len() as u16).to_le_bytes());
        tap.extend_from_slice(p);
    }
    let tape = rctape::from_tap(&tap).unwrap();
    assert_eq!(tape.blocks.len(), 4);
    assert!(
        tape.blocks
            .iter()
            .all(|b| b.id == 0x10 && b.pause_ms() == Some(1000))
    );
    let tzx = Tape::parse(&tape.to_bytes()).unwrap();
    let (back, notes) = rctape::to_tap(&tzx, false).unwrap();
    assert_eq!(back, tap);
    assert!(notes.is_empty());
    assert!(
        rctape::from_tap(&tap[..tap.len() - 1])
            .unwrap_err()
            .contains("block 4")
    );
}

#[test]
fn only_standard_blocks_fit_a_tap() {
    let mut tape = Tape::new();
    tape.blocks.push(Block::standard(&framed(0xFF, &[1]), 1000));
    tape.blocks.push(Block::pause(500));
    tape.blocks.push(Block::text("hi"));
    let e = rctape::to_tap(&tape, false).unwrap_err();
    assert!(e.contains("--lossy"), "{e}");
    let (bytes, notes) = rctape::to_tap(&tape, true).unwrap();
    assert_eq!(bytes.len(), 2 + 3);
    assert!(notes[0].contains("dropped 2"));
    tape.blocks.push(Block {
        id: 0x12,
        body: vec![1, 0, 1, 0],
    });
    assert!(rctape::to_tap(&tape, true).unwrap_err().contains("block 4"));
}

#[test]
fn headers_decode_and_describe_themselves() {
    let h = make_header(Kind::Program, "LOADER", 1234, 10, 1200);
    assert!(checksum_ok(&h));
    let p = parse_header(&h).unwrap();
    assert_eq!(
        (p.kind, p.name.as_str(), p.length, p.param1, p.param2),
        (Kind::Program, "LOADER", 1234, 10, 1200)
    );
    assert_eq!(p.describe(), "Program: LOADER LINE 10 (1234 bytes)");
    assert_eq!(p.file_name(), "LOADER.bas");
    let c = parse_header(&make_header(Kind::Code, "screen", 6912, 16384, 32768)).unwrap();
    assert_eq!(c.describe(), "Bytes: screen start 16384, length 6912");
    assert!(parse_header(&framed(0xFF, &[0; 17])).is_none());
    let mut bad = h.clone();
    bad[5] ^= 1;
    assert!(!checksum_ok(&bad));
}

#[test]
fn basic_is_listed_with_its_tokens() {
    // 10 PRINT "HI": 3.5 ; 20 LET a=5 (with the hidden number form)
    let mut prog = vec![0, 10, 6, 0, 0xF5, b'"', b'H', b'I', b'"', 0x0D];
    let line2: Vec<u8> = [
        &[0xF1, b'a', b'='][..],
        &[b'5', 0x0E, 0, 0, 5, 0, 0],
        &[0x0D],
    ]
    .concat();
    prog.extend_from_slice(&[0, 20, line2.len() as u8, 0]);
    prog.extend_from_slice(&line2);
    assert_eq!(spectrum::list_basic(&prog), "10 PRINT \"HI\"\n20 LET a=5\n");
}

#[test]
fn block_ranges() {
    assert_eq!(rctape::parse_selection("2-3,5", 6).unwrap(), vec![1, 2, 4]);
    assert_eq!(rctape::parse_selection("4-", 6).unwrap(), vec![3, 4, 5]);
    assert_eq!(rctape::parse_selection("-2", 6).unwrap(), vec![0, 1]);
    assert_eq!(rctape::parse_selection("all", 3).unwrap(), vec![0, 1, 2]);
    assert_eq!(rctape::parse_selection("3,3,1", 3).unwrap(), vec![0, 2]);
    assert!(rctape::parse_selection("0", 3).is_err());
    assert!(
        rctape::parse_selection("4", 3)
            .unwrap_err()
            .contains("only 3")
    );
    assert!(rctape::parse_selection("3-2", 5).is_err());
    assert!(rctape::parse_selection("x", 5).is_err());
}

#[test]
fn split_and_merge_are_inverse() {
    let mut tape = Tape::new();
    tape.blocks.push(Block::text("a collection"));
    tape.blocks.extend(program("first", &[1, 2]));
    tape.blocks.push(Block::pause(200));
    tape.blocks.extend(program("second", &[3]));
    tape.blocks.extend(program("first", &[4]));
    tape.blocks
        .push(Block::standard(&framed(0xFF, &[7, 7]), 1000)); // a headerless block
    let parts = rctape::split(&tape);
    let names: Vec<&str> = parts.iter().map(|p| p.0.as_str()).collect();
    assert_eq!(names, ["first", "second", "first-2", "block01"]);
    // The text went with the first program, the pause stayed with it as well.
    assert_eq!(parts[0].1.blocks.len(), 4);
    let tapes: Vec<Tape> = parts.into_iter().map(|p| p.1).collect();
    assert_eq!(rctape::merge(&tapes), tape);
}

#[test]
fn cleanup_does_what_it_is_asked() {
    let mut bad = framed(0xFF, &[1, 2, 3]);
    *bad.last_mut().unwrap() ^= 0xFF;
    let mut tape = Tape::new();
    tape.blocks.push(Block::text("remove me"));
    tape.blocks.push(Block::standard(&bad, 1000));
    tape.blocks.push(Block::pause(100));
    tape.blocks.push(Block::pause(200));
    tape.blocks.push(Block::pause(0));
    tape.blocks.push(Block::standard(&[], 50));
    let opts = CleanupOptions {
        strip_info: true,
        fix_checksums: true,
        pause_ms: Some(2000),
        merge_pauses: true,
        drop_empty: true,
    };
    let (clean, notes) = rctape::cleanup(&tape, &opts);
    assert_eq!(clean.blocks.len(), 3, "{notes:?}");
    assert!(checksum_ok(clean.blocks[0].payload().unwrap()));
    assert_eq!(clean.blocks[0].pause_ms(), Some(2000));
    assert_eq!(
        clean.blocks[1].pause_ms(),
        Some(2000),
        "the joined pause is then set like any other"
    );
    assert_eq!(
        clean.blocks[2].pause_ms(),
        Some(0),
        "a stop-the-tape block stays one"
    );
    let (_, again) = rctape::cleanup(&clean, &opts);
    assert!(again.is_empty(), "{again:?}");
}

#[test]
fn extract_names_files_after_their_headers() {
    let mut tape = Tape::new();
    tape.blocks.extend(program("screen", &[1, 2, 3]));
    tape.blocks.extend(program("screen", &[4]));
    tape.blocks.push(Block::standard(&framed(0xFF, &[9, 9]), 0));
    let all: Vec<usize> = (0..tape.blocks.len()).collect();
    let files = rctape::extract(&tape, &all, false);
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["screen.bin", "screen-2.bin", "005-data.bin"]);
    assert_eq!(files[0].bytes, [1, 2, 3]);
    let only_second = rctape::extract(&tape, &[3], false);
    assert_eq!(only_second.len(), 1);
    assert_eq!(only_second[0].bytes, [4]);
    let raw = rctape::extract(&tape, &[0, 1], true);
    assert_eq!(
        raw.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        ["001-header.bin", "002-data.bin"]
    );
    assert_eq!(raw[1].bytes, framed(0xFF, &[1, 2, 3]));
}

#[test]
fn playing_times_agree_with_the_emulators_tape_player() {
    // Pauses excluded: zx-tape counts a millisecond as 3546.9 T-states, the TZX document as 3500.
    let mut tape = Tape::new();
    tape.blocks
        .push(Block::standard(&make_header(Kind::Code, "t", 5, 0, 0), 0));
    tape.blocks
        .push(Block::standard(&framed(0xFF, &[1, 2, 3, 4, 5]), 0));
    for id in [0x11, 0x12, 0x13, 0x14] {
        tape.blocks
            .push(every_block().into_iter().find(|b| b.id == id).unwrap());
    }
    for b in &mut tape.blocks {
        if b.pause_ms().is_some() {
            b.set_pause_ms(0);
        }
    }
    let ours: u64 = tape.blocks.iter().map(|b| b.cycles().unwrap()).sum();
    let theirs = zx_tape::parse_tzx(&tape.to_bytes())
        .expect("zx-tape parses it")
        .total_cycles();
    let diff = ours.abs_diff(theirs);
    // zx-tape adds a closing edge after a block whose pause is 0; that is a few thousand T-states at most.
    assert!(diff <= 5000, "ours {ours}, zx-tape {theirs}");
}
