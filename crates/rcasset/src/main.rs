use clap::{Parser, Subcommand};
use rc_common::write_file;
use rcasset::Image;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "rcasset",
    version,
    about = "Convert pictures to the tile and palette formats of retro machines",
    long_about = "Reads a PNG (indexed, or with few enough colours) and writes the machine's data: tiles (nes, snes2, snes4, md4, pce4), a palette (snes, md) or a ZX Spectrum screen."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Cut the picture into 8x8 tiles in a machine's format.
    Tiles {
        picture: PathBuf,
        #[arg(long, short = 'f')]
        format: String,
        #[arg(long, short = 'o')]
        output: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Write the picture's palette as machine words (snes, md).
    Palette {
        picture: PathBuf,
        #[arg(long, short = 'f')]
        format: String,
        #[arg(long, short = 'o')]
        output: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// A 256x192 picture as a 6912-byte Spectrum screen (`.scr`).
    Spectrum {
        picture: PathBuf,
        #[arg(long, short = 'o')]
        output: PathBuf,
        #[arg(long)]
        force: bool,
    },
}

fn load(p: &PathBuf) -> Result<Image, String> {
    let bytes = std::fs::read(p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
    Image::from_png(&bytes).map_err(|e| format!("{}: {e}", p.display()))
}

fn run(cli: Cli) -> Result<(), String> {
    let (bytes, output, force) = match cli.command {
        Command::Tiles {
            picture,
            format,
            output,
            force,
        } => (
            load(&picture)?.tiles(&format).map_err(|e| e.to_string())?,
            output,
            force,
        ),
        Command::Palette {
            picture,
            format,
            output,
            force,
        } => (
            load(&picture)?
                .palette_words(&format)
                .map_err(|e| e.to_string())?,
            output,
            force,
        ),
        Command::Spectrum {
            picture,
            output,
            force,
        } => (
            load(&picture)?
                .spectrum_screen()
                .map_err(|e| e.to_string())?,
            output,
            force,
        ),
    };
    write_file(&output, &bytes, force)?;
    eprintln!("rcasset: {} bytes to {}", bytes.len(), output.display());
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rcasset: {e}");
            ExitCode::from(1)
        }
    }
}
