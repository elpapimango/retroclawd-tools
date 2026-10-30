//! `createcd`: a CHD from a CD image (cue sheet and files, or an iso).
//!
//! Every sector becomes one frame of 2448 bytes: the track's data from the start of the frame (the rest of the sector zero), 96 bytes
//! of subchannel data (zero), audio turned big-endian. Each track is padded with zero frames to a multiple of four, which is how the
//! `CHT2` metadata (one entry a track) places the tracks in the file.

use crate::cdsrc::{self, CdSource, SrcTrack};
use crate::types::FRAME_SIZE;
use crate::{Error, Result};
use chd::{Codec, CreateOptions, MetaEntry, Summary, Writer};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Codecs chdman uses for a CD unless told otherwise.
pub const DEFAULT_CODECS: [Codec; 3] = [Codec::CdLzma, Codec::CdZlib, Codec::CdFlac];
/// Eight frames to a hunk.
pub const DEFAULT_HUNK_BYTES: u32 = 8 * FRAME_SIZE as u32;

pub struct CreateCd {
    pub input: PathBuf,
    pub output: PathBuf,
    pub codecs: Vec<Codec>,
    pub hunk_bytes: u32,
    pub parent: Option<PathBuf>,
    pub force: bool,
    pub threads: usize,
}

impl CreateCd {
    pub fn new(input: PathBuf, output: PathBuf) -> Self {
        CreateCd {
            input,
            output,
            codecs: DEFAULT_CODECS.to_vec(),
            hunk_bytes: DEFAULT_HUNK_BYTES,
            parent: None,
            force: false,
            threads: 0,
        }
    }
}

/// Turns the bytes of one sector of a track into a frame.
fn frame_of(track: &SrcTrack, data: &[u8], out: &mut [u8]) {
    out[..data.len()].copy_from_slice(data);
    if track.swap_audio {
        for pair in out[..data.len()].as_chunks_mut::<2>().0 {
            pair.swap(0, 1);
        }
    }
}

/// Reads a track's stored frames from its file and pushes them, then the padding.
fn push_track(
    w: &mut Writer,
    track: &SrcTrack,
    done: &mut u64,
    total: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let size = track.kind.data_size();
    let mut file =
        File::open(&track.file).map_err(|e| Error(format!("{}: {e}", track.file.display())))?;
    file.seek(SeekFrom::Start(track.offset))?;
    let mut sector = vec![0u8; size];
    let mut batch = Vec::with_capacity(FRAME_SIZE * 64);
    for _ in 0..track.file_frames() {
        file.read_exact(&mut sector)
            .map_err(|e| Error(format!("{}: {e}", track.file.display())))?;
        let start = batch.len();
        batch.resize(start + FRAME_SIZE, 0);
        frame_of(track, &sector, &mut batch[start..]);
        if batch.len() >= FRAME_SIZE * 64 {
            w.push(&batch)?;
            *done += 64;
            batch.clear();
            progress(*done, total);
        }
    }
    let pad = track.frames().div_ceil(4) * 4 - track.file_frames();
    batch.resize(batch.len() + pad as usize * FRAME_SIZE, 0);
    *done += (batch.len() / FRAME_SIZE) as u64;
    w.push(&batch)?;
    progress(*done, total);
    Ok(())
}

pub fn create(opts: &CreateCd, mut progress: impl FnMut(u64, u64)) -> Result<Summary> {
    if opts.codecs.iter().any(|c| !c.is_cd()) {
        return crate::fail("createcd takes CD codecs only (cdzl, cdzs, cdlz, cdfl) or none");
    }
    let source: CdSource = cdsrc::open(&opts.input)?;
    let total = source.total_frames();
    let metadata: Vec<MetaEntry> = source
        .tracks
        .iter()
        .map(|t| {
            let m = t.meta();
            MetaEntry::text(m.tag(), &m.text())
        })
        .collect();
    let parent = match &opts.parent {
        Some(p) => Some(chd::Image::open(Path::new(p), None)?),
        None => None,
    };
    let create = CreateOptions {
        hunk_bytes: opts.hunk_bytes,
        unit_bytes: FRAME_SIZE as u32,
        codecs: opts.codecs.clone(),
        logical_bytes: total * FRAME_SIZE as u64,
        metadata,
        overwrite: opts.force,
        threads: opts.threads,
    };
    let mut w = Writer::create(&opts.output, create, parent)?;
    let mut done = 0u64;
    for t in &source.tracks {
        push_track(&mut w, t, &mut done, total, &mut progress)?;
    }
    Ok(w.finish()?)
}
