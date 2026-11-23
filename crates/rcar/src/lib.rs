//! `rcar`: object libraries (`.rca`). The operations the command line is made of.

use rc_common::obj::{Archive, Object};
use std::path::Path;

/// A member's name as stored: the file name without its directory.
pub fn member_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{}: not a file name", path.display()))
}

/// Reads an object file, checking that it is one.
pub fn read_object(path: &Path) -> Result<(String, Vec<u8>, Object), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let object = Object::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((member_name(path)?, bytes, object))
}

pub fn read_archive(path: &Path) -> Result<Archive, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Archive::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Adds the objects in `files` to `archive`, replacing members of the same name.
pub fn add_files(archive: &mut Archive, files: &[std::path::PathBuf]) -> Result<(), String> {
    for f in files {
        let (name, bytes, _) = read_object(f)?;
        archive.add_bytes(&name, bytes);
    }
    Ok(())
}

/// One line per member (name, size, sections, exports, imports); with `symbols`, one line per exported name too.
pub fn list(archive: &Archive, symbols: bool) -> Result<String, String> {
    let mut out = String::new();
    for (i, name) in archive.names().enumerate() {
        let o = archive.member(i)?;
        let size = archive.member_bytes(i).map_or(0, <[u8]>::len);
        let code: u32 = o.sections.iter().map(|s| s.size).sum();
        out.push_str(&format!(
            "{name}  {size} bytes  {} section(s)  {code} bytes of code and data  cpu {}\n",
            o.sections.len(),
            o.cpu
        ));
        if symbols {
            let exports: Vec<&str> = o.exports().collect();
            let imports: Vec<&str> = o.imports().collect();
            out.push_str(&format!("    exports: {}\n", exports.join(" ")));
            out.push_str(&format!("    imports: {}\n", imports.join(" ")));
        }
    }
    Ok(out)
}
