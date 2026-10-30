use chd::{Codec, Image};
use clap::{Args, Parser, Subcommand};
use rcchd::createcd::{self, CreateCd};
use rcchd::extractcd::{self, ExtractCd};
use rcchd::hdgeom::{self, Chs};
use rcchd::rawio::{self, Copy, CreateData, ExtractData, Media};
use rcchd::report;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "rcchd",
    version,
    about = "Make, unpack, inspect, check and edit CHD files (compressed hunks of data)",
    long_about = "Make, unpack, inspect, check and edit CHD files, the work of MAME's chdman.\n\n\
        The command names and options are chdman's (`-ip`, `-hs`, `-np` and the other short options of more than \
        one letter are accepted as chdman spells them). Nothing overwrites an existing file without --force. \
        Codecs: zlib, zstd, lzma, huff, flac for data; cdzl, cdzs, cdlz, cdfl for CD images; `none` for no compression.\n\n\
        Not done: laser disc images (createld, extractld: they need video input), CHD versions 1 to 4, GD-ROM \
        (.gdi) images, subchannel data."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// The options most commands that write a CHD share.
#[derive(Args)]
struct Out {
    #[arg(short, long)]
    output: PathBuf,
    #[arg(long)]
    outputparent: Option<PathBuf>,
    /// Up to four codecs, comma separated, or `none`.
    #[arg(short, long)]
    compression: Option<String>,
    /// Bytes per hunk.
    #[arg(long)]
    hunksize: Option<u32>,
    #[arg(short, long)]
    force: bool,
    #[arg(long, default_value_t = 0)]
    numprocessors: usize,
}

/// The part of an input file (or CHD) to use.
#[derive(Args)]
struct Slice {
    #[arg(long)]
    inputstartbyte: Option<u64>,
    #[arg(long)]
    inputstarthunk: Option<u64>,
    #[arg(long)]
    inputbytes: Option<u64>,
    #[arg(long)]
    inputhunks: Option<u64>,
}

#[derive(Subcommand)]
enum Command {
    /// Show a CHD's header, codecs, hashes and metadata.
    Info {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        verbose: bool,
    },
    /// Read every hunk and check the hashes the file holds.
    Verify {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(long)]
        inputparent: Option<PathBuf>,
        /// Write the right hashes into the file if they are wrong.
        #[arg(long)]
        fix: bool,
    },
    /// Make a CHD from a CD image (a .cue sheet with its files, or an .iso).
    Createcd {
        #[arg(short, long)]
        input: PathBuf,
        #[command(flatten)]
        out: Out,
    },
    /// Make a CHD from a DVD image.
    Createdvd {
        #[arg(short, long)]
        input: PathBuf,
        #[command(flatten)]
        slice: Slice,
        #[command(flatten)]
        out: Out,
    },
    /// Make a CHD from a hard disk image, or a blank one (--size, --chs or --template).
    Createhd {
        #[arg(short, long)]
        input: Option<PathBuf>,
        #[command(flatten)]
        slice: Slice,
        #[command(flatten)]
        out: Out,
        /// Bytes per sector (default 512).
        #[arg(long)]
        sectorsize: Option<u32>,
        /// Size of a blank disk in bytes.
        #[arg(short, long)]
        size: Option<u64>,
        /// Geometry: cylinders,heads,sectors.
        #[arg(long)]
        chs: Option<String>,
        /// A drive from `listtemplates`.
        #[arg(long)]
        template: Option<usize>,
    },
    /// Make a CHD from any file, cut into units.
    Createraw {
        #[arg(short, long)]
        input: PathBuf,
        #[command(flatten)]
        slice: Slice,
        #[command(flatten)]
        out: Out,
        /// Bytes per unit (required unless a parent gives it).
        #[arg(long)]
        unitsize: Option<u32>,
    },
    /// Write a CD CHD back as a .cue sheet and a .bin file.
    Extractcd {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(long)]
        inputparent: Option<PathBuf>,
        /// The cue sheet.
        #[arg(short, long)]
        output: PathBuf,
        /// The bin file (default: the sheet's name with .bin).
        #[arg(long)]
        outputbin: Option<PathBuf>,
        /// One bin file a track.
        #[arg(long)]
        splitbin: bool,
        #[arg(short, long)]
        force: bool,
    },
    /// Write a DVD CHD back as an image.
    Extractdvd(Extract),
    /// Write a hard disk CHD back as an image.
    Extracthd(Extract),
    /// Write the data of any CHD back as a file.
    Extractraw(Extract),
    /// Make a copy with other codecs, hunk size or parent (the metadata comes along).
    Copy {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(long)]
        inputparent: Option<PathBuf>,
        #[command(flatten)]
        out: Out,
    },
    /// Add or replace a metadata entry (in place).
    Addmeta {
        #[arg(short, long)]
        input: PathBuf,
        /// Four characters or eight hex digits.
        #[arg(short, long)]
        tag: String,
        #[arg(long, default_value_t = 0)]
        index: u32,
        /// The text (stored with a terminating zero).
        #[arg(long)]
        valuetext: Option<String>,
        /// A file whose bytes are the value.
        #[arg(long)]
        valuefile: Option<PathBuf>,
        /// Leave the entry out of the file's overall SHA-1.
        #[arg(long)]
        nochecksum: bool,
    },
    /// Remove a metadata entry (in place).
    Delmeta {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        tag: String,
        #[arg(long, default_value_t = 0)]
        index: u32,
    },
    /// Print or save the bytes of a metadata entry.
    Dumpmeta {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        tag: String,
        #[arg(long, default_value_t = 0)]
        index: u32,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(short, long)]
        force: bool,
    },
    /// List the hard disk templates `createhd --template` takes.
    Listtemplates,
    /// Laser disc images (not available: they need video input).
    Createld {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        rest: Vec<String>,
    },
    /// Laser disc images (not available).
    Extractld {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        rest: Vec<String>,
    },
}

#[derive(Args)]
struct Extract {
    #[arg(short, long)]
    input: PathBuf,
    #[arg(long)]
    inputparent: Option<PathBuf>,
    #[command(flatten)]
    slice: Slice,
    #[arg(short, long)]
    output: PathBuf,
    #[arg(short, long)]
    force: bool,
}

/// chdman's options of several letters with one dash, as clap's long options.
fn normalize(args: impl Iterator<Item = String>) -> Vec<String> {
    const MAP: &[(&str, &str)] = &[
        ("-ip", "--inputparent"),
        ("-op", "--outputparent"),
        ("-hs", "--hunksize"),
        ("-np", "--numprocessors"),
        ("-ob", "--outputbin"),
        ("-sb", "--splitbin"),
        ("-us", "--unitsize"),
        ("-ss", "--sectorsize"),
        ("-isb", "--inputstartbyte"),
        ("-ish", "--inputstarthunk"),
        ("-ib", "--inputbytes"),
        ("-ih", "--inputhunks"),
        ("-tp", "--template"),
        ("-ix", "--index"),
        ("-vt", "--valuetext"),
        ("-vf", "--valuefile"),
        ("-nocs", "--nochecksum"),
        ("-chs", "--chs"),
    ];
    args.map(|a| {
        MAP.iter()
            .find(|(short, _)| *short == a)
            .map_or(a.clone(), |(_, long)| (*long).to_string())
    })
    .collect()
}

fn parse_codecs(text: &str) -> Result<Vec<Codec>, rcchd::Error> {
    if text.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }
    text.split(',')
        .map(|t| {
            let tag: [u8; 4] = t
                .trim()
                .as_bytes()
                .try_into()
                .map_err(|_| rcchd::Error(format!("{t:?} is not a codec name")))?;
            Codec::from_tag(tag).ok_or_else(|| {
                rcchd::Error(format!(
                    "unknown codec {t:?} (zlib, zstd, lzma, huff, flac, cdzl, cdzs, cdlz, cdfl)"
                ))
            })
        })
        .collect()
}

fn percent(label: &'static str) -> impl FnMut(u64, u64) {
    let mut last = 1000u64;
    let show = std::io::stderr().is_terminal();
    move |done, total| {
        if !show {
            return;
        }
        let p = (done * 1000).checked_div(total).unwrap_or(1000);
        if p != last && (p % 10 == 0 || p == 1000) {
            last = p;
            eprint!("\r{label}, {:.1}% complete...", p as f64 / 10.0);
            let _ = std::io::stderr().flush();
        }
    }
}

fn done_line(output: &std::path::Path, s: &chd::Summary) {
    eprintln!();
    println!(
        "{}: {} hunks, {} bytes{}",
        output.display(),
        s.hunks,
        report::grouped(s.file_bytes),
        if s.sha1 == [0; 20] {
            String::new()
        } else {
            format!(" (SHA1 {})", report::hex(&s.sha1))
        }
    );
}

fn create_data(
    media: Media,
    input: Option<PathBuf>,
    slice: &Slice,
    out: &Out,
) -> Result<CreateData, rcchd::Error> {
    let mut o = CreateData::new(media, out.output.clone());
    o.input = input;
    o.parent = out.outputparent.clone();
    o.codecs = out.compression.as_deref().map(parse_codecs).transpose()?;
    o.hunk_bytes = out.hunksize;
    o.start_byte = slice.inputstartbyte;
    o.start_hunk = slice.inputstarthunk;
    o.bytes = slice.inputbytes;
    o.hunks = slice.inputhunks;
    o.force = out.force;
    o.threads = out.numprocessors;
    Ok(o)
}

fn extract_data(media: Media, e: Extract) -> Result<ExitCode, rcchd::Error> {
    let o = ExtractData {
        media,
        input: e.input,
        parent: e.inputparent,
        output: e.output,
        start_byte: e.slice.inputstartbyte,
        start_hunk: e.slice.inputstarthunk,
        bytes: e.slice.inputbytes,
        hunks: e.slice.inputhunks,
        force: e.force,
    };
    rawio::extract(&o, percent("Extracting"))?;
    eprintln!();
    Ok(ExitCode::SUCCESS)
}

fn run(cli: Cli) -> Result<ExitCode, rcchd::Error> {
    match cli.command {
        Command::Info { input, verbose } => {
            let image = match Image::open(&input, None) {
                Ok(i) => i,
                Err(chd::ChdError::Unsupported(m)) if m.contains("parent") => {
                    return Err(rcchd::Error(
                        "this file is a delta of a parent: name the parent (verify --inputparent) to read it".into(),
                    ));
                }
                Err(e) => return Err(e.into()),
            };
            print!(
                "{}",
                report::info(&image, &input.display().to_string(), verbose)
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Verify {
            input,
            inputparent,
            fix,
        } => {
            let mut image = Image::open(&input, inputparent.as_deref())?;
            let mut p = percent("Verifying");
            let r = image.verify(|d, t| p(u64::from(d), u64::from(t)))?;
            eprintln!();
            if !r.hashes_recorded {
                println!(
                    "No verification to be done; the file is not compressed and holds no hashes."
                );
                return Ok(ExitCode::SUCCESS);
            }
            println!(
                "Raw SHA1 {}: {}",
                if r.raw_ok {
                    "verification successful"
                } else {
                    "MISMATCH"
                },
                report::hex(&r.raw_sha1)
            );
            println!(
                "Overall SHA1 {}: {}",
                if r.overall_ok {
                    "verification successful"
                } else {
                    "MISMATCH"
                },
                report::hex(&r.overall_sha1)
            );
            if r.ok() {
                return Ok(ExitCode::SUCCESS);
            }
            if fix {
                drop(image);
                chd::write_hashes(&input, &r.raw_sha1, &r.overall_sha1)?;
                println!("The hashes in the file were replaced with the computed ones.");
                return Ok(ExitCode::SUCCESS);
            }
            Ok(ExitCode::from(1))
        }
        Command::Createcd { input, out } => {
            let mut o = CreateCd::new(input, out.output.clone());
            if let Some(c) = &out.compression {
                o.codecs = parse_codecs(c)?;
            }
            if let Some(h) = out.hunksize {
                o.hunk_bytes = h;
            }
            o.parent = out.outputparent.clone();
            o.force = out.force;
            o.threads = out.numprocessors;
            let s = createcd::create(&o, percent("Compressing"))?;
            done_line(&out.output, &s);
            Ok(ExitCode::SUCCESS)
        }
        Command::Createdvd { input, slice, out } => {
            let o = create_data(Media::Dvd, Some(input), &slice, &out)?;
            let s = rawio::create(&o, percent("Compressing"))?;
            done_line(&out.output, &s);
            Ok(ExitCode::SUCCESS)
        }
        Command::Createraw {
            input,
            slice,
            out,
            unitsize,
        } => {
            let mut o = create_data(Media::Raw, Some(input), &slice, &out)?;
            o.unit_bytes = unitsize;
            let s = rawio::create(&o, percent("Compressing"))?;
            done_line(&out.output, &s);
            Ok(ExitCode::SUCCESS)
        }
        Command::Createhd {
            input,
            slice,
            out,
            sectorsize,
            size,
            chs,
            template,
        } => {
            let mut o = create_data(Media::Hd, input, &slice, &out)?;
            o.unit_bytes = sectorsize;
            o.size = size;
            o.chs = chs.as_deref().map(Chs::from_arg).transpose()?;
            o.template = template;
            let s = rawio::create(&o, percent("Compressing"))?;
            done_line(&out.output, &s);
            Ok(ExitCode::SUCCESS)
        }
        Command::Extractcd {
            input,
            inputparent,
            output,
            outputbin,
            splitbin,
            force,
        } => {
            let o = ExtractCd {
                input,
                parent: inputparent,
                output,
                output_bin: outputbin,
                split: splitbin,
                force,
            };
            extractcd::extract(&o, percent("Extracting"))?;
            eprintln!();
            Ok(ExitCode::SUCCESS)
        }
        Command::Extractdvd(e) => extract_data(Media::Dvd, e),
        Command::Extracthd(e) => extract_data(Media::Hd, e),
        Command::Extractraw(e) => extract_data(Media::Raw, e),
        Command::Copy {
            input,
            inputparent,
            out,
        } => {
            let o = Copy {
                input,
                input_parent: inputparent,
                output: out.output.clone(),
                output_parent: out.outputparent.clone(),
                codecs: out.compression.as_deref().map(parse_codecs).transpose()?,
                hunk_bytes: out.hunksize,
                force: out.force,
                threads: out.numprocessors,
            };
            let s = rawio::copy(&o, percent("Compressing"))?;
            done_line(&out.output, &s);
            Ok(ExitCode::SUCCESS)
        }
        Command::Addmeta {
            input,
            tag,
            index,
            valuetext,
            valuefile,
            nochecksum,
        } => {
            let tag = rawio::parse_tag(&tag)?;
            let data = match (valuetext, valuefile) {
                (Some(t), None) => {
                    let mut d = t.into_bytes();
                    d.push(0);
                    d
                }
                (None, Some(f)) => std::fs::read(&f)
                    .map_err(|e| rcchd::Error(format!("{}: {e}", f.display())))?,
                _ => {
                    return Err(rcchd::Error(
                        "give the value as --valuetext or --valuefile (one of them)".into(),
                    ));
                }
            };
            chd::set_metadata(&input, tag, index, &data, !nochecksum)?;
            println!("Metadata added");
            Ok(ExitCode::SUCCESS)
        }
        Command::Delmeta { input, tag, index } => {
            chd::delete_metadata(&input, rawio::parse_tag(&tag)?, index)?;
            println!("Metadata deleted");
            Ok(ExitCode::SUCCESS)
        }
        Command::Dumpmeta {
            input,
            tag,
            index,
            output,
            force,
        } => {
            let data = rawio::dump_metadata(&input, rawio::parse_tag(&tag)?, index)?;
            match output {
                Some(path) => {
                    let mut o = std::fs::OpenOptions::new();
                    o.write(true);
                    if force {
                        o.create(true).truncate(true);
                    } else {
                        o.create_new(true);
                    }
                    let mut f = o.open(&path).map_err(|e| {
                        rcchd::Error(format!(
                            "{}: {e} (use --force to replace it)",
                            path.display()
                        ))
                    })?;
                    f.write_all(&data)?;
                }
                None => std::io::stdout().write_all(&data)?,
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Listtemplates => {
            print!("{}", hdgeom::template_table());
            Ok(ExitCode::SUCCESS)
        }
        Command::Createld { .. } | Command::Extractld { .. } => Err(rcchd::Error(
            "laser disc images are not supported: they hold video (the avhu codec) that this tool has no input or output for"
                .into(),
        )),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse_from(normalize(std::env::args()));
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("rcchd: {e}");
            ExitCode::from(1)
        }
    }
}
