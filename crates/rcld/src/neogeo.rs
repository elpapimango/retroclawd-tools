//! The Neo Geo target: a cartridge set, one zip whose file names carry the regions (the convention the emulator's
//! `neo-set` reads): `NNN-p1.p1` (the 68000 program; a part above 1 MB is `-p2.p2`), `NNN-s1.s1` (the fix layer),
//! `NNN-c1.c1` and `NNN-c2.c2` (the sprite data: the `c` file is the byte-interleaved region, `c1` the even bytes, `c2`
//! the odd ones), `NNN-m1.m1` (the Z80 program), `NNN-v1.v11` and `NNN-v2.v21` (the two sample regions).
//!
//! The layout names 68000 sources (flat, absolute addresses with `.org`, the image starts at `$000000`) and, in
//! `[options]`, files for the other regions: `s`, `c`, `m`, `v1`, `v2`; `ngh` is the game number `NNN` (default 999).
//! The program's own header (`NEO-GEO` at `$100`, the jump table, the security code pointer) is the program's: the sources
//! write it, as the SDKs' startup files do [H?: the header is not checked here]. The output is a `.zip`; the files are
//! stored, not compressed.
//!
//! The **security code** (the 61 words at the address in `$182`, which the BIOS compares with its own copy before it
//! starts a cartridge) is SNK's: this tool does not contain it. `options.security` names a file with it, either the
//! 122 bytes themselves or the program file (`.p1`, byte-swapped as dumped) of a cartridge of your own, from which it is
//! copied; the bytes go where `$182` points (the sources reserve them). Without it the BIOS shows its crosshatch.

use crate::Build;
use crate::layout::{self, Layout};
use rcasm::m68k::{M68kOptions, assemble_m68k, symbols_text};
use std::path::{Path, PathBuf};

fn resolve(base: &Path, p: &str) -> PathBuf {
    if Path::new(p).is_absolute() {
        PathBuf::from(p)
    } else {
        base.join(p)
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *t = c;
    }
    !data.iter().fold(!0u32, |c, &b| {
        table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8)
    })
}

/// A zip with every file stored.
pub fn stored_zip(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        if data.len() > u32::MAX as usize / 2 || out.len() > u32::MAX as usize / 2 {
            return Err("a zip entry larger than 2 GB".to_string());
        }
        let crc = crc32(data);
        let offset = out.len() as u32;
        let mut local = Vec::new();
        local.extend_from_slice(&0x0403_4B50u32.to_le_bytes());
        local.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]); // version, flags, stored, time, date 1980-01-01
        local.extend_from_slice(&crc.to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(data.len() as u32).to_le_bytes());
        local.extend_from_slice(&(name.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&local);
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4B50u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    Ok(out)
}

pub fn build(
    layout: &Layout,
    base: &Path,
    output: &Path,
    cli_defines: &[(String, i64)],
) -> Result<Build, String> {
    let ext = output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "zip" {
        return Err(format!(
            "{}: a Neo Geo cartridge set is written as .zip, not .{ext}",
            output.display()
        ));
    }
    if !layout.region.is_empty() || !layout.reserve.is_empty() {
        return Err("regions and reserves are not used for the Neo Geo: the sources place the program with .org".to_string());
    }
    let o = &layout.options;
    let mut sources = Vec::new();
    let mut pieces: Vec<(String, u32, Vec<u8>)> = Vec::new();
    for (n, input) in layout.input.iter().enumerate() {
        if input.src.is_some() || input.obj.is_some() || input.lib.is_some() {
            continue;
        }
        match (&input.asm, &input.bin, &input.prg, &input.symbols) {
            (Some(asm), None, None, None) => sources.extend(asm.list()),
            (None, Some(bin), None, None) => {
                let at = input
                    .at
                    .as_ref()
                    .ok_or_else(|| format!("input {}: bin \"{bin}\" needs \"at\"", n + 1))?;
                let at = layout::number(at).map_err(|e| format!("input {} at: {e}", n + 1))?;
                let at = u32::try_from(at)
                    .map_err(|_| format!("input {} at: {at:#X} is not an address", n + 1))?;
                let path = resolve(base, bin);
                let bytes = std::fs::read(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                pieces.push((bin.clone(), at, bytes));
            }
            _ => {
                return Err(format!(
                    "input {}: the Neo Geo target takes asm and bin inputs",
                    n + 1
                ));
            }
        }
    }
    let mut defines: Vec<(String, i64)> =
        layout.define.iter().map(|(k, v)| (k.clone(), *v)).collect();
    defines.extend(cli_defines.iter().cloned());
    let mut symbols_txt = String::new();
    if !sources.is_empty() {
        let mut text = String::new();
        for s in &sources {
            text.push_str(&format!(
                "    .include \"{}\"\n",
                resolve(base, s)
                    .display()
                    .to_string()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
            ));
        }
        let opts = M68kOptions {
            relocatable: false,
            object: false,
            include_dirs: vec![base.to_path_buf()],
            defines,
        };
        let asm = assemble_m68k("layout", &text, &opts, &rcasm::DiskFiles).map_err(|e| {
            e.iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        })?;
        let image = asm.image();
        if !image.is_empty() {
            pieces.push(("assembled program".to_string(), asm.origin, image));
        }
        symbols_txt = symbols_text(&asm, false);
    }
    let items = crate::linked_items(layout, base);
    if !items.is_empty() {
        let defines: Vec<(String, i64)> = layout
            .define
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .chain(cli_defines.iter().cloned())
            .collect();
        // the program ROM starts at $000000; the work RAM is at $100000
        let l = crate::m68klink::link_flat(layout, base, &items, &defines, 0, 0x10_0000)?;
        for w in l.wide {
            pieces.push((w.name, w.start, w.bytes));
        }
        symbols_txt.push_str(&l.symbols_text);
    }
    if pieces.is_empty() {
        return Err("no program: give [[input]] asm = [\"main.s\"]".to_string());
    }
    let used = pieces
        .iter()
        .map(|p| p.1 as usize + p.2.len())
        .max()
        .unwrap_or(0);
    if used > 0x80_0000 {
        return Err(format!(
            "the program reaches ${used:X}: the cartridge's program space is 8 MB"
        ));
    }
    let mut prom = vec![layout.fill.unwrap_or(0xFF); used.next_multiple_of(2)];
    let mut covered = vec![false; prom.len()];
    for (name, at, bytes) in &pieces {
        let at = *at as usize;
        if let Some(i) = (at..at + bytes.len()).find(|&i| covered[i]) {
            return Err(format!("{name} overlaps earlier data at ${i:06X}"));
        }
        prom[at..at + bytes.len()].copy_from_slice(bytes);
        covered[at..at + bytes.len()].fill(true);
    }
    let ngh = o.ngh.unwrap_or(999);
    if ngh > 9999 {
        return Err(format!(
            "options.ngh {ngh}: the game number has at most four digits"
        ));
    }
    let has_header = prom.get(0x100..0x107) == Some(b"NEO-GEO".as_slice());
    let mut notes = Vec::new();
    if has_header {
        security_code(&mut prom, base, o.security.as_deref(), &mut notes)?;
    }
    // the program files hold every 16-bit word byte-swapped, as the cartridges' dumps do ("NEO-GEO" reads "EN-OEG")
    for pair in prom.as_chunks_mut::<2>().0 {
        pair.swap(0, 1);
    }
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    if prom.len() > 0x10_0000 {
        let rest = prom.split_off(0x10_0000);
        files.push((format!("{ngh:03}-p1.p1"), prom));
        files.push((format!("{ngh:03}-p2.p2"), rest));
    } else {
        files.push((format!("{ngh:03}-p1.p1"), prom));
    }
    let read = |what: &str, p: &Option<String>| -> Result<Option<Vec<u8>>, String> {
        match p {
            None => Ok(None),
            Some(p) => {
                let path = resolve(base, p);
                std::fs::read(&path)
                    .map(Some)
                    .map_err(|e| format!("options.{what}: cannot read {}: {e}", path.display()))
            }
        }
    };
    if let Some(s) = read("s", &o.s)? {
        files.push((format!("{ngh:03}-s1.s1"), s));
    }
    if let Some(c) = read("c", &o.c)? {
        if c.len() % 2 != 0 {
            return Err("options.c: the interleaved sprite region has an even size".to_string());
        }
        let even: Vec<u8> = c.iter().step_by(2).copied().collect();
        let odd: Vec<u8> = c.iter().skip(1).step_by(2).copied().collect();
        files.push((format!("{ngh:03}-c1.c1"), even));
        files.push((format!("{ngh:03}-c2.c2"), odd));
    }
    if let Some(m) = read("m", &o.m)? {
        files.push((format!("{ngh:03}-m1.m1"), m));
    }
    if let Some(v) = read("v1", &o.v1)? {
        files.push((format!("{ngh:03}-v1.v11"), v));
    }
    if let Some(v) = read("v2", &o.v2)? {
        files.push((format!("{ngh:03}-v2.v21"), v));
    }
    if !has_header {
        notes.push(
            "the program has no NEO-GEO header at $100: the BIOS will not start it".to_string(),
        );
    }
    let map: String = files
        .iter()
        .map(|(n, d)| format!("{n}  {} bytes\n", d.len()))
        .collect();
    Ok(Build {
        bytes: stored_zip(&files)?,
        notes,
        map,
        symbols_text: symbols_txt,
    })
}

/// The length of the security code: 61 words.
const SECURITY_LEN: usize = 122;

/// Fills the security code from `options.security` into the place the header's `$182` points to (`prom` is the
/// program before its words are swapped).
fn security_code(
    prom: &mut [u8],
    base: &Path,
    file: Option<&str>,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let at = prom
        .get(0x182..0x186)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let Some(file) = file else {
        notes.push(
            "no options.security: the BIOS compares SNK's security code (at the address in $182) before it starts a cartridge, and shows its crosshatch without it"
                .to_string(),
        );
        return Ok(());
    };
    let at = match at {
        Some(a) if a % 2 == 0 && a + SECURITY_LEN <= prom.len() => a,
        Some(a) => {
            return Err(format!(
                "options.security: the header's $182 points to ${a:06X}, which is odd or leaves no {SECURITY_LEN} bytes in the program"
            ));
        }
        None => return Err("options.security: the program ends before $186".to_string()),
    };
    let path = resolve(base, file);
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("options.security: cannot read {}: {e}", path.display()))?;
    let code = if bytes.len() == SECURITY_LEN {
        bytes
    } else {
        // a cartridge's program file: words byte-swapped; follow its own $182
        let word = |i: usize| bytes.get(i..i + 2).map(|w| [w[1], w[0]]);
        let from = match (word(0x182), word(0x184)) {
            (Some(h), Some(l)) => u32::from_be_bytes([h[0], h[1], l[0], l[1]]) as usize,
            _ => {
                return Err(format!(
                    "options.security: {} is neither the {SECURITY_LEN}-byte code nor a cartridge program file",
                    path.display()
                ));
            }
        };
        if bytes.get(0x100..0x106) != Some(b"EN-OEG".as_slice()) || from % 2 != 0 {
            return Err(format!(
                "options.security: {} is neither the {SECURITY_LEN}-byte code nor a cartridge program file (no EN-OEG header)",
                path.display()
            ));
        }
        (from..from + SECURITY_LEN)
            .step_by(2)
            .map(word)
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                format!(
                    "options.security: {} ends inside its security code",
                    path.display()
                )
            })?
            .concat()
    };
    prom[at..at + SECURITY_LEN].copy_from_slice(&code);
    Ok(())
}
