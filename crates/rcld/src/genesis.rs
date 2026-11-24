//! The Sega Mega Drive / Genesis target: a ROM image.
//!
//! The layout names 68000 sources (`[[input]] asm = [...]`, assembled as one flat program at absolute addresses with
//! `.org`; the ROM starts at `$000000`) and raw binaries (`bin` with `at`). Output: `.md`, `.bin` or `.gen`.
//!
//! The 256-byte vector table at `$000000` and the header at `$000100` are written by the sources, or, when the sources
//! put nothing at `$000000-$0001FF`, generated: the vectors from the labels `reset` (or `start`, or `entry`), `hint`
//! (level 4), `vint` (level 6), `ext` (level 2) and the exception names `bus_error`, `address_error`, `illegal`,
//! `div_zero`, `chk`, `trapv`, `privilege`, `trace`, `line_a`, `line_f`; any vector without a label points at `error`
//! if that label exists, else at the reset code (with a note). The header comes from `[options]` (`console`, `title`,
//! `serial`, `copyright`, `region`, `devices`; `stack`). In both cases the ROM end and the checksum (the 16-bit sum of
//! the words from `$200`) are written unless `fix_checksum = false`. The image is padded to `options.size` (default the
//! next power of two, at least 128 KB).

use crate::Build;
use crate::layout::{self, Layout};
use rcasm::m68k::{M68kOptions, assemble_m68k, symbols_text};
use std::fmt::Write;
use std::path::{Path, PathBuf};

fn resolve(base: &Path, p: &str) -> PathBuf {
    if Path::new(p).is_absolute() {
        PathBuf::from(p)
    } else {
        base.join(p)
    }
}

fn put_text(image: &mut [u8], at: usize, len: usize, text: &str) {
    for i in 0..len {
        image[at + i] = text
            .as_bytes()
            .get(i)
            .copied()
            .filter(|c| (0x20..0x7F).contains(c))
            .unwrap_or(b' ');
    }
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
    if !matches!(ext.as_str(), "md" | "bin" | "gen") {
        return Err(format!(
            "{}: a Mega Drive ROM is written as .md, .bin or .gen, not .{ext}",
            output.display()
        ));
    }
    if !layout.region.is_empty() || !layout.reserve.is_empty() {
        return Err(
            "regions and reserves are not used for the Mega Drive: the sources place the ROM with .org"
                .to_string(),
        );
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
                    "input {}: the Mega Drive target takes asm and bin inputs",
                    n + 1
                ));
            }
        }
    }
    let mut defines: Vec<(String, i64)> =
        layout.define.iter().map(|(k, v)| (k.clone(), *v)).collect();
    defines.extend(cli_defines.iter().cloned());
    let mut symbols: Vec<(String, i64)> = Vec::new();
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
        symbols = asm
            .symbols
            .iter()
            .map(|s| (s.name.clone(), s.value))
            .collect();
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
        // the vectors and the header (generated, or `[[segment]] name = "TEXT" start = 0` to write them) come before $200;
        // the work RAM is at $FF0000
        let l = crate::m68klink::link_flat(layout, base, &items, &defines, 0x200, 0xFF_0000)?;
        for w in l.wide {
            pieces.push((w.name, w.start, w.bytes));
        }
        symbols.extend(l.symbols);
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
    if used > 0x40_0000 {
        return Err(format!(
            "the program reaches ${used:X}: a Mega Drive cartridge is at most 4 MB"
        ));
    }
    let fill = layout.fill.unwrap_or(0xFF);
    let size = match &o.size {
        Some(v) => {
            let n = layout::number(v).map_err(|e| format!("options.size: {e}"))?;
            usize::try_from(n).map_err(|_| format!("options.size: {n} is not a size"))?
        }
        None => used.max(0x200).next_power_of_two().max(0x2_0000),
    };
    if size < used.max(0x200) || !size.is_multiple_of(2) {
        return Err(format!(
            "options.size {size}: an even size of at least the {} bytes the program needs",
            used.max(0x200)
        ));
    }
    let mut rom = vec![fill; size];
    let mut covered = vec![false; size];
    for (name, at, bytes) in &pieces {
        let at = *at as usize;
        if let Some(i) = (at..at + bytes.len()).find(|&i| covered[i]) {
            return Err(format!("{name} overlaps earlier data at ${i:06X}"));
        }
        rom[at..at + bytes.len()].copy_from_slice(bytes);
        covered[at..at + bytes.len()].fill(true);
    }
    let mut notes = Vec::new();
    let wrote_header = covered[..0x200].iter().any(|c| *c);
    if !wrote_header {
        let find = |names: &[&str]| -> Option<i64> {
            names.iter().find_map(|n| {
                symbols
                    .iter()
                    .find(|(s, _)| s.eq_ignore_ascii_case(n))
                    .map(|(_, v)| *v)
            })
        };
        let entry = match &layout.entry {
            Some(layout::Value::Int(a)) => Some(*a),
            Some(layout::Value::Text(t)) => find(&[t]).or_else(|| rc_common::parse_number(t).ok()),
            None => find(&["reset", "start", "entry"]),
        };
        let reset = entry.ok_or(
            "no reset vector: define the label `reset` (or `start`), set `entry`, or write the vectors yourself",
        )?;
        let stack = match &o.stack {
            Some(v) => layout::number(v).map_err(|e| format!("options.stack: {e}"))?,
            None => 0x0100_0000,
        };
        let error_label = find(&["error"]);
        let default = error_label.unwrap_or(reset);
        let long = |rom: &mut Vec<u8>, i: usize, v: i64| {
            rom[i * 4..i * 4 + 4].copy_from_slice(&(v as u32).to_be_bytes())
        };
        long(&mut rom, 0, stack);
        long(&mut rom, 1, reset);
        let named: [(usize, &str); 10] = [
            (2, "bus_error"),
            (3, "address_error"),
            (4, "illegal"),
            (5, "div_zero"),
            (6, "chk"),
            (7, "trapv"),
            (8, "privilege"),
            (9, "trace"),
            (10, "line_a"),
            (11, "line_f"),
        ];
        for (i, n) in named {
            long(&mut rom, i, find(&[n]).unwrap_or(default));
        }
        for i in 12..64 {
            long(&mut rom, i, default);
        }
        for (i, n) in [(26usize, "ext"), (28, "hint"), (30, "vint")] {
            match find(&[n]) {
                Some(v) => long(&mut rom, i, v),
                None => notes.push(format!(
                    "no label `{n}`: that interrupt vector points at {}",
                    if error_label.is_some() {
                        "error"
                    } else {
                        "the reset code"
                    }
                )),
            }
        }
        let console = match o.console.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None | Some("megadrive") => "SEGA MEGA DRIVE ",
            Some("genesis") => "SEGA GENESIS    ",
            Some(other) => {
                return Err(format!("options.console \"{other}\": megadrive or genesis"));
            }
        };
        put_text(&mut rom, 0x100, 16, console);
        put_text(
            &mut rom,
            0x110,
            16,
            o.copyright.as_deref().unwrap_or("(C)RCLD 2026.NOV"),
        );
        let title = o
            .title
            .clone()
            .or_else(|| layout.name.clone())
            .unwrap_or_else(|| "RETROCLAWD".to_string());
        put_text(&mut rom, 0x120, 48, &title);
        put_text(&mut rom, 0x150, 48, &title);
        put_text(
            &mut rom,
            0x180,
            14,
            o.serial.as_deref().unwrap_or("GM 00000000-00"),
        );
        put_text(&mut rom, 0x190, 16, o.devices.as_deref().unwrap_or("J"));
        rom[0x1A0..0x1A4].copy_from_slice(&0u32.to_be_bytes());
        rom[0x1A8..0x1AC].copy_from_slice(&0x00FF_0000u32.to_be_bytes());
        rom[0x1AC..0x1B0].copy_from_slice(&0x00FF_FFFFu32.to_be_bytes());
        put_text(&mut rom, 0x1F0, 16, o.region.as_deref().unwrap_or("JUE"));
    }
    if o.fix_checksum != Some(false) {
        rom[0x1A4..0x1A8].copy_from_slice(&((size - 1) as u32).to_be_bytes());
        let mut sum = 0u16;
        for w in rom[0x200..].chunks(2) {
            sum = sum.wrapping_add(u16::from_be_bytes([w[0], *w.get(1).unwrap_or(&0)]));
        }
        rom[0x18E..0x190].copy_from_slice(&sum.to_be_bytes());
    }
    if &rom[0x100..0x104] != b"SEGA" {
        notes.push(
            "the header at $100 does not start with SEGA: a console with TMSS will refuse the ROM"
                .to_string(),
        );
    }
    let mut map = String::new();
    for (name, at, bytes) in &pieces {
        let _ = writeln!(
            map,
            "${:06X}-${:06X}  {name}",
            at,
            *at as usize + bytes.len().saturating_sub(1)
        );
    }
    let _ = writeln!(
        map,
        "{} KB{}",
        size / 1024,
        if wrote_header {
            ", header written by the sources"
        } else {
            ""
        }
    );
    Ok(Build {
        bytes: rom,
        notes,
        map,
        symbols_text: symbols_txt,
    })
}
