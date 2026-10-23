use clap::{Parser, ValueEnum};
use rc_common::{Diag, parse_number, write_file};
use rcasm::{Cpu, DiskFiles, Options, SymbolStyle};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "rcasm",
    version,
    about = "Two-pass assembler for the 6502, the Z80 and the 68000",
    long_about = "Two-pass assembler for the 6502 and the Z80 with one shared syntax.\n\n\
        Labels (`name:`, local `@name`), `=`/`equ`, expressions, .org .byte .word .text .fill .align \
        .incbin .include .macro/.endm .if/.else/.endif .cpu .illegal .assert .error, `;` comments.\n\
        Writes a flat binary (or a .prg), a symbol file and a listing."
)]
struct Cli {
    /// The source file.
    source: PathBuf,

    /// CPU: 6502 (also 6510, 8502) or z80. A `.cpu` directive in the source can say it instead.
    #[arg(long, short = 'c')]
    cpu: Option<String>,

    /// Output file.
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,

    /// What to write to the output file.
    #[arg(long, short = 'f', value_enum, default_value = "bin")]
    format: Format,

    /// Byte used between segments in `bin` and `prg` output.
    #[arg(long, default_value = "0", value_parser = parse_byte)]
    fill: u8,

    /// Write the symbol file here.
    #[arg(long, short = 's')]
    symbols: Option<PathBuf>,

    /// Symbol file style.
    #[arg(long, value_enum, default_value = "rc")]
    symbols_style: Style,

    /// Include constants (`=`, `equ`, -D) in the symbol file, not only labels.
    #[arg(long)]
    all_symbols: bool,

    /// Write a listing here.
    #[arg(long, short = 'l')]
    listing: Option<PathBuf>,

    /// Allow the undocumented 6502 opcodes.
    #[arg(long)]
    illegal: bool,

    /// Define a constant: NAME or NAME=VALUE (VALUE is 1 when left out).
    #[arg(short = 'D', value_name = "NAME[=VALUE]")]
    define: Vec<String>,

    /// Directory searched for .include / .incbin files (repeatable).
    #[arg(short = 'I', value_name = "DIR")]
    include: Vec<PathBuf>,

    /// Assemble and report errors, write nothing.
    #[arg(long)]
    check: bool,

    /// Overwrite output files that already exist.
    #[arg(long)]
    force: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    /// One flat image from the lowest to the highest address.
    Bin,
    /// A .prg: the load address, then the flat image.
    Prg,
    /// 68000 only: a relocatable GEMDOS executable (`.PRG`) with text, data and bss sections.
    Gemdos,
    /// One file per segment: `<output>.<address>.bin`.
    Segments,
}

#[derive(Clone, Copy, ValueEnum)]
enum Style {
    Rc,
    Vice,
}

fn parse_byte(text: &str) -> Result<u8, String> {
    let v = parse_number(text)?;
    u8::try_from(v).map_err(|_| format!("\"{text}\" does not fit in a byte"))
}

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("rcasm: {message}");
    ExitCode::from(1)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.cpu.as_deref().is_some_and(is_68000) {
        return run_68000(&cli);
    }
    if matches!(cli.format, Format::Gemdos) {
        return fail("--format gemdos is for --cpu 68000");
    }
    let cpu = match cli.cpu.as_deref() {
        None => None,
        Some(name) => match Cpu::parse(name) {
            Some(c) => Some(c),
            None => return fail(format!("unknown CPU \"{name}\" (6502 or z80)")),
        },
    };
    let mut defines = Vec::new();
    for d in &cli.define {
        let (name, value) = match d.split_once('=') {
            Some((n, v)) => match parse_number(v) {
                Ok(v) => (n, v),
                Err(e) => return fail(format!("-D {d}: {e}")),
            },
            None => (d.as_str(), 1),
        };
        defines.push((name.to_string(), value));
    }
    let source = match std::fs::read(&cli.source) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return fail(format!("cannot read {}: {e}", cli.source.display())),
    };
    let opts = Options {
        cpu,
        illegal: cli.illegal,
        include_dirs: cli.include.clone(),
        defines,
    };
    let name = cli.source.to_string_lossy().into_owned();
    let asm = match rcasm::assemble(&name, &source, &opts, &DiskFiles) {
        Ok(a) => a,
        Err(errors) => {
            report(&errors);
            return ExitCode::from(1);
        }
    };
    let total: usize = asm.segments.iter().map(|s| s.bytes.len()).sum();
    if cli.check {
        eprintln!(
            "rcasm: ok, {total} bytes in {} segment(s)",
            asm.segments.len()
        );
        return ExitCode::SUCCESS;
    }
    let Some(output) = cli.output.as_ref() else {
        return fail("no output file: give -o <file> (or --check)");
    };
    let result = (|| -> Result<(), String> {
        match cli.format {
            Format::Bin => {
                let (_, image) = rcasm::flat_image(&asm, cli.fill)?;
                write_file(output, &image, cli.force)?;
            }
            Format::Prg => {
                let image = rcasm::prg_image(&asm, cli.fill)?;
                write_file(output, &image, cli.force)?;
            }
            Format::Gemdos => unreachable!("checked above"),
            Format::Segments => {
                if asm.segments.is_empty() {
                    return Err("the program produced no bytes".to_string());
                }
                for seg in &asm.segments {
                    let mut file = output.as_os_str().to_owned();
                    file.push(format!(".{:04X}.bin", seg.start));
                    write_file(&PathBuf::from(file), &seg.bytes, cli.force)?;
                }
            }
        }
        if let Some(path) = &cli.symbols {
            let style = match cli.symbols_style {
                Style::Rc => SymbolStyle::Rc,
                Style::Vice => SymbolStyle::Vice,
            };
            let text = rcasm::symbols_text(&asm, cli.all_symbols, style);
            write_file(path, text.as_bytes(), cli.force)?;
        }
        if let Some(path) = &cli.listing {
            write_file(path, rcasm::listing_text(&asm).as_bytes(), cli.force)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        return fail(e);
    }
    let ranges: Vec<String> = asm
        .segments
        .iter()
        .map(|s| format!("${:04X}-${:04X}", s.start, s.end() - 1))
        .collect();
    eprintln!("rcasm: {total} bytes ({})", ranges.join(", "));
    ExitCode::SUCCESS
}

fn is_68000(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "68000" | "m68000" | "68k" | "mc68000"
    )
}

/// `--cpu 68000`: the 68000 back end. `bin` is a flat image at `.org`; `gemdos` is a relocatable `.PRG`.
fn run_68000(cli: &Cli) -> ExitCode {
    use rcasm::m68k::{M68kOptions, assemble_m68k, listing_text, symbols_text};
    let mut defines = Vec::new();
    for d in &cli.define {
        let (name, value) = match d.split_once('=') {
            Some((n, v)) => match parse_number(v) {
                Ok(v) => (n, v),
                Err(e) => return fail(format!("-D {d}: {e}")),
            },
            None => (d.as_str(), 1),
        };
        defines.push((name.to_string(), value));
    }
    let source = match std::fs::read(&cli.source) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return fail(format!("cannot read {}: {e}", cli.source.display())),
    };
    let relocatable = matches!(cli.format, Format::Gemdos);
    if matches!(cli.format, Format::Prg | Format::Segments) {
        return fail(
            "for the 68000 the formats are bin (a flat image at .org) and gemdos (a relocatable .PRG)",
        );
    }
    let opts = M68kOptions {
        relocatable,
        include_dirs: cli.include.clone(),
        defines,
    };
    let name = cli.source.to_string_lossy().into_owned();
    let asm = match assemble_m68k(&name, &source, &opts, &DiskFiles) {
        Ok(a) => a,
        Err(errors) => {
            report(&errors);
            return ExitCode::from(1);
        }
    };
    let summary = format!(
        "text {} bytes, data {} bytes, bss {} bytes{}",
        asm.text.len(),
        asm.data.len(),
        asm.bss,
        if relocatable {
            format!(", {} relocations", asm.relocs.len())
        } else {
            format!(", origin ${:06X}", asm.origin)
        }
    );
    if cli.check {
        eprintln!("rcasm: ok, {summary}");
        return ExitCode::SUCCESS;
    }
    let Some(output) = cli.output.as_ref() else {
        return fail("no output file: give -o <file> (or --check)");
    };
    let result = (|| -> Result<(), String> {
        if relocatable {
            let prg = rc_common::prg::Prg {
                text: asm.text.clone(),
                data: asm.data.clone(),
                bss: asm.bss,
                symbols: Vec::new(),
                flags: 0,
                fixups: asm.relocs.clone(),
            };
            write_file(output, &prg.to_bytes(), cli.force)?;
        } else {
            let image = asm.image();
            if image.is_empty() {
                return Err("the program produced no bytes".to_string());
            }
            write_file(output, &image, cli.force)?;
        }
        if let Some(path) = &cli.symbols {
            write_file(
                path,
                symbols_text(&asm, cli.all_symbols).as_bytes(),
                cli.force,
            )?;
        }
        if let Some(path) = &cli.listing {
            write_file(path, listing_text(&asm).as_bytes(), cli.force)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        return fail(e);
    }
    eprintln!("rcasm: {summary}");
    ExitCode::SUCCESS
}

fn report(errors: &[Diag]) {
    for e in errors {
        eprintln!("{e}");
    }
    eprintln!("rcasm: {} error(s)", errors.len());
}
