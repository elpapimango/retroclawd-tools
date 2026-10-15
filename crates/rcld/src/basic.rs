//! The small BASIC programs that start a machine program: `10 SYS <entry>` for the C64 and the
//! `CLEAR` / `LOAD ""CODE` / `RANDOMIZE USR` loader for the Spectrum.

/// The C64 line `10 SYS <entry>` as it sits in memory from 0x0801: link to the next line, the line number, the
/// SYS token, the address in digits, the end of the line and the end of the program.
pub fn c64_stub(entry: u16) -> Vec<u8> {
    let digits = entry.to_string();
    let len = 2 + 2 + 1 + digits.len() + 1 + 2;
    let next = 0x0801u16 + (len as u16 - 2);
    let mut v = next.to_le_bytes().to_vec();
    v.extend_from_slice(&10u16.to_le_bytes());
    v.push(0x9E); // SYS
    v.extend_from_slice(digits.as_bytes());
    v.push(0);
    v.extend_from_slice(&[0, 0]);
    v
}

/// A number in a Spectrum program: its digits, then the hidden five-byte form the interpreter reads
/// (0x0E, 0, 0, low, high, 0 for whole numbers up to 65535).
pub fn spectrum_number(n: u16) -> Vec<u8> {
    let mut v = n.to_string().into_bytes();
    v.extend_from_slice(&[0x0E, 0, 0, n as u8, (n >> 8) as u8, 0]);
    v
}

/// One program line: number (big endian), length (little endian), the body and the newline.
pub fn spectrum_line(number: u16, body: &[u8]) -> Vec<u8> {
    let mut v = number.to_be_bytes().to_vec();
    v.extend_from_slice(&(body.len() as u16 + 1).to_le_bytes());
    v.extend_from_slice(body);
    v.push(0x0D);
    v
}

const CLEAR: u8 = 0xFD;
const LOAD: u8 = 0xEF;
const CODE: u8 = 0xAF;
const SCREEN: u8 = 0xAA;
const RANDOMIZE: u8 = 0xF9;
const USR: u8 = 0xC0;

/// `CLEAR ramtop`, one `LOAD ""SCREEN$` if asked, a `LOAD ""CODE` per code block, `RANDOMIZE USR entry`; the lines
/// are numbered 10, 20, 30...
pub fn spectrum_loader(ramtop: u16, screen: bool, code_blocks: usize, entry: u16) -> Vec<u8> {
    let mut lines: Vec<Vec<u8>> = Vec::new();
    let mut clear = vec![CLEAR];
    clear.extend(spectrum_number(ramtop));
    lines.push(clear);
    if screen {
        lines.push(vec![LOAD, b'"', b'"', SCREEN]);
    }
    for _ in 0..code_blocks {
        lines.push(vec![LOAD, b'"', b'"', CODE]);
    }
    let mut run = vec![RANDOMIZE, USR];
    run.extend(spectrum_number(entry));
    lines.push(run);
    lines
        .iter()
        .enumerate()
        .flat_map(|(i, body)| spectrum_line(10 * (i as u16 + 1), body))
        .collect()
}
