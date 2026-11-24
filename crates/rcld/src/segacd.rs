//! The Sega Mega-CD target: a data disc image (`.iso`, 2048-byte sectors) the BIOS boots.
//!
//! The layout names 68000 programs, each assembled on its own at absolute addresses (`.org`), and raw binaries (`bin` with
//! `at`). A piece in the main CPU's work RAM (`$FF0584` and up) is the **initial program** (IP); a piece in the sub CPU's
//! program RAM (`$006000` up) is the **sub program** (SP, with its module header: see the manual's "SP header").
//! The disc has the system area (sectors 0-15: the header, the IP, the SP) and an ISO 9660 volume (sector 16 on) holding the
//! `[[file]]` entries, which the sub program reads through the BIOS.
//!
//! The BIOS runs a region **security code** in `$200-$783` of the first sector and jumps to `$FF0584` after it. That code is
//! Sega's: this tool does not contain it. `options.security` names a file with it (extracted from a disc of your own; 1412
//! bytes, or the 1536 bytes of the whole `$200-$7FF` block). Without it the area is zeros and the disc is for emulators
//! that do not check it.
//!
//! `[options]`: `region` (`J`, `U` or `E`), `title`, `serial`, `copyright`, `console` (`megadrive` or `genesis`), `security`.
//! The volume name is the layout's `name` (11 characters).
//!
//! Facts: the header layout (the id, the IP/SP offset and size fields at `$30-$4F`, the Genesis-style header at `$100`) is
//! read from discs and from Michael Pavone's Sega CD page; `[H?]` for the entry offset and work RAM size fields (written 0).

use crate::layout::{self, Layout};
use crate::Build;
use rcasm::m68k::{M68kOptions, assemble_m68k, symbols_text};
use std::path::{Path, PathBuf};

const SECTOR: usize = 2048;
const IP_START: usize = 0x200;
/// Where the user's code starts in the IP block: the security code is the first `$584` bytes.
const SECURITY_LEN: usize = 0x584;
const IP_BASE: u32 = 0xFF_0000;
const SP_BASE: u32 = 0x6000;
const SYSTEM_SECTORS: usize = 16;

fn resolve(base: &Path, p: &str) -> PathBuf {
    if Path::new(p).is_absolute() {
        PathBuf::from(p)
    } else {
        base.join(p)
    }
}

fn pad_text(out: &mut [u8], at: usize, len: usize, text: &str) {
    for i in 0..len {
        out[at + i] = text.as_bytes().get(i).copied().unwrap_or(b' ');
    }
}

fn put_be32(out: &mut [u8], at: usize, v: u32) {
    out[at..at + 4].copy_from_slice(&v.to_be_bytes());
}

fn both32(out: &mut [u8], at: usize, v: u32) {
    out[at..at + 4].copy_from_slice(&v.to_le_bytes());
    out[at + 4..at + 8].copy_from_slice(&v.to_be_bytes());
}

fn both16(out: &mut [u8], at: usize, v: u16) {
    out[at..at + 2].copy_from_slice(&v.to_le_bytes());
    out[at + 2..at + 4].copy_from_slice(&v.to_be_bytes());
}

fn align_up(v: usize, a: usize) -> usize {
    v.div_ceil(a) * a
}

/// An ISO 9660 directory record.
fn dir_record(name: &[u8], extent: u32, len: u32, dir: bool) -> Vec<u8> {
    let mut r = vec![0u8; 33 + name.len()];
    r[0] = (33 + name.len()) as u8;
    both32(&mut r, 2, extent);
    both32(&mut r, 10, len);
    r[18..25].copy_from_slice(&[93, 10, 23, 0, 0, 0, 0]);
    r[25] = if dir { 2 } else { 0 };
    both16(&mut r, 28, 1);
    r[32] = name.len() as u8;
    r[33..].copy_from_slice(name);
    if r.len() % 2 == 1 {
        r.push(0);
        r[0] += 1;
    }
    r
}

fn iso_name(name: &str) -> Result<Vec<u8>, String> {
    let upper = name.to_ascii_uppercase();
    let (stem, ext) = upper.rsplit_once('.').unwrap_or((&upper, ""));
    let ok = |s: &str, max: usize| {
        !s.is_empty() && s.len() <= max && s.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
    };
    if !ok(stem, 8) || !(ext.is_empty() || ok(ext, 3)) {
        return Err(format!(
            "file \"{name}\": an ISO 9660 level 1 name is up to 8 letters, digits or _ and a 3 character extension"
        ));
    }
    Ok(format!("{stem}.{ext};1").into_bytes())
}

pub fn build(
    layout: &Layout,
    base: &Path,
    output: &Path,
    cli_defines: &[(String, i64)],
) -> Result<Build, String> {
    let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext != "iso" {
        return Err(format!(
            "{}: a Mega-CD disc is written as .iso (a data track of 2048-byte sectors)",
            output.display()
        ));
    }
    let o = &layout.options;
    let mut defines: Vec<(String, i64)> = layout.define.iter().map(|(k, v)| (k.clone(), *v)).collect();
    defines.extend(cli_defines.iter().cloned());

    // the programs and binaries, each with its address
    let mut pieces: Vec<(String, u32, Vec<u8>)> = Vec::new();
    let mut symbols = String::new();
    for (n, input) in layout.input.iter().enumerate() {
        match (&input.asm, &input.bin) {
            (Some(list), None) => {
                for src in list.list() {
                    let path = resolve(base, &src);
                    let text = format!(
                        "    .include \"{}\"\n",
                        path.display().to_string().replace('\\', "\\\\").replace('"', "\\\"")
                    );
                    let opts = M68kOptions {
                        include_dirs: vec![base.to_path_buf()],
                        defines: defines.clone(),
                        ..M68kOptions::default()
                    };
                    let asm = assemble_m68k(&src, &text, &opts, &rcasm::DiskFiles)
                        .map_err(|e| e.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n"))?;
                    let image = asm.image();
                    if !image.is_empty() {
                        pieces.push((src.clone(), asm.origin, image));
                    }
                    symbols.push_str(&symbols_text(&asm, false));
                }
            }
            (None, Some(bin)) => {
                let at = input
                    .at
                    .as_ref()
                    .ok_or_else(|| format!("input {}: bin \"{bin}\" needs \"at\"", n + 1))?;
                let at = layout::number(at).map_err(|e| format!("input {} at: {e}", n + 1))?;
                let at = u32::try_from(at).map_err(|_| format!("input {} at: {at:#X} is not an address", n + 1))?;
                let path = resolve(base, bin);
                let bytes =
                    std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                pieces.push((bin.clone(), at, bytes));
            }
            _ => {
                return Err(format!(
                    "input {}: the Mega-CD target takes `asm` and `bin` inputs (the IP at $FF0584 and up, the SP at $006000 and up)",
                    n + 1
                ));
            }
        }
    }

    // the IP block: disc bytes from $200 (work RAM $FF0000 + offset - $200)
    let mut notes = Vec::new();
    let mut ip: Vec<u8> = vec![0; SECURITY_LEN];
    let mut sp: Vec<u8> = Vec::new();
    for (name, at, bytes) in &pieces {
        let end = *at as u64 + bytes.len() as u64;
        if *at >= IP_BASE + SECURITY_LEN as u32 && end <= 0x1_0000_0000 {
            let off = (*at - IP_BASE) as usize;
            if ip.len() < off + bytes.len() {
                ip.resize(off + bytes.len(), 0);
            }
            if ip[off..off + bytes.len()].iter().any(|b| *b != 0) {
                return Err(format!("{name} overlaps other initial program code"));
            }
            ip[off..off + bytes.len()].copy_from_slice(bytes);
        } else if (SP_BASE..0x8_0000).contains(at) && end <= 0x8_0000 {
            let off = (*at - SP_BASE) as usize;
            if sp.len() < off + bytes.len() {
                sp.resize(off + bytes.len(), 0);
            }
            sp[off..off + bytes.len()].copy_from_slice(bytes);
        } else if *at >= IP_BASE && *at < IP_BASE + SECURITY_LEN as u32 {
            return Err(format!(
                "{name} at ${at:06X}: the first ${SECURITY_LEN:X} bytes of the initial program are the security code; start the program at $FF0584"
            ));
        } else {
            return Err(format!(
                "{name} at ${at:06X}: the main CPU's initial program lives at $FF0584 and up, the sub program at $006000 and up"
            ));
        }
    }
    if ip.len() == SECURITY_LEN {
        return Err("no initial program: assemble a source with `.org $FF0584` (the main CPU starts there)".to_string());
    }
    if sp.is_empty() {
        return Err("no sub program: assemble a source with `.org $6000` and the SP module header".to_string());
    }
    if let Some(sec) = &o.security {
        let path = resolve(base, sec);
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if bytes.len() != SECURITY_LEN && bytes.len() != 0x600 {
            return Err(format!(
                "{}: the security code is {SECURITY_LEN} ($584) or 1536 ($600) bytes, not {}",
                path.display(),
                bytes.len()
            ));
        }
        ip[..SECURITY_LEN].copy_from_slice(&bytes[..SECURITY_LEN]);
    } else {
        notes.push(
            "no options.security: the region security code at $200-$783 is zeros, so a BIOS that checks it will not boot the disc".to_string(),
        );
    }

    // the system area
    let ip_end = align_up(IP_START + ip.len(), SECTOR);
    let sp_start = ip_end;
    let sp_end = sp_start + align_up(sp.len(), SECTOR);
    if sp_end > SYSTEM_SECTORS * SECTOR {
        return Err(format!(
            "the initial and sub programs take {} bytes: the system area (sectors 0-15) holds {}; load the rest from files",
            sp_end,
            SYSTEM_SECTORS * SECTOR
        ));
    }
    let mut disc = vec![0u8; SYSTEM_SECTORS * SECTOR];
    pad_text(&mut disc, 0, 16, "SEGADISCSYSTEM  ");
    let volume = layout.name.clone().unwrap_or_else(|| "RCDISC".to_string()).to_ascii_uppercase();
    pad_text(&mut disc, 0x10, 11, &volume);
    disc[0x1B..0x20].copy_from_slice(&[0, 1, 0, 0, 1]);
    pad_text(&mut disc, 0x20, 11, "SEGAOS");
    disc[0x2B..0x30].copy_from_slice(&[0, 0, 1, 0, 0]);
    put_be32(&mut disc, 0x30, IP_START as u32);
    put_be32(&mut disc, 0x34, (ip_end - IP_START) as u32);
    put_be32(&mut disc, 0x40, sp_start as u32);
    put_be32(&mut disc, 0x44, (sp_end - sp_start) as u32);
    pad_text(&mut disc, 0x50, 0xB0, "");
    let title = o.title.clone().unwrap_or_else(|| volume.clone());
    let console = match o.console.as_deref() {
        Some("megadrive") => "SEGA MEGA DRIVE ",
        _ => "SEGA GENESIS    ",
    };
    pad_text(&mut disc, 0x100, 16, console);
    pad_text(&mut disc, 0x110, 16, o.copyright.as_deref().unwrap_or(""));
    pad_text(&mut disc, 0x120, 48, &title);
    pad_text(&mut disc, 0x150, 48, &title);
    pad_text(&mut disc, 0x180, 14, o.serial.as_deref().unwrap_or("GM 00000000-00"));
    pad_text(&mut disc, 0x18E, 2, "");
    pad_text(&mut disc, 0x190, 16, "");
    pad_text(&mut disc, 0x1A0, 0x50, "");
    let region = o.region.as_deref().unwrap_or("U");
    pad_text(&mut disc, 0x1F0, 16, &region.chars().take(3).collect::<String>());
    disc[IP_START..IP_START + ip.len()].copy_from_slice(&ip);
    disc[sp_start..sp_start + sp.len()].copy_from_slice(&sp);

    // the files and the ISO 9660 volume
    let mut files: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for f in &layout.file {
        let path = resolve(base, &f.path);
        let data = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let name = f.name.clone().unwrap_or_else(|| {
            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        });
        files.push((iso_name(&name)?, data));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    for w in files.windows(2) {
        if w[0].0 == w[1].0 {
            return Err(format!("two files are named {}", String::from_utf8_lossy(&w[0].0)));
        }
    }
    let (l_path, m_path, root) = (18u32, 19u32, 20u32);
    let mut next = 21u32;
    let mut extents = Vec::new();
    for (_, data) in &files {
        extents.push(next);
        next += data.len().div_ceil(SECTOR).max(1) as u32;
    }
    let total = next;
    let mut dir: Vec<u8> = Vec::new();
    dir.extend(dir_record(&[0], root, SECTOR as u32, true));
    dir.extend(dir_record(&[1], root, SECTOR as u32, true));
    for ((name, data), extent) in files.iter().zip(&extents) {
        let rec = dir_record(name, *extent, data.len() as u32, false);
        if dir.len() + rec.len() > SECTOR {
            return Err("too many files for one directory sector (about 40)".to_string());
        }
        dir.extend(rec);
    }
    let mut pvd = vec![0u8; SECTOR];
    pvd[0] = 1;
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[6] = 1;
    pad_text(&mut pvd, 8, 32, "SEGA MEGA DRIVE");
    pad_text(&mut pvd, 40, 32, &volume);
    both32(&mut pvd, 80, total);
    both16(&mut pvd, 120, 1);
    both16(&mut pvd, 124, 1);
    both16(&mut pvd, 128, SECTOR as u16);
    both32(&mut pvd, 132, 10);
    pvd[140..144].copy_from_slice(&l_path.to_le_bytes());
    pvd[148..152].copy_from_slice(&m_path.to_be_bytes());
    let root_rec = dir_record(&[0], root, SECTOR as u32, true);
    pvd[156..156 + 34].copy_from_slice(&root_rec[..34]);
    for (at, len) in [(190, 128), (318, 128), (446, 128), (574, 128), (702, 37), (739, 37), (776, 37)] {
        pad_text(&mut pvd, at, len, "");
    }
    for at in [813, 830, 847, 864] {
        pvd[at..at + 16].copy_from_slice(b"1993102300000000");
    }
    pvd[881] = 1;
    disc.extend(&pvd);
    let mut term = vec![0u8; SECTOR];
    term[0] = 255;
    term[1..6].copy_from_slice(b"CD001");
    term[6] = 1;
    disc.extend(&term);
    for big in [false, true] {
        let mut t = vec![0u8; SECTOR];
        t[0] = 1;
        let extent = if big { root.to_be_bytes() } else { root.to_le_bytes() };
        t[2..6].copy_from_slice(&extent);
        let parent = if big { 1u16.to_be_bytes() } else { 1u16.to_le_bytes() };
        t[6..8].copy_from_slice(&parent);
        disc.extend(&t);
    }
    dir.resize(SECTOR, 0);
    disc.extend(&dir);
    for (_, data) in &files {
        let n = data.len().div_ceil(SECTOR).max(1);
        let mut padded = data.clone();
        padded.resize(n * SECTOR, 0);
        disc.extend(&padded);
    }
    debug_assert_eq!(disc.len(), total as usize * SECTOR);

    let map = format!(
        "system area: IP ${:X}-${:X} (loaded at $FF0000), SP ${:X}-${:X} (loaded at $006000)\n{} file(s), {} sectors ({} bytes)\n",
        IP_START,
        ip_end - 1,
        sp_start,
        sp_end - 1,
        files.len(),
        total,
        disc.len()
    );
    Ok(Build { bytes: disc, notes, map, symbols_text: symbols })
}
