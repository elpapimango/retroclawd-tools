//! ZX Spectrum timing acid programs: small programs of our own whose result is a T-state count, judged against the published
//! contention rules.
//!
//! A program starts at T-state 0 of the frame (the machine is not booted, interrupts are off), runs a calibrated number of
//! filler instructions so that one memory or port access starts at a chosen T-state of the frame, and halts. The run reports the
//! cycles it used; the same program with its access moved to uncontended memory (or an uncontended port) gives the cycles
//! without delay, and the difference is the delay the ULA imposed at that T-state.
//!
//! The expected delay is computed from the rules as the documents state them (`delay` below):
//!
//! * `WOS48` = the World of Spectrum 48K reference, <https://worldofspectrum.org/faq/reference/48kreference.htm>: contended memory
//!   is 0x4000-0x7FFF; "at cycle 14335 ... the delay is 6 cycles", then 5, 4, 3, 2, 1 for 14336-14340, none for 14341 and 14342,
//!   and the pattern restarts at 14343; it repeats on each of the 192 screen lines of 224 T-states, 128 of them with the screen
//!   ("until the cycle 14463 ... the electron beam reaches the border again"); "while the ULA is updating the border the delay
//!   does not happen"; and its table of I/O contention (high byte in 0x40-0x7F or not, low bit reset or set).
//! * `WOS48` also: "for the 128k/+2 models the contention sequence starts at cycle 14361"; a 128K line is 228 T-states.
//! * `FLOATBUS` = the Sinclair wiki's "Floating bus" page, <https://sinclair.wiki.zxnet.co.uk/wiki/Floating_bus>: while the ULA
//!   draws the picture its bus carries, in the first four T-states of every group of eight, the bitmap byte, the attribute byte,
//!   the next bitmap byte and the next attribute byte (from T-state 14338 on the 48K, 128 T-states a line), and 0xFF the rest of
//!   the time; a port nothing answers reads what the bus carries. "These timings will all be one T-state later on late-timing
//!   machines".
//! * `ZILOG` = the Z80 instruction timing tables: `LD A,(nn)` is 13 T (the data is read in its last 3), `IN r,(C)` and
//!   `OUT (C),r` are 12 T (the port cycle is the last 4), `HALT` is 4 T.
//!
//! "Some machines use the timings given in this FAQ, while others are one T state later" (WOS48): the emulator is an early-timing
//! machine, as the document's own numbers are.
//!
//! Needs `RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS` and `RC_ROMS`; skips without.

use rc_suite::{Area, Case, Env, expect_eq, run_cases, z80_program};

const ROM_48: &str = "sinclair/roms/zx-spectrum/spec48.rom";
const ROM_128: &str = "sinclair/roms/zx-spectrum/128.rom";

/// The contention rule of one machine: the T-state of the first contended cycle, the length of a line, and how many T-states of
/// each line (from the first) the ULA is busy.
struct Ula {
    first: i64,
    line: i64,
    busy: i64,
}

const ULA_48: Ula = Ula {
    first: 14335,
    line: 224,
    busy: 128,
};
const ULA_128: Ula = Ula {
    first: 14361,
    line: 228,
    busy: 128,
};

/// WOS48: the delay of a contended access that starts at T-state `t` of the frame.
fn delay(ula: &Ula, t: i64) -> i64 {
    let rel = t - ula.first;
    if rel < 0 {
        return 0;
    }
    let (row, col) = (rel / ula.line, rel % ula.line);
    if row >= 192 || col >= ula.busy {
        return 0;
    }
    [6, 5, 4, 3, 2, 1, 0, 0][(col % 8) as usize]
}

/// Exactly `t` T-states of filler: NOP (4) and `LD D,0` (7) -- D, because the port programs keep their port in BC.
fn pad(t: i64) -> String {
    assert!(t >= 0, "negative padding");
    let mut sevens = 0;
    while (t - 7 * sevens) % 4 != 0 {
        sevens += 1;
        assert!(7 * sevens <= t, "cannot pad {t} T-states");
    }
    " ld d,0\n".repeat(sevens as usize) + &" nop\n".repeat(((t - 7 * sevens) / 4) as usize)
}

fn cycles_of(env: &Env, machine: &str, rom: &str, tag: &str, source: &str) -> Result<i64, String> {
    let file = z80_program(tag, 0x8000, source)?;
    let out = env
        .run(machine)
        .rom(&env.file(rom)?)
        .arg("--no-border")
        .arg("--program")
        .arg(file.to_string_lossy())
        .arg("--org")
        .arg("0x8000")
        .go()?;
    Ok(out.cycles as i64)
}

/// `DI`, `setup` (of `setup_t` T-states), filler, then `access` whose contended cycle starts `offset` T-states into it at
/// frame T-state `at`, then `HALT`.
fn program(setup: &str, setup_t: i64, access: &str, offset: i64, at: i64) -> String {
    let start = at - offset; // frame T-state at which the access instruction starts
    format!(" di\n{setup}{}{access} halt\n", pad(start - 4 - setup_t))
}

/// The delay the emulator imposes on `access` when it is moved from `fast` (uncontended) to `slow` (contended).
#[allow(clippy::too_many_arguments)]
fn measured(
    env: &Env,
    machine: &str,
    rom: &str,
    tag: &str,
    setup: (&str, i64),
    offset: i64,
    at: i64,
    slow: &str,
    fast: &str,
) -> Result<i64, String> {
    let a = cycles_of(
        env,
        machine,
        rom,
        tag,
        &program(setup.0, setup.1, slow, offset, at),
    )?;
    let b = cycles_of(
        env,
        machine,
        rom,
        tag,
        &program(setup.0, setup.1, fast, offset, at),
    )?;
    Ok(a - b)
}

/// A memory read: `LD A,(nn)` reads its data in the last 3 of its 13 T-states, 10 T-states in.
fn memory_delay(
    env: &Env,
    machine: &str,
    rom: &str,
    ula: &Ula,
    at: i64,
) -> Result<(i64, i64), String> {
    let got = measured(
        env,
        machine,
        rom,
        "contention",
        ("", 0),
        10,
        at,
        " ld a,($4000)\n",
        " ld a,($8000)\n",
    )?;
    Ok((got, delay(ula, at)))
}

fn the_contention_delay_runs_6_5_4_3_2_1_0_0_from_cycle_14335(env: &Env) -> Result<(), String> {
    for at in 14325..14352 {
        let (got, want) = memory_delay(env, "zx-spectrum", ROM_48, &ULA_48, at)?;
        expect_eq(&format!("delay of a read at T-state {at}"), got, want)?;
    }
    Ok(())
}

fn the_pattern_repeats_on_each_screen_line(env: &Env) -> Result<(), String> {
    for line in [1i64, 2, 63, 100, 191] {
        for phase in 0..8 {
            let at = 14335 + line * 224 + phase;
            let (got, want) = memory_delay(env, "zx-spectrum", ROM_48, &ULA_48, at)?;
            expect_eq(
                &format!("delay at T-state {at} (line {line}, +{phase})"),
                got,
                want,
            )?;
        }
    }
    Ok(())
}

fn there_is_no_delay_in_the_border_or_the_retrace(env: &Env) -> Result<(), String> {
    // Before the picture, in the right border and retrace of a line (from 14463), and after the last of the 192 lines.
    let points = [
        100,
        5000,
        14334,
        14335 + 128,
        14335 + 128 + 5,
        14335 + 223,
        14335 + 191 * 224 + 128,
        14335 + 191 * 224 + 200,
        14335 + 192 * 224,
        14335 + 192 * 224 + 1,
        14335 + 192 * 224 + 8,
        60000,
        69000,
    ];
    for at in points {
        let (got, _) = memory_delay(env, "zx-spectrum", ROM_48, &ULA_48, at)?;
        expect_eq(&format!("delay at T-state {at}"), got, 0)?;
    }
    Ok(())
}

fn the_128k_contention_starts_at_cycle_14361_and_its_line_is_228(env: &Env) -> Result<(), String> {
    for at in [
        14355,
        14360,
        14361,
        14362,
        14366,
        14367,
        14368,
        14369,
        14370,
        14361 + 228,
        14361 + 228 * 100 + 3,
    ] {
        let (got, want) = memory_delay(env, "zx-spectrum-128", ROM_128, &ULA_128, at)?;
        expect_eq(&format!("128K delay at T-state {at}"), got, want)?;
    }
    Ok(())
}

/// WOS48's table, as the sequence of (contended?, T-states) the port access goes through; each contended step waits for the ULA
/// and then takes its T-states.
fn port_delay(ula: &Ula, start: i64, high_in_40_7f: bool, low_bit_set: bool) -> i64 {
    let steps: &[(bool, i64)] = match (high_in_40_7f, low_bit_set) {
        (false, false) => &[(false, 1), (true, 3)],
        (false, true) => &[(false, 4)],
        (true, false) => &[(true, 1), (true, 3)],
        (true, true) => &[(true, 1), (true, 1), (true, 1), (true, 1)],
    };
    let (mut t, mut total) = (start, 0);
    for &(contended, len) in steps {
        let d = if contended { delay(ula, t) } else { 0 };
        total += d;
        t += d + len;
    }
    total
}

/// `cases`: (BC, high byte in 0x40-0x7F, low bit set). 0xFE is the ULA's own port; 0x..FF is not answered by anything.
fn port_cases(env: &Env, instruction: &str, cases: &[(u16, bool, bool)]) -> Result<(), String> {
    for &(bc, high, low) in cases {
        for phase in 0..8 {
            // the port cycle is the last 4 of the 12 T-states, 8 in
            let at = 14335 + phase;
            let total = cycles_of(
                env,
                "zx-spectrum",
                ROM_48,
                "port",
                &program(
                    &format!(
                        " ld bc,${bc:04X}
"
                    ),
                    10,
                    instruction,
                    8,
                    at,
                ),
            )?;
            // di 4 + setup 10 + filler + instruction 12 + halt 4 = at - 8 + 12 + 4 + delay
            let want = at - 8 + 12 + 4 + port_delay(&ULA_48, at, high, low);
            expect_eq(
                &format!(
                    "delay of {instruction:?} with BC={bc:04X} reaching its port cycle at T-state {at}"
                ),
                total - (at - 8 + 12 + 4),
                want - (at - 8 + 12 + 4),
            )?;
        }
    }
    Ok(())
}

const OUTSIDE_40_7F: &[(u16, bool, bool)] = &[(0xFFFE, false, false), (0xFFFF, false, true)];
const INSIDE_40_7F: &[(u16, bool, bool)] = &[(0x40FE, true, false), (0x7FFF, true, true)];

fn in_outside_40_7f(env: &Env) -> Result<(), String> {
    port_cases(
        env,
        " in a,(c)
",
        OUTSIDE_40_7F,
    )
}
fn out_outside_40_7f(env: &Env) -> Result<(), String> {
    port_cases(
        env,
        " out (c),a
",
        OUTSIDE_40_7F,
    )
}
fn in_inside_40_7f(env: &Env) -> Result<(), String> {
    port_cases(
        env,
        " in a,(c)
",
        INSIDE_40_7F,
    )
}
fn out_inside_40_7f(env: &Env) -> Result<(), String> {
    port_cases(
        env,
        " out (c),a
",
        INSIDE_40_7F,
    )
}

/// What the floating bus carries when the CPU samples it at frame T-state `t` of the first screen line (FLOATBUS). `bitmap(i)` and
/// `attr(i)` are the bytes at 0x4000+i and 0x5800+i.
fn floating_bus(t: i64, bitmap: impl Fn(i64) -> u8, attr: impl Fn(i64) -> u8) -> u8 {
    let rel = t - 14338;
    if !(0..128).contains(&rel) {
        return 0xFF;
    }
    let (group, phase) = (rel / 8, rel % 8);
    match phase {
        0 => bitmap(2 * group),
        1 => attr(2 * group),
        2 => bitmap(2 * group + 1),
        3 => attr(2 * group + 1),
        _ => 0xFF,
    }
}

/// Bytes of the screen the floating-bus programs show: 16 different bitmap bytes and 16 different attribute bytes.
fn bitmap(i: i64) -> u8 {
    0x10 + i as u8
}
fn attr(i: i64) -> u8 {
    0x80 + i as u8
}

/// What `IN A,(0xFF)` reads when it starts at frame T-state `at`: the screen bytes above are written before the picture starts
/// (T-states before 14335 are uncontended, so the filler is exact), then filler up to `at`.
fn floating_read(env: &Env, at: i64) -> Result<u8, String> {
    let mut init = String::new();
    for i in 0..16 {
        init += &format!(
            " ld a,${:02X}
 ld ($4000+{i}),a
 ld a,${:02X}
 ld ($5800+{i}),a
",
            bitmap(i),
            attr(i)
        );
    }
    let init_t = 16 * 2 * (7 + 13);
    // The 16K of the picture is written at the top of the program only for the first line; anything after `at` > 14335 + 224
    // reads other bytes of the screen (zero), which is not what the model looks at.
    let setup_t = init_t + 7;
    if at - 4 - setup_t < 0 {
        return Err(format!("T-state {at} is too early for the setup"));
    }
    let source = format!(
        " di
{init} ld a,$FF
{} in a,($FF)
 ld ($9000),a
 halt
",
        pad(at - 4 - setup_t)
    );
    let file = z80_program("floating", 0x8000, &source)?;
    let out = env
        .run("zx-spectrum")
        .rom(&env.file(ROM_48)?)
        .arg("--no-border")
        .arg("--program")
        .arg(file.to_string_lossy())
        .arg("--org")
        .arg("0x8000")
        .dump(0x9000, 1)
        .go()?;
    out.byte(0x9000)
}

fn the_floating_bus_carries_bitmap_attribute_bitmap_attribute_then_ff(
    env: &Env,
) -> Result<(), String> {
    // FLOATBUS: "the Z80 samples the data bus during the final T-state of the I/O machine cycle". `IN A,(n)` is 11 T (4 + 3 + 4),
    // so an instruction that starts at T-state `at` samples at `at + 10`.
    let starts: Vec<i64> = (14330..14330 + 40).collect();
    let seen: Vec<u8> = starts
        .iter()
        .map(|&at| floating_read(env, at))
        .collect::<Result<_, _>>()?;
    let explained_by = |k: i64| {
        starts
            .iter()
            .zip(&seen)
            .all(|(&at, &got)| got == floating_bus(at + k, bitmap, attr))
    };
    if explained_by(10) {
        Ok(())
    } else if explained_by(11) {
        Err("the bytes come in the documented order and period, but one T-state late: they match a sample one T-state after the I/O cycle's final one".to_string())
    } else {
        Err(format!(
            "the bytes read at starts {}..{} match no sample point: {seen:02X?}",
            starts[0],
            starts[starts.len() - 1]
        ))
    }
}

fn the_floating_bus_is_ff_in_the_border_and_the_retrace(env: &Env) -> Result<(), String> {
    for at in [
        3000i64,
        14000,
        14335 + 128 + 10,
        14335 + 100 * 224 + 140,
        14335 + 192 * 224 + 20,
        62000,
    ] {
        expect_eq(
            &format!("byte read at T-state {at}"),
            floating_read(env, at)?,
            0xFF,
        )?;
    }
    Ok(())
}

const ZXT_SOURCE: &str =
    "WOS48 contention rules and the Z80 timing tables, measured as cycles from T-state 0";

const CASES: &[Case] = &[
    Case::new(
        Area::Timers,
        "contended memory: the delay is 6,5,4,3,2,1,0,0 from cycle 14335",
        ZXT_SOURCE,
        the_contention_delay_runs_6_5_4_3_2_1_0_0_from_cycle_14335,
    ),
    Case::new(
        Area::Timers,
        "contended memory: the pattern repeats on each of the 192 lines of 224 T-states",
        ZXT_SOURCE,
        the_pattern_repeats_on_each_screen_line,
    ),
    Case::new(
        Area::Timers,
        "no delay before the picture, in the border and retrace, or after the last line",
        ZXT_SOURCE,
        there_is_no_delay_in_the_border_or_the_retrace,
    ),
    Case::new(
        Area::Timers,
        "the 128K contends from cycle 14361 with 228 T-state lines",
        ZXT_SOURCE,
        the_128k_contention_starts_at_cycle_14361_and_its_line_is_228,
    ),
    Case::new(
        Area::Io,
        "the floating bus carries bitmap, attribute, bitmap+1, attribute+1 and then idles at 0xFF while the picture is drawn",
        ZXT_SOURCE,
        the_floating_bus_carries_bitmap_attribute_bitmap_attribute_then_ff,
    ),
    Case::new(
        Area::Io,
        "the floating bus reads 0xFF in the border and the retrace",
        ZXT_SOURCE,
        the_floating_bus_is_ff_in_the_border_and_the_retrace,
    ),
    Case::new(
        Area::Io,
        "IN (C): a port with the high byte outside 0x40-0x7F is delayed as in WOS48's table (ULA port: N:1,C:3; other: N:4)",
        ZXT_SOURCE,
        in_outside_40_7f,
    ),
    Case::new(
        Area::Io,
        "OUT (C): a port with the high byte outside 0x40-0x7F is delayed as in WOS48's table",
        ZXT_SOURCE,
        out_outside_40_7f,
    ),
    Case::new(
        Area::Io,
        "IN (C): a port with the high byte in 0x40-0x7F is treated as contended memory (C:1,C:3 / C:1 x4)",
        ZXT_SOURCE,
        in_inside_40_7f,
    ),
    Case::new(
        Area::Io,
        "OUT (C): a port with the high byte in 0x40-0x7F is treated as contended memory",
        ZXT_SOURCE,
        out_inside_40_7f,
    ),
];

#[test]
fn spectrum_timing() {
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
