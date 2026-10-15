//! Pieces of memory, where they sit, and the checks that they sit legally.

use crate::layout::{self, Gaps, Layout};
use std::fmt::Write;

/// A run of bytes at an address, with a name for messages and the map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub name: String,
    pub start: u16,
    pub bytes: Vec<u8>,
}

impl Piece {
    /// One past the last address (up to 0x10000).
    pub fn end(&self) -> u32 {
        self.start as u32 + self.bytes.len() as u32
    }

    pub fn range(&self) -> String {
        format!("${:04X}-${:04X}", self.start, self.end().saturating_sub(1))
    }
}

/// A named range of addresses, inclusive.
#[derive(Clone, Debug)]
pub struct Range {
    pub name: String,
    pub start: u16,
    pub end: u16,
}

pub fn regions(layout: &Layout) -> Result<Vec<Range>, String> {
    layout
        .region
        .iter()
        .map(|r| {
            let (start, end) = (
                layout::address(&r.start, &format!("region \"{}\" start", r.name))?,
                layout::address(&r.end, &format!("region \"{}\" end", r.name))?,
            );
            if end < start {
                return Err(format!(
                    "region \"{}\" ends (${end:04X}) before it starts (${start:04X})",
                    r.name
                ));
            }
            Ok(Range {
                name: r.name.clone(),
                start,
                end,
            })
        })
        .collect()
}

pub fn reserves(layout: &Layout) -> Result<Vec<Range>, String> {
    layout
        .reserve
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let name = r
                .why
                .clone()
                .unwrap_or_else(|| format!("reserved range {}", i + 1));
            let (start, end) = (
                layout::address(&r.start, &format!("{name} start"))?,
                layout::address(&r.end, &format!("{name} end"))?,
            );
            if end < start {
                return Err(format!(
                    "{name} ends (${end:04X}) before it starts (${start:04X})"
                ));
            }
            Ok(Range { name, start, end })
        })
        .collect()
}

/// Sorts the pieces by address and refuses overlaps, ranges outside every region, and reserved ranges.
pub fn check(pieces: &mut [Piece], regions: &[Range], reserves: &[Range]) -> Result<(), String> {
    pieces.sort_by_key(|p| (p.start, p.name.clone()));
    if let Some(p) = pieces.iter().find(|p| p.bytes.is_empty()) {
        return Err(format!("{} holds no bytes", p.name));
    }
    for w in pieces.windows(2) {
        if w[0].end() > w[1].start as u32 {
            return Err(format!(
                "{} ({}) overlaps {} ({})",
                w[0].name,
                w[0].range(),
                w[1].name,
                w[1].range()
            ));
        }
    }
    for p in pieces.iter() {
        let last = (p.end() - 1) as u16;
        if !regions.is_empty() && !regions.iter().any(|r| p.start >= r.start && last <= r.end) {
            let list = regions
                .iter()
                .map(|r| format!("{} ${:04X}-${:04X}", r.name, r.start, r.end))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "{} ({}) is outside every region ({list})",
                p.name,
                p.range()
            ));
        }
        if let Some(r) = reserves
            .iter()
            .find(|r| p.start <= r.end && last >= r.start)
        {
            return Err(format!(
                "{} ({}) is in the reserved range ${:04X}-${:04X} ({})",
                p.name,
                p.range(),
                r.start,
                r.end,
                r.name
            ));
        }
    }
    Ok(())
}

/// The pieces as runs of touching bytes (pieces that meet are joined), sorted. `pieces` must be checked.
pub fn runs(pieces: &[Piece]) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for p in pieces {
        match out.last_mut() {
            Some(last) if last.end() == p.start as u32 => last.bytes.extend_from_slice(&p.bytes),
            _ => out.push(p.clone()),
        }
    }
    out
}

/// One image from the lowest address to the highest. A hole is an error, or with [`Gaps::Fill`] filled.
pub fn image(
    pieces: &[Piece],
    gaps: Gaps,
    fill: u8,
    notes: &mut Vec<String>,
) -> Result<(u16, Vec<u8>), String> {
    let Some(first) = pieces.first() else {
        return Err("there is nothing to put in the image".to_string());
    };
    let mut out = Vec::new();
    let mut at = first.start as u32;
    for p in pieces {
        if (p.start as u32) > at {
            let hole = p.start as u32 - at;
            match gaps {
                Gaps::Error => {
                    return Err(format!(
                        "a hole of {hole} bytes (${at:04X}-${:04X}) lies before {}; the image has to be in one piece \
                         (set gaps = \"fill\" to fill holes)",
                        p.start - 1,
                        p.name
                    ));
                }
                Gaps::Fill => {
                    out.resize(out.len() + hole as usize, fill);
                    notes.push(format!(
                        "filled a hole of {hole} bytes (${at:04X}-${:04X}) with ${fill:02X}",
                        p.start - 1
                    ));
                }
            }
        }
        out.extend_from_slice(&p.bytes);
        at = p.end();
    }
    Ok((first.start, out))
}

/// The map file: every piece, the regions and how full they are.
pub fn map_text(
    title: &str,
    entry: Option<(u16, String)>,
    pieces: &[Piece],
    regions: &[Range],
) -> String {
    let mut out = String::new();
    writeln!(out, "{title}").unwrap();
    match entry {
        Some((addr, how)) => writeln!(out, "entry: ${addr:04X} ({how})").unwrap(),
        None => writeln!(out, "entry: none").unwrap(),
    }
    writeln!(out, "\npieces:").unwrap();
    for p in pieces {
        writeln!(
            out,
            "  {}  {:>6} bytes  {}",
            p.range(),
            p.bytes.len(),
            p.name
        )
        .unwrap();
    }
    let total: usize = pieces.iter().map(|p| p.bytes.len()).sum();
    writeln!(out, "  total {total} bytes").unwrap();
    if !regions.is_empty() {
        writeln!(out, "\nregions:").unwrap();
        for r in regions {
            let size = r.end as u32 - r.start as u32 + 1;
            let used: u32 = pieces
                .iter()
                .filter(|p| p.start >= r.start && p.end() - 1 <= r.end as u32)
                .map(|p| p.bytes.len() as u32)
                .sum();
            writeln!(
                out,
                "  {:<10} ${:04X}-${:04X}  used {used} of {size} ({}%)",
                r.name,
                r.start,
                r.end,
                used * 100 / size
            )
            .unwrap();
        }
    }
    out
}
