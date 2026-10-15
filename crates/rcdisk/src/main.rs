use clap::{Parser, Subcommand};
use rc_common::json::Json;
use rc_common::write_file;
use rcdisk::{AddOptions, Disk, FORMATS, ReadOptions};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "rcdisk",
    version,
    about = "Look inside, edit, create and convert disk images",
    long_about = "Look inside, edit, create and convert disk images.\n\n\
        Commodore .d64 (35/40 tracks), .x64, .d71, .d81 are read and written; .g64 is read (its standard \
        sectors) and converts to .d64.\n\n\
        add, rm and rename change IMAGE in place (written atomically); use -o to write a copy instead. \
        Nothing else overwrites an existing file without --force."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the directory.
    List {
        image: PathBuf,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Show facts about the disk: format, name, ID, free blocks.
    Info {
        image: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Check the disk for damage (broken or shared sector chains, a wrong BAM). Exit status 1 on errors.
    Validate {
        image: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Copy files out of the image.
    Extract {
        image: PathBuf,
        /// Names or patterns (`*`, `?`); with none, every file.
        names: Vec<String>,
        /// Directory to write into (created if missing); default: the current directory.
        #[arg(long, short = 'd')]
        dir: Option<PathBuf>,
        /// With a single file: write it to this exact path instead.
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        /// CP/M: keep the AMSDOS / +3DOS header in the extracted file.
        #[arg(long)]
        keep_header: bool,
        #[arg(long)]
        force: bool,
    },
    /// Put host files on the disk.
    Add {
        image: PathBuf,
        files: Vec<PathBuf>,
        /// Name on the disk (single file only); default: the file name without extension.
        #[arg(long)]
        name: Option<String>,
        /// File type. Commodore: prg, seq or usr (default from the extension: .seq/.txt -> seq, .usr -> usr,
        /// else prg). CP/M with --header: bin or bas.
        #[arg(long = "type", short = 't')]
        kind: Option<String>,
        /// CP/M user area 0-15 (or write the name as `3:NAME.EXT`).
        #[arg(long, default_value_t = 0)]
        user: u8,
        /// CP/M: put an `amsdos` or `plus3dos` header in front of the data (`none` stores the file as it is).
        #[arg(long)]
        header: Option<String>,
        /// Load address for a binary with a header (`0x4000`, `$4000`, `16384`).
        #[arg(long, value_parser = parse_u16)]
        load: Option<u16>,
        /// Entry address for an AMSDOS binary (default: the load address).
        #[arg(long, value_parser = parse_u16)]
        entry: Option<u16>,
        /// Replace files of the same name.
        #[arg(long)]
        replace: bool,
        /// Write the changed image here instead of changing IMAGE.
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Delete files (patterns allowed).
    Rm {
        image: PathBuf,
        names: Vec<String>,
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Rename a file.
    Rename {
        image: PathBuf,
        old: String,
        new: String,
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Make a new, blank, formatted image.
    Create {
        output: PathBuf,
        /// One of: d64, d64-40, x64, d71, d81, dsk, dsk-system, dsk-ibm, dsk-plus3 (see `rcdisk formats`;
        /// default: from the output file's extension).
        #[arg(long, short = 'f')]
        format: Option<String>,
        /// Disk name.
        #[arg(long, short = 'n')]
        name: Option<String>,
        /// Two-character disk ID.
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        force: bool,
    },
    /// Convert to another format (by the output's extension, or --to).
    Convert {
        input: PathBuf,
        output: PathBuf,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        force: bool,
    },
    /// List the formats create and convert know.
    Formats,
}

fn parse_u16(text: &str) -> Result<u16, String> {
    rc_common::parse_address(text)
}

fn fail(message: impl std::fmt::Display) -> ExitCode {
    rc_common::err(&format!("rcdisk: {message}\n"));
    ExitCode::from(1)
}

fn open_image(path: &Path) -> Result<Box<dyn Disk>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    rcdisk::open(&path.to_string_lossy(), &bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn format_for(path: &Path, given: &Option<String>) -> Result<String, String> {
    if let Some(f) = given {
        return Ok(f.to_ascii_lowercase());
    }
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
    {
        Some(e) if ["d64", "x64", "d71", "d81", "g64"].contains(&e.as_str()) => Ok(e),
        Some(e) if e == "dsk" || e == "edsk" => Ok("dsk".to_string()),
        _ => Err(format!(
            "cannot tell the format from \"{}\"; name it .d64/.x64/.d71/.d81/.dsk or give --format (see `rcdisk formats`)",
            path.display()
        )),
    }
}

/// Where a changed image goes: back to IMAGE (replacing it atomically) or to a new file.
fn save(
    disk: &dyn Disk,
    image: &Path,
    output: &Option<PathBuf>,
    force: bool,
) -> Result<(), String> {
    let bytes = disk.to_bytes();
    match output {
        Some(out) => write_file(out, &bytes, force),
        None => write_file(image, &bytes, true),
    }
}

fn files_json(disk: &dyn Disk) -> Json {
    Json::Arr(
        disk.files()
            .into_iter()
            .map(|f| {
                Json::obj([
                    ("name", Json::str(f.name)),
                    ("name_hex", Json::str(f.name_hex)),
                    ("type", Json::str(f.kind)),
                    ("blocks", Json::Num(f.blocks as i64)),
                    ("size", f.size.map_or(Json::Null, |s| Json::Num(s as i64))),
                    (
                        "flags",
                        Json::Arr(f.flags.into_iter().map(Json::Str).collect()),
                    ),
                ])
            })
            .collect(),
    )
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => fail(e),
    }
}

fn run(cli: Cli) -> Result<ExitCode, String> {
    match cli.command {
        Command::Formats => {
            for (name, text) in FORMATS {
                rc_common::out(&format!("{name:<8} {text}\n"));
            }
            rc_common::out("g64      read only (converts to d64)\n");
        }
        Command::List { image, json } => {
            let disk = open_image(&image)?;
            if json {
                rc_common::out(
                    &Json::obj([
                        ("format", Json::str(disk.format())),
                        ("name", Json::str(disk.label().0)),
                        ("id", Json::str(disk.label().1)),
                        ("files", files_json(disk.as_ref())),
                        ("free_blocks", Json::Num(disk.free_blocks() as i64)),
                    ])
                    .pretty(),
                );
            } else {
                if disk.format() == "dsk" {
                    // CP/M: name, size, type and what the header says.
                    for f in disk.files() {
                        let size = f.size.map_or(String::new(), |s| format!("{s}"));
                        let flags = if f.flags.is_empty() {
                            String::new()
                        } else {
                            format!(" <{}>", f.flags.join(","))
                        };
                        rc_common::out(&format!("{:<14}{size:>8}  {}{flags}\n", f.name, f.kind));
                    }
                    rc_common::out(&format!("{} KB free.\n", disk.free_blocks()));
                    return Ok(ExitCode::SUCCESS);
                }
                let (name, id) = disk.label();
                rc_common::out(&format!("0 \"{name:<16}\" {id}\n"));
                for f in disk.files() {
                    let quoted = format!("\"{}\"", f.name);
                    let flags = if f.flags.is_empty() {
                        String::new()
                    } else {
                        format!(" <{}>", f.flags.join(","))
                    };
                    rc_common::out(&format!("{:<5}{quoted:<19}{}{flags}\n", f.blocks, f.kind));
                }
                rc_common::out(&format!("{} blocks free.\n", disk.free_blocks()));
            }
        }
        Command::Info { image, json } => {
            let disk = open_image(&image)?;
            let rows = disk.info();
            if json {
                rc_common::out(
                    &Json::Obj(rows.into_iter().map(|(k, v)| (k, Json::Str(v))).collect()).pretty(),
                );
            } else {
                for (k, v) in rows {
                    rc_common::out(&format!("{k:<20}{v}\n"));
                }
            }
        }
        Command::Validate { image, json } => {
            let disk = open_image(&image)?;
            let check = disk.check();
            if json {
                rc_common::out(
                    &Json::obj([
                        ("ok", Json::Bool(check.errors.is_empty())),
                        ("files", Json::Num(check.files as i64)),
                        ("free_blocks", Json::Num(check.free_blocks as i64)),
                        (
                            "errors",
                            Json::Arr(check.errors.iter().cloned().map(Json::Str).collect()),
                        ),
                        (
                            "warnings",
                            Json::Arr(check.warnings.iter().cloned().map(Json::Str).collect()),
                        ),
                    ])
                    .pretty(),
                );
            } else {
                for e in &check.errors {
                    rc_common::out(&format!("error: {e}\n"));
                }
                for w in &check.warnings {
                    rc_common::out(&format!("warning: {w}\n"));
                }
                rc_common::out(&format!(
                    "{}: {} file(s), {} blocks free, {} error(s), {} warning(s)\n",
                    image.display(),
                    check.files,
                    check.free_blocks,
                    check.errors.len(),
                    check.warnings.len()
                ));
            }
            if !check.errors.is_empty() {
                return Ok(ExitCode::from(1));
            }
        }
        Command::Extract {
            image,
            names,
            dir,
            output,
            keep_header,
            force,
        } => {
            let disk = open_image(&image)?;
            let patterns = if names.is_empty() {
                vec!["*".to_string()]
            } else {
                names
            };
            let mut taken = Vec::new();
            for p in &patterns {
                match disk.read(p, &ReadOptions { keep_header }) {
                    Ok(found) => taken.extend(found),
                    Err(e) if names_were_given(&patterns) => return Err(e),
                    Err(_) => {}
                }
            }
            if taken.is_empty() {
                return Err("the disk has no files".to_string());
            }
            if let Some(out) = output {
                if taken.len() != 1 {
                    return Err(format!(
                        "-o takes one file, {} matched; use -d",
                        taken.len()
                    ));
                }
                write_file(&out, &taken[0].data, force)?;
                rc_common::err(&format!(
                    "rcdisk: {} -> {} ({} bytes)\n",
                    taken[0].name,
                    out.display(),
                    taken[0].data.len()
                ));
            } else {
                let dir = dir.unwrap_or_else(|| PathBuf::from("."));
                std::fs::create_dir_all(&dir)
                    .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
                let mut used = std::collections::HashSet::new();
                for f in taken {
                    // Two files may map to one host name; keep both.
                    let mut file_name = f.file_name.clone();
                    let mut n = 2;
                    while !used.insert(file_name.to_ascii_lowercase()) {
                        let (stem, ext) =
                            f.file_name.rsplit_once('.').unwrap_or((&f.file_name, ""));
                        file_name = format!("{stem}-{n}.{ext}");
                        n += 1;
                    }
                    let path = dir.join(&file_name);
                    write_file(&path, &f.data, force)?;
                    rc_common::err(&format!(
                        "rcdisk: {} -> {} ({} bytes)\n",
                        f.name,
                        path.display(),
                        f.data.len()
                    ));
                }
            }
        }
        Command::Add {
            image,
            files,
            name,
            kind,
            user,
            header,
            load,
            entry,
            replace,
            output,
            force,
        } => {
            if files.is_empty() {
                return Err("no files to add".to_string());
            }
            if name.is_some() && files.len() != 1 {
                return Err("--name works with a single file".to_string());
            }
            let mut disk = open_image(&image)?;
            for f in &files {
                let data =
                    std::fs::read(f).map_err(|e| format!("cannot read {}: {e}", f.display()))?;
                let opts = AddOptions {
                    name: name.clone(),
                    kind: kind.clone(),
                    replace,
                    user,
                    header: header.clone(),
                    load,
                    entry,
                };
                let stored = disk.add(&f.to_string_lossy(), &data, &opts)?;
                rc_common::err(&format!(
                    "rcdisk: {} -> \"{stored}\" ({} bytes)\n",
                    f.display(),
                    data.len()
                ));
            }
            save(disk.as_ref(), &image, &output, force)?;
        }
        Command::Rm {
            image,
            names,
            output,
            force,
        } => {
            if names.is_empty() {
                return Err("no names to delete".to_string());
            }
            let mut disk = open_image(&image)?;
            for n in &names {
                for removed in disk.remove(n)? {
                    rc_common::err(&format!("rcdisk: deleted \"{removed}\"\n"));
                }
            }
            save(disk.as_ref(), &image, &output, force)?;
        }
        Command::Rename {
            image,
            old,
            new,
            output,
            force,
        } => {
            let mut disk = open_image(&image)?;
            disk.rename(&old, &new)?;
            save(disk.as_ref(), &image, &output, force)?;
            rc_common::err(&format!("rcdisk: \"{old}\" is now \"{new}\"\n"));
        }
        Command::Create {
            output,
            format,
            name,
            id,
            force,
        } => {
            let format = format_for(&output, &format)?;
            let disk = rcdisk::create(&format, name.as_deref(), id.as_deref())?;
            write_file(&output, &disk.to_bytes(), force)?;
            rc_common::err(&format!(
                "rcdisk: created {} ({})\n",
                output.display(),
                disk.description()
            ));
        }
        Command::Convert {
            input,
            output,
            to,
            force,
        } => {
            let source = open_image(&input)?;
            let format = format_for(&output, &to)?;
            let (disk, notes) = rcdisk::convert(source.as_ref(), &format)?;
            write_file(&output, &disk.to_bytes(), force)?;
            for n in &notes {
                rc_common::err(&format!("rcdisk: note: {n}\n"));
            }
            rc_common::err(&format!(
                "rcdisk: {} ({}) -> {} ({})",
                input.display(),
                source.format(),
                output.display(),
                disk.description()
            ));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn names_were_given(patterns: &[String]) -> bool {
    !(patterns.len() == 1 && patterns[0] == "*")
}
