//! ZX Spectrum I/O acid programs: small programs of our own that read and write ports and compare what comes back with what the
//! published documents say a real machine does.
//!
//! * `WOS48` = the World of Spectrum 48K reference, <https://worldofspectrum.org/faq/reference/48kreference.htm>: the ULA answers
//!   an `IN` whose port has bit 0 reset; bits 5 and 7 of what it returns are always one, bit 6 is the EAR input and the low five
//!   bits are the keys (a zero is a pressed key); bit 4 of an `OUT` is the loudspeaker.
//! * `Z80TEST` = Patrik Rak's z80test, run in `spectrum_z80test`: on a real 48K with no tape and no key an `IN` from 0xFE reads
//!   0xBF (the EAR bit reads low).
//! * `FLOATBUS` = <https://sinclair.wiki.zxnet.co.uk/wiki/Floating_bus>: a port nothing answers reads what the ULA bus carries,
//!   which is 0xFF whenever the picture is not being drawn.
//! * `KEMPSTON` = the Kempston Joystick Interface instructions (<https://spectrumcomputing.co.uk/pub/sinclair/hardware-info/k/
//!   KempstonJoystickInterface.pdf>): "port mapped to address 31"; central position reads 0; UP 8, DOWN 4, LEFT 2, RIGHT 1,
//!   "pressing either fire buttons adds 16".
//! * `WOS128` = the World of Spectrum 128K reference, <https://worldofspectrum.org/faq/reference/128kreference.htm>: `OUT 0xFFFD`
//!   selects an AY register, `IN 0xFFFD` reads the selected one and `OUT 0xBFFD` writes it; port 0x7FFD answers "any port address
//!   with bits 1 and 15 reset", and on the +2A/+3 port 0x1FFD "all port addresses with bit 1 reset, bit 12 set and bits 13, 14 and
//!   15 reset".
//!
//! Needs `RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS` and `RC_ROMS`; skips without.

use rc_suite::{Area, Case, Env, Outcome, expect_eq, run_cases, z80_program};

const ROM_48: &str = "sinclair/roms/zx-spectrum/spec48.rom";
const ROM_128: &str = "sinclair/roms/zx-spectrum/128.rom";
const ROM_PLUS3: &str = "sinclair/roms/zx-spectrum/plus3.rom";

#[allow(clippy::too_many_arguments)]
fn run(
    env: &Env,
    machine: &str,
    rom: &str,
    tag: &str,
    source: &str,
    frames: u32,
    dumps: &[(u16, u16)],
    extra: &[&str],
) -> Result<Outcome, String> {
    let file = z80_program(tag, 0x8000, source)?;
    let mut run = env
        .run(machine)
        .rom(&env.file(rom)?)
        .arg("--no-border")
        .arg("--program")
        .arg(file.to_string_lossy())
        .arg("--org")
        .arg("0x8000")
        .frames(frames);
    for e in extra {
        run = run.arg(*e);
    }
    for (start, len) in dumps {
        run = run.dump(*start, *len);
    }
    run.go()
}

/// `IN A,(C)` of each port in turn, stored from 0x9000, with interrupts off at the start of the frame (the picture is not drawn
/// yet for the first 14000 T-states, so nothing is contended or floating but 0xFF).
fn read_ports(env: &Env, ports: &[u16]) -> Result<Vec<u8>, String> {
    let mut source = String::from(" di\n");
    for (i, port) in ports.iter().enumerate() {
        source += &format!(" ld bc,${port:04X}\n in a,(c)\n ld ($9000+{i}),a\n");
    }
    source += "here: jr here\n";
    let out = run(
        env,
        "zx-spectrum",
        ROM_48,
        "ports",
        &source,
        2,
        &[(0x9000, ports.len() as u16)],
        &[],
    )?;
    out.bytes(0x9000, ports.len() as u16)
}

fn the_ula_answers_every_even_port_with_bf_and_nothing_answers_the_others(
    env: &Env,
) -> Result<(), String> {
    // High byte 0xFF selects no keyboard row, so no key can show. Ports with bit 5 reset are left out: the Kempston interface
    // (always fitted here) answers those as well.
    let even = [0xFFFE, 0xFFFC, 0xFF20, 0xFF3E, 0xFFA8, 0xFFEE, 0xFF7E];
    let seen = read_ports(env, &even)?;
    // WOS48: bits 5 and 7 read 1, the five key bits read 1 (no key); Z80TEST: the EAR bit reads 0 with no tape.
    expect_eq("even ports (ULA)", seen, vec![0xBF; even.len()])?;
    // Odd ports with bit 5 set (bit 5 clear is the Kempston joystick): nothing answers. FLOATBUS: 0xFF while the picture is not
    // being drawn.
    let odd = [0xFFFF, 0xFF21, 0xFFA1, 0xFF3F];
    let seen = read_ports(env, &odd)?;
    expect_eq("odd ports (nothing)", seen, vec![0xFF; odd.len()])
}

fn the_kempston_interface_reads_8_4_2_1_and_16_for_fire(env: &Env) -> Result<(), String> {
    // B = 0: the Kempston interface ignores the high byte, and the emulated mouse interface (always fitted) decodes it.
    let source = " ld bc,$001F
loop: in a,(c)
 ld ($9000),a
 jr loop
";
    // (keys held, byte read)
    let cases: &[(&[&str], u8)] = &[
        (&[], 0),
        (&["Joy1Up"], 8),
        (&["Joy1Down"], 4),
        (&["Joy1Left"], 2),
        (&["Joy1Right"], 1),
        (&["Joy1Fire1"], 16),
        (&["Joy1Up", "Joy1Left"], 10),
        (&["Joy1Up", "Joy1Right"], 9),
        (&["Joy1Down", "Joy1Left"], 6),
        (&["Joy1Down", "Joy1Right"], 5),
        (&["Joy1Up", "Joy1Fire1"], 24),
    ];
    for (keys, want) in cases {
        let mut extra = Vec::new();
        let ats: Vec<String> = keys.iter().map(|k| format!("2:key_down={k}")).collect();
        for at in &ats {
            extra.push("--at");
            extra.push(at);
        }
        let out = run(
            env,
            "zx-spectrum",
            ROM_48,
            "kempston",
            source,
            6,
            &[(0x9000, 1)],
            &extra,
        )?;
        expect_eq(&format!("IN 31 with {keys:?}"), out.byte(0x9000)?, *want)?;
    }
    Ok(())
}

fn the_ay_registers_read_back_what_was_written(env: &Env) -> Result<(), String> {
    // Registers whose eight bits are all stored: the tone periods' low bytes, the mixer and the envelope period.
    let registers: [(u8, u8); 6] = [
        (0, 0xA5),
        (2, 0x5A),
        (4, 0xC3),
        (7, 0x3F),
        (11, 0x96),
        (12, 0x69),
    ];
    let mut source = String::from(" di\n");
    for (i, (reg, value)) in registers.iter().enumerate() {
        source += &format!(
            " ld bc,$FFFD\n ld a,{reg}\n out (c),a\n ld bc,$BFFD\n ld a,${value:02X}\n out (c),a\n ld bc,$FFFD\n in a,(c)\n ld ($9000+{i}),a\n"
        );
    }
    source += "here: jr here\n";
    let out = run(
        env,
        "zx-spectrum-128",
        ROM_128,
        "ay",
        &source,
        2,
        &[(0x9000, registers.len() as u16)],
        &[],
    )?;
    expect_eq(
        "registers read back through 0xFFFD",
        out.bytes(0x9000, registers.len() as u16)?,
        registers.iter().map(|r| r.1).collect::<Vec<_>>(),
    )
}

/// Pages RAM bank `bank` in at 0xC000 through `port` (any address that decodes to 0x7FFD), writes `mark` there, then selects bank
/// 0 through 0x7FFD and reads 0xC000: if `port` paged, the byte just written is in another bank and 0xC000 shows bank 0's own.
fn paging_through(env: &Env, port: u16) -> Result<(u8, u8), String> {
    let source = format!(
        " di\n ld bc,$7FFD\n xor a\n out (c),a\n ld a,$11\n ld ($C000),a\n ld bc,${port:04X}\n ld a,3\n out (c),a\n ld a,$33\n ld ($C000),a\n ld bc,$7FFD\n xor a\n out (c),a\n ld a,($C000)\n ld ($9000),a\n ld bc,$7FFD\n ld a,3\n out (c),a\n ld a,($C000)\n ld ($9001),a\nhere: jr here\n"
    );
    let out = run(
        env,
        "zx-spectrum-128",
        ROM_128,
        "decode7ffd",
        &source,
        2,
        &[(0x9000, 2)],
        &[],
    )?;
    Ok((out.byte(0x9000)?, out.byte(0x9001)?))
}

fn port_0x7ffd_answers_any_address_with_bits_1_and_15_reset(env: &Env) -> Result<(), String> {
    // Bank 3 was selected through `port` and written 0x33; bank 0 holds 0x11. Reading bank 0 gives 0x11 and bank 3 gives 0x33
    // exactly when `port` selected it.
    for port in [0x7FFD, 0x3FFD, 0x7FF9, 0x4000, 0x0001] {
        expect_eq(
            &format!("banks 0 and 3 after paging through {port:04X}"),
            paging_through(env, port)?,
            (0x11, 0x33),
        )?;
    }
    // Bit 1 set, or bit 15 set: not the paging port; the write at 0xC000 went to bank 0 and bank 3 keeps its own byte.
    for port in [0x7FFF, 0xFFFD ^ 0x8000 | 0x0002, 0x7FFE] {
        let (bank0, bank3) = paging_through(env, port)?;
        expect_eq(
            &format!("bank 0 after a write meant for bank 3 through {port:04X}"),
            bank0,
            0x33,
        )?;
        let _ = bank3;
    }
    Ok(())
}

fn port_0x1ffd_answers_the_addresses_with_bit_12_set_and_13_to_15_and_1_reset(
    env: &Env,
) -> Result<(), String> {
    // Special paging (bit 0 of 0x1FFD) turns 0x0000 into RAM: a write there sticks. Through the decoded port it must; through an
    // address with bit 13 set it must not.
    let probe = |port: u16| -> Result<u8, String> {
        let source = format!(
            " di\n ld bc,${port:04X}\n ld a,1\n out (c),a\n ld a,$5A\n ld ($0100),a\n ld a,($0100)\n ld ($9000),a\nhere: jr here\n"
        );
        let out = run(
            env,
            "zx-spectrum-2a3",
            ROM_PLUS3,
            "decode1ffd",
            &source,
            2,
            &[(0x9000, 1)],
            &[],
        )?;
        out.byte(0x9000)
    };
    for port in [0x1FFD, 0x1FFC, 0x1000] {
        expect_eq(
            &format!("RAM at 0x0100 after OUT {port:04X},1"),
            probe(port)?,
            0x5A,
        )?;
    }
    for port in [0x3FFD, 0x1FFF] {
        let seen = probe(port)?;
        if seen == 0x5A {
            return Err(format!(
                "OUT {port:04X},1 switched special paging on: it is not the 0x1FFD port"
            ));
        }
    }
    Ok(())
}

fn bit_4_of_port_0xfe_drives_the_loudspeaker(env: &Env) -> Result<(), String> {
    // Twenty 400 T-state periods on bit 4 (3.5 MHz / 400 = 8750 Hz) in the bottom border, from T-state 58000 on (the picture ended
    // at 57343, so nothing is contended and the periods are exact): OUT (C),D = 12 T, filler to 200, OUT (C),E = 12 T, filler to
    // 400. A loudspeaker that follows bit 4 gives twenty rising edges in the frame's audio, 400 T = 5.04 samples apart.
    fn exact(t: usize) -> String {
        let mut sevens = 0;
        while !(t - 7 * sevens).is_multiple_of(4) {
            sevens += 1;
        }
        " ld b,0
"
        .repeat(sevens)
            + &" nop
"
            .repeat((t - 7 * sevens) / 4)
    }
    let mut source = format!(
        " di
 ld bc,$00FE
 ld d,$10
 ld e,0
{}",
        exact(58000 - 4 - 24)
    );
    for _ in 0..20 {
        source += &format!(
            " out (c),d
{} out (c),e
{}",
            exact(188),
            exact(188)
        );
    }
    source += "here: jr here
";
    let file = z80_program("beeper", 0x8000, &source)?;
    let out = env
        .run("zx-spectrum")
        .rom(&env.file(ROM_48)?)
        .arg("--no-border")
        .arg("--program")
        .arg(file.to_string_lossy())
        .arg("--org")
        .arg("0x8000")
        .frames(1)
        .audio()
        .go()?;
    let samples = out.audio()?.window(0.0, 1.0);
    let top = samples.iter().cloned().fold(f32::MIN, f32::max);
    if top <= 0.0 {
        return Err("the loudspeaker made no sound at all".to_string());
    }
    let level = top / 2.0;
    let edges: Vec<usize> = (1..samples.len())
        .filter(|&i| samples[i - 1] < level && samples[i] >= level)
        .collect();
    expect_eq("rising edges in the frame", edges.len(), 20)?;
    let spacing = (edges[19] - edges[0]) as f64 / 19.0;
    if !(4.9..=5.2).contains(&spacing) {
        return Err(format!(
            "edges {spacing:.3} samples apart, expected 44100 * 400 / 3500000 = 5.04"
        ));
    }
    Ok(())
}

const IO_SOURCE: &str =
    "WOS48, WOS128, FLOATBUS, KEMPSTON and Z80TEST, as named in the file header";

const CASES: &[Case] = &[
    Case::new(
        Area::Io,
        "the ULA answers every even port with 0xBF (no key, no tape) and nothing answers the odd ones",
        IO_SOURCE,
        the_ula_answers_every_even_port_with_bf_and_nothing_answers_the_others,
    ),
    Case::new(
        Area::Io,
        "the Kempston interface reads UP 8, DOWN 4, LEFT 2, RIGHT 1 and fire adds 16",
        IO_SOURCE,
        the_kempston_interface_reads_8_4_2_1_and_16_for_fire,
    ),
    Case::new(
        Area::Io,
        "the AY registers read back what was written through 0xFFFD / 0xBFFD",
        IO_SOURCE,
        the_ay_registers_read_back_what_was_written,
    ),
    Case::new(
        Area::Io,
        "port 0x7FFD answers any address with bits 1 and 15 reset",
        IO_SOURCE,
        port_0x7ffd_answers_any_address_with_bits_1_and_15_reset,
    ),
    Case::new(
        Area::Io,
        "port 0x1FFD answers the addresses with bit 12 set and bits 13-15 and 1 reset",
        IO_SOURCE,
        port_0x1ffd_answers_the_addresses_with_bit_12_set_and_13_to_15_and_1_reset,
    ),
    Case::new(
        Area::Sound,
        "bit 4 of port 0xFE drives the loudspeaker: a 400 T-state wave is heard at 8750 Hz",
        IO_SOURCE,
        bit_4_of_port_0xfe_drives_the_loudspeaker,
    ),
];

#[test]
fn spectrum_io() {
    let Some(env) = Env::discover() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    if [ROM_48, ROM_128, ROM_PLUS3]
        .iter()
        .any(|r| env.file(r).is_err())
    {
        eprintln!("skipping: the Spectrum ROMs are not present");
        return;
    }
    let report = run_cases(&env, CASES);
    eprintln!("{}", report.summary());
    assert!(report.ok(), "{}", report.summary());
}
