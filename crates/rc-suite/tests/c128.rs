//! Commodore 128 suite: what the real machine does, checked against the emulator.
//!
//! The expected values are hardware facts, each named in its case:
//!
//! * `MMU` = the 8722 memory management unit: oxyron.de's MMU reference
//!   (<https://www.oxyron.de/html/registers_mmu.html>: the configuration register `CR` at `$D500` / `$FF00`, the
//!   pre-configuration registers `PCRA-D` at `$D501-$D504` that a write to `$FF01-$FF04` loads into `CR`, the mode
//!   configuration register at `$D505`, the version register `VR` at `$D50B`) and the C128 memory expansion
//!   article (<https://www.commodore.ca/manuals/funet/cbm/documents/projects/memory/c128/1028/1028.html>: the
//!   system version register always reads `$20`).
//! * `PRG` = the Commodore 128 Programmer's Reference Guide (<https://www.manualslib.com/manual/862628/Commodore-128.html?page=599>:
//!   `$D030` bit 0 selects 2 MHz and the screen is blanked first by clearing bit 4 of `$D011`).
//! * `BANNER` = the text the BASIC 7.0 power-on screen shows: `COMMODORE BASIC V7.0 122365 BYTES FREE`.
//! * `PAL` = the 6569 timing the C128 keeps in 1 MHz mode: 312 lines of 63 cycles, so a frame is 19656 cycles.
//!
//! A C128 powers up on its Z80, so the programs run through `rc_suite::c128_8502_program`: a Z80 stub sets the map
//! and hands the machine to the 8502, which then runs the 6502 code with nothing initialised (no KERNAL boot).
//!
//! Needs `RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS` and `RC_ROMS` (see the crate docs); skips without.

use rc_suite::{
    Area, C128_CODE_OFFSET, Case, Env, Outcome, c128_8502_program, expect_eq, expect_in, find_text,
    run_cases,
};

/// Where the program sits: above the Z80's colour RAM window (0x1000-0x13FF, with I/O on).
const ORG: u16 = 0x1400;

/// Runs 6502 `source` on a C128 for `frames` frames, dumping `dumps`.
fn run(
    env: &Env,
    tag: &str,
    source: &str,
    frames: u32,
    dumps: &[(u16, u16)],
) -> Result<Outcome, String> {
    let file = c128_8502_program(tag, ORG, source)?;
    let mut run = env
        .c128()?
        .arg("--program")
        .arg(file.to_string_lossy())
        .arg("--org")
        .arg(format!("0x{ORG:04X}"))
        .frames(frames);
    for (start, len) in dumps {
        run = run.dump(*start, *len);
    }
    run.go()
}

fn boots_to_the_basic_7_banner(env: &Env) -> Result<(), String> {
    let out = env.c128()?.frames(200).dump(0x0400, 1000).go()?;
    let screen = out.bytes(0x0400, 1000)?;
    // BANNER: the 40-column screen is at 0x0400; after the KERNAL's power-on the screen shows the BASIC 7.0 banner
    // with the free-memory figure of a 128K machine, the copyright lines, and the READY prompt.
    for line in [
        "COMMODORE BASIC V7.0 122365 BYTES FREE",
        "(C)1986 COMMODORE ELECTRONICS, LTD.",
        "(C)1977 MICROSOFT CORP.",
        "ALL RIGHTS RESERVED",
        "READY.",
    ] {
        if find_text(&screen, line).is_none() {
            return Err(format!("the screen does not show {line:?}"));
        }
    }
    Ok(())
}

fn the_roms_show_and_a_write_goes_to_the_ram_under_them(env: &Env) -> Result<(), String> {
    let out = run(
        env,
        "rom-ram",
        "
 lda #$00
 sta $FF00
 ldx #15
cp: lda $4000,x
 sta $0F10,x
 dex
 bpl cp
 lda #$A5
 sta $4000
 lda $4000
 sta $0F20
 lda #$3E
 sta $FF00
 lda $4000
 sta $0F21
here: jmp here
",
        2,
        &[(0x0F10, 18)],
    )?;
    // MMU: CR = $00 puts the system ROMs in (BASIC low at $4000, I/O at $D000); a write always lands in the RAM
    // under a ROM; CR = $3E (bits 1-5 set) puts RAM there instead.
    let basic = std::fs::read(env.file("commodore/roms/c128/basiclo-318018-04.bin")?)
        .map_err(|e| e.to_string())?;
    let seen = out.bytes(0x0F10, 16)?;
    expect_eq(
        "the 16 bytes at 0x4000 with CR = $00",
        seen,
        basic[..16].to_vec(),
    )?;
    expect_eq(
        "a read at 0x4000 after a write, ROM still in",
        out.byte(0x0F20)?,
        basic[0],
    )?;
    expect_eq("the same address with CR = $3E", out.byte(0x0F21)?, 0xA5)
}

fn ram_banks_0_and_1_are_separate(env: &Env) -> Result<(), String> {
    // The code that switches banks has to be in RAM both banks share, or the next instruction would come from the other
    // bank. RCR = $04 shares the bottom 1K (the value the KERNAL sets at power-on), the routine is copied to $0300 and
    // written without absolute addresses of its own, so it runs wherever it sits.
    let out = run(
        env,
        "banks",
        "
 lda #$04
 sta $D506
 ldx #$30
cp: lda routine,x
 sta $0300,x
 dex
 bpl cp
 jmp $0300
routine:
 lda #$3E
 sta $FF00
 lda #$11
 sta $4000
 lda #$7E
 sta $FF00
 lda #$22
 sta $4000
 lda $4000
 tax
 lda #$3E
 sta $FF00
 lda $4000
 sta $0380
 stx $0381
 lda #$00
spin: beq spin
",
        2,
        &[(0x0380, 2)],
    )?;
    // MMU: bit 6 of CR selects RAM bank 1 (a 128K machine has two 64K banks), so the same address holds two values.
    expect_eq("bank 0 at 0x4000", out.byte(0x0380)?, 0x11)?;
    expect_eq("bank 1 at 0x4000", out.byte(0x0381)?, 0x22)
}

fn a_video_frame_is_312_lines_of_63_cycles(env: &Env) -> Result<(), String> {
    let out = run(
        env,
        "frame",
        "
 lda #$FF
 sta $DC04
 sta $DC05
 lda #$11
 sta $DC0E
w1: lda $D012
 cmp #100
 bne w1
rd1: lda $DC05
 ldx $DC04
 cmp $DC05
 bne rd1
 sta $0F40
 stx $0F41
w2: lda $D012
 cmp #100
 beq w2
w3: lda $D012
 cmp #100
 bne w3
rd2: lda $DC05
 ldx $DC04
 cmp $DC05
 bne rd2
 sta $0F42
 stx $0F43
here: jmp here
",
        4,
        &[(0x0F40, 4)],
    )?;
    // PAL: CIA 1 timer A counts system clock cycles and runs down from $FFFF; the time between two passes of the same
    // raster line is one frame, 312 lines * 63 cycles = 19656 cycles. The raster is polled every 9 cycles, so each edge is
    // found to within 9.
    let first = u32::from(out.byte(0x0F40)?) << 8 | u32::from(out.byte(0x0F41)?);
    let second = u32::from(out.byte(0x0F42)?) << 8 | u32::from(out.byte(0x0F43)?);
    let elapsed = i64::from(first.wrapping_sub(second) & 0xFFFF);
    expect_in(
        "cycles between two passes of raster line 100",
        elapsed,
        19_656 - 30,
        19_656 + 30,
    )
}

fn the_two_megahertz_mode_runs_a_ram_loop_twice_as_fast(env: &Env) -> Result<(), String> {
    // `run` counts batches of 64 DEY/BNE passes (338 cycles with its bookkeeping) between two passes of raster line 100,
    // that is, over exactly one frame; it is called once at 1 MHz and once with $D030 bit 0 set, the screen blanked.
    let out = run(
        env,
        "fast",
        "
 lda $D011
 and #$EF
 sta $D011
 lda #$00
 sta $D030
 jsr count
 lda $FB
 sta $0F50
 lda $FC
 sta $0F51
 lda #$01
 sta $D030
 jsr count
 lda $FB
 sta $0F52
 lda $FC
 sta $0F53
here: jmp here
count: lda #$00
 sta $FB
 sta $FC
s1: lda $D012
 cmp #100
 bne s1
s2: lda $D012
 cmp #100
 beq s2
l1: ldy #$40
l2: dey
 bne l2
 inc $FB
 bne l3
 inc $FC
l3: lda $D012
 cmp #100
 bne l1
 rts
",
        8,
        &[(0x0F50, 4)],
    )?;
    let slow = i64::from(out.word(0x0F50)?);
    let fast = i64::from(out.word(0x0F52)?);
    // PRG: $D030 bit 0 = 1 selects 2 MHz (the screen must be blanked, bit 4 of $D011 clear). A frame is 19656 us, so the loop
    // gets 19656 / 338 = 58 batches at 1 MHz and twice that at 2 MHz.
    expect_in("batches in a frame at 1 MHz", slow, 56, 60)?;
    expect_in("batches in a frame at 2 MHz", fast, 112, 120)
}

/// The screen of a C128 (40 columns, at 0x0400) as screen codes after `frames` frames, with `typed` typed from frame `after`.
fn screen_after_typing(env: &Env, frames: u32, after: u32, typed: &str) -> Result<Vec<u8>, String> {
    let out = env
        .c128()?
        .frames(frames)
        .arg("--type-after")
        .arg(after.to_string())
        .arg("--type")
        .arg(typed)
        .dump(0x0400, 1000)
        .go()?;
    out.bytes(0x0400, 1000)
}

fn basic_evaluates_a_typed_expression(env: &Env) -> Result<(), String> {
    let screen = screen_after_typing(env, 400, 150, "PRINT 12*34\n")?;
    // The keyboard matrix, the KERNAL's screen editor and the BASIC interpreter all take part: the typed line is echoed,
    // BASIC runs it, and 12 * 34 = 408 is printed under it (numbers print with a leading space for the sign).
    let echo = find_text(&screen, "PRINT 12*34").ok_or("the typed line is not on the screen")?;
    let answer =
        find_text(&screen[echo..], " 408").ok_or("BASIC did not print 408 after the typed line")?;
    expect_in(
        "rows between the typed line and its answer",
        (answer / 40) as i64,
        0,
        2,
    )
}

fn the_c64_mode_boots_to_the_c64_banner(env: &Env) -> Result<(), String> {
    let out = env
        .c128()?
        .arg("--mode")
        .arg("c64")
        .frames(250)
        .dump(0x0400, 1000)
        .go()?;
    let screen = out.bytes(0x0400, 1000)?;
    // The C128's C64 mode is a C64: its power-on screen is the C64's banner with the C64's free-memory figure.
    for line in [
        "**** COMMODORE 64 BASIC V2 ****",
        "64K RAM SYSTEM  38911 BASIC BYTES FREE",
        "READY.",
    ] {
        if find_text(&screen, line).is_none() {
            return Err(format!("the screen does not show {line:?}"));
        }
    }
    Ok(())
}

/// A disk drive the C128 can have and the disk format it reads.
struct Drive {
    kind: &'static str,
    rom: &'static str,
    format: &'static str,
}

const DRIVES: [Drive; 3] = [
    Drive {
        kind: "1541",
        rom: "commodore/roms/1541/dos1541-325302-01+901229-05.bin",
        format: "d64",
    },
    Drive {
        kind: "1571",
        rom: "commodore/roms/c64/VC1571",
        format: "d71",
    },
    Drive {
        kind: "1581",
        rom: "commodore/roms/c64/VC1581",
        format: "d81",
    },
];

/// Puts a program file on a disk `rcdisk` makes, and has the C128's BASIC load it from a real emulated drive.
fn a_file_rcdisk_added_loads_through_the_kernal(env: &Env, drive: &Drive) -> Result<(), String> {
    // 700 bytes at 0x2000: three blocks, so the drive follows a sector chain; the pattern shows a block read out of place.
    let data: Vec<u8> = (0..700u32).map(|i| (i * 7 + i / 251) as u8).collect();
    let mut program = vec![0x00, 0x20];
    program.extend_from_slice(&data);
    let mut disk = rcdisk::create(drive.format, Some("RC TEST"), Some("01"))?;
    disk.add("payload.prg", &program, &rcdisk::AddOptions::default())?;
    let path = std::env::temp_dir().join(format!(
        "rc-suite-c128-{}-{}.{}",
        drive.kind,
        std::process::id(),
        drive.format
    ));
    std::fs::write(&path, disk.to_bytes()).map_err(|e| e.to_string())?;

    // A C128 with a drive probes it for a boot sector while it starts, and keys typed meanwhile are lost, so typing starts
    // later than without one.
    let out = env
        .c128()?
        .arg("--true-drive")
        .arg("--drive-type")
        .arg(drive.kind)
        .arg("--rom-1541")
        .arg(env.file(drive.rom)?.to_string_lossy())
        .arg("--disk")
        .arg(path.to_string_lossy())
        .frames(1300)
        .arg("--type-after")
        .arg("500")
        .arg("--type")
        .arg("LOAD\"PAYLOAD\",8,1\n")
        .dump(0x2000, 700)
        .go()?;
    let _ = std::fs::remove_file(&path);
    // `LOAD"name",8,1` loads the file at the address in its first two bytes: what the disk holds comes back unchanged.
    expect_eq(
        &format!(
            "the file loaded from a {} drive ({} disk)",
            drive.kind, drive.format
        ),
        out.bytes(0x2000, 700)?,
        data,
    )
}

fn a_1541_drive_loads_a_file_rcdisk_added(env: &Env) -> Result<(), String> {
    a_file_rcdisk_added_loads_through_the_kernal(env, &DRIVES[0])
}

fn a_1571_drive_loads_a_file_rcdisk_added(env: &Env) -> Result<(), String> {
    a_file_rcdisk_added_loads_through_the_kernal(env, &DRIVES[1])
}

fn a_1581_drive_loads_a_file_rcdisk_added(env: &Env) -> Result<(), String> {
    a_file_rcdisk_added_loads_through_the_kernal(env, &DRIVES[2])
}

const CASES: &[Case] = &[
    Case::new(
        Area::Video,
        "boots to the BASIC 7.0 banner",
        "BANNER: the BASIC 7.0 power-on screen",
        boots_to_the_basic_7_banner,
    ),
    Case::new(
        Area::Io,
        "BASIC evaluates a typed expression",
        "BASIC: 12*34 = 408, through the keyboard, the screen editor and the interpreter",
        basic_evaluates_a_typed_expression,
    ),
    Case::new(
        Area::Video,
        "C64 mode boots to the C64 banner",
        "the C64 power-on screen: 38911 BASIC BYTES FREE",
        the_c64_mode_boots_to_the_c64_banner,
    ),
    Case::new(
        Area::Media,
        "a 1541 drive loads a file rcdisk added to a .d64",
        "the file on the disk is the file BASIC loads (LOAD\"name\",8,1)",
        a_1541_drive_loads_a_file_rcdisk_added,
    ),
    Case::new(
        Area::Media,
        "a 1571 drive loads a file rcdisk added to a .d71",
        "the file on the disk is the file BASIC loads (LOAD\"name\",8,1)",
        a_1571_drive_loads_a_file_rcdisk_added,
    ),
    Case::new(
        Area::Media,
        "a 1581 drive loads a file rcdisk added to a .d81",
        "the file on the disk is the file BASIC loads (LOAD\"name\",8,1)",
        a_1581_drive_loads_a_file_rcdisk_added,
    ),
    Case::new(
        Area::Memory,
        "the ROMs show and a write goes to the RAM under them",
        "MMU: configuration register $FF00 bits 1-5",
        the_roms_show_and_a_write_goes_to_the_ram_under_them,
    ),
    Case::new(
        Area::Memory,
        "RAM banks 0 and 1 are separate",
        "MMU: CR bit 6, two 64K banks",
        ram_banks_0_and_1_are_separate,
    ),
    Case::new(
        Area::Timers,
        "a video frame is 312 lines of 63 cycles",
        "PAL: 6569 timing, 19656 cycles a frame",
        a_video_frame_is_312_lines_of_63_cycles,
    ),
    Case::new(
        Area::Timers,
        "2 MHz mode runs a RAM loop twice as fast",
        "PRG: $D030 bit 0",
        the_two_megahertz_mode_runs_a_ram_loop_twice_as_fast,
    ),
];

#[test]
fn c128() {
    let Some(env) = Env::discover() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    if env.c128().is_err() {
        eprintln!("skipping: the C128 ROMs are not present");
        return;
    }
    let report = run_cases(&env, CASES);
    eprintln!("{}", report.summary());
    assert!(report.ok(), "{}", report.summary());
}

#[allow(dead_code)]
const _: u16 = C128_CODE_OFFSET;
