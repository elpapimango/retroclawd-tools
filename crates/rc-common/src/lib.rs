//! Pieces every retroclawd tool shares: how numbers are written on a command line, how a problem in a
//! source file is reported, and the "never overwrite without `--force`" rule for output files.

pub mod json;
pub mod prg;

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// A problem tied to a place in a source file. `Display` is `file:line: message`, the shape editors
/// and terminals make clickable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diag {
    pub file: String,
    /// 1-based; 0 when the problem is not about a line.
    pub line: usize,
    pub message: String,
}

impl Diag {
    pub fn new(file: impl Into<String>, line: usize, message: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            line,
            message: message.into(),
        }
    }
}

impl fmt::Display for Diag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}: {}", self.file, self.message)
        } else {
            write!(f, "{}:{}: {}", self.file, self.line, self.message)
        }
    }
}

impl std::error::Error for Diag {}

/// Parses a number as people write addresses: `$C000`, `0xC000`, `C000h`, `%1010`, `0b1010` or decimal.
pub fn parse_number(text: &str) -> Result<i64, String> {
    let t = text.trim();
    let (digits, radix) = if let Some(h) = t.strip_prefix('$') {
        (h, 16)
    } else if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        (h, 16)
    } else if let Some(b) = t.strip_prefix('%') {
        (b, 2)
    } else if let Some(b) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        (b, 2)
    } else if t.len() > 1
        && t.ends_with(['h', 'H'])
        && t.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        (&t[..t.len() - 1], 16)
    } else {
        (t, 10)
    };
    if digits.is_empty() {
        return Err(format!("\"{text}\" is not a number"));
    }
    i64::from_str_radix(&digits.replace('_', ""), radix)
        .map_err(|_| format!("\"{text}\" is not a number"))
}

/// Like [`parse_number`] but the value must be a 16-bit address.
pub fn parse_address(text: &str) -> Result<u16, String> {
    let value = parse_number(text)?;
    u16::try_from(value).map_err(|_| format!("\"{text}\" does not fit in 16 bits"))
}

/// Prints to standard output; a closed pipe (`rcdisk list | head`) ends the program quietly instead of
/// panicking like `println!`.
pub fn out(text: &str) {
    use std::io::Write as _;
    let mut stdout = io::stdout().lock();
    if let Err(e) = stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
        && e.kind() == io::ErrorKind::BrokenPipe
    {
        std::process::exit(0);
    }
}

/// Writes raw bytes to standard output, with the same closed-pipe rule as [`out`].
pub fn out_bytes(bytes: &[u8]) {
    use std::io::Write as _;
    let mut stdout = io::stdout().lock();
    if let Err(e) = stdout.write_all(bytes).and_then(|()| stdout.flush())
        && e.kind() == io::ErrorKind::BrokenPipe
    {
        std::process::exit(0);
    }
}

/// Prints to standard error (messages and progress); never panics.
pub fn err(text: &str) {
    use std::io::Write as _;
    let _ = io::stderr().lock().write_all(text.as_bytes());
}

/// Writes `bytes` to `path`. An existing file is an error unless `force`; the file is written to a
/// sibling temporary name first so a failure never leaves half a file behind.
pub fn write_file(path: &Path, bytes: &[u8], force: bool) -> Result<(), String> {
    if !force && path.exists() {
        return Err(format!(
            "{} exists; use --force to overwrite it",
            path.display()
        ));
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".rc-tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let write = || -> io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("cannot write {}: {e}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_in_every_notation() {
        assert_eq!(parse_number("$C000"), Ok(0xC000));
        assert_eq!(parse_number("0xc000"), Ok(0xC000));
        assert_eq!(parse_number("0C000h"), Ok(0xC000));
        assert_eq!(parse_number("%1010"), Ok(10));
        assert_eq!(parse_number("0b1010"), Ok(10));
        assert_eq!(parse_number("49152"), Ok(49152));
        assert_eq!(parse_number("1_000"), Ok(1000));
        assert!(parse_number("nope").is_err());
        assert!(parse_number("$").is_err());
        assert_eq!(
            parse_address("0x10000").unwrap_err(),
            "\"0x10000\" does not fit in 16 bits"
        );
    }

    #[test]
    fn an_existing_file_needs_force() {
        let dir = std::env::temp_dir().join(format!("rc-common-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.bin");
        write_file(&path, b"one", false).unwrap();
        assert!(
            write_file(&path, b"two", false)
                .unwrap_err()
                .contains("--force")
        );
        assert_eq!(fs::read(&path).unwrap(), b"one");
        write_file(&path, b"two", true).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        fs::remove_dir_all(&dir).unwrap();
    }
}
