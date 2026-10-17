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
        Area::Timers,
        "the KERNAL jiffy clock ticks 60 times a second",
        "C64WIKI Zeropage ($A0-$A2); 985248 Hz",
        the_kernal_jiffy_clock_ticks_at_60_hz,
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

// ---- third-party diagnostics (crystalct's Unified Diagnostic Cartridge: the dead test, then the standard diagnostic)

const UNIFIED_32K: &str = "commodore/software/diagnostics/unified_32k.bin";

/// The `.crt` of crystalct's Unified Diagnostic Cartridge: retroclawd's own hardware type 0xC640 (the real board is written straight
/// to an EPROM, the `.crt` format has no entry for it), the standard diagnostic as the 8K chip at `$8000` and the modified dead test
/// as the one at `$E000`, EXROM high and GAME low for the board's default Ultimax start.
fn unified_crt(image: &[u8]) -> Vec<u8> {
    let mut crt = b"C64 CARTRIDGE   ".to_vec();
    crt.extend_from_slice(&0x40u32.to_be_bytes());
    crt.extend_from_slice(&0x0100u16.to_be_bytes());
    crt.extend_from_slice(&0xC640u16.to_be_bytes());
    crt.extend_from_slice(&[1, 0]);
    crt.extend_from_slice(&[0; 6]);
    crt.extend_from_slice(&[b"UNIFIED DIAG".as_slice(), &[0; 20]].concat());
    for (load, half) in [
        (0x8000u16, &image[..0x2000]),
        (0xE000, &image[0x2000..0x4000]),
    ] {
        crt.extend_from_slice(b"CHIP");
        crt.extend_from_slice(&(16 + half.len() as u32).to_be_bytes());
        crt.extend_from_slice(&0u16.to_be_bytes());
        crt.extend_from_slice(&0u16.to_be_bytes());
        crt.extend_from_slice(&load.to_be_bytes());
        crt.extend_from_slice(&(half.len() as u16).to_be_bytes());
        crt.extend_from_slice(half);
    }
    crt
}

/// The screen RAM of the whole unified cartridge (see below) after 8000 frames, with `extra` command-line arguments.
fn unified_screen(env: &Env, extra: &[&str]) -> Result<Vec<u8>, String> {
    let image = std::fs::read(env.file(UNIFIED_32K)?).map_err(|e| e.to_string())?;
    if image.len() != 0x8000 {
        return Err(format!(
            "{UNIFIED_32K} is {} bytes, expected 32768",
            image.len()
        ));
    }
    let crt = std::env::temp_dir().join(format!(
        "rc-suite-unified-{}-{}.crt",
        std::process::id(),
        extra.len()
    ));
    std::fs::write(&crt, unified_crt(&image)).map_err(|e| e.to_string())?;
    let mut run = env.c64()?.arg("--cartridge").arg(crt.to_string_lossy());
    for arg in extra {
        run = run.arg(*arg);
    }
    let out = run.frames(8000).dump(0x0400, 1000).go();
    let _ = std::fs::remove_file(&crt);
    out?.bytes(0x0400, 1000)
}

/// The whole unified cartridge, as the hardware does it: the modified dead test runs in Ultimax mode (zero page, stack page, screen
/// RAM, colour RAM, the low-RAM test, the sound test), ends its first cycle by copying a routine to `$0334` that writes `$01` to
/// `$DE00` -- the board's flip-flop turns the cartridge into an ordinary 8K one -- and jumps to the KERNAL's reset routine, which finds
/// `CBM80` in the first 8K and starts the *standard* diagnostic (rev 586220++, Commodore's). The switch comes between frames 5000 and
/// 6000; at frame 8000 the standard diagnostic has filled in its first pass. What it must show on a healthy machine without its test
/// plugs: every RAM and ROM test OK, the three ROM checksums, and the CIA timers and interrupt OK. Cassette, control port, serial
/// port and user port read BAD/OPEN, as they do on a real machine with nothing plugged in, so they are not asserted.
fn the_unified_cartridge_switches_to_the_standard_diagnostic_and_it_passes(
    env: &Env,
) -> Result<(), String> {
    let screen = unified_screen(env, &[])?;
    if find_text(&screen, "C-64 DIAGNOSTIC REV586220").is_none() {
        return Err(
            "the standard diagnostic's title is not on the screen: the cartridge never switched out of Ultimax mode".into(),
        );
    }
    let ok = [0x0F, 0x0B];
    let row_of = |label: &str| -> Result<&[u8], String> {
        let at = find_text(&screen, label).ok_or(format!("no {label:?} on the screen"))?;
        Ok(&screen[at..(at / 40 + 1) * 40])
    };
    let has_ok = |row: &[u8]| row.windows(2).any(|w| w == ok);
    for label in [
        "ZERO PAGE",
        "STACK PAGE",
        "SCREEN RAM",
        "RAM TEST1",
        "RAM TEST2",
        "PLA TEST",
        "COLOR RAM",
        "INTERRUPT",
    ] {
        if !has_ok(row_of(label)?) {
            return Err(format!("{label}: no OK on its line"));
        }
    }
    // TIMER1 and TIMER2 each have an A line (the label's own) and a B line under it.
    for label in ["TIMER1", "TIMER2"] {
        let at = find_text(&screen, label).ok_or(format!("no {label:?} on the screen"))?;
        let (a, b) = (at / 40, at / 40 + 1);
        for row in [a, b] {
            if !has_ok(&screen[row * 40 + 20..row * 40 + 40]) {
                return Err(format!(
                    "{label}: no OK on its line {}",
                    if row == a { "A" } else { "B" }
                ));
            }
        }
    }
    // The checksums of the three committed ROMs, as the diagnostic prints them.
    for (label, sum) in [
        ("KERNAL ROM", "B6C70A"),
        ("BASIC ROM", "043D56"),
        ("CHARAC ROM", "F3F7F8"),
    ] {
        let row = row_of(label)?;
        let text =
            String::from_utf8_lossy(&row.iter().map(|c| screen_char(*c)).collect::<Vec<u8>>())
                .to_string();
        if !text.contains(sum) {
            return Err(format!(
                "{label}: expected checksum {sum}, the line reads {text:?}"
            ));
        }
    }
    // The diagnostic judges the 6581 by reading POTX/POTY and wants both in $50-$8F, what its test plug (paddle resistors) gives; with
    // nothing in the control ports they read $FF, as on a real machine, so the chip table marks it BAD.
    if !has_bad(row_of("6581 U18")?) {
        return Err("with no paddle in the ports the 6581 should read BAD (open POT lines)".into());
    }
    Ok(())
}

/// "BAD" (screen codes 2, 1, 4) somewhere in a screen row.
fn has_bad(row: &[u8]) -> bool {
    row.windows(3).any(|w| w == [0x02, 0x01, 0x04])
}

/// A screen code as ASCII (upper case set: 1-26 letters, 48-57 digits, 32 space), anything else a dot.
fn screen_char(code: u8) -> u8 {
    match code {
        1..=26 => b'A' + code - 1,
        0x20..=0x3F => code,
        _ => b'.',
    }
}

const DIAG_CASES: &[Case] = &[Case::new(
    Area::Memory,
    "the whole unified cartridge: the dead test switches it out of Ultimax mode and the standard diagnostic passes its RAM, ROM, timer and interrupt tests",
    "the Unified Diagnostic Cartridge: a write to $DE00 flips Ultimax to 8K mode, the KERNAL restarts and starts the standard diagnostic (rev 586220++)",
    the_unified_cartridge_switches_to_the_standard_diagnostic_and_it_passes,
)];

#[test]
fn c64_diagnostics() {
    let Some(env) = Env::discover() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    if env.file(UNIFIED_32K).is_err()
        || env.file("commodore/roms/c64/kernal-901227-03.bin").is_err()
    {
        eprintln!(
            "skipping: {UNIFIED_32K} or the C64 ROMs are not present (tools/fetch_diagnostics.sh fetches the former)"
        );
        return;
    }
    let report = run_cases(&env, DIAG_CASES);
    eprintln!("{}", report.summary());
    assert!(report.ok(), "{}", report.summary());
}
