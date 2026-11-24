use clap::{Parser, Subcommand};
use rc_common::write_file;
use rcb::{Project, templates};
use std::path::PathBuf;
use std::process::{Command as Proc, ExitCode};

#[derive(Parser)]
#[command(
    name = "rcb",
    version,
    about = "Project driver: new, build, run, debug, clean, check",
    long_about = "A project is a folder with an rcb.toml (an rcld layout plus a [project] table naming the sources). \
        `rcb build` assembles and links it into the machine's own file (build/<name>.<ext>) with a map and a symbol \
        file; `rcb run` and `rcb debug` start the emulator (`rcd`, `rcdd`; RCB_EMULATOR names another program) on it."
)]
struct Cli {
    /// The project folder (default: the current one).
    #[arg(long, short = 'C', global = true, default_value = ".")]
    dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Make a new project folder.
    New {
        name: String,
        #[arg(long, short = 'm')]
        machine: String,
    },
    /// Assemble, link and write the image, the map and the symbol file.
    Build {
        #[arg(long)]
        force: bool,
    },
    /// Build, then start the emulator on the image.
    Run,
    /// Build, then start the debugging emulator (`rcdd`) with the symbols.
    Debug,
    /// Remove the build folder.
    Clean,
    /// Assemble and link without writing anything.
    Check,
    /// List the machines and the file each project makes.
    Targets,
    /// List the hardware kits (include files for the machines' registers).
    Kits,
    /// Print a machine's hardware kit (`rcb kit c64 > src/kit.inc`).
    Kit { machine: String },
}

fn fail(m: impl std::fmt::Display) -> ExitCode {
    eprintln!("rcb: {m}");
    ExitCode::from(1)
}

fn build(p: &Project, write: bool, force: bool) -> Result<(), String> {
    let b = p.build()?;
    for n in &b.notes {
        eprintln!("rcb: note: {n}");
    }
    if write {
        if let Some(dir) = p.output.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        write_file(&p.output, &b.bytes, true)
            .map_err(|e| format!("{}: {e}", p.output.display()))?;
        write_file(&p.map_path(), b.map.as_bytes(), true)?;
        if !b.symbols_text.is_empty() {
            write_file(&p.symbols_path(), b.symbols_text.as_bytes(), true)?;
        }
        let _ = force;
        eprintln!("rcb: {} ({} bytes)", p.output.display(), b.bytes.len());
    } else {
        eprintln!("rcb: ok, {} bytes", b.bytes.len());
    }
    Ok(())
}

fn launch(p: &Project, debug: bool) -> Result<(), String> {
    build(p, true, true)?;
    let program = std::env::var("RCB_EMULATOR")
        .unwrap_or_else(|_| if debug { "rcdd" } else { "rcd" }.to_string());
    let args = p.run_args(debug)?;
    eprintln!("rcb: {program} {}", args.join(" "));
    let status = Proc::new(&program).args(&args).status().map_err(|e| {
        format!("cannot start {program}: {e} (is it on the PATH? RCB_EMULATOR names another)")
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} stopped with {status}"))
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result: Result<(), String> = match cli.command {
        Command::New { name, machine } => (|| {
            let t = templates::find(&machine).ok_or_else(|| {
                format!(
                    "no template for \"{machine}\"; try {}",
                    templates::ALL
                        .iter()
                        .map(|t| t.machine)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
            let root = cli.dir.join(&name);
            if root.exists() {
                return Err(format!("{} exists already", root.display()));
            }
            for (path, text) in templates::project(&name, t) {
                let full = root.join(&path);
                if let Some(parent) = full.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&full, text).map_err(|e| format!("{}: {e}", full.display()))?;
            }
            eprintln!(
                "rcb: made {} ({machine}); cd {name} && rcb build",
                root.display()
            );
            Ok(())
        })(),
        Command::Build { force } => Project::load(&cli.dir).and_then(|p| build(&p, true, force)),
        Command::Check => Project::load(&cli.dir).and_then(|p| build(&p, false, false)),
        Command::Run => Project::load(&cli.dir).and_then(|p| launch(&p, false)),
        Command::Debug => Project::load(&cli.dir).and_then(|p| launch(&p, true)),
        Command::Clean => (|| {
            let dir = cli.dir.join("build");
            if dir.exists() {
                std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            Ok(())
        })(),
        Command::Targets => {
            for (m, emu) in rcb::machines() {
                println!(
                    "{:<10} .{:<4} {}",
                    m.name(),
                    rcb::default_extension(m),
                    if emu.is_some() {
                        "runs in retroclawd"
                    } else {
                        "builds; not in the emulator yet"
                    }
                );
            }
            Ok(())
        }
        Command::Kits => {
            for k in rcb::kits::ALL {
                println!("{:<10} {}", k.machine, k.about);
            }
            Ok(())
        }
        Command::Kit { machine } => rcb::kits::find(&machine)
            .map(|k| print!("{}", k.text))
            .ok_or_else(|| {
                format!(
                    "no kit for \"{machine}\"; try {}",
                    rcb::kits::ALL.iter().map(|k| k.machine).collect::<Vec<_>>().join(", ")
                )
            }),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
}
