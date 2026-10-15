//! `rcsnap` as a program.

use std::path::PathBuf;
use std::process::{Command, Output};

fn rcsnap(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rcsnap"))
        .args(args)
        .output()
        .expect("runs rcsnap")
}

fn ok(args: &[&str]) -> String {
    let out = rcsnap(args);
    assert!(
        out.status.success(),
        "rcsnap {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn fails(args: &[&str]) -> String {
    let out = rcsnap(args);
    assert!(!out.status.success(), "rcsnap {args:?} should fail");
    String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout)
}

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Temp {
        let dir = std::env::temp_dir().join(format!("rcsnap-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
    fn write(&self, name: &str, bytes: &[u8]) -> String {
        std::fs::write(self.0.join(name), bytes).unwrap();
        self.path(name)
    }
    fn read(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.0.join(name)).unwrap()
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// True when the listing has the line `key: value`, whatever the padding between them.
fn has(info: &str, key: &str, value: &str) -> bool {
    info.lines().any(|l| {
        l.split_once(':')
            .is_some_and(|(k, v)| k == key && v.trim() == value)
    })
}

fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
        .collect()
}

#[test]
fn create_info_dump_in_every_format() {
    let t = Temp::new("create");
    let prog = t.write("prog.bin", &bytes(300, 1));
    for ext in ["z80", "sna", "szx"] {
        let out = t.path(&format!("p.{ext}"));
        ok(&[
            "create",
            &out,
            "--load",
            &format!("0x8000:{prog}"),
            "--border",
            "2",
            "--sp",
            "0xFF00",
        ]);
        let info = ok(&["info", &out]);
        assert!(info.contains("ZX Spectrum 48K"), "{info}");
        assert!(has(&info, "pc", "8000"), "{info}");
        assert!(has(&info, "sp", "FF00"), "{info}");
        assert!(has(&info, "border", "2"), "{info}");
        assert!(has(&info, "im", "1"), "{info}");
        assert!(has(&info, "iff1", "false"), "{info}");
        let dump = t.path(&format!("mem-{ext}.bin"));
        ok(&["dump", &out, "-o", &dump, "--range", "0x8000:300"]);
        assert_eq!(t.read(&format!("mem-{ext}.bin")), bytes(300, 1), "{ext}");
    }
    // The whole address space is 0xC000 bytes from 0x4000.
    let all = rcsnap(&["dump", &t.path("p.z80")]);
    assert_eq!(all.stdout.len(), 0xC000);
    assert!(fails(&["dump", &t.path("p.z80"), "--range", "0x1000:4"]).contains("ROM"));
    assert!(fails(&["dump", &t.path("p.z80"), "--bank", "1"]).contains("48K"));
}

#[test]
fn convert_between_the_spectrum_formats() {
    let t = Temp::new("convert");
    let prog = t.write("prog.bin", &bytes(64, 5));
    ok(&[
        "create",
        &t.path("a.sna"),
        "-l",
        &format!("0x9000:{prog}"),
        "--pc",
        "0x9010",
        "--ei",
        "--im",
        "2",
    ]);
    ok(&["convert", &t.path("a.sna"), &t.path("a.z80")]);
    ok(&["convert", &t.path("a.z80"), &t.path("a.szx")]);
    ok(&["convert", &t.path("a.szx"), &t.path("b.sna")]);
    for f in ["a.sna", "a.z80", "a.szx", "b.sna"] {
        let info = ok(&["info", &t.path(f)]);
        assert!(has(&info, "pc", "9010"), "{f}: {info}");
        assert!(
            has(&info, "im", "2") && has(&info, "iff1", "true"),
            "{f}: {info}"
        );
    }
    assert_eq!(t.read("a.sna"), t.read("b.sna"));
    assert!(fails(&["convert", &t.path("a.z80"), &t.path("a.szx")]).contains("--force"));
    ok(&["convert", &t.path("a.z80"), &t.path("a.szx"), "--force"]);
    assert!(fails(&["convert", &t.path("a.z80"), &t.path("a.tap")]).contains(".z80, .sna or .szx"));
    let json = ok(&["info", &t.path("a.z80"), "--json"]);
    assert!(
        json.contains("\"format\": \".z80 version 3\"") && json.contains("\"details\""),
        "{json}"
    );
}

#[test]
fn create_rejects_bad_requests() {
    let t = Temp::new("bad");
    let prog = t.write("prog.bin", &[1, 2, 3]);
    let out = t.path("x.sna");
    assert!(fails(&["create", &out, "-l", &format!("0x1000:{prog}")]).contains("ROM"));
    assert!(fails(&["create", &out, "-l", "0x8000"]).contains("ADDRESS:FILE"));
    assert!(fails(&["create", &out, "-l", &format!("0xFFFF:{prog}")]).contains("past the end"));
    assert!(
        fails(&["create", &t.path("x.tap"), "-l", &format!("0x8000:{prog}")])
            .contains(".z80, .sna or .szx")
    );
    assert!(
        fails(&["create", &out, "-l", &format!("0x8000:{prog}"), "--im", "3"]).contains("--im")
    );
    assert!(!std::path::Path::new(&out).exists());
}

#[test]
fn zx81_programs() {
    let t = Temp::new("zx81");
    let mut data = vec![0u8; 0x407D - 0x4009];
    data[3..5].copy_from_slice(&0x4083u16.to_le_bytes());
    data.extend_from_slice(&[0, 10, 2, 0, 0xEA, 0x76]);
    let p = t.write("game.p", &data);
    let info = ok(&["info", &p]);
    assert!(has(&info, "basic_lines", "1"), "{info}");
    ok(&["convert", &p, &t.path("game.p81"), "--name", "my game"]);
    let info = ok(&["info", &t.path("game.p81")]);
    assert!(has(&info, "name", "MY GAME"), "{info}");
    ok(&["convert", &t.path("game.p81"), &t.path("back.p")]);
    assert_eq!(t.read("back.p"), data);
    let dumped = rcsnap(&["dump", &p, "--range", "0x407D:4"]);
    assert_eq!(dumped.stdout, [0, 10, 2, 0]);
}

#[test]
fn cartridges() {
    let t = Temp::new("carts");
    let rom = t.write("game.bin", &bytes(0x2000, 3));
    ok(&["convert", &rom, &t.path("game.crt"), "--name", "TESTCART"]);
    let info = ok(&["info", &t.path("game.crt")]);
    assert!(
        info.contains("TESTCART") && info.contains("Normal cartridge (type 0)"),
        "{info}"
    );
    assert!(info.contains("8000  8192 bytes"), "{info}");
    ok(&["convert", &t.path("game.crt"), &t.path("back.bin")]);
    assert_eq!(t.read("back.bin"), bytes(0x2000, 3));
    ok(&["extract", &t.path("game.crt"), "-d", &t.path("chips")]);
    assert_eq!(t.read("chips/chip01-bank0-8000.bin"), bytes(0x2000, 3));
    assert!(
        fails(&[
            "convert",
            &t.write("odd.bin", &[1; 100]),
            &t.path("odd.crt")
        ])
        .contains("8192 or 16384")
    );

    let gx = t.write("gx.bin", &bytes(0x4000 * 2 + 10, 8));
    ok(&["convert", &gx, &t.path("gx.cpr")]);
    let info = ok(&["info", &t.path("gx.cpr")]);
    assert!(
        info.contains("pages:") && info.contains("page  2"),
        "{info}"
    );
    ok(&["convert", &t.path("gx.cpr"), &t.path("gx2.bin")]);
    let back = t.read("gx2.bin");
    assert_eq!(back.len(), 3 * 0x4000);
    assert_eq!(back[..0x4000 * 2 + 10], bytes(0x4000 * 2 + 10, 8)[..]);
    ok(&["extract", &t.path("gx.cpr"), "-d", &t.path("pages")]);
    assert_eq!(t.read("pages/page01.bin").len(), 0x4000);
    assert!(fails(&["extract", &t.path("a.z80")]).contains("cannot read"));
}

#[test]
fn a_cpc_snapshot_is_described_and_changes_version() {
    let t = Temp::new("cpc");
    let snap = cpc_snapshot::Snapshot {
        version: 3,
        cpu: z80::Cpu::new(),
        gate_array: cpc_snapshot::GateArrayState {
            selected_pen: 0,
            pen_colors: [0; 17],
            rom_config: 0x8C,
            ram_config: 0,
        },
        crtc: cpc_snapshot::CrtcState {
            selected_register: 0,
            registers: [0; 18],
        },
        upper_rom: 0,
        ppi: cpc_snapshot::PpiState {
            port_a: 0,
            port_b: 0,
            port_c: 0,
            control: 0x82,
        },
        ay: cpc_snapshot::AyState {
            selected_register: 0,
            registers: [0; 16],
        },
        ram: bytes(64 * 1024, 2),
        cpc_model: Some(2),
        last_interrupt: Some(0),
        screen_modes: [0; 6],
        version3: None,
    };
    let file = t.write("a.sna", &cpc_snapshot::write(&snap).unwrap());
    let info = ok(&["info", &file]);
    assert!(
        info.contains("CPC .sna version 3") && info.contains("CPC6128") && has(&info, "ram", "64K"),
        "{info}"
    );
    ok(&["convert", &file, &t.path("v1.sna"), "--version", "1"]);
    assert!(ok(&["info", &t.path("v1.sna")]).contains("version 1"));
    assert!(fails(&["convert", &file, &t.path("x.sna"), "--version", "9"]).contains("1, 2 or 3"));
    let dumped = rcsnap(&["dump", &file, "--range", "0x100:4"]);
    assert_eq!(dumped.stdout, bytes(64 * 1024, 2)[0x100..0x104]);
}

#[test]
fn unknown_and_damaged_files_are_refused() {
    let t = Temp::new("junk");
    assert!(fails(&["info", &t.write("x.txt", b"hello")]).contains("not a snapshot"));
    assert!(fails(&["info", &t.write("x.z80", b"short")]).contains("not a valid .z80"));
    assert!(fails(&["info", &t.write("x.crt", b"C64 CARTRIDGE   ")]).contains(".crt"));
}
