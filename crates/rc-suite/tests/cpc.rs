//! Amstrad CPC suite: what the real machine does, checked against the emulator.
//!
//! The expected values are hardware facts, each named in its case:
//!
//! * `GATE ARRAY` = the Gate Array documentation: Amstrad CPC Gate-Array
//!   (<https://cpctech.cpcwiki.de/docs/garray.html>) and CPCWiki "Gate Array"
//!   (<https://www.cpcwiki.eu/index.php/Gate_Array>): the interrupt counter (52 HSYNCs), the ROM enable bits of
//!   the mode register (`0` enables, `1` disables).
//! * `Z80 for the CPC` = Chibi Akumas, "Z80 Assembly programming for the Amstrad CPC"
//!   (<https://www.chibiakumas.com/z80/AmstradCPC.php>): the RAM configuration table C0-C7, the CRTC default
//!   register values, "interrupts occur 6x per frame".
//! * `CRTC` = the standard CPC display registers R0=63 (64 characters of 1 us = a 64 us line), R4=38, R9=7.
//!
//! The programs run with `--program`, so the firmware is not booted and the CRTC is not programmed: a case that
//! needs the video timing sets the standard registers itself, as any CPC program does.
//!
//! Needs `RETROCLAWD_BIN`, `RETROCLAWD_MACHINE_PLUGIN_DIRS` and `RC_ROMS` (see the crate docs); skips without.

use rc_suite::{
    Area, Case, Env, Outcome, Resident, expect_eq, expect_in, run_cases, z80_image, z80_program,
};
use std::path::PathBuf;

const ROM_FILE: &str = "amstrad/roms/cpc/cpc6128.rom";

fn rom(env: &Env) -> Result<PathBuf, String> {
    env.file(ROM_FILE)
}

/// Runs `source` loaded at `org` on a CPC6128 for `frames` frames, dumping `dumps`.
fn run(
    env: &Env,
    tag: &str,
    org: u16,
    source: &str,
    frames: u32,
    dumps: &[(u16, u16)],
) -> Result<Outcome, String> {
    let file = z80_program(tag, org, source)?;
    let mut run = env
        .run("cpc6128")
        .rom(&rom(env)?)
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

/// Programs the standard CPC display registers R0-R9 (CRTC defaults from `Z80 for the CPC`); it ends with the data table
/// label `crtc`, so a program that includes it puts `crtc_init` before its own code and the table after.
const CRTC_INIT: &str = "
 ld hl,crtc
 ld d,0
 ld c,0
crtc_next: ld b,$BC
 out (c),d
 ld b,$BD
 ld a,(hl)
 out (c),a
 inc hl
 inc d
 ld a,d
 cp 10
 jr nz,crtc_next
";
const CRTC_TABLE: &str = "crtc: .byte 63,40,46,$8E,38,0,25,30,0,7\n";

fn instructions_take_whole_microseconds(env: &Env) -> Result<(), String> {
    const FRAMES: u32 = 10;
    let out = run(
        env,
        "nops",
        0x4000,
        "di\n ld hl,0\nloop: inc hl\n jr loop\n",
        FRAMES,
        &[],
    )?;
    // GATE ARRAY: the Gate Array holds the Z80 so that every instruction takes a whole number of 1 us NOPs (4 T).
    // INC HL is 6 T on a plain Z80 and 8 T here, JR taken is 12 T and LD HL,nn is 10 T and 12 T here, so after
    // LD HL,0 each pass of the loop is 20 T.
    let hl = i64::from(out.reg("HL")?);
    let cycles = out.cycles as i64;
    let want = (cycles - 12) / 20;
    expect_in("HL after the loop", hl, want - 1, want + 1)
}

fn gate_array_interrupts_every_52_lines(env: &Env) -> Result<(), String> {
    const COUNT: u16 = 0x4F00;
    let source = format!(
        "di
 ld bc,$7F8D
 out (c),c
{CRTC_INIT} ld a,$C3
 ld ($0038),a
 ld hl,handler
 ld ($0039),hl
 ld hl,0
 ld ({COUNT}),hl
 im 1
 ei
wait: jr wait
handler: push hl
 ld hl,({COUNT})
 inc hl
 ld ({COUNT}),hl
 pop hl
 ei
 reti
{CRTC_TABLE}"
    );
    let at = |frames| run(env, "irq", 0x4000, &source, frames, &[(COUNT, 2)]);
    let a = at(50)?;
    let b = at(150)?;
    let interrupts = i64::from(b.word(COUNT)?) - i64::from(a.word(COUNT)?);
    let cycles = b.cycles as i64 - a.cycles as i64;
    // GATE ARRAY: the Gate Array raises /INT every 52 HSYNCs; CRTC: R0=63 makes a line 64 us, so 52 lines are
    // 3.328 ms = 13312 T at 4 MHz (6 interrupts per 312-line frame, 300 a second).
    expect_in(
        "interrupts counted between 50 and 150 frames",
        interrupts,
        1,
        100_000,
    )?;
    let period = cycles / interrupts;
    expect_in("T-states per interrupt", period, 13_312 - 66, 13_312 + 66)
}

/// The C0-C7 RAM configurations of the 6128: which of its eight 16K RAM blocks each 16K window shows.
const RAM_CONFIGS: [[u8; 4]; 8] = [
    [0, 1, 2, 3], // C0
    [0, 1, 2, 7], // C1
    [4, 5, 6, 7], // C2
    [0, 3, 2, 7], // C3
    [0, 4, 2, 3], // C4
    [0, 5, 2, 3], // C5
    [0, 6, 2, 3], // C6
    [0, 7, 2, 3], // C7
];

fn ram_configurations_c0_to_c7(env: &Env) -> Result<(), String> {
    // Every block gets a marker (its own number) at offset 0x100. Blocks 0-3 are written in C0, blocks 4-7 through
    // the 0x4000 window of C4-C7. The code runs at 0x8200, which is block 2 in every configuration but C2, so its
    // first 1K (the program is shorter) is copied into block 6 (the 0x4000 window of C6) first: in C2 the
    // 0x8000 window is block 6.
    let mut source = String::from(
        "start: di
 ld bc,$7F8D
 out (c),c
 ld bc,$7FC0
 out (c),c
 xor a
 ld ($0100),a
 ld a,1
 ld ($4100),a
 ld a,2
 ld ($8100),a
 ld a,3
 ld ($C100),a
",
    );
    for block in 4..8 {
        source += &format!(" ld bc,$7FC{block}\n out (c),c\n ld a,{block}\n ld ($4100),a\n");
    }
    source += " ld bc,$7FC6
 out (c),c
 ld hl,start
 ld de,$4200
 ld bc,$0400
 ldir
 ld bc,$7FC0
 out (c),c
";
    for n in 0..8 {
        source += &format!(
            " ld bc,$7FC{n}
 out (c),c
 ld a,($0100)
 ld l,a
 ld a,($4100)
 ld h,a
 ld a,($8100)
 ld e,a
 ld a,($C100)
 ld d,a
 ld bc,$7FC0
 out (c),c
 ld ({addr}),hl
 ld ({addr2}),de
",
            addr = 0x9000 + 4 * n,
            addr2 = 0x9002 + 4 * n
        );
    }
    source += "here: jr here\nprog_end:\n";
    let out = run(env, "banks", 0x8200, &source, 2, &[(0x9000, 32)])?;
    // Z80 for the CPC: the C0-C7 table.
    for (n, want) in RAM_CONFIGS.iter().enumerate() {
        let got = out.bytes(0x9000 + 4 * n as u16, 4)?;
        expect_eq(&format!("blocks seen in C{n}"), got, want.to_vec())?;
    }
    Ok(())
}

fn the_lower_rom_shows_at_reset_and_writes_go_to_the_ram_under_it(env: &Env) -> Result<(), String> {
    let out = run(
        env,
        "lowrom",
        0x4000,
        "di
 ld hl,$0000
 ld de,$9100
 ld bc,16
 ldir
 ld a,$5A
 ld ($0000),a
 ld a,($0000)
 ld ($9110),a
 ld bc,$7F8D
 out (c),c
 ld a,($0000)
 ld ($9111),a
here: jr here
",
        2,
        &[(0x9100, 18)],
    )?;
    // GATE ARRAY: at reset the lower ROM (0x0000-0x3FFF, bit 2 of the mode register = 0) answers reads; a write
    // always goes to the RAM below it; setting bit 2 uncovers that RAM.
    let file = std::fs::read(rom(env)?).map_err(|e| e.to_string())?;
    expect_eq(
        "the first 16 bytes read with the ROM on",
        out.bytes(0x9100, 16)?,
        file[..16].to_vec(),
    )?;
    expect_eq(
        "a read after a write, ROM still on",
        out.byte(0x9110)?,
        file[0],
    )?;
    expect_eq("a read with the lower ROM off", out.byte(0x9111)?, 0x5A)
}

/// Where a firmware-resident program sits: free BASIC memory, above the program area and below HIMEM.
const RESIDENT_AT: u16 = 0x8000;
/// Where such a program leaves its results.
const RESULT: u16 = 0x9F10;

/// A program that opens `name` through the firmware (AMSDOS: `CAS IN OPEN` 0xBC77, `CAS IN DIRECT` 0xBC83,
/// `CAS IN CLOSE` 0xBC7A), loads the whole file to `dest` and leaves at `RESULT`: the file type, a success flag, then
/// the load address and the length the header gives.
fn amsdos_read(name: &str, dest: u16) -> String {
    format!(
        " ld hl,fname
 ld b,{len}
 ld de,$8800
 call $BC77
 ld ({RESULT}),a
 ld ({load}),de
 ld ({length}),bc
 jr nc,done
 ld hl,{dest}
 call $BC83
 call $BC7A
 ld a,1
 ld ({ok}),a
done: jr done
fname: .text \"{name}\"
",
        len = name.len(),
        ok = RESULT + 1,
        load = RESULT + 2,
        length = RESULT + 4,
    )
}

/// What the firmware made of a file on a disk.
struct Loaded {
    file_type: u8,
    ok: bool,
    load: u16,
    length: u16,
    bytes: Vec<u8>,
}

/// Boots the firmware with `disk` in drive A and loads `name` to 0x4000 through AMSDOS.
fn amsdos_load(
    env: &Env,
    tag: &str,
    disk: &std::path::Path,
    name: &str,
    len: usize,
) -> Result<Loaded, String> {
    let resident = Resident {
        machine: "cpc6128",
        rom: &rom(env)?,
        extra: vec!["--disk".to_string(), disk.to_string_lossy().into_owned()],
        sna_version: None,
    };
    let out = resident.run(
        env,
        tag,
        RESIDENT_AT,
        &amsdos_read(name, 0x4000),
        250,
        &[(RESULT, 6), (0x4000, len as u16)],
    )?;
    Ok(Loaded {
        file_type: out.byte(RESULT)?,
        ok: out.byte(RESULT + 1)? == 1,
        load: out.word(RESULT + 2)?,
        length: out.word(RESULT + 4)?,
        bytes: out.bytes(0x4000, len as u16)?,
    })
}

fn temp_dir(tag: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("rc-suite-cpc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn a_disk_rcld_made_loads_through_amsdos(env: &Env) -> Result<(), String> {
    let program = "start: ld a,$AA
 ld ($9000),a
here: jr here
msg: .text \"HELLO\"
";
    let dir = temp_dir("rcld")?;
    std::fs::write(
        dir.join("main.asm"),
        format!(".cpu z80\n.org $4000\n{program}"),
    )
    .map_err(|e| e.to_string())?;
    let layout = "machine = \"cpc\"
name = \"HELLO\"
entry = \"start\"
output = \"hello.dsk\"
[[input]]
asm = [\"main.asm\"]
[[region]]
name = \"ram\"
start = 0x4000
end = 0xA6FF
";
    let built = rcld::build(
        &rcld::layout::parse(layout)?,
        &dir,
        &dir.join("hello.dsk"),
        &[],
    )?;
    std::fs::write(dir.join("hello.dsk"), &built.bytes).map_err(|e| e.to_string())?;
    let want = z80_image(0x4000, program)?;
    let got = amsdos_load(env, "rcld", &dir.join("hello.dsk"), "HELLO.BIN", want.len())?;
    // What was put on the disk is what AMSDOS hands back: the file opens, it is a binary (type 2), its header
    // gives the load address and length rcld was told, and the bytes are the assembled program.
    expect_eq("the file opened", got.ok, true)?;
    expect_eq("file type (2 = binary)", got.file_type, 2)?;
    expect_eq("load address", got.load, 0x4000)?;
    expect_eq("length", usize::from(got.length), want.len())?;
    expect_eq("the file's bytes", got.bytes, want)
}

fn a_multi_sector_file_rcdisk_added_loads_through_amsdos(env: &Env) -> Result<(), String> {
    // 3000 bytes cross six 512-byte sectors; the pattern repeats only every 7 * 251 bytes, so a sector read in the
    // wrong place shows.
    let data: Vec<u8> = (0..3000u32).map(|i| (i * 7 + i / 251) as u8).collect();
    let mut disk = rcdisk::create("dsk", None, None)?;
    disk.add(
        "TWO.BIN",
        &data,
        &rcdisk::AddOptions {
            header: Some("amsdos".to_string()),
            load: Some(0x4000),
            entry: Some(0x4000),
            ..rcdisk::AddOptions::default()
        },
    )?;
    let dir = temp_dir("rcdisk")?;
    let path = dir.join("two.dsk");
    std::fs::write(&path, disk.to_bytes()).map_err(|e| e.to_string())?;
    let got = amsdos_load(env, "rcdisk", &path, "TWO.BIN", data.len())?;
    expect_eq("the file opened", got.ok, true)?;
    expect_eq("load address", got.load, 0x4000)?;
    expect_eq("length", usize::from(got.length), data.len())?;
    expect_eq("the file's bytes", got.bytes, data)
}

fn a_sna_of_every_version_loads_and_runs(env: &Env) -> Result<(), String> {
    let rom = rom(env)?;
    for version in 1..=3u8 {
        let resident = Resident {
            machine: "cpc6128",
            rom: &rom,
            extra: Vec::new(),
            sna_version: Some(version),
        };
        let out = resident.run(
            env,
            "sna",
            RESIDENT_AT,
            "ld a,$AA
 ld ($9000),a
here: jr here
",
            20,
            &[(0x9000, 1)],
        )?;
        expect_eq(
            &format!("marker written by a version {version} snapshot's program"),
            out.byte(0x9000)?,
            0xAA,
        )?;
    }
    Ok(())
}

const CASES: &[Case] = &[
    Case::new(
        Area::Timers,
        "instructions take whole microseconds",
        "GATE ARRAY (instructions run in whole 1 us NOPs); INC HL 8 T, JR 12 T",
        instructions_take_whole_microseconds,
    ),
    Case::new(
        Area::Timers,
        "the Gate Array interrupts every 52 lines",
        "GATE ARRAY 52 HSYNC counter; CRTC R0=63 (64 us line); Z80 for the CPC '6x per frame'",
        gate_array_interrupts_every_52_lines,
    ),
    Case::new(
        Area::Memory,
        "RAM configurations C0-C7",
        "Z80 for the CPC, the C0-C7 table",
        ram_configurations_c0_to_c7,
    ),
    Case::new(
        Area::Media,
        "a disk rcld made loads through AMSDOS",
        "the file rcld wrote is the file the firmware reads back",
        a_disk_rcld_made_loads_through_amsdos,
    ),
    Case::new(
        Area::Media,
        "a multi-sector file rcdisk added loads through AMSDOS",
        "the file rcdisk wrote is the file the firmware reads back",
        a_multi_sector_file_rcdisk_added_loads_through_amsdos,
    ),
    Case::new(
        Area::Media,
        "a .sna of every version loads and runs",
        "a snapshot resumes the program in it (CPC .sna versions 1-3)",
        a_sna_of_every_version_loads_and_runs,
    ),
    Case::new(
        Area::Rom,
        "the lower ROM shows at reset and writes go to the RAM under it",
        "GATE ARRAY mode register bit 2 (0 = lower ROM on)",
        the_lower_rom_shows_at_reset_and_writes_go_to_the_ram_under_it,
    ),
];

#[test]
fn cpc() {
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
