use clap::{Parser, Subcommand};
use rc_common::write_file;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "rcld",
    version,
    about = "Place assembled programs in memory and write a runnable image",
    long_about = "Place assembled programs in memory and write a runnable image.\n\n\
        A layout file (TOML) names the inputs (assembler sources, raw binaries, .prg files, symbol files), the memory \
        regions they may use and the machine. rcld assembles the sources, checks that nothing overlaps or leaves its \
        region, and writes the image the output's extension asks for: .prg, .d64, .d81 or .crt for the C64; .tap, \
        .tzx, .sna, .z80, .szx or .dsk for the Spectrum; .dsk for the CPC; .xex for Atari; .bin for any.\n\n\
        `rcld template c64|spectrum|cpc` prints a starting layout. Nothing overwrites an existing file without --force."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect ordered XEX load segments without executing INITAD/RUNAD.
    Inspect { image: PathBuf },
    /// Link: assemble, place, check and write the image.
    Build {
        layout: PathBuf,
        /// The image to write (default: `output` in the layout). The extension chooses the kind.
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,
        /// Also write a map of the placed pieces and the region usage.
        #[arg(long)]
        map: Option<PathBuf>,
        /// Also write the labels of the sources as a symbol file (for `retroclawd --symbols`).
        #[arg(long)]
        symbols: Option<PathBuf>,
        /// A constant for the sources, like `rcasm -D`.
        #[arg(short = 'D', value_name = "NAME=VALUE")]
        defines: Vec<String>,
        #[arg(long)]
        force: bool,
    },
    /// Print a starting layout file for a machine.
    Template { machine: String },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            rc_common::err(&format!("rcld: {message}\n"));
            ExitCode::from(2)
        }
    }
}

fn parse_define(text: &str) -> Result<(String, i64), String> {
    let (name, value) = text
        .split_once('=')
        .ok_or_else(|| format!("-D wants NAME=VALUE, not \"{text}\""))?;
    Ok((name.trim().to_string(), rc_common::parse_number(value)?))
}

fn run() -> Result<(), String> {
    match Cli::parse().command {
        Command::Inspect { image } => {
            let bytes = std::fs::read(&image)
                .map_err(|e| format!("cannot read {}: {e}", image.display()))?;
            for (index, segment) in rcld::atari::inspect(&bytes)?.iter().enumerate() {
                let end = usize::from(segment.start) + segment.bytes.len() - 1;
                rc_common::out(&format!(
                    "{index}: ${:04X}-${end:04X} ({} bytes)\n",
                    segment.start,
                    segment.bytes.len()
                ));
            }
        }
        Command::Template { machine } => {
            let text = rcld::templates::template(&machine).ok_or_else(|| {
                format!("no template for \"{machine}\"; try c64, spectrum or cpc")
            })?;
            rc_common::out(text);
        }
        Command::Build {
            layout: path,
            output,
            map,
            symbols,
            defines,
            force,
        } => {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let layout =
                rcld::layout::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            let base = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let out = match (output, &layout.output) {
                (Some(o), _) => o,
                (None, Some(o)) => base.join(o),
                (None, None) => {
                    return Err(
                        "no output: give -o FILE or output = \"...\" in the layout".to_string()
                    );
                }
            };
            let defines = defines
                .iter()
                .map(|d| parse_define(d))
                .collect::<Result<Vec<_>, _>>()?;
            let built = rcld::build(&layout, base, &out, &defines)?;
            for n in &built.notes {
                rc_common::err(&format!("rcld: note: {n}\n"));
            }
            write_file(&out, &built.bytes, force)?;
            if let Some(m) = map {
                write_file(&m, built.map.as_bytes(), force)?;
            }
            if let Some(s) = symbols {
                write_file(&s, built.symbols_text.as_bytes(), force)?;
            }
            rc_common::err(&format!(
                "rcld: {} ({} bytes)\n",
                out.display(),
                built.bytes.len()
            ));
        }
    }
    Ok(())
}
