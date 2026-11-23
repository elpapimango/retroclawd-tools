//! GD-ROM images (`.gdi`): a count, then one line a track: number, first LBA, type (4 data, 0 audio), sector size, file, byte offset.
//!
//! A CHD keeps the whole LBA space: each track takes the frames from its first LBA to the next track's, with the data first and
//! the rest zero (`PAD` frames); the last track takes just its data.

use crate::cdsrc::{CdSource, SrcTrack};
use crate::types::TrackType;
use crate::{Error, Result, fail};
use std::path::Path;

/// One line of the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GdiLine {
    pub number: u32,
    pub lba: u32,
    pub data: bool,
    pub sector_size: u32,
    pub file: String,
    pub offset: u64,
}

fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut cur, mut quoted, mut any) = (String::new(), false, false);
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn parse(text: &str) -> Result<Vec<GdiLine>> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let count: usize = lines
        .next()
        .and_then(|l| l.parse().ok())
        .ok_or_else(|| Error("a .gdi file starts with the number of tracks".into()))?;
    let mut out = Vec::new();
    for l in lines {
        let w = words(l);
        let num = |i: usize, what: &str| -> Result<u64> {
            w.get(i)
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| Error(format!("bad {what} in the .gdi line {l:?}")))
        };
        if w.len() < 6 {
            return fail(format!("the .gdi line {l:?} needs six fields"));
        }
        out.push(GdiLine {
            number: num(0, "track number")? as u32,
            lba: num(1, "LBA")? as u32,
            data: match num(2, "type")? {
                4 => true,
                0 => false,
                other => {
                    return fail(format!(
                        "the .gdi track type {other} is neither 4 (data) nor 0 (audio)"
                    ));
                }
            },
            sector_size: num(3, "sector size")? as u32,
            file: w[4].clone(),
            offset: num(5, "offset")?,
        });
    }
    if out.len() != count {
        return fail(format!(
            "the .gdi file says {count} tracks and lists {}",
            out.len()
        ));
    }
    if out.is_empty() {
        return fail("the .gdi file has no tracks");
    }
    Ok(out)
}

pub fn open(path: &Path) -> Result<CdSource> {
    let text = std::fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    let lines = parse(&String::from_utf8_lossy(&text))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut tracks = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let kind = match (l.data, l.sector_size) {
            (false, 2352) => TrackType::Audio,
            (true, 2352) => TrackType::Mode1Raw,
            (true, 2048) => TrackType::Mode1,
            (_, size) => {
                return fail(format!(
                    "track {}: a sector size of {size} is not handled",
                    l.number
                ));
            }
        };
        let file = dir.join(&l.file);
        let len = std::fs::metadata(&file)
            .map_err(|e| Error(format!("{}: {e}", file.display())))?
            .len();
        let size = u64::from(l.sector_size);
        if l.offset > len || !(len - l.offset).is_multiple_of(size) {
            return fail(format!(
                "{}: not a whole number of {size}-byte sectors after offset {}",
                file.display(),
                l.offset
            ));
        }
        let data_frames = ((len - l.offset) / size) as u32;
        let pad = match lines.get(i + 1) {
            Some(next) => {
                let span = next.lba.checked_sub(l.lba).filter(|&s| s >= data_frames).ok_or_else(|| {
                    Error(format!(
                        "track {} has {data_frames} sectors but the next track starts {} after it",
                        l.number,
                        next.lba as i64 - l.lba as i64
                    ))
                })?;
                span - data_frames
            }
            None => 0,
        };
        tracks.push(SrcTrack {
            number: l.number,
            kind,
            file,
            offset: l.offset,
            stored_pregap: 0,
            data_frames,
            virtual_pregap: 0,
            postgap: 0,
            swap_audio: kind.is_audio(),
            pad: Some(pad),
        });
    }
    Ok(CdSource { tracks })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_read_with_quotes_and_the_count_is_checked() {
        let l = parse("2\n1 0 4 2352 \"track 01.bin\" 0\n2 600 0 2352 t2.raw 0\n").unwrap();
        assert_eq!(l[0].file, "track 01.bin");
        assert_eq!((l[1].lba, l[1].data, l[1].sector_size), (600, false, 2352));
        assert!(parse("3\n1 0 4 2352 a 0\n").is_err());
        assert!(parse("x\n").is_err());
        assert!(parse("1\n1 0 7 2352 a 0\n").is_err());
        assert!(parse("1\n1 0 4 2352 a\n").is_err());
    }
}
