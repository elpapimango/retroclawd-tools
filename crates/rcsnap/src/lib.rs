//! Snapshots and cartridges: look inside, convert between formats, build a snapshot from a program.
//!
//! * ZX Spectrum `.z80`, `.sna`, `.szx`: any into any ([`spectrum`]).
//! * ZX81 `.p`, `.81`, `.p81` ([`zx81`]).
//! * Amstrad CPC `.sna` (versions 1-3).
//! * C64 `.crt` and CPC Plus / GX4000 `.cpr` cartridges ([`carts`]).

pub mod carts;
pub mod spectrum;
pub mod zx81;

use c64_cartridge::Crt;
use cpc_cartridge::Cartridge;
use spectrum::{Format, Ram, Spec};

/// A snapshot or cartridge file, read.
pub enum Image {
    Spectrum {
        format: Format,
        version: Option<u8>,
        spec: Spec,
    },
    Zx81 {
        p81: bool,
        content: zx81::P81,
    },
    CpcSna(cpc_snapshot::Snapshot),
    Crt(Crt),
    Cpr(Cartridge),
}

fn extension(path: &str) -> String {
    path.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Reads a file, deciding the kind from its content (signatures) and then from the extension.
pub fn open(path: &str, bytes: &[u8]) -> Result<Image, String> {
    let ext = extension(path);
    let err = |what: &str, e: &dyn std::fmt::Display| format!("{path}: not a valid {what}: {e}");
    if bytes.starts_with(b"ZXST") {
        let spec = spectrum::parse(Format::Szx, bytes).map_err(|e| err(".szx", &e))?;
        return Ok(Image::Spectrum {
            format: Format::Szx,
            version: None,
            spec,
        });
    }
    if bytes.starts_with(b"MV - SNA") {
        let s = cpc_snapshot::parse(bytes).map_err(|e| err("CPC .sna", &e))?;
        return Ok(Image::CpcSna(s));
    }
    if bytes.starts_with(b"C64 CARTRIDGE   ") {
        return c64_cartridge::parse(bytes)
            .map(Image::Crt)
            .map_err(|e| err(".crt", &e));
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"AMS!") {
        return cpc_cartridge::parse(bytes)
            .map(Image::Cpr)
            .map_err(|e| err(".cpr", &e));
    }
    match ext.as_str() {
        "z80" | "sna" => {
            let format = if ext == "z80" {
                Format::Z80
            } else {
                Format::Sna
            };
            let spec = spectrum::parse(format, bytes).map_err(|e| err(format.name(), &e))?;
            let version = (format == Format::Z80)
                .then(|| spectrum::z80_version(bytes))
                .flatten();
            Ok(Image::Spectrum {
                format,
                version,
                spec,
            })
        }
        "p" | "81" | "p81" => {
            let p81 = ext == "p81";
            let content = zx81::parse(bytes, p81).map_err(|e| err("ZX81 program", &e))?;
            Ok(Image::Zx81 { p81, content })
        }
        _ => Err(format!(
            "{path}: not a snapshot or cartridge I know (.z80 .sna .szx .p .81 .p81, CPC .sna, .crt, .cpr)"
        )),
    }
}

/// Text for a listing: ordered fields, then lines of detail (memory blocks, chips).
pub struct Info {
    pub fields: Vec<(String, String)>,
    pub items: Vec<String>,
}

impl Info {
    fn new() -> Info {
        Info {
            fields: Vec::new(),
            items: Vec::new(),
        }
    }
    fn field(&mut self, k: &str, v: impl Into<String>) {
        self.fields.push((k.to_string(), v.into()));
    }
}

fn hex16(v: u16) -> String {
    format!("{v:04X}")
}

fn registers(info: &mut Info, cpu: &z80::Cpu) {
    info.field("pc", hex16(cpu.pc));
    info.field("sp", hex16(cpu.sp));
    info.field("af", hex16(cpu.af));
    info.field("bc", hex16(cpu.bc));
    info.field("de", hex16(cpu.de));
    info.field("hl", hex16(cpu.hl));
    info.field("af'", hex16(cpu.af_));
    info.field("bc'", hex16(cpu.bc_));
    info.field("de'", hex16(cpu.de_));
    info.field("hl'", hex16(cpu.hl_));
    info.field("ix", hex16(cpu.ix));
    info.field("iy", hex16(cpu.iy));
    info.field("i", format!("{:02X}", cpu.i));
    info.field("r", format!("{:02X}", cpu.r));
    info.field("im", cpu.im.to_string());
    info.field("iff1", cpu.iff1.to_string());
    info.field("iff2", cpu.iff2.to_string());
    info.field("halted", cpu.halted.to_string());
}

pub fn info(image: &Image, file_len: usize) -> Info {
    let mut i = Info::new();
    i.field("file_bytes", file_len.to_string());
    match image {
        Image::Spectrum {
            format,
            version,
            spec,
        } => {
            i.field(
                "format",
                match version {
                    Some(v) => format!("{} version {v}", format.name()),
                    None => format.name().to_string(),
                },
            );
            i.field("machine", spec.model().name());
            registers(&mut i, &spec.cpu);
            i.field("border", spec.border.to_string());
            if let Some(p) = spec.p128 {
                i.field(
                    "paging",
                    format!(
                        "ROM {}, RAM bank {} at 0xC000, screen {}{}",
                        p.rom_bank,
                        p.ram_bank,
                        if p.shadow_screen {
                            "shadow (bank 7)"
                        } else {
                            "normal (bank 5)"
                        },
                        if p.locked { ", locked" } else { "" }
                    ),
                );
            }
            if let Some(p) = spec.p2a3 {
                i.field(
                    "paging",
                    format!(
                        "{}RAM bank {} at 0xC000, ROM low bit {}, 0x1FFD bits 1-2 {}, screen {}{}, disk motor {}",
                        if p.special_mode { "special all-RAM mode, " } else { "" },
                        p.ram_bank,
                        p.rom_bit_lo as u8,
                        p.bits_1_2,
                        if p.shadow_screen { "shadow (bank 7)" } else { "normal (bank 5)" },
                        if p.locked { ", locked" } else { "" },
                        if p.disk_motor { "on" } else { "off" }
                    ),
                );
            }
            if let Some(a) = spec.ay {
                i.field("ay_selected_register", a.current_register.to_string());
                i.field(
                    "ay_registers",
                    a.registers
                        .iter()
                        .map(|b| format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            match &spec.ram {
                Ram::K48(r) => {
                    i.field(
                        "memory",
                        format!("48K RAM, 0x4000-0xFFFF ({} bytes)", r.len()),
                    );
                    i.items.push(format!(
                        "0x4000-0xFFFF  {:>5} bytes non-zero",
                        r.iter().filter(|&&b| b != 0).count()
                    ));
                }
                Ram::Banks(b) => {
                    i.field("memory", "128K RAM, 8 banks of 16K");
                    for (n, bank) in b.iter().enumerate() {
                        i.items.push(format!(
                            "bank {n}  {:>5} bytes non-zero",
                            bank.iter().filter(|&&x| x != 0).count()
                        ));
                    }
                }
            }
        }
        Image::Zx81 { p81, content } => {
            i.field("format", if *p81 { ".p81" } else { ".p / .81" });
            if let Some(n) = &content.name {
                i.field("name", n.clone());
            }
            i.field("data_bytes", content.data.len().to_string());
            i.field("loads_at", "4009");
            if let Some(f) = zx81::facts(&content.data) {
                i.field("d_file", hex16(f.d_file));
                i.field("vars", hex16(f.vars));
                i.field("e_line", hex16(f.e_line));
                i.field("basic_lines", f.lines.to_string());
                if let (Some(a), Some(b)) = (f.first_line, f.last_line) {
                    i.field("line_range", format!("{a}-{b}"));
                }
            }
        }
        Image::CpcSna(s) => {
            i.field("format", format!("CPC .sna version {}", s.version));
            i.field(
                "machine",
                match s.cpc_model {
                    Some(0) => "CPC464".to_string(),
                    Some(1) => "CPC664".to_string(),
                    Some(2) => "CPC6128".to_string(),
                    Some(3) => "CPC Plus".to_string(),
                    Some(n) => format!("model {n}"),
                    None => "not recorded (version 1)".to_string(),
                },
            );
            registers(&mut i, &s.cpu);
            i.field("ram", format!("{}K", s.ram.len() / 1024));
            i.field(
                "gate_array_rom_config",
                format!("{:02X}", s.gate_array.rom_config),
            );
            i.field(
                "gate_array_ram_config",
                format!("{:02X}", s.gate_array.ram_config),
            );
            i.field("upper_rom", s.upper_rom.to_string());
            i.field(
                "pens",
                s.gate_array
                    .pen_colors
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            i.field(
                "crtc_registers",
                s.crtc
                    .registers
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            i.field(
                "ppi",
                format!(
                    "A={:02X} B={:02X} C={:02X} control={:02X}",
                    s.ppi.port_a, s.ppi.port_b, s.ppi.port_c, s.ppi.control
                ),
            );
            i.field(
                "ay_registers",
                s.ay.registers
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            if let Some(v3) = &s.version3 {
                i.field("crtc_type", v3.crtc_type.to_string());
            }
        }
        Image::Crt(c) => {
            i.field(
                "format",
                format!(".crt version {}.{:02}", c.version >> 8, c.version & 0xFF),
            );
            i.field("name", c.name.clone());
            i.field("hardware", carts::crt_type_name(c.hardware_type));
            i.field("exrom", carts::line_name(c.exrom_level));
            i.field("game", carts::line_name(c.game_level));
            i.field("chips", c.chips.len().to_string());
            for (n, ch) in c.chips.iter().enumerate() {
                i.items.push(format!(
                    "{:>3}  {:<5} bank {:<3} at {:04X}  {} bytes",
                    n + 1,
                    carts::chip_kind(ch.kind),
                    ch.bank,
                    ch.load_address,
                    ch.data.len()
                ));
            }
        }
        Image::Cpr(c) => {
            i.field("format", ".cpr (RIFF / AMS!)");
            i.field("pages", c.pages.len().to_string());
            i.field(
                "rom_bytes",
                (c.pages.len() * cpc_cartridge::PAGE_SIZE).to_string(),
            );
            for p in &c.pages {
                i.items.push(format!(
                    "page {:>2}  {} bytes, {} non-zero",
                    p.bank,
                    p.data.len(),
                    p.data.iter().filter(|&&b| b != 0).count()
                ));
            }
        }
    }
    i
}
