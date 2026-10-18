//! Commodore 64 timing and I/O acid programs: small programs of our own that exercise the VIC-II, the CIAs and the ports and
//! compare what happens with what the published documents say a real machine does.
//!
//! * `BAUER` = Christian Bauer, "The MOS 6567/6569 video controller (VIC-II) and its application in the Commodore 64",
//!   <https://www.cebix.net/VIC-Article.txt>: the 6569 has 312 lines of 63 cycles; a bad line (raster $30-$F7, the line's low
//!   three bits equal to YSCROLL, DEN set) "stuns" the processor for 40-43 cycles with the c-accesses in cycles 15-54; the bits
//!   marked `-` in its register table "are not connected and yield 1 on reading" (the upper nibble of $D020-$D02E, bits 4-6 of
//!   $D019, bits 4-7 of $D01A, bits 6-7 of $D016, bit 0 of $D018); "the unused addresses $d02f-$d03f yield $ff on reading, a
//!   write access is ignored"; the registers "are repeated every 64 bytes in the area $d000-$d3ff"; $D01E and $D01F "are
//!   automatically cleared on reading".
//! * `C64WIKI-KEYBOARD` = <https://www.c64-wiki.com/wiki/Keyboard>: the matrix: a column is selected by driving its line of
//!   CIA 1 port A ($DC00) low and the keys of that column read as zeros on port B ($DC01) -- RETURN PA0/PB1, A PA1/PB2,
//!   SPACE PA7/PB4, 1 PA7/PB0, Q PA7/PB6, RUN/STOP PA7/PB7, left SHIFT PA1/PB7, CRSR right PA0/PB2, DEL PA0/PB0, F1 PA0/PB4.
//! * `C64WIKI-JOYSTICK` = <https://www.c64-wiki.com/wiki/Joystick>: "a joystick on port #1 is read through $DC01, and one on
//!   control port #2 via $DC00"; up is bit 0, down bit 1, left bit 2, right bit 3, fire bit 4, each reading 0 when active.
//! * `C64WIKI-CIA` = <https://www.c64-wiki.com/wiki/CIA>: control register A bit 3 "1 = timer stops after underflow" (one-shot),
//!   "0 = timer restart after underflow (latch will be reloaded)"; control register B bits 5-6 `%10` = "timer counts underflow of
//!   timer A"; "flags will be cleared after reading" the interrupt control register ($DC0D).
//! * `CIA6526` = the MOS 6526 datasheet: a timer counts down once per phi2; a latch of N gives a period of N+1 cycles.
//! * `MOS6502` = the 6502 cycle counts: `INC zp` 5, `BNE` 3 taken and 2 not, `JMP abs` 3, `DEX`/`DEY`/`INX` 2, `LDX/LDY #` 2.
//!
//! Programs run through `rc_suite::mos6502_program` and `--program`: the CPU starts at the program with nothing initialised (no
//! KERNAL boot). Needs `RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS` and `RC_ROMS`; skips without.

use rc_suite::{Area, Case, Env, Outcome, expect_eq, mos6502_program, run_cases};

const ORG: u16 = 0xC000;
/// Where programs leave their results (always RAM).
const RESULTS: u16 = 0x0340;

fn run_program(
    env: &Env,
    tag: &str,
    source: &str,
    frames: u32,
    dumps: &[(u16, u16)],
    extra: &[&str],
) -> Result<Outcome, String> {
    let file = mos6502_program(tag, ORG, source)?;
    let mut run = env
        .c64()?
        .arg("--program")
        .arg(file.to_string_lossy())
        .arg("--org")
        .arg(format!("0x{ORG:04X}"))
        .frames(frames);
    for e in extra {
        run = run.arg(*e);
    }
    for (start, len) in dumps {
        run = run.dump(*start, *len);
    }
    run.go()
}

/// RAM everywhere but the I/O area ($01 = $35), so the IRQ vector at $FFFE/$FFFF is ours; the CIA interrupts off.
fn irq_setup(handler: &str) -> String {
    format!(
        " sei
 lda #$2F
 sta $00
 lda #$35
 sta $01
 lda #<{handler}
 sta $FFFE
 lda #>{handler}
 sta $FFFF
 lda #$7F
 sta $DC0D
 sta $DD0D
 lda $DC0D
 lda $DD0D
"
    )
}

// ---- timing

fn a_raster_interrupt_arrives_in_the_line_it_was_set_for(env: &Env) -> Result<(), String> {
    // BAUER: 312 lines on the 6569, counted from 0; the line is in $D012 and, as bit 8, in bit 7 of $D011. The handler reads both
    // as soon as it is entered, a few cycles into the line.
    for line in [1u16, 0x30, 0x80, 0xF8, 0x100, 0x137] {
        let source = format!(
            "{}\
 lda #$0B
 sta $D011
 lda #${:02X}
 sta $D012
 lda $D011
 and #$7F
 ora #${:02X}
 sta $D011
 lda #1
 sta $D01A
 lda $D019
 sta $D019
 cli
here: jmp here
irq: pha
 lda $D012
 sta ${:04X}
 lda $D011
 sta ${:04X}
 lda #1
 sta $D019
 lda #0
 sta $D01A
 pla
 rti
",
            irq_setup("irq"),
            line & 0xFF,
            if line > 0xFF { 0x80 } else { 0 },
            RESULTS,
            RESULTS + 1
        );
        let out = run_program(env, "raster_irq", &source, 3, &[(RESULTS, 2)], &[])?;
        let seen = u16::from(out.byte(RESULTS)?)
            | if out.byte(RESULTS + 1)? & 0x80 != 0 {
                0x100
            } else {
                0
            };
        expect_eq(
            &format!("line read in the handler for line {line}"),
            seen,
            line,
        )?;
    }
    Ok(())
}

/// How many iterations of the 8-cycle counting loop (`INC $FB; BNE`, `INC $FC` and `JMP` every 256th) fit in one frame, from one
/// raster IRQ at line 0 to the next, with the display on (`d011` = $1B) or off ($0B). The loop costs 8 cycles an iteration, plus
/// 7 more each time the low byte wraps.
fn work_in_a_frame(env: &Env, d011: u8) -> Result<i64, String> {
    let source = format!(
        "{}\
 lda #{d011}
 sta $D011
 lda #0
 sta $D012
 sta $FB
 sta $FC
 sta $FD
 lda #1
 sta $D01A
 lda $D019
 sta $D019
 cli
main: inc $FB
 bne main
 inc $FC
 jmp main
irq: pha
 lda #1
 sta $D019
 lda $FD
 bne second
 inc $FD
 lda #0
 sta $FB
 sta $FC
 pla
 rti
second: lda $FB
 sta ${:04X}
 lda $FC
 sta ${:04X}
 lda #0
 sta $D01A
 pla
 rti
",
        irq_setup("irq"),
        RESULTS,
        RESULTS + 1
    );
    let out = run_program(env, "badlines", &source, 4, &[(RESULTS, 2)], &[])?;
    let (low, high) = (
        i64::from(out.byte(RESULTS)?),
        i64::from(out.byte(RESULTS + 1)?),
    );
    // cycles of work = 8 per iteration + 7 per wrap of the low byte
    Ok(8 * (high * 256 + low) + 7 * high)
}

fn bad_lines_stun_the_processor_for_40_to_43_cycles_each(env: &Env) -> Result<(), String> {
    // BAUER: with the display on, the 25 text lines of a screen each have a bad line ($30-$F7, one in 8) in which the VIC takes
    // the bus for 40-43 cycles. With DEN clear there are none, so the work a frame leaves the processor differs by exactly the
    // stolen cycles (everything else is the same in both runs).
    let off = work_in_a_frame(env, 0x0B)?;
    let on = work_in_a_frame(env, 0x1B)?;
    let stolen = off - on;
    // The loop is 8 cycles an iteration, so the figure is good to 8 either way.
    if !(25 * 40 - 8..=25 * 43 + 8).contains(&stolen) {
        return Err(format!(
            "the display on cost the processor {stolen} cycles a frame (off {off}, on {on}); 25 bad lines of 40-43 cycles is {}-{}",
            25 * 40,
            25 * 43
        ));
    }
    Ok(())
}

fn a_pal_frame_has_312_lines_numbered_from_0(env: &Env) -> Result<(), String> {
    // BAUER: the 6569 has 312 lines. Poll the raster ($D012, and bit 7 of $D011 as its ninth bit) for three frames with the display
    // off (no bad lines, so the loop gets every line) and mark each line seen. A line can be missed or double read only where
    // $D012 and $D011 are read either side of the line changing; the two places are 255 to 256 and 311 to 0, and the marks
    // they leave are ignored.
    let source = "\
 sei
 lda #$2F
 sta $00
 lda #$35
 sta $01
 lda #$0B
 sta $D011
 lda #0
 tay
clear: sta $2000,y
 sta $2100,y
 iny
 bne clear
 ldx #0
 ldy #0
poll: lda $D012
 sta $FB
 lda $D011
 bmi high
 ldx $FB
 lda #1
 sta $2000,x
 jmp next
high: ldx $FB
 lda #1
 sta $2100,x
next: inc $FC
 bne poll
 inc $FD
 lda $FD
 cmp #$30
 bne poll
here: jmp here
";
    let out = run_program(env, "lines", source, 12, &[(0x2000, 0x200)], &[])?;
    let (low, high) = (out.bytes(0x2000, 0x100)?, out.bytes(0x2100, 0x100)?);
    let missing: Vec<usize> = (0..0x100)
        .filter(|&i| low[i] == 0)
        .chain((0..0x38).filter(|&i| high[i] == 0).map(|i| i + 0x100))
        .collect();
    expect_eq("lines never seen", missing, Vec::<usize>::new())?;
    // Line 312 and up do not exist ($138-$1FE; $1FF is the 255 to 256 hand-over).
    let beyond: Vec<usize> = (0x38..0xFF)
        .filter(|&i| high[i] != 0)
        .map(|i| i + 0x100)
        .collect();
    expect_eq("lines seen beyond 311", beyond, Vec::<usize>::new())
}

fn a_cia_timer_period_is_its_latch_plus_one_cycle(env: &Env) -> Result<(), String> {
    // CIA6526 / C64WIKI-CIA: timer A, latch 99, runs continuously; timer B counts timer A's underflows. In a delay of
    // 16 x (2 + 255 x 5 + 4 + 2 + 3) - 1 = 20575 cycles that is 20575 / 100 = 205.75 underflows, so 205 or 206; a period of 99
    // cycles (latch, not latch + 1) would give 207 or 208.
    let source = format!(
        " sei
 lda #$7F
 sta $DC0D
 lda #99
 sta $DC04
 lda #0
 sta $DC05
 lda #$FF
 sta $DC06
 sta $DC07
 lda #$41
 sta $DC0F
 lda #$01
 sta $DC0E
 ldy #16
outer: ldx #0
inner: dex
 bne inner
 dey
 bne outer
 lda $DC06
 sta ${:04X}
 lda $DC07
 sta ${:04X}
here: jmp here
",
        RESULTS,
        RESULTS + 1
    );
    let out = run_program(env, "cia_period", &source, 3, &[(RESULTS, 2)], &[])?;
    let underflows = 0xFFFF - i64::from(out.word(RESULTS)?);
    if !(205..=206).contains(&underflows) {
        return Err(format!(
            "timer B counted {underflows} underflows of timer A (latch 99) in 20575 cycles; a period of latch + 1 = 100 cycles gives 205 or 206"
        ));
    }
    Ok(())
}

fn a_one_shot_timer_stops_at_underflow_and_sets_its_flag(env: &Env) -> Result<(), String> {
    // C64WIKI-CIA: control register A bit 3 = 1 stops the timer after underflow; the underflow sets bit 0 of the interrupt
    // register. Latch 50, start with bit 3 set, wait 200 cycles.
    let source = format!(
        " sei
 lda #$7F
 sta $DC0D
 lda $DC0D
 lda #50
 sta $DC04
 lda #0
 sta $DC05
 lda #$09
 sta $DC0E
 ldx #60
w: dex
 bne w
 lda $DC0E
 sta ${:04X}
 lda $DC0D
 sta ${:04X}
here: jmp here
",
        RESULTS,
        RESULTS + 1
    );
    let out = run_program(env, "cia_oneshot", &source, 3, &[(RESULTS, 2)], &[])?;
    expect_eq(
        "control register A bit 0 (start) after underflow",
        out.byte(RESULTS)? & 1,
        0,
    )?;
    expect_eq(
        "interrupt register bit 0 (timer A underflow)",
        out.byte(RESULTS + 1)? & 1,
        1,
    )
}

// ---- I/O

fn the_unconnected_bits_of_the_vic_registers_read_one(env: &Env) -> Result<(), String> {
    // BAUER's register table: writing zeros leaves exactly the `-` bits reading 1.
    let mut source = String::from(" sei\n lda #$2F\n sta $00\n lda #$35\n sta $01\n lda #0\n");
    for (at, reg) in (RESULTS..).zip((0xD020..=0xD02E).chain([0xD016, 0xD018, 0xD01A])) {
        source += &format!(" sta ${reg:04X}\n lda ${reg:04X}\n sta ${at:04X}\n lda #0\n");
    }
    source += "here: jmp here
";
    let out = run_program(env, "vic_unused", &source, 3, &[(RESULTS, 18)], &[])?;
    let got = out.bytes(RESULTS, 18)?;
    // $D020-$D02E: upper nibble reads 1 and the written zeros read 0 in the lower one.
    for (i, &v) in got[..15].iter().enumerate() {
        expect_eq(&format!("${:04X} after writing 0", 0xD020 + i), v, 0xF0)?;
    }
    expect_eq("$D016 bits 6-7", got[15] & 0xC0, 0xC0)?;
    expect_eq("$D018 bit 0", got[16] & 0x01, 0x01)?;
    expect_eq("$D01A bits 4-7", got[17] & 0xF0, 0xF0)?;
    // $D019 was read last into the slot after those: bits 4-6.
    let d019 = run_program(
        env,
        "vic_d019",
        " sei\n lda #$2F\n sta $00\n lda #$35\n sta $01\n lda $D019\n sta $0340\nhere: jmp here\n",
        3,
        &[(RESULTS, 1)],
        &[],
    )?;
    expect_eq("$D019 bits 4-6", d019.byte(RESULTS)? & 0x70, 0x70)
}

fn the_unused_vic_addresses_read_ff_and_ignore_writes(env: &Env) -> Result<(), String> {
    // BAUER: $D02F-$D03F yield $FF on reading, a write access is ignored.
    let mut source = String::from(" sei\n lda #$2F\n sta $00\n lda #$35\n sta $01\n lda #$55\n");
    for reg in 0xD02F..=0xD03F {
        source += &format!(" sta ${reg:04X}\n");
    }
    for (i, reg) in (0xD02F..=0xD03F).enumerate() {
        source += &format!(" lda ${reg:04X}\n sta ${:04X}\n", RESULTS + i as u16);
    }
    source += "here: jmp here\n";
    let out = run_program(env, "vic_open", &source, 3, &[(RESULTS, 17)], &[])?;
    expect_eq("$D02F-$D03F", out.bytes(RESULTS, 17)?, vec![0xFF; 17])
}

fn the_vic_registers_repeat_every_64_bytes_up_to_d3ff(env: &Env) -> Result<(), String> {
    // BAUER: "the VIC registers are repeated every 64 bytes in the area $d000-$d3ff". $D021 = 5 seen through the images at $D061,
    // $D0A1 and $D3E1 (the upper nibble reads 1 where BAUER says it does; only the lower one is compared).
    let mut source =
        String::from(" sei\n lda #$2F\n sta $00\n lda #$35\n sta $01\n lda #$05\n sta $D021\n");
    for (i, reg) in [0xD061u16, 0xD0A1, 0xD3E1].into_iter().enumerate() {
        source += &format!(" lda ${reg:04X}\n sta ${:04X}\n", RESULTS + i as u16);
    }
    source += "here: jmp here\n";
    let out = run_program(env, "vic_mirror", &source, 3, &[(RESULTS, 3)], &[])?;
    let low: Vec<u8> = out.bytes(RESULTS, 3)?.iter().map(|b| b & 0x0F).collect();
    expect_eq("$D021 seen at $D061, $D0A1, $D3E1", low, vec![5; 3])
}

fn the_collision_registers_are_cleared_by_reading_them(env: &Env) -> Result<(), String> {
    // BAUER: $D01E (sprite-sprite) and $D01F (sprite-data) "are automatically cleared on reading". Two solid sprites on the same
    // spot, visible in the display window, collide: the first read of $D01E shows both, the second shows none.
    let source = format!(
        " sei
 lda #$2F
 sta $00
 lda #$35
 sta $01
 ldx #62
 lda #$FF
fill: sta $0340,x
 dex
 bpl fill
 lda #$14
 sta $D018
 lda #$0D
 sta $07F8
 sta $07F9
 lda #100
 sta $D000
 sta $D002
 sta $D001
 sta $D003
 lda #3
 sta $D015
 lda #$1B
 sta $D011
 ldy #$20
 ldx #0
wait: dex
 bne wait
 dey
 bne wait
 lda $D01E
 sta ${:04X}
 lda $D01E
 sta ${:04X}
here: jmp here
",
        RESULTS,
        RESULTS + 1
    );
    let out = run_program(env, "collision", &source, 4, &[(RESULTS, 2)], &[])?;
    expect_eq("first read of $D01E", out.byte(RESULTS)?, 0x03)?;
    expect_eq("second read of $D01E", out.byte(RESULTS + 1)?, 0x00)
}

fn keys_pull_their_matrix_line_low_on_port_b_when_their_column_is_selected(
    env: &Env,
) -> Result<(), String> {
    // C64WIKI-KEYBOARD: (key, column PA, row PB).
    let keys: [(&str, usize, u8); 10] = [
        ("Enter", 0, 1),
        ("KeyA", 1, 2),
        ("Space", 7, 4),
        ("Digit1", 7, 0),
        ("KeyQ", 7, 6),
        ("RunStop", 7, 7),
        ("ShiftLeft", 1, 7),
        ("ArrowRight", 0, 2),
        ("Delete", 0, 0),
        ("F1", 0, 4),
    ];
    // The program scans over and over: the key goes down at frame 3.
    let mut source = String::from(" sei\n lda #$FF\n sta $DC02\n lda #0\n sta $DC03\nscan:\n");
    for col in 0..8 {
        source += &format!(
            " lda #${:02X}\n sta $DC00\n nop\n nop\n lda $DC01\n sta ${:04X}\n",
            !(1u8 << col),
            RESULTS + col
        );
    }
    source += " jmp scan\n";
    for (key, col, row) in keys {
        let at = format!("3:key_down={key}");
        let out = run_program(env, "matrix", &source, 6, &[(RESULTS, 8)], &["--at", &at])?;
        let got = out.bytes(RESULTS, 8)?;
        let mut want = [0xFFu8; 8];
        want[col] = !(1 << row);
        expect_eq(
            &format!("port B for each column with {key} held"),
            got,
            want.to_vec(),
        )?;
    }
    Ok(())
}

fn joysticks_pull_bits_0_to_4_low_on_port_a_for_port_2_and_port_b_for_port_1(
    env: &Env,
) -> Result<(), String> {
    // C64WIKI-JOYSTICK: up, down, left, right, fire = bits 0-4, 0 when active; port 2 on $DC00, port 1 on $DC01. No keyboard
    // column is selected ($DC00 = $FF) so no key can pull a line.
    let source = format!(
        " sei\n lda #$FF\n sta $DC02\n lda #0\n sta $DC03\n lda #$FF\n sta $DC00\nscan: lda $DC00\n sta ${:04X}\n lda $DC01\n sta ${:04X}\n jmp scan\n",
        RESULTS,
        RESULTS + 1
    );
    let cases: &[(&[&str], u8, u8)] = &[
        (&[], 0x1F, 0x1F),
        (&["Joy2Up"], 0x1E, 0x1F),
        (&["Joy2Down"], 0x1D, 0x1F),
        (&["Joy2Left"], 0x1B, 0x1F),
        (&["Joy2Right"], 0x17, 0x1F),
        (&["Joy2Fire1"], 0x0F, 0x1F),
        (&["Joy1Up"], 0x1F, 0x1E),
        (&["Joy1Down"], 0x1F, 0x1D),
        (&["Joy1Left"], 0x1F, 0x1B),
        (&["Joy1Right"], 0x1F, 0x17),
        (&["Joy1Fire1"], 0x1F, 0x0F),
        (&["Joy2Up", "Joy2Left", "Joy1Down", "Joy1Fire1"], 0x1A, 0x0D),
    ];
    for (keys, port2, port1) in cases {
        let ats: Vec<String> = keys.iter().map(|k| format!("3:key_down={k}")).collect();
        let mut extra = Vec::new();
        for at in &ats {
            extra.push("--at");
            extra.push(at.as_str());
        }
        let out = run_program(env, "joystick", &source, 6, &[(RESULTS, 2)], &extra)?;
        expect_eq(
            &format!("$DC00 & $1F (port 2) with {keys:?}"),
            out.byte(RESULTS)? & 0x1F,
            *port2,
        )?;
        expect_eq(
            &format!("$DC01 & $1F (port 1) with {keys:?}"),
            out.byte(RESULTS + 1)? & 0x1F,
            *port1,
        )?;
    }
    Ok(())
}

fn reading_the_cia_interrupt_register_clears_its_flags(env: &Env) -> Result<(), String> {
    // C64WIKI-CIA: "flags will be cleared after reading"; bit 7 is set while an enabled source is flagged. Timer A underflows
    // (latch 10) with its interrupt enabled in the mask ($DC0D = $81), so the first read gives $81, the second $00; interrupts
    // stay off in the CPU (I set).
    let source = format!(
        " sei
 lda #$7F
 sta $DC0D
 lda $DC0D
 lda #10
 sta $DC04
 lda #0
 sta $DC05
 lda #$81
 sta $DC0D
 lda #$09
 sta $DC0E
 ldx #20
w: dex
 bne w
 lda $DC0D
 sta ${:04X}
 lda $DC0D
 sta ${:04X}
here: jmp here
",
        RESULTS,
        RESULTS + 1
    );
    let out = run_program(env, "cia_icr", &source, 3, &[(RESULTS, 2)], &[])?;
    expect_eq("first read of $DC0D", out.byte(RESULTS)?, 0x81)?;
    expect_eq("second read of $DC0D", out.byte(RESULTS + 1)?, 0x00)
}

const C64_SOURCE: &str =
    "BAUER, C64WIKI-KEYBOARD/-JOYSTICK/-CIA and CIA6526, as named in the file header";

const CASES: &[Case] = &[
    Case::new(
        Area::Timers,
        "a raster interrupt arrives in the line it was set for (1, $30, $80, $F8, 256, 311)",
        C64_SOURCE,
        a_raster_interrupt_arrives_in_the_line_it_was_set_for,
    ),
    Case::new(
        Area::Timers,
        "bad lines stun the processor for 40-43 cycles each: a frame with the display on leaves 1000-1075 fewer cycles",
        C64_SOURCE,
        bad_lines_stun_the_processor_for_40_to_43_cycles_each,
    )
    .gap("the VIC steals no cycles from the processor on bad lines: a frame costs the same with the display on and off (cycle-exact VIC access is the planned `vic_accuracy = cycle`, see TODO.md)"),
    Case::new(
        Area::Video,
        "a PAL frame has 312 lines, numbered from 0",
        C64_SOURCE,
        a_pal_frame_has_312_lines_numbered_from_0,
    ),
    Case::new(
        Area::Timers,
        "a CIA timer's period is its latch plus one cycle (timer B counting timer A's underflows)",
        C64_SOURCE,
        a_cia_timer_period_is_its_latch_plus_one_cycle,
    ),
    Case::new(
        Area::Timers,
        "a one-shot CIA timer stops at underflow and sets its interrupt flag",
        C64_SOURCE,
        a_one_shot_timer_stops_at_underflow_and_sets_its_flag,
    ),
    Case::new(
        Area::Io,
        "the unconnected bits of the VIC registers read as 1",
        C64_SOURCE,
        the_unconnected_bits_of_the_vic_registers_read_one,
    )
    .gap("the unconnected bits read back as written (0): $D020 after writing 0 reads $00 where BAUER's table gives $F0"),
    Case::new(
        Area::Io,
        "$D02F-$D03F read $FF and ignore writes",
        C64_SOURCE,
        the_unused_vic_addresses_read_ff_and_ignore_writes,
    )
    .gap("writes to the unused VIC addresses are stored and read back ($55), where BAUER says they read $FF and are ignored"),
    Case::new(
        Area::Io,
        "the VIC registers repeat every 64 bytes up to $D3FF",
        C64_SOURCE,
        the_vic_registers_repeat_every_64_bytes_up_to_d3ff,
    ),
    Case::new(
        Area::Io,
        "the sprite collision registers are cleared by reading them",
        C64_SOURCE,
        the_collision_registers_are_cleared_by_reading_them,
    ),
    Case::new(
        Area::Io,
        "keys pull their matrix line low when their column is selected (10 keys)",
        C64_SOURCE,
        keys_pull_their_matrix_line_low_on_port_b_when_their_column_is_selected,
    ),
    Case::new(
        Area::Io,
        "joysticks pull bits 0-4 low: port 2 on $DC00, port 1 on $DC01",
        C64_SOURCE,
        joysticks_pull_bits_0_to_4_low_on_port_a_for_port_2_and_port_b_for_port_1,
    ),
    Case::new(
        Area::Io,
        "reading the CIA interrupt register clears its flags",
        C64_SOURCE,
        reading_the_cia_interrupt_register_clears_its_flags,
    ),
];

#[test]
fn c64_hardware() {
    let Some(env) = Env::discover() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    if env.c64().is_err() {
        eprintln!("skipping: the C64 ROMs are not present");
        return;
    }
    let report = run_cases(&env, CASES);
    eprintln!("{}", report.summary());
    assert!(report.ok(), "{}", report.summary());
}
