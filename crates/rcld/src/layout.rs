//! The layout file (TOML): what goes into the image, where it may live, and what the image is.
//!
//! ```toml
//! machine = "c64"
//! name = "HELLO"
//! entry = "start"
//!
//! [[input]]
//! asm = ["main.asm", "player.asm"]     # assembled together as one program
//!
//! [[input]]
//! bin = "music.bin"
//! at = 0x4000
//!
//! [[region]]
//! name = "ram"
//! start = 0x0801
//! end = 0x9FFF
//!
//! [options]
//! basic_stub = true
//! ```

use serde::Deserialize;
use std::collections::BTreeMap;

/// An address or number written as a TOML integer (`0x8000` works) or as text (`"$8000"`, `"start"`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Value {
    Int(i64),
    Text(String),
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Machine {
    C64,
    Atari,
    Spectrum,
    Cpc,
    /// The Atari ST: one 68000 program, written as a GEMDOS `.PRG`.
    #[serde(rename = "atari-st")]
    AtariSt,
    /// No machine: raw images only, and the CPU has to be named.
    Generic,
}

impl Machine {
    pub fn name(self) -> &'static str {
        match self {
            Machine::C64 => "c64",
            Machine::Atari => "atari",
            Machine::Spectrum => "spectrum",
            Machine::Cpc => "cpc",
            Machine::AtariSt => "atari-st",
            Machine::Generic => "generic",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Gaps {
    /// A hole between two pieces of a one-piece image is an error.
    #[default]
    Error,
    /// The hole is filled with the `fill` byte (and said in a note).
    Fill,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// Assembler sources, assembled together in this order as one program.
    pub asm: Option<OneOrMany>,
    /// A raw binary, placed at `at`.
    pub bin: Option<String>,
    pub at: Option<Value>,
    /// A `.prg`: its first two bytes are the load address.
    pub prg: Option<String>,
    /// A symbol file whose names the sources can use (`NAME = $1234` or VICE `al C:1234 .NAME`).
    pub symbols: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub fn list(&self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s.clone()],
            OneOrMany::Many(v) => v.clone(),
        }
    }
}

/// Where pieces may be. With any region defined, every piece has to lie wholly inside one.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub name: String,
    pub start: Value,
    pub end: Value,
}

/// A range nothing may occupy (the system's own use of memory).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reserve {
    pub start: Value,
    pub end: Value,
    pub why: Option<String>,
}

/// An extra file for a disk image.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtraFile {
    pub path: String,
    pub name: Option<String>,
    /// Commodore: `prg`, `seq` or `usr`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// CP/M: user area.
    pub user: Option<u8>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    /// C64 `.prg` / disk: put a `10 SYS <entry>` BASIC line at 0x0801 so `RUN` starts the program.
    pub basic_stub: Option<bool>,
    /// C64 disk title and two-character ID.
    pub disk_name: Option<String>,
    pub disk_id: Option<String>,
    /// C64 `.crt`: write the cold/warm start vectors and the CBM80 signature at 0x8000 (default true).
    pub cart_header: Option<bool>,
    /// Spectrum tape: put a BASIC loader in front of the code (default true).
    pub loader: Option<bool>,
    /// Spectrum tape: a 6912 byte `.scr` file shown while the code loads.
    pub screen: Option<String>,
    /// Spectrum snapshot: stack pointer (default 0, the top of memory), border, interrupt mode, interrupts.
    pub sp: Option<Value>,
    pub border: Option<u8>,
    pub im: Option<u8>,
    pub interrupts: Option<bool>,
    /// Spectrum tape: pause after each block in milliseconds (default 1000).
    pub pause: Option<u16>,
    /// Name of the program file on a disk (default: `name`; on a CPC disk `.BIN` is added if there is no extension).
    pub file_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub machine: Machine,
    /// CPU for `generic`: `6502` or `z80`.
    pub cpu: Option<String>,
    /// The file to write, relative to the layout file; the extension chooses the kind of image. `rcld -o` overrides.
    pub output: Option<String>,
    /// Name of the program (tape header, disk file, cartridge); default: the output's file name.
    pub name: Option<String>,
    /// Where the program starts: an address or a symbol.
    pub entry: Option<Value>,
    pub fill: Option<u8>,
    #[serde(default)]
    pub gaps: Gaps,
    #[serde(default)]
    pub input: Vec<Input>,
    #[serde(default)]
    pub region: Vec<Region>,
    #[serde(default)]
    pub reserve: Vec<Reserve>,
    #[serde(default)]
    pub file: Vec<ExtraFile>,
    /// `NAME = value` constants for the assembler.
    #[serde(default)]
    pub define: BTreeMap<String, i64>,
    #[serde(default)]
    pub options: Options,
}

pub fn parse(text: &str) -> Result<Layout, String> {
    toml::from_str(text).map_err(|e| e.to_string())
}

/// A number from a layout value; text is read as a number (`"$C000"`) here, symbols are the caller's job.
pub fn number(v: &Value) -> Result<i64, String> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Text(t) => rc_common::parse_number(t),
    }
}

pub fn address(v: &Value, what: &str) -> Result<u16, String> {
    let n = number(v).map_err(|e| format!("{what}: {e}"))?;
    u16::try_from(n).map_err(|_| format!("{what}: {n:#X} does not fit in 16 bits"))
}
