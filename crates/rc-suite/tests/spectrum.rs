//! ZX Spectrum suite: what the real machine does, checked against the emulator.
//!
//! The expected values are hardware facts, each named in its case:
//!
//! * `WOS48` = the World of Spectrum 48K reference, <https://worldofspectrum.org/faq/reference/48kreference.htm>
//!   (the keyboard half rows on port 0xFE, the border bits, 312 lines of 224 T-states = 69888 per frame, the
//!   contended range 0x4000-0x7FFF, ROM 16K and RAM from 0x4000).
//! * `WOS128` = the same site's 128K reference, <https://worldofspectrum.org/faq/reference/128kreference.htm> (port
//!   0x7FFD: bits 0-2 the RAM page at 0xC000, bit 4 the ROM, bit 5 the lock; 0x4000 is bank 5 and 0x8000 bank 2;
//!   311 lines of 228 T-states = 70908 per frame).
//! * `ROM` = the 16K ROM itself (the character set at 0x3D00, which the copyright message is drawn from; the
//!   system variables the power-on routine sets).
//! * `Z80` = Zilog's instruction timing tables.
//!
//! Needs `RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS` and `RC_ROMS` (see the crate docs); skips without.

use rc_suite::{Area, Case, Env, Outcome, expect_eq, expect_in, run_cases, z80_program};

const ROM_48: &str = "sinclair/roms/zx-spectrum/spec48.rom";
const ROM_128: &str = "sinclair/roms/zx-spectrum/128.rom";

// System variables (the 48K manual, chapter 25).
const FRAMES: u16 = 0x5C78;
const P_RAMT: u16 = 0x5CB4;

/// Frames from reset to the copyright message and the stable BASIC loop, with room to spare.
const BOOT_FRAMES: u32 = 250;

fn machine_name(rom: &str) -> &'static str {
    if rom == ROM_128 {
        "zx-spectrum-128"
    } else {
        "zx-spectrum"
    }
}

/// Runs `source` loaded at `org` for `frames` frames. The ROM is not booted: the CPU starts in the program with
/// interrupts off, so the ROM's interrupt routine does not run.
fn run_program(
    env: &Env,
    rom_file: &str,
    tag: &str,
    org: u16,
    source: &str,
    frames: u32,
    dumps: &[(u16, u16)],
) -> Result<Outcome, String> {
    let file = z80_program(tag, org, source)?;
    let mut run = env
        .run(machine_name(rom_file))
        .rom(&env.file(rom_file)?)
        .arg("--no-border")
        .arg("--program")
        .arg(file.to_string_lossy())
        .arg("--org")
        .arg(format!("0x{org:04X}"))
        .frames(frames);
    for (start, len) in dumps {
        run = run.dump(*start, *len);
    }
    run.go()
}

fn boots_to_the_copyright_message(env: &Env) -> Result<(), String> {
    let rom = env.file(ROM_48)?;
    let out = env
        .run("zx-spectrum")
        .rom(&rom)
        .arg("--no-border")
        .frames(BOOT_FRAMES)
        .screen()
        .go()?;
    // ROM: the power-on routine prints "(c) 1982 Sinclair Research Ltd" on the bottom line, black ink on white
    // paper, in the ROM's own character set (8 bytes a character from 0x3D00 for code 32; the (c) is code 127).
    let rom_bytes = std::fs::read(&rom).map_err(|e| e.to_string())?;
    let glyph = |code: u8| {
        let at = 0x3D00 + (usize::from(code) - 32) * 8;
        rom_bytes[at..at + 8].to_vec()
    };
    let message: Vec<u8> = std::iter::once(0x7F)
        .chain(" 1982 Sinclair Research Ltd".bytes())
        .collect();
    let screen = out.screen()?;
    expect_eq("picture size", (screen.width, screen.height), (256, 192))?;
    for (column, &code) in message.iter().enumerate() {
        expect_eq(
            &format!("character {column} of the message"),
            screen.cell(column * 8, 23 * 8).to_vec(),
            glyph(code),
        )?;
    }
    Ok(())
}

fn the_power_on_routine_finds_48k_of_ram(env: &Env) -> Result<(), String> {
    let out = env
        .run("zx-spectrum")
        .rom(&env.file(ROM_48)?)
        .frames(BOOT_FRAMES)
        .dump(P_RAMT, 2)
        .go()?;
    // ROM / 48K manual ch. 25: P_RAMT is the last byte of physical RAM, which the power-on test finds by writing
    // upward from 0x4000: 0xFFFF on a 48K machine.
    expect_eq("P_RAMT", out.word(P_RAMT)?, 0xFFFF)
}

fn frames_counts_at_the_50_hz_interrupt(env: &Env) -> Result<(), String> {
    let at = |frames| -> Result<u32, String> {
        let out = env
            .run("zx-spectrum")
            .rom(&env.file(ROM_48)?)
            .frames(frames)
            .dump(FRAMES, 3)
            .go()?;
        let b = out.bytes(FRAMES, 3)?;
        Ok(u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16)
    };
    let early = at(BOOT_FRAMES)?;
    let later = at(BOOT_FRAMES + 100)?;
    // WOS48: the ULA interrupts once a frame; the ROM's interrupt routine adds one to FRAMES each time.
    expect_in("FRAMES over 100 frames", i64::from(later - early), 99, 101)
}

fn frame_length(env: &Env, rom: &str, per_frame: i64) -> Result<(), String> {
    const FRAMES_RUN: u32 = 10;
    let out = run_program(
        env,
        rom,
        "frame",
        0x8000,
        "here: jr here
",
        FRAMES_RUN,
        &[],
    )?;
    // A frame ends at the first instruction boundary past its length, so each may run over by less than one
    // instruction (JR is 12 T).
    expect_in(
        &format!("cycles in {FRAMES_RUN} frames"),
        out.cycles as i64,
        i64::from(FRAMES_RUN) * per_frame,
        i64::from(FRAMES_RUN) * (per_frame + 12),
    )
}

/// WOS48: 312 lines of 224 T-states.
fn a_48k_frame_is_69888_t_states(env: &Env) -> Result<(), String> {
    frame_length(env, ROM_48, 69_888)
}

/// WOS128: 311 lines of 228 T-states.
fn a_128k_frame_is_70908_t_states(env: &Env) -> Result<(), String> {
    frame_length(env, ROM_128, 70_908)
}

fn the_rom_is_read_only_and_ram_starts_at_0x4000(env: &Env) -> Result<(), String> {
    let out = run_program(
        env,
        ROM_48,
        "rom-ram",
        0x8000,
        "ld a,($0000)\n ld b,a\n ld a,$AA\n ld ($0000),a\n ld a,($0000)\n ld c,a\n ld a,$AA\n ld ($4000),a\n ld a,($4000)\n ld d,a\nhere: jr here\n",
        2,
        &[],
    )?;
    let rom = std::fs::read(env.file(ROM_48)?).map_err(|e| e.to_string())?;
    // WOS48: a 16K ROM at the bottom of the address space, RAM from 0x4000. A write to the ROM changes nothing.
    expect_eq(
        "ROM byte before the write",
        out.reg("BC")? >> 8,
        u16::from(rom[0]),
    )?;
    expect_eq(
        "ROM byte after the write",
        out.reg("BC")? & 0xFF,
        u16::from(rom[0]),
    )?;
    expect_eq(
        "RAM at 0x4000 keeps what is written",
        out.reg("DE")? >> 8,
        0xAA,
    )
}

/// The eight half rows of the keyboard (WOS48): the high byte of the port address that selects each, and the keys on
/// bits 0-4. Shift and Symbol Shift cannot be typed alone, so they are left out (`'\0'`).
const HALF_ROWS: [(u8, [char; 5]); 8] = [
    (0xFE, ['\0', 'z', 'x', 'c', 'v']),
    (0xFD, ['a', 's', 'd', 'f', 'g']),
    (0xFB, ['q', 'w', 'e', 'r', 't']),
    (0xF7, ['1', '2', '3', '4', '5']),
    (0xEF, ['0', '9', '8', '7', '6']),
    (0xDF, ['p', 'o', 'i', 'u', 'y']),
    (0xBF, ['\n', 'l', 'k', 'j', 'h']),
    (0x7F, [' ', '\0', 'm', 'n', 'b']),
];

/// Types `c` on a Spectrum running a program that reads all eight half rows over and over and keeps, for each, the AND
/// of everything it read (a bit that ever went low stays low). Returns those eight bytes.
fn keys_seen_by_a_program(env: &Env, c: char) -> Result<Vec<u8>, String> {
    let mut source = String::new();
    for i in 0..8 {
        source += &format!(" ld a,$FF\n ld ($9000+{i}),a\n");
    }
    source += "scan: ld c,$FE\n";
    for (i, (row, _)) in HALF_ROWS.iter().enumerate() {
        source +=
            &format!(" ld b,${row:02X}\n in a,(c)\n ld hl,$9000+{i}\n and (hl)\n ld (hl),a\n");
    }
    source += " jp scan\n";
    let file = z80_program("keys", 0x8100, &source)?;
    let out = env
        .run("zx-spectrum")
        .rom(&env.file(ROM_48)?)
        .arg("--program")
        .arg(file.to_string_lossy())
        .arg("--org")
        .arg("0x8100")
        .arg("--type-after")
        .arg("4")
        .arg("--type-delay")
        .arg("160")
        .arg("--type")
        .arg(c.to_string())
        .frames(40)
        .dump(0x9000, 8)
        .go()?;
    out.bytes(0x9000, 8)
}

fn every_key_pulls_its_half_row_bit_low(env: &Env) -> Result<(), String> {
    for (row, (_, keys)) in HALF_ROWS.iter().enumerate() {
        for (bit, &key) in keys.iter().enumerate() {
            if key == '\0' {
                continue;
            }
            let rows = keys_seen_by_a_program(env, key)?;
            // WOS48: a pressed key reads 0 on its bit of its half row; bits 5 and 7 always read 1 (bit 6 is EAR).
            for (r, &value) in rows.iter().enumerate() {
                let want_low = if r == row { 1u8 << bit } else { 0 };
                expect_eq(
                    &format!(
                        "half row {r} (port {:02X}FE) after typing {key:?}",
                        HALF_ROWS[r].0
                    ),
                    value & 0x1F,
                    0x1F & !want_low,
                )?;
            }
        }
    }
    Ok(())
}

fn the_border_follows_bits_0_to_2_of_port_0xfe(env: &Env) -> Result<(), String> {
    let border_pixel = |colour: u8| -> Result<[u8; 3], String> {
        let file = z80_program(
            "border",
            0x8000,
            &format!(" ld a,{colour}\n out ($FE),a\nhere: jr here\n"),
        )?;
        let out = env
            .run("zx-spectrum")
            .rom(&env.file(ROM_48)?)
            .arg("--program")
            .arg(file.to_string_lossy())
            .arg("--org")
            .arg("0x8000")
            .frames(3)
            .screen()
            .go()?;
        Ok(out.screen()?.pixel(0, 0))
    };
    // WOS48: bits 0-2 of a write to port 0xFE set the border colour: 0 black, 1 blue, 2 red, 7 white.
    let (black, blue, red, white) = (
        border_pixel(0)?,
        border_pixel(1)?,
        border_pixel(2)?,
        border_pixel(7)?,
    );
    expect_eq("black", black, [0, 0, 0])?;
    if !(blue[2] > 0 && blue[0] == 0 && blue[1] == 0) {
        return Err(format!("colour 1 should be pure blue, got {blue:?}"));
    }
    if !(red[0] > 0 && red[1] == 0 && red[2] == 0) {
        return Err(format!("colour 2 should be pure red, got {red:?}"));
    }
    if !(white[0] > 0xC0 && white[0] == white[1] && white[1] == white[2]) {
        return Err(format!("colour 7 should be a grey-white, got {white:?}"));
    }
    Ok(())
}

/// The address of the display-file byte holding pixel line `y`, byte column `col`: the ULA reads the picture in the
/// interleaved order Y7 Y6 | Y2 Y1 Y0 | Y5 Y4 Y3 (the screen's three thirds, then the line within a character row, then the
/// character row), which every Spectrum reference documents.
fn display_address(y: u16, col: u16) -> u16 {
    0x4000 | ((y & 0xC0) << 5) | ((y & 0x07) << 8) | ((y & 0x38) << 2) | col
}

fn the_display_file_is_interleaved(env: &Env) -> Result<(), String> {
    let lines = [0u16, 1, 7, 8, 63, 64, 128, 191];
    let mut source =
        String::from(" ld hl,$5800\n ld de,$5801\n ld bc,767\n ld (hl),$38\n ldir\n ld a,$FF\n");
    for &y in &lines {
        source += &format!(" ld (${:04X}),a\n", display_address(y, 0));
    }
    source += "here: jr here\n";
    let file = z80_program("layout", 0x8000, &source)?;
    let out = env
        .run("zx-spectrum")
        .rom(&env.file(ROM_48)?)
        .arg("--no-border")
        .arg("--program")
        .arg(file.to_string_lossy())
        .arg("--org")
        .arg("0x8000")
        .frames(3)
        .screen()
        .go()?;
    let screen = out.screen()?;
    // Black ink on white paper (attribute 0x38): exactly the eight lines written have ink, in their first 8 pixels.
    for y in 0..192usize {
        let inked = lines.contains(&(y as u16));
        for x in 0..8 {
            expect_eq(&format!("pixel ({x},{y})"), screen.is_ink(x, y), inked)?;
        }
    }
    expect_eq("ink pixels in all", screen.ink_count(), lines.len() * 8)
}

fn contended_ram_slows_a_loop_down(env: &Env) -> Result<(), String> {
    const FRAMES_RUN: u32 = 10;
    let run = |org: u16| -> Result<(i64, i64), String> {
        let out = run_program(
            env,
            ROM_48,
            "contention",
            org,
            "ld hl,0
lp: inc hl
 jr lp
",
            FRAMES_RUN,
            &[],
        )?;
        Ok((i64::from(out.reg("HL")?), out.cycles as i64))
    };
    let ((uncontended, cycles), (contended, _)) = (run(0x8000)?, run(0x6000)?);
    // Z80: LD HL,nn = 10 T, then INC HL = 6 T and a taken JR = 12 T: 18 T an iteration. Uncontended RAM never delays
    // the CPU, so the count follows from the cycles run.
    let want = (cycles - 10) / 18;
    expect_in("iterations in uncontended RAM", uncontended, want, want + 1)?;
    // WOS48: 0x4000-0x7FFF is contended: while the picture is drawn the ULA holds the CPU for up to 6 T-states in
    // 8, so the same loop in that RAM runs measurably slower (it still gets the whole border and retrace time).
    expect_in(
        "iterations in contended RAM",
        contended,
        uncontended * 55 / 100,
        uncontended * 95 / 100,
    )
}

// ---- 128K (WOS128)

fn the_eight_ram_pages_are_separate(env: &Env) -> Result<(), String> {
    let mut source = String::from(" ld bc,$7FFD\n");
    for page in 0..8u8 {
        source += &format!(
            " ld a,{page}\n out (c),a\n ld a,{}\n ld ($C000),a\n",
            0x10 + page
        );
    }
    for page in 0..8u8 {
        source += &format!(" ld a,{page}\n out (c),a\n ld a,($C000)\n ld ($9000+{page}),a\n");
    }
    source += "here: jr here\n";
    let out = run_program(env, ROM_128, "pages", 0x6000, &source, 3, &[(0x9000, 8)])?;
    // WOS128: bits 0-2 of port 0x7FFD choose which of the eight 16K pages is at 0xC000; each keeps its own byte.
    expect_eq(
        "the byte each page kept",
        out.bytes(0x9000, 8)?,
        (0..8u8).map(|p| 0x10 + p).collect::<Vec<_>>(),
    )
}

fn pages_5_and_2_are_the_fixed_banks_at_0x4000_and_0x8000(env: &Env) -> Result<(), String> {
    let out = run_program(
        env,
        ROM_128,
        "fixed",
        0x6000,
        " ld bc,$7FFD\n ld a,5\n out (c),a\n ld a,$C5\n ld ($C000),a\n ld a,2\n out (c),a\n ld a,$C2\n ld ($C000),a\n ld a,($4000)\n ld ($9100),a\n ld a,($8000)\n ld ($9101),a\nhere: jr here\n",
        3,
        &[(0x9100, 2)],
    )?;
    // WOS128: 0x4000-0x7FFF is always page 5 and 0x8000-0xBFFF is always page 2, so what is written through the
    // paging window with those pages selected shows there.
    expect_eq(
        "0x4000 after writing page 5 at 0xC000",
        out.byte(0x9100)?,
        0xC5,
    )?;
    expect_eq(
        "0x8000 after writing page 2 at 0xC000",
        out.byte(0x9101)?,
        0xC2,
    )
}

fn bit_4_of_port_0x7ffd_selects_the_rom(env: &Env) -> Result<(), String> {
    let out = run_program(
        env,
        ROM_128,
        "romsel",
        0x8100,
        " ld bc,$7FFD\n xor a\n out (c),a\n ld hl,$0100\n ld de,$9000\n ld bc,16\n ldir\n ld bc,$7FFD\n ld a,$10\n out (c),a\n ld hl,$0100\n ld de,$9010\n ld bc,16\n ldir\nhere: jr here\n",
        3,
        &[(0x9000, 32)],
    )?;
    let rom = std::fs::read(env.file(ROM_128)?).map_err(|e| e.to_string())?;
    // WOS128: bit 4 chooses the ROM: 0 the 128K editor ROM, 1 the 48K BASIC ROM (the file holds them in that order).
    expect_eq(
        "ROM 0",
        out.bytes(0x9000, 16)?,
        rom[0x0100..0x0110].to_vec(),
    )?;
    expect_eq(
        "ROM 1",
        out.bytes(0x9010, 16)?,
        rom[0x4100..0x4110].to_vec(),
    )
}

fn bit_5_of_port_0x7ffd_locks_the_paging(env: &Env) -> Result<(), String> {
    let out = run_program(
        env,
        ROM_128,
        "lock",
        0x6000,
        " ld bc,$7FFD\n ld a,$21\n out (c),a\n ld a,$77\n ld ($C000),a\n ld a,3\n out (c),a\n ld a,($C000)\n ld ($9200),a\nhere: jr here\n",
        3,
        &[(0x9200, 1)],
    )?;
    // WOS128: once bit 5 is set, further writes to the port are ignored until reset, so page 1 stays mapped (and still
    // holds the 0x77 written to it) after the attempt to select page 3.
    expect_eq(
        "what page 1 holds after the locked write",
        out.byte(0x9200)?,
        0x77,
    )
}

const CASES: &[Case] = &[
    Case::new(
        Area::Rom,
        "the 48K ROM boots to the copyright message",
        "ROM power-on routine; the ROM's character set at 3D00h",
        boots_to_the_copyright_message,
    ),
    Case::new(
        Area::Memory,
        "the power-on routine finds 48K of RAM",
        "48K manual ch. 25 (ERR_NR, P_RAMT)",
        the_power_on_routine_finds_48k_of_ram,
    ),
    Case::new(
        Area::Timers,
        "FRAMES counts one a frame",
        "WOS48 (the ULA interrupts once a frame)",
        frames_counts_at_the_50_hz_interrupt,
    ),
    Case::new(
        Area::Timers,
        "a 48K frame is 69888 T-states",
        "WOS48 (312 lines x 224 T-states)",
        a_48k_frame_is_69888_t_states,
    )
    .gap("every Spectrum model runs clock_hz 3546900 / frame_hz 50 = 70938 T a frame; the 48K is 3.5 MHz, 69888 a frame"),
    Case::new(
        Area::Timers,
        "a 128K frame is 70908 T-states",
        "WOS128 (311 lines x 228 T-states)",
        a_128k_frame_is_70908_t_states,
    )
    .gap("the 128K shares the 70938 T frame (30 T too long): clock/frame_hz cannot express 311 x 228"),
    Case::new(
        Area::Memory,
        "the ROM is read-only and RAM starts at 0x4000",
        "WOS48 memory map",
        the_rom_is_read_only_and_ram_starts_at_0x4000,
    ),
    Case::new(
        Area::Io,
        "every key pulls its bit of its half row low",
        "WOS48 port 0xFE half-row table",
        every_key_pulls_its_half_row_bit_low,
    ),
    Case::new(
        Area::Video,
        "the border follows bits 0-2 of port 0xFE",
        "WOS48 port 0xFE writes",
        the_border_follows_bits_0_to_2_of_port_0xfe,
    ),
    Case::new(
        Area::Video,
        "the display file is stored in the interleaved order",
        "the ULA's display file layout (Y7Y6 Y2Y1Y0 Y5Y4Y3)",
        the_display_file_is_interleaved,
    ),
    Case::new(
        Area::Timers,
        "RAM at 0x4000-0x7FFF is contended: a loop there is slower",
        "WOS48 contention; Z80 timing (INC HL 6 T, JR 12 T)",
        contended_ram_slows_a_loop_down,
    ),
    Case::new(
        Area::Memory,
        "the 128K has eight separate RAM pages at 0xC000",
        "WOS128 port 0x7FFD bits 0-2",
        the_eight_ram_pages_are_separate,
    ),
    Case::new(
        Area::Memory,
        "pages 5 and 2 are fixed at 0x4000 and 0x8000",
        "WOS128 memory map",
        pages_5_and_2_are_the_fixed_banks_at_0x4000_and_0x8000,
    ),
    Case::new(
        Area::Rom,
        "bit 4 of port 0x7FFD selects the ROM",
        "WOS128 port 0x7FFD bit 4",
        bit_4_of_port_0x7ffd_selects_the_rom,
    ),
    Case::new(
        Area::Memory,
        "bit 5 of port 0x7FFD locks the paging",
        "WOS128 port 0x7FFD bit 5",
        bit_5_of_port_0x7ffd_locks_the_paging,
    ),
];

#[test]
fn spectrum() {
    let Some(env) = Env::discover() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    if env.file(ROM_48).is_err() || env.file(ROM_128).is_err() {
        eprintln!("skipping: the Spectrum ROMs are not present");
        return;
    }
    let report = run_cases(&env, CASES);
    eprintln!("{}", report.summary());
    assert!(report.ok(), "{}", report.summary());
}
