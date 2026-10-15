use clap::{Parser, Subcommand};
use rc_common::json::Json;
use rc_common::write_file;
use rcsnap::carts;
use rcsnap::spectrum::{self, Format, Spec};
use rcsnap::{Image, zx81};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "rcsnap",
    version,
    about = "Look inside, convert and build snapshots and cartridges",
    long_about = "Look inside, convert and build snapshots and cartridges.\n\n\
        ZX Spectrum .z80, .sna and .szx convert into one another; ZX81 .p/.81/.p81, Amstrad CPC .sna, C64 .crt and \
        CPC Plus .cpr can be listed, dumped and (where it makes sense) converted. The kind of a file is decided \
        by its content, then by its extension.\n\n\
        Nothing overwrites an existing file without --force."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show what is in the file: registers, memory, paging; name, chips, pages.
    Info {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Convert to another format (by the output's extension).
    Convert {
        input: PathBuf,
        output: PathBuf,
        /// CPC .sna: the version to write (1, 2 or 3; default: keep).
        #[arg(long)]
        version: Option<u8>,
        /// Name for a .p81 or .crt (default: the output file name without extension).
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        force: bool,
    },
    /// Write the memory of a snapshot as a flat file (to standard output without -o).
    Dump {
        file: PathBuf,
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        /// Only this part of the CPU address space: START:LENGTH, e.g. 0x8000:0x100.
        #[arg(long, value_name = "START:LEN")]
        range: Option<String>,
        /// 128K Spectrum: write this RAM bank (0-7) instead of the CPU's view.
        #[arg(long)]
        bank: Option<u8>,
        #[arg(long)]
        force: bool,
    },
    /// Write the chips of a .crt or the pages of a .cpr as files.
    Extract {
        file: PathBuf,
        #[arg(long, short = 'd')]
        dir: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Make a 48K ZX Spectrum snapshot from programs (.z80, .sna or .szx by the output's extension).
    Create {
        output: PathBuf,
        /// Put a file in memory: ADDRESS:FILE (repeatable), e.g. 0x8000:game.bin.
        #[arg(long = "load", short = 'l', value_name = "ADDR:FILE", required = true)]
        loads: Vec<String>,
        /// Address to start at (default: the first --load address).
        #[arg(long, value_parser = parse_u16)]
        pc: Option<u16>,
        /// Stack pointer (default 0, the top of memory).
        #[arg(long, value_parser = parse_u16)]
        sp: Option<u16>,
        /// Border colour 0-7 (default 7).
        #[arg(long, default_value_t = 7)]
        border: u8,
        /// Interrupt mode 0-2 (default 1).
        #[arg(long, default_value_t = 1)]
        im: u8,
        /// Start with interrupts enabled (the default is disabled).
        #[arg(long)]
        ei: bool,
        #[arg(long)]
        force: bool,
    },
}

fn parse_u16(text: &str) -> Result<u16, String> {
    rc_common::parse_address(text)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            rc_common::err(&format!("rcsnap: {message}\n"));
            ExitCode::from(2)
        }
    }
}

fn read(path: &Path) -> Result<(Image, usize), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let n = bytes.len();
    Ok((rcsnap::open(&path.to_string_lossy(), &bytes)?, n))
}

fn ext(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn note(text: &str) {
    rc_common::err(&format!("rcsnap: note: {text}\n"));
}

fn run() -> Result<(), String> {
    match Cli::parse().command {
        Command::Info { file, json } => {
            let (image, len) = read(&file)?;
            let info = rcsnap::info(&image, len);
            if json {
                rc_common::out(
                    &Json::Obj(
                        info.fields
                            .iter()
                            .map(|(k, v)| (k.clone(), Json::str(v.clone())))
                            .chain(std::iter::once((
                                "details".to_string(),
                                Json::Arr(info.items.iter().cloned().map(Json::Str).collect()),
                            )))
                            .collect(),
                    )
                    .pretty(),
                );
            } else {
                let mut text = String::new();
                for (k, v) in &info.fields {
                    text.push_str(&format!("{:<22}{v}\n", format!("{k}:")));
                }
                for item in &info.items {
                    text.push_str(&format!("  {item}\n"));
                }
                rc_common::out(&text);
            }
        }
        Command::Convert {
            input,
            output,
            version,
            name,
            force,
        } => convert(&input, &output, version, name, force)?,
        Command::Dump {
            file,
            output,
            range,
            bank,
            force,
        } => {
            let (image, _) = read(&file)?;
            let range = range.as_deref().map(parse_range).transpose()?;
            let bytes = match (&image, bank) {
                (Image::Spectrum { spec, .. }, Some(b)) => match &spec.ram {
                    spectrum::Ram::Banks(banks) if b < 8 => banks[b as usize].clone(),
                    spectrum::Ram::Banks(_) => return Err("--bank takes 0-7".to_string()),
                    spectrum::Ram::K48(_) => {
                        return Err("--bank works on 128K snapshots; this one is 48K".to_string());
                    }
                },
                (_, Some(_)) => {
                    return Err("--bank works on 128K Spectrum snapshots only".to_string());
                }
                (Image::Spectrum { spec, .. }, None) => match range {
                    Some((start, len)) => spec.read(start, len)?,
                    None => spec.address_space(),
                },
                (Image::CpcSna(s), None) => slice_of(&s.ram, 0, range)?,
                (Image::Zx81 { content, .. }, None) => slice_of(&content.data, 0x4009, range)?,
                _ => {
                    return Err(
                        "only snapshots have memory to dump; use extract for cartridges"
                            .to_string(),
                    );
                }
            };
            match output {
                Some(path) => write_file(&path, &bytes, force)?,
                None => rc_common::out_bytes(&bytes),
            }
        }
        Command::Extract { file, dir, force } => {
            let (image, _) = read(&file)?;
            let files: Vec<(String, Vec<u8>)> = match image {
                Image::Crt(c) => c
                    .chips
                    .iter()
                    .enumerate()
                    .map(|(n, ch)| {
                        (
                            format!(
                                "chip{:02}-bank{}-{:04X}.bin",
                                n + 1,
                                ch.bank,
                                ch.load_address
                            ),
                            ch.data.clone(),
                        )
                    })
                    .collect(),
                Image::Cpr(c) => c
                    .pages
                    .iter()
                    .map(|p| (format!("page{:02}.bin", p.bank), p.data.clone()))
                    .collect(),
                _ => {
                    return Err(
                        "extract works on cartridges (.crt, .cpr); use dump for snapshots"
                            .to_string(),
                    );
                }
            };
            let dir = dir.unwrap_or_else(|| PathBuf::from("."));
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            for (name, bytes) in &files {
                write_file(&dir.join(name), bytes, force)?;
                rc_common::err(&format!("rcsnap: {name} ({} bytes)\n", bytes.len()));
            }
        }
        Command::Create {
            output,
            loads,
            pc,
            sp,
            border,
            im,
            ei,
            force,
        } => {
            let format = Format::from_extension(&output.to_string_lossy())
                .ok_or("the output must end in .z80, .sna or .szx")?;
            if border > 7 || im > 2 {
                return Err("--border takes 0-7 and --im 0-2".to_string());
            }
            let mut first = None;
            let mut spec = Spec::blank48(0);
            for l in &loads {
                let (addr, file) = l
                    .split_once(':')
                    .ok_or_else(|| format!("--load wants ADDRESS:FILE, not \"{l}\""))?;
                let addr = rc_common::parse_address(addr)?;
                let data = std::fs::read(file).map_err(|e| format!("cannot read {file}: {e}"))?;
                spec.poke(addr, &data)?;
                first.get_or_insert(addr);
                rc_common::err(&format!(
                    "rcsnap: {file} -> {addr:#06X}..{:#06X}\n",
                    addr as usize + data.len().max(1) - 1
                ));
            }
            spec.cpu.pc = pc.or(first).expect("at least one --load");
            spec.cpu.sp = sp.unwrap_or(0);
            spec.cpu.im = im;
            spec.cpu.iff1 = ei;
            spec.cpu.iff2 = ei;
            spec.border = border;
            let bytes = spectrum::write(&spec, format)?;
            write_file(&output, &bytes, force)?;
        }
    }
    Ok(())
}

fn parse_range(text: &str) -> Result<(u16, usize), String> {
    let (a, l) = text.split_once(':').ok_or("--range wants START:LENGTH")?;
    let start = rc_common::parse_address(a)?;
    let len = rc_common::parse_number(l)?;
    if len <= 0 {
        return Err("the length must be positive".to_string());
    }
    Ok((start, len as usize))
}

/// `data` starts at CPU address `base`; the range (if any) is in CPU addresses.
fn slice_of(data: &[u8], base: usize, range: Option<(u16, usize)>) -> Result<Vec<u8>, String> {
    let Some((start, len)) = range else {
        return Ok(data.to_vec());
    };
    let from = (start as usize)
        .checked_sub(base)
        .ok_or_else(|| format!("{start:#06X} is below the start of the data ({base:#06X})"))?;
    data.get(from..from + len)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| format!("{len} bytes from {start:#06X} run past the end of the data"))
}

fn convert(
    input: &Path,
    output: &Path,
    version: Option<u8>,
    name: Option<String>,
    force: bool,
) -> Result<(), String> {
    let out_ext = ext(output);
    let default_name = || name.clone().unwrap_or_else(|| stem(output));
    // A raw ROM image is the one input that has no signature.
    if matches!(ext(input).as_str(), "bin" | "rom") {
        let raw =
            std::fs::read(input).map_err(|e| format!("cannot read {}: {e}", input.display()))?;
        let bytes = match out_ext.as_str() {
            "crt" => c64_cartridge::write(&carts::crt_from_raw(&default_name(), &raw)?),
            "cpr" => {
                let (cart, n) = carts::cpr_from_raw(&raw)?;
                if let Some(n) = n {
                    note(&n);
                }
                cpc_cartridge::write(&cart).map_err(|e| e.to_string())?
            }
            _ => return Err("a raw image converts to .crt (8K or 16K) or .cpr".to_string()),
        };
        return write_file(output, &bytes, force);
    }
    let (image, _) = read(input)?;
    let bytes = match image {
        Image::Spectrum { spec, .. } => {
            let to = Format::from_extension(&output.to_string_lossy())
                .ok_or("a Spectrum snapshot converts to .z80, .sna or .szx")?;
            for n in spectrum::losses(&spec, to)? {
                note(&n);
            }
            spectrum::write(&spec, to)?
        }
        Image::Zx81 { content, .. } => match out_ext.as_str() {
            "p81" => zx81::write_p81(&default_name(), &content.data)?,
            "p" | "81" => content.data,
            _ => return Err("a ZX81 program converts to .p, .81 or .p81".to_string()),
        },
        Image::CpcSna(mut s) => {
            if out_ext != "sna" {
                return Err("a CPC snapshot converts to .sna (--version 1, 2 or 3)".to_string());
            }
            if let Some(v) = version {
                if !(1..=3).contains(&v) {
                    return Err("--version takes 1, 2 or 3".to_string());
                }
                if v < s.version {
                    note(&format!(
                        "version {v} does not record the fields version {} added",
                        s.version
                    ));
                }
                if v > s.version {
                    note(&format!(
                        "version {v} fields not present in the source are written as zeros"
                    ));
                }
                s.version = v;
            }
            cpc_snapshot::write(&s).map_err(|e| e.to_string())?
        }
        Image::Crt(c) => match out_ext.as_str() {
            "bin" | "rom" => carts::raw_from_crt(&c)?,
            "crt" => c64_cartridge::write(&c),
            _ => {
                return Err("a .crt converts to .bin / .rom (plain cartridges) or .crt".to_string());
            }
        },
        Image::Cpr(c) => match out_ext.as_str() {
            "bin" | "rom" => carts::raw_from_cpr(&c)?,
            "cpr" => cpc_cartridge::write(&c).map_err(|e| e.to_string())?,
            _ => return Err("a .cpr converts to .bin / .rom or .cpr".to_string()),
        },
    };
    write_file(output, &bytes, force)
}
