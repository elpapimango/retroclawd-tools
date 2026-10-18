//! Runs every `*.manifest.toml` below `RC_MANIFESTS` (a folder, searched two levels deep) against the real
//! emulator. A new machine adds a manifest there; this test needs no change. Skips without `RC_MANIFESTS`,
//! `RETROCLAWD_BIN`, `RC_ROMS` and `RC_GOLDENS`; manifests are still parsed and validated whenever
//! `RC_MANIFESTS` is set.

use std::path::{Path, PathBuf};

use rc_suite::Env;
use rc_suite::scenario::{Manifest, run_manifest};

fn manifests_below(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && depth > 0 {
            manifests_below(&p, depth - 1, out);
        } else if p
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".manifest.toml"))
        {
            out.push(p);
        }
    }
}

#[test]
fn every_manifest_is_valid_and_its_scenarios_pass() {
    let Some(dir) = std::env::var_os("RC_MANIFESTS").map(PathBuf::from) else {
        eprintln!("skipped: RC_MANIFESTS is not set");
        return;
    };
    let mut paths = Vec::new();
    manifests_below(&dir, 2, &mut paths);
    paths.sort();
    let manifests: Vec<(PathBuf, Manifest)> = paths
        .into_iter()
        .map(|p| {
            let m = Manifest::load(&p).unwrap_or_else(|e| panic!("{e}"));
            (p, m)
        })
        .collect();
    let (Some(env), Some(goldens)) = (
        Env::discover(),
        std::env::var_os("RC_GOLDENS").map(PathBuf::from),
    ) else {
        eprintln!(
            "{} manifest(s) are valid; the scenarios were skipped (RETROCLAWD_BIN, RC_ROMS, RC_GOLDENS)",
            manifests.len()
        );
        return;
    };
    let mut failures = String::new();
    for (path, manifest) in &manifests {
        let report = run_manifest(&env, &goldens, manifest);
        eprintln!("{}:\n{}", path.display(), report.summary());
        if !report.ok() {
            failures += &format!("{}:\n{}", path.display(), report.summary());
        }
    }
    assert!(failures.is_empty(), "{failures}");
}
