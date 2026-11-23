//! The 65816 disassembler: a linear sweep that tracks `REP` and `SEP` and writes source `rcasm --cpu 65816`
//! assembles back to the same bytes.
//!
//! The instruction text is the CPU crate's (`w65c816::disassemble`). The length of an immediate depends on the `M` and
//! `X` flags, so the sweep follows `REP`/`SEP` (the assembler's smart mode does the same when it reads the output);
//! the starting widths are an option. Every instruction is assembled again before it is trusted: one the assembler
//! would not write back identically is written as `.byte`. Branch, jump and call targets inside the image get labels.

use crate::Disassembly;
use rcasm::{Cpu, DiskFiles, Options};
use std::collections::BTreeSet;
use std::fmt::Write;

pub struct W65816Options {
    /// The 24-bit address of the first byte.
    pub org: u32,
    /// The accumulator / index registers are 16 bits wide at the start.
    pub m16: bool,
    pub x16: bool,
    /// Add `; ADDR: BYTES` to every line.
    pub comments: bool,
}

fn reassembles(text: &str, addr: u32, m16: bool, x16: bool, bytes: &[u8]) -> bool {
    let source = format!(
        ".{}\n.{}\n.org ${addr:06X}\n{text}\n",
        if m16 { "a16" } else { "a8" },
        if x16 { "i16" } else { "i8" }
    );
    let opts = Options {
        cpu: Some(Cpu::W65816),
        ..Options::default()
    };
    rcasm::assemble("check", &source, &opts, &DiskFiles).is_ok_and(|a| {
        a.segments
            .iter()
            .flat_map(|s| s.bytes.iter().copied())
            .eq(bytes.iter().copied())
    })
}

/// The address a branch, jump or call goes to, if the instruction is one (a 24-bit address).
fn target_of(text: &str, addr: u32) -> Option<u32> {
    let (mn, operand) = text.split_once(' ')?;
    let value = |s: &str| u32::from_str_radix(s.strip_prefix('$')?, 16).ok();
    match mn {
        "BPL" | "BMI" | "BVC" | "BVS" | "BCC" | "BCS" | "BNE" | "BEQ" | "BRA" | "BRL" | "JMP"
        | "JSR" => value(operand).map(|t| (addr & 0xFF_0000) | t),
        "JML" | "JSL" => value(operand),
        _ => None,
    }
}

fn ends_flow(text: &str) -> bool {
    let mn = text.split(' ').next().unwrap_or("");
    matches!(
        mn,
        "RTS" | "RTL" | "RTI" | "JMP" | "JML" | "BRA" | "BRL" | "STP"
    )
}

pub fn disassemble(image: &[u8], o: &W65816Options) -> Result<Disassembly, String> {
    let end = o.org as u64 + image.len() as u64;
    if end > 0x100_0000 {
        return Err(format!(
            "{} bytes at ${:06X} run past $FFFFFF",
            image.len(),
            o.org
        ));
    }
    let in_image = |a: u32| (o.org..end as u32).contains(&a);
    struct Row {
        addr: u32,
        len: usize,
        text: Option<String>,
    }
    let mut rows: Vec<Row> = Vec::new();
    let (mut m16, mut x16) = (o.m16, o.x16);
    let mut i = 0;
    let mut targets: BTreeSet<u32> = BTreeSet::new();
    let _ = ends_flow;
    while i < image.len() {
        let addr = o.org + i as u32;
        let window = &image[i..image.len().min(i + 4)];
        let (text, len) = w65c816::disassemble::disassemble(window, addr, m16, x16);
        if i + len <= image.len() && reassembles(&text, addr, m16, x16, &image[i..i + len]) {
            if let Some(t) = target_of(&text, addr).filter(|t| in_image(*t)) {
                targets.insert(t);
            }
            // REP / SEP change what the next immediates are
            if len == 2 && (image[i] == 0xC2 || image[i] == 0xE2) {
                let set = image[i] == 0xE2;
                let v = image[i + 1];
                if v & 0x20 != 0 {
                    m16 = !set;
                }
                if v & 0x10 != 0 {
                    x16 = !set;
                }
            }
            rows.push(Row {
                addr,
                len,
                text: Some(text),
            });
            i += len;
        } else {
            rows.push(Row {
                addr,
                len: 1,
                text: None,
            });
            i += 1;
        }
    }
    // a label sits only where an item starts
    let starts: BTreeSet<u32> = rows.iter().map(|r| r.addr).collect();
    let labels: BTreeSet<u32> = targets.into_iter().filter(|t| starts.contains(t)).collect();
    let mut body = String::new();
    let (mut instructions, mut data_bytes) = (0, 0);
    for r in &rows {
        if labels.contains(&r.addr) {
            let _ = writeln!(body, "L_{:06X}:", r.addr);
        }
        let at = (r.addr - o.org) as usize;
        match &r.text {
            Some(text) => {
                let shown = match target_of(text, r.addr).filter(|t| labels.contains(t)) {
                    Some(t) => {
                        let (mn, _) = text.split_once(' ').unwrap_or((text, ""));
                        format!("{mn} L_{t:06X}")
                    }
                    None => text.clone(),
                };
                let _ = write!(body, "        {shown}");
                instructions += 1;
            }
            None => {
                let _ = write!(body, "        .byte ${:02X}", image[at]);
                data_bytes += 1;
            }
        }
        if o.comments {
            let bytes: Vec<String> = image[at..at + r.len]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect();
            let _ = write!(body, "  ; {:06X}: {}", r.addr, bytes.join(" "));
        }
        body.push('\n');
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "; rcdis: {} bytes at ${:06X}-${:06X}, linear",
        image.len(),
        o.org,
        end.saturating_sub(1)
    );
    let _ = writeln!(out, ".cpu 65816");
    let _ = writeln!(out, ".{}", if o.m16 { "a16" } else { "a8" });
    let _ = writeln!(out, ".{}", if o.x16 { "i16" } else { "i8" });
    let _ = writeln!(out, "        .org ${:06X}", o.org);
    out.push_str(&body);
    Ok(Disassembly {
        source: out,
        instructions,
        data_bytes,
        labels: labels.len(),
    })
}
