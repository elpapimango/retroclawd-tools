//! `createraw`, `createhd`, `createdvd` and their `extract` counterparts: CHDs of plain data (a hard disk image, a DVD image, any file cut into
//! units), and `copy` and the metadata commands that work on any CHD.

use crate::hdgeom::{self, Chs};
use crate::{Error, Result, fail};
use chd::{Codec, CreateOptions, Image, MetaEntry, Summary, Writer};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// What the data is, which decides the unit size, the metadata and the default hunk size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
    Raw,
    Hd,
    Dvd,
}

pub const DVD_SECTOR: u32 = 2048;
/// Codecs chdman uses for data unless told otherwise.
pub const DEFAULT_CODECS: [Codec; 4] = [Codec::Lzma, Codec::Zlib, Codec::Huff, Codec::Flac];

/// The default hunk size: as many whole units as fit in 4 KiB, or one unit when it is larger.
pub fn default_hunk(unit: u32) -> u32 {
    if unit == 0 || unit > 4096 {
        unit
    } else {
        4096 / unit * unit
    }
}

pub struct CreateData {
    pub media: Media,
    pub input: Option<PathBuf>,
    pub output: PathBuf,
    pub parent: Option<PathBuf>,
    pub codecs: Option<Vec<Codec>>,
    pub hunk_bytes: Option<u32>,
    /// `createraw --unitsize`, `createhd --sectorsize`.
    pub unit_bytes: Option<u32>,
    /// The part of the input to use: where it starts and how long it is, each in bytes or in hunks.
    pub start_byte: Option<u64>,
    pub start_hunk: Option<u64>,
    pub bytes: Option<u64>,
    pub hunks: Option<u64>,
    /// `createhd` without an input: the size, the geometry or a template.
    pub size: Option<u64>,
    pub chs: Option<Chs>,
    pub template: Option<usize>,
    pub force: bool,
    pub threads: usize,
}

impl CreateData {
    pub fn new(media: Media, output: PathBuf) -> Self {
        CreateData {
            media,
            input: None,
            output,
            parent: None,
            codecs: None,
            hunk_bytes: None,
            unit_bytes: None,
            start_byte: None,
            start_hunk: None,
            bytes: None,
            hunks: None,
            size: None,
            chs: None,
            template: None,
            force: false,
            threads: 0,
        }
    }
}

pub fn create(o: &CreateData, mut progress: impl FnMut(u64, u64)) -> Result<Summary> {
    let parent = match &o.parent {
        Some(p) => Some(Image::open(p, None)?),
        None => None,
    };
    // geometry and unit size
    let template = match o.template {
        Some(i) => Some(
            hdgeom::TEMPLATES
                .get(i)
                .ok_or_else(|| Error(format!("there is no template {i} (see listtemplates)")))?,
        ),
        None => None,
    };
    let unit = match o.media {
        Media::Dvd => DVD_SECTOR,
        Media::Hd => o
            .unit_bytes
            .or(template.map(|t| t.sector_size))
            .unwrap_or(512),
        Media::Raw => match (o.unit_bytes, &parent) {
            (Some(u), _) => u,
            (None, Some(p)) => p.header().unit_bytes,
            (None, None) => return fail("createraw needs --unitsize (unless a parent gives it)"),
        },
    };
    if unit == 0 {
        return fail("the unit size is zero");
    }
    if let Some(p) = &parent
        && p.header().unit_bytes != unit
    {
        return fail(format!(
            "the parent's unit size is {} and this file's is {unit}",
            p.header().unit_bytes
        ));
    }
    let hunk_bytes = o
        .hunk_bytes
        .or(parent.as_ref().map(|p| p.header().hunk_bytes))
        .unwrap_or(match o.media {
            Media::Dvd => 2 * DVD_SECTOR,
            _ => default_hunk(unit),
        });
    if hunk_bytes == 0 || !hunk_bytes.is_multiple_of(unit) {
        return fail(format!(
            "the hunk size {hunk_bytes} is not a whole number of {unit}-byte units"
        ));
    }
    let start = match (o.start_byte, o.start_hunk) {
        (Some(b), _) => b,
        (None, Some(h)) => h * u64::from(hunk_bytes),
        (None, None) => 0,
    };
    let wanted = match (o.bytes, o.hunks) {
        (Some(b), _) => Some(b),
        (None, Some(h)) => Some(h * u64::from(hunk_bytes)),
        (None, None) => None,
    };

    // the data: a file, or zeros for a blank disk
    let (mut source, data_len): (Option<File>, u64) = match &o.input {
        Some(path) => {
            let mut f = File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
            let len = f.metadata()?.len();
            if start > len {
                return fail("the start is past the end of the input");
            }
            let avail = len - start;
            let n = wanted.unwrap_or(avail);
            if n > avail {
                return fail("the input is shorter than the part asked for");
            }
            f.seek(SeekFrom::Start(start))?;
            (Some(f), n)
        }
        None => {
            if o.media != Media::Hd {
                return fail("an input file is needed");
            }
            let size = match (o.size, o.chs, template) {
                (Some(s), _, _) => s,
                (None, Some(c), _) => c.total_sectors() * u64::from(unit),
                (None, None, Some(t)) => t.chs.total_sectors() * u64::from(t.sector_size),
                _ => return fail("createhd needs an input, --size, --chs or --template"),
            };
            (None, size)
        }
    };
    if data_len == 0 || !data_len.is_multiple_of(u64::from(unit)) {
        return fail(format!(
            "Data size {data_len} is not divisible by unit size {unit}"
        ));
    }

    let mut metadata = Vec::new();
    match o.media {
        Media::Hd => {
            let total = data_len / u64::from(unit);
            let chs = match (o.chs, template) {
                (Some(c), _) => c,
                (None, Some(t))
                    if o.input.is_none() || data_len == t.chs.total_sectors() * u64::from(unit) =>
                {
                    t.chs
                }
                _ => hdgeom::guess(total),
            };
            if o.chs.is_some() && chs.total_sectors() != total {
                return fail(format!(
                    "the geometry {},{},{} holds {} sectors but the data is {total}",
                    chs.cylinders,
                    chs.heads,
                    chs.sectors,
                    chs.total_sectors()
                ));
            }
            metadata.push(MetaEntry::text(*b"GDDD", &chs.text(unit)));
        }
        Media::Dvd => metadata.push(MetaEntry::new(*b"DVD ", vec![0])),
        Media::Raw => {}
    }
    let codecs = match &o.codecs {
        Some(c) => c.clone(),
        None if o.input.is_none() => Vec::new(),
        None => DEFAULT_CODECS.to_vec(),
    };
    let opts = CreateOptions {
        hunk_bytes,
        unit_bytes: unit,
        codecs,
        logical_bytes: data_len,
        metadata,
        overwrite: o.force,
        threads: o.threads,
    };
    let mut w = Writer::create(&o.output, opts, parent)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut left = data_len;
    while left > 0 {
        let n = left.min(buf.len() as u64) as usize;
        match source.as_mut() {
            Some(f) => f.read_exact(&mut buf[..n])?,
            None => buf[..n].fill(0),
        }
        w.push(&buf[..n])?;
        left -= n as u64;
        progress(data_len - left, data_len);
    }
    Ok(w.finish()?)
}

pub struct ExtractData {
    pub media: Media,
    pub input: PathBuf,
    pub parent: Option<PathBuf>,
    pub output: PathBuf,
    pub start_byte: Option<u64>,
    pub start_hunk: Option<u64>,
    pub bytes: Option<u64>,
    pub hunks: Option<u64>,
    pub force: bool,
}

fn create_out(path: &Path, force: bool) -> Result<BufWriter<File>> {
    let mut o = OpenOptions::new();
    o.write(true);
    if force {
        o.create(true).truncate(true);
    } else {
        o.create_new(true);
    }
    let f = o.open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            Error(format!(
                "{}: exists (use --force to replace it)",
                path.display()
            ))
        } else {
            Error(format!("{}: {e}", path.display()))
        }
    })?;
    Ok(BufWriter::with_capacity(1 << 20, f))
}

pub fn extract(o: &ExtractData, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    let mut image = Image::open(&o.input, o.parent.as_deref())?;
    let has = |tag: &[u8; 4]| image.metadata().iter().any(|e| &e.tag == tag);
    match o.media {
        Media::Hd if !has(b"GDDD") => {
            return fail(
                "this CHD has no hard disk description (GDDD); use extractraw for other data",
            );
        }
        Media::Dvd if !has(b"DVD ") => {
            return fail(
                "this CHD is not a DVD image (no DVD metadata); use extractraw for other data",
            );
        }
        _ => {}
    }
    let h = image.header().clone();
    let hb = u64::from(h.hunk_bytes);
    let start = match (o.start_byte, o.start_hunk) {
        (Some(b), _) => b,
        (None, Some(n)) => n * hb,
        (None, None) => 0,
    };
    if start > h.logical_bytes {
        return fail("the start is past the end of the data");
    }
    let avail = h.logical_bytes - start;
    let want = match (o.bytes, o.hunks) {
        (Some(b), _) => b,
        (None, Some(n)) => n * hb,
        (None, None) => avail,
    }
    .min(avail);
    let mut out = create_out(&o.output, o.force)?;
    let mut at = start;
    let end = start + want;
    while at < end {
        let n = (end - at).min(1 << 20) as usize;
        let bytes = image.read_bytes(at, n)?;
        out.write_all(&bytes)?;
        at += n as u64;
        progress(at - start, want);
    }
    out.flush()?;
    Ok(())
}

/// `copy`: the same data in a new file, with other codecs, hunk size or parent; the metadata comes along.
pub struct Copy {
    pub input: PathBuf,
    pub input_parent: Option<PathBuf>,
    pub output: PathBuf,
    pub output_parent: Option<PathBuf>,
    pub codecs: Option<Vec<Codec>>,
    pub hunk_bytes: Option<u32>,
    pub force: bool,
    pub threads: usize,
}

pub fn copy(o: &Copy, mut progress: impl FnMut(u64, u64)) -> Result<Summary> {
    let mut image = Image::open(&o.input, o.input_parent.as_deref())?;
    let h = image.header().clone();
    let parent = match &o.output_parent {
        Some(p) => Some(Image::open(p, None)?),
        None => None,
    };
    let codecs = match &o.codecs {
        Some(c) => c.clone(),
        None => h
            .compressors
            .iter()
            .filter(|t| **t != [0; 4])
            .map(|t| {
                Codec::from_tag(*t).ok_or_else(|| {
                    Error(format!(
                        "the codec {:?} cannot be written",
                        String::from_utf8_lossy(t)
                    ))
                })
            })
            .collect::<Result<Vec<_>>>()?,
    };
    let metadata: Vec<MetaEntry> = image
        .metadata()
        .iter()
        .map(|e| MetaEntry {
            tag: e.tag,
            flags: e.flags,
            data: e.data.clone(),
            offset: 0,
            next: 0,
        })
        .collect();
    let opts = CreateOptions {
        hunk_bytes: o.hunk_bytes.unwrap_or(h.hunk_bytes),
        unit_bytes: h.unit_bytes,
        codecs,
        logical_bytes: h.logical_bytes,
        metadata,
        overwrite: o.force,
        threads: o.threads,
    };
    let mut w = Writer::create(&o.output, opts, parent)?;
    let mut at = 0u64;
    while at < h.logical_bytes {
        let n = (h.logical_bytes - at).min(1 << 20) as usize;
        w.push(&image.read_bytes(at, n)?)?;
        at += n as u64;
        progress(at, h.logical_bytes);
    }
    Ok(w.finish()?)
}

/// A metadata tag as chdman takes it: four characters, or eight hex digits.
pub fn parse_tag(text: &str) -> Result<[u8; 4]> {
    if text.len() == 4 && text.is_ascii() {
        return Ok(text.as_bytes().try_into().expect("four bytes"));
    }
    if text.len() == 8
        && let Ok(v) = u32::from_str_radix(text, 16)
    {
        return Ok(v.to_be_bytes());
    }
    fail(format!(
        "{text:?} is not a metadata tag (four characters or eight hex digits)"
    ))
}

/// The bytes of an entry: the data of the `index`-th entry with `tag`.
pub fn dump_metadata(path: &Path, tag: [u8; 4], index: u32) -> Result<Vec<u8>> {
    let (_, entries) = chd::read_header_and_metadata(path)?;
    entries
        .into_iter()
        .filter(|e| e.tag == tag)
        .nth(index as usize)
        .map(|e| e.data)
        .ok_or_else(|| Error("there is no such metadata entry".to_string()))
}
