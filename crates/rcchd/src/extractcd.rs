//! `extractcd`: the sectors of a CD CHD back as a cue sheet and bin file(s), in the form chdman writes them.
//!
//! Each track's data is the first bytes of its frames (2048 for `MODE1`, 2352 for raw and audio, ...), audio turned back to
//! little-endian. A stored pregap is part of the track's data and is announced by `INDEX 00`; a pregap that is not stored is a
//! `PREGAP` command. Every frame counts one 75th of a second in the sheet's times, whatever the size of its sector.

use crate::cue::format_time;
use crate::types::{FRAME_SIZE, SubType, TrackMeta};
use crate::{Error, Result, fail};
use chd::Image;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub struct ExtractCd {
    pub input: PathBuf,
    pub parent: Option<PathBuf>,
    /// The cue sheet to write.
    pub output: PathBuf,
    /// The bin file (default: the sheet's name with `.bin`).
    pub output_bin: Option<PathBuf>,
    /// One bin file a track, named `<name> (Track N).bin`.
    pub split: bool,
    pub force: bool,
}

/// The tracks of a CD CHD, in order, with the frame of the file where each starts.
pub fn tracks_of(image: &Image) -> Result<Vec<(TrackMeta, u32)>> {
    let mut out = Vec::new();
    let mut at = 0u32;
    for e in image.metadata() {
        match &e.tag {
            b"CHT2" | b"CHTR" | b"CHGD" | b"CHGT" => {
                let t = TrackMeta::parse(&e.data)?;
                let first = at;
                at += t.frames.div_ceil(4) * 4;
                out.push((t, first));
            }
            b"CHCD" => return fail("a CHD with the old binary track table (CHCD) is not read"),
            _ => {}
        }
    }
    if out.is_empty() {
        return fail("the file describes no CD tracks (it is not a CD image)");
    }
    Ok(out)
}

fn open_out(path: &Path, force: bool) -> Result<BufWriter<std::fs::File>> {
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

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn extract(opts: &ExtractCd, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    let mut image = Image::open(&opts.input, opts.parent.as_deref())?;
    if image.header().unit_bytes as usize != FRAME_SIZE {
        return fail("this CHD is not a CD image (its units are not CD frames)");
    }
    let tracks = tracks_of(&image)?;
    if let Some((t, _)) = tracks.iter().find(|(t, _)| t.sub != SubType::None) {
        return fail(format!(
            "track {} carries subchannel data, which cannot be extracted yet",
            t.number
        ));
    }
    if tracks.iter().any(|(t, _)| t.pad.is_some()) {
        return extract_gdi(&mut image, &tracks, opts, &mut progress);
    }
    let stem = opts.output.with_extension("");
    let stem_name = file_name(&stem);
    let dir = opts.output.parent().unwrap_or(Path::new(""));
    // the bin files to write, with the tracks each one holds
    let bin_path = |n: Option<u32>| -> PathBuf {
        match (n, &opts.output_bin) {
            (Some(n), _) => dir.join(format!("{stem_name} (Track {n}).bin")),
            (None, Some(p)) => p.clone(),
            (None, None) => dir.join(format!("{stem_name}.bin")),
        }
    };
    let outputs: Vec<PathBuf> = if opts.split {
        tracks
            .iter()
            .map(|(t, _)| bin_path(Some(t.number)))
            .collect()
    } else {
        vec![bin_path(None)]
    };
    if !opts.force {
        for p in outputs.iter().chain(std::iter::once(&opts.output)) {
            if p.exists() {
                return fail(format!(
                    "{}: exists (use --force to replace it)",
                    p.display()
                ));
            }
        }
    }

    let total: u64 = tracks.iter().map(|(t, _)| u64::from(t.frames)).sum();
    let mut done = 0u64;
    let mut cue = String::new();
    let mut current: Option<BufWriter<std::fs::File>> = None;
    // frames written to the current bin: the times of the sheet
    let mut position = 0u32;
    for (n, (t, first)) in tracks.iter().enumerate() {
        if opts.split || n == 0 {
            if let Some(mut w) = current.take() {
                w.flush()?;
            }
            let path = &outputs[if opts.split { n } else { 0 }];
            current = Some(open_out(path, opts.force)?);
            cue.push_str(&format!("FILE \"{}\" BINARY\n", file_name(path)));
            position = 0;
        }
        cue.push_str(&format!("  TRACK {:02} {}\n", t.number, t.kind.cue_mode()));
        let stored = t.pregap_stored();
        if stored {
            cue.push_str(&format!("    INDEX 00 {}\n", format_time(position)));
            cue.push_str(&format!(
                "    INDEX 01 {}\n",
                format_time(position + t.pregap)
            ));
        } else {
            if t.pregap > 0 {
                cue.push_str(&format!("    PREGAP {}\n", format_time(t.pregap)));
            }
            cue.push_str(&format!("    INDEX 01 {}\n", format_time(position)));
        }
        if t.postgap > 0 {
            cue.push_str(&format!("    POSTGAP {}\n", format_time(t.postgap)));
        }
        let size = t.kind.data_size();
        let w = current.as_mut().expect("opened above");
        let mut buf = Vec::with_capacity(size * 64);
        for f in 0..t.frames {
            let frame = image.read_bytes(u64::from(first + f) * FRAME_SIZE as u64, FRAME_SIZE)?;
            let start = buf.len();
            buf.extend_from_slice(&frame[..size]);
            if t.kind.is_audio() {
                for pair in buf[start..].as_chunks_mut::<2>().0 {
                    pair.swap(0, 1);
                }
            }
            if buf.len() >= size * 64 {
                w.write_all(&buf)?;
                buf.clear();
            }
            done += 1;
            if done.is_multiple_of(256) {
                progress(done, total);
            }
        }
        w.write_all(&buf)?;
        position += t.frames;
    }
    if let Some(mut w) = current {
        w.flush()?;
    }
    progress(total, total);
    // chdman writes the sheet in text mode: CRLF on Windows
    let cue = if cfg!(windows) {
        cue.replace('\n', "\r\n")
    } else {
        cue
    };
    let mut out = open_out(&opts.output, opts.force)?;
    out.write_all(cue.as_bytes())?;
    out.flush()?;
    Ok(())
}

/// A GD-ROM CHD: a `.gdi` file and one file a track (`<name>01.bin`, `<name>02.raw`, ...), each with just its data, not the padding.
fn extract_gdi(
    image: &mut Image,
    tracks: &[(TrackMeta, u32)],
    opts: &ExtractCd,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let stem = opts.output.with_extension("");
    let stem_name = file_name(&stem);
    let dir = opts.output.parent().unwrap_or(Path::new(""));
    let names: Vec<PathBuf> = tracks
        .iter()
        .map(|(t, _)| {
            let ext = if t.kind.is_audio() { "raw" } else { "bin" };
            dir.join(format!("{stem_name}{:02}.{ext}", t.number))
        })
        .collect();
    if !opts.force {
        for p in names.iter().chain(std::iter::once(&opts.output)) {
            if p.exists() {
                return fail(format!(
                    "{}: exists (use --force to replace it)",
                    p.display()
                ));
            }
        }
    }
    let total: u64 = tracks
        .iter()
        .map(|(t, _)| u64::from(t.frames - t.pad.unwrap_or(0)))
        .sum();
    let mut done = 0u64;
    let mut gdi = format!("{}\n", tracks.len());
    let mut lba = 0u32;
    for ((t, first), path) in tracks.iter().zip(&names) {
        let size = t.kind.data_size();
        let name = file_name(path);
        let shown = if name.contains(' ') {
            format!("\"{name}\"")
        } else {
            name
        };
        gdi.push_str(&format!(
            "{} {} {} {} {} 0\n",
            t.number,
            lba,
            if t.kind.is_audio() { 0 } else { 4 },
            size,
            shown
        ));
        lba += t.frames;
        let mut w = open_out(path, opts.force)?;
        let mut buf = Vec::with_capacity(size * 64);
        for f in 0..t.frames - t.pad.unwrap_or(0) {
            let frame = image.read_bytes(u64::from(first + f) * FRAME_SIZE as u64, FRAME_SIZE)?;
            let start = buf.len();
            buf.extend_from_slice(&frame[..size]);
            if t.kind.is_audio() {
                for pair in buf[start..].as_chunks_mut::<2>().0 {
                    pair.swap(0, 1);
                }
            }
            if buf.len() >= size * 64 {
                w.write_all(&buf)?;
                buf.clear();
            }
            done += 1;
            if done.is_multiple_of(256) {
                progress(done, total);
            }
        }
        w.write_all(&buf)?;
        w.flush()?;
    }
    progress(total, total);
    let gdi = if cfg!(windows) {
        gdi.replace('\n', "\r\n")
    } else {
        gdi
    };
    let mut out = open_out(&opts.output, opts.force)?;
    out.write_all(gdi.as_bytes())?;
    out.flush()?;
    Ok(())
}
