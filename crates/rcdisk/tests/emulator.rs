//! A disk made by rcdisk, with a program made by rcasm on it, loaded by the emulator.
//!
//! Needs a built `retroclawd` with its machine libraries and the ROM images, so it only runs when the
//! environment says where they are:
//!
//! ```text
//! RETROCLAWD_BIN=... RETROCLAWD_MACHINE_PLUGIN_DIRS=... RC_ROMS=.../machines cargo test -p rcdisk --test emulator
//! ```
//!
//! The CPC side (AMSDOS reading a `.dsk` made by `rcdisk create` / `rcdisk add --header amsdos`) was checked by
//! hand in the emulator window: `CAT` lists the file and `RUN"DISC` runs it; see `CLAUDE.md`.

use rcasm::{Cpu, DiskFiles, Options, assemble};
use std::path::PathBuf;
use std::process::Command;

#[test]
fn the_c64_kernal_loads_a_file_from_a_disk_rcdisk_made() {
    let (Some(bin), Some(roms)) = (
        std::env::var_os("RETROCLAWD_BIN").map(PathBuf::from),
        std::env::var_os("RC_ROMS").map(PathBuf::from),
    ) else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    // The program: SETNAM "PAYLOAD", SETLFS 1,8,1, LOAD (to the file's own address), then spin.
    let loader = assemble(
        "loader.asm",
        ".org $C000\nname_len = 7\n lda #name_len\n ldx #<name\n ldy #>name\n jsr $FFBD\n lda #1\n ldx #8\n ldy #1\n jsr $FFBA\n lda #0\n jsr $FFD5\ndone: jmp done\nname: .text \"PAYLOAD\"\n",
        &Options { cpu: Some(Cpu::Mos6502), ..Options::default() },
        &DiskFiles,
    )
    .expect("assembles");
    let dir = std::env::temp_dir().join(format!("rcdisk-emu-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let prg = dir.join("loader.prg");
    std::fs::write(&prg, rcasm::prg_image(&loader, 0).unwrap()).unwrap();

    // The disk: a PRG that loads at $C100.
    let mut disk = rcdisk::create("d64", Some("RC TEST"), Some("01")).unwrap();
    let mut payload = vec![0x00, 0xC1];
    payload.extend(b"RCDISK");
    disk.add("payload.prg", &payload, &rcdisk::AddOptions::default())
        .unwrap();
    let image = dir.join("test.d64");
    std::fs::write(&image, disk.to_bytes()).unwrap();

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
            "--program",
            prg.to_str().unwrap(),
            "--disk",
            image.to_str().unwrap(),
            "--headless",
            "--max-cycles",
            "100000",
            "--dump-memory",
            "0xC100:6",
        ])
        .output()
        .expect("runs retroclawd");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("0xC100: 52 43 44 49 53 4B"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}
