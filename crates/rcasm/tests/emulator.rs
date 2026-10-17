//! Programs assembled here run in the emulator and leave the expected bytes in memory.
//!
//! Needs a built `retroclawd` with its machine libraries and the ROM images, so it only runs when the
//! environment says where they are, and is skipped otherwise:
//!
//! ```text
//! RETROCLAWD_BIN=/path/to/retroclawd RETROCLAWD_MACHINE_PLUGIN_DIRS=/path/to/plugins \
//! RC_ROMS=/path/to/retroclawd/machines cargo test -p rcasm --test emulator
//! ```

use rcasm::{Cpu, DiskFiles, Options, assemble};
use std::path::PathBuf;
use std::process::Command;

fn setup() -> Option<(PathBuf, PathBuf)> {
    let bin = PathBuf::from(std::env::var_os("RETROCLAWD_BIN")?);
    let roms = PathBuf::from(std::env::var_os("RC_ROMS")?);
    Some((bin, roms))
}

fn build(cpu: Cpu, source: &str) -> rcasm::Assembly {
    let opts = Options {
        cpu: Some(cpu),
        ..Options::default()
    };
    assemble("emu.asm", source, &opts, &DiskFiles).unwrap_or_else(|e| panic!("{e:?}"))
}

fn dump(bin: &PathBuf, args: &[&str]) -> String {
    let out = Command::new(bin)
        .args(args)
        .output()
        .expect("runs retroclawd");
    assert!(
        out.status.success(),
        "retroclawd failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_c64_program_runs_in_the_emulator() {
    let Some((bin, roms)) = setup() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    let asm = build(
        Cpu::Mos6502,
        ".org $C000\nstart: ldx #0\nloop: lda msg,x\n beq done\n sta $C100,x\n inx\n bne loop\ndone: lda #$2A\n sta $C0FF\n jmp done\nmsg: .text \"HELLO\"\n .byte 0\n",
    );
    let dir = std::env::temp_dir().join(format!("rcasm-emu-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let prg = dir.join("hello.prg");
    std::fs::write(&prg, rcasm::prg_image(&asm, 0).unwrap()).unwrap();
    let c64 = roms.join("commodore/roms/c64");
    let out = dump(
        &bin,
        &[
            "c64",
            "--rom-kernal",
            c64.join("kernal-901227-03.bin").to_str().unwrap(),
            "--rom-basic",
            c64.join("basic-901226-01.bin").to_str().unwrap(),
            "--rom-char",
            c64.join("chargen-901225-01.bin").to_str().unwrap(),
            "--program",
            prg.to_str().unwrap(),
            "--headless",
            "--max-cycles",
            "20000",
            "--dump-memory",
            "0xC0FF:6",
        ],
    );
    assert!(out.contains("0xC0FF: 2A 48 45 4C 4C 4F"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_spectrum_program_runs_in_the_emulator() {
    let Some((bin, roms)) = setup() else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    let asm = build(
        Cpu::Z80,
        ".org 0x8000\n ld hl,msg\n ld de,0x8100\n ld bc,len\n ldir\n ld a,0x2A\n ld (0x80FF),a\nhere: jr here\nmsg: db \"HELLO\"\nlen equ $ - msg\n",
    );
    let dir = std::env::temp_dir().join(format!("rcasm-emu-z80-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let image = dir.join("hello.bin");
    std::fs::write(&image, rcasm::flat_image(&asm, 0).unwrap().1).unwrap();
    let out = dump(
        &bin,
        &[
            "zx-spectrum",
            "--rom",
            roms.join("sinclair/roms/zx-spectrum/spec48.rom")
                .to_str()
                .unwrap(),
            "--program",
            image.to_str().unwrap(),
            "--org",
            "0x8000",
            "--headless",
            "--max-cycles",
            "5000",
            "--dump-memory",
            "0x80FF:6",
        ],
    );
    assert!(out.contains("0x80FF: 2A 48 45 4C 4C 4F"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}
