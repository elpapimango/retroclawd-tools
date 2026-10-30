//! What `info` and `verify` print.

use chd::{Codec, Image};
use std::fmt::Write;

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Thousands separators, as chdman prints sizes.
pub fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn printable(data: &[u8]) -> Option<String> {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    let text = std::str::from_utf8(&data[..end]).ok()?;
    (!text.is_empty() && text.chars().all(|c| !c.is_control() || c == '\n'))
        .then(|| text.to_string())
}

/// The report of `info`; `verbose` adds the per-hunk codec statistics.
pub fn info(image: &Image, name: &str, verbose: bool) -> String {
    let h = image.header();
    let mut s = String::new();
    let _ = writeln!(s, "File:         {name}");
    let _ = writeln!(s, "File Version: 5");
    let _ = writeln!(s, "Logical size: {} bytes", grouped(h.logical_bytes));
    let _ = writeln!(
        s,
        "Hunk Size:    {} bytes",
        grouped(u64::from(h.hunk_bytes))
    );
    let _ = writeln!(
        s,
        "Total Hunks:  {}",
        grouped(u64::from(image.hunk_count()))
    );
    let _ = writeln!(
        s,
        "Unit Size:    {} bytes",
        grouped(u64::from(h.unit_bytes))
    );
    let _ = writeln!(
        s,
        "Total Units:  {}",
        grouped(h.logical_bytes.div_ceil(u64::from(h.unit_bytes)))
    );
    let codecs: Vec<String> = h
        .compressors
        .iter()
        .filter(|t| **t != [0; 4])
        .map(|t| {
            Codec::from_tag(*t).map_or_else(
                || String::from_utf8_lossy(t).into_owned(),
                |c| c.name().to_string(),
            )
        })
        .collect();
    let _ = writeln!(
        s,
        "Compression:  {}",
        if codecs.is_empty() {
            "none".to_string()
        } else {
            codecs.join(", ")
        }
    );
    let _ = writeln!(s, "CHD size:     {} bytes", grouped(image.file_len()));
    if h.logical_bytes > 0 {
        let _ = writeln!(
            s,
            "Ratio:        {:.1}%",
            image.file_len() as f64 * 100.0 / h.logical_bytes as f64
        );
    }
    if h.compressed() {
        let _ = writeln!(s, "SHA1:         {}", hex(&h.sha1));
        let _ = writeln!(s, "Data SHA1:    {}", hex(&h.raw_sha1));
    }
    if h.has_parent() {
        let _ = writeln!(s, "Parent SHA1:  {}", hex(&h.parent_sha1));
    }
    for (i, e) in image.metadata().iter().enumerate() {
        let tag = String::from_utf8_lossy(&e.tag);
        let _ = writeln!(
            s,
            "Metadata:     Tag='{tag}'  Index={i}  Length={} bytes",
            e.data.len()
        );
        match printable(&e.data) {
            Some(text) => {
                let _ = writeln!(s, "              {text}");
            }
            None => {
                let preview: Vec<String> =
                    e.data.iter().take(32).map(|b| format!("{b:02x}")).collect();
                let _ = writeln!(
                    s,
                    "              {}{}",
                    preview.join(" "),
                    if e.data.len() > 32 { " ..." } else { "" }
                );
            }
        }
    }
    if verbose {
        let mut counts = [0u32; 4];
        let (mut plain, mut selfs, mut parents) = (0u32, 0u32, 0u32);
        for n in 0..image.hunk_count() {
            match image.hunk_kind(n) {
                Some(chd::HunkKind::Codec(slot)) => counts[usize::from(slot)] += 1,
                Some(chd::HunkKind::Plain) => plain += 1,
                Some(chd::HunkKind::SelfRef(_)) => selfs += 1,
                Some(chd::HunkKind::Parent(_)) => parents += 1,
                None => {}
            }
        }
        let _ = writeln!(s, "     Hunks  Percent  Name");
        let _ = writeln!(
            s,
            "----------  -------  ------------------------------------"
        );
        let total = f64::from(image.hunk_count().max(1));
        for (slot, tag) in h
            .compressors
            .iter()
            .enumerate()
            .filter(|(_, t)| **t != [0; 4])
        {
            let _ = writeln!(
                s,
                "{:>10}  {:>6.1}%  {}",
                counts[slot],
                f64::from(counts[slot]) * 100.0 / total,
                String::from_utf8_lossy(tag)
            );
        }
        for (n, label) in [
            (plain, "uncompressed"),
            (selfs, "same as another hunk"),
            (parents, "from the parent"),
        ] {
            if n > 0 {
                let _ = writeln!(
                    s,
                    "{:>10}  {:>6.1}%  {label}",
                    n,
                    f64::from(n) * 100.0 / total
                );
            }
        }
    }
    s
}
