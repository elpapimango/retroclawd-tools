//! Tape images as lists of blocks: look inside, cut, merge, split, clean up and convert. ZX Spectrum `.tap`
//! and `.tzx` are handled block by block (a file written back is byte-for-byte what was read); Commodore `.tap`
//! and `.t64` can be inspected and, for `.t64`, extracted (see [`c64`]).

pub mod c64;
pub mod spectrum;
pub mod tzx;

pub use tzx::{Block, Tape};

use spectrum::{Header, checksum_ok, parse_header};

/// A tape file of any supported kind, by what it contains.
pub enum Image {
    /// `.tap` and `.tzx` both load into the same block list; `from_tap` remembers which one it was.
    Spectrum {
        tape: Tape,
        from_tap: bool,
    },
    C64Tap(c64_tape::Tape, Vec<u8>),
    T64(c64_tape::T64),
}

/// Opens a tape image, deciding the kind from the bytes (the `ZXTape!` signature, `C64-TAPE-RAW`, `C64`
/// in a `.t64` header) and falling back to the extension for a `.tap` of Spectrum blocks.
pub fn open(name: &str, bytes: &[u8]) -> Result<Image, String> {
    let lower = name.to_ascii_lowercase();
    if bytes.starts_with(tzx::MAGIC) {
        return Ok(Image::Spectrum {
            tape: Tape::parse(bytes).map_err(|e| e.to_string())?,
            from_tap: false,
        });
    }
    if bytes.starts_with(b"C64-TAPE-RAW") {
        let tape = c64_tape::parse_tap(bytes).map_err(|e| format!("C64 .tap: {e}"))?;
        return Ok(Image::C64Tap(tape, bytes.to_vec()));
    }
    if lower.ends_with(".t64") || bytes.starts_with(b"C64") {
        let t = c64_tape::parse_t64(bytes).map_err(|e| format!(".t64: {e}"))?;
        return Ok(Image::T64(t));
    }
    if lower.ends_with(".tap") {
        return Ok(Image::Spectrum {
            tape: from_tap(bytes)?,
            from_tap: true,
        });
    }
    Err(format!(
        "{name}: not a tape image I know (.tap, .tzx, Commodore .tap, .t64)"
    ))
}

/// The pause the blocks of a `.tap` get when it becomes a `.tzx`.
pub const TAP_PAUSE_MS: u16 = 1000;

/// A Spectrum `.tap` (length-prefixed raw blocks) as a block list of standard-speed data blocks.
pub fn from_tap(bytes: &[u8]) -> Result<Tape, String> {
    let mut tape = Tape::new();
    let mut at = 0;
    while at < bytes.len() {
        let Some(len) = bytes.get(at..at + 2) else {
            return Err(format!(
                "the .tap ends in the middle of block {}'s length",
                tape.blocks.len() + 1
            ));
        };
        let len = u16::from_le_bytes([len[0], len[1]]) as usize;
        let Some(data) = bytes.get(at + 2..at + 2 + len) else {
            return Err(format!(
                "block {} is {len} bytes long, but only {} are left in the .tap",
                tape.blocks.len() + 1,
                bytes.len() - at - 2
            ));
        };
        tape.blocks.push(Block::standard(data, TAP_PAUSE_MS));
        at += 2 + len;
    }
    Ok(tape)
}

/// The tape as a `.tap`. Standard-speed data blocks become `.tap` blocks; everything else cannot be stored in a
/// `.tap`. With `lossy`, blocks that carry no data (pauses, text, groups and the like) are dropped, said in the
/// notes; any other block is an error either way.
pub fn to_tap(tape: &Tape, lossy: bool) -> Result<(Vec<u8>, Vec<String>), String> {
    let mut out = Vec::new();
    let mut dropped = 0;
    for (i, b) in tape.blocks.iter().enumerate() {
        if b.id == 0x10 {
            let data = b.payload().unwrap_or(&[]);
            out.extend_from_slice(&(data.len() as u16).to_le_bytes());
            out.extend_from_slice(data);
        } else if is_neutral(b) {
            dropped += 1;
        } else {
            return Err(format!(
                "block {} ({}) cannot be stored in a .tap; only standard speed data can",
                i + 1,
                b.kind()
            ));
        }
    }
    let mut notes = Vec::new();
    if dropped > 0 {
        if !lossy {
            return Err(format!(
                "{dropped} block(s) (pauses, text, groups) have no place in a .tap; use --lossy to drop them"
            ));
        }
        notes.push(format!(
            "dropped {dropped} block(s) a .tap cannot hold (pauses, text, groups)"
        ));
    }
    Ok((out, notes))
}

/// Blocks that carry no signal data: removing them changes the file, not what plays (a pause is the exception
/// in time, which is why `to_tap` asks first).
fn is_neutral(b: &Block) -> bool {
    b.is_info() || matches!(b.id, 0x20..=0x22)
}

/// One row of a listing.
#[derive(Clone, Debug)]
pub struct Row {
    /// 1-based position in the tape.
    pub index: usize,
    pub id: u8,
    pub kind: &'static str,
    /// Payload size for data blocks.
    pub size: Option<usize>,
    pub pause_ms: Option<u16>,
    /// A readable line: the header's program/bytes description, or "Data", or the text of a text block.
    pub description: String,
    /// `Some(false)` when a ROM block's checksum is wrong.
    pub checksum_ok: Option<bool>,
}

pub fn rows(tape: &Tape) -> Vec<Row> {
    let mut rows = Vec::new();
    for (i, b) in tape.blocks.iter().enumerate() {
        let payload = b.payload();
        let mut checksum = None;
        let description = match (b.id, payload) {
            (0x10 | 0x11, Some(data)) => {
                checksum = Some(checksum_ok(data));
                match parse_header(data) {
                    Some(h) => h.describe(),
                    None if data.first() == Some(&0xFF) => {
                        format!("Data ({} bytes)", data.len().saturating_sub(2))
                    }
                    None => format!(
                        "Raw block, flag {:#04X}",
                        data.first().copied().unwrap_or(0)
                    ),
                }
            }
            (0x14, Some(data)) => format!("{} bytes", data.len()),
            (0x30 | 0x31, _) => b.text_of().unwrap_or_default(),
            (0x12, _) => format!("{} pulses of {} T", le16(&b.body, 2), le16(&b.body, 0)),
            (0x13, _) => format!("{} pulses", b.body.first().copied().unwrap_or(0)),
            (0x20, _) => match b.pause_ms() {
                Some(0) => "Stop the tape".to_string(),
                Some(ms) => format!("{ms} ms"),
                None => String::new(),
            },
            (0x21, _) => b
                .body
                .get(1..)
                .map(|t| t.iter().map(|&c| tzx::printable(c)).collect())
                .unwrap_or_default(),
            _ => String::new(),
        };
        rows.push(Row {
            index: i + 1,
            id: b.id,
            kind: b.kind(),
            size: payload.map(|p| p.len()),
            pause_ms: if b.id == 0x20 { None } else { b.pause_ms() },
            description,
            checksum_ok: checksum,
        });
    }
    rows
}

fn le16(b: &[u8], at: usize) -> usize {
    match (b.get(at), b.get(at + 1)) {
        (Some(&l), Some(&h)) => u16::from_le_bytes([l, h]) as usize,
        _ => 0,
    }
}

/// Block numbers as people write them: `3`, `2-5`, `4-` (to the end), `-3` (from the start), `all`, joined with
/// commas. 1-based, as `list` prints them. Returns zero-based indexes, sorted and without repeats.
pub fn parse_selection(text: &str, count: usize) -> Result<Vec<usize>, String> {
    let mut chosen = vec![false; count];
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (from, to) = if part.eq_ignore_ascii_case("all") {
            (1, count)
        } else if let Some((a, b)) = part.split_once('-') {
            let a = if a.trim().is_empty() {
                1
            } else {
                number(a, part)?
            };
            let b = if b.trim().is_empty() {
                count
            } else {
                number(b, part)?
            };
            (a, b)
        } else {
            let n = number(part, part)?;
            (n, n)
        };
        if from == 0 || to == 0 || from > to {
            return Err(format!(
                "\"{part}\" is not a block range (blocks are numbered from 1, the first number comes first)"
            ));
        }
        if to > count {
            return Err(format!("\"{part}\": the tape has only {count} block(s)"));
        }
        for c in &mut chosen[from - 1..to] {
            *c = true;
        }
    }
    if !chosen.iter().any(|&c| c) {
        return Err("no blocks selected".to_string());
    }
    Ok(chosen
        .iter()
        .enumerate()
        .filter(|(_, c)| **c)
        .map(|(i, _)| i)
        .collect())
}

fn number(text: &str, whole: &str) -> Result<usize, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("\"{whole}\" is not a block number or range"))
}

/// A new tape with only the chosen blocks.
pub fn keep(tape: &Tape, chosen: &[usize]) -> Tape {
    Tape {
        major: tape.major,
        minor: tape.minor,
        blocks: chosen.iter().map(|&i| tape.blocks[i].clone()).collect(),
    }
}

/// A new tape without the chosen blocks.
pub fn remove(tape: &Tape, chosen: &[usize]) -> Tape {
    Tape {
        major: tape.major,
        minor: tape.minor,
        blocks: tape
            .blocks
            .iter()
            .enumerate()
            .filter(|(i, _)| !chosen.contains(i))
            .map(|(_, b)| b.clone())
            .collect(),
    }
}

/// The tapes one after the other, in the newer format revision of the two.
pub fn merge(tapes: &[Tape]) -> Tape {
    let mut out = Tape::new();
    out.major = tapes.iter().map(|t| t.major).max().unwrap_or(1);
    out.minor = tapes
        .iter()
        .filter(|t| t.major == out.major)
        .map(|t| t.minor)
        .max()
        .unwrap_or(20);
    for t in tapes {
        out.blocks.extend(t.blocks.iter().cloned());
    }
    out
}

/// Cuts a tape into the pieces a loader treats as one program: a header block with the data block after it
/// (information blocks in front belong to the piece they precede). Returns a file-name stem and the piece.
pub fn split(tape: &Tape) -> Vec<(String, Tape)> {
    let mut parts: Vec<(String, Tape)> = Vec::new();
    let mut current: Vec<Block> = Vec::new();
    let mut name: Option<String> = None;
    let mut waiting_for_data = false;
    let mut unnamed = 0;
    let finish = |current: &mut Vec<Block>,
                  name: &mut Option<String>,
                  parts: &mut Vec<(String, Tape)>,
                  unnamed: &mut usize| {
        if current.is_empty() {
            return;
        }
        let stem = name.take().unwrap_or_else(|| {
            *unnamed += 1;
            format!("block{:02}", *unnamed)
        });
        parts.push((
            stem,
            Tape {
                major: tape.major,
                minor: tape.minor,
                blocks: std::mem::take(current),
            },
        ));
    };
    for b in &tape.blocks {
        let header: Option<Header> = if b.id == 0x10 {
            b.payload().and_then(parse_header)
        } else {
            None
        };
        if let Some(h) = header {
            if waiting_for_data || current.iter().any(Block::is_data) {
                finish(&mut current, &mut name, &mut parts, &mut unnamed);
            }
            name = Some(spectrum::sanitize(&h.name));
            if name.as_deref() == Some("") {
                name = None;
            }
            current.push(b.clone());
            waiting_for_data = true;
        } else if b.is_data() {
            if !waiting_for_data && current.iter().any(Block::is_data) {
                finish(&mut current, &mut name, &mut parts, &mut unnamed);
            }
            current.push(b.clone());
            if waiting_for_data {
                waiting_for_data = false;
                finish(&mut current, &mut name, &mut parts, &mut unnamed);
            }
        } else if current.is_empty() && matches!(b.id, 0x20 | 0x22) && !parts.is_empty() {
            // a pause or group end after a program belongs to that program
            parts.last_mut().expect("checked").1.blocks.push(b.clone());
        } else {
            current.push(b.clone());
        }
    }
    finish(&mut current, &mut name, &mut parts, &mut unnamed);
    // Equal names would overwrite each other as files: number the repeats.
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (stem, _) in &mut parts {
        let n = seen.entry(stem.to_ascii_lowercase()).or_insert(0);
        *n += 1;
        if *n > 1 {
            *stem = format!("{stem}-{n}");
        }
    }
    parts
}

#[derive(Clone, Debug, Default)]
pub struct CleanupOptions {
    /// Drop text, message, archive, hardware and custom-info blocks and glue.
    pub strip_info: bool,
    /// Recompute the checksum byte of ROM blocks that have a wrong one.
    pub fix_checksums: bool,
    /// Set the pause after every data block (and every pause block that is not a stop) to this.
    pub pause_ms: Option<u16>,
    /// Join pause blocks that follow each other into one.
    pub merge_pauses: bool,
    /// Drop data blocks with no data in them.
    pub drop_empty: bool,
}

/// Tidies a tape; returns the new tape and a line per change.
pub fn cleanup(tape: &Tape, opts: &CleanupOptions) -> (Tape, Vec<String>) {
    let mut notes = Vec::new();
    let mut blocks: Vec<Block> = Vec::new();
    for (i, b) in tape.blocks.iter().enumerate() {
        let n = i + 1;
        if opts.strip_info && b.is_info() {
            notes.push(format!("block {n}: removed {}", b.kind().to_lowercase()));
            continue;
        }
        if opts.drop_empty && b.is_data() && b.payload().is_none_or(<[u8]>::is_empty) {
            notes.push(format!(
                "block {n}: removed an empty {}",
                b.kind().to_lowercase()
            ));
            continue;
        }
        let mut b = b.clone();
        if opts.fix_checksums
            && b.id == 0x10
            && let Some(data) = b.payload().map(<[u8]>::to_vec)
            && !data.is_empty()
            && !checksum_ok(&data)
        {
            let mut fixed = data;
            let last = fixed.len() - 1;
            fixed[last] = spectrum::checksum_of(&fixed[..last]);
            b.set_payload(&fixed);
            notes.push(format!("block {n}: fixed the checksum"));
        }
        let mut joined = false;
        if opts.merge_pauses
            && b.id == 0x20
            && b.pause_ms() != Some(0)
            && let Some(prev) = blocks.last_mut()
            && prev.id == 0x20
            && prev.pause_ms() != Some(0)
        {
            let total = (prev.pause_ms().unwrap_or(0) as u32 + b.pause_ms().unwrap_or(0) as u32)
                .min(u16::MAX as u32);
            prev.set_pause_ms(total as u16);
            notes.push(format!("block {n}: joined with the pause before it"));
            joined = true;
        }
        if joined {
            // the joined pause is set below like any other
            b = blocks.pop().expect("just joined into it");
        }
        if let (Some(ms), Some(old)) = (opts.pause_ms, b.pause_ms())
            && old != ms
            && !(b.id == 0x20 && old == 0)
        {
            b.set_pause_ms(ms);
            notes.push(format!("block {n}: pause {old} ms -> {ms} ms"));
        }
        blocks.push(b);
    }
    (
        Tape {
            major: tape.major,
            minor: tape.minor,
            blocks,
        },
        notes,
    )
}

/// One file out of a tape.
pub struct Extracted {
    pub name: String,
    pub bytes: Vec<u8>,
    pub description: String,
}

/// The data of the chosen data blocks as files. A header block is not written itself: it names the data block
/// after it (`NAME.bas`, `NAME.bin`). With `raw`, every block's whole payload (flag byte and checksum too) is
/// written, under `NNN-kind.bin`, and headers are files too.
pub fn extract(tape: &Tape, chosen: &[usize], raw: bool) -> Vec<Extracted> {
    let mut out: Vec<Extracted> = Vec::new();
    let mut header: Option<Header> = None;
    for (i, b) in tape.blocks.iter().enumerate() {
        let Some(data) = b.payload() else { continue };
        let picked = chosen.contains(&i);
        let h = if b.id == 0x10 {
            parse_header(data)
        } else {
            None
        };
        if raw {
            if picked {
                out.push(Extracted {
                    name: format!(
                        "{:03}-{}.bin",
                        i + 1,
                        if h.is_some() { "header" } else { "data" }
                    ),
                    bytes: data.to_vec(),
                    description: b.kind().to_string(),
                });
            }
            continue;
        }
        if let Some(h) = h {
            header = Some(h);
            continue;
        }
        if !picked {
            header = None;
            continue;
        }
        let body = match data {
            [0xFF, rest @ .., _] => rest.to_vec(),
            _ => data.to_vec(),
        };
        let (name, description) = match header.take() {
            Some(h) => (h.file_name(), h.describe()),
            None => (
                format!("{:03}-data.bin", i + 1),
                format!("{} bytes", body.len()),
            ),
        };
        out.push(Extracted {
            name,
            bytes: body,
            description,
        });
    }
    // Two headers with one name would overwrite each other.
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for e in &mut out {
        let n = seen.entry(e.name.to_ascii_lowercase()).or_insert(0);
        *n += 1;
        if *n > 1 {
            let (stem, ext) = e.name.rsplit_once('.').unwrap_or((&e.name, ""));
            e.name = format!("{stem}-{n}.{ext}");
        }
    }
    out
}

/// Playing time of the whole tape in seconds, if every block's time is known; otherwise the time of the blocks
/// that are known and `false`.
pub fn duration_seconds(tape: &Tape) -> (f64, bool) {
    let mut total = 0u64;
    let mut exact = true;
    for b in &tape.blocks {
        match b.cycles() {
            Some(c) => total += c,
            None if b.is_info() || matches!(b.id, 0x21 | 0x22 | 0x2A | 0x2B) => {}
            None => exact = false,
        }
    }
    (total as f64 / 3_500_000.0, exact)
}

/// A hexdump in the usual three columns.
pub fn hexdump(data: &[u8]) -> String {
    let mut out = String::new();
    for (row, chunk) in data.chunks(16).enumerate() {
        out.push_str(&format!("{:04X}  ", row * 16));
        for i in 0..16 {
            match chunk.get(i) {
                Some(b) => out.push_str(&format!("{b:02X} ")),
                None => out.push_str("   "),
            }
        }
        out.push(' ');
        out.extend(chunk.iter().map(|&b| tzx::printable(b)));
        out.push('\n');
    }
    out
}
