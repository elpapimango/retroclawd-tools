//! Debug information (`.rcdbg`): which source line made which bytes. A text format a debugger reads without a library:
//!
//! ```text
//! rcdbg 1
//! file 0 src/main.asm
//! line 0 12 0xE000 3        # file index, line, address, length in bytes
//! sym reset 0xE000
//! ```
//!
//! A program the system loads where it likes (an Atari ST `.PRG`, an Amiga or X68000 executable) has addresses that are
//! offsets from the start of its code, and two more records say how a debugger finds that start:
//!
//! ```text
//! base text gemdos          # offsets from the text; `gemdos`: the GEMDOS basepage 256 bytes before it names it; `scan`: no check
//! anchor 41F9????????4E75   # the first bytes of the text, `??` where the loader patches an address
//! ```
//!
//! Lines starting with `#` and blank lines are ignored. `parse` never panics on any input.

use std::fmt::Write;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DebugInfo {
    pub files: Vec<String>,
    /// (file index, line, address, length), in the order written.
    pub lines: Vec<(u32, u32, u32, u32)>,
    pub symbols: Vec<(String, u32)>,
    /// `base text METHOD`: the addresses are offsets from where the program's text was loaded.
    pub base: Option<String>,
    /// `anchor HEX`: the text's first bytes, `??` for the ones the loader changes.
    pub anchor: Option<String>,
}

impl DebugInfo {
    pub fn file_index(&mut self, name: &str) -> u32 {
        match self.files.iter().position(|f| f == name) {
            Some(i) => i as u32,
            None => {
                self.files.push(name.to_string());
                self.files.len() as u32 - 1
            }
        }
    }

    pub fn add_line(&mut self, file: &str, line: u32, addr: u32, len: u32) {
        let f = self.file_index(file);
        self.lines.push((f, line, addr, len));
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from("rcdbg 1\n");
        if let Some(b) = &self.base {
            let _ = writeln!(out, "base {b}");
        }
        if let Some(a) = &self.anchor {
            let _ = writeln!(out, "anchor {a}");
        }
        for (i, f) in self.files.iter().enumerate() {
            let _ = writeln!(out, "file {i} {f}");
        }
        for (f, l, a, n) in &self.lines {
            let _ = writeln!(out, "line {f} {l} {a:#X} {n}");
        }
        for (s, a) in &self.symbols {
            let _ = writeln!(out, "sym {s} {a:#X}");
        }
        out
    }

    pub fn parse(text: &str) -> Result<DebugInfo, String> {
        let mut info = DebugInfo::default();
        let mut seen_header = false;
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let bad = |what: &str| format!("line {}: {what}", n + 1);
            let mut it = line.splitn(3, ' ');
            match it.next() {
                Some("rcdbg") => {
                    if it.next() != Some("1") {
                        return Err(bad("unsupported version"));
                    }
                    seen_header = true;
                }
                Some("file") => {
                    let (Some(i), Some(name)) = (it.next(), it.next()) else {
                        return Err(bad("file needs an index and a name"));
                    };
                    if i.parse::<usize>().ok() != Some(info.files.len()) {
                        return Err(bad("files are numbered 0, 1, 2 ... in order"));
                    }
                    info.files.push(name.to_string());
                }
                Some("line") => {
                    let nums: Vec<u32> = line
                        .split_whitespace()
                        .skip(1)
                        .map(|w| rc_num(w).ok_or_else(|| bad("a number is expected")))
                        .collect::<Result<_, _>>()?;
                    let [f, l, a, len] = nums[..] else {
                        return Err(bad("line needs: file line address length"));
                    };
                    if f as usize >= info.files.len() {
                        return Err(bad("line names a file that was not declared"));
                    }
                    info.lines.push((f, l, a, len));
                }
                Some("sym") => {
                    let (Some(name), Some(v)) = (it.next(), it.next()) else {
                        return Err(bad("sym needs a name and an address"));
                    };
                    let v = rc_num(v.trim()).ok_or_else(|| bad("a number is expected"))?;
                    info.symbols.push((name.to_string(), v));
                }
                Some("base") => {
                    let rest = line["base".len()..].trim();
                    if !rest.starts_with("text ") {
                        return Err(bad("base needs `text` and a method"));
                    }
                    info.base = Some(rest.to_string());
                }
                Some("anchor") => {
                    let hex = line["anchor".len()..].trim();
                    if hex.is_empty()
                        || hex.len() % 2 != 0
                        || !hex.chars().all(|c| c.is_ascii_hexdigit() || c == '?')
                    {
                        return Err(bad("anchor is pairs of hex digits or ??"));
                    }
                    info.anchor = Some(hex.to_string());
                }
                _ => return Err(bad("unknown record")),
            }
        }
        if !seen_header {
            return Err("not an rcdbg file (no `rcdbg 1` line)".to_string());
        }
        Ok(info)
    }

    /// The source line that made the byte at `addr`.
    pub fn line_at(&self, addr: u32) -> Option<(&str, u32)> {
        self.lines
            .iter()
            .find(|(_, _, a, n)| addr >= *a && addr < a.saturating_add((*n).max(1)))
            .map(|(f, l, _, _)| (self.files[*f as usize].as_str(), *l))
    }

    /// The address of the first instruction made by `file:line`.
    pub fn address_of(&self, file: &str, line: u32) -> Option<u32> {
        let idx = self.files.iter().position(|f| f == file)? as u32;
        self.lines
            .iter()
            .find(|(f, l, _, n)| *f == idx && *l == line && *n > 0)
            .map(|(_, _, a, _)| *a)
    }
}

fn rc_num(w: &str) -> Option<u32> {
    match w.strip_prefix("0x").or_else(|| w.strip_prefix("0X")) {
        Some(h) => u32::from_str_radix(h, 16).ok(),
        None => w.parse().ok(),
    }
}
