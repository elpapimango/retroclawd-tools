//! Commodore 64 suite: what the real machine does, checked against the emulator.
//!
//! The expected values are hardware facts, each named in its case:
//!
//! * `C64WIKI` = c64-wiki.com: "Bank Switching" (the table of what LORAM, HIRAM and CHAREN in `$01` show at `$A000`,
//!   `$D000` and `$E000`), "Zeropage" (`$00` the data direction register, `$A0-$A2` the software jiffy clock the KERNAL
//!   IRQ updates every 1/60 second), "Memory Map" (`$01` = `$37` in the standard configuration, screen RAM at
//!   `$0400`, 38911 BASIC bytes).
//! * `VIC` = the 6569 (PAL) timing, Christian Bauer's "The MOS 6567/6569 video controller (VIC-II) and its application
//!   in the Commodore 64": 312 lines of 63 cycles, so a frame is 19656 cycles at 985248 Hz.
//! * `SID` = the 6581 datasheet (Commodore 64 Programmer's Reference Guide, appendix O): register `$1B` (`$D41B`)
//!   reads the upper 8 bits of voice 3's waveform generator.
//! * `CIA` = the MOS 6526 datasheet: a timer counts down once per system clock (phi2) cycle.
//! * `BANNER` = the power-on screen: `**** COMMODORE 64 BASIC V2 ****` and `64K RAM SYSTEM  38911 BASIC BYTES FREE`.
//!
//! Programs run through `rc_suite::mos6502_program` and `--program`: the CPU starts at the program with nothing
//! initialised (no KERNAL boot), so the programs set up what they use.
//!
//! Needs `RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS` and `RC_ROMS` (see the crate docs); skips without.

use rc_suite::{
    Area, Case, Env, Outcome, expect_eq, expect_in, find_text, mos6502_program, run_cases,
};

const ORG: u16 = 0xC000;
/// Where programs leave their results (always RAM).
const RESULTS: u16 = 0x0340;

fn run_program(
    env: &Env,
    tag: &str,
    source: &str,
    frames: u32,
    dumps: &[(u16, u16)],
) -> Result<Outcome, String> {
    let file = mos6502_program(tag, ORG, source)?;
    let mut run = env
        .c64()?
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

fn boots_to_the_basic_v2_banner(env: &Env) -> Result<(), String> {
    let out = env.c64()?.frames(200).dump(0x0400, 1000).go()?;
    let screen = out.bytes(0x0400, 1000)?;
    // BANNER (screen RAM at $0400, C64WIKI memory map: 38911 BASIC bytes).
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

fn the_cpu_port_is_set_up_for_the_standard_map(env: &Env) -> Result<(), String> {
    // Read through the CPU (BASIC's PEEK), not the debugger's view of the port. C64WIKI Memory Map: $01 holds $37 = 55 in
    // the standard configuration (BASIC, KERNAL and I/O visible); Zeropage: $00 is the data direction register, bits 0-3
    // and 5 outputs and bit 4 (the cassette sense) an input = $2F = 47.
    let out = env
        .c64()?
        .arg("--type-after")
        .arg("150")
        .arg("--type-delay")
        .arg("100")
        .arg("--type")
        .arg(
            "PRINT PEEK(0);PEEK(1)
",
        )
        .frames(450)
        .dump(0x0400, 1000)
        .go()?;
    let screen = out.bytes(0x0400, 1000)?;
    // BASIC prints a positive number with a leading space (the sign position) and `;` joins them without a gap.
    if find_text(&screen, " 47  55").is_none() {
        return Err("PRINT PEEK(0);PEEK(1) should show 47 and 55".into());
    }
    Ok(())
}

/// What each of the eight settings of LORAM/HIRAM/CHAREN shows, from the C64WIKI bank switching table (no cartridge):
/// BASIC ROM at `$A000` when LORAM and HIRAM are both set; KERNAL ROM at `$E000` when HIRAM is set; at `$D000` RAM
/// when LORAM and HIRAM are both clear, otherwise I/O when CHAREN is set and the character ROM when it is clear.
fn the_port_bits_choose_rom_ram_and_io(env: &Env) -> Result<(), String> {
    let rom = |name: &str| -> Result<Vec<u8>, String> {
        std::fs::read(env.file(&format!("commodore/roms/c64/{name}"))?).map_err(|e| e.to_string())
    };
    let (basic, kernal, chargen) = (
        rom("basic-901226-01.bin")?,
        rom("kernal-901227-03.bin")?,
        rom("chargen-901225-01.bin")?,
    );
    // The marker byte in RAM under each ROM, and the VIC register 0 value that shows I/O. All three differ from the
    // first byte of every ROM so a read is unambiguous.
    const RAM: u8 = 0x5A;
    const IO: u8 = 0x33;
    for rom_byte in [basic[0], kernal[0], chargen[0]] {
        if rom_byte == RAM || rom_byte == IO {
            return Err("a ROM starts with a marker byte; pick other markers".into());
        }
    }
    let mut source = format!(
        " sei\n lda #$2F\n sta $00\n lda #$34\n sta $01\n lda #${RAM:02X}\n sta $A000\n sta $D000\n sta $E000\n lda #$37\n sta $01\n lda #${IO:02X}\n sta $D000\n"
    );
    for mode in 0..8u8 {
        let at = RESULTS + u16::from(mode) * 3;
        source += &format!(
            " lda #${:02X}\n sta $01\n lda $A000\n sta ${at:04X}\n lda $D000\n sta ${:04X}\n lda $E000\n sta ${:04X}\n",
            0x30 | mode,
            at + 1,
            at + 2
        );
    }
    source += " lda #$37\n sta $01\nhere: jmp here\n";
    let out = run_program(env, "banking", &source, 3, &[(RESULTS, 24)])?;
    for mode in 0..8u8 {
        let (loram, hiram, charen) = (mode & 1 != 0, mode & 2 != 0, mode & 4 != 0);
        let want_a000 = if loram && hiram { basic[0] } else { RAM };
        let want_e000 = if hiram { kernal[0] } else { RAM };
        let want_d000 = match (loram || hiram, charen) {
            (false, _) => RAM,
            (true, true) => IO,
            (true, false) => chargen[0],
        };
        let at = RESULTS + u16::from(mode) * 3;
        let what = format!(
            "$01 = ${:02X} (LORAM {loram}, HIRAM {hiram}, CHAREN {charen})",
            0x30 | mode
        );
        expect_eq(&format!("$A000 with {what}"), out.byte(at)?, want_a000)?;
        expect_eq(&format!("$D000 with {what}"), out.byte(at + 1)?, want_d000)?;
        expect_eq(&format!("$E000 with {what}"), out.byte(at + 2)?, want_e000)?;
    }
    Ok(())
}

fn a_pal_frame_is_19656_cycles(env: &Env) -> Result<(), String> {
    // VIC: 312 lines of 63 cycles. Measured with a CIA 1 timer (one count a cycle) between two successive starts of raster
    // line 0: wait for line 0, start the timer, leave line 0, wait for line 0 again, read the timer. Each wait polls
    // with a loop of a few cycles, so the result is good to a few tens of cycles. $D012 is read before $D011: the other
    // order can read bit 8 just before line 256 starts and the low byte (0) just after it, which looks like line 0.
    let wait_line_0 = |label: &str| {
        format!(
            "{label}: lda $D012
 bne {label}
 lda $D011
 bmi {label}
"
        )
    };
    let source = format!(
        " sei
 lda #$7F
 sta $DC0D
 lda #$FF
 sta $DC04
 sta $DC05
{}         lda #$11
 sta $DC0E
leave: lda $D012
 beq leave
{}         lda $DC04
 sta ${:04X}
 lda $DC05
 sta ${:04X}
here: jmp here
",
        wait_line_0("w1"),
        wait_line_0("w2"),
        RESULTS,
        RESULTS + 1
    );
    let out = run_program(env, "frame", &source, 4, &[(RESULTS, 2)])?;
    let elapsed = 0xFFFF - i64::from(out.word(RESULTS)?);
    expect_in(
        "cycles between two raster line 0s",
        elapsed,
        19_656 - 30,
        19_656 + 30,
    )
}

fn the_raster_runs_through_312_lines(env: &Env) -> Result<(), String> {
    // Read $D011 (bit 7 is bit 8 of the raster line) and $D012 over and over, keeping the highest low byte seen while
    // the line is 256 or more. VIC: a PAL frame has lines 0-311, so that is 311 - 256 = 55 = $37.
    let source = format!(
        " sei\n lda #0\n sta ${RESULTS:04X}\nloop: lda $D011\n bpl loop\n lda $D012\n cmp ${RESULTS:04X}\n bcc loop\n sta ${RESULTS:04X}\n jmp loop\n"
    );
    let out = run_program(env, "raster", &source, 4, &[(RESULTS, 1)])?;
    expect_eq("the last line past 255", out.byte(RESULTS)?, 0x37)
}

fn the_kernal_jiffy_clock_ticks_at_60_hz(env: &Env) -> Result<(), String> {
    let jiffies = |frames| -> Result<i64, String> {
        let out = env.c64()?.frames(frames).dump(0x00A0, 3).go()?;
        let b = out.bytes(0x00A0, 3)?;
        Ok(i64::from(b[0]) << 16 | i64::from(b[1]) << 8 | i64::from(b[2]))
    };
    let early = jiffies(200)?;
    let later = jiffies(300)?;
    // C64WIKI: the jiffy clock at $A0-$A2 is updated every 1/60 second by the KERNAL IRQ. 100 PAL frames are 100 x
    // 19656 cycles at 985248 Hz = 1.9951 s, i.e. 119.7 jiffies.
    expect_in("jiffies in 100 frames", later - early, 118, 121)
}

fn basic_evaluates_a_typed_expression(env: &Env) -> Result<(), String> {
    let out = env
        .c64()?
        .arg("--type-after")
        .arg("150")
        .arg("--type-delay")
        .arg("100")
        .arg("--type")
        .arg("PRINT 12*34\n")
        .frames(400)
        .dump(0x0400, 1000)
        .go()?;
    let screen = out.bytes(0x0400, 1000)?;
    // BASIC V2 prints the value on the line under the typed one.
    if find_text(&screen, "408").is_none() {
        return Err("the screen does not show 408 after PRINT 12*34".into());
    }
    Ok(())
}

fn voice_3_oscillator_reads_back_through_the_sid(env: &Env) -> Result<(), String> {
    // SID: register $1B (OSC3) holds the upper 8 bits of voice 3's waveform generator. With a sawtooth running it sweeps
    // through the whole range; with the frequency at 0 it stands still.
    let sample = |freq: u8| -> Result<Vec<u8>, String> {
        let source = format!(
            " sei\n lda #$0F\n sta $D418\n lda #${freq:02X}\n sta $D40E\n sta $D40F\n lda #$21\n sta $D412\n ldx #0\ns: lda $D41B\n sta ${RESULTS:04X},x\n ldy #7\nd: dey\n bne d\n inx\n cpx #64\n bne s\nhere: jmp here\n"
        );
        run_program(env, "sid", &source, 3, &[(RESULTS, 64)])?.bytes(RESULTS, 64)
    };
    let running = sample(0xFF)?;
    let (min, max) = (running.iter().min().unwrap(), running.iter().max().unwrap());
    if i32::from(*max) - i32::from(*min) < 0x80 {
        return Err(format!(
            "a running sawtooth should sweep, got min {min:#04X} max {max:#04X}"
        ));
    }
    let standing = sample(0x00)?;
    expect_eq(
        "samples with frequency 0",
        standing
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        1,
    )
}

fn a_cia_timer_counts_system_clock_cycles(env: &Env) -> Result<(), String> {
    // CIA: timer A of CIA 1 counts down once per phi2 cycle. Load $FFFF, start it, run a loop of known length, read it.
    // The loop is LDX #$FF (2) + 255 x (DEX 2 + BNE 3) - 1 (the last BNE falls through) = 1276 cycles, then the LDA reads
    // the counter on its last cycle (4 in all): about 1280 cycles after the start.
    let source = format!(
        " sei\n lda #$7F\n sta $DC0D\n lda #$FF\n sta $DC04\n sta $DC05\n lda #$11\n sta $DC0E\n ldx #$FF\nd: dex\n bne d\n lda $DC04\n sta ${:04X}\n lda $DC05\n sta ${:04X}\nhere: jmp here\n",
        RESULTS,
        RESULTS + 1
    );
    let out = run_program(env, "cia", &source, 3, &[(RESULTS, 2)])?;
    let counter = i64::from(out.word(RESULTS)?);
    expect_in(
        "timer after the loop",
        counter,
        0xFFFF - 1280 - 12,
        0xFFFF - 1280 + 12,
    )
}

const CASES: &[Case] = &[
    Case::new(
        Area::Rom,
        "the C64 boots to the BASIC V2 banner",
        "BANNER; C64WIKI memory map (38911 bytes)",
        boots_to_the_basic_v2_banner,
    ),
    Case::new(
        Area::Io,
        "the CPU port holds $2F / $37 after the KERNAL's start-up",
        "C64WIKI Zeropage and Memory Map",
        the_cpu_port_is_set_up_for_the_standard_map,
    ),
    Case::new(
        Area::Memory,
        "LORAM, HIRAM and CHAREN choose BASIC, KERNAL, RAM, character ROM and I/O",
        "C64WIKI Bank Switching table",
        the_port_bits_choose_rom_ram_and_io,
    ),
    Case::new(
        Area::Timers,
        "a PAL frame is 19656 cycles",
        "VIC (6569: 312 lines of 63 cycles)",
        a_pal_frame_is_19656_cycles,
    ),
    Case::new(
        Area::Video,
        "the raster counts lines 0-311",
        "VIC (6569 has 312 lines)",
        the_raster_runs_through_312_lines,
    ),
    Case::new(
        Area::Timers,
        "the KERNAL jiffy clock ticks 60 times a second",
        "C64WIKI Zeropage ($A0-$A2); 985248 Hz",
        the_kernal_jiffy_clock_ticks_at_60_hz,
    ),
    Case::new(
        Area::Io,
        "BASIC evaluates a typed PRINT 12*34",
        "the BASIC V2 interpreter on the emulated machine",
        basic_evaluates_a_typed_expression,
    ),
    Case::new(
        Area::Sound,
        "voice 3's oscillator reads back through $D41B",
        "SID 6581 datasheet (OSC3, register $1B)",
        voice_3_oscillator_reads_back_through_the_sid,
    ),
    Case::new(
        Area::Timers,
        "a CIA timer counts system clock cycles",
        "CIA MOS 6526 datasheet",
        a_cia_timer_counts_system_clock_cycles,
    ),
];

#[test]
fn c64() {
    let Some(env) = Env::discover() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    if env.file("commodore/roms/c64/kernal-901227-03.bin").is_err() {
        eprintln!("skipping: the C64 ROMs are not present");
        return;
    }
    let report = run_cases(&env, CASES);
    eprintln!("{}", report.summary());
    assert!(report.ok(), "{}", report.summary());
}

// ---- third-party diagnostics (the C64 dead test)

const DEAD_TEST: &str = "commodore/software/diagnostics/deadtest_mod.bin";

/// A `.crt` for an 8K ROM at `$E000` in Ultimax mode (EXROM high, GAME low: the cartridge supplies the reset vector).
fn ultimax_crt(rom: &[u8]) -> Vec<u8> {
    let mut crt = b"C64 CARTRIDGE   ".to_vec();
    crt.extend_from_slice(&0x40u32.to_be_bytes());
    crt.extend_from_slice(&0x0100u16.to_be_bytes());
    crt.extend_from_slice(&0u16.to_be_bytes()); // hardware type 0: a normal cartridge
    crt.extend_from_slice(&[1, 0]); // EXROM 1, GAME 0
    crt.extend_from_slice(&[0; 6]);
    crt.extend_from_slice(&[b"DEADTEST".as_slice(), &[0; 24]].concat());
    crt.extend_from_slice(b"CHIP");
    crt.extend_from_slice(&(16 + rom.len() as u32).to_be_bytes());
    crt.extend_from_slice(&0u16.to_be_bytes()); // ROM
    crt.extend_from_slice(&0u16.to_be_bytes()); // bank 0
    crt.extend_from_slice(&0xE000u16.to_be_bytes());
    crt.extend_from_slice(&(rom.len() as u16).to_be_bytes());
    crt.extend_from_slice(rom);
    crt
}

/// The Commodore dead test (rev 781220, as modified for crystalct's unified cartridge) runs from an Ultimax cartridge with no
/// KERNAL involved at all: it tests the zero page, the stack page, the screen RAM and the colour RAM, prints OK next to each that
/// passes, and goes on to the main RAM test. That one marches every byte of the 64K through 20 patterns with a delay loop each --
/// about half an hour of machine time for a pass -- so the case stops once it has begun, at frame 4000 (about 80 s of machine
/// time; the colour RAM test ends around frame 3500).
fn the_dead_test_cartridge_boots_in_ultimax_mode_and_passes_the_low_ram_tests(
    env: &Env,
) -> Result<(), String> {
    let rom = std::fs::read(env.file(DEAD_TEST)?).map_err(|e| e.to_string())?;
    let crt = std::env::temp_dir().join(format!("rc-suite-deadtest-{}.crt", std::process::id()));
    std::fs::write(&crt, ultimax_crt(&rom)).map_err(|e| e.to_string())?;
    let out = env
        .c64()?
        .arg("--cartridge")
        .arg(crt.to_string_lossy())
        .frames(4000)
        .dump(0x0400, 1000)
        .go();
    let _ = std::fs::remove_file(&crt);
    let screen = out?.bytes(0x0400, 1000)?;
    if find_text(&screen, "C-64 DEAD TEST REV 781220").is_none() {
        return Err(
            "the dead test's title is not on the screen: the Ultimax cartridge did not start"
                .into(),
        );
    }
    // "OK" (screen codes 15, 11) in the row of each result, after its label.
    let ok = [0x0F, 0x0B];
    for label in ["ZERO PAGE", "STACK PAGE", "SCREEN RAM", "COLOR RAM"] {
        let at = find_text(&screen, label).ok_or(format!("no {label:?} line on the screen"))?;
        let row = &screen[at..(at / 40 + 1) * 40];
        if !row.windows(2).any(|w| w == ok) {
            return Err(format!("{label}: no OK on its line"));
        }
    }
    // The main RAM test has begun (its label is up, with no result yet and no failure).
    let at = find_text(&screen, "RAM TEST").ok_or("the main RAM test never started")?;
    let row = &screen[at..(at / 40 + 1) * 40];
    if row.windows(2).any(|w| w == ok) {
        return Err(
            "the main RAM test cannot be finished yet; the picture is not what was expected".into(),
        );
    }
    Ok(())
}

const DIAG_CASES: &[Case] = &[Case::new(
    Area::Memory,
    "the Commodore dead test boots from an Ultimax cartridge, passes the zero page, stack, screen and colour RAM tests and starts the RAM test",
    "the dead test (rev 781220): OK printed per passing test; Ultimax reset vector from ROMH at $E000",
    the_dead_test_cartridge_boots_in_ultimax_mode_and_passes_the_low_ram_tests,
)];

#[test]
fn c64_diagnostics() {
    let Some(env) = Env::discover() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    if env.file(DEAD_TEST).is_err() || env.file("commodore/roms/c64/kernal-901227-03.bin").is_err()
    {
        eprintln!(
            "skipping: {DEAD_TEST} or the C64 ROMs are not present (tools/fetch_diagnostics.sh fetches the former)"
        );
        return;
    }
    let report = run_cases(&env, DIAG_CASES);
    eprintln!("{}", report.summary());
    assert!(report.ok(), "{}", report.summary());
}
