//! A tape made by rctape, loaded by the Spectrum ROM's own tape loader in the emulator.
//!
//! Needs a built `retroclawd` with its machine libraries and the ROM images, so it only runs when the
//! environment says where they are:
//!
//! ```text
//! RETROCLAWD_BIN=... RETROCLAWD_MACHINE_PLUGIN_DIRS=... RC_ROMS=.../machines cargo test -p rctape --test emulator
//! ```

use rcasm::{Cpu, DiskFiles, Options, assemble};
use std::path::PathBuf;
use std::process::Command;

fn bytes(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
        .collect()
}

/// A program that calls the ROM's LD-BYTES for `len` bytes of a data block (flag 0xFF) into `at` (after the header
/// block, if there is one), then spins. It leaves 0x2A in 0x8FFF when the ROM reports success, 0x66 otherwise.
fn loader(at: u16, len: u16, header_first: bool) -> Vec<u8> {
    // LD-BYTES gives up at a block whose flag is not the one asked for, so a header block has to be read first.
    let header = if header_first {
        " ld ix,0xA000\n ld de,17\n xor a\n scf\n call 0x0556\n jr nc,failed\n"
    } else {
        ""
    };
    let src = format!(
        ".org 0x8000\n{header} ld ix,{at}\n ld de,{len}\n ld a,0xFF\n scf\n call 0x0556\n jr nc,failed\n ld a,0x2A\n ld (0x8FFF),a\nhere: jr here\nfailed: ld a,0x66\n ld (0x8FFF),a\n jr failed\n"
    );
    let opts = Options {
        cpu: Some(Cpu::Z80),
        ..Options::default()
    };
    let asm = assemble("loader.asm", &src, &opts, &DiskFiles).unwrap_or_else(|e| panic!("{e:?}"));
    rcasm::flat_image(&asm, 0).unwrap().1
}

fn run(tape_file: &str, tag: &str, expect: &[u8], header_first: bool) {
    let (Some(bin), Some(roms)) = (
        std::env::var_os("RETROCLAWD_BIN").map(PathBuf::from),
        std::env::var_os("RC_ROMS").map(PathBuf::from),
    ) else {
        eprintln!("skipping: RETROCLAWD_BIN / RC_ROMS not set");
        return;
    };
    let dir = std::env::temp_dir().join(format!("rctape-emu-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("loader.bin");
    std::fs::write(&program, loader(0x9000, expect.len() as u16, header_first)).unwrap();
    let rom = roms.join("sinclair/roms/zx-spectrum/48.rom");
    // --program and --tape do not go together, so the loader is first put into a saved state (the program,
    // placed and about to run), and that state is then started with the tape in the drive.
    let state = dir.join("loader.state");
    let prepared = Command::new(&bin)
        .args([
            "zx-spectrum",
            "--rom",
            rom.to_str().unwrap(),
            "--program",
            program.to_str().unwrap(),
            "--org",
            "0x8000",
            "--headless",
            "--max-cycles",
            "1",
            "--dump-state",
            state.to_str().unwrap(),
        ])
        .output()
        .expect("runs retroclawd");
    assert!(
        prepared.status.success(),
        "preparing the state failed: {}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    let out = Command::new(&bin)
        .args([
            "zx-spectrum",
            "--rom",
            rom.to_str().unwrap(),
            "--load-state",
            state.to_str().unwrap(),
            "--tape",
            tape_file,
            "--headless",
            "--max-cycles",
            "12000000",
            "--dump-memory",
            "0x8FFF:16",
        ])
        .output()
        .expect("runs retroclawd");
    assert!(
        out.status.success(),
        "retroclawd failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    // The status byte (2A = loaded, 66 = the ROM reported an error) and the first 15 bytes of the block.
    let first: String = expect
        .iter()
        .take(15)
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        text.contains(&format!("0x8FFF: 2A {first}")),
        "the ROM loader failed or never finished:\n{text}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_rom_loads_a_block_from_a_tzx_rctape_made() {
    let dir = std::env::temp_dir().join(format!("rctape-emu-srctzx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let data = bytes(200, 3);
    let mut tape = rctape::Tape::new();
    let mut framed = vec![0xFFu8];
    framed.extend_from_slice(&data);
    framed.push(framed.iter().fold(0, |a, &b| a ^ b));
    tape.blocks.push(rctape::Block::standard(&framed, 1000));
    let path = dir.join("data.tzx");
    std::fs::write(&path, tape.to_bytes()).unwrap();
    run(path.to_str().unwrap(), "tzx", &data, false);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_rom_loads_a_block_from_a_tap_converted_from_a_tzx() {
    let dir = std::env::temp_dir().join(format!("rctape-emu-srctap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let data = bytes(120, 9);
    let mut framed = vec![0xFFu8];
    framed.extend_from_slice(&data);
    framed.push(framed.iter().fold(0, |a, &b| a ^ b));
    // A header first (the loader skips blocks whose flag is not 0xFF), then the data: through a .tzx and back.
    let mut tape = rctape::Tape::new();
    tape.blocks.push(rctape::Block::standard(
        &rctape::spectrum::make_header(rctape::spectrum::Kind::Code, "x", 120, 0x9000, 32768),
        1000,
    ));
    tape.blocks.push(rctape::Block::standard(&framed, 1000));
    let (tap, _) = rctape::to_tap(&rctape::Tape::parse(&tape.to_bytes()).unwrap(), false).unwrap();
    let path = dir.join("data.tap");
    std::fs::write(&path, tap).unwrap();
    run(path.to_str().unwrap(), "tap", &data, true);
    std::fs::remove_dir_all(&dir).unwrap();
}
