//! Layouts built into images, and the images read back with the other tools' libraries.

use rcld::layout;
use std::path::{Path, PathBuf};

struct Project(PathBuf);

impl Project {
    fn new(tag: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("rcld-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Project(dir)
    }
    fn file(&self, name: &str, text: &str) -> &Project {
        std::fs::write(self.0.join(name), text).unwrap();
        self
    }
    fn bytes(&self, name: &str, bytes: &[u8]) -> &Project {
        std::fs::write(self.0.join(name), bytes).unwrap();
        self
    }
    fn path(&self) -> &Path {
        &self.0
    }
    /// Builds `layout` into an output called `out`; returns the image or the error text.
    fn build(&self, layout_text: &str, out: &str) -> Result<rcld::Build, String> {
        let layout = layout::parse(layout_text)?;
        rcld::build(&layout, &self.0, &self.0.join(out), &[])
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const C64_CODE: &str =
    "* = $080D\nstart: lda #$2A\n sta $C0FF\nloop: jmp loop\nmsg: .text \"HI\"\n";
const C64_LAYOUT: &str =
    "machine = \"c64\"\nname = \"HELLO\"\nentry = \"start\"\n[[input]]\nasm = \"main.asm\"\n";
const Z80_CODE: &str =
    ".org 0x8000\nstart: ld a,0x2A\n ld (0x9000),a\nloop: jr loop\ndata: db 1,2,3\n";

fn z80_layout(extra: &str) -> String {
    format!(
        "machine = \"spectrum\"\nname = \"HELLO\"\nentry = \"start\"\n[[input]]\nasm = \"main.asm\"\n{extra}"
    )
}

fn a_c64_prg_gets_a_basic_line_that_calls_the_entry() {
    let p = Project::new("prg");
    p.file("main.asm", C64_CODE);
    let b = p.build(C64_LAYOUT, "hello.prg").unwrap();
    // load address 0x0801, then: link, line 10, SYS, "2061", end of line, end of program, then the code
    assert_eq!(b.bytes[..2], [0x01, 0x08]);
    assert_eq!(
        b.bytes[2..15],
        [
            0x0B, 0x08, 0x0A, 0x00, 0x9E, b'2', b'0', b'6', b'1', 0x00, 0x00, 0x00, 0xA9
        ]
    );
    assert_eq!(b.bytes.len(), 2 + 12 + 10);
    assert!(
        b.notes.iter().any(|n| n.contains("10 SYS 2061")),
        "{:?}",
        b.notes
    );
    assert!(
        b.map.contains("BASIC loader (10 SYS 2061)") && b.map.contains("entry: $080D (start)"),
        "{}",
        b.map
    );
    assert!(
        b.symbols_text.contains("start = $080D"),
        "{}",
        b.symbols_text
    );
}

fn the_basic_line_can_be_left_out_and_is_not_added_over_basic() {
    let p = Project::new("nostub");
    p.file("main.asm", "* = $0801\n .word 0\nstart: rts\n");
    // Code at $0801: no stub is added by default.
    let b = p.build(C64_LAYOUT, "x.prg").unwrap();
    assert_eq!(b.bytes[..2], [0x01, 0x08]);
    assert!(!b.map.contains("BASIC loader"));
    let p2 = Project::new("nostub2");
    p2.file("main.asm", C64_CODE);
    let layout = format!("{C64_LAYOUT}[options]\nbasic_stub = false\n");
    let b = p2.build(&layout, "x.prg").unwrap();
    assert_eq!(
        b.bytes[..2],
        [0x0D, 0x08],
        "load address of the code itself"
    );
    // Asking for a stub where the code sits at $0801 is an overlap, said by name.
    let layout = format!("{C64_LAYOUT}[options]\nbasic_stub = true\n");
    let e = p.build(&layout, "x.prg").unwrap_err();
    assert!(e.contains("BASIC loader") && e.contains("overlaps"), "{e}");
}
/// Merged (every check kept): `a_c64_prg_gets_a_basic_line_that_calls_the_entry`, `the_basic_line_can_be_left_out_and_is_not_added_over_basic`.
#[test]
fn c64_basic_line() {
    a_c64_prg_gets_a_basic_line_that_calls_the_entry();
    the_basic_line_can_be_left_out_and_is_not_added_over_basic();
}

fn a_hole_is_an_error_unless_filled() {
    let p = Project::new("gaps");
    p.file("main.asm", "* = $080D\nstart: rts\n* = $0820\n nop\n");
    let e = p.build(C64_LAYOUT, "x.prg").unwrap_err();
    assert!(
        e.contains("hole of 18 bytes") && e.contains("$080E-$081F"),
        "{e}"
    );
    let layout = format!("gaps = \"fill\"\nfill = 0xEA\n{C64_LAYOUT}");
    let b = p.build(&layout, "x.prg").unwrap();
    assert_eq!(b.bytes[2 + 12 + 1..2 + 12 + 1 + 18], [0xEA; 18]);
    assert!(
        b.notes.iter().any(|n| n.contains("filled a hole of 18")),
        "{:?}",
        b.notes
    );
}

fn overlaps_regions_and_reserved_ranges_are_refused_by_name() {
    let p = Project::new("checks");
    p.file("main.asm", C64_CODE).bytes("data.bin", &[1; 16]);
    let overlap = format!("{C64_LAYOUT}[[input]]\nbin = \"data.bin\"\nat = 0x0810\n");
    let e = p.build(&overlap, "x.prg").unwrap_err();
    assert!(
        e.contains("data.bin") && e.contains("overlaps") && e.contains("assembled code"),
        "{e}"
    );
    let outside = format!(
        "{C64_LAYOUT}[[region]]\nname = \"low\"\nstart = 0x0801\nend = 0x0820\n[[input]]\nbin = \"data.bin\"\nat = 0x2000\n"
    );
    let e = p.build(&outside, "x.prg").unwrap_err();
    assert!(
        e.contains("data.bin")
            && e.contains("outside every region")
            && e.contains("low $0801-$0820"),
        "{e}"
    );
    let reserved =
        format!("{C64_LAYOUT}[[reserve]]\nstart = 0x0800\nend = 0x0900\nwhy = \"BASIC\"\n");
    let e = p.build(&reserved, "x.prg").unwrap_err();
    assert!(e.contains("reserved range $0800-$0900 (BASIC)"), "{e}");
    let straddle = format!("{C64_LAYOUT}[[region]]\nname = \"r\"\nstart = 0x0801\nend = 0x0815\n");
    assert!(
        p.build(&straddle, "x.prg")
            .unwrap_err()
            .contains("outside every region")
    );
}

fn bins_prgs_and_symbol_files_are_inputs() {
    let p = Project::new("inputs");
    p.file("main.asm", "* = $080D\nstart: jsr helper\n jmp *\n")
        .file("helper.sym", "helper = $C000\n")
        .file("vice.sym", "al C:C100 .other\n; comment\n")
        .bytes("lib.prg", &[0x00, 0xC0, 0x60]);
    let layout = format!(
        "gaps = \"fill\"\n{C64_LAYOUT}[[input]]\nsymbols = \"helper.sym\"\n[[input]]\nprg = \"lib.prg\"\n[[input]]\nsymbols = \"vice.sym\"\n"
    );
    let b = p.build(&layout, "x.prg").unwrap();
    // The JSR target comes from the symbol file; the .prg's byte sits at $C000.
    assert_eq!(b.bytes[2 + 12..2 + 12 + 3], [0x20, 0x00, 0xC0]);
    assert_eq!(*b.bytes.last().unwrap(), 0x60);
    assert_eq!(
        rcld::parse_symbols("al C:C100 .other\nx = $10\ny equ 5\nz 1F\n").unwrap(),
        [
            ("other".into(), 0xC100),
            ("x".into(), 16),
            ("y".into(), 5),
            ("z".into(), 31)
        ]
    );
    assert!(
        rcld::parse_symbols("what is this")
            .unwrap_err()
            .contains("line 1")
    );
}

// On Windows every path has backslashes, so all the other tests cover this there; a file name with one cannot exist.
#[cfg(unix)]
#[test]
fn a_path_with_a_backslash_reaches_the_assembler_intact() {
    let p = Project::new("backslash");
    p.file("we\\ird.asm", C64_CODE);
    let b = p
        .build(&C64_LAYOUT.replace("main.asm", "we\\\\ird.asm"), "x.prg")
        .unwrap();
    assert_eq!(b.bytes[..2], [0x01, 0x08]);
}

fn constants_come_from_the_layout() {
    let p = Project::new("define");
    p.file("main.asm", "* = $080D\nstart: lda #LEVEL\n jmp *\n");
    let layout = format!("{C64_LAYOUT}[define]\nLEVEL = 7\n");
    let b = p.build(&layout, "x.prg").unwrap();
    assert_eq!(b.bytes[2 + 12 + 1], 7);
    let l = layout::parse(&layout).unwrap();
    let b = rcld::build(
        &l,
        p.path(),
        &p.path().join("x.prg"),
        &[("LEVEL".to_string(), 9)],
    )
    .unwrap();
    assert_eq!(b.bytes[2 + 12 + 1], 9, "-D on the command line wins");
}
/// Merged (every check kept): `bins_prgs_and_symbol_files_are_inputs`, `constants_come_from_the_layout`.
#[test]
fn inputs_and_constants() {
    bins_prgs_and_symbol_files_are_inputs();
    constants_come_from_the_layout();
}

#[test]
fn a_d64_and_a_d81_hold_the_program_and_extra_files() {
    let p = Project::new("disks");
    p.file("main.asm", C64_CODE).bytes("level.bin", &[5; 700]);
    let layout = format!(
        "{C64_LAYOUT}[options]\ndisk_name = \"MY GAME\"\ndisk_id = \"RC\"\n[[file]]\npath = \"level.bin\"\nname = \"LEVEL1\"\ntype = \"seq\"\n"
    );
    let prg = p.build(&layout, "x.prg").unwrap().bytes;
    for ext in ["d64", "d81"] {
        let b = p.build(&layout, &format!("x.{ext}")).unwrap();
        let disk = rcdisk::open(&format!("x.{ext}"), &b.bytes).unwrap();
        let rows = disk.files();
        let names: Vec<_> = rows
            .iter()
            .map(|r| (r.name.as_str(), r.kind.as_str()))
            .collect();
        assert_eq!(names, [("HELLO", "PRG"), ("LEVEL1", "SEQ")], "{ext}");
        let got = disk.read("HELLO", &rcdisk::ReadOptions::default()).unwrap();
        assert_eq!(got[0].data, prg, "{ext}");
        let level = disk
            .read("LEVEL1", &rcdisk::ReadOptions::default())
            .unwrap();
        assert_eq!(level[0].data, vec![5; 700], "{ext}");
        assert!(
            disk.check().errors.is_empty(),
            "{ext}: {:?}",
            disk.check().errors
        );
    }
}

#[test]
fn a_cartridge_gets_vectors_and_the_signature() {
    let p = Project::new("crt");
    p.file("main.asm", "* = $8009\nstart: sei\n jmp *\n");
    let layout = format!("{C64_LAYOUT}[[region]]\nname = \"rom\"\nstart = 0x8000\nend = 0x9FFF\n");
    let b = p.build(&layout, "x.crt").unwrap();
    let crt = c64_cartridge::parse(&b.bytes).unwrap();
    assert_eq!(crt.name, "HELLO");
    assert_eq!(crt.chips.len(), 1);
    let rom = &crt.chips[0].data;
    assert_eq!(rom.len(), 0x2000);
    assert_eq!(
        rom[..9],
        [0x09, 0x80, 0x09, 0x80, 0xC3, 0xC2, 0xCD, 0x38, 0x30]
    );
    assert_eq!(rom[9], 0x78);
    // Out of the ROM window.
    p.file("low.asm", "* = $0900\nstart: nop\n");
    let e = p
        .build(&C64_LAYOUT.replace("main.asm", "low.asm"), "x.crt")
        .unwrap_err();
    assert!(e.contains("outside the cartridge ROM"), "{e}");
    // Code on top of the header.
    p.file("clash.asm", "* = $8000\nstart: nop\n");
    assert!(
        p.build(&C64_LAYOUT.replace("main.asm", "clash.asm"), "x.crt")
            .unwrap_err()
            .contains("overlaps")
    );
    // Bigger than 8K becomes 16K.
    p.file("big.asm", "* = $8009\nstart: nop\n* = $A000\n nop\n");
    let b = p
        .build(&C64_LAYOUT.replace("main.asm", "big.asm"), "x.crt")
        .unwrap();
    assert_eq!(
        c64_cartridge::parse(&b.bytes).unwrap().chips[0].data.len(),
        0x4000
    );
}

#[test]
fn a_flat_binary_is_the_pieces_from_the_lowest_address() {
    let p = Project::new("bin");
    p.file("main.asm", Z80_CODE);
    let b = p.build(&z80_layout(""), "x.bin").unwrap();
    assert_eq!(b.bytes, [0x3E, 0x2A, 0x32, 0x00, 0x90, 0x18, 0xFE, 1, 2, 3]);
    assert!(
        b.notes
            .iter()
            .any(|n| n.contains("starts at address $8000"))
    );
}

fn a_spectrum_tape_has_a_basic_loader_and_the_code() {
    let p = Project::new("tap");
    p.file("main.asm", Z80_CODE);
    for ext in ["tap", "tzx"] {
        let b = p.build(&z80_layout(""), &format!("x.{ext}")).unwrap();
        let tape = if ext == "tap" {
            rctape::from_tap(&b.bytes).unwrap()
        } else {
            rctape::Tape::parse(&b.bytes).unwrap()
        };
        let rows = rctape::rows(&tape);
        assert_eq!(rows.len(), 4, "{ext}");
        assert_eq!(
            rows[0].description, "Program: HELLO LINE 10 (44 bytes)",
            "{ext}"
        );
        assert_eq!(
            rows[2].description, "Bytes: HELLO start 32768, length 10",
            "{ext}"
        );
        assert!(rows.iter().all(|r| r.checksum_ok == Some(true)), "{ext}");
        let program = tape.blocks[1].payload().unwrap();
        let basic = rctape::spectrum::list_basic(&program[1..program.len() - 1]);
        let lines: Vec<&str> = basic.lines().map(str::trim_end).collect();
        assert_eq!(
            lines,
            [
                "10 CLEAR 32767",
                "20 LOAD \"\"CODE",
                "30 RANDOMIZE USR 32768"
            ],
            "{ext}"
        );
        let code = tape.blocks[3].payload().unwrap();
        assert_eq!(
            code[1..code.len() - 1],
            [0x3E, 0x2A, 0x32, 0x00, 0x90, 0x18, 0xFE, 1, 2, 3]
        );
    }
}

fn a_spectrum_tape_loads_each_run_and_an_optional_screen() {
    let p = Project::new("tap2");
    p.file(
        "main.asm",
        ".org 0x8000\nstart: jr start\n.org 0xC000\n nop\n",
    )
    .bytes("load.scr", &[7; 6912]);
    let b = p
        .build(
            &z80_layout("[options]\nscreen = \"load.scr\"\npause = 500\n"),
            "x.tzx",
        )
        .unwrap();
    let tape = rctape::Tape::parse(&b.bytes).unwrap();
    let rows = rctape::rows(&tape);
    assert_eq!(rows.len(), 8);
    assert_eq!(rows[2].description, "Bytes: HELLO start 16384, length 6912");
    assert_eq!(rows[4].description, "Bytes: HELLO start 32768, length 2");
    assert_eq!(rows[6].description, "Bytes: HELLO start 49152, length 1");
    assert_eq!(tape.blocks[0].pause_ms(), Some(500));
    let program = tape.blocks[1].payload().unwrap();
    let basic = rctape::spectrum::list_basic(&program[1..program.len() - 1]);
    let lines: Vec<&str> = basic.lines().map(str::trim_end).collect();
    assert_eq!(
        lines,
        [
            "10 CLEAR 32767",
            "20 LOAD \"\"SCREEN$",
            "30 LOAD \"\"CODE",
            "40 LOAD \"\"CODE",
            "50 RANDOMIZE USR 32768"
        ]
    );
    p.bytes("short.scr", &[1; 100]);
    assert!(
        p.build(&z80_layout("[options]\nscreen = \"short.scr\"\n"), "x.tap")
            .unwrap_err()
            .contains("6912")
    );
}

fn a_tape_without_a_loader_is_bare_blocks_and_low_code_needs_that() {
    let p = Project::new("tap3");
    p.file("main.asm", Z80_CODE)
        .file("low.asm", ".org 0x5E00\nstart: nop\n");
    let b = p
        .build(&z80_layout("[options]\nloader = false\n"), "x.tap")
        .unwrap();
    assert_eq!(rctape::rows(&rctape::from_tap(&b.bytes).unwrap()).len(), 2);
    let layout = z80_layout("").replace("main.asm", "low.asm");
    let e = p.build(&layout, "x.tap").unwrap_err();
    assert!(
        e.contains("below $6000") && e.contains("loader = false"),
        "{e}"
    );
    let bare = z80_layout("[options]\nloader = false\n").replace("main.asm", "low.asm");
    assert!(p.build(&bare, "x.tap").is_ok());
    p.file("rom.asm", ".org 0x1000\nstart: nop\n");
    assert!(
        p.build(&z80_layout("").replace("main.asm", "rom.asm"), "x.tap")
            .unwrap_err()
            .contains("ROM")
    );
}
/// Merged (every check kept): `a_spectrum_tape_has_a_basic_loader_and_the_code`, `a_spectrum_tape_loads_each_run_and_an_optional_screen`, `a_tape_without_a_loader_is_bare_blocks_and_low_code_needs_that`.
#[test]
fn spectrum_tapes() {
    a_spectrum_tape_has_a_basic_loader_and_the_code();
    a_spectrum_tape_loads_each_run_and_an_optional_screen();
    a_tape_without_a_loader_is_bare_blocks_and_low_code_needs_that();
}

#[test]
fn spectrum_snapshots_start_at_the_entry_with_the_requested_state() {
    let p = Project::new("snap");
    p.file("main.asm", Z80_CODE);
    for ext in ["sna", "z80", "szx"] {
        let b = p
            .build(
                &z80_layout("[options]\nsp = 0xFF00\nborder = 4\nim = 2\ninterrupts = true\n"),
                &format!("x.{ext}"),
            )
            .unwrap();
        match rcsnap::open(&format!("x.{ext}"), &b.bytes).unwrap() {
            rcsnap::Image::Spectrum { spec, .. } => {
                assert_eq!(spec.cpu.pc, 0x8000, "{ext}");
                assert_eq!(spec.cpu.sp, 0xFF00, "{ext}");
                assert_eq!(
                    (spec.border, spec.cpu.im, spec.cpu.iff1),
                    (4, 2, true),
                    "{ext}"
                );
                assert_eq!(
                    spec.read(0x8000, 7).unwrap(),
                    [0x3E, 0x2A, 0x32, 0x00, 0x90, 0x18, 0xFE],
                    "{ext}"
                );
            }
            _ => panic!("not a Spectrum snapshot"),
        }
    }
    let e = p
        .build(&z80_layout("[options]\nborder = 9\n"), "x.sna")
        .unwrap_err();
    assert!(e.contains("border"), "{e}");
}

fn a_cpc_disk_has_an_amsdos_binary() {
    let p = Project::new("cpc");
    p.file("main.asm", ".org 0x4000\nstart: ld a,1\n jr start\n")
        .bytes("data.bin", &[9; 100]);
    let layout = "machine = \"cpc\"\nname = \"hello\"\nentry = \"start\"\n[[input]]\nasm = \"main.asm\"\n[[file]]\npath = \"data.bin\"\nname = \"DATA.DAT\"\n";
    let b = p.build(layout, "x.dsk").unwrap();
    let disk = rcdisk::open("x.dsk", &b.bytes).unwrap();
    let rows = disk.files();
    assert_eq!(
        rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        ["HELLO.BIN", "DATA.DAT"]
    );
    let opts = rcdisk::ReadOptions { keep_header: true };
    let f = &disk.read("HELLO.BIN", &opts).unwrap()[0].data;
    // A CP/M file is a whole number of 128 byte records; the header holds the real length.
    assert_eq!(f.len(), 256);
    assert_eq!(u16::from_le_bytes([f[0x18], f[0x19]]), 4, "length");
    assert_eq!(
        u16::from_le_bytes([f[0x15], f[0x16]]),
        0x4000,
        "load address"
    );
    assert_eq!(u16::from_le_bytes([f[0x1A], f[0x1B]]), 0x4000, "entry");
    assert_eq!(f[0x12], 2, "binary file type");
    assert_eq!(f[128..132], [0x3E, 0x01, 0x18, 0xFC]);
    assert!(disk.check().errors.is_empty());
}

fn a_plus3_disk_has_a_3dos_binary() {
    let p = Project::new("plus3");
    p.file("main.asm", Z80_CODE);
    let b = p.build(&z80_layout(""), "x.dsk").unwrap();
    let disk = rcdisk::open("x.dsk", &b.bytes).unwrap();
    let opts = rcdisk::ReadOptions { keep_header: true };
    let f = &disk.read("HELLO", &opts).unwrap()[0].data;
    assert_eq!(&f[..8], b"PLUS3DOS");
    assert_eq!(
        f[128..138],
        [0x3E, 0x2A, 0x32, 0x00, 0x90, 0x18, 0xFE, 1, 2, 3]
    );
}
/// Merged (every check kept): `a_cpc_disk_has_an_amsdos_binary`, `a_plus3_disk_has_a_3dos_binary`.
#[test]
fn cpc_and_plus3_disks() {
    a_cpc_disk_has_an_amsdos_binary();
    a_plus3_disk_has_a_3dos_binary();
}

fn a_machine_that_cannot_make_the_output_says_what_it_can() {
    let p = Project::new("wrong");
    p.file("main.asm", C64_CODE);
    let e = p.build(C64_LAYOUT, "x.tap").unwrap_err();
    assert!(
        e.contains("a c64 program cannot be written as \".tap\"")
            && e.contains(".prg .d64 .d81 .crt .bin"),
        "{e}"
    );
}

fn layout_mistakes_are_named() {
    let p = Project::new("mistakes");
    p.file("main.asm", C64_CODE);
    // A typo in a key.
    assert!(
        layout::parse("machine = \"c64\"\nentri = \"start\"\n")
            .unwrap_err()
            .contains("entri")
    );
    assert!(layout::parse("machine = \"unsupported\"\n").is_err());
    // Two kinds in one input.
    let e = p
        .build(
            "machine = \"c64\"\n[[input]]\nasm = \"main.asm\"\nbin = \"x\"\nat = 0x1000\n",
            "x.prg",
        )
        .unwrap_err();
    assert!(e.contains("exactly one of asm, bin, prg or symbols"), "{e}");
    let e = p
        .build(
            "machine = \"c64\"\n[[input]]\nbin = \"main.asm\"\n",
            "x.prg",
        )
        .unwrap_err();
    assert!(e.contains("needs \"at\""), "{e}");
    // An entry that is nothing.
    let e = p
        .build(&C64_LAYOUT.replace("start", "nowhere"), "x.prg")
        .unwrap_err();
    assert!(e.contains("\"nowhere\" is not a symbol"), "{e}");
    // Assembler errors keep their file and line.
    p.file("bad.asm", "* = $080D\n lda #\n");
    let e = p
        .build(&C64_LAYOUT.replace("main.asm", "bad.asm"), "x.prg")
        .unwrap_err();
    assert!(e.contains("bad.asm:2"), "{e}");
    // generic needs a cpu
    assert!(
        p.build(
            "machine = \"generic\"\n[[input]]\nasm = \"main.asm\"\n",
            "x.bin"
        )
        .unwrap_err()
        .contains("cpu")
    );
    assert!(
        p.build(
            "machine = \"c64\"\n[[input]]\nasm = \"missing.asm\"\n",
            "x.prg"
        )
        .is_err()
    );
    assert!(
        p.build("machine = \"c64\"\n", "x.prg")
            .unwrap_err()
            .contains("nothing to put")
    );
}
/// Merged (every check kept): `a_hole_is_an_error_unless_filled`, `overlaps_regions_and_reserved_ranges_are_refused_by_name`, `layout_mistakes_are_named`, `a_machine_that_cannot_make_the_output_says_what_it_can`.
#[test]
fn layout_errors() {
    a_hole_is_an_error_unless_filled();
    overlaps_regions_and_reserved_ranges_are_refused_by_name();
    layout_mistakes_are_named();
    a_machine_that_cannot_make_the_output_says_what_it_can();
}

#[test]
fn the_templates_parse() {
    for m in ["c64", "spectrum", "cpc"] {
        let text = rcld::templates::template(m).unwrap();
        let l = layout::parse(text).unwrap_or_else(|e| panic!("{m}: {e}"));
        assert_eq!(l.machine.name(), m);
        assert!(l.output.is_some());
    }
    assert!(rcld::templates::template("atari").is_none());
}

#[test]
fn the_spectrum_loader_has_the_hidden_number_forms_the_interpreter_reads() {
    let prog = rcld::basic::spectrum_loader(32767, false, 1, 32768);
    // 10 CLEAR 32767 <0E 00 00 FF 7F 00>
    assert_eq!(prog[..4], [0, 10, 13, 0]);
    assert_eq!(prog[4], 0xFD);
    assert_eq!(&prog[5..10], b"32767");
    assert_eq!(prog[10..16], [0x0E, 0, 0, 0xFF, 0x7F, 0]);
    assert_eq!(prog[16], 0x0D);
    assert_eq!(rcld::basic::c64_stub(2061).len(), 12);
    assert_eq!(rcld::basic::c64_stub(49152).len(), 13);
    assert_eq!(
        rcld::basic::c64_stub(999).len(),
        11,
        "a three digit address makes a shorter line"
    );
    assert_eq!(rcld::basic::c64_stub(999)[..2], [0x0A, 0x08]);
}
