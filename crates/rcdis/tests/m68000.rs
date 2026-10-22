//! The 68000 disassembler writes source that assembles back to the same bytes.

use rcasm::Files;
use rcasm::m68k::{M68kOptions, assemble_m68k};
use rcdis::m68k::{M68kDisOptions, disassemble_m68k};
use std::path::Path;

struct NoFiles;

impl Files for NoFiles {
    fn read(&self, path: &Path) -> Result<Vec<u8>, String> {
        Err(format!("no file {}", path.display()))
    }
}

fn assemble(source: &str) -> Vec<u8> {
    assemble_m68k("t.s", source, &M68kOptions::default(), &NoFiles)
        .unwrap_or_else(|e| {
            panic!(
                "{}",
                e.iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })
        .image()
}

/// What `assemble` made must be the image, bar the one pad byte the assembler puts after an odd last byte.
fn same_image(made: &[u8], image: &[u8]) -> bool {
    made.len() >= image.len() && made.len() - image.len() <= 1 && made[..image.len()] == *image
}

fn opts(org: u32) -> M68kDisOptions {
    M68kDisOptions {
        org,
        comments: false,
    }
}

#[test]
fn a_program_comes_back_with_labels() {
    let image = assemble(
        "    .org $8000
start:
    lea     msg(pc),a0
loop:
    move.b  (a0)+,d0
    beq.s   done
    bsr.s   emit
    bra.s   loop
done:
    rts
emit:
    move.w  d0,-(sp)
    trap    #1
    addq.l  #2,sp
    rts
msg:
    dc.b    \"HI\",0
    .even
",
    );
    let d = disassemble_m68k(&image, &opts(0x8000)).unwrap();
    assert!(
        d.source.contains("L_008004:"),
        "the loop target is labelled:\n{}",
        d.source
    );
    assert!(d.source.contains("bsr.s L_00800e"), "{}", d.source);
    // `msg` is data, not an instruction, so the PC-relative operand stays an address.
    assert!(d.source.contains("lea $8016(pc),a0"), "{}", d.source);
    assert_eq!(assemble(&d.source), image, "{}", d.source);
}

#[test]
fn a_word_that_is_no_instruction_is_data() {
    let image = [0x4E, 0x71, 0xFF, 0xFF, 0x4E, 0x75, 0x12];
    let d = disassemble_m68k(&image, &opts(0x1000)).unwrap();
    assert!(d.source.contains("dc.w $ffff"), "{}", d.source);
    assert!(
        d.source.contains("dc.b $12"),
        "a trailing odd byte:\n{}",
        d.source
    );
    assert_eq!(d.instructions, 2);
    assert_eq!(d.data_bytes, 3);
    assert!(same_image(&assemble(&d.source), &image), "{}", d.source);
}

/// Bits the assembler would not produce (the unused bits of an index word, a byte immediate's upper byte) make the
/// word data rather than an instruction that would change when assembled again.
#[test]
fn a_word_with_unused_bits_set_stays_exactly_as_it_was() {
    // move.b #$12,d0 with the upper byte of the immediate word set.
    let image = [0x10, 0x3C, 0xAB, 0x12];
    let d = disassemble_m68k(&image, &opts(0)).unwrap();
    assert_eq!(assemble(&d.source), image, "{}", d.source);
}

#[test]
fn an_odd_origin_is_an_error() {
    assert!(disassemble_m68k(&[0x4E, 0x71], &opts(1)).is_err());
    assert!(disassemble_m68k(&[0x4E, 0x71], &opts(0xFFFFFF)).is_err());
}

#[test]
fn addresses_go_in_comments_on_request() {
    let d = disassemble_m68k(
        &[0x4E, 0x71],
        &M68kDisOptions {
            org: 0x400,
            comments: true,
        },
    )
    .unwrap();
    assert!(d.source.contains("nop    ; 000400: 4E 71"), "{}", d.source);
}

/// Whatever bytes come in, what goes out assembles back to them.
#[test]
fn random_bytes_assemble_back_to_themselves() {
    let mut state = 0xDEAD_BEEF_CAFE_F00Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for round in 0..40
        * std::env::var("RC_SWEEP")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1)
    {
        let len = 2 + (next() % 3000) as usize;
        // Real code has a lot of structure: mix noise with opcode-shaped words.
        let image: Vec<u8> = (0..len)
            .map(|i| {
                if round % 2 == 0 || i % 2 == 1 {
                    next() as u8
                } else {
                    [0x4E, 0x30, 0x20, 0x60, 0x67, 0x51, 0xD0, 0x41][(next() % 8) as usize]
                }
            })
            .collect();
        let org = 0x2000 * (round as u32 % 5);
        let d = disassemble_m68k(&image, &opts(org)).unwrap();
        let again = assemble_m68k("t.s", &d.source, &M68kOptions::default(), &NoFiles)
            .unwrap_or_else(|e| panic!("round {round}: {}", e[0]))
            .image();
        assert!(
            same_image(&again, &image),
            "round {round}: the image does not come back"
        );
    }
}

fn emutos() -> Option<Vec<u8>> {
    let path = std::env::var_os("RC_EMUTOS")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let base = std::env::var_os("LOCALAPPDATA")
                .map(|p| Path::new(&p).join("retroclawd").join("cache"))
                .or_else(|| {
                    std::env::var_os("XDG_CACHE_HOME").map(|p| Path::new(&p).join("retroclawd"))
                })
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(|p| Path::new(&p).join(".cache").join("retroclawd"))
                })?;
            Some(
                base.join("firmware")
                    .join("904ea9dc09ff8286f6e0272c12255dfe85beb17a.bin"),
            )
        })?;
    std::fs::read(path).ok()
}

/// The EmuTOS 1.4 image from `--firmware auto`'s cache (or `RC_EMUTOS`): all 192 KB of it disassemble and assemble back
/// byte for byte. Skipped (and said so) when it is not there.
#[test]
fn rcdis_of_the_emutos_image_assembles_to_identical_bytes() {
    let Some(image) = emutos() else {
        eprintln!(
            "EmuTOS not found (set RC_EMUTOS or run `retroclawd atari-st --firmware auto` once): skipped"
        );
        return;
    };
    let d = disassemble_m68k(&image, &opts(0xFC0000)).unwrap();
    assert!(d.instructions > 20_000, "{} instructions", d.instructions);
    let again = assemble_m68k("emutos.s", &d.source, &M68kOptions::default(), &NoFiles)
        .unwrap_or_else(|e| panic!("{}", e[0]))
        .image();
    assert!(same_image(&again, &image), "the reassembled image differs");
}
