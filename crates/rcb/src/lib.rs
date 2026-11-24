//! `rcb`: the project driver. A project is a folder with an `rcb.toml`: an `rcld` layout plus a `[project]` table.
//!
//! ```toml
//! machine = "pcengine"          # the rcld layout comes first (keys after a table header belong to the table)
//! entry = "reset"
//! [options]
//! # ...
//!
//! [project]                     # last
//! name = "hello"
//! sources = ["src/*.asm"]       # globs (a `*` in the file name); become the first [[input]] (asm = [...])
//! separate = false              # true: each source is assembled to its own object and linked (src = [...])
//! output = "build/hello.pce"    # default build/<name>.<the machine's usual extension>
//! ```
//!
//! `rcb build` makes the image, a map (`build/<name>.map`) and a symbol file (`build/<name>.sym`); `rcb run` and
//! `rcb debug` start the emulator (`rcd`, `rcdd`) on it.

use rcld::layout::{self, Machine};
use std::path::{Path, PathBuf};

pub mod kits;
pub mod templates;

pub struct Project {
    pub root: PathBuf,
    pub name: String,
    pub machine: Machine,
    pub layout: layout::Layout,
    pub output: PathBuf,
    pub run: Option<Vec<String>>,
}

/// The usual extension of a machine's image (what `rcb build` writes by default).
pub fn default_extension(m: Machine) -> &'static str {
    match m {
        Machine::C64 => "prg",
        Machine::Atari => "xex",
        Machine::Spectrum => "tap",
        Machine::Cpc => "dsk",
        Machine::AtariSt => "prg",
        Machine::PcEngine => "pce",
        Machine::Nes => "nes",
        Machine::Snes => "sfc",
        Machine::Amiga => "adf",
        Machine::X68000 => "x",
        Machine::Genesis => "md",
        Machine::NeoGeo => "zip",
        Machine::SegaCd => "iso",
        Machine::Generic => "bin",
    }
}

/// Every machine the layouts know, with the id the emulator uses for it (`None`: not in the emulator yet).
pub fn machines() -> Vec<(Machine, Option<&'static str>)> {
    vec![
        (Machine::C64, Some("c64")),
        (Machine::Atari, Some("atari-800xl")),
        (Machine::Spectrum, Some("zx-spectrum")),
        (Machine::Cpc, Some("cpc6128")),
        (Machine::AtariSt, Some("atari-st")),
        (Machine::PcEngine, Some("pcengine")),
        (Machine::Genesis, Some("genesis")),
        (Machine::NeoGeo, Some("neogeo")),
        (Machine::SegaCd, Some("segacd")),
        (Machine::Nes, None),
        (Machine::Snes, None),
        (Machine::Amiga, None),
        (Machine::X68000, None),
    ]
}

/// Expands `*` in the last component of `pattern` (relative to `root`).
pub fn glob(root: &Path, pattern: &str) -> Result<Vec<String>, String> {
    if !pattern.contains('*') {
        return Ok(vec![pattern.to_string()]);
    }
    let (dir, file) = match pattern.rsplit_once('/') {
        Some((d, f)) => (d, f),
        None => ("", pattern),
    };
    if dir.contains('*') {
        return Err(format!(
            "\"{pattern}\": a * is allowed in the file name only"
        ));
    }
    let (pre, post) = file.split_once('*').unwrap_or((file, ""));
    let mut found = Vec::new();
    let entries = std::fs::read_dir(root.join(dir)).map_err(|e| format!("{pattern}: {e}"))?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.len() >= pre.len() + post.len()
            && name.starts_with(pre)
            && name.ends_with(post)
            && e.path().is_file()
        {
            found.push(if dir.is_empty() {
                name
            } else {
                format!("{dir}/{name}")
            });
        }
    }
    found.sort();
    if found.is_empty() {
        return Err(format!("\"{pattern}\" matches no file"));
    }
    Ok(found)
}

impl Project {
    /// Reads `rcb.toml` from `root`.
    pub fn load(root: &Path) -> Result<Project, String> {
        let path = root.join("rcb.toml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::parse(root, &text)
    }

    pub fn parse(root: &Path, text: &str) -> Result<Project, String> {
        let mut table: toml::Table = text.parse().map_err(|e| format!("rcb.toml: {e}"))?;
        let project = match table.remove("project") {
            Some(toml::Value::Table(t)) => t,
            Some(_) => return Err("rcb.toml: [project] is a table".to_string()),
            None => toml::Table::new(),
        };
        let run = match table.remove("run") {
            Some(toml::Value::Table(t)) => match t.get("args") {
                Some(toml::Value::Array(a)) => Some(
                    a.iter()
                        .map(|v| {
                            v.as_str()
                                .map(str::to_string)
                                .ok_or("rcb.toml: run.args are strings")
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                None => None,
                Some(_) => return Err("rcb.toml: run.args is a list of strings".to_string()),
            },
            _ => None,
        };
        for key in project.keys() {
            if !["name", "sources", "separate", "output"].contains(&key.as_str()) {
                return Err(format!(
                    "rcb.toml: unknown [project] key \"{key}\" (name, sources, separate, output)"
                ));
            }
        }
        let name = project
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "project".to_string());
        if let Some(srcs) = project.get("sources") {
            let patterns: Vec<String> = srcs
                .as_array()
                .ok_or("rcb.toml: project.sources is a list")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or("rcb.toml: project.sources are strings")
                })
                .collect::<Result<_, _>>()?;
            let mut files = Vec::new();
            for p in &patterns {
                files.extend(glob(root, p)?);
            }
            let separate = project
                .get("separate")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut input = toml::Table::new();
            input.insert(
                if separate { "src" } else { "asm" }.to_string(),
                toml::Value::Array(files.into_iter().map(toml::Value::String).collect()),
            );
            let mut inputs = vec![toml::Value::Table(input)];
            if let Some(toml::Value::Array(existing)) = table.remove("input") {
                inputs.extend(existing);
            }
            table.insert("input".to_string(), toml::Value::Array(inputs));
        }
        if !table.contains_key("name") {
            table.insert("name".to_string(), toml::Value::String(name.clone()));
        }
        let layout_text = toml::to_string(&table).map_err(|e| e.to_string())?;
        let layout = layout::parse(&layout_text).map_err(|e| format!("rcb.toml: {e}"))?;
        let machine = layout.machine;
        let output = match project.get("output").and_then(|v| v.as_str()) {
            Some(o) => root.join(o),
            None => root
                .join("build")
                .join(format!("{name}.{}", default_extension(machine))),
        };
        Ok(Project {
            root: root.to_path_buf(),
            name,
            machine,
            layout,
            output,
            run,
        })
    }

    /// Builds the image; returns it with the notes, the map and the symbol text (not written).
    pub fn build(&self) -> Result<rcld::Build, String> {
        rcld::build(&self.layout, &self.root, &self.output, &[])
    }

    pub fn map_path(&self) -> PathBuf {
        self.output.with_extension("map")
    }

    pub fn symbols_path(&self) -> PathBuf {
        self.output.with_extension("sym")
    }

    /// The emulator command line (without the program name) that runs the image, or why it is not known.
    pub fn run_args(&self, debug: bool) -> Result<Vec<String>, String> {
        let out = self.output.display().to_string();
        let mut args: Vec<String> = match &self.run {
            Some(a) => a.iter().map(|s| s.replace("{output}", &out)).collect(),
            None => {
                let id = machines()
                    .into_iter()
                    .find(|(m, _)| *m == self.machine)
                    .and_then(|(_, id)| id)
                    .ok_or_else(|| {
                        format!(
                            "the emulator has no {} machine yet: `rcb build` makes the image; give [run] args = [...] to run it elsewhere",
                            self.machine.name()
                        )
                    })?;
                // the emulator's media options by the kind of image [H?: checked for --rom and --program only]
                let ext = self
                    .output
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let flag = match ext.as_str() {
                    "tap" | "tzx" => "--tape",
                    "dsk" | "d64" | "d81" | "adf" | "st" => "--disk",
                    "iso" | "cue" | "chd" => "--disc",
                    "prg" | "xex" => "--program",
                    _ => "--rom",
                };
                vec![id.to_string(), flag.to_string(), out]
            }
        };
        if debug {
            args.push("--symbols".to_string());
            args.push(self.symbols_path().display().to_string());
        }
        Ok(args)
    }
}
