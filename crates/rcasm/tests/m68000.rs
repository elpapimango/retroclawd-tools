//! The 68000 back end: every instruction the CPU crate's disassembler accepts assembles back to itself, the addressing
//! modes encode, and the mistakes it is meant to catch are caught.

use mc68000::disassemble::disassemble;
use rcasm::Files;
use rcasm::m68k::{M68kAssembly, M68kOptions, assemble_m68k};
use std::path::Path;

struct NoFiles;

impl Files for NoFiles {
    fn read(&self, path: &Path) -> Result<Vec<u8>, String> {
        Err(format!("no file {}", path.display()))
    }
}

fn flat(source: &str) -> Result<M68kAssembly, String> {
    assemble_m68k("test.s", source, &M68kOptions::default(), &NoFiles).map_err(|d| {
        d.iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })
}

fn bytes(source: &str) -> Vec<u8> {
    flat(source).unwrap_or_else(|e| panic!("{e}")).image()
}

fn words(source: &str) -> Vec<u16> {
    bytes(source)
        .chunks(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect()
}

const ORIGIN: u32 = 0x1000;

/// Disassembles `opcode` followed by `tail` at `ORIGIN`.
fn dis(opcode: u16, tail: &[u16]) -> (String, usize, Vec<u8>) {
    let mut b = opcode.to_be_bytes().to_vec();
    for w in tail {
        b.extend(w.to_be_bytes());
    }
    let (text, len) = disassemble(&b, ORIGIN);
    b.truncate(len);
    (text, len, b)
}

/// Assembles one line at `ORIGIN`.
fn assemble_at_origin(text: &str) -> Result<Vec<u8>, String> {
    Ok(flat(&format!("    .org ${ORIGIN:x}\n    {text}\n"))?.image())
}

/// Every one of the 65 536 first words, with extension words after it that give an index register, a byte, a word
/// and a long that mean something: what the disassembler prints, the assembler turns back into the same bytes.
#[test]
fn every_opcode_word_assembles_back_to_itself() {
    // A brief index extension has bits 10-8 clear, a byte immediate has a zero upper byte.
    let tail = [0x0034, 0x0012, 0x0078];
    let mut checked = 0;
    let mut bad = Vec::new();
    for op in 0..=0xFFFFu16 {
        let (text, len, original) = dis(op, &tail);
        if text.starts_with("dc.") {
            continue;
        }
        checked += 1;
        match assemble_at_origin(&text) {
            Ok(b) if b == original => {}
            Ok(b) => bad.push(format!(
                "{op:04X} `{text}` -> {b:02X?}, wanted {original:02X?}"
            )),
            Err(e) => bad.push(format!("{op:04X} `{text}`: {e} ({len} bytes)")),
        }
    }
    assert!(checked > 40_000, "only {checked} words are instructions");
    assert!(
        bad.is_empty(),
        "{} mismatches, first ones:\n{}",
        bad.len(),
        bad.iter().take(25).cloned().collect::<Vec<_>>().join("\n")
    );
}

/// The same with extension words that are negative, set every flag and are not small: the text of what is assembled
/// is the text that was read.
#[test]
fn every_opcode_word_survives_large_extension_words() {
    let tail = [0xF9B4, 0x8123, 0xFEDC];
    let mut bad = Vec::new();
    for op in 0..=0xFFFFu16 {
        let (text, _, _) = dis(op, &tail);
        if text.starts_with("dc.") {
            continue;
        }
        match assemble_at_origin(&text) {
            Ok(b) => {
                let again = disassemble(&b, ORIGIN).0;
                if again != text {
                    bad.push(format!("{op:04X} `{text}` -> `{again}`"));
                }
            }
            Err(e) => bad.push(format!("{op:04X} `{text}`: {e}")),
        }
    }
    assert!(
        bad.is_empty(),
        "{} mismatches, first ones:\n{}",
        bad.len(),
        bad.iter().take(25).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn all_twelve_addressing_modes_encode() {
    let cases: [(&str, &[u16]); 12] = [
        ("move.w d1,d2", &[0x3401]),
        ("move.w a1,d2", &[0x3409]),
        ("move.w (a1),d2", &[0x3411]),
        ("move.w (a1)+,d2", &[0x3419]),
        ("move.w -(a1),d2", &[0x3421]),
        ("move.w $10(a1),d2", &[0x3429, 0x0010]),
        ("move.w $10(a1,d3.w),d2", &[0x3431, 0x3010]),
        ("move.w $1234.w,d2", &[0x3438, 0x1234]),
        ("move.w $12345678,d2", &[0x3439, 0x1234, 0x5678]),
        ("move.w *+$10(pc),d2", &[0x343A, 0x000E]),
        ("move.w *+$10(pc,a3.l),d2", &[0x343B, 0xB80E]),
        ("move.w #$1234,d2", &[0x343C, 0x1234]),
    ];
    for (text, want) in cases {
        assert_eq!(words(&format!("    {text}\n")), want, "{text}");
    }
}

#[test]
fn a_program_with_labels_and_data_assembles() {
    let src = "
    .org $8000
start:
    lea     msg(pc),a0
    move.l  #msg,d0
    bra.s   start
    .even
msg:
    dc.b    \"Hi\",13,10,0
    .even
table:
    dc.w    1,2,3
    dc.l    start,table
";
    let a = flat(src).unwrap();
    assert_eq!(a.origin, 0x8000);
    let image = a.image();
    // lea msg(pc),a0: the displacement counts from the extension word at $8002: msg is at $800C.
    assert_eq!(&image[..4], &[0x41, 0xFA, 0x00, 0x0A], "{image:02X?}");
    assert_eq!(&image[4..10], &[0x20, 0x3C, 0x00, 0x00, 0x80, 0x0C]);
    assert_eq!(&image[10..12], &[0x60, 0xF4], "bra.s start: back 12");
    assert_eq!(&image[12..14], b"Hi");
    assert_eq!(&image[0x12..0x14], &[0x00, 0x01]);
    assert_eq!(&image[0x18..0x20], &[0, 0, 0x80, 0, 0, 0, 0x80, 0x12]);
    let names: Vec<_> = a
        .symbols
        .iter()
        .map(|s| (s.name.as_str(), s.value))
        .collect();
    assert_eq!(
        names,
        vec![("start", 0x8000), ("msg", 0x800C), ("table", 0x8012)]
    );
}

#[test]
fn ambiguous_branch_is_an_error() {
    let e = flat("here:\n    bra here\n").unwrap_err();
    assert!(
        e.contains("size of this branch") && e.contains(".opt autobranch"),
        "{e}"
    );
    assert!(flat("here:\n    bra.s here\n").is_ok());
    assert!(flat("here:\n    bne.w here\n").is_ok());
}

#[test]
fn autobranch_takes_the_short_form_only_for_a_near_target_behind() {
    // Backward and near: short. Forward: word (the size is settled before the target is known).
    let w =
        words("    .opt autobranch\nback:\n    nop\n    bra back\n    bra fwd\nfwd:\n    nop\n");
    assert_eq!(w, vec![0x4E71, 0x60FC, 0x6000, 0x0002, 0x4E71]);
}

#[test]
fn a_short_branch_that_is_too_far_is_an_error() {
    let src = format!("    bra.s far\n    ds.b {}\nfar:\n    nop\n", 200);
    let e = flat(&src).unwrap_err();
    assert!(e.contains("not a short branch"), "{e}");
}

#[test]
fn org_beyond_24_bits_is_an_error() {
    let e = flat("    .org $1000000\n    nop\n").unwrap_err();
    assert!(e.contains("24-bit"), "{e}");
    assert!(flat("    .org $FFFFFE\n    nop\n").is_ok());
}

#[test]
fn byte_sized_address_register_operand_is_an_error() {
    for line in [
        "move.b d0,a1",
        "movea.b d0,a1",
        "add.b d0,a1",
        "sub.b #1,a1",
        "cmpa.b d0,a1",
        "addq.b #1,a1",
    ] {
        assert!(
            flat(&format!("    {line}\n")).is_err(),
            "{line} was accepted"
        );
    }
    assert!(
        flat("    move.w a0,d1\n").is_ok(),
        "an address register as a word source is fine"
    );
}

#[test]
fn operand_combinations_the_68000_does_not_have_are_refused() {
    for line in [
        "move.w d0,#1",
        "lea d0,a1",
        "move.w (a0)+,(a1)+,d0",
        "jmp d0",
        "clr.w a0",
        "movem.l d0,d1",
        "btst d0,a0",
        "add.w d0,d1,d2",
        "nop d0",
        "moveq #300,d0",
        "addq.w #9,d0",
        "asl.w #9,d0",
        "trap #16",
        "move.w d0,$12345.w",
        "move.w $10000(a0),d0",
        "bogus d0",
    ] {
        assert!(
            flat(&format!("    {line}\n")).is_err(),
            "`{line}` was accepted"
        );
    }
}

#[test]
fn immediate_forms_are_chosen_for_the_generic_spellings() {
    // To a data register the plain form takes the immediate as its source; to anything else it is the `i` form.
    assert_eq!(words("    add.w #5,d0\n"), vec![0xD07C, 0x0005]);
    assert_eq!(words("    cmp.l #5,d1\n"), vec![0xB2BC, 0x0000, 0x0005]);
    assert_eq!(words("    and.b #$f,d2\n"), vec![0xC43C, 0x000F]);
    assert_eq!(words("    add.w #5,(a0)\n"), vec![0x0650, 0x0005], "addi");
    assert_eq!(
        words("    cmp.l #5,(a1)\n"),
        vec![0x0C91, 0x0000, 0x0005],
        "cmpi"
    );
    assert_eq!(
        words("    and.b #$f,2(a2)\n"),
        [0x022A, 0x000F, 0x0002],
        "andi"
    );
    assert_eq!(words("    sub.w d1,a2\n"), vec![0x94C1], "suba");
    assert_eq!(words("    addq.w #3,d0\n"), vec![0x5640], "addq");
    assert_eq!(words("    moveq #-1,d3\n"), vec![0x76FF]);
    assert_eq!(words("    move.l sp,a0\n"), vec![0x204F], "sp is a7");
    assert_eq!(words("    movem.l d0-d2/a0,-(sp)\n"), vec![0x48E7, 0xE080]);
    assert_eq!(words("    movem.l (sp)+,d0-d2/a0\n"), vec![0x4CDF, 0x0107]);
    assert_eq!(words("    dbra d0,*\n"), vec![0x51C8, 0xFFFE]);
    assert_eq!(
        words("    andi #$fe,ccr\n    ori #$700,sr\n    move usp,a1\n"),
        vec![0x023C, 0x00FE, 0x007C, 0x0700, 0x4E69]
    );
}

#[test]
fn equ_labels_sections_and_forward_references() {
    let a = flat(
        "
SIZE equ 4
BIG = SIZE*SIZE
    .text
    move.w #BIG,d0
    move.l #later,d1
    .data
buf: ds.b SIZE
later: dc.l buf
    .bss
bssbuf: ds.b 10
",
    )
    .unwrap();
    assert_eq!(
        a.text,
        vec![0x30, 0x3C, 0x00, 0x10, 0x22, 0x3C, 0x00, 0x00, 0x00, 0x0E]
    );
    // `later` is at text (10 bytes) + 4.
    assert_eq!(a.data, vec![0, 0, 0, 0, 0, 0, 0, 0x0A]);
    assert_eq!(a.bss, 10);
    let value = |n: &str| {
        a.symbols
            .iter()
            .find(|s| s.name == n)
            .map(|s| (s.value, s.label))
    };
    assert_eq!(value("BIG"), Some((16, false)));
    assert_eq!(value("later"), Some((14, true)));
    assert_eq!(value("bssbuf"), Some((18, true)));
}

#[test]
fn a_relocatable_program_marks_the_longs_that_hold_addresses() {
    let opts = M68kOptions {
        relocatable: true,
        ..M68kOptions::default()
    };
    let a = assemble_m68k(
        "t.s",
        "
    .text
    lea     msg,a0
    move.l  #msg,d0
    move.l  #msg-start,d1
    pea     (a0)
    move.w  #msg-start,d2
start:
    nop
    .data
msg: dc.b 1,2
ptr: dc.l msg
    dc.l 0x1000
",
        &opts,
        &NoFiles,
    )
    .unwrap_or_else(|e| {
        panic!(
            "{}",
            e.iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    // lea abs.l: 2 + 2 = long at 2; move.l #imm,d0: long at 8; the difference `msg-start` is not an address; `ptr` in
    // the data holds an address.
    let text_len = a.text.len() as u32;
    assert_eq!(a.relocs, vec![2, 8, text_len + 2]);
    // The long in `ptr` holds msg's section-relative value.
    assert_eq!(&a.data[2..6], &(text_len).to_be_bytes());
}

#[test]
fn a_relocatable_expression_that_is_neither_address_nor_constant_is_an_error() {
    let opts = M68kOptions {
        relocatable: true,
        ..M68kOptions::default()
    };
    let e = assemble_m68k("t.s", "a: nop\n    move.l #a*2,d0\n", &opts, &NoFiles).unwrap_err();
    assert!(
        e.iter().any(|d| d.message.contains("cannot be relocated")),
        "{e:?}"
    );
    let e = assemble_m68k("t.s", "a: nop\n    move.w #a,d0\n", &opts, &NoFiles).unwrap_err();
    assert!(e.iter().any(|d| d.message.contains("16-bit")), "{e:?}");
    let e = assemble_m68k("t.s", "    .org $1000\n    nop\n", &opts, &NoFiles).unwrap_err();
    assert!(e.iter().any(|d| d.message.contains("relocatable")), "{e:?}");
}

#[test]
fn errors_carry_file_and_line_and_do_not_stop_at_the_first() {
    let e = flat("    nop\n    bogus\n    move.w d0,#1\n").unwrap_err();
    assert!(e.contains("test.s:2:") && e.contains("test.s:3:"), "{e}");
}

#[test]
fn duplicate_labels_and_undefined_symbols_are_errors() {
    assert!(
        flat("a: nop\na: nop\n")
            .unwrap_err()
            .contains("defined twice")
    );
    assert!(
        flat("    move.l #nowhere,d0\n")
            .unwrap_err()
            .contains("undefined symbol \"nowhere\"")
    );
}

#[test]
fn odd_addresses_for_words_and_instructions_are_errors() {
    assert!(
        flat("    dc.b 1\n    nop\n")
            .unwrap_err()
            .contains("odd address")
    );
    assert!(
        flat("    dc.b 1\n    dc.w 2\n")
            .unwrap_err()
            .contains("odd address")
    );
    assert!(flat("    dc.b 1\n    .even\n    nop\n").is_ok());
}

#[test]
fn includes_are_read_and_a_loop_is_stopped() {
    struct One;
    impl Files for One {
        fn read(&self, path: &Path) -> Result<Vec<u8>, String> {
            match path.to_str() {
                Some("inc.s") => Ok(b"    nop\n".to_vec()),
                Some("loop.s") => Ok(b"    .include \"loop.s\"\n".to_vec()),
                _ => Err("no".to_string()),
            }
        }
    }
    let ok = assemble_m68k(
        "t.s",
        "    .include \"inc.s\"\n    rts\n",
        &M68kOptions::default(),
        &One,
    )
    .unwrap();
    assert_eq!(ok.image(), vec![0x4E, 0x71, 0x4E, 0x75]);
    let e = assemble_m68k(
        "t.s",
        "    .include \"loop.s\"\n",
        &M68kOptions::default(),
        &One,
    )
    .unwrap_err();
    assert!(e[0].message.contains("too deep"), "{e:?}");
}

#[test]
fn hostile_input_never_panics() {
    let mut state = 0x1234_5678_9ABC_DEF1u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let pieces = [
        "move",
        "add",
        ".w",
        ".l",
        ".b",
        "d0",
        "a7",
        "(",
        ")",
        ",",
        "#",
        "$",
        "-",
        "+",
        "pc",
        "sp",
        "movem",
        "d0-d7/a0",
        "bra",
        "label",
        ":",
        "dc.b",
        "\"",
        "'",
        "ds.l",
        "\n",
        " ",
        "    ",
        "0x10",
        "1e9",
        "equ",
        ".org",
        "*",
        "<<",
        "999999999999",
        ".include",
        ".opt",
        "autobranch",
        "lea",
        "(a0,d1.w)",
        "$10(a0)",
        "btst",
        "trap",
        "dbra",
    ];
    for _ in 0..4_000
        * std::env::var("RC_SWEEP")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1)
    {
        let n = 1 + next() % 14;
        let mut line = String::from("    ");
        for _ in 0..n {
            line.push_str(pieces[(next() % pieces.len() as u64) as usize]);
            if next() % 3 == 0 {
                line.push(' ');
            }
        }
        line.push('\n');
        let _ = assemble_m68k("fuzz.s", &line, &M68kOptions::default(), &NoFiles);
        let _ = assemble_m68k(
            "fuzz.s",
            &line,
            &M68kOptions {
                relocatable: true,
                ..M68kOptions::default()
            },
            &NoFiles,
        );
    }
}
