//! ZX81 suite: what the real machine does, checked against the emulator.
//!
//! The expected values are hardware facts, each named in its case:
//!
//! * `ZXDOCS` = Martin Korth's "Sinclair ZX Specifications", <https://problemkaputt.de/zxdocs.htm> (memory
//!   map, system variables, line and NMI timing).
//! * `ROM` = the 8K ZX81 ROM itself (the character set at 0x1E00, the routines it runs).
//! * `Z80` = Zilog's instruction timing tables.
//!
//! Needs `RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS` and `RC_ROMS` (see the crate docs); skips without.

use rc_suite::{Area, Case, Env, expect_eq, expect_in, run_cases};
use rcasm::{Cpu, DiskFiles, Options, assemble, flat_image};
use std::path::PathBuf;

const ROM_FILE: &str = "sinclair/roms/zx81/zx81.rom";

// System variables (ZXDOCS, "system variables"), RAM at 0x4000.
const ERR_NR: u16 = 0x4000;
const RAMTOP: u16 = 0x4004;
const DF_SZ: u16 = 0x4022;
const MARGIN: u16 = 0x4028;
const FRAMES: u16 = 0x4034;

/// Frames a ROM needs from reset to the `K` prompt: ~1.5 s at 50 Hz, with room to spare.
const BOOT_FRAMES: u32 = 100;

fn rom(env: &Env) -> Result<PathBuf, String> {
    env.file(ROM_FILE)
}

/// Assembles `source` at `org` and returns the file to hand to `--program`.
fn program(tag: &str, org: u16, source: &str) -> Result<PathBuf, String> {
    let opts = Options {
        cpu: Some(Cpu::Z80),
        ..Options::default()
    };
    let asm = assemble(
        "case.asm",
        &format!(".org ${org:04X}\n{source}"),
        &opts,
        &DiskFiles,
    )
    .map_err(|e| format!("{e:?}"))?;
    let (start, image) = flat_image(&asm, 0)?;
    expect_eq("program start", start, org)?;
    let path = std::env::temp_dir().join(format!("rc-suite-zx81-{tag}-{}.bin", std::process::id()));
    std::fs::write(&path, image).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Runs `source` loaded at 0x4100 for `frames` frames. The ROM is not booted: the CPU starts in the program,
/// with the NMI generator off (FAST mode), as it is at reset.
fn run_program(
    env: &Env,
    tag: &str,
    source: &str,
    frames: u32,
) -> Result<rc_suite::Outcome, String> {
    let file = program(tag, 0x4100, source)?;
    env.run("zx81")
        .rom(&rom(env)?)
        .arg("--program")
        .arg(file.to_string_lossy())
        .arg("--org")
        .arg("0x4100")
        .frames(frames)
        .go()
}

fn boots_to_the_k_prompt(env: &Env) -> Result<(), String> {
    let out = env
        .run("zx81")
        .rom(&rom(env)?)
        .arg("--no-border")
        .frames(BOOT_FRAMES)
        .dump(ERR_NR, 0x40)
        .screen()
        .go()?;
    // ZXDOCS: ERR_NR holds the report code minus one, 0xFF = "0 OK"; RAMTOP points past the detected RAM
    // (0x4400 for the stock 1K); MARGIN is 55 blank lines for a 50 Hz machine; DF_SZ is the 2 lines of the
    // lower screen the prompt lives in.
    expect_eq("ERR_NR", out.byte(ERR_NR)?, 0xFF)?;
    expect_eq("RAMTOP", out.word(RAMTOP)?, 0x4400)?;
    expect_eq("MARGIN", out.byte(MARGIN)?, 55)?;
    expect_eq("DF_SZ", out.byte(DF_SZ)?, 2)?;

    // ROM: the prompt is an inverse `K` (character code 0x30) in the first column of the bottom line, and
    // nothing else is drawn. Its pixels are the ROM's own character set at 0x1E00, inverted.
    let screen = out.screen()?;
    expect_eq("picture size", (screen.width, screen.height), (256, 192))?;
    let rom_bytes = std::fs::read(rom(env)?).map_err(|e| e.to_string())?;
    let glyph = &rom_bytes[0x1E00 + 0x30 * 8..0x1E00 + 0x30 * 8 + 8];
    let inverse: Vec<u8> = glyph.iter().map(|b| !b).collect();
    let cell = screen.cell(0, 23 * 8);
    expect_eq("bottom-left cell", cell.to_vec(), inverse)?;
    let cell_ink: usize = cell.iter().map(|b| b.count_ones() as usize).sum();
    expect_eq("ink outside the K cell", screen.ink_count() - cell_ink, 0)
}

fn slow_mode_counts_50_frames_a_second(env: &Env) -> Result<(), String> {
    let at = |frames| -> Result<u16, String> {
        env.run("zx81")
            .rom(&rom(env)?)
            .frames(frames)
            .dump(FRAMES, 2)
            .go()?
            .word(FRAMES)
    };
    let early = at(BOOT_FRAMES)?;
    let later = at(BOOT_FRAMES + 50)?;
    // ZXDOCS: FRAMES is a decrementing 15-bit counter (bit 15 is a flag) that the ROM's display routine steps once
    // per displayed frame; the picture is 50 Hz, so 50 frames of running time is 50 steps.
    let steps = i64::from(early.wrapping_sub(later) & 0x7FFF);
    expect_in("FRAMES steps over 50 frames", steps, 49, 51)
}

fn fast_mode_runs_at_3_25_mhz_with_no_nmis(env: &Env) -> Result<(), String> {
    const FRAMES_RUN: u32 = 10;
    let out = run_program(env, "fast", "ld hl,0\nloop: inc hl\n jr loop\n", FRAMES_RUN)?;
    // ZXDOCS: the CPU runs at 3.25 MHz and the picture is 50 Hz, so a frame is 65000 T-states. A frame ends at
    // the first instruction boundary past that, so each one may run over by less than one instruction (JR is
    // the longest here, 12 T).
    expect_in(
        "cycles in 10 frames",
        out.cycles as i64,
        i64::from(FRAMES_RUN) * 65_000,
        i64::from(FRAMES_RUN) * (65_000 + 12),
    )?;
    // Z80: LD HL,nn = 10 T; INC HL = 6 T; JR taken = 12 T. With the NMI generator off (FAST mode, as at reset)
    // nothing interrupts the loop, so HL counts one per 18 T after the first 10.
    let hl = i64::from(out.reg("HL")?);
    let cycles = out.cycles as i64;
    let want = (cycles - 10) / 18;
    expect_in("HL after the loop", hl, want, want + 1)
}

fn rom_is_shadowed_at_0x2000(env: &Env) -> Result<(), String> {
    let out = env
        .run("zx81")
        .rom(&rom(env)?)
        .frames(2)
        .dump(0x0000, 0x40)
        .dump(0x2000, 0x40)
        .dump(0x3FC0, 0x40)
        .dump(0x1FC0, 0x40)
        .go()?;
    // ZXDOCS: the ROM select signal mirrors the 8K ROM across 0x0000-0x3FFF.
    let file = std::fs::read(rom(env)?).map_err(|e| e.to_string())?;
    expect_eq(
        "ROM at 0x0000",
        out.bytes(0x0000, 0x40)?,
        file[..0x40].to_vec(),
    )?;
    expect_eq(
        "ROM at 0x2000",
        out.bytes(0x2000, 0x40)?,
        file[..0x40].to_vec(),
    )?;
    expect_eq(
        "ROM end at 0x1FC0",
        out.bytes(0x1FC0, 0x40)?,
        file[0x1FC0..0x2000].to_vec(),
    )?;
    expect_eq(
        "ROM end at 0x3FC0",
        out.bytes(0x3FC0, 0x40)?,
        file[0x1FC0..0x2000].to_vec(),
    )
}

fn rom_ignores_writes(env: &Env) -> Result<(), String> {
    let out = run_program(
        env,
        "romwrite",
        "ld a,$AA\n ld ($0000),a\n ld ($2000),a\n ld ($1FFF),a\n ld a,($0000)\n ld b,a\nhere: jr here\n",
        2,
    )?;
    // A ROM cannot be written: the cell keeps its value, and so does its shadow.
    let file = std::fs::read(rom(env)?).map_err(|e| e.to_string())?;
    expect_eq(
        "read-back of ROM[0] after a write",
        out.reg("BC")? >> 8,
        u16::from(file[0]),
    )
}

fn ram_1k_repeats_through_0x4000_0x7fff(env: &Env) -> Result<(), String> {
    let out = run_program(
        env,
        "ram1k",
        "ld a,$5A\n ld ($4300),a\n ld a,($4700)\n ld b,a\n ld a,($5B00)\n ld c,a\n ld a,($7F00)\n ld d,a\nhere: jr here\n",
        2,
    )?;
    // ZXDOCS: the 1K of RAM at 0x4000-0x43FF is repeated through 0x4000-0x7FFF, so 0x4700, 0x5B00 and 0x7F00
    // are the same cell as 0x4300.
    expect_eq("0x4700", out.reg("BC")? >> 8, 0x5A)?;
    expect_eq("0x5B00", out.reg("BC")? & 0xFF, 0x5A)?;
    expect_eq("0x7F00", out.reg("DE")? >> 8, 0x5A)
}

fn data_reads_above_0xc000_see_the_ram(env: &Env) -> Result<(), String> {
    let out = run_program(
        env,
        "echo-data",
        "ld a,$6B
 ld ($4200),a
 ld a,($C200)
 ld b,a
 ld a,($4200)
 ld c,a
here: jr here
",
        2,
    )?;
    // ZXDOCS: the RAM select also answers at 0xC000-0xFFFF for data reads, so 0xC200 is the cell at 0x4200.
    expect_eq("read at 0xC200", out.reg("BC")? >> 8, 0x6B)
}

fn opcodes_with_bit_6_clear_run_as_nops_above_0x8000(env: &Env) -> Result<(), String> {
    // The program copies three INC A (0x3C) and a HALT (0x76) to 0x4300, then jumps to their echo at 0xC300.
    let out = run_program(
        env,
        "echo-fetch",
        "ld hl,bytes
 ld de,$4300
 ld bc,4
 ldir
 xor a
 jp $C300
bytes: .byte $3C,$3C,$3C,$76
",
        2,
    )?;
    // ZXDOCS: an opcode fetch at 0x8000 or above whose byte has bit 6 clear is a NOP to the CPU (the byte goes
    // to the picture instead), so the INC A bytes do nothing; 0x76 has bit 6 set and is a real HALT.
    expect_eq(
        "A after the three INC A in the echo area",
        out.reg("AF")? >> 8,
        0,
    )?;
    expect_in(
        "PC after the HALT",
        i64::from(out.reg("PC")?),
        0xC303,
        0xC304,
    )
}

fn sixteen_k_ram_pack_is_detected(env: &Env) -> Result<(), String> {
    // ZXDOCS: RAMTOP points past the RAM the ROM found at power-on; with a 16K pack that is 0x8000.
    let out = env
        .run("zx81")
        .rom(&rom(env)?)
        .arg("--ram-kb")
        .arg("16")
        .frames(BOOT_FRAMES)
        .dump(RAMTOP, 2)
        .go()?;
    expect_eq("RAMTOP with 16K", out.word(RAMTOP)?, 0x8000)
}

const CASES: &[Case] = &[
    Case::new(
        Area::Video,
        "boots to the K prompt",
        "ZXDOCS system variables; ROM character set at 0x1E00",
        boots_to_the_k_prompt,
    ),
    Case::new(
        Area::Timers,
        "SLOW mode counts 50 frames a second",
        "ZXDOCS FRAMES (0x4034), 50 Hz picture",
        slow_mode_counts_50_frames_a_second,
    )
    .gap("roadmap item 3: no video fetch -- the display routine's jump into 0xC000+ runs unmapped 0xFF"),
    Case::new(
        Area::Cpu,
        "FAST mode runs at 3.25 MHz with no NMIs",
        "ZXDOCS CPU clock; Z80 timing tables",
        fast_mode_runs_at_3_25_mhz_with_no_nmis,
    ),
    Case::new(
        Area::Rom,
        "the ROM is shadowed across 0x0000-0x3FFF",
        "ZXDOCS memory mirroring",
        rom_is_shadowed_at_0x2000,
    )
    .gap("roadmap item 2: the memory map has no mirrors (0x2000-0x3FFF reads 0xFF)"),
    Case::new(
        Area::Memory,
        "the ROM ignores writes",
        "a ROM is read-only",
        rom_ignores_writes,
    ),
    Case::new(
        Area::Memory,
        "the 1K RAM repeats through 0x4000-0x7FFF",
        "ZXDOCS memory mirroring",
        ram_1k_repeats_through_0x4000_0x7fff,
    )
    .gap("roadmap item 2: the memory map has no mirrors (0x4400-0x7FFF reads 0xFF)"),
    Case::new(
        Area::Memory,
        "data reads above 0xC000 see the RAM",
        "ZXDOCS memory mirroring",
        data_reads_above_0xc000_see_the_ram,
    )
    .gap("roadmap item 2: the memory map has no mirrors (0xC000-0xFFFF reads 0xFF)"),
    Case::new(
        Area::Video,
        "opcodes with bit 6 clear run as NOPs above 0x8000",
        "ZXDOCS 8000h-FFFFh (video output, machine code cannot run there)",
        opcodes_with_bit_6_clear_run_as_nops_above_0x8000,
    )
    .gap("roadmap item 3: no M1 distinction, so the CPU runs the echo area as ordinary code"),
    Case::new(
        Area::Memory,
        "a 16K RAM pack is detected",
        "ZXDOCS RAMTOP",
        sixteen_k_ram_pack_is_detected,
    )
    .gap("roadmap item 2: the RAM is a fixed 1K, no pack option yet"),
];

#[test]
fn zx81() {
    let Some(env) = Env::discover() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    if env.file(ROM_FILE).is_err() {
        eprintln!("skipping: {ROM_FILE} not present");
        return;
    }
    let report = run_cases(&env, CASES);
    eprintln!("{}", report.summary());
    assert!(report.ok(), "{}", report.summary());
}
