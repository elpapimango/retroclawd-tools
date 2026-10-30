//! The kinds of CD tracks a CHD describes, with their names as the metadata text spells them and the bytes of user data each holds.

use crate::{Result, fail};

/// One CD frame in a CHD: a 2352-byte sector then 96 bytes of subchannel data.
pub const FRAME_SIZE: usize = chd::FRAME_SIZE;
/// Frames per second of CD time (the minutes:seconds:frames of a cue sheet).
pub const FRAMES_PER_SECOND: u32 = 75;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackType {
    /// 2048 bytes of user data per sector.
    Mode1,
    /// The whole 2352-byte sector.
    Mode1Raw,
    /// 2336 bytes: a mode 2 sector without sync and header.
    Mode2,
    Mode2Form1,
    Mode2Form2,
    Mode2FormMix,
    Mode2Raw,
    Audio,
}

impl TrackType {
    pub fn name(self) -> &'static str {
        match self {
            TrackType::Mode1 => "MODE1",
            TrackType::Mode1Raw => "MODE1_RAW",
            TrackType::Mode2 => "MODE2",
            TrackType::Mode2Form1 => "MODE2_FORM1",
            TrackType::Mode2Form2 => "MODE2_FORM2",
            TrackType::Mode2FormMix => "MODE2_FORM_MIX",
            TrackType::Mode2Raw => "MODE2_RAW",
            TrackType::Audio => "AUDIO",
        }
    }

    pub fn from_name(name: &str) -> Result<TrackType> {
        Ok(match name {
            "MODE1" | "MODE1/2048" => TrackType::Mode1,
            "MODE1_RAW" | "MODE1/2352" => TrackType::Mode1Raw,
            "MODE2" | "MODE2/2336" => TrackType::Mode2,
            "MODE2_FORM1" | "MODE2/2048" => TrackType::Mode2Form1,
            "MODE2_FORM2" | "MODE2/2324" => TrackType::Mode2Form2,
            "MODE2_FORM_MIX" => TrackType::Mode2FormMix,
            "MODE2_RAW" | "MODE2/2352" | "CDI/2352" => TrackType::Mode2Raw,
            "AUDIO" => TrackType::Audio,
            other => return fail(format!("unknown track type {other}")),
        })
    }

    /// Bytes of a sector's data as the track keeps them in a file and at the start of its frames.
    pub fn data_size(self) -> usize {
        match self {
            TrackType::Mode1 | TrackType::Mode2Form1 => 2048,
            TrackType::Mode2Form2 => 2324,
            TrackType::Mode2 | TrackType::Mode2FormMix => 2336,
            TrackType::Mode1Raw | TrackType::Mode2Raw | TrackType::Audio => 2352,
        }
    }

    pub fn is_audio(self) -> bool {
        self == TrackType::Audio
    }

    /// How a cue sheet spells the track's mode.
    pub fn cue_mode(self) -> String {
        match self {
            TrackType::Audio => "AUDIO".to_string(),
            TrackType::Mode1 => "MODE1/2048".to_string(),
            TrackType::Mode1Raw => "MODE1/2352".to_string(),
            TrackType::Mode2 | TrackType::Mode2FormMix => "MODE2/2336".to_string(),
            TrackType::Mode2Form1 => "MODE2/2048".to_string(),
            TrackType::Mode2Form2 => "MODE2/2324".to_string(),
            TrackType::Mode2Raw => "MODE2/2352".to_string(),
        }
    }
}

/// The subchannel data a track carries in its frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubType {
    None,
    Rw,
    RwRaw,
}

impl SubType {
    pub fn name(self) -> &'static str {
        match self {
            SubType::None => "NONE",
            SubType::Rw => "RW",
            SubType::RwRaw => "RW_RAW",
        }
    }

    pub fn from_name(name: &str) -> Result<SubType> {
        Ok(match name {
            "NONE" => SubType::None,
            "RW" => SubType::Rw,
            "RW_RAW" => SubType::RwRaw,
            other => return fail(format!("unknown subchannel type {other}")),
        })
    }
}

/// One track as the CHD's `CHT2` metadata describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackMeta {
    pub number: u32,
    pub kind: TrackType,
    pub sub: SubType,
    /// Frames the track takes in the file, a stored pregap included.
    pub frames: u32,
    pub pregap: u32,
    /// The pregap type: a name starting with `V` says the pregap's frames are stored.
    pub pregap_type: String,
    pub pregap_sub: String,
    pub postgap: u32,
    /// A GD-ROM track (`CHGD`) also records how many of its frames are padding after the data.
    pub pad: Option<u32>,
}

impl TrackMeta {
    pub fn pregap_stored(&self) -> bool {
        self.pregap_type.starts_with('V')
    }

    /// The metadata tag the description is stored under.
    pub fn tag(&self) -> [u8; 4] {
        if self.pad.is_some() {
            *b"CHGD"
        } else {
            *b"CHT2"
        }
    }

    /// The metadata text, as chdman writes it (the terminating zero is added by the caller).
    pub fn text(&self) -> String {
        match self.pad {
            Some(pad) => format!(
                "TRACK:{} TYPE:{} SUBTYPE:{} FRAMES:{} PAD:{} PREGAP:{} PGTYPE:{} PGSUB:{} POSTGAP:{}",
                self.number,
                self.kind.name(),
                self.sub.name(),
                self.frames,
                pad,
                self.pregap,
                self.pregap_type,
                self.pregap_sub,
                self.postgap
            ),
            None => format!(
                "TRACK:{} TYPE:{} SUBTYPE:{} FRAMES:{} PREGAP:{} PGTYPE:{} PGSUB:{} POSTGAP:{}",
                self.number,
                self.kind.name(),
                self.sub.name(),
                self.frames,
                self.pregap,
                self.pregap_type,
                self.pregap_sub,
                self.postgap
            ),
        }
    }

    /// Reads a `CHT2` (or the older `CHTR`, which has no pregap fields) description.
    pub fn parse(data: &[u8]) -> Result<TrackMeta> {
        let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
        let text = String::from_utf8_lossy(&data[..end]).into_owned();
        let field = |name: &str| -> Option<&str> {
            text.split_ascii_whitespace()
                .find_map(|part| part.strip_prefix(name)?.strip_prefix(':'))
        };
        let number = |name: &str| -> Result<u32> {
            field(name)
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| crate::Error(format!("a track description lacks {name}: {text}")))
        };
        let get = |name: &str| -> Result<&str> {
            field(name)
                .ok_or_else(|| crate::Error(format!("a track description lacks {name}: {text}")))
        };
        Ok(TrackMeta {
            number: number("TRACK")?,
            kind: TrackType::from_name(get("TYPE")?)?,
            sub: SubType::from_name(field("SUBTYPE").unwrap_or("NONE"))?,
            frames: number("FRAMES")?,
            pregap: number("PREGAP").unwrap_or(0),
            pregap_type: field("PGTYPE").unwrap_or("MODE1").to_string(),
            pregap_sub: field("PGSUB").unwrap_or("NONE").to_string(),
            postgap: number("POSTGAP").unwrap_or(0),
            pad: number("PAD").ok(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_description_reads_back_as_written() {
        let t = TrackMeta {
            number: 2,
            kind: TrackType::Audio,
            sub: SubType::None,
            frames: 190,
            pregap: 150,
            pregap_type: "VAUDIO".into(),
            pregap_sub: "NONE".into(),
            postgap: 0,
            pad: None,
        };
        let text = t.text();
        assert_eq!(
            text,
            "TRACK:2 TYPE:AUDIO SUBTYPE:NONE FRAMES:190 PREGAP:150 PGTYPE:VAUDIO PGSUB:NONE POSTGAP:0"
        );
        assert_eq!(TrackMeta::parse(text.as_bytes()).unwrap(), t);
        assert!(t.pregap_stored());
        assert!(TrackMeta::parse(b"TRACK:1 SUBTYPE:NONE FRAMES:3").is_err());
        let gd = TrackMeta {
            pad: Some(590),
            frames: 600,
            ..t.clone()
        };
        assert_eq!(&gd.tag(), b"CHGD");
        assert!(gd.text().contains("FRAMES:600 PAD:590 PREGAP:150"));
        assert_eq!(TrackMeta::parse(gd.text().as_bytes()).unwrap(), gd);
    }

    #[test]
    fn cue_modes_and_names_agree_on_sizes() {
        for (mode, kind, size) in [
            ("MODE1/2048", TrackType::Mode1, 2048),
            ("MODE1/2352", TrackType::Mode1Raw, 2352),
            ("MODE2/2336", TrackType::Mode2, 2336),
            ("MODE2/2048", TrackType::Mode2Form1, 2048),
            ("MODE2/2352", TrackType::Mode2Raw, 2352),
            ("AUDIO", TrackType::Audio, 2352),
        ] {
            assert_eq!(TrackType::from_name(mode).unwrap(), kind);
            assert_eq!(kind.data_size(), size);
            assert_eq!(TrackType::from_name(&kind.cue_mode()).unwrap(), kind);
        }
        assert!(TrackType::from_name("CDG").is_err());
    }
}
