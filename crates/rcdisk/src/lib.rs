//! `rcdisk`: look inside disk images, change them, make new ones and convert between formats.
//!
//! A [`Disk`] is a disk image as its file system sees it. The adapters behind it ([`cbm`] for the
//! Commodore `.d64`/`.x64`/`.d71`/`.d81` and read-only `.g64`) keep the format logic in the format
//! crates; this layer only maps it to common operations. The trait is deliberately small and
//! provisional: more systems (CP/M `.dsk` next) will show what it is missing.

pub mod atari;
pub mod cbm;
pub mod cpm;

/// One file in a directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRow {
    /// Printable name.
    pub name: String,
    /// The name's bytes in hex, to tell apart names that print the same.
    pub name_hex: String,
    /// `PRG`, `SEQ`, ... (Commodore) or the user area (CP/M).
    pub kind: String,
    pub blocks: u32,
    /// Size in bytes when the file system records it.
    pub size: Option<u64>,
    /// `locked`, `unclosed`, `read-only`, ...
    pub flags: Vec<String>,
}

/// A file taken out of an image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extracted {
    /// The name as stored.
    pub name: String,
    /// A name that is safe on a host file system, with an extension for the file type.
    pub file_name: String,
    pub data: Vec<u8>,
}

/// The result of checking an image.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Check {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub files: usize,
    pub free_blocks: u64,
}

/// How a file is stored by [`Disk::add`].
#[derive(Clone, Debug, Default)]
pub struct AddOptions {
    /// Name on the disk; default: the host file name without extension.
    pub name: Option<String>,
    /// File type (`prg`, `seq`, `usr` for Commodore); default from the host file's extension.
    pub kind: Option<String>,
    /// Replace a file of the same name.
    pub replace: bool,
    /// CP/M user area (0-15).
    pub user: u8,
    /// CP/M: put an `amsdos` or `plus3dos` header in front of the data (`none` stores it as is).
    pub header: Option<String>,
    /// Load and entry address for a binary with a header.
    pub load: Option<u16>,
    pub entry: Option<u16>,
}

/// How a file is read out of an image.
#[derive(Clone, Debug, Default)]
pub struct ReadOptions {
    /// CP/M: keep the AMSDOS / +3DOS header instead of stripping it.
    pub keep_header: bool,
}

pub trait Disk {
    /// `d64`, `d71`, ... as given to `create --format` and `convert --to`.
    fn format(&self) -> String;
    fn description(&self) -> String;
    /// Why the image cannot be changed, if it cannot.
    fn read_only(&self) -> Option<String> {
        None
    }
    /// Facts about the disk as `(label, value)`.
    fn info(&self) -> Vec<(String, String)>;
    fn files(&self) -> Vec<FileRow>;
    fn free_blocks(&self) -> u64;
    /// The files matching `pattern` (`*` and `?` wildcards), in directory order.
    fn read(&self, pattern: &str, opts: &ReadOptions) -> Result<Vec<Extracted>, String>;
    /// Stores a host file; returns the name it was stored under.
    fn add(&mut self, host_name: &str, data: &[u8], opts: &AddOptions) -> Result<String, String>;
    /// Deletes the files matching `pattern`; returns their names.
    fn remove(&mut self, pattern: &str) -> Result<Vec<String>, String>;
    fn rename(&mut self, old: &str, new: &str) -> Result<(), String>;
    fn check(&self) -> Check;
    fn to_bytes(&self) -> Vec<u8>;
    /// The disk name and ID, for carrying over in a conversion.
    fn label(&self) -> (String, String);
    /// Every file that can be copied to another kind of image, with the reasons others cannot.
    fn export(&self) -> (Vec<Exported>, Vec<String>);
}

/// A file ready to be added to another image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exported {
    pub name: String,
    /// `prg` / `seq` / `usr` for Commodore sources, empty otherwise.
    pub kind: String,
    pub data: Vec<u8>,
}

/// The formats `create` and `convert` know, in the order `--help` lists them.
pub const FORMATS: &[(&str, &str)] = &[
    ("d64", "1541, 35 tracks (174848 bytes)"),
    ("d64-40", "1541, 40 tracks (196608 bytes)"),
    ("x64", "a .d64 with the 64-byte .x64 header"),
    ("d71", "1571, double sided (349696 bytes)"),
    ("d81", "1581 (819200 bytes)"),
    ("dsk", "CPC data format, 40 tracks, 178 KB free (AMSDOS)"),
    (
        "dsk-system",
        "CPC system format, 2 reserved tracks (AMSDOS)",
    ),
    ("dsk-ibm", "CPC IBM format, 8 sectors per track"),
    ("dsk-plus3", "Spectrum +3 format, 173 KB free (+3DOS)"),
];

/// Which format a file is, from its name and, failing that, its size and magic.
pub fn sniff(name: &str, bytes: &[u8]) -> Result<&'static str, String> {
    let ext = name
        .rsplit('.')
        .next()
        .filter(|e| e.len() < name.len())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("d64") => return Ok("d64"),
        Some("x64") => return Ok("x64"),
        Some("d71") => return Ok("d71"),
        Some("d81") => return Ok("d81"),
        Some("g64") => return Ok("g64"),
        Some("dsk") | Some("edsk") => return Ok("dsk"),
        Some("atr") => return Ok("atr"),
        _ => {}
    }
    if bytes.starts_with(b"MV - CPC") || bytes.starts_with(b"EXTENDED CPC DSK") {
        return Ok("dsk");
    }
    if bytes.starts_with(&[0x96, 0x02]) {
        return Ok("atr");
    }
    if bytes.starts_with(b"GCR-1541") {
        return Ok("g64");
    }
    if bytes.starts_with(&[0x43, 0x15, 0x41, 0x64]) {
        return Ok("x64");
    }
    match bytes.len() {
        174848 | 175531 | 196608 | 197376 => Ok("d64"),
        349696 | 351062 => Ok("d71"),
        819200 | 822400 => Ok("d81"),
        n => Err(format!(
            "cannot tell what kind of disk image {name} is ({n} bytes); name it .d64, .d71, .d81, .x64, .g64, .dsk or .atr"
        )),
    }
}

/// Opens an image.
pub fn open(name: &str, bytes: &[u8]) -> Result<Box<dyn Disk>, String> {
    match sniff(name, bytes)? {
        "atr" => Err(
            "ATR filesystem operations are unsupported; use info or add/extract --sector".into(),
        ),
        "dsk" => cpm::open(bytes),
        format => cbm::open(format, bytes),
    }
}

/// A new, blank, formatted image.
pub fn create(format: &str, name: Option<&str>, id: Option<&str>) -> Result<Box<dyn Disk>, String> {
    if format.starts_with("dsk") {
        cpm::create(format)
    } else {
        cbm::create(format, name, id)
    }
}

/// Copies the contents of `source` into a new image of `format`. Returns the image and what was
/// left behind.
pub fn convert(source: &dyn Disk, format: &str) -> Result<(Box<dyn Disk>, Vec<String>), String> {
    let cbm_to_cbm = !format.starts_with("dsk") && !source.format().starts_with("dsk");
    if cbm_to_cbm {
        return cbm::convert(source, format);
    }
    // Anything involving a CP/M disk goes file by file.
    let (name, id) = source.label();
    let mut target = create(
        format,
        Some(&name).filter(|n| !n.is_empty()).map(String::as_str),
        Some(&id)
            .filter(|i| i.chars().count() == 2)
            .map(String::as_str),
    )?;
    let (files, mut skipped) = source.export();
    let to_cpm = format.starts_with("dsk");
    let mut taken = std::collections::HashSet::new();
    for f in files {
        let stored = if to_cpm {
            cpm::sanitize_name(&f.name, &mut taken)
        } else {
            f.name.clone()
        };
        let (user, bare) = cpm::split_user(&stored);
        let opts = AddOptions {
            name: Some(bare.to_string()),
            kind: Some(f.kind.clone()).filter(|k| !k.is_empty() && !to_cpm),
            user,
            ..AddOptions::default()
        };
        if let Err(e) = target.add(&stored, &f.data, &opts) {
            skipped.push(format!("\"{}\": {e}", f.name));
        } else if stored != f.name {
            skipped.push(format!("\"{}\" was stored as \"{stored}\"", f.name));
        }
    }
    Ok((target, skipped))
}
