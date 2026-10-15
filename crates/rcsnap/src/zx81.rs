//! ZX81 program files: `.p` and `.81` (RAM from 0x4009 on) and `.p81` (the same behind a name).

pub struct P81 {
    /// Decoded name of a `.p81`.
    pub name: Option<String>,
    /// The bytes from 0x4009 on.
    pub data: Vec<u8>,
}

/// The ZX81 character set as ASCII: 0 space, 0x0B `"`, 0x0C `£`, 0x0D `$`, 0x0E `:`, 0x0F `?`, 0x10-0x1B
/// `( ) > < = + - * / ; , .`, 0x1C-0x25 `0`-`9`, 0x26-0x3F `A`-`Z`. Inverse video (bit 7) is ignored; anything
/// else (graphics, keywords) is `?`.
pub fn decode_char(code: u8) -> char {
    match code & 0x7F {
        0 => ' ',
        0x0B => '"',
        0x0C => '£',
        0x0D => '$',
        0x0E => ':',
        0x0F => '?',
        0x10 => '(',
        0x11 => ')',
        0x12 => '>',
        0x13 => '<',
        0x14 => '=',
        0x15 => '+',
        0x16 => '-',
        0x17 => '*',
        0x18 => '/',
        0x19 => ';',
        0x1A => ',',
        0x1B => '.',
        c @ 0x1C..=0x25 => (b'0' + (c - 0x1C)) as char,
        c @ 0x26..=0x3F => (b'A' + (c - 0x26)) as char,
        _ => '?',
    }
}

pub fn encode_char(c: char) -> Option<u8> {
    let c = c.to_ascii_uppercase();
    Some(match c {
        ' ' => 0,
        '"' => 0x0B,
        '£' => 0x0C,
        '$' => 0x0D,
        ':' => 0x0E,
        '?' => 0x0F,
        '(' => 0x10,
        ')' => 0x11,
        '>' => 0x12,
        '<' => 0x13,
        '=' => 0x14,
        '+' => 0x15,
        '-' => 0x16,
        '*' => 0x17,
        '/' => 0x18,
        ';' => 0x19,
        ',' => 0x1A,
        '.' => 0x1B,
        '0'..='9' => 0x1C + (c as u8 - b'0'),
        'A'..='Z' => 0x26 + (c as u8 - b'A'),
        _ => return None,
    })
}

pub fn parse(bytes: &[u8], is_p81: bool) -> Result<P81, String> {
    if !is_p81 {
        return Ok(P81 {
            name: None,
            data: bytes.to_vec(),
        });
    }
    let s = zx81_snapshot::parse_p81(bytes).map_err(|e| e.to_string())?;
    let name = s
        .program_name_raw
        .map(|raw| raw.iter().map(|&b| decode_char(b)).collect::<String>());
    Ok(P81 { name, data: s.data })
}

/// A `.p81`: the name (characters of the ZX81 set, the last with bit 7 set), then the data.
pub fn write_p81(name: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    if name.is_empty() || name.chars().count() > 127 {
        return Err("a .p81 needs a name of 1 to 127 characters".to_string());
    }
    let mut out = Vec::new();
    for c in name.chars() {
        out.push(encode_char(c).ok_or_else(|| {
            format!("\"{c}\" is not in the ZX81 character set (letters, digits, space and . , ; : ? \" $ £ ( ) < > = + - * /)")
        })?);
    }
    *out.last_mut().expect("not empty") |= 0x80;
    out.extend_from_slice(data);
    Ok(out)
}

/// Facts from the system variables at the start of the data (address 0x4009 on): VERSN, D_FILE (0x400C),
/// VARS (0x4010), E_LINE (0x4014) and the BASIC lines between 0x407D and D_FILE.
pub struct Facts {
    pub d_file: u16,
    pub vars: u16,
    pub e_line: u16,
    pub lines: usize,
    pub first_line: Option<u16>,
    pub last_line: Option<u16>,
}

pub fn facts(data: &[u8]) -> Option<Facts> {
    let word = |at: usize| Some(u16::from_le_bytes([*data.get(at)?, *data.get(at + 1)?]));
    let (d_file, vars, e_line) = (word(3)?, word(7)?, word(11)?);
    let mut lines = 0;
    let (mut first, mut last) = (None, None);
    let mut at = 0x407Dusize - 0x4009;
    let end = (d_file as usize).saturating_sub(0x4009).min(data.len());
    while at + 4 <= end {
        let number = u16::from_be_bytes([data[at], data[at + 1]]);
        let len = u16::from_le_bytes([data[at + 2], data[at + 3]]) as usize;
        first.get_or_insert(number);
        last = Some(number);
        lines += 1;
        at += 4 + len;
    }
    Some(Facts {
        d_file,
        vars,
        e_line,
        lines,
        first_line: first,
        last_line: last,
    })
}
