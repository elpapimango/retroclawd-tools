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

use rc_suite::{Area, Case, Env, expect_eq, expect_in, run_cases, z80_program};
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

/// Runs `source` loaded at 0x4100 for `frames` frames. The ROM is not booted: the CPU starts in the program,
/// with the NMI generator off (FAST mode), as it is at reset.
fn run_program(
    env: &Env,
    tag: &str,
    source: &str,
    frames: u32,
) -> Result<rc_suite::Outcome, String> {
    let file = z80_program(tag, 0x4100, source)?;
    env.run("zx81")
        .rom(&rom(env)?)
        .arg("--program")
        .arg(file.to_string_lossy())
        .arg("--org")
        .arg("0x4100")
        .frames(frames)
        .go()
}

fn boot_to_the_k_prompt(env: &Env, rom_path: &std::path::Path) -> Result<(), String> {
    let out = env
        .run("zx81")
        .rom(rom_path)
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
    let rom_bytes = std::fs::read(rom_path).map_err(|e| e.to_string())?;
    let glyph = &rom_bytes[0x1E00 + 0x30 * 8..0x1E00 + 0x30 * 8 + 8];
    let inverse: Vec<u8> = glyph.iter().map(|b| !b).collect();
    let cell = screen.cell(0, 23 * 8);
    expect_eq("bottom-left cell", cell.to_vec(), inverse)?;
    let cell_ink: usize = cell.iter().map(|b| b.count_ones() as usize).sum();
    expect_eq("ink outside the K cell", screen.ink_count() - cell_ink, 0)
}

fn boots_to_the_k_prompt(env: &Env) -> Result<(), String> {
    boot_to_the_k_prompt(env, &rom(env)?)
}

/// The three ZX81 ROM dumps RomM holds (the standard edition, the 550 original and the `b` edition). Whatever
/// the revision, the machine must reach the same stable state: the same system variables, and the prompt drawn
/// from *that* ROM's own character set.
const ROM_REVISIONS: [&str; 3] = [
    "sinclair/roms/zx81/zx81.rom",
    "sinclair/roms/zx81/zx81-550-original.rom",
    "sinclair/roms/zx81/zx81b.rom",
];

fn every_rom_revision_boots_to_the_k_prompt(env: &Env) -> Result<(), String> {
    let mut tried = 0;
    for name in ROM_REVISIONS {
        let Ok(path) = env.file(name) else {
            eprintln!("  (skipping {name}: not present)");
            continue;
        };
        boot_to_the_k_prompt(env, &path).map_err(|e| format!("{name}: {e}"))?;
        tried += 1;
    }
    expect_in("ROM revisions present", tried, 1, 3)
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

/// RAM sizes the plugin offers (`--ram-kb`) and where the ROM's power-on test leaves RAMTOP for each.
///
/// ZXDOCS ("Memory Mirrors and Expansions"): RAMTOP points to the address after the detected RAM (4400h for the 1K
/// machine, so 4800h for 2K), and the ROM's memory detection ends at 8000h -- a 16K pack gives 8000h, and a 32K
/// or 56K pack is not detected beyond it ("RAMTOP must be changed manually by POKE").
const RAM_SIZES: [(u32, u16); 5] = [
    (1, 0x4400),
    (2, 0x4800),
    (16, 0x8000),
    (32, 0x8000),
    (56, 0x8000),
];

fn the_power_on_ram_test_sets_ramtop_for_every_ram_size(env: &Env) -> Result<(), String> {
    for (kb, want) in RAM_SIZES {
        let out = env
            .run("zx81")
            .rom(&rom(env)?)
            .arg("--ram-kb")
            .arg(kb.to_string())
            .frames(BOOT_FRAMES)
            .dump(RAMTOP, 2)
            .go()?;
        expect_eq(&format!("RAMTOP with {kb}K"), out.word(RAMTOP)?, want)?;
    }
    Ok(())
}

fn the_last_cell_of_every_ram_size_is_ram(env: &Env) -> Result<(), String> {
    // ZXDOCS: the RAM is 4000h up for 1K-48K and 2000h up for the 56K pack ("only 2000h-FFFFh is usable"). Two
    // different values written at the first and last cell must both stay: the RAM is that big and does not wrap.
    // (1K and 2K repeat through 4000h-7FFFh; the mirror case covers them.)
    for (kb, first, last) in [
        (16u32, 0x4000u16, 0x7FFFu16),
        (32, 0x4000, 0xBFFF),
        (56, 0x2000, 0xFFFF),
    ] {
        let source = format!(
            "ld a,$A5\n ld (${first:04X}),a\n ld a,$5A\n ld (${last:04X}),a\n ld a,(${first:04X})\n ld b,a\n ld a,(${last:04X})\n ld c,a\nhere: jr here\n"
        );
        let program = z80_program("ramtop", 0x4100, &source)?;
        let out = env
            .run("zx81")
            .rom(&rom(env)?)
            .arg("--ram-kb")
            .arg(kb.to_string())
            .arg("--program")
            .arg(program.to_string_lossy())
            .arg("--org")
            .arg("0x4100")
            .frames(2)
            .go()?;
        let bc = out.reg("BC")?;
        expect_eq(&format!("first cell with {kb}K"), bc >> 8, 0xA5)?;
        expect_eq(&format!("last cell with {kb}K"), bc & 0xFF, 0x5A)?;
    }
    Ok(())
}

// ---- `.p` files (ZXDOCS "Cassette File Images / Content"): a copy of memory from 4009h to E_LINE-1, starting with
// the system variables (VERSN = 0 for a ZX81). NXTLIN pointing at a program line is the autostart.

const E_LINE: u16 = 0x4014;
const PROGRAM: u16 = 0x407D;

/// A `.p` file holding the BASIC `lines` (each a line number and its tokenised body, without the line's final
/// NEWLINE), with the collapsed display file (25 HALTs) the ZX81 keeps in SLOW mode and NXTLIN on the first line.
fn p_file(lines: &[(u16, &[u8])]) -> Vec<u8> {
    let mut program = Vec::new();
    for (number, body) in lines {
        program.extend_from_slice(&number.to_be_bytes());
        program.extend_from_slice(&(body.len() as u16 + 1).to_le_bytes());
        program.extend_from_slice(body);
        program.push(0x76);
    }
    let d_file = PROGRAM + program.len() as u16;
    let vars = d_file + 25;
    let e_line = vars + 1;
    let mut file = vec![0u8; usize::from(PROGRAM - 0x4009)];
    let mut word = |addr: u16, value: u16| {
        let i = usize::from(addr - 0x4009);
        file[i..i + 2].copy_from_slice(&value.to_le_bytes());
    };
    word(0x400C, d_file); // D_FILE
    word(0x400E, d_file + 1); // DF_CC
    word(0x4010, vars); // VARS
    word(0x4014, e_line); // E_LINE
    word(0x4016, e_line - 1); // CH_ADD
    word(0x401A, e_line + 2); // STKBOT
    word(0x401C, e_line + 2); // STKEND
    word(0x401F, 0x405D); // MEM
    word(0x4025, 0xFFFF); // LAST_K
    word(0x4029, PROGRAM); // NXTLIN: the first line is the autostart
    word(0x4030, 0x0C8D); // T_ADDR
    word(0x4034, 0xFFFF); // FRAMES
    file[0x4022 - 0x4009] = 2; // DF_SZ
    file[0x4028 - 0x4009] = 55; // MARGIN
    file[0x4038 - 0x4009] = 0xBC; // PR_CC
    file[0x403B - 0x4009] = 0x40; // CDFLAG: SLOW mode
    file.extend_from_slice(&program);
    file.extend(std::iter::repeat_n(0x76, 25)); // the collapsed display file
    file.push(0x80); // end of the variables
    file
}

fn temp_file(tag: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(format!("rc-suite-zx81-{tag}-{}.p", std::process::id()));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

fn a_p_file_is_loaded_at_0x4009_and_its_program_autostarts(env: &Env) -> Result<(), String> {
    // ZX81 tokens (the manual's character set table): PRINT = F5h; `"` = 0Bh; letters start at 26h (A = 26h).
    let file = p_file(&[(1, &[0xF5, 0x0B, 0x26, 0x0B])]); // 1 PRINT "A"
    let path = temp_file("print", &file)?;
    let out = env
        .run("zx81")
        .rom(&rom(env)?)
        .arg("--no-border")
        .arg("--snapshot")
        .arg(path.to_string_lossy())
        .frames(BOOT_FRAMES)
        .dump(PROGRAM, 9)
        .screen()
        .go()?;
    // ZXDOCS: the file is memory from 4009h, so the program is where VERSN..PROGRAM put it.
    let at = usize::from(PROGRAM - 0x4009);
    expect_eq(
        "the BASIC line in RAM",
        out.bytes(PROGRAM, 9)?,
        file[at..at + 9].to_vec(),
    )?;
    // ROM: the autostarted line prints an `A` in the top left cell, drawn from the ROM's own character set (1E00h).
    let rom_bytes = std::fs::read(rom(env)?).map_err(|e| e.to_string())?;
    let glyph = rom_bytes[0x1E00 + 0x26 * 8..0x1E00 + 0x26 * 8 + 8].to_vec();
    expect_eq("top-left cell", out.screen()?.cell(0, 0).to_vec(), glyph)
}

fn saving_a_p_file_writes_memory_from_0x4009_to_e_line(env: &Env) -> Result<(), String> {
    let loaded = temp_file("save-in", &p_file(&[(1, &[0xF5, 0x0B, 0x26, 0x0B])]))?;
    let saved = std::env::temp_dir().join(format!("rc-suite-zx81-saved-{}.p", std::process::id()));
    let _ = std::fs::remove_file(&saved);
    let e_line = env
        .run("zx81")
        .rom(&rom(env)?)
        .arg("--snapshot")
        .arg(loaded.to_string_lossy())
        .frames(BOOT_FRAMES)
        .dump(E_LINE, 2)
        .go()?
        .word(E_LINE)?;
    let len = e_line - 0x4009;
    // The same deterministic run, now saving a `.p` and reading back the memory it should contain.
    let out = env
        .run("zx81")
        .rom(&rom(env)?)
        .arg("--snapshot")
        .arg(loaded.to_string_lossy())
        .arg("--dump-snapshot")
        .arg(saved.to_string_lossy())
        .frames(BOOT_FRAMES)
        .dump(0x4009, len)
        .go()?;
    // ZXDOCS: a ZX81 file is a copy of memory from 4009h up to E_LINE-1 (its length is E_LINE - 4009h).
    let file = std::fs::read(&saved).map_err(|e| format!("no .p was written: {e}"))?;
    expect_eq("file length", file.len(), usize::from(len))?;
    expect_eq("file content", file, out.bytes(0x4009, len)?)
}

fn a_saved_file_loads_back_and_runs(env: &Env) -> Result<(), String> {
    let loaded = temp_file("rt-in", &p_file(&[(1, &[0xF5, 0x0B, 0x26, 0x0B])]))?;
    let pid = std::process::id();
    let save = |ext: &str| -> Result<std::path::PathBuf, String> {
        let path = std::env::temp_dir().join(format!("rc-suite-zx81-rt-{pid}.{ext}"));
        let _ = std::fs::remove_file(&path);
        env.run("zx81")
            .rom(&rom(env)?)
            .arg("--snapshot")
            .arg(loaded.to_string_lossy())
            .arg("--dump-snapshot")
            .arg(path.to_string_lossy())
            .frames(BOOT_FRAMES)
            .go()?;
        Ok(path)
    };
    let (p, e81) = (save("p")?, save("81")?);
    let read = |path: &std::path::Path| std::fs::read(path).map_err(|e| e.to_string());
    expect_eq(".81 against .p", read(&e81)?, read(&p)?)?;
    // ROM: loading the saved file autostarts the program again, so the `A` is drawn from the ROM's character set.
    let rom_bytes = std::fs::read(rom(env)?).map_err(|e| e.to_string())?;
    let glyph = rom_bytes[0x1E00 + 0x26 * 8..0x1E00 + 0x26 * 8 + 8].to_vec();
    for path in [&p, &e81] {
        let out = env
            .run("zx81")
            .rom(&rom(env)?)
            .arg("--no-border")
            .arg("--snapshot")
            .arg(path.to_string_lossy())
            .frames(BOOT_FRAMES)
            .screen()
            .go()?;
        expect_eq(
            "top-left cell",
            out.screen()?.cell(0, 0).to_vec(),
            glyph.clone(),
        )?;
    }
    Ok(())
}

fn load_reads_a_program_from_the_tape_input(env: &Env) -> Result<(), String> {
    let file = p_file(&[(1, &[0xF5, 0x0B, 0x26, 0x0B])]); // 1 PRINT "A"
    let path = temp_file("tape", &file)?;
    // ZXDOCS: the ZX81 loads through the EAR input (bit 7 of port 0xFE), and what `LOAD ""` brings in is a copy of
    // memory from 4009h, so the BASIC line lands at 407Dh. `J` is the LOAD keyword; `""` is Shift+P twice.
    let out = env
        .run("zx81")
        .rom(&rom(env)?)
        .arg("--tape")
        .arg(path.to_string_lossy())
        .arg("--type-after")
        .arg("100")
        .arg("--type-delay")
        .arg("160")
        .arg("--type")
        .arg(
            "J\"\"
",
        )
        .frames(2000)
        .dump(PROGRAM, 9)
        .go()?;
    let at = usize::from(PROGRAM - 0x4009);
    expect_eq(
        "the BASIC line LOAD brought in",
        out.bytes(PROGRAM, 9)?,
        file[at..at + 9].to_vec(),
    )
}

/// ZX81 character codes (the manual's character set) of the characters a key types.
fn zx81_code(c: char) -> Option<u8> {
    Some(match c {
        ' ' => 0x00,
        '"' => 0x0B,
        '$' => 0x0D,
        ':' => 0x0E,
        '?' => 0x0F,
        '(' => 0x10,
        ')' => 0x11,
        '>' => 0x12,
        '<' => 0x13,
        '=' => 0x14,
        '+' => 0x15,
        '-' => 0x16,
        '*' => 0x17,
        '/' => 0x18,
        ';' => 0x19,
        ',' => 0x1A,
        '.' => 0x1B,
        '0'..='9' => 0x1C + (c as u8 - b'0'),
        'A'..='Z' => 0x26 + (c as u8 - b'A'),
        '\n' => 0x76,
        _ => return None,
    })
}

/// The keyboard matrix position (half row 0-7, bit 0-4) the ROM's key table puts a character at. The ROM's `KEYBOARD`
/// routine scans the 40 keys in the order of these 39-byte tables (every key but Shift, which is row 0 bit 0): the
/// unshifted table is at 0x007E and the shifted one at 0x00A5.
fn rom_key_position(rom: &[u8], shifted: bool, c: char) -> Result<(usize, u8), String> {
    let table = if shifted { 0x00A5 } else { 0x007E };
    let code = zx81_code(c).ok_or_else(|| format!("no ZX81 character code for {c:?}"))?;
    let index = rom[table..table + 39]
        .iter()
        .position(|&b| b == code)
        .ok_or_else(|| {
            format!(
                "{c:?} is not in the ROM's {} key table",
                if shifted { "shifted" } else { "unshifted" }
            )
        })?;
    Ok(if index < 4 {
        (0, 1 + index as u8)
    } else {
        (1 + (index - 4) / 5, ((index - 4) % 5) as u8)
    })
}

/// The ports' half-row selects, row 0 first (a row is selected by a 0 on its address line A8-A15).
const ROW_SELECTS: [u8; 8] = [0xFE, 0xFD, 0xFB, 0xF7, 0xEF, 0xDF, 0xBF, 0x7F];

/// Types `c` on the keyboard of a ZX81 running a program that reads all eight half rows over and over and keeps, for each,
/// the AND of every value it read (so a bit that ever went low stays low). Returns those eight bytes.
fn keys_seen_by_a_program(env: &Env, c: char) -> Result<Vec<u8>, String> {
    let mut source = String::new();
    for i in 0..8 {
        source += &format!(" ld a,$FF\n ld (${:04X}),a\n", 0x4F00 + i);
    }
    source += "scan: ld c,$FE\n";
    for (i, row) in ROW_SELECTS.iter().enumerate() {
        source += &format!(
            " ld b,${row:02X}\n in a,(c)\n ld hl,${:04X}\n and (hl)\n ld (hl),a\n",
            0x4F00 + i
        );
    }
    source += " jp scan\n";
    let file = z80_program("keys", 0x4100, &source)?;
    let out = env
        .run("zx81")
        .rom(&rom(env)?)
        .arg("--program")
        .arg(file.to_string_lossy())
        .arg("--org")
        .arg("0x4100")
        .frames(40)
        .arg("--type-after")
        .arg("4")
        .arg("--type-delay")
        .arg("160")
        .arg("--type")
        .arg(c.to_string())
        .dump(0x4F00, 8)
        .go()?;
    out.bytes(0x4F00, 8)
}

/// What the eight half rows should read after typing: bits 0-4 high except the keys held, bits 5-7 high.
fn expected_rows(held: &[(usize, u8)]) -> Vec<u8> {
    let mut rows = vec![0xFFu8; 8];
    for &(row, bit) in held {
        rows[row] &= !(1 << bit);
    }
    rows
}

fn every_unshifted_key_pulls_its_matrix_line_low(env: &Env) -> Result<(), String> {
    let rom_bytes = std::fs::read(rom(env)?).map_err(|e| e.to_string())?;
    let keys: Vec<char> = ('A'..='Z')
        .chain('0'..='9')
        .chain([' ', '.', '\n'])
        .collect();
    for c in keys {
        let position = rom_key_position(&rom_bytes, false, c)?;
        let got = keys_seen_by_a_program(env, c)?;
        expect_eq(
            &format!(
                "the half rows read after typing {c:?} (the ROM puts it at row {}, bit {})",
                position.0, position.1
            ),
            got,
            expected_rows(&[position]),
        )?;
    }
    Ok(())
}

fn every_shifted_symbol_pulls_shift_and_its_key_low(env: &Env) -> Result<(), String> {
    let rom_bytes = std::fs::read(rom(env)?).map_err(|e| e.to_string())?;
    for c in [
        ':', ';', '?', '/', '"', ')', '(', '$', '=', '+', '-', ',', '>', '<', '*',
    ] {
        let position = rom_key_position(&rom_bytes, true, c)?;
        let got = keys_seen_by_a_program(env, c)?;
        // The ROM's shifted table is read with Shift held (row 0, bit 0) as well as the key.
        expect_eq(
            &format!(
                "the half rows read after typing {c:?} (Shift and row {}, bit {})",
                position.0, position.1
            ),
            got,
            expected_rows(&[(0, 0), position]),
        )?;
    }
    Ok(())
}

/// Types `text` into a booted ZX81 and returns its picture, 256x192 without the border.
fn screen_after_typing(env: &Env, text: &str) -> Result<rc_suite::Outcome, String> {
    env.run("zx81")
        .rom(&rom(env)?)
        .arg("--no-border")
        .frames(500)
        .arg("--type-delay")
        .arg("160")
        .arg("--type")
        .arg(text)
        .screen()
        .go()
}

/// Whether the character cell at (`col`, `row`) shows the ROM's glyph for `code`.
fn cell_shows(
    out: &rc_suite::Outcome,
    rom: &[u8],
    col: usize,
    row: usize,
    code: u8,
) -> Result<bool, String> {
    let glyph = &rom[0x1E00 + usize::from(code) * 8..0x1E00 + usize::from(code) * 8 + 8];
    Ok(out.screen()?.cell(col * 8, row * 8).as_slice() == glyph)
}

fn typed_print_statements_run_in_the_rom(env: &Env) -> Result<(), String> {
    let rom_bytes = std::fs::read(rom(env)?).map_err(|e| e.to_string())?;
    // BASIC: P is the PRINT keyword. Each statement is typed on the keyboard (Shift and the `.` key included), entered, run by
    // the ROM, and the result printed at the top left; the ROM then reports `0/0` (0 = no error, line 0) bottom left.
    for (typed, result) in [
        ("P12\n", "12"),
        ("P12*34\n", "408"),
        ("P1.5+1\n", "2.5"),
        ("P7-2*3\n", "1"),
    ] {
        let out = screen_after_typing(env, typed)?;
        for (col, c) in result.chars().enumerate() {
            let code = zx81_code(c).ok_or("no code")?;
            if !cell_shows(&out, &rom_bytes, col, 0, code)? {
                return Err(format!(
                    "typing {typed:?}: the top-left of the screen does not show {result:?}"
                ));
            }
        }
        for (col, c) in "0/0".chars().enumerate() {
            let code = if c == '/' {
                0x18
            } else {
                zx81_code(c).ok_or("no code")?
            };
            if !cell_shows(&out, &rom_bytes, col, 23, code)? {
                return Err(format!(
                    "typing {typed:?}: the ROM's report 0/0 is not at the bottom left"
                ));
            }
        }
    }
    Ok(())
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
    ),
    Case::new(
        Area::Io,
        "every unshifted key pulls its matrix line low",
        "the ROM's key table (0x007E): the ROM scans the keys in that order",
        every_unshifted_key_pulls_its_matrix_line_low,
    ),
    Case::new(
        Area::Io,
        "every shifted symbol pulls Shift and its key low",
        "the ROM's shifted key table (0x00A5)",
        every_shifted_symbol_pulls_shift_and_its_key_low,
    ),
    Case::new(
        Area::Rom,
        "typed PRINT statements run in the ROM",
        "BASIC arithmetic; the ROM's own report 0/0 after a statement; the ROM's character set",
        typed_print_statements_run_in_the_rom,
    ),
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
    ),
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
    ),
    Case::new(
        Area::Memory,
        "data reads above 0xC000 see the RAM",
        "ZXDOCS memory mirroring",
        data_reads_above_0xc000_see_the_ram,
    ),
    Case::new(
        Area::Video,
        "opcodes with bit 6 clear run as NOPs above 0x8000",
        "ZXDOCS 8000h-FFFFh (video output, machine code cannot run there)",
        opcodes_with_bit_6_clear_run_as_nops_above_0x8000,
    ),
    Case::new(
        Area::Memory,
        "a 16K RAM pack is detected",
        "ZXDOCS RAMTOP",
        sixteen_k_ram_pack_is_detected,
    ),
    Case::new(
        Area::Memory,
        "RAMTOP follows the RAM size: 1K, 2K, 16K, 32K, 56K",
        "ZXDOCS Memory Mirrors and Expansions (4400h for 1K; detection ends at 8000h)",
        the_power_on_ram_test_sets_ramtop_for_every_ram_size,
    ),
    Case::new(
        Area::Memory,
        "the first and last cell of a 16K, 32K and 56K machine are separate RAM",
        "ZXDOCS Memory Mirrors and Expansions (RAM at 4000h up; 56K from 2000h)",
        the_last_cell_of_every_ram_size_is_ram,
    ),
    Case::new(
        Area::Media,
        "a .p file is loaded at 4009h and its program autostarts",
        "ZXDOCS Cassette File Content (memory from 4009h; NXTLIN = autostart); ROM character set",
        a_p_file_is_loaded_at_0x4009_and_its_program_autostarts,
    ),
    Case::new(
        Area::Media,
        "saving a .p writes memory from 4009h to E_LINE-1",
        "ZXDOCS Cassette File Images (a copy of memory 4009h..E_LINE-1)",
        saving_a_p_file_writes_memory_from_0x4009_to_e_line,
    ),
    Case::new(
        Area::Media,
        "a saved .p and a saved .81 are the same bytes, and loading either runs the program again",
        "ZXDOCS Cassette File Images (.81 is an exact copy; .p the same up to E_LINE-1)",
        a_saved_file_loads_back_and_runs,
    ),
    Case::new(
        Area::Rom,
        "every ZX81 ROM revision boots to the K prompt",
        "ZXDOCS system variables; each ROM's own character set at 1E00h",
        every_rom_revision_boots_to_the_k_prompt,
    ),
    Case::new(
        Area::Media,
        "LOAD \"\" reads a program from the tape input",
        "ZXDOCS Cassette (EAR input, bit 7 of port FEh); a file is memory from 4009h",
        load_reads_a_program_from_the_tape_input,
    )
    .gap("the zx81 plugin has no tape (`unsupported media kind ... Tape`); a .p played as a tape is the first step"),
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
