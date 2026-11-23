use clap::{Parser, Subcommand};
use rc_common::obj::Archive;
use rc_common::write_file;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "rcar",
    version,
    about = "Object libraries for rcasm and rcld",
    long_about = "Create, list, extract and edit object libraries (.rca): collections of rcasm objects (.rco) with an index of the names they export. rcld links a member only when something needs a name it defines."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Make a library from objects.
    Create {
        library: PathBuf,
        #[arg(required = true)]
        objects: Vec<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Show the members (and with --symbols what they export and import).
    List {
        library: PathBuf,
        #[arg(long)]
        symbols: bool,
    },
    /// Write members out as `.rco` files into a directory.
    Extract {
        library: PathBuf,
        /// Members to extract (all when none are named).
        members: Vec<String>,
        #[arg(long, short = 'd', default_value = ".")]
        dir: PathBuf,
        #[arg(long)]
        force: bool,
    },
    /// Add objects to a library, replacing members of the same name.
    Add {
        library: PathBuf,
        #[arg(required = true)]
        objects: Vec<PathBuf>,
    },
    /// Remove members.
    Delete {
        library: PathBuf,
        #[arg(required = true)]
        members: Vec<String>,
    },
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Create {
            library,
            objects,
            force,
        } => {
            let mut a = Archive::new();
            rcar::add_files(&mut a, &objects)?;
            write_file(&library, &a.encode()?, force)?;
            eprintln!("rcar: {} member(s)", a.len());
        }
        Command::List { library, symbols } => {
            let a = rcar::read_archive(&library)?;
            print!("{}", rcar::list(&a, symbols)?);
        }
        Command::Extract {
            library,
            members,
            dir,
            force,
        } => {
            let a = rcar::read_archive(&library)?;
            let names: Vec<String> = if members.is_empty() {
                a.names().map(str::to_string).collect()
            } else {
                members
            };
            for name in names {
                let i = a
                    .find(&name)
                    .ok_or_else(|| format!("{}: no member \"{name}\"", library.display()))?;
                let bytes = a.member_bytes(i).unwrap_or(&[]);
                // a member name is a file name, never a path
                let file = dir.join(rcar::member_name(std::path::Path::new(&name))?);
                write_file(&file, bytes, force)?;
            }
        }
        Command::Add { library, objects } => {
            let mut a = if library.exists() {
                rcar::read_archive(&library)?
            } else {
                Archive::new()
            };
            rcar::add_files(&mut a, &objects)?;
            write_file(&library, &a.encode()?, true)?;
            eprintln!("rcar: {} member(s)", a.len());
        }
        Command::Delete { library, members } => {
            let mut a = rcar::read_archive(&library)?;
            for m in &members {
                if !a.remove(m) {
                    return Err(format!("{}: no member \"{m}\"", library.display()));
                }
            }
            write_file(&library, &a.encode()?, true)?;
            eprintln!("rcar: {} member(s)", a.len());
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rcar: {e}");
            ExitCode::from(1)
        }
    }
}
