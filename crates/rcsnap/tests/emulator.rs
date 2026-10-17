//! Snapshots made or converted by rcsnap, loaded by the emulator, and snapshots the emulator wrote, read back.
//!
//! Needs a built `retroclawd` with its machine libraries and the ROM images, so it only runs when the
//! environment says where they are:
//!
//! ```text
//! RETROCLAWD_BIN=... RETROCLAWD_MACHINE_PLUGIN_DIRS=... RC_ROMS=.../machines cargo test -p rcsnap --test emulator
//! ```

use rcasm::{Cpu, DiskFiles, Options, assemble};
use rcsnap::spectrum::{self, Format, Spec};
use std::path::{Path, PathBuf};
use std::process::Command;

const SOURCE: &str = ".org 0x8000\n ld hl,msg\n ld de,0x9000\n ld bc,5\n ldir\n ld a,0x2A\n ld (0x8FFF),a\nhere: jr here\nmsg: db \"HELLO\"\n";

fn setup() -> Option<(PathBuf, PathBuf)> {
    Some((
        PathBuf::from(std::env::var_os("RETROCLAWD_BIN")?),
        PathBuf::from(std::env::var_os("RC_ROMS")?),
    ))
}

fn program() -> Vec<u8> {
    let opts = Options {
        cpu: Some(Cpu::Z80),
        ..Options::default()
    };
    let asm = assemble("snap.asm", SOURCE, &opts, &DiskFiles).unwrap_or_else(|e| panic!("{e:?}"));
    rcasm::flat_image(&asm, 0).unwrap().1
}

fn emulator(bin: &Path, roms: &Path, args: &[&str]) -> String {
    machine(bin, roms, "zx-spectrum", "spec48.rom", args)
}

fn machine(bin: &Path, roms: &Path, id: &str, rom: &str, args: &[&str]) -> String {
    let out = Command::new(bin)
        .args([
            id,
            "--rom",
            roms.join("sinclair/roms/zx-spectrum")
                .join(rom)
                .to_str()
                .unwrap(),
        ])
        .args(args)
        .args([
            "--headless",
            "--max-cycles",
            "20000",
            "--dump-memory",
            "0x8FFF:6",
        ])
        .output()
        .expect("runs retroclawd");
    assert!(
        out.status.success(),
        "retroclawd {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const EXPECTED: &str = "0x8FFF: 2A 48 45 4C 4C 4F";

#[test]
fn snapshots_rcsnap_made_run_in_the_emulator_in_every_format() {
    let Some((bin, roms)) = setup() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    let dir = std::env::temp_dir().join(format!("rcsnap-emu-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut spec = Spec::blank48(0x8000);
    spec.poke(0x8000, &program()).unwrap();
    for f in [Format::Z80, Format::Sna, Format::Szx] {
        let path = dir.join(format!("made{}", f.name()));
        std::fs::write(&path, spectrum::write(&spec, f).unwrap()).unwrap();
        let out = emulator(&bin, &roms, &["--snapshot", path.to_str().unwrap()]);
        assert!(out.contains(EXPECTED), "{f:?}:\n{out}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_snapshot_the_emulator_wrote_converts_and_runs_again() {
    let Some((bin, roms)) = setup() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    let dir = std::env::temp_dir().join(format!("rcsnap-emu-rt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let prog = dir.join("prog.bin");
    std::fs::write(&prog, program()).unwrap();
    // Run the program, let the emulator save its state, then take that file through the other formats.
    let saved = dir.join("saved.z80");
    emulator(
        &bin,
        &roms,
        &[
            "--program",
            prog.to_str().unwrap(),
            "--org",
            "0x8000",
            "--dump-snapshot",
            saved.to_str().unwrap(),
        ],
    );
    let spec = spectrum::parse(Format::Z80, &std::fs::read(&saved).unwrap()).unwrap();
    assert_eq!(
        spec.read(0x8FFF, 6).unwrap(),
        b"\x2AHELLO",
        "the emulator's own snapshot holds the result"
    );
    assert!(
        (0x8000..0x8020).contains(&spec.cpu.pc),
        "PC is at the idle loop, not {:#06X}",
        spec.cpu.pc
    );
    for f in [Format::Sna, Format::Szx, Format::Z80] {
        let path = dir.join(format!("converted{}", f.name()));
        std::fs::write(&path, spectrum::write(&spec, f).unwrap()).unwrap();
        let out = emulator(&bin, &roms, &["--snapshot", path.to_str().unwrap()]);
        assert!(out.contains(EXPECTED), "{f:?}:\n{out}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_128k_snapshot_the_emulator_wrote_converts_and_runs_again() {
    let Some((bin, roms)) = setup() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    let dir = std::env::temp_dir().join(format!("rcsnap-emu-128-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let prog = dir.join("prog.bin");
    std::fs::write(&prog, program()).unwrap();
    let saved = dir.join("saved.z80");
    machine(
        &bin,
        &roms,
        "zx-spectrum-128",
        "128.rom",
        &[
            "--program",
            prog.to_str().unwrap(),
            "--org",
            "0x8000",
            "--dump-snapshot",
            saved.to_str().unwrap(),
        ],
    );
    let spec = spectrum::parse(Format::Z80, &std::fs::read(&saved).unwrap()).unwrap();
    assert_eq!(spec.model(), spectrum::Model::Spectrum128);
    assert_eq!(spec.read(0x8FFF, 6).unwrap(), b"\x2AHELLO");
    for f in [Format::Sna, Format::Szx, Format::Z80] {
        let path = dir.join(format!("converted{}", f.name()));
        std::fs::write(&path, spectrum::write(&spec, f).unwrap()).unwrap();
        let out = machine(
            &bin,
            &roms,
            "zx-spectrum-128",
            "128.rom",
            &["--snapshot", path.to_str().unwrap()],
        );
        assert!(out.contains(EXPECTED), "{f:?}:\n{out}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_c64_cartridge_made_from_a_raw_rom_starts_in_the_emulator() {
    let Some((bin, roms)) = setup() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    // An 8K ROM at $8000 with the cold-start and warm-start vectors and the CBM80 signature the KERNAL looks for.
    let src = ".org $8000\n .word start\n .word start\n .byte $C3,$C2,$CD,$38,$30\nstart: sei\n lda #$2A\n sta $C0FF\n ldx #0\nl: lda msg,x\n sta $C100,x\n inx\n cpx #5\n bne l\ndone: jmp done\nmsg: .text \"HELLO\"\n";
    let opts = Options {
        cpu: Some(Cpu::Mos6502),
        ..Options::default()
    };
    let asm = assemble("cart.asm", src, &opts, &DiskFiles).unwrap_or_else(|e| panic!("{e:?}"));
    let mut rom = rcasm::flat_image(&asm, 0).unwrap().1;
    rom.resize(0x2000, 0);
    let dir = std::env::temp_dir().join(format!("rcsnap-emu-crt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let crt = dir.join("cart.crt");
    std::fs::write(
        &crt,
        c64_cartridge::write(&rcsnap::carts::crt_from_raw("RCSNAP", &rom).unwrap()),
    )
    .unwrap();
    let c64 = roms.join("commodore/roms/c64");
    let out = Command::new(bin)
        .args([
            "c64",
            "--rom-kernal",
            c64.join("kernal-901227-03.bin").to_str().unwrap(),
            "--rom-basic",
            c64.join("basic-901226-01.bin").to_str().unwrap(),
            "--rom-char",
            c64.join("chargen-901225-01.bin").to_str().unwrap(),
            "--cartridge",
            crt.to_str().unwrap(),
            "--headless",
            "--max-cycles",
            "3000000",
            "--dump-memory",
            "0xC0FF:6",
        ])
        .output()
        .expect("runs retroclawd");
    assert!(
        out.status.success(),
        "retroclawd failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("0xC0FF: 2A 48 45 4C 4C 4F"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}
