//! `rc-scenarios`: runs a machine's scenario manifest against the real emulator, and pins goldens by hash.
//!
//! ```text
//! rc-scenarios run <manifest.toml> [--goldens DIR]   needs RETROCLAWD_BIN and RC_ROMS (and RC_GOLDENS unless --goldens)
//! rc-scenarios hash <file>...                        prints the SHA-256 to put in `golden_sha256`
//! rc-scenarios check <manifest.toml>...              validates manifests only (no emulator)
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use rc_suite::Env;
use rc_suite::scenario::{Manifest, run_manifest, sha256_hex};

#[derive(Parser)]
#[command(version, about = "Data-driven emulator scenarios")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run every scenario of a manifest.
    Run {
        manifest: PathBuf,
        /// The folder holding the goldens (default: `RC_GOLDENS`).
        #[arg(long)]
        goldens: Option<PathBuf>,
    },
    /// Print the SHA-256 of files, for pinning goldens.
    Hash { files: Vec<PathBuf> },
    /// Check that manifests parse and keep the rules.
    Check { manifests: Vec<PathBuf> },
}

fn main() -> ExitCode {
    match real_main(Cli::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("rc-scenarios: {e}");
            ExitCode::from(2)
        }
    }
}

fn real_main(cli: Cli) -> Result<bool, String> {
    match cli.command {
        Cmd::Hash { files } => {
            for f in files {
                let bytes = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
                println!("{}  {}", sha256_hex(&bytes), f.display());
            }
            Ok(true)
        }
        Cmd::Check { manifests } => {
            for m in &manifests {
                Manifest::load(m)?;
                println!("ok  {}", m.display());
            }
            Ok(true)
        }
        Cmd::Run { manifest, goldens } => {
            let manifest = Manifest::load(&manifest)?;
            let env = Env::discover().ok_or("set RETROCLAWD_BIN and RC_ROMS")?;
            let goldens = goldens
                .or_else(|| std::env::var_os("RC_GOLDENS").map(PathBuf::from))
                .ok_or("set RC_GOLDENS or pass --goldens")?;
            let report = run_manifest(&env, &goldens, &manifest);
            print!("{}", report.summary());
            Ok(report.ok())
        }
    }
}
