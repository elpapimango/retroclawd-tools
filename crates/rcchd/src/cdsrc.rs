//! CD images to turn into a CHD, from a cue sheet or an iso file: which bytes of which file make each track's frames.

use crate::cue::{self, CueSheet};
use crate::types::{SubType, TrackMeta, TrackType};
use crate::{Error, Result, fail};
use std::path::{Path, PathBuf};

/// One track of the source, ready to be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SrcTrack {
    pub number: u32,
    pub kind: TrackType,
    pub file: PathBuf,
    /// The byte of `file` where the track's first stored frame is.
    pub offset: u64,
    /// Frames of the pregap that are in the file, before the track's data (cue `INDEX 00`).
    pub stored_pregap: u32,
    pub data_frames: u32,
    /// A pregap that is not in the file (cue `PREGAP`).
    pub virtual_pregap: u32,
    pub postgap: u32,
    /// Audio in the file is little-endian and the CHD keeps it big-endian.
    pub swap_audio: bool,
    /// GD-ROM: frames of zeros after the data that fill the track out to the next track's start (`PAD`).
    pub pad: Option<u32>,
}

impl SrcTrack {
    /// Frames this track takes in the CHD (before the padding to a multiple of four).
    pub fn frames(&self) -> u32 {
        self.stored_pregap + self.data_frames + self.pad.unwrap_or(0)
    }

    /// Frames read from the track's file (the padding is not in it).
    pub fn file_frames(&self) -> u32 {
        self.stored_pregap + self.data_frames
    }

    pub fn meta(&self) -> TrackMeta {
        let stored = self.stored_pregap > 0;
        TrackMeta {
            number: self.number,
            kind: self.kind,
            sub: SubType::None,
            frames: self.frames(),
            pregap: if stored {
                self.stored_pregap
            } else {
                self.virtual_pregap
            },
            // chdman writes the type of a missing pregap as MODE1 whatever the track is
            pregap_type: if stored {
                format!("V{}", self.kind.name())
            } else {
                "MODE1".to_string()
            },
            pregap_sub: "NONE".to_string(),
            postgap: self.postgap,
            pad: self.pad,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CdSource {
    pub tracks: Vec<SrcTrack>,
}

impl CdSource {
    /// Total frames of the CHD: each track padded to a multiple of four.
    pub fn total_frames(&self) -> u64 {
        self.tracks
            .iter()
            .map(|t| u64::from(t.frames().div_ceil(4) * 4))
            .sum()
    }
}

/// Opens a source by its extension.
pub fn open(path: &Path) -> Result<CdSource> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "cue" => {
            let text =
                std::fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
            let sheet = cue::parse(&String::from_utf8_lossy(&text))?;
            from_cue(&sheet, path.parent().unwrap_or(Path::new(".")))
        }
        "iso" => from_iso(path),
        "gdi" => crate::gdi::open(path),
        other => fail(format!(
            "{}: cannot tell what a .{other} file is (expected .cue or .iso)",
            path.display()
        )),
    }
}

pub fn from_iso(path: &Path) -> Result<CdSource> {
    let len = std::fs::metadata(path)
        .map_err(|e| Error(format!("{}: {e}", path.display())))?
        .len();
    if len == 0 || !len.is_multiple_of(2048) {
        return fail(format!(
            "{}: an iso file holds whole 2048-byte sectors, this one is {len} bytes",
            path.display()
        ));
    }
    Ok(CdSource {
        tracks: vec![SrcTrack {
            number: 1,
            kind: TrackType::Mode1,
            file: path.to_path_buf(),
            offset: 0,
            stored_pregap: 0,
            data_frames: (len / 2048) as u32,
            virtual_pregap: 0,
            postgap: 0,
            swap_audio: false,
            pad: None,
        }],
    })
}

pub fn from_cue(sheet: &CueSheet, dir: &Path) -> Result<CdSource> {
    let mut sizes = Vec::new();
    for f in &sheet.files {
        let path = dir.join(&f.name);
        let len = std::fs::metadata(&path)
            .map_err(|e| Error(format!("{}: {e}", path.display())))?
            .len();
        sizes.push((path, len));
    }
    let mut tracks = Vec::new();
    for (i, t) in sheet.tracks.iter().enumerate() {
        let kind = TrackType::from_name(&t.mode)?;
        let size = kind.data_size() as u64;
        let (path, file_len) = &sizes[t.file];
        let format = &sheet.files[t.file].format;
        if format != "BINARY" && format != "MOTOROLA" {
            return fail(format!(
                "{}: file type {format} is not supported (BINARY and MOTOROLA are)",
                path.display()
            ));
        }
        let index1 = u64::from(t.index1.expect("checked by the parser"));
        let start = u64::from(t.index0.unwrap_or(t.index1.expect("checked by the parser")));
        if t.index0.is_some() && t.pregap > 0 {
            return fail(format!(
                "track {} has both INDEX 00 and a PREGAP command; chdman does not take that either",
                t.number
            ));
        }
        // the data ends where the next track of the same file starts (its pregap, if stored), else with the file
        let next = sheet.tracks[i + 1..].iter().find(|n| n.file == t.file);
        let end_bytes = match next {
            Some(n) => {
                let nk = TrackType::from_name(&n.mode)?;
                if nk.data_size() != kind.data_size() {
                    return fail(format!(
                        "tracks {} and {} share a file but not a sector size",
                        t.number, n.number
                    ));
                }
                u64::from(n.index0.or(n.index1).expect("checked by the parser")) * size
            }
            None => *file_len,
        };
        let start_bytes = start * size;
        if end_bytes < start_bytes || end_bytes > *file_len {
            return fail(format!(
                "track {} lies outside {}",
                t.number,
                path.display()
            ));
        }
        if !(end_bytes - start_bytes).is_multiple_of(size) {
            return fail(format!(
                "track {} is not a whole number of {size}-byte sectors in {}",
                t.number,
                path.display()
            ));
        }
        let frames = ((end_bytes - start_bytes) / size) as u32;
        let stored_pregap = (index1 - start) as u32;
        if stored_pregap > frames {
            return fail(format!(
                "track {}'s pregap is longer than the track",
                t.number
            ));
        }
        tracks.push(SrcTrack {
            number: t.number,
            kind,
            file: path.clone(),
            offset: start_bytes,
            stored_pregap,
            data_frames: frames - stored_pregap,
            virtual_pregap: t.pregap,
            postgap: t.postgap,
            swap_audio: kind.is_audio() && format == "BINARY",
            pad: None,
        });
    }
    Ok(CdSource { tracks })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rcchd-src-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn a_cue_with_a_stored_pregap_and_a_virtual_one_gets_its_frames() {
        std::fs::write(scratch("d.bin"), vec![0u8; 2352 * 30]).unwrap();
        std::fs::write(scratch("a.bin"), vec![0u8; 2352 * 190]).unwrap();
        std::fs::write(scratch("b.bin"), vec![0u8; 2352 * 10]).unwrap();
        let sheet = cue::parse(
            "FILE d.bin BINARY\nTRACK 01 MODE1/2352\nINDEX 01 00:00:00\n\
             FILE a.bin BINARY\nTRACK 02 AUDIO\nINDEX 00 00:00:00\nINDEX 01 00:02:00\n\
             FILE b.bin BINARY\nTRACK 03 AUDIO\nPREGAP 00:02:00\nINDEX 01 00:00:00\n",
        )
        .unwrap();
        let src = from_cue(&sheet, scratch("x").parent().unwrap()).unwrap();
        let t: Vec<_> = src
            .tracks
            .iter()
            .map(|t| (t.number, t.frames(), t.stored_pregap, t.virtual_pregap))
            .collect();
        assert_eq!(t, [(1, 30, 0, 0), (2, 190, 150, 0), (3, 10, 0, 150)]);
        assert_eq!(
            src.tracks[1].meta().text(),
            "TRACK:2 TYPE:AUDIO SUBTYPE:NONE FRAMES:190 PREGAP:150 PGTYPE:VAUDIO PGSUB:NONE POSTGAP:0"
        );
        assert_eq!(
            src.tracks[2].meta().text(),
            "TRACK:3 TYPE:AUDIO SUBTYPE:NONE FRAMES:10 PREGAP:150 PGTYPE:MODE1 PGSUB:NONE POSTGAP:0"
        );
        assert!(src.tracks[1].swap_audio && !src.tracks[0].swap_audio);
        // 30 -> 32, 190 -> 192, 10 -> 12
        assert_eq!(src.total_frames(), 32 + 192 + 12);
    }

    #[test]
    fn two_tracks_in_one_file_split_where_the_next_begins() {
        std::fs::write(scratch("one.bin"), vec![0u8; 2352 * 50]).unwrap();
        let sheet = cue::parse("FILE one.bin BINARY\nTRACK 01 MODE1/2352\nINDEX 01 00:00:00\nTRACK 02 AUDIO\nINDEX 00 00:00:20\nINDEX 01 00:00:24\n").unwrap();
        let src = from_cue(&sheet, scratch("x").parent().unwrap()).unwrap();
        assert_eq!(
            (
                src.tracks[0].frames(),
                src.tracks[1].frames(),
                src.tracks[1].stored_pregap
            ),
            (20, 30, 4)
        );
        assert_eq!(src.tracks[1].offset, 2352 * 20);
    }

    #[test]
    fn bad_sources_are_refused_in_words() {
        std::fs::write(scratch("odd.iso"), vec![0u8; 3000]).unwrap();
        assert!(
            from_iso(&scratch("odd.iso"))
                .unwrap_err()
                .0
                .contains("whole 2048")
        );
        assert!(open(&scratch("nothing.xyz")).is_err());
        let sheet =
            cue::parse("FILE missing.bin BINARY\nTRACK 01 AUDIO\nINDEX 01 00:00:00\n").unwrap();
        assert!(from_cue(&sheet, scratch("x").parent().unwrap()).is_err());
    }
}
