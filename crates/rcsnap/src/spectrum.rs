//! ZX Spectrum snapshots (`.z80`, `.sna`, `.szx`) in one format-independent model, so any of them can be turned
//! into any other. The three format crates each have their own `Snapshot` type with the same shape; this is the
//! common middle.

use z80::Cpu;

pub const BANK_LEN: usize = 0x4000;
/// The RAM of a 48K machine: 0x4000..=0xFFFF.
pub const RAM48_LEN: usize = 0xC000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ram {
    /// 48K: addresses 0x4000..=0xFFFF in one piece.
    K48(Vec<u8>),
    /// 128K family: the eight 16K banks.
    Banks(Vec<Vec<u8>>),
}

/// 128K paging (the last value written to port 0x7FFD).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct P128 {
    pub rom_bank: u8,
    pub ram_bank: u8,
    pub shadow_screen: bool,
    pub locked: bool,
}

/// +2A/+3 paging (ports 0x7FFD and 0x1FFD).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct P2A3 {
    pub ram_bank: u8,
    pub shadow_screen: bool,
    pub rom_bit_lo: bool,
    pub locked: bool,
    pub special_mode: bool,
    pub bits_1_2: u8,
    pub disk_motor: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ay {
    pub current_register: u8,
    pub registers: [u8; 16],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub cpu: Cpu,
    pub border: u8,
    pub ram: Ram,
    pub p128: Option<P128>,
    pub p2a3: Option<P2A3>,
    pub ay: Option<Ay>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Z80,
    Sna,
    Szx,
}

impl Format {
    pub fn from_extension(path: &str) -> Option<Format> {
        match path.rsplit_once('.')?.1.to_ascii_lowercase().as_str() {
            "z80" => Some(Format::Z80),
            "sna" => Some(Format::Sna),
            "szx" => Some(Format::Szx),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Z80 => ".z80",
            Format::Sna => ".sna",
            Format::Szx => ".szx",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Spectrum48,
    Spectrum128,
    Spectrum2A3,
}

impl Model {
    pub fn name(self) -> &'static str {
        match self {
            Model::Spectrum48 => "ZX Spectrum 48K",
            Model::Spectrum128 => "ZX Spectrum 128K / +2",
            Model::Spectrum2A3 => "ZX Spectrum +2A / +3",
        }
    }
}

impl Spec {
    /// A 48K machine with empty RAM, interrupts off, mode 1, starting at `pc`.
    pub fn blank48(pc: u16) -> Spec {
        let mut cpu = Cpu::new();
        cpu.pc = pc;
        cpu.im = 1;
        Spec {
            cpu,
            border: 7,
            ram: Ram::K48(vec![0; RAM48_LEN]),
            p128: None,
            p2a3: None,
            ay: None,
        }
    }

    pub fn model(&self) -> Model {
        if self.p2a3.is_some() {
            Model::Spectrum2A3
        } else if self.p128.is_some() {
            Model::Spectrum128
        } else {
            Model::Spectrum48
        }
    }

    /// Copies `data` into RAM at the CPU address `address` (48K machines: 0x4000 and up).
    pub fn poke(&mut self, address: u16, data: &[u8]) -> Result<(), String> {
        let Ram::K48(ram) = &mut self.ram else {
            return Err("poke works on 48K snapshots only".to_string());
        };
        let start = address as usize;
        if start < 0x4000 {
            return Err(format!(
                "{address:#06X} is in the ROM; RAM starts at 0x4000"
            ));
        }
        let end = start + data.len();
        if end > 0x10000 {
            return Err(format!(
                "{} bytes at {address:#06X} run past the end of memory (0xFFFF)",
                data.len()
            ));
        }
        ram[start - 0x4000..end - 0x4000].copy_from_slice(data);
        Ok(())
    }

    /// What the CPU sees from 0x4000 to 0xFFFF (the ROM is not part of a snapshot). For the 128K machines the
    /// banks at 0x4000 (5), 0x8000 (2) and the one paged in at 0xC000.
    pub fn address_space(&self) -> Vec<u8> {
        match &self.ram {
            Ram::K48(r) => r.clone(),
            Ram::Banks(b) => {
                let top = match (&self.p128, &self.p2a3) {
                    (Some(p), _) => p.ram_bank,
                    (_, Some(p)) => p.ram_bank,
                    _ => 0,
                } as usize
                    & 7;
                [&b[5][..], &b[2][..], &b[top][..]].concat()
            }
        }
    }

    /// Reads `len` bytes of the CPU address space from `start` (0x4000 and up).
    pub fn read(&self, start: u16, len: usize) -> Result<Vec<u8>, String> {
        if start < 0x4000 {
            return Err(format!(
                "{start:#06X} is in the ROM; a snapshot holds 0x4000 and up"
            ));
        }
        let space = self.address_space();
        let from = start as usize - 0x4000;
        space
            .get(from..from + len)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| format!("{len} bytes from {start:#06X} run past 0xFFFF"))
    }
}

macro_rules! conversions {
    ($m:ident, $to_ram:ident, $from_ram:ident) => {
        fn $to_ram(blocks: &[$m::MemoryBlock]) -> Result<Ram, String> {
            let mut k48: Option<Vec<u8>> = None;
            let mut banks: Option<Vec<Vec<u8>>> = None;
            for b in blocks {
                match b {
                    $m::MemoryBlock::Absolute { address, data } => {
                        let ram = k48.get_or_insert_with(|| vec![0; RAM48_LEN]);
                        let start = (*address as usize).checked_sub(0x4000).ok_or_else(|| {
                            format!("a memory block at {address:#06X} is in the ROM")
                        })?;
                        let end = start + data.len();
                        if end > RAM48_LEN {
                            return Err(format!(
                                "a memory block at {address:#06X} runs past 0xFFFF"
                            ));
                        }
                        ram[start..end].copy_from_slice(data);
                    }
                    $m::MemoryBlock::RamBank { bank, data } => {
                        if *bank > 7 || data.len() != BANK_LEN {
                            return Err(format!("RAM bank {bank} is not a 16K bank 0-7"));
                        }
                        let banks = banks.get_or_insert_with(|| vec![vec![0; BANK_LEN]; 8]);
                        banks[*bank as usize].copy_from_slice(data);
                    }
                }
            }
            match (k48, banks) {
                (Some(r), None) => Ok(Ram::K48(r)),
                (None, Some(b)) => Ok(Ram::Banks(b)),
                (None, None) => Err("the snapshot holds no memory".to_string()),
                (Some(_), Some(_)) => Err("the snapshot mixes 48K and banked memory".to_string()),
            }
        }

        fn $from_ram(ram: &Ram) -> Vec<$m::MemoryBlock> {
            match ram {
                Ram::K48(r) => vec![$m::MemoryBlock::Absolute {
                    address: 0x4000,
                    data: r.clone(),
                }],
                Ram::Banks(b) => b
                    .iter()
                    .enumerate()
                    .map(|(i, d)| $m::MemoryBlock::RamBank {
                        bank: i as u8,
                        data: d.clone(),
                    })
                    .collect(),
            }
        }
    };
}

conversions!(z80_snapshot, z80_ram, z80_blocks);
conversions!(sna_snapshot, sna_ram, sna_blocks);
conversions!(szx_snapshot, szx_ram, szx_blocks);

macro_rules! p128_conv {
    ($m:ident) => {
        impl From<$m::PagingState> for P128 {
            fn from(p: $m::PagingState) -> P128 {
                P128 {
                    rom_bank: p.rom_bank,
                    ram_bank: p.ram_bank,
                    shadow_screen: p.shadow_screen,
                    locked: p.locked,
                }
            }
        }
        impl From<P128> for $m::PagingState {
            fn from(p: P128) -> Self {
                $m::PagingState {
                    rom_bank: p.rom_bank,
                    ram_bank: p.ram_bank,
                    shadow_screen: p.shadow_screen,
                    locked: p.locked,
                }
            }
        }
    };
}

macro_rules! p2a3_conv {
    ($m:ident) => {
        impl From<$m::Paging2A3State> for P2A3 {
            fn from(p: $m::Paging2A3State) -> P2A3 {
                P2A3 {
                    ram_bank: p.ram_bank,
                    shadow_screen: p.shadow_screen,
                    rom_bit_lo: p.rom_bit_lo,
                    locked: p.locked,
                    special_mode: p.special_mode,
                    bits_1_2: p.bits_1_2,
                    disk_motor: p.disk_motor,
                }
            }
        }
        impl From<P2A3> for $m::Paging2A3State {
            fn from(p: P2A3) -> Self {
                $m::Paging2A3State {
                    ram_bank: p.ram_bank,
                    shadow_screen: p.shadow_screen,
                    rom_bit_lo: p.rom_bit_lo,
                    locked: p.locked,
                    special_mode: p.special_mode,
                    bits_1_2: p.bits_1_2,
                    disk_motor: p.disk_motor,
                }
            }
        }
    };
}

p128_conv!(z80_snapshot);
p128_conv!(sna_snapshot);
p128_conv!(szx_snapshot);
p2a3_conv!(z80_snapshot);
p2a3_conv!(szx_snapshot);

/// Reads a snapshot of a known format.
pub fn parse(format: Format, bytes: &[u8]) -> Result<Spec, String> {
    match format {
        Format::Z80 => {
            let s = z80_snapshot::parse(bytes).map_err(|e| e.to_string())?;
            Ok(Spec {
                ram: z80_ram(&s.memory)?,
                cpu: s.cpu,
                border: s.border_color,
                p128: s.paging.map(Into::into),
                p2a3: s.paging_2a3.map(Into::into),
                ay: None,
            })
        }
        Format::Sna => {
            let s = sna_snapshot::parse(bytes).map_err(|e| e.to_string())?;
            Ok(Spec {
                ram: sna_ram(&s.memory)?,
                cpu: s.cpu,
                border: s.border_color,
                p128: s.paging.map(Into::into),
                p2a3: None,
                ay: None,
            })
        }
        Format::Szx => {
            let s = szx_snapshot::parse(bytes).map_err(|e| e.to_string())?;
            Ok(Spec {
                ram: szx_ram(&s.memory)?,
                cpu: s.cpu,
                border: s.border_color,
                p128: s.paging.map(Into::into),
                p2a3: s.paging_2a3.map(Into::into),
                ay: s.ay.map(|a| Ay {
                    current_register: a.current_register,
                    registers: a.registers,
                }),
            })
        }
    }
}

/// What a format cannot keep of this snapshot, one line each; `Err` when it cannot hold the machine at all.
pub fn losses(spec: &Spec, to: Format) -> Result<Vec<String>, String> {
    let mut notes = Vec::new();
    let cpu = &spec.cpu;
    match to {
        Format::Sna => {
            if spec.p2a3.is_some() {
                return Err("a .sna cannot hold a +2A/+3 snapshot; use .z80 or .szx".to_string());
            }
            if cpu.iff1 != cpu.iff2 {
                notes.push(
                    "a .sna keeps one interrupt flip-flop (IFF2); IFF1 is set equal to it"
                        .to_string(),
                );
            }
            if spec.p128.is_none() {
                notes.push(format!(
                    "a 48K .sna keeps the PC on the stack: the two bytes at {:#06X} are overwritten with it",
                    spec.cpu.sp.wrapping_sub(2)
                ));
            }
            if spec.p128.is_some_and(|p| p.locked) {
                notes.push("a .sna cannot record the 128K paging lock".to_string());
            }
        }
        Format::Z80 => {}
        Format::Szx => {}
    }
    if to != Format::Szx && spec.ay.is_some() {
        notes.push(format!("{} does not keep the AY register state", to.name()));
    }
    if to != Format::Szx && cpu.halted {
        notes.push(format!(
            "{} does not record that the CPU was halted",
            to.name()
        ));
    }
    Ok(notes)
}

/// Writes the snapshot in a format.
pub fn write(spec: &Spec, format: Format) -> Result<Vec<u8>, String> {
    losses(spec, format)?;
    match format {
        Format::Z80 => z80_snapshot::write(&z80_snapshot::Snapshot {
            cpu: spec.cpu,
            border_color: spec.border,
            memory: z80_blocks(&spec.ram),
            paging: spec.p128.map(Into::into),
            paging_2a3: spec.p2a3.map(Into::into),
        })
        .map_err(|e| e.to_string()),
        Format::Sna => sna_snapshot::write(&sna_snapshot::Snapshot {
            cpu: spec.cpu,
            border_color: spec.border,
            memory: sna_blocks(&spec.ram),
            paging: spec.p128.map(Into::into),
        })
        .map_err(|e| e.to_string()),
        Format::Szx => Ok(szx_snapshot::write(&szx_snapshot::Snapshot {
            cpu: spec.cpu,
            border_color: spec.border,
            memory: szx_blocks(&spec.ram),
            paging: spec.p128.map(Into::into),
            paging_2a3: spec.p2a3.map(Into::into),
            ay: spec.ay.map(|a| szx_snapshot::AyState {
                current_register: a.current_register,
                registers: a.registers,
            }),
        })),
    }
}

/// The `.z80` file version (1, 2 or 3), from the header.
pub fn z80_version(bytes: &[u8]) -> Option<u8> {
    if bytes.len() < 32 {
        return None;
    }
    if u16::from_le_bytes([bytes[6], bytes[7]]) != 0 {
        return Some(1);
    }
    match u16::from_le_bytes([bytes[30], bytes[31]]) {
        23 => Some(2),
        54 | 55 => Some(3),
        _ => None,
    }
}
