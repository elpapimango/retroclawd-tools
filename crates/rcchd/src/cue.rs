//! Cue sheets: the commands that matter for making a CHD (FILE, TRACK, INDEX, PREGAP, POSTGAP) and nothing else.

use crate::types::FRAMES_PER_SECOND;
use crate::{Error, Result, fail};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CueFile {
    pub name: String,
    /// `BINARY` (little-endian audio) or `MOTOROLA` (big-endian audio).
    pub format: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CueTrack {
    pub number: u32,
    /// The mode as the sheet spells it (`AUDIO`, `MODE1/2352`, ...).
    pub mode: String,
    /// Index into `CueSheet::files`.
    pub file: usize,
    /// Frames from the start of the file, by index number.
    pub index0: Option<u32>,
    pub index1: Option<u32>,
    /// `PREGAP`: a gap not in the file.
    pub pregap: u32,
    pub postgap: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CueSheet {
    pub files: Vec<CueFile>,
    pub tracks: Vec<CueTrack>,
}

/// `mm:ss:ff` as frames.
pub fn parse_time(text: &str) -> Result<u32> {
    let parts: Vec<&str> = text.split(':').collect();
    let num = |s: &str| {
        s.parse::<u32>()
            .map_err(|_| Error(format!("bad time {text}")))
    };
    match parts.as_slice() {
        [m, s, f] => {
            let (m, s, f) = (num(m)?, num(s)?, num(f)?);
            if s >= 60 || f >= FRAMES_PER_SECOND {
                return fail(format!("bad time {text}"));
            }
            Ok((m * 60 + s) * FRAMES_PER_SECOND + f)
        }
        _ => fail(format!("bad time {text}")),
    }
}

/// Frames as `mm:ss:ff`.
pub fn format_time(frames: u32) -> String {
    let f = frames % FRAMES_PER_SECOND;
    let s = frames / FRAMES_PER_SECOND;
    format!("{:02}:{:02}:{:02}", s / 60, s % 60, f)
}

/// Splits a line into words; a word in double quotes may hold spaces.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
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

pub fn parse(text: &str) -> Result<CueSheet> {
    let mut sheet = CueSheet::default();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim_start_matches('\u{feff}').trim();
        let w = words(line);
        let Some(command) = w.first() else { continue };
        let at = |msg: &str| Error(format!("cue line {}: {msg}", n + 1));
        match command.to_ascii_uppercase().as_str() {
            "FILE" => {
                if w.len() < 3 {
                    return Err(at("FILE needs a name and a type"));
                }
                sheet.files.push(CueFile {
                    name: w[1].clone(),
                    format: w[2].to_ascii_uppercase(),
                });
            }
            "TRACK" => {
                if w.len() < 3 {
                    return Err(at("TRACK needs a number and a mode"));
                }
                if sheet.files.is_empty() {
                    return Err(at("TRACK before any FILE"));
                }
                sheet.tracks.push(CueTrack {
                    number: w[1].parse().map_err(|_| at("bad track number"))?,
                    mode: w[2].to_ascii_uppercase(),
                    file: sheet.files.len() - 1,
                    index0: None,
                    index1: None,
                    pregap: 0,
                    postgap: 0,
                });
            }
            "INDEX" => {
                let track = sheet
                    .tracks
                    .last_mut()
                    .ok_or_else(|| at("INDEX before any TRACK"))?;
                if w.len() < 3 {
                    return Err(at("INDEX needs a number and a time"));
                }
                let time = parse_time(&w[2]).map_err(|e| at(&e.0))?;
                match w[1].as_str() {
                    "0" | "00" => track.index0 = Some(time),
                    "1" | "01" => track.index1 = Some(time),
                    _ => {} // further indices do not change what the file holds
                }
            }
            "PREGAP" | "POSTGAP" => {
                let track = sheet
                    .tracks
                    .last_mut()
                    .ok_or_else(|| at("a gap before any TRACK"))?;
                let time = parse_time(w.get(1).map_or("", String::as_str)).map_err(|e| at(&e.0))?;
                if command.eq_ignore_ascii_case("PREGAP") {
                    track.pregap = time;
                } else {
                    track.postgap = time;
                }
            }
            _ => {}
        }
    }
    if sheet.tracks.is_empty() {
        return fail("the cue sheet has no tracks");
    }
    for t in &sheet.tracks {
        if t.index1.is_none() {
            return fail(format!("track {} has no INDEX 01", t.number));
        }
    }
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_convert_both_ways() {
        assert_eq!(parse_time("00:02:00").unwrap(), 150);
        assert_eq!(parse_time("12:34:56").unwrap(), (12 * 60 + 34) * 75 + 56);
        assert_eq!(format_time(150), "00:02:00");
        assert_eq!(format_time((12 * 60 + 34) * 75 + 56), "12:34:56");
        assert!(parse_time("00:60:00").is_err());
        assert!(parse_time("00:00:75").is_err());
        assert!(parse_time("1:2").is_err());
    }

    #[test]
    fn a_sheet_with_two_files_quotes_and_odd_commands_parses() {
        let sheet = parse(
            "REM COMMENT \"x\"\nCATALOG 123\nFILE \"my disc.bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\n\
             FILE other.bin MOTOROLA\n  TRACK 02 AUDIO\n    FLAGS DCP\n    PREGAP 00:02:00\n    INDEX 00 00:00:00\n    INDEX 01 00:00:20\n    POSTGAP 00:01:00\n",
        )
        .unwrap();
        assert_eq!(sheet.files[0].name, "my disc.bin");
        assert_eq!(
            sheet.files[1],
            CueFile {
                name: "other.bin".into(),
                format: "MOTOROLA".into()
            }
        );
        let t = &sheet.tracks[1];
        assert_eq!(
            (t.number, t.file, t.index0, t.index1, t.pregap, t.postgap),
            (2, 1, Some(0), Some(20), 150, 75)
        );
    }

    #[test]
    fn broken_sheets_are_refused_with_a_line() {
        assert!(parse("").is_err());
        assert!(parse("TRACK 01 AUDIO\n").unwrap_err().0.contains("line 1"));
        assert!(
            parse("FILE a BINARY\nTRACK 01 AUDIO\n")
                .unwrap_err()
                .0
                .contains("INDEX 01")
        );
        assert!(parse("FILE a BINARY\nTRACK 01 AUDIO\nINDEX 01 xx\n").is_err());
    }
}
