use clap::{Parser, Subcommand};
use rc_common::json::Json;
use rc_common::write_file;
use rctape::c64;
use rctape::spectrum::{self, Kind};
use rctape::{Block, CleanupOptions, Image, Tape};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "rctape",
    version,
    about = "Look inside, cut, merge, split, clean up and convert tape images",
    long_about = "Look inside, cut, merge, split, clean up and convert tape images.\n\n\
        ZX Spectrum .tap and .tzx are handled block by block, so a file written back is what was read. \
        Commodore .t64 can be listed and extracted, a raw Commodore .tap can be described. \
        C16/Plus4 version-2 .tap can also be structurally validated and normalized to another .tap.\n\n\
        VIC-20 version-0/1 .tap can be inspected, structurally validated and normalized; from-prg creates a KERNAL recording.\n\n\
        Block numbers start at 1, as `list` prints them: `3`, `2-5`, `4-`, `-3`, `1,4-6` or `all`.\n\n\
        rm, cleanup and add change TAPE in place (written atomically); use -o to write a copy instead. \
        Nothing else overwrites an existing file without --force."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Encode a VIC-20 PRG as a real KERNAL cassette recording (TAP v1, platform 1).
    FromPrg {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, default_value = "pal", value_parser = ["pal", "ntsc"])]
        region: String,
        /// PETSCII file name (ASCII, at most 16 characters; default: upper-case file stem).
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        force: bool,
    },
    /// Show the blocks.
    List {
        tape: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Show facts about the tape: format, number of blocks, playing time, checksum problems.
    Info {
        tape: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Check Spectrum blocks/checksums, or VIC-20/C16 TAP structure (not decoded files). Exit status 1 on block problems.
    Validate {
        tape: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Write the data of blocks (or .t64 programs) out as files.
    Extract {
        tape: PathBuf,
        /// Which blocks (default: all).
        blocks: Option<String>,
        /// Directory to write into (created if missing); default: the current directory.
        #[arg(long, short = 'd')]
        dir: Option<PathBuf>,
        /// With a single block: write it to this exact path instead.
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        /// Write whole blocks, flag byte and checksum included, headers too.
        #[arg(long)]
        raw: bool,
        #[arg(long)]
        force: bool,
    },
    /// Print one block's data to standard output: raw bytes, or with --hex a dump, or with --basic a BASIC listing.
    Cat {
        tape: PathBuf,
        block: usize,
        #[arg(long, conflicts_with = "basic")]
        hex: bool,
        /// List a tokenised BASIC program.
        #[arg(long)]
        basic: bool,
    },
    /// Make a new tape from some of the blocks of another.
    Cut {
        tape: PathBuf,
        blocks: String,
        #[arg(long, short = 'o')]
        output: PathBuf,
        /// Drop blocks a .tap cannot hold instead of failing.
        #[arg(long)]
        lossy: bool,
        #[arg(long)]
        force: bool,
    },
    /// Delete blocks.
    Rm {
        tape: PathBuf,
        blocks: String,
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Join tapes, one after the other.
    Merge {
        inputs: Vec<PathBuf>,
        #[arg(long, short = 'o')]
        output: PathBuf,
        #[arg(long)]
        lossy: bool,
        #[arg(long)]
        force: bool,
    },
    /// Cut a tape into one file per program (a header and the data after it).
    Split {
        tape: PathBuf,
        /// Directory to write into (created if missing); default: the current directory.
        #[arg(long, short = 'd')]
        dir: Option<PathBuf>,
        /// Output kind: tap or tzx (default: the kind of the input).
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        lossy: bool,
        #[arg(long)]
        force: bool,
    },
    /// Tidy a tape: strip information blocks, fix checksums, set pauses.
    Cleanup {
        tape: PathBuf,
        /// Remove text, message, archive info, hardware and custom info blocks and glue.
        #[arg(long)]
        strip_info: bool,
        /// Recompute wrong checksums of ROM blocks.
        #[arg(long)]
        fix_checksums: bool,
        /// Set every pause (after data blocks and in pause blocks, stops excepted) to this many milliseconds.
        #[arg(long, value_name = "MS")]
        pause: Option<u16>,
        /// Join pause blocks that follow each other.
        #[arg(long)]
        merge_pauses: bool,
        /// Remove data blocks with no data.
        #[arg(long)]
        drop_empty: bool,
        /// Everything above except --pause.
        #[arg(long)]
        all: bool,
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Put host files on the tape as header + data blocks (the tape is created if it does not exist).
    Add {
        tape: PathBuf,
        files: Vec<PathBuf>,
        /// Name in the header (single file only; default: the file name without extension, 10 characters).
        #[arg(long)]
        name: Option<String>,
        /// What the file is: `code` (default), `basic`, or `data` (a block with no header).
        #[arg(long = "type", short = 't', default_value = "code")]
        kind: String,
        /// Load address of code (`0x8000`, `$8000`, `32768`; default 32768).
        #[arg(long, value_parser = parse_u16)]
        load: Option<u16>,
        /// BASIC: line to start at (default: none).
        #[arg(long)]
        line: Option<u16>,
        /// Pause after each block in ms.
        #[arg(long, default_value_t = 1000)]
        pause: u16,
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Convert Spectrum .tap/.tzx or normalize VIC-20/C16 .tap to a new .tap.
    Convert {
        input: PathBuf,
        output: PathBuf,
        /// Drop blocks a .tap cannot hold instead of failing.
        #[arg(long)]
        lossy: bool,
        #[arg(long)]
        force: bool,
    },
}

fn parse_u16(text: &str) -> Result<u16, String> {
    rc_common::parse_address(text)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            rc_common::err(&format!("rctape: {message}\n"));
            ExitCode::from(2)
        }
    }
}

fn read(path: &Path) -> Result<Image, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    rctape::open(&path.to_string_lossy(), &bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// A tape that has blocks (.tap or .tzx); the Commodore kinds are refused with a hint.
fn read_blocks(path: &Path, what: &str) -> Result<(Tape, bool), String> {
    into_blocks(read(path)?, path, what)
}

fn into_blocks(image: Image, path: &Path, what: &str) -> Result<(Tape, bool), String> {
    match image {
        Image::Spectrum { tape, from_tap } => Ok((tape, from_tap)),
        Image::C64Tap(..) | Image::Vic20Tap(..) | Image::C16Tap(_) => Err(format!(
            "{}: a Commodore .tap is a raw pulse recording, there are no blocks to {what}",
            path.display()
        )),
        Image::T64(_) => Err(format!(
            "{}: a .t64 is a directory of programs, there are no blocks to {what} (try `extract`)",
            path.display()
        )),
    }
}

fn describe_vic20(video: c64_tape::vic20::VideoStandard, tape: &c64_tape::Tape, json: bool) {
    let cycles: u64 = tape.pulses().iter().map(|&n| u64::from(n)).sum();
    let seconds = cycles as f64 / f64::from(video.clock_hz());
    let region = match video {
        c64_tape::vic20::VideoStandard::Pal => "PAL",
        c64_tape::vic20::VideoStandard::Ntsc => "NTSC",
    };
    if json {
        rc_common::out(
            &Json::obj([
                ("format", Json::str("VIC-20 .tap")),
                ("region", Json::str(region)),
                ("pulses", Json::Num(tape.pulse_count() as i64)),
                ("cycles", Json::Num(cycles as i64)),
                ("clock_hz", Json::Num(i64::from(video.clock_hz()))),
                ("seconds", Json::Num(seconds.round() as i64)),
            ])
            .pretty(),
        );
    } else {
        rc_common::out(&format!(
            "VIC-20 .tap {region}: {} pulses, {seconds:.3} s\n",
            tape.pulse_count()
        ));
    }
}

fn describe_c16(tape: &c64_tape::ted::TedTape, json: bool) {
    let halfwaves = tape.halfwaves();
    let cycles: u64 = halfwaves.iter().map(|&n| u64::from(n)).sum();
    let clock = tape.video().clock_hz();
    let seconds = cycles as f64 / f64::from(clock);
    let shortest = halfwaves.iter().copied().min().unwrap_or(0);
    let longest = halfwaves.iter().copied().max().unwrap_or(0);
    if json {
        rc_common::out(
            &Json::obj([
                ("format", Json::str("C16/Plus4 .tap version 2")),
                ("video", Json::str(tape.video().name())),
                ("platform", Json::Num(2)),
                ("clock_hz", Json::Num(i64::from(clock))),
                ("halfwaves", Json::Num(halfwaves.len() as i64)),
                ("cycles", Json::Num(cycles as i64)),
                ("seconds", Json::Num(seconds.round() as i64)),
                ("shortest_cycles", Json::Num(i64::from(shortest))),
                ("longest_cycles", Json::Num(i64::from(longest))),
            ])
            .pretty(),
        );
    } else {
        rc_common::out(&format!(
            "format:    C16/Plus4 .tap version 2\nvideo:     {}\nclock:     {clock} Hz (TED timer)\nhalfwaves: {}\nplays for: {seconds:.1} s\nhalfwave range: {shortest} to {longest} cycles\n",
            tape.video().name(),
            halfwaves.len()
        ));
    }
}

fn is_tap(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("tap"))
}

/// Writes a tape as `.tap` or `.tzx` by the path's extension.
fn write_tape(path: &Path, tape: &Tape, lossy: bool, force: bool) -> Result<(), String> {
    if is_tap(path) {
        let (bytes, notes) = rctape::to_tap(tape, lossy)?;
        for n in notes {
            rc_common::err(&format!("rctape: note: {n}\n"));
        }
        write_file(path, &bytes, force)
    } else if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("tzx"))
    {
        write_file(path, &tape.to_bytes(), force)
    } else {
        Err(format!(
            "{}: the output must end in .tap or .tzx",
            path.display()
        ))
    }
}

/// Saves a changed tape: in place (replacing the input) unless `-o` names another file.
fn save(tape: &Tape, input: &Path, output: &Option<PathBuf>, force: bool) -> Result<(), String> {
    match output {
        Some(out) => write_tape(out, tape, false, force),
        None if is_tap(input) => write_tape(input, tape, false, true),
        None => write_file(input, &tape.to_bytes(), true),
    }
}

fn run() -> Result<ExitCode, String> {
    let cli = Cli::parse();
    match cli.command {
        Command::List { tape, json } => match read(&tape)? {
            Image::Spectrum { tape, .. } => {
                let rows = rctape::rows(&tape);
                if json {
                    let items = rows
                        .iter()
                        .map(|r| {
                            Json::obj([
                                ("block", Json::Num(r.index as i64)),
                                ("id", Json::Num(r.id as i64)),
                                ("kind", Json::str(r.kind)),
                                ("size", r.size.map_or(Json::Null, |s| Json::Num(s as i64))),
                                (
                                    "pause_ms",
                                    r.pause_ms.map_or(Json::Null, |p| Json::Num(p as i64)),
                                ),
                                ("description", Json::str(r.description.clone())),
                                ("checksum_ok", r.checksum_ok.map_or(Json::Null, Json::Bool)),
                            ])
                        })
                        .collect();
                    rc_common::out(&Json::Arr(items).pretty());
                } else {
                    let mut text = String::new();
                    for r in &rows {
                        let flag = if r.checksum_ok == Some(false) {
                            "  BAD CHECKSUM"
                        } else {
                            ""
                        };
                        let size = r.size.map_or(String::new(), |s| format!("{s}"));
                        text.push_str(&format!(
                            "{:>3}  {:<20} {:>6}  {}{flag}\n",
                            r.index, r.kind, size, r.description
                        ));
                    }
                    rc_common::out(&text);
                }
            }
            Image::T64(t) => {
                let rows = c64::t64_rows(&t);
                if json {
                    let items = rows
                        .iter()
                        .map(|r| {
                            Json::obj([
                                ("entry", Json::Num(r.index as i64)),
                                ("name", Json::str(r.name.clone())),
                                ("type", Json::str(r.kind)),
                                ("start", Json::Num(r.start as i64)),
                                ("end", Json::Num(r.end as i64)),
                                ("size", Json::Num(r.size as i64)),
                            ])
                        })
                        .collect();
                    rc_common::out(&Json::Arr(items).pretty());
                } else {
                    let mut text = String::new();
                    for r in &rows {
                        text.push_str(&format!(
                            "{:>3}  {:<16} {}  ${:04X}-${:04X}  {} bytes\n",
                            r.index, r.name, r.kind, r.start, r.end, r.size
                        ));
                    }
                    rc_common::out(&text);
                }
            }
            Image::C64Tap(tap, bytes) => {
                let i = c64::tap_info(&tap, &bytes);
                rc_common::err(
                    "rctape: a Commodore .tap is a raw pulse recording; there are no blocks to list (see `info`)\n",
                );
                rc_common::out(&format!("{} pulses, {:.1} s\n", i.pulses, i.seconds));
            }
            Image::Vic20Tap(video, tape) => {
                rc_common::err("rctape: a VIC-20 .tap holds raw pulses, not decoded file blocks\n");
                describe_vic20(video, &tape, json);
            }
            Image::C16Tap(tape) => {
                rc_common::err(
                    "rctape: a C16 .tap is a raw pulse recording; decoded file listing is not supported\n",
                );
                describe_c16(&tape, json);
            }
        },
        Command::Info { tape, json } => {
            let size = std::fs::metadata(&tape).map(|m| m.len()).unwrap_or(0);
            match read(&tape)? {
                Image::Spectrum { tape: t, from_tap } => {
                    let rows = rctape::rows(&t);
                    let bad = rows.iter().filter(|r| r.checksum_ok == Some(false)).count();
                    let data: usize = rows.iter().filter_map(|r| r.size).sum();
                    let (secs, exact) = rctape::duration_seconds(&t);
                    let format = if from_tap {
                        "ZX Spectrum .tap".to_string()
                    } else {
                        format!("TZX {}.{:02}", t.major, t.minor)
                    };
                    if json {
                        rc_common::out(
                            &Json::obj([
                                ("format", Json::str(format)),
                                ("file_bytes", Json::Num(size as i64)),
                                ("blocks", Json::Num(t.blocks.len() as i64)),
                                ("data_bytes", Json::Num(data as i64)),
                                ("seconds", Json::Num(secs.round() as i64)),
                                ("seconds_exact", Json::Bool(exact)),
                                ("bad_checksums", Json::Num(bad as i64)),
                            ])
                            .pretty(),
                        );
                    } else {
                        rc_common::out(&format!(
                            "format:    {format}\nblocks:    {}\ndata:      {data} bytes\nplays for: {}{:.0} s\nchecksums: {}\n",
                            t.blocks.len(),
                            if exact { "" } else { "at least " },
                            secs,
                            if bad == 0 {
                                "all good".to_string()
                            } else {
                                format!("{bad} bad")
                            }
                        ));
                    }
                }
                Image::T64(t) => {
                    let rows = c64::t64_rows(&t);
                    let bytes: usize = rows.iter().map(|r| r.size).sum();
                    rc_common::out(&format!(
                        "format:    Commodore .t64\nentries:   {}\ndata:      {bytes} bytes\n",
                        rows.len()
                    ));
                }
                Image::C64Tap(tap, bytes) => {
                    let i = c64::tap_info(&tap, &bytes);
                    if json {
                        rc_common::out(
                            &Json::obj([
                                (
                                    "format",
                                    Json::str(format!("Commodore .tap version {}", i.version)),
                                ),
                                ("video", Json::str(i.video)),
                                ("platform", Json::Num(i.platform as i64)),
                                ("pulses", Json::Num(i.pulses as i64)),
                                ("seconds", Json::Num(i.seconds.round() as i64)),
                                ("shortest_cycles", Json::Num(i.shortest as i64)),
                                ("longest_cycles", Json::Num(i.longest as i64)),
                            ])
                            .pretty(),
                        );
                    } else {
                        rc_common::out(&format!(
                            "format:    Commodore .tap version {}\nvideo:     {}\npulses:    {}\nplays for: {:.1} s\npulse range: {} to {} cycles\n",
                            i.version, i.video, i.pulses, i.seconds, i.shortest, i.longest
                        ));
                    }
                }
                Image::C16Tap(tape) => describe_c16(&tape, json),
                Image::Vic20Tap(video, tape) => describe_vic20(video, &tape, json),
            }
        }
        Command::Validate { tape, json } => {
            let (t, _) = match read(&tape)? {
                Image::Vic20Tap(..) => {
                    let checks = "header, size and full-pulse encoding only; decoded file checksums are not checked";
                    if json {
                        rc_common::out(
                            &Json::obj([("ok", Json::Bool(true)), ("checks", Json::str(checks))])
                                .pretty(),
                        );
                    } else {
                        rc_common::out(&format!("VIC-20 TAP validated: {checks}\n"));
                    }
                    return Ok(ExitCode::SUCCESS);
                }
                Image::C16Tap(_) => {
                    let checks = "header, size and halfwave encoding only; decoded file checksums are not checked";
                    if json {
                        rc_common::out(
                            &Json::obj([("ok", Json::Bool(true)), ("checks", Json::str(checks))])
                                .pretty(),
                        );
                    } else {
                        rc_common::out(&format!("C16 TAP validated: {checks}\n"));
                    }
                    return Ok(ExitCode::SUCCESS);
                }
                image => into_blocks(image, &tape, "validate")?,
            };
            let mut problems: Vec<String> = Vec::new();
            // The header block waiting for its data block: (block number, header).
            let mut waiting: Option<(usize, spectrum::Header)> = None;
            for (i, b) in t.blocks.iter().enumerate() {
                let n = i + 1;
                let Some(data) = b.payload() else { continue };
                if b.id != 0x14 && !spectrum::checksum_ok(data) {
                    problems.push(format!("block {n}: wrong checksum"));
                }
                let header = if b.id == 0x10 {
                    spectrum::parse_header(data)
                } else {
                    None
                };
                if let Some((hn, h)) = waiting.take() {
                    if header.is_some() {
                        problems.push(format!(
                            "block {hn}: header \"{}\" has no data block after it",
                            h.name
                        ));
                    } else if data.len() != h.length as usize + 2 {
                        problems.push(format!(
                            "block {n}: {} bytes of data, but the header (block {hn}) says {}",
                            data.len().saturating_sub(2),
                            h.length
                        ));
                    }
                }
                if let Some(h) = header {
                    waiting = Some((n, h));
                }
            }
            if let Some((hn, h)) = waiting {
                problems.push(format!(
                    "block {hn}: header \"{}\" has no data block after it",
                    h.name
                ));
            }
            if json {
                rc_common::out(
                    &Json::obj([
                        ("ok", Json::Bool(problems.is_empty())),
                        (
                            "problems",
                            Json::Arr(problems.iter().cloned().map(Json::Str).collect()),
                        ),
                    ])
                    .pretty(),
                );
            } else if problems.is_empty() {
                rc_common::out("ok\n");
            } else {
                for p in &problems {
                    rc_common::out(&format!("{p}\n"));
                }
            }
            if !problems.is_empty() {
                return Ok(ExitCode::from(1));
            }
        }
        Command::Extract {
            tape,
            blocks,
            dir,
            output,
            raw,
            force,
        } => {
            let files: Vec<(String, Vec<u8>)> = match read(&tape)? {
                Image::Spectrum { tape: t, .. } => {
                    let chosen = match &blocks {
                        Some(sel) => rctape::parse_selection(sel, t.blocks.len())?,
                        None => (0..t.blocks.len()).collect(),
                    };
                    rctape::extract(&t, &chosen, raw)
                        .into_iter()
                        .map(|e| {
                            rc_common::err(&format!("rctape: {} ({})\n", e.name, e.description));
                            (e.name, e.bytes)
                        })
                        .collect()
                }
                Image::T64(t) => {
                    let chosen = match &blocks {
                        Some(sel) => rctape::parse_selection(sel, t.entries().len())?,
                        None => (0..t.entries().len()).collect(),
                    };
                    chosen
                        .into_iter()
                        .filter_map(|i| {
                            let e = &t.entries()[i];
                            c64::t64_prg(e).map(|b| (c64::t64_file_name(e), b))
                        })
                        .collect()
                }
                Image::C64Tap(..) | Image::Vic20Tap(..) | Image::C16Tap(_) => {
                    return Err(format!(
                        "{}: a Commodore .tap holds pulses, not files",
                        tape.display()
                    ));
                }
            };
            if files.is_empty() {
                return Err("nothing to extract".to_string());
            }
            if let Some(out) = output {
                if files.len() != 1 {
                    return Err(format!(
                        "-o needs exactly one block; {} would be written",
                        files.len()
                    ));
                }
                write_file(&out, &files[0].1, force)?;
            } else {
                let dir = dir.unwrap_or_else(|| PathBuf::from("."));
                std::fs::create_dir_all(&dir)
                    .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
                for (name, bytes) in &files {
                    write_file(&dir.join(name), bytes, force)?;
                }
            }
        }
        Command::Cat {
            tape,
            block,
            hex,
            basic,
        } => {
            let (t, _) = read_blocks(&tape, "print")?;
            let b = block
                .checked_sub(1)
                .and_then(|i| t.blocks.get(i))
                .ok_or_else(|| {
                    format!(
                        "the tape has {} block(s); there is no block {block}",
                        t.blocks.len()
                    )
                })?;
            let data = b
                .payload()
                .ok_or_else(|| format!("block {block} ({}) holds no data to print", b.kind()))?;
            let body = match data {
                [0xFF, rest @ .., _] | [0x00, rest @ .., _] => rest,
                _ => data,
            };
            if basic {
                rc_common::out(&spectrum::list_basic(body));
            } else if hex {
                rc_common::out(&rctape::hexdump(body));
            } else {
                rc_common::out_bytes(body);
            }
        }
        Command::Cut {
            tape,
            blocks,
            output,
            lossy,
            force,
        } => {
            let (t, _) = read_blocks(&tape, "cut")?;
            let chosen = rctape::parse_selection(&blocks, t.blocks.len())?;
            write_tape(&output, &rctape::keep(&t, &chosen), lossy, force)?;
            rc_common::err(&format!(
                "rctape: {} block(s) -> {}\n",
                chosen.len(),
                output.display()
            ));
        }
        Command::Rm {
            tape,
            blocks,
            output,
            force,
        } => {
            let (t, _) = read_blocks(&tape, "delete")?;
            let chosen = rctape::parse_selection(&blocks, t.blocks.len())?;
            let rest = rctape::remove(&t, &chosen);
            save(&rest, &tape, &output, force)?;
            rc_common::err(&format!(
                "rctape: removed {} block(s), {} left\n",
                chosen.len(),
                rest.blocks.len()
            ));
        }
        Command::Merge {
            inputs,
            output,
            lossy,
            force,
        } => {
            if inputs.len() < 2 {
                return Err("merge needs at least two tapes".to_string());
            }
            let tapes = inputs
                .iter()
                .map(|p| read_blocks(p, "merge").map(|r| r.0))
                .collect::<Result<Vec<_>, _>>()?;
            let merged = rctape::merge(&tapes);
            write_tape(&output, &merged, lossy, force)?;
            rc_common::err(&format!(
                "rctape: {} tapes, {} blocks -> {}\n",
                tapes.len(),
                merged.blocks.len(),
                output.display()
            ));
        }
        Command::Split {
            tape,
            dir,
            to,
            lossy,
            force,
        } => {
            let (t, from_tap) = read_blocks(&tape, "split")?;
            let ext = match to.as_deref() {
                Some("tap") => "tap",
                Some("tzx") => "tzx",
                None if from_tap => "tap",
                None => "tzx",
                Some(other) => return Err(format!("--to must be tap or tzx, not \"{other}\"")),
            };
            let dir = dir.unwrap_or_else(|| PathBuf::from("."));
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            let parts = rctape::split(&t);
            for (i, (stem, part)) in parts.iter().enumerate() {
                let path = dir.join(format!("{:02}-{stem}.{ext}", i + 1));
                write_tape(&path, part, lossy, force)?;
                rc_common::err(&format!("rctape: {}\n", path.display()));
            }
        }
        Command::Cleanup {
            tape,
            strip_info,
            fix_checksums,
            pause,
            merge_pauses,
            drop_empty,
            all,
            output,
            force,
        } => {
            let (t, _) = read_blocks(&tape, "clean up")?;
            let opts = CleanupOptions {
                strip_info: strip_info || all,
                fix_checksums: fix_checksums || all,
                pause_ms: pause,
                merge_pauses: merge_pauses || all,
                drop_empty: drop_empty || all,
            };
            let (clean, notes) = rctape::cleanup(&t, &opts);
            for n in &notes {
                rc_common::err(&format!("rctape: {n}\n"));
            }
            if notes.is_empty() {
                rc_common::err("rctape: nothing to change\n");
            }
            save(&clean, &tape, &output, force)?;
        }
        Command::Add {
            tape,
            files,
            name,
            kind,
            load,
            line,
            pause,
            output,
            force,
        } => {
            if files.is_empty() {
                return Err("no files to add".to_string());
            }
            if name.is_some() && files.len() != 1 {
                return Err("--name works with a single file".to_string());
            }
            let mut t = if tape.exists() {
                read_blocks(&tape, "add to")?.0
            } else {
                Tape::new()
            };
            for f in &files {
                let data =
                    std::fs::read(f).map_err(|e| format!("cannot read {}: {e}", f.display()))?;
                let stem = f
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let tape_name = name.clone().unwrap_or(stem);
                if data.len() > 0xFFFF - 2 {
                    return Err(format!(
                        "{}: {} bytes do not fit in one block (65533 at most)",
                        f.display(),
                        data.len()
                    ));
                }
                let framed = |flag: u8| {
                    let mut v = vec![flag];
                    v.extend_from_slice(&data);
                    v.push(spectrum::checksum_of(&v));
                    v
                };
                match kind.as_str() {
                    "code" => {
                        let at = load.unwrap_or(32768);
                        t.blocks.push(Block::standard(
                            &spectrum::make_header(
                                Kind::Code,
                                &tape_name,
                                data.len() as u16,
                                at,
                                32768,
                            ),
                            pause,
                        ));
                    }
                    "basic" => {
                        let auto = line.unwrap_or(32768);
                        t.blocks.push(Block::standard(
                            &spectrum::make_header(
                                Kind::Program,
                                &tape_name,
                                data.len() as u16,
                                auto,
                                data.len() as u16,
                            ),
                            pause,
                        ));
                    }
                    "data" => {}
                    other => {
                        return Err(format!(
                            "--type must be code, basic or data, not \"{other}\""
                        ));
                    }
                }
                t.blocks.push(Block::standard(&framed(0xFF), pause));
                rc_common::err(&format!(
                    "rctape: {} -> {} block(s)\n",
                    f.display(),
                    if kind == "data" { 1 } else { 2 }
                ));
            }
            if tape.exists() {
                save(&t, &tape, &output, force)?;
            } else {
                write_tape(output.as_ref().unwrap_or(&tape), &t, false, force)?;
            }
        }
        Command::Convert {
            input,
            output,
            lossy,
            force,
        } => {
            match read(&input)? {
                Image::Vic20Tap(video, tape) => {
                    if !is_tap(&output) {
                        return Err("VIC-20 TAP normalization requires a .tap output".to_string());
                    }
                    let bytes = c64_tape::vic20::write(video, &tape)
                        .map_err(|e| format!("VIC-20 .tap: {e}"))?;
                    write_file(&output, &bytes, force)?;
                }
                Image::C16Tap(tape) => {
                    if !is_tap(&output) {
                        return Err("C16 TAP normalization requires a .tap output".to_string());
                    }
                    let bytes = tape
                        .to_bytes()
                        .map_err(|e| format!("C16/Plus4 .tap: {e}"))?;
                    write_file(&output, &bytes, force)?;
                }
                image => {
                    let (t, _) = into_blocks(image, &input, "convert")?;
                    write_tape(&output, &t, lossy, force)?;
                }
            }
            rc_common::err(&format!(
                "rctape: {} -> {}\n",
                input.display(),
                output.display()
            ));
        }
        Command::FromPrg {
            input,
            output,
            region,
            name,
            force,
        } => {
            if !is_tap(&output) {
                return Err("VIC-20 PRG encoding requires a .tap output".to_string());
            }
            let bytes = std::fs::read(&input)
                .map_err(|e| format!("cannot read {}: {e}", input.display()))?;
            if !(3..=65_538).contains(&bytes.len()) {
                return Err(
                    "a PRG requires a two-byte load address and 1..=65536 data bytes".to_string(),
                );
            }
            let start = u16::from_le_bytes([bytes[0], bytes[1]]);
            if usize::from(start) + bytes.len() - 2 > 65_535 {
                return Err("PRG end address does not fit in the KERNAL tape header".to_string());
            }
            let name = name
                .unwrap_or_else(|| {
                    input
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default()
                })
                .to_ascii_uppercase();
            if name.is_empty()
                || name.len() > 16
                || !name.bytes().all(|b| (0x20..=0x5f).contains(&b))
            {
                return Err("VIC-20 tape name must contain 1..=16 ASCII/PETSCII characters from space through underscore".to_string());
            }
            let video = if region == "ntsc" {
                c64_tape::vic20::VideoStandard::Ntsc
            } else {
                c64_tape::vic20::VideoStandard::Pal
            };
            let tape = c64_tape::vic20::encode_program(video, name.as_bytes(), start, &bytes[2..])
                .map_err(|e| format!("cannot encode VIC-20 tape: {e}"))?;
            write_file(&output, &tape, force)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}
