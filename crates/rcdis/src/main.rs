use clap::Parser;
use machine::SymbolTable;
use rc_common::{parse_address, write_file};
use rcasm::Cpu;
use rcdis::{Options, disassemble};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "rcdis",
    version,
    about = "Disassembler for the 6502 and the Z80 that writes source rcasm assembles back",
    long_about = "Disassembler for the 6502 and the Z80. The output is rcasm source: assembling it gives \
        the input bytes back (rcdis checks every instruction).\n\n\
        Without --entry/--follow every byte is decoded in order (linear). With them, jumps, calls and \
        branches are followed from the entry points and everything else stays data."
)]
struct Cli {
    /// The file to disassemble: a raw binary, or a .prg (the load address is read from its header).
    file: PathBuf,

    /// CPU: 6502 (also 6510, 8502) or z80.
    #[arg(long, short = 'c')]
    cpu: String,

    /// Address of the first byte (needed for raw files; a .prg brings its own).
    #[arg(long)]
    org: Option<String>,

    /// Treat the file as a .prg: the first two bytes are the load address.
    #[arg(long)]
    prg: bool,

    /// Skip this many bytes at the start of the file (after a .prg header).
    #[arg(long, default_value_t = 0)]
    skip: usize,

    /// Use only this many bytes.
    #[arg(long)]
    length: Option<usize>,

    /// Entry point: follow the flow of control from here (repeatable).
    #[arg(long, short = 'e', value_name = "ADDR")]
    entry: Vec<String>,

    /// Follow the flow of control from the first address.
    #[arg(long)]
    follow: bool,

    /// Addresses START-END (inclusive) that are data (repeatable).
    #[arg(long, value_name = "START-END")]
    data: Vec<String>,

    /// Symbol file naming addresses (NAME = $1234, NAME 1234, VICE labels); repeatable.
    #[arg(long, short = 's', value_name = "FILE")]
    symbols: Vec<PathBuf>,

    /// Add `; ADDR: BYTES` to every line.
    #[arg(long)]
    addresses: bool,

    /// Write the source here instead of the terminal.
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,

    /// Overwrite the output file if it exists.
    #[arg(long)]
    force: bool,
}

fn fail(message: impl std::fmt::Display) -> ExitCode {
    rc_common::err(&format!("rcdis: {message}\n"));
    ExitCode::from(1)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(cpu) = Cpu::parse(&cli.cpu) else {
        return fail(format!("unknown CPU \"{}\" (6502 or z80)", cli.cpu));
    };
    let raw = match std::fs::read(&cli.file) {
        Ok(b) => b,
        Err(e) => return fail(format!("cannot read {}: {e}", cli.file.display())),
    };
    let is_prg = cli.prg
        || cli
            .file
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("prg"));
    let (header_org, body) = if is_prg {
        if raw.len() < 2 {
            return fail(format!("{} is too short for a .prg", cli.file.display()));
        }
        (Some(u16::from_le_bytes([raw[0], raw[1]])), &raw[2..])
    } else {
        (None, &raw[..])
    };
    let org = match (&cli.org, header_org) {
        (Some(o), _) => match parse_address(o) {
            Ok(a) => a,
            Err(e) => return fail(format!("--org: {e}")),
        },
        (None, Some(a)) => a,
        (None, None) => return fail("a raw file needs --org <address>"),
    };
    if cli.skip > body.len() {
        return fail(format!("--skip {} is past the end of the file", cli.skip));
    }
    let body = &body[cli.skip..];
    let body = &body[..cli.length.map_or(body.len(), |l| l.min(body.len()))];
    let org = match u16::try_from(org as usize + cli.skip) {
        Ok(a) => a,
        Err(_) => return fail("the start address passes $FFFF"),
    };

    let mut opts = Options::new(cpu, org);
    for e in &cli.entry {
        match parse_address(e) {
            Ok(a) => opts.entries.push(a),
            Err(m) => return fail(format!("--entry: {m}")),
        }
    }
    if cli.follow && opts.entries.is_empty() {
        opts.entries.push(org);
    }
    for d in &cli.data {
        let Some((s, e)) = d.split_once('-') else {
            return fail(format!("--data {d}: expected START-END"));
        };
        match (parse_address(s), parse_address(e)) {
            (Ok(s), Ok(e)) if s <= e => opts.data.push((s, e)),
            (Ok(_), Ok(_)) => return fail(format!("--data {d}: START is after END")),
            (Err(m), _) | (_, Err(m)) => return fail(format!("--data {d}: {m}")),
        }
    }
    for path in &cli.symbols {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return fail(format!("cannot read {}: {e}", path.display())),
        };
        match SymbolTable::parse(&text) {
            Ok(t) => opts.symbols.merge(&t),
            Err(e) => return fail(format!("{}: {e}", path.display())),
        }
    }
    opts.comments = cli.addresses;

    let result = match disassemble(body, &opts) {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    match &cli.output {
        Some(path) => {
            if let Err(e) = write_file(path, result.source.as_bytes(), cli.force) {
                return fail(e);
            }
        }
        None => rc_common::out(&result.source),
    }
    rc_common::err(&format!(
        "rcdis: {} instructions, {} data bytes, {} labels",
        result.instructions, result.data_bytes, result.labels
    ));
    ExitCode::SUCCESS
}
