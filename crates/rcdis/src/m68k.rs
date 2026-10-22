//! The 68000 disassembler: a linear sweep that writes source `rcasm --cpu 68000` assembles back to the same bytes.
//!
//! The instruction text is the CPU crate's own disassembler (`mc68000::disassemble`), which is driven by the table the
//! core executes from, so what is shown is what runs. Every instruction is assembled again before it is trusted: a
//! word that is no instruction, one whose extension words carry bits the assembler would not produce (the unused bits
//! of an index word or a byte immediate), and anything that would not come back identical, is written as `dc.w`
//! instead. Branch targets, `dbcc` targets and PC-relative operands inside the image get labels (`L_` and the
//! address in hex); anything else stays a number.

use crate::Disassembly;
use rcasm::Files;
use rcasm::m68k::{M68kOptions, assemble_m68k};
use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::Path;

pub struct M68kDisOptions {
    /// The address of the first byte (even).
    pub org: u32,
    /// Add `; ADDR: BYTES` to every line.
    pub comments: bool,
}

struct NoFiles;

impl Files for NoFiles {
    fn read(&self, path: &Path) -> Result<Vec<u8>, String> {
        Err(format!("no file {}", path.display()))
    }
}

enum Item {
    Insn { addr: u32, len: usize, text: String },
    Words { addr: u32, words: Vec<u16> },
    Byte { addr: u32, byte: u8 },
}

/// Whether `text` assembles at `addr` to exactly `bytes`.
fn reassembles(text: &str, addr: u32, bytes: &[u8]) -> bool {
    let source = format!("    .org ${addr:x}\n    {text}\n");
    assemble_m68k("check", &source, &M68kOptions::default(), &NoFiles)
        .is_ok_and(|a| a.image() == bytes)
}

fn is_branch(mnemonic: &str) -> bool {
    let stem = mnemonic.split('.').next().unwrap_or(mnemonic);
    (stem.starts_with('b') && !matches!(stem, "bchg" | "bclr" | "bset" | "btst"))
        || stem.starts_with("db")
}

/// The address a branch or PC-relative operand of `text` names, if any.
fn target_of(text: &str) -> Option<u32> {
    let (mnemonic, operands) = text.split_once(' ')?;
    let hex_at = |s: &str| -> Option<(usize, u32)> {
        let i = s.rfind('$')?;
        let digits: String = s[i + 1..]
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect();
        u32::from_str_radix(&digits, 16).ok().map(|v| (i, v))
    };
    if is_branch(mnemonic) {
        return hex_at(operands).map(|(_, v)| v);
    }
    if let Some(p) = operands.find("(pc") {
        return hex_at(&operands[..p]).map(|(_, v)| v);
    }
    None
}

pub fn disassemble_m68k(image: &[u8], opts: &M68kDisOptions) -> Result<Disassembly, String> {
    if !opts.org.is_multiple_of(2) {
        return Err("68000 code starts at an even address".to_string());
    }
    if opts.org as u64 + image.len() as u64 > 0x100_0000 {
        return Err("the image runs past the 24-bit address space".to_string());
    }
    let mut items = Vec::new();
    let mut pos = 0usize;
    while pos < image.len() {
        let addr = opts.org + pos as u32;
        if image.len() - pos < 2 {
            items.push(Item::Byte {
                addr,
                byte: image[pos],
            });
            pos += 1;
            continue;
        }
        let (text, len) = mc68000::disassemble::disassemble(&image[pos..], addr);
        let word = |at: usize| u16::from_be_bytes([image[at], image[at + 1]]);
        if text.starts_with("dc.") || len == 0 || !reassembles(&text, addr, &image[pos..pos + len])
        {
            // A word that is data, or that the assembler would write differently: keep the word as it is. The next
            // word may start an instruction again.
            items.push(Item::Words {
                addr,
                words: vec![word(pos)],
            });
            pos += 2;
            continue;
        }
        items.push(Item::Insn { addr, len, text });
        pos += len;
    }
    // Labels: targets that fall on an instruction's first byte.
    let starts: BTreeSet<u32> = items
        .iter()
        .filter_map(|i| match i {
            Item::Insn { addr, .. } => Some(*addr),
            _ => None,
        })
        .collect();
    let labels: BTreeSet<u32> = items
        .iter()
        .filter_map(|i| match i {
            Item::Insn { text, .. } => target_of(text).filter(|t| starts.contains(t)),
            _ => None,
        })
        .collect();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "; rcdis: 68000, {} bytes at ${:06X}",
        image.len(),
        opts.org
    );
    let _ = writeln!(out, "    .org ${:x}", opts.org);
    let (mut instructions, mut data_bytes) = (0usize, 0usize);
    let comment = |addr: u32, bytes: &[u8]| -> String {
        if opts.comments {
            let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
            format!("    ; {addr:06X}: {}", hex.join(" "))
        } else {
            String::new()
        }
    };
    let mut i = 0;
    while i < items.len() {
        match &items[i] {
            Item::Insn { addr, len, text } => {
                if labels.contains(addr) {
                    let _ = writeln!(out, "L_{addr:06x}:");
                }
                let mut shown = text.clone();
                if let Some(t) = target_of(text).filter(|t| labels.contains(t)) {
                    shown = shown.replacen(&format!("${t:x}"), &format!("L_{t:06x}"), 1);
                }
                let at = (*addr - opts.org) as usize;
                let _ = writeln!(out, "    {shown}{}", comment(*addr, &image[at..at + len]));
                instructions += 1;
                i += 1;
            }
            Item::Words { addr, .. } => {
                // A run of data words in one `dc.w` (up to eight to a line).
                let start = i;
                let mut words = Vec::new();
                while i < items.len() && words.len() < 8 {
                    match &items[i] {
                        Item::Words { words: w, .. } => words.extend(w.iter().copied()),
                        _ => break,
                    }
                    i += 1;
                }
                let _ = start;
                let list: Vec<String> = words.iter().map(|w| format!("${w:04x}")).collect();
                let at = (*addr - opts.org) as usize;
                let _ = writeln!(
                    out,
                    "    dc.w {}{}",
                    list.join(","),
                    comment(*addr, &image[at..at + 2 * words.len()])
                );
                data_bytes += 2 * words.len();
            }
            Item::Byte { addr, byte } => {
                let _ = writeln!(out, "    dc.b ${byte:02x}{}", comment(*addr, &[*byte]));
                data_bytes += 1;
                i += 1;
            }
        }
    }
    Ok(Disassembly {
        source: out,
        instructions,
        data_bytes,
        labels: labels.len(),
    })
}
