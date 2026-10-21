//! The images: what each machine can have and how it is made.

use crate::basic;
use crate::layout::{self, Layout, Machine};
use crate::place::{self, Piece, Range};
use crate::{Build, Program, entry};
use rcdisk::AddOptions;
use rcsnap::spectrum::{self as snap, Format, Spec};
use rctape::spectrum::{Kind, checksum_of, make_header};
use rctape::{Block, Tape};
use std::path::{Path, PathBuf};

struct Ctx<'a> {
    layout: &'a Layout,
    base: &'a Path,
    name: String,
    entry: Option<(u16, String)>,
    regions: Vec<Range>,
    reserves: Vec<Range>,
    notes: Vec<String>,
    fill: u8,
    /// What the inputs gave.
    user: Vec<Piece>,
}

/// What an emitter hands back: the file, and every piece that is in it (inputs and generated ones).
struct Made {
    bytes: Vec<u8>,
    pieces: Vec<Piece>,
}

impl Ctx<'_> {
    fn lowest(&self) -> Option<u16> {
        self.user.iter().map(|p| p.start).min()
    }

    /// The entry address; without one in the layout, the lowest address of the inputs (said in a note).
    fn need_entry(&mut self, what: &str) -> Result<(u16, String), String> {
        if let Some(e) = &self.entry {
            return Ok(e.clone());
        }
        let low = self
            .lowest()
            .ok_or("there is nothing to put in the image")?;
        self.notes.push(format!(
            "no entry in the layout: {what} starts at the lowest address, ${low:04X}"
        ));
        let e = (low, "lowest address".to_string());
        self.entry = Some(e.clone());
        Ok(e)
    }

    fn checked(&self, mut pieces: Vec<Piece>) -> Result<Vec<Piece>, String> {
        place::check(&mut pieces, &self.regions, &self.reserves)?;
        Ok(pieces)
    }

    fn read(&self, p: &str) -> Result<Vec<u8>, String> {
        let path = if Path::new(p).is_absolute() {
            PathBuf::from(p)
        } else {
            self.base.join(p)
        };
        std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
    }

    fn option_name(&self) -> String {
        self.layout
            .options
            .file_name
            .clone()
            .unwrap_or_else(|| self.name.clone())
    }
}

fn no_rom(pieces: &[Piece]) -> Result<(), String> {
    match pieces.iter().find(|p| p.start < 0x4000) {
        Some(p) => Err(format!(
            "{} ({}) is in the ROM (below $4000); a Spectrum program lives at $4000 and up",
            p.name,
            p.range()
        )),
        None => Ok(()),
    }
}

/// The program as one `.prg`: load address and bytes, with the BASIC `SYS` line if wanted.
fn c64_prg(c: &mut Ctx) -> Result<Made, String> {
    let mut pieces = c.user.clone();
    let stub = match c.layout.options.basic_stub {
        Some(b) => b,
        None => c.entry.is_some() && c.lowest() != Some(0x0801),
    };
    if stub {
        let (e, _) = c
            .entry
            .clone()
            .ok_or("basic_stub needs an entry (the address SYS jumps to)")?;
        if c.layout.options.basic_stub.is_none() {
            c.notes.push(format!(
                "added the BASIC line 10 SYS {e} at $0801 (basic_stub = false leaves it out)"
            ));
        }
        pieces.push(Piece {
            name: format!("BASIC loader (10 SYS {e})"),
            start: 0x0801,
            bytes: basic::c64_stub(e),
        });
    }
    let pieces = c.checked(pieces)?;
    let (start, image) = place::image(&pieces, c.layout.gaps, c.fill, &mut c.notes)?;
    let mut bytes = start.to_le_bytes().to_vec();
    bytes.extend(image);
    Ok(Made { bytes, pieces })
}

fn extra_files(c: &Ctx, disk: &mut dyn rcdisk::Disk) -> Result<(), String> {
    for f in &c.layout.file {
        let data = c.read(&f.path)?;
        let opts = AddOptions {
            name: f.name.clone(),
            kind: f.kind.clone(),
            user: f.user.unwrap_or(0),
            ..AddOptions::default()
        };
        disk.add(&f.path, &data, &opts)?;
    }
    Ok(())
}

fn c64_disk(c: &mut Ctx, format: &str) -> Result<Made, String> {
    let prg = c64_prg(c)?;
    let title = c
        .layout
        .options
        .disk_name
        .clone()
        .unwrap_or_else(|| c.name.clone());
    let id = c
        .layout
        .options
        .disk_id
        .clone()
        .unwrap_or_else(|| "01".to_string());
    let mut disk = rcdisk::create(format, Some(&title), Some(&id))?;
    let opts = AddOptions {
        name: Some(c.option_name()),
        kind: Some("prg".to_string()),
        ..AddOptions::default()
    };
    disk.add(&c.option_name(), &prg.bytes, &opts)?;
    extra_files(c, disk.as_mut())?;
    Ok(Made {
        bytes: disk.to_bytes(),
        pieces: prg.pieces,
    })
}

/// An 8K or 16K plain cartridge at 0x8000, with the start vectors and `CBM80` signature the KERNAL looks for.
fn c64_crt(c: &mut Ctx) -> Result<Made, String> {
    let mut pieces = c.user.clone();
    if c.layout.options.cart_header.unwrap_or(true) {
        let (e, _) = c.need_entry("the cartridge")?;
        let mut h = e.to_le_bytes().to_vec(); // cold start
        h.extend_from_slice(&e.to_le_bytes()); // warm start
        h.extend_from_slice(&[0xC3, 0xC2, 0xCD, 0x38, 0x30]); // CBM80
        pieces.push(Piece {
            name: "cartridge header".to_string(),
            start: 0x8000,
            bytes: h,
        });
    }
    let pieces = c.checked(pieces)?;
    if let Some(p) = pieces.iter().find(|p| p.start < 0x8000 || p.end() > 0xC000) {
        return Err(format!(
            "{} ({}) is outside the cartridge ROM ($8000-$BFFF)",
            p.name,
            p.range()
        ));
    }
    let top = pieces.iter().map(Piece::end).max().unwrap_or(0x8000);
    let size = if top <= 0xA000 { 0x2000 } else { 0x4000 };
    let mut rom = vec![c.fill; size];
    for p in &pieces {
        let at = (p.start - 0x8000) as usize;
        rom[at..at + p.bytes.len()].copy_from_slice(&p.bytes);
    }
    let crt = rcsnap::carts::crt_from_raw(&c.name, &rom)?;
    Ok(Made {
        bytes: c64_cartridge::write(&crt),
        pieces,
    })
}

fn bin(c: &mut Ctx) -> Result<Made, String> {
    let pieces = c.checked(c.user.clone())?;
    let (start, bytes) = place::image(&pieces, c.layout.gaps, c.fill, &mut c.notes)?;
    c.notes
        .push(format!("the .bin starts at address ${start:04X}"));
    Ok(Made { bytes, pieces })
}

fn atari_xex(c: &mut Ctx) -> Result<Made, String> {
    let mut pieces = c.user.clone();
    if let Some((entry, _)) = c.entry {
        pieces.push(Piece {
            name: "RUNAD".into(),
            start: 0x02e0,
            bytes: entry.to_le_bytes().to_vec(),
        });
    }
    let pieces = c.checked(pieces)?;
    let segments = pieces
        .iter()
        .map(|p| atari_media::Segment {
            start: p.start,
            bytes: p.bytes.clone(),
        })
        .collect();
    let bytes = crate::atari::create(segments)?;
    Ok(Made { bytes, pieces })
}

fn spectrum_snapshot(c: &mut Ctx, format: Format) -> Result<Made, String> {
    let pieces = c.checked(c.user.clone())?;
    no_rom(&pieces)?;
    let (e, _) = c.need_entry("the snapshot")?;
    let o = &c.layout.options;
    let mut spec = Spec::blank48(e);
    for p in &pieces {
        spec.poke(p.start, &p.bytes)?;
    }
    if let Some(sp) = &o.sp {
        spec.cpu.sp = layout::address(sp, "options.sp")?;
    }
    if let Some(b) = o.border {
        if b > 7 {
            return Err("options.border takes 0-7".to_string());
        }
        spec.border = b;
    }
    if let Some(im) = o.im {
        if im > 2 {
            return Err("options.im takes 0-2".to_string());
        }
        spec.cpu.im = im;
    }
    let ei = o.interrupts.unwrap_or(false);
    spec.cpu.iff1 = ei;
    spec.cpu.iff2 = ei;
    let stack = spec.cpu.sp.wrapping_sub(2);
    if format == Format::Sna
        && pieces
            .iter()
            .any(|p| (p.start as u32) < stack as u32 + 2 && p.end() > stack as u32)
    {
        c.notes.push(format!("the program covers ${stack:04X}-${:04X}, where a .sna stores the start address on the stack; move SP (options.sp)", stack.wrapping_add(1)));
    }
    c.notes.extend(
        snap::losses(&spec, format)?
            .into_iter()
            .filter(|n| !n.contains("48K .sna keeps the PC")),
    );
    Ok(Made {
        bytes: snap::write(&spec, format)?,
        pieces,
    })
}

/// `.tap` / `.tzx`: a BASIC loader that loads each block of code and jumps to the entry, then the blocks.
fn spectrum_tape(c: &mut Ctx, tap: bool) -> Result<Made, String> {
    let pieces = c.checked(c.user.clone())?;
    no_rom(&pieces)?;
    let runs = place::runs(&pieces);
    let pause = c.layout.options.pause.unwrap_or(1000);
    let loader = c.layout.options.loader.unwrap_or(true);
    let header_name = if c.name.chars().count() > 10 {
        c.notes.push(format!(
            "the tape name \"{}\" is cut to 10 characters",
            c.name
        ));
        c.name.chars().take(10).collect::<String>()
    } else {
        c.name.clone()
    };
    let framed = |flag: u8, data: &[u8]| {
        let mut v = vec![flag];
        v.extend_from_slice(data);
        v.push(checksum_of(&v));
        v
    };
    let mut tape = Tape::new();
    let mut screen_data = None;
    if let Some(path) = &c.layout.options.screen {
        let data = c.read(path)?;
        if data.len() != 6912 {
            return Err(format!(
                "{path}: a screen is 6912 bytes, this is {}",
                data.len()
            ));
        }
        if let Some(p) = pieces.iter().find(|p| p.start < 0x5B00) {
            return Err(format!(
                "{} ({}) is in the screen memory ($4000-$5AFF); drop options.screen or move it",
                p.name,
                p.range()
            ));
        }
        screen_data = Some(data);
    }
    if loader {
        let (e, _) = c.need_entry("RANDOMIZE USR")?;
        let low = runs
            .first()
            .map(|r| r.start)
            .ok_or("there is nothing to put on the tape")?;
        if low < 0x6000 {
            return Err(format!(
                "the BASIC loader needs the memory below $6000, but code starts at ${low:04X}; \
                 move the code up, or set options.loader = false for a tape of bare code blocks"
            ));
        }
        let program = basic::spectrum_loader(low - 1, screen_data.is_some(), runs.len(), e);
        tape.blocks.push(Block::standard(
            &make_header(
                Kind::Program,
                &header_name,
                program.len() as u16,
                10,
                program.len() as u16,
            ),
            pause,
        ));
        tape.blocks
            .push(Block::standard(&framed(0xFF, &program), pause));
    } else if c.entry.is_some() {
        c.notes.push(
            "loader = false: the entry address is not used by a tape of bare code blocks"
                .to_string(),
        );
    }
    if let Some(data) = screen_data {
        tape.blocks.push(Block::standard(
            &make_header(Kind::Code, &header_name, 6912, 0x4000, 32768),
            pause,
        ));
        tape.blocks
            .push(Block::standard(&framed(0xFF, &data), pause));
    }
    for r in &runs {
        tape.blocks.push(Block::standard(
            &make_header(
                Kind::Code,
                &header_name,
                r.bytes.len() as u16,
                r.start,
                32768,
            ),
            pause,
        ));
        tape.blocks
            .push(Block::standard(&framed(0xFF, &r.bytes), pause));
    }
    let bytes = if tap {
        let (b, notes) = rctape::to_tap(&tape, false)?;
        c.notes.extend(notes);
        b
    } else {
        tape.to_bytes()
    };
    Ok(Made { bytes, pieces })
}

/// A CP/M disk (CPC with AMSDOS, +3 with +3DOS) holding the program as one binary file.
fn cpm_disk(c: &mut Ctx, format: &str, header: &str) -> Result<Made, String> {
    let pieces = c.checked(c.user.clone())?;
    if header == "plus3dos" {
        no_rom(&pieces)?;
    }
    let (load, image) = place::image(&pieces, c.layout.gaps, c.fill, &mut c.notes)?;
    let (e, _) = c.need_entry("the program")?;
    let mut name = c.option_name().to_ascii_uppercase();
    if header == "amsdos" && !name.contains('.') {
        name.push_str(".BIN");
    }
    let mut disk = rcdisk::create(format, None, None)?;
    let opts = AddOptions {
        name: Some(name),
        kind: Some("bin".to_string()),
        header: Some(header.to_string()),
        load: Some(load),
        entry: Some(e),
        ..AddOptions::default()
    };
    disk.add(&c.option_name(), &image, &opts)?;
    extra_files(c, disk.as_mut())?;
    Ok(Made {
        bytes: disk.to_bytes(),
        pieces,
    })
}

fn can_make(machine: Machine) -> &'static str {
    match machine {
        Machine::C64 => ".prg .d64 .d81 .crt .bin",
        Machine::Atari => ".xex .bin",
        Machine::Spectrum => ".tap .tzx .sna .z80 .szx .dsk (+3) .bin",
        Machine::Cpc => ".dsk .bin",
        Machine::Generic => ".bin",
    }
}

pub fn emit(
    layout: &Layout,
    base: &Path,
    output: &Path,
    program: Program,
) -> Result<Build, String> {
    let ext = output
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let name = layout
        .name
        .clone()
        .or_else(|| output.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .ok_or("give the program a name (name = \"...\")")?;
    let entry = entry(layout, &program)?;
    let mut c = Ctx {
        layout,
        base,
        name,
        entry: entry.clone(),
        regions: place::regions(layout)?,
        reserves: place::reserves(layout)?,
        notes: program.notes.clone(),
        fill: layout.fill.unwrap_or(0),
        user: program.pieces.clone(),
    };
    let made = match (layout.machine, ext.as_str()) {
        (_, "bin") => bin(&mut c),
        (Machine::Atari, "xex") => atari_xex(&mut c),
        (Machine::C64, "prg") => c64_prg(&mut c),
        (Machine::C64, "d64") => c64_disk(&mut c, "d64"),
        (Machine::C64, "d81") => c64_disk(&mut c, "d81"),
        (Machine::C64, "crt") => c64_crt(&mut c),
        (Machine::Spectrum, "tap") => spectrum_tape(&mut c, true),
        (Machine::Spectrum, "tzx") => spectrum_tape(&mut c, false),
        (Machine::Spectrum, "sna") => spectrum_snapshot(&mut c, Format::Sna),
        (Machine::Spectrum, "z80") => spectrum_snapshot(&mut c, Format::Z80),
        (Machine::Spectrum, "szx") => spectrum_snapshot(&mut c, Format::Szx),
        (Machine::Spectrum, "dsk") => cpm_disk(&mut c, "dsk-plus3", "plus3dos"),
        (Machine::Cpc, "dsk") => cpm_disk(&mut c, "dsk", "amsdos"),
        (m, other) => {
            return Err(format!(
                "a {} program cannot be written as \".{other}\"; it can be: {}",
                m.name(),
                can_make(m)
            ));
        }
    }?;
    let title = format!(
        "{} -> {} ({})",
        c.name,
        output.display(),
        layout.machine.name()
    );
    let map = place::map_text(&title, c.entry.clone(), &made.pieces, &c.regions);
    Ok(Build {
        bytes: made.bytes,
        notes: c.notes,
        map,
        symbols_text: program.symbols_text,
    })
}
