//! Images made by rcld, loaded by the emulator.
//!
//! Needs a built `retroclawd` with its machine libraries and the ROM images, so it only runs when the
//! environment says where they are:
//!
//! ```text
//! RETROCLAWD_BIN=... RETROCLAWD_MACHINE_PLUGIN_DIRS=... RC_ROMS=.../machines cargo test -p rcld --test emulator
//! ```
//!
//! The BASIC lines the loaders put in front (`10 SYS 2061`, `LOAD ""CODE`) and AMSDOS's `RUN"NAME.BIN` need the
//! BASIC interpreter and a keyboard, which a headless run does not have; those were checked by hand in the
//! emulator window (see `CLAUDE.md`).

use rcasm::{Cpu, DiskFiles, Options, assemble};
use std::path::{Path, PathBuf};
use std::process::Command;

fn setup() -> Option<(PathBuf, PathBuf)> {
    Some((
        PathBuf::from(std::env::var_os("RETROCLAWD_BIN")?),
        PathBuf::from(std::env::var_os("RC_ROMS")?),
    ))
}

struct Work(PathBuf);

impl Work {
    fn new(tag: &str) -> Work {
        let dir = std::env::temp_dir().join(format!("rcld-emu-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Work(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        std::fs::write(self.0.join(name), bytes).unwrap();
        self.path(name)
    }
    /// Links `layout` (with main.asm already written) into an image called `out`.
    fn link(&self, layout: &str, out: &str) -> PathBuf {
        let l = rcld::layout::parse(layout).unwrap();
        let built =
            rcld::build(&l, &self.0, &self.path(out), &[]).unwrap_or_else(|e| panic!("{e}"));
        self.write(out, built.bytes)
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn flat(cpu: Cpu, source: &str) -> Vec<u8> {
    let opts = Options {
        cpu: Some(cpu),
        ..Options::default()
    };
    let asm = assemble("loader.asm", source, &opts, &DiskFiles).unwrap_or_else(|e| panic!("{e:?}"));
    rcasm::flat_image(&asm, 0).unwrap().1
}

fn run(bin: &Path, args: &[&str]) -> String {
    let out = Command::new(bin)
        .args(args)
        .output()
        .expect("runs retroclawd");
    assert!(
        out.status.success(),
        "retroclawd {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn c64_args(roms: &Path) -> Vec<String> {
    let c64 = roms.join("commodore/roms/c64");
    let mut v = vec!["c64".to_string()];
    for (flag, file) in [
        ("--rom-kernal", "kernal-901227-03.bin"),
        ("--rom-basic", "basic-901226-01.bin"),
        ("--rom-char", "chargen-901225-01.bin"),
    ] {
        v.push(flag.to_string());
        v.push(c64.join(file).to_string_lossy().into_owned());
    }
    v
}

const SPECTRUM_PROGRAM: &str = ".org 0x8000\nstart: ld hl,msg\n ld de,0x9000\n ld bc,5\n ldir\n ld a,0x2A\n ld (0x8FFF),a\nhere: jr here\nmsg: db \"HELLO\"\n";
const SPECTRUM_LAYOUT: &str =
    "machine = \"spectrum\"\nname = \"HELLO\"\nentry = \"start\"\n[[input]]\nasm = \"main.asm\"\n";

#[test]
fn a_spectrum_snapshot_from_a_layout_runs() {
    let Some((bin, roms)) = setup() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    let w = Work::new("sna");
    w.write("main.asm", SPECTRUM_PROGRAM);
    for ext in ["sna", "z80", "szx"] {
        let image = w.link(SPECTRUM_LAYOUT, &format!("game.{ext}"));
        let out = run(
            &bin,
            &[
                "zx-spectrum",
                "--rom",
                roms.join("sinclair/roms/zx-spectrum/spec48.rom")
                    .to_str()
                    .unwrap(),
                "--snapshot",
                image.to_str().unwrap(),
                "--headless",
                "--max-cycles",
                "20000",
                "--dump-memory",
                "0x8FFF:6",
            ],
        );
        assert!(out.contains("0x8FFF: 2A 48 45 4C 4C 4F"), "{ext}:\n{out}");
    }
}

#[test]
fn the_spectrum_rom_loads_the_loader_and_the_code_from_a_tape_rcld_made() {
    let Some((bin, roms)) = setup() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    let w = Work::new("tap");
    w.write("main.asm", SPECTRUM_PROGRAM);
    let tape = w.link(SPECTRUM_LAYOUT, "game.tap");
    // The ROM's LD-BYTES, four times: the loader's header and program, the code's header and code. The code goes
    // to 0x8000, the BASIC program to 0x8100; the run ends with a HALT.
    let blocks = rctape::from_tap(&std::fs::read(&tape).unwrap())
        .unwrap()
        .blocks;
    let program_len = blocks[1].payload().unwrap().len() - 2;
    let code_len = blocks[3].payload().unwrap().len() - 2;
    let loader = flat(
        Cpu::Z80,
        &format!(
            ".org 0xB000\n\
             ld ix,0xA000\n ld de,17\n xor a\n scf\n call 0x0556\n jr nc,failed\n\
             ld ix,0x8100\n ld de,{program_len}\n ld a,0xFF\n scf\n call 0x0556\n jr nc,failed\n\
             ld ix,0xA000\n ld de,17\n xor a\n scf\n call 0x0556\n jr nc,failed\n\
             ld ix,0x8000\n ld de,{code_len}\n ld a,0xFF\n scf\n call 0x0556\n jr nc,failed\n\
             ld a,0x2A\n ld (0x8FFF),a\n di\n halt\n\
             failed: ld a,0x66\n ld (0x8FFF),a\n di\n halt\n"
        ),
    );
    let w2 = Work::new("tap-run");
    let rom = roms.join("sinclair/roms/zx-spectrum/spec48.rom");
    let program = w2.write("loader.bin", loader);
    let state = w2.path("loader.state");
    run(
        &bin,
        &[
            "zx-spectrum",
            "--rom",
            rom.to_str().unwrap(),
            "--program",
            program.to_str().unwrap(),
            "--org",
            "0xB000",
            "--headless",
            "--max-cycles",
            "1",
            "--dump-state",
            state.to_str().unwrap(),
        ],
    );
    let out = run(
        &bin,
        &[
            "zx-spectrum",
            "--rom",
            rom.to_str().unwrap(),
            "--load-state",
            state.to_str().unwrap(),
            "--tape",
            tape.to_str().unwrap(),
            "--headless",
            "--max-cycles",
            "30000000",
            "--dump-memory",
            "0x8FFF:1",
        ],
    );
    assert!(
        out.contains("0x8FFF: 2A"),
        "the ROM reported an error or never finished:\n{out}"
    );
}

#[test]
fn the_c64_kernal_loads_the_prg_from_a_d64_rcld_made() {
    let Some((bin, roms)) = setup() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    let w = Work::new("d64");
    w.write("main.asm", "* = $080D\nstart: inc $D020\n jmp start\n");
    let layout =
        "machine = \"c64\"\nname = \"HELLO\"\nentry = \"start\"\n[[input]]\nasm = \"main.asm\"\n";
    let disk = w.link(layout, "game.d64");
    let prg = w.link(layout, "game.prg");
    // KERNAL: SETNAM "HELLO", SETLFS 1,8,1 (load at the file's own address), LOAD, then idle.
    let loader = w.write(
        "loader.prg",
        {
            let mut v = vec![0x00, 0xC0];
            v.extend(flat(
                Cpu::Mos6502,
                ".org $C000\n lda #5\n ldx #<name\n ldy #>name\n jsr $FFBD\n lda #1\n ldx #8\n ldy #1\n jsr $FFBA\n lda #0\n jsr $FFD5\ndone: jmp done\nname: .text \"HELLO\"\n",
            ));
            v
        },
    );
    let mut args = c64_args(&roms);
    args.extend(
        [
            "--program",
            loader.to_str().unwrap(),
            "--disk",
            disk.to_str().unwrap(),
            "--headless",
            "--max-cycles",
            "3000000",
            "--dump-memory",
            "0x0801:18",
        ]
        .map(String::from),
    );
    let out = run(&bin, &args.iter().map(String::as_str).collect::<Vec<_>>());
    // The stub and the code, as they are in the .prg after the load address.
    let want: Vec<String> = std::fs::read(&prg).unwrap()[2..]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect();
    assert_eq!(want.len(), 18);
    assert!(
        out.contains(&format!("0x0801: {}", want[..16].join(" "))),
        "{out}"
    );
}

#[test]
fn a_c64_cartridge_from_a_layout_starts() {
    let Some((bin, roms)) = setup() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    let w = Work::new("crt");
    w.write(
        "main.asm",
        "* = $8009\nstart: sei\n lda #$2A\n sta $C0FF\ndone: jmp done\n",
    );
    let layout =
        "machine = \"c64\"\nname = \"CART\"\nentry = \"start\"\n[[input]]\nasm = \"main.asm\"\n";
    let crt = w.link(layout, "game.crt");
    let mut args = c64_args(&roms);
    args.extend(
        [
            "--cartridge",
            crt.to_str().unwrap(),
            "--headless",
            "--max-cycles",
            "3000000",
            "--dump-memory",
            "0xC0FF:1",
        ]
        .map(String::from),
    );
    let out = run(&bin, &args.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(out.contains("0xC0FF: 2A"), "{out}");
}
